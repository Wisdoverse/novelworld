#!/usr/bin/env bash
# NovelWorld scripted backup — backup-restore-v1 (docs/BACKUP_RESTORE.md).
#
# Produces, in one run:
#   <base>.dump.gz.enc          the whole authoritative database
#   <base>.erasure.tsv.gz.enc   the erasure records of the SAME snapshot
#   <base>.manifest             SHA-256 of both files and the covered-through
#                               timestamp
#
# pg_dump runs inside the pinned PostgreSQL container, so client and server
# versions can never skew. The erasure export is cut out of that one dump rather
# than queried separately, which is the only way to guarantee that the export's
# coverage and the dump's contents come from the same snapshot.
#
# Scheduling, off-host copies, and retention (30 days default, operator override
# bounded to 7–90 days) are operator duties. This script does none of them.
set -euo pipefail

container=${POSTGRES_CONTAINER:-novel-postgres}
pg_user=${POSTGRES_USER:-novel}
pg_db=${POSTGRES_DB:-novel_world}
backup_dir=${BACKUP_DIR:-./backups}

fail() {
  printf 'backup: %s\n' "$1" >&2
  exit 1
}

[ -n "${BACKUP_ENCRYPTION_KEY:-}" ] ||
  fail 'BACKUP_ENCRYPTION_KEY is required; an unencrypted artifact is non-conformant'
[ "${#BACKUP_ENCRYPTION_KEY}" -ge 32 ] ||
  fail 'BACKUP_ENCRYPTION_KEY must be at least 32 characters'
docker inspect --format '{{.State.Running}}' "$container" 2>/dev/null | grep -qx true ||
  fail "postgres container '$container' is not running (set POSTGRES_CONTAINER)"

mkdir -p "$backup_dir"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
base=$backup_dir/novelworld-$stamp
work=$(mktemp -d)
trap 'rm -rf "$work" "$base".*.partial' EXIT

# The covered-through timestamp is read from the database immediately BEFORE
# pg_dump opens its snapshot, so it is always slightly EARLIER than the snapshot
# the artifact actually contains. A restore derives the residual window from it,
# and an earlier start only ever makes that window a superset of the true one —
# the safe direction. Reading it afterwards, or from the host clock, could
# understate the window and silently drop deletions.
covered_through=$(docker exec "$container" psql -U "$pg_user" -d "$pg_db" -At -c \
  "SELECT pg_catalog.to_char(pg_catalog.now() AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US+00')")
[ -n "$covered_through" ] || fail 'could not read the covered-through timestamp'

docker exec "$container" pg_dump -U "$pg_user" -d "$pg_db" \
  --format=plain --no-owner --no-privileges >"$work/dump.sql"

# One dump, two artifacts: the sidecar is the erasure_records COPY block of the
# dump itself, so the export and the dump can never come from different
# snapshots. Fields are selected through the dump's own COPY header — validated
# even when the table is empty — so the sidecar is always the five immutable
# facts in a fixed order however the table is laid out.
#
# source_requeued_at is deliberately not exported. It is lineage-local, and a
# restored dump keeps its own stamps because stamp and outbox row are written in
# one transaction that a single-snapshot dump preserves: either both are in the
# artifact, or the object was already deleted.
grep -q '^COPY public\.erasure_records (' "$work/dump.sql" ||
  fail 'this database has no erasure_records journal; apply the migrations before backing it up'
grep -q '^COPY public\.database_lineage (' "$work/dump.sql" ||
  fail 'this database has no lineage token; apply the migrations before backing it up'
awk '
  /^COPY public\.erasure_records \(/ {
    header = $0
    sub(/^[^(]*\(/, "", header)
    sub(/\).*$/, "", header)
    gsub(/,/, " ", header)
    count = split(header, columns, " ")
    for (i = 1; i <= count; i++) index_of[columns[i]] = i
    wanted_count = split("subject_type,subject_id,user_id,erased_at,had_source", wanted, ",")
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
' "$work/dump.sql" >"$work/erasure.tsv" ||
  fail 'the erasure journal in this dump is missing a required column'

# Everything is built under temporary names and renamed into place only once all
# three outputs exist, so a failed run never leaves a final-named artifact that
# a restore could pick up.
encrypt() {
  gzip -9 -c "$1" |
    openssl enc -aes-256-cbc -pbkdf2 -iter 200000 -salt -pass env:BACKUP_ENCRYPTION_KEY \
      -out "$2"
}
# The manifest token is read out of the dump itself, so manifest and dump agree
# by construction; the restore still compares them, because an artifact can be
# edited after it is written.
lineage_token=$(awk '
  /^COPY public\.database_lineage \(/ {
    header = $0
    sub(/^[^(]*\(/, "", header)
    sub(/\).*$/, "", header)
    gsub(/,/, " ", header)
    count = split(header, columns, " ")
    for (i = 1; i <= count; i++) index_of[columns[i]] = i
    if (!("token" in index_of)) exit 3
    inside = 1
    next
  }
  inside && $0 == "\\." { inside = 0; next }
  inside { print $index_of["token"]; exit }
' "$work/dump.sql") || fail 'the lineage table in this dump has no token column'
[ -n "$lineage_token" ] || fail 'this database has no lineage token row; apply the migrations before backing it up'

encrypt "$work/dump.sql" "$base.dump.gz.enc.partial"
encrypt "$work/erasure.tsv" "$base.erasure.tsv.gz.enc.partial"

{
  printf 'schema=backup-artifact-v1\n'
  printf 'covered_through=%s\n' "$covered_through"
  printf 'lineage_token=%s\n' "$lineage_token"
  printf 'database=%s\n' "$pg_db"
  printf 'dump=%s\n' "$(basename "$base.dump.gz.enc")"
  printf 'dump_sha256=%s\n' "$(sha256sum "$base.dump.gz.enc.partial" | cut -d' ' -f1)"
  printf 'erasure=%s\n' "$(basename "$base.erasure.tsv.gz.enc")"
  printf 'erasure_sha256=%s\n' "$(sha256sum "$base.erasure.tsv.gz.enc.partial" | cut -d' ' -f1)"
} >"$base.manifest.partial"

mv "$base.dump.gz.enc.partial" "$base.dump.gz.enc"
mv "$base.erasure.tsv.gz.enc.partial" "$base.erasure.tsv.gz.enc"
mv "$base.manifest.partial" "$base.manifest"

printf 'backup: wrote %s.manifest (lineage %s, covered through %s, %s erasure records)\n' \
  "$base" "$lineage_token" "$covered_through" "$(wc -l <"$work/erasure.tsv" | tr -d ' ')"
