#!/usr/bin/env bash
# NovelWorld scripted restore — backup-restore-v1 (docs/BACKUP_RESTORE.md).
#
#   restore.sh --manifest FILE [--newer-artifact MANIFEST]... [--decisions FILE]
#              [--declared-failure-time 'YYYY-MM-DD HH:MM:SS+00']
#              [--i-stopped-writes] [--env-file FILE]
#
# Order is the restore procedure contract, and nothing before step 5 changes any
# data:
#   1 verify manifests, checksums and decryptability
#   2 confirm application services are stopped
#   3 collect erasure sources (this artifact, newer artifacts, live database)
#   4 compute the residual window and gate on attest-or-erase decisions
#   5 load the dump into a clean database
#   6 re-insert erasure sources, write decisions, rotate JWT_SECRET, drop tokens
#   7 replay erasure through the standard migration path
#
# --i-stopped-writes is an acknowledgement, not a bypass: step 2 always verifies.
# The preserved .env must already be installed (secret custody prerequisite);
# this script rotates JWT_SECRET in it and reports any secret that is missing.
# Set COMPOSE_FILE when the deployment uses more than docker-compose.yml.
set -euo pipefail

container=${POSTGRES_CONTAINER:-novel-postgres}
pg_user=${POSTGRES_USER:-novel}
pg_db=${POSTGRES_DB:-novel_world}
repo_root=$(cd "$(dirname "$0")/../.." && pwd)
app_services="gateway user-service novel-service agent-service narrative-service"
tab=$(printf '\t')

manifest=""
decisions=""
declared_failure_time=""
env_file=""
newer_manifests=""

fail() {
  printf 'restore: %s\n' "$1" >&2
  exit 1
}

abspath() {
  case "$1" in
  /*) printf '%s' "$1" ;;
  *) printf '%s/%s' "$PWD" "$1" ;;
  esac
}

# Manifests and decision files are operator-supplied text that ends up inside
# SQL literals, so every value is shape-checked at the boundary where it is
# read. Timestamps use exactly the format backup.sh writes.
# awk anchors match the whole string; grep -E would match any single line and
# let a multiline payload through with a well-formed first line.
matches() {
  # Through the environment, not -v: awk processes escape sequences in a -v
  # assignment, so the copy being validated would not be the value being used.
  match_value=$1 match_pattern=$2 awk \
    'BEGIN { exit !(ENVIRON["match_value"] ~ ("^" ENVIRON["match_pattern"] "$")) }'
}

check_timestamp() {
  matches "$1" '[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}([.][0-9]{1,6})?[+]00' ||
    fail "$2 is not a 'YYYY-MM-DD HH:MM:SS[.ffffff]+00' UTC timestamp: '$1'"
}

check_digest() {
  matches "$1" '[0-9a-f]{64}' || fail "$2 is not a SHA-256 digest: '$1'"
}

check_uuid() {
  matches "$1" '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' ||
    fail "$2 is not a UUID: '$1'"
}

# The shape check above leaves only digits and separators, so the value is safe
# to hand to the server that will store it. PostgreSQL owns the calendar: it is
# the only thing that agrees with the column the value lands in.
check_calendar() {
  docker exec "$container" psql -U "$pg_user" -d postgres -At -c \
    "SELECT '$1'::pg_catalog.timestamptz" >/dev/null 2>&1 ||
    fail "$2 is not a real UTC timestamp: '$1'"
}

while [ $# -gt 0 ]; do
  case "$1" in
  --manifest)
    manifest=$(abspath "${2:?--manifest needs a value}")
    shift 2
    ;;
  --newer-artifact)
    newer_manifests="$newer_manifests$(abspath "${2:?--newer-artifact needs a value}")
"
    shift 2
    ;;
  --decisions)
    decisions=$(abspath "${2:?--decisions needs a value}")
    shift 2
    ;;
  --declared-failure-time)
    declared_failure_time=${2:?--declared-failure-time needs a value}
    shift 2
    ;;
  --env-file)
    env_file=$(abspath "${2:?--env-file needs a value}")
    shift 2
    ;;
  --i-stopped-writes) shift ;;
  *) fail "unknown argument '$1'" ;;
  esac
done

[ -n "$manifest" ] ||
  fail 'usage: restore.sh --manifest FILE [--newer-artifact M]... [--decisions FILE]'
[ -n "$env_file" ] || env_file=$repo_root/.env
[ -n "${BACKUP_ENCRYPTION_KEY:-}" ] || fail 'BACKUP_ENCRYPTION_KEY is required to decrypt the artifact'
[ "${#BACKUP_ENCRYPTION_KEY}" -ge 32 ] || fail 'BACKUP_ENCRYPTION_KEY must be at least 32 characters'
[ -f "$env_file" ] ||
  fail "no deployment environment at $env_file; install the preserved .env before restoring"
[ -z "$declared_failure_time" ] ||
  check_timestamp "$declared_failure_time" '--declared-failure-time'


cd "$repo_root"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/sources"

# ─── 1. verify ─────────────────────────────────────────────────────────────
# Every artifact is checked against its manifest and decrypted here, before any
# other step, so a corrupt artifact or a wrong key fails closed with nothing
# changed.

manifest_value() {
  awk -F= -v key="$2" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$1"
}

verify_file() {
  # $1 manifest, $2 file key, $3 digest key; prints the verified path
  local dir file digest
  dir=$(dirname "$1")
  file=$(manifest_value "$1" "$2")
  digest=$(manifest_value "$1" "$3")
  [ -n "$file" ] && [ -n "$digest" ] || fail "manifest $1 is missing $2/$3"
  [ -f "$dir/$file" ] || fail "artifact $dir/$file named by $1 is missing"
  printf '%s  %s\n' "$digest" "$dir/$file" | sha256sum --check --status ||
    fail "checksum mismatch for $dir/$file; refusing to restore an unverified artifact"
  printf '%s' "$dir/$file"
}

# First value of one column of one COPY block, selected through the block's own
# header so column order cannot shift a field.
copy_column() {
  awk -v table="$2" -v want="$3" '
    $0 ~ "^COPY public\\." table " \\(" {
      header = $0
      sub(/^[^(]*\(/, "", header)
      sub(/\).*$/, "", header)
      gsub(/,/, " ", header)
      count = split(header, columns, " ")
      for (i = 1; i <= count; i++) index_of[columns[i]] = i
      if (!(want in index_of)) exit 3
      inside = 1
      next
    }
    inside && $0 == "\\." { inside = 0; next }
    inside { print $index_of[want]; exit }
  ' "$1"
}

decrypt() {
  openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 -salt -pass env:BACKUP_ENCRYPTION_KEY \
    -in "$1" 2>/dev/null | gzip -dc >"$2" ||
    fail "cannot decrypt $1; check BACKUP_ENCRYPTION_KEY and the artifact"
}

[ -f "$manifest" ] || fail "no manifest at $manifest"
[ "$(manifest_value "$manifest" schema)" = backup-artifact-v1 ] ||
  fail "$manifest is not a backup-artifact-v1 manifest"
covered_through=$(manifest_value "$manifest" covered_through)
[ -n "$covered_through" ] || fail "$manifest has no covered_through timestamp"
check_timestamp "$covered_through" "the covered_through of $manifest"
check_digest "$(manifest_value "$manifest" dump_sha256)" "the dump_sha256 of $manifest"
check_digest "$(manifest_value "$manifest" erasure_sha256)" "the erasure_sha256 of $manifest"
dump_file=$(verify_file "$manifest" dump dump_sha256)
erasure_file=$(verify_file "$manifest" erasure erasure_sha256)
decrypt "$dump_file" "$work/dump.sql"
decrypt "$erasure_file" "$work/sources/artifact.tsv"
grep -q '^COPY public\.users (' "$work/dump.sql" ||
  fail "the decrypted dump has no users table; it is not a NovelWorld artifact"
# Only digests this run verified enter the inventory, each labelled with what it
# covers: the artifact whose dump was loaded, and the erasure export of every
# newer artifact that contributed records. A newer artifact's dump is never
# verified here — it is not restored — so its digest is not claimed.
inventory="dump:$(manifest_value "$manifest" dump_sha256)"
inventory="$inventory,erasure:$(manifest_value "$manifest" erasure_sha256)"
newest_covered_through=$covered_through

# The manifest token must agree with the token inside the verified dump. A
# disagreement between two present tokens, or one present without the other, is
# tampering. A wholly token-less artifact is legal — it predates the token
# migration — but can never establish continuation, so it always faces the
# disaster gate.
dump_token=$(copy_column "$work/dump.sql" database_lineage token)
manifest_token=$(manifest_value "$manifest" lineage_token)
[ -z "$manifest_token" ] || check_uuid "$manifest_token" "the lineage_token of $manifest"
if [ -n "$manifest_token" ] && [ -n "$dump_token" ]; then
  [ "$manifest_token" = "$dump_token" ] ||
    fail "the manifest lineage token ($manifest_token) disagrees with the dump's ($dump_token)"
elif [ -n "$manifest_token$dump_token" ]; then
  fail 'the artifact carries a lineage token in only one of its manifest and dump'
fi
artifact_token=$dump_token

index=0
newer_covered_list=""
while IFS= read -r newer; do
  [ -n "$newer" ] || continue
  [ -f "$newer" ] || fail "no manifest at $newer"
  [ "$(manifest_value "$newer" schema)" = backup-artifact-v1 ] ||
    fail "$newer is not a backup-artifact-v1 manifest"
  index=$((index + 1))
  newer_covered=$(manifest_value "$newer" covered_through)
  [ -n "$newer_covered" ] || fail "$newer has no covered_through timestamp"
  check_timestamp "$newer_covered" "the covered_through of $newer"
  newer_covered_list="$newer_covered_list$newer_covered
"
  check_digest "$(manifest_value "$newer" dump_sha256)" "the dump_sha256 of $newer"
  check_digest "$(manifest_value "$newer" erasure_sha256)" "the erasure_sha256 of $newer"
  decrypt "$(verify_file "$newer" erasure erasure_sha256)" "$work/sources/newer-$index.tsv"
  inventory="$inventory,erasure:$(manifest_value "$newer" erasure_sha256)"
  # Fixed-width UTC timestamps, so lexical order is chronological order.
  if [ "$newer_covered" \> "$newest_covered_through" ]; then
    newest_covered_through=$newer_covered
  fi
done <<EOF
$newer_manifests
EOF

# ─── 2. stop writes ────────────────────────────────────────────────────────
# No deletion may commit between the erasure export and the replacement.

running=""
for service in $app_services; do
  if docker inspect --format '{{.State.Running}}' "novel-$service" 2>/dev/null |
    grep -qx true; then
    running="$running novel-$service"
  fi
done
if compose_running=$(docker compose ps --services --status running 2>/dev/null); then
  for service in $app_services; do
    if printf '%s\n' "$compose_running" | grep -qx "$service"; then
      running="$running $service"
    fi
  done
fi
[ -z "$running" ] ||
  fail "stop the application services before restoring; still running:$running"

docker inspect --format '{{.State.Running}}' "$container" 2>/dev/null | grep -qx true ||
  fail "postgres container '$container' is not running; start it with: docker compose up -d postgres"

check_calendar "$covered_through" "the covered_through of $manifest"
while IFS= read -r newer_covered; do
  [ -z "$newer_covered" ] || check_calendar "$newer_covered" 'a newer artifact covered_through'
done <<EOF
$newer_covered_list
EOF
[ -z "$declared_failure_time" ] ||
  check_calendar "$declared_failure_time" '--declared-failure-time'

writes_stopped_at=$(docker exec "$container" psql -U "$pg_user" -d postgres -At -c \
  "SELECT pg_catalog.to_char(pg_catalog.now() AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US+00')")
[ -n "$writes_stopped_at" ] || fail 'could not read the writes-stopped timestamp'
check_timestamp "$writes_stopped_at" 'the writes-stopped timestamp'

# ─── 3. collect erasure sources ────────────────────────────────────────────

# Account and novel inventory of the restored dump, read from the dump's own
# COPY headers so a column order change cannot silently shift a field. Read
# before the live export because the lineage test below compares against it.
copy_block() {
  awk -v table="$1" -v want="$2" '
    $0 ~ "^COPY public\\." table " \\(" {
      header = $0
      sub(/^[^(]*\(/, "", header)
      sub(/\).*$/, "", header)
      gsub(/,/, " ", header)
      count = split(header, columns, " ")
      for (i = 1; i <= count; i++) index_of[columns[i]] = i
      wanted_count = split(want, wanted, ",")
      for (i = 1; i <= wanted_count; i++) if (!(wanted[i] in index_of)) exit 3
      inside = 1
      next
    }
    inside && $0 == "\\." { inside = 0; next }
    inside {
      split($0, field, "\t")
      line = ""
      for (i = 1; i <= wanted_count; i++) {
        line = line (i > 1 ? "\t" : "") field[index_of[wanted[i]]]
      }
      print line
    }
  ' "$work/dump.sql"
}
copy_block users id,role >"$work/accounts.tsv" ||
  fail 'the dump has no id/role columns on users'
copy_block novels id,user_id >"$work/novels.tsv" ||
  fail 'the dump has no id/user_id columns on novels'
cut -f1 "$work/accounts.tsv" | sort >"$work/account-ids.txt"

# Live continuation is established only by token equality. Row counts and shared
# UUIDs prove nothing: two restores of one artifact are siblings that share every
# UUID by construction, and a token-less artifact can never match, so it always
# faces the disaster gate.
live_token=$(docker exec "$container" psql -U "$pg_user" -d "$pg_db" -At -c \
  "SELECT token FROM public.database_lineage" 2>/dev/null || true)
live_source=false
if [ -n "$artifact_token" ] && [ "$live_token" = "$artifact_token" ]; then
  live_source=true
  docker exec "$container" psql -U "$pg_user" -d "$pg_db" -At -c \
    "COPY (SELECT subject_type, subject_id, user_id, erased_at, had_source
             FROM public.erasure_records) TO STDOUT" >"$work/sources/live.tsv"
  inventory="$inventory,live:$writes_stopped_at"
  newest_covered_through=$writes_stopped_at
fi

# Disagreement is judged on the deletion facts alone: subject type, subject,
# owning user and deletion time. had_source is monotone bookkeeping that only an
# authoritative delete can raise, and a false-then-true succession is a
# legitimate upgrade — the restore writes a decision record before the delete
# that can see the key — so sources are merged with OR rather than treated as
# contradicting each other.
cat "$work"/sources/*.tsv |
  awk -F'\t' 'NF >= 5 { print $1 "\t" $2 "\t" $3 "\t" $4 }' |
  sort -u >"$work/facts.tsv"
cut -f1,2 "$work/facts.tsv" | sort | uniq -d >"$work/conflicts.txt"
if [ -s "$work/conflicts.txt" ]; then
  printf 'restore: erasure sources disagree on these subjects:\n' >&2
  awk -F'\t' 'NR == FNR { conflicted[$1 "\t" $2] = 1; next }
              ($1 "\t" $2) in conflicted { print "  " $0 }' \
    "$work/conflicts.txt" "$work/facts.tsv" >&2
  fail 'conflicting erasure records; resolve the sources and re-run'
fi
# One merged row per subject, so the re-insert below stays a single upsert.
cat "$work"/sources/*.tsv |
  awk -F'\t' 'NF >= 5 {
        key = $1 "\t" $2
        facts[key] = $1 "\t" $2 "\t" $3 "\t" $4
        if ($5 == "t" || $5 == "true") { had[key] = "t" }
        else if (!(key in had)) { had[key] = "f" }
      }
      END { for (key in facts) print facts[key] "\t" had[key] }' |
  sort >"$work/union.tsv"

# Collected records are facts, not proposals: their subjects are already decided
# and cannot be retained by any decision file.
awk -F'\t' '$1 == "user" { print $2 }' "$work/union.tsv" | sort -u >"$work/erased-users.txt"
awk -F'\t' '$1 == "novel" { print $2 }' "$work/union.tsv" | sort -u >"$work/erased-novels.txt"

# ─── 4. residual window and the attest-or-erase gate ───────────────────────

window_start=$newest_covered_through
window_end=${declared_failure_time:-$writes_stopped_at}
if [ "$live_source" = true ]; then
  window_state='empty (the current database was reachable and was exported)'
else
  window_state="non-empty ($window_start .. $window_end)"
  # A window that ends before it starts is not a window. The server that stores
  # the bounds decides the ordering, so the check agrees with the column.
  [ "$(docker exec "$container" psql -U "$pg_user" -d postgres -At -c \
    "SELECT '$window_start'::pg_catalog.timestamptz <= '$window_end'::pg_catalog.timestamptz")" = t ] ||
    fail "the residual window ends before it starts ($window_start .. $window_end); check --declared-failure-time"
fi

owns() { # owns FILE KEY VALUE
  awk -F'\t' -v key="$2" -v value="$3" '$1 == key && $2 == value { found = 1 }
                                        END { exit !found }' "$1"
}

listed() { # listed FILE VALUE
  grep -qxF "$2" "$1"
}

operator_identity=""
designated_admin=""
if [ -n "$decisions" ]; then
  [ "$live_source" = false ] ||
    fail 'the residual window is empty; attest-or-erase decisions are not accepted for this restore'
  [ -f "$decisions" ] || fail "no decision file at $decisions"
  : >"$work/decided.tsv"
  : >"$work/retained-novels.tsv"
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
    '' | '#'*) continue ;;
    operator=*) operator_identity=${line#operator=} ;;
    admin=*)
      designated_admin=${line#admin=}
      check_uuid "$designated_admin" 'the designated administrator'
      ;;
    'retain '*)
      rest=${line#retain }
      account=${rest%% *}
      novels=${rest#* }
      check_uuid "$account" 'a retained account'
      case "$novels" in
      novels=*) : ;;
      *) fail "retain $account must list its retained novels as novels=<uuid,...>" ;;
      esac
      printf 'retain\t%s\n' "$account" >>"$work/decided.tsv"
      # The trailing newline matters: read drops an unterminated last field,
      # which would silently unlist the last retained novel of every account.
      printf '%s\n' "${novels#novels=}" | tr ',' '\n' | while read -r novel; do
        if [ -n "$novel" ]; then
          printf '%s\t%s\n' "$account" "$novel" >>"$work/retained-novels.tsv"
        fi
      done
      ;;
    'erase '*)
      account=${line#erase }
      account=${account%% *}
      check_uuid "$account" 'an erased account'
      printf 'erase\t%s\n' "$account" >>"$work/decided.tsv"
      ;;
    *) fail "unrecognised decision line: '$line'" ;;
    esac
  done <"$decisions"
  [ -n "$operator_identity" ] || fail 'the decision file must carry operator=<identity>'

  while IFS="$tab" read -r decision account; do
    check_uuid "$account" 'a decided account'
    if listed "$work/erased-users.txt" "$account"; then
      fail "account $account is covered by a collected erasure record; replay enforces it and no decision may name it"
    fi
  done <"$work/decided.tsv"
  cut -f2 "$work/decided.tsv" | sort >"$work/decided-ids.txt"
  [ "$(sort -u <"$work/decided-ids.txt" | wc -l)" = "$(wc -l <"$work/decided-ids.txt")" ] ||
    fail 'the decision file decides the same account twice'
  # Subjects covered by a collected record are already decided facts.
  comm -23 "$work/account-ids.txt" "$work/erased-users.txt" >"$work/decision-required.txt"
  undecided=$(comm -23 "$work/decision-required.txt" "$work/decided-ids.txt")
  [ -z "$undecided" ] ||
    fail "every restored account needs a decision; undecided: $(printf '%s' "$undecided" | tr '\n' ' ')"
  unknown=$(comm -13 "$work/account-ids.txt" "$work/decided-ids.txt")
  [ -z "$unknown" ] ||
    fail "decisions name accounts that are not in the artifact: $(printf '%s' "$unknown" | tr '\n' ' ')"
  while IFS="$tab" read -r account novel; do
    check_uuid "$novel" 'a retained novel'
    owns "$work/novels.tsv" "$novel" "$account" ||
      fail "retained novel $novel does not belong to account $account in this artifact"
    if listed "$work/erased-novels.txt" "$novel"; then
      fail "novel $novel is covered by a collected erasure record and cannot be retained"
    fi
  done <"$work/retained-novels.tsv"

  retained_any=false
  retained_admin=false
  while IFS="$tab" read -r decision account; do
    [ "$decision" = retain ] || continue
    retained_any=true
    if owns "$work/accounts.tsv" "$account" admin; then
      retained_admin=true
    fi
  done <"$work/decided.tsv"
  if [ "$retained_any" = false ]; then
    # Erase-all is a sanctioned outcome: replay clears the runtime configuration
    # and the installation returns to first-run setup, so there is no account
    # left to administer and nothing to designate.
    [ -z "$designated_admin" ] ||
      fail 'these decisions retain no account, so admin= has nothing to designate'
  elif [ "$retained_admin" = false ]; then
    [ -n "$designated_admin" ] ||
      fail 'these decisions leave no administrator; add admin=<retained account uuid>'
    if listed "$work/erased-users.txt" "$designated_admin"; then
      fail "the designated administrator $designated_admin is covered by a collected erasure record"
    fi
    owns "$work/decided.tsv" retain "$designated_admin" ||
      fail "the designated administrator $designated_admin is not a retained account"
  else
    designated_admin=""
  fi
elif [ "$live_source" = false ]; then
  comm -23 "$work/account-ids.txt" "$work/erased-users.txt" >"$work/decision-required.txt"
  printf 'restore: refusing to complete a disaster restore with a non-empty residual window.\n' >&2
  printf 'restore: window %s .. %s; deletions committed inside it cannot be replayed.\n' \
    "$window_start" "$window_end" >&2
  printf 'restore: supply --decisions FILE with one line per account:\n' >&2
  printf '  operator=<identity string>\n  retain <user_uuid> novels=<uuid,uuid>\n  erase <user_uuid>\n' >&2
  # Novels already covered by a collected record are erased facts; the prompt
  # must not advertise them as retainable.
  awk -F'\t' 'NR == FNR { erased[$1] = 1; next } !($1 in erased)' \
    "$work/erased-novels.txt" "$work/novels.tsv" >"$work/available-novels.tsv"
  printf 'restore: accounts in this artifact that still need a decision:\n' >&2
  while IFS= read -r account; do
    printf '  %s role=%s novels=%s\n' "$account" \
      "$(awk -F'\t' -v id="$account" '$1 == id { print $2 }' "$work/accounts.tsv")" \
      "$(awk -F'\t' -v owner="$account" \
        '$2 == owner { printf "%s%s", separator, $1; separator = "," }' \
        "$work/available-novels.tsv")" >&2
  done <"$work/decision-required.txt"
  exit 2
fi

# ─── 5. load the dump into a clean database ────────────────────────────────

printf 'restore: verification passed; loading %s into a clean %s\n' "$dump_file" "$pg_db"
docker exec "$container" psql -U "$pg_user" -d postgres -q \
  -c "DROP DATABASE IF EXISTS $pg_db WITH (FORCE)"
docker exec "$container" psql -U "$pg_user" -d postgres -q -c "CREATE DATABASE $pg_db"

# One transaction: the restored data and its regenerated lineage token become
# reachable together. A crash before this commits leaves an empty database, and
# after it the live token is already the new one, so neither a partial nor a
# completed attempt can ever present the artifact's token as a live lineage —
# a retry always faces at least the gate the first attempt faced.
{
  cat "$work/dump.sql"
  printf 'CREATE TABLE IF NOT EXISTS public.database_lineage ('
  printf 'token UUID PRIMARY KEY, parent UUID,'
  printf ' created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.now());\n'
  printf 'CREATE UNIQUE INDEX IF NOT EXISTS database_lineage_singleton'
  printf ' ON public.database_lineage ((TRUE));\n'
  printf 'DELETE FROM public.database_lineage;\n'
  printf 'INSERT INTO public.database_lineage (token, parent) VALUES'
  printf " (pg_catalog.gen_random_uuid(), %s);\n" \
    "$([ -n "$artifact_token" ] && printf "'%s'" "$artifact_token" || printf NULL)"
  # Test-only injection point for the drill's crash cases; never set in
  # operation. Aborting here rolls back the load with the token regeneration.
  [ "${RESTORE_FAIL_BEFORE_COMMIT:-}" != 1 ] ||
    printf "DO \$injected\$ BEGIN RAISE EXCEPTION 'injected pre-commit failure'; END \$injected\$;\n"
} | docker exec -i "$container" psql -U "$pg_user" -d "$pg_db" -q \
  --single-transaction -v ON_ERROR_STOP=1 >/dev/null
[ "${RESTORE_FAIL_AFTER_COMMIT:-}" != 1 ] ||
  fail 'injected post-commit failure'

# The artifact may predate any migration, including the erasure journal itself,
# so bring the schema forward before writing erasure state into it.
docker compose run --rm postgres-migrate >/dev/null

# ─── 6. erasure sources, decisions, secret rotation ────────────────────────

{
  printf 'BEGIN;\n'
  printf 'CREATE TEMP TABLE staged_erasure (subject_type text, subject_id uuid,'
  printf ' user_id uuid, erased_at timestamptz, had_source boolean) ON COMMIT DROP;\n'
  printf 'COPY staged_erasure (subject_type, subject_id, user_id, erased_at, had_source)'
  printf ' FROM stdin;\n'
  cat "$work/union.tsv"
  printf '\\.\n'
  # had_source travels with the record, so a novel whose subject row is in no
  # dump still re-queues its retained source exactly once in the new lineage.
  printf 'INSERT INTO public.erasure_records'
  printf ' (subject_type, subject_id, user_id, erased_at, had_source)\n'
  printf 'SELECT subject_type, subject_id, user_id, erased_at, had_source FROM staged_erasure\n'
  printf 'ON CONFLICT (subject_type, subject_id) DO UPDATE\n'
  printf 'SET had_source = public.erasure_records.had_source OR EXCLUDED.had_source;\n'
  if [ -n "$decisions" ]; then
    while IFS="$tab" read -r decision account; do
      if [ "$decision" = erase ]; then
        # Replay deletes the account; its cascade writes the per-novel records.
        printf "INSERT INTO public.erasure_records (subject_type, subject_id, user_id)"
        printf " VALUES ('user', '%s', '%s')" "$account" "$account"
        printf " ON CONFLICT (subject_type, subject_id) DO NOTHING;\n"
      else
        awk -F'\t' -v owner="$account" '$2 == owner { print $1 }' "$work/novels.tsv" |
          while read -r novel; do
            if ! owns "$work/retained-novels.tsv" "$account" "$novel"; then
              printf "INSERT INTO public.erasure_records (subject_type, subject_id, user_id)"
              printf " VALUES ('novel', '%s', '%s')" "$novel" "$account"
              printf " ON CONFLICT (subject_type, subject_id) DO NOTHING;\n"
            fi
          done
      fi
      designated=FALSE
      [ "$account" != "$designated_admin" ] || designated=TRUE
      printf "INSERT INTO public.restore_attestations (subject_id, decision,"
      printf " window_start, window_end, artifact_inventory, operator_identity,"
      printf " designated_admin)"
      printf " VALUES ('%s', '%s', '%s', '%s', '%s', '%s', %s);\n" \
        "$account" "$decision" "$window_start" "$window_end" "$inventory" \
        "$(printf '%s' "$operator_identity" | sed "s/'/''/g")" "$designated"
    done <"$work/decided.tsv"
    # A collected record is the pre-decided fact replay enforces. The operator
    # never decides it, but the restore still records the provenance, so the
    # audit trail distinguishes a restore that carried erasures forward from a
    # genesis database.
    comm -12 "$work/account-ids.txt" "$work/erased-users.txt" | while read -r covered; do
      printf "INSERT INTO public.restore_attestations (subject_id, decision,"
      printf " window_start, window_end, artifact_inventory, operator_identity,"
      printf " designated_admin)"
      printf " VALUES ('%s', 'replayed', '%s', '%s', '%s', '%s', FALSE);\n" \
        "$covered" "$window_start" "$window_end" "$inventory" \
        "$(printf '%s' "$operator_identity" | sed "s/'/''/g")"
    done
    [ -z "$designated_admin" ] ||
      printf "UPDATE public.users SET role = 'admin' WHERE id = '%s';\n" "$designated_admin"
  fi
  # No session issued before the restore may survive it: the rotation below
  # invalidates every access token, and this removes every refresh token.
  printf 'DELETE FROM public.refresh_tokens;\n'
  printf 'COMMIT;\n'
} | docker exec -i "$container" psql -U "$pg_user" -d "$pg_db" -q -v ON_ERROR_STOP=1

stamp=$(date -u +%Y%m%dT%H%M%SZ)
cp -p "$env_file" "$env_file.pre-restore.$stamp"
chmod 600 "$env_file.pre-restore.$stamp"
awk -v secret="JWT_SECRET=$(openssl rand -hex 32)" '
  /^JWT_SECRET=/ { print secret; rotated = 1; next }
  { print }
  END { if (!rotated) print secret }
' "$env_file" >"$work/env.rotated"
cat "$work/env.rotated" >"$env_file"

missing_secrets=""
for secret in RUNTIME_CONFIG_KEY INTERNAL_SERVICE_TOKEN; do
  grep -Eq "^$secret=.+" "$env_file" || missing_secrets="$missing_secrets $secret"
done

# ─── 7. replay ─────────────────────────────────────────────────────────────
# The standard migration path replays every collected and decision-written
# erasure record before any service starts.
docker compose run --rm postgres-migrate >/dev/null

new_token=$(docker exec "$container" psql -U "$pg_user" -d "$pg_db" -At -c \
  "SELECT token || ' parent=' || COALESCE(parent::pg_catalog.text, 'absent')
     FROM public.database_lineage")
printf 'restore: complete.\n'
printf 'restore:   artifact          %s\n' "$dump_file"
printf 'restore:   lineage           %s\n' "$new_token"
printf 'restore:   covered through   %s\n' "$covered_through"
printf 'restore:   residual window   %s\n' "$window_state"
printf 'restore:   erasure sources   %s record(s) from %s\n' \
  "$(wc -l <"$work/union.tsv" | tr -d ' ')" "$inventory"
if [ -n "$decisions" ]; then
  printf 'restore:   decisions         %s retained, %s erased, operator %s\n' \
    "$(cut -f1 "$work/decided.tsv" | grep -c retain || true)" \
    "$(cut -f1 "$work/decided.tsv" | grep -c erase || true)" \
    "$operator_identity"
  [ -z "$designated_admin" ] ||
    printf 'restore:   administrator     %s promoted by decision\n' "$designated_admin"
fi
printf 'restore:   JWT_SECRET        rotated in %s (previous file kept as %s)\n' \
  "$env_file" "$env_file.pre-restore.$stamp"
printf 'restore:   refresh tokens    deleted; no pre-restore session survives\n'
if [ -n "$missing_secrets" ]; then
  printf 'restore: WARNING:%s missing from %s.\n' "$missing_secrets" "$env_file" >&2
  printf 'restore: a regenerated RUNTIME_CONFIG_KEY cannot decrypt the stored LLM key (redo first-run setup);\n' >&2
  printf 'restore: INTERNAL_SERVICE_TOKEN must be identical across every service.\n' >&2
fi
printf 'restore: start the deployment with: docker compose up -d\n'
