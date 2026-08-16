#!/usr/bin/env bash
# RTO scale rehearsal for backup-restore-v1 (docs/BACKUP_RESTORE.md).
#
# Judges the ≤ 30 minute RTO target: a scripted restore of a synthetic database
# whose plain dump is at least 5 GB, on the DEPLOY.md reference envelope
# (2-core / 4 GB / SSD), measured from starting the restore to the deployment
# passing readiness.
#
# NOT a CI drill. It costs tens of gigabytes and tens of minutes, it destroys
# the target deployment's data, and its result is release evidence recorded once
# per policy version and re-run after any change to backup.sh or restore.sh.
#
#   BACKUP_ENCRYPTION_KEY=... infra/backup/scale_rehearsal.sh [target_gb]
set -euo pipefail

container=${POSTGRES_CONTAINER:-novel-postgres}
pg_user=${POSTGRES_USER:-novel}
pg_db=${POSTGRES_DB:-novel_world}
here=$(cd "$(dirname "$0")" && pwd)
target_gb=${1:-5}
target_bytes=$((target_gb * 1024 * 1024 * 1024))

printf 'scale: DESTRUCTIVE rehearsal against %s/%s; interrupt now to abort.\n' \
  "$container" "$pg_db" >&2
sleep 5

psql() { docker exec -i "$container" psql -U "$pg_user" -d "$pg_db" -v ON_ERROR_STOP=1 -At "$@"; }

psql -c "INSERT INTO users (id, email, password_hash)
         VALUES ('00000000-0000-4000-8000-00000000ffff', 'scale@test.invalid', 'x')
         ON CONFLICT DO NOTHING" >/dev/null

# One batch is ~200 MB of chapter text: 100 novels x 20 chapters x ~100 KB.
batch=0
while [ "$(psql -c "SELECT pg_catalog.pg_database_size('$pg_db')")" -lt "$target_bytes" ]; do
  batch=$((batch + 1))
  psql -c "
    WITH new_novels AS (
        INSERT INTO novels (user_id, title, status, total_chapters)
        SELECT '00000000-0000-4000-8000-00000000ffff',
               'scale batch $batch #' || series, 'ready', 20
        FROM pg_catalog.generate_series(1, 100) AS series
        RETURNING id
    )
    INSERT INTO chapters (novel_id, chapter_number, content, word_count)
    SELECT new_novels.id, chapter,
           pg_catalog.repeat('风暴之塔的章节内容用于容量演练。', 3000), 30000
    FROM new_novels, pg_catalog.generate_series(1, 20) AS chapter" >/dev/null
  printf 'scale: batch %s, database now %s\n' "$batch" \
    "$(psql -c "SELECT pg_catalog.pg_size_pretty(pg_catalog.pg_database_size('$pg_db'))")"
done

started=$(date +%s)
"$here/backup.sh"
printf 'scale: backup took %s seconds\n' "$(($(date +%s) - started))"

manifest=$(ls -t "${BACKUP_DIR:-./backups}"/*.manifest | head -1)
printf 'scale: stop the application services, then time the restore:\n'
printf '  time BACKUP_ENCRYPTION_KEY=... %s/restore.sh --manifest %s\n' "$here" "$manifest"
printf 'scale: the RTO clock stops when docker compose up -d reports every service healthy.\n'
