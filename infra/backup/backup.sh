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
trap 'rm -rf "$work"' EXIT

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
# dump itself. An artifact taken before the erasure journal existed has no such
# block, which restore reports rather than silently treating as "no deletions".
#
# Fields are selected through the dump's own COPY header, so the sidecar is
# always the five immutable facts in a fixed order however the table is laid
# out. The lineage-local re-queue bookkeeping is deliberately not exported: a
# restore starts a new lineage.
awk '
  /^COPY public\.erasure_records \(/ {
    header = $0
    sub(/^[^(]*\(/, "", header)
    sub(/\).*$/, "", header)
    gsub(/,/, " ", header)
    count = split(header, columns, " ")
    for (i = 1; i <= count; i++) index_of[columns[i]] = i
    wanted_count = split("subject_type,subject_id,user_id,erased_at,had_source", wanted, ",")
    inside = 1
    next
  }
  inside && $0 == "\\." { inside = 0; next }
  inside {
    split($0, field, "\t")
    line = ""
    for (i = 1; i <= wanted_count; i++) {
      if (!(wanted[i] in index_of)) exit 3
      line = line (i > 1 ? "\t" : "") field[index_of[wanted[i]]]
    }
    print line
  }
' "$work/dump.sql" >"$work/erasure.tsv" ||
  fail 'the erasure export in this dump is missing a required column'
if ! grep -q '^COPY public\.erasure_records (' "$work/dump.sql"; then
  printf 'backup: warning: this database has no erasure_records table; the artifact carries no erasure source\n' >&2
fi

encrypt() {
  gzip -9 -c "$1" |
    openssl enc -aes-256-cbc -pbkdf2 -iter 200000 -salt -pass env:BACKUP_ENCRYPTION_KEY \
      -out "$2"
}
encrypt "$work/dump.sql" "$base.dump.gz.enc"
encrypt "$work/erasure.tsv" "$base.erasure.tsv.gz.enc"

{
  printf 'schema=backup-artifact-v1\n'
  printf 'covered_through=%s\n' "$covered_through"
  printf 'database=%s\n' "$pg_db"
  printf 'dump=%s\n' "$(basename "$base.dump.gz.enc")"
  printf 'dump_sha256=%s\n' "$(sha256sum "$base.dump.gz.enc" | cut -d' ' -f1)"
  printf 'erasure=%s\n' "$(basename "$base.erasure.tsv.gz.enc")"
  printf 'erasure_sha256=%s\n' "$(sha256sum "$base.erasure.tsv.gz.enc" | cut -d' ' -f1)"
} >"$base.manifest"

printf 'backup: wrote %s.manifest (covered through %s, %s erasure records)\n' \
  "$base" "$covered_through" "$(wc -l <"$work/erasure.tsv" | tr -d ' ')"
