#!/usr/bin/env bash
# Opt-in synthetic measurement for backup-restore-v2. See docs/BACKUP_RESTORE.md.
# Reuses the production backup/restore procedures; never run against user data.
# Docker discovery: 30s per call; fixture statements: 10m; backup/size scan: 1h;
# restore plus startup/readiness: 30m. No automatic retry of side effects.
set -euo pipefail

fail() { printf 'scale: %s\n' "$1" >&2; exit 1; }
here=$(cd "$(dirname "$0")" && pwd)
repo_root=$(cd "$here/../.." && pwd)
container=${POSTGRES_CONTAINER:-}
env_file=''
confirmation=''
target_gb=5
while [ "$#" -gt 0 ]; do
  case "$1" in
    --env-file|--confirm-target|--target-gb)
      [ "$#" -ge 2 ] || fail "missing value for $1"
      case "$1" in
        --env-file) env_file=$2 ;;
        --confirm-target) confirmation=$2 ;;
        --target-gb) target_gb=$2 ;;
      esac
      shift 2 ;;
    *) fail 'usage: scale_rehearsal.sh --env-file FILE --confirm-target CONTAINER [--target-gb N]' ;;
  esac
done
[[ "$container" =~ ^[a-zA-Z0-9][a-zA-Z0-9_.-]*$ ]] ||
  fail 'set POSTGRES_CONTAINER explicitly to the dedicated synthetic PostgreSQL container'
[ "$confirmation" = "$container" ] || fail '--confirm-target must equal POSTGRES_CONTAINER'
[[ "$target_gb" =~ ^[1-9][0-9]{0,2}$ ]] && [ "$target_gb" -ge 5 ] ||
  fail '--target-gb must be an integer from 5 to 999 (GiB)'
[ -f "$env_file" ] || fail '--env-file must name the preserved deployment environment'
env_file=$(realpath "$env_file")
[ -n "${BACKUP_ENCRYPTION_KEY:-}" ] || fail 'BACKUP_ENCRYPTION_KEY is required'
[ "${#BACKUP_ENCRYPTION_KEY}" -ge 32 ] ||
  fail 'BACKUP_ENCRYPTION_KEY must be at least 32 characters'
target_bytes=$((target_gb * 1024 * 1024 * 1024))
cd "$repo_root"
compose=(docker compose --env-file "$env_file")

# Read only non-secret fields; config loads the selected files, unlike compose ps.
connection=$(timeout 30s "${compose[@]}" config --format json | python3 -c '
import json, sys
config = json.load(sys.stdin)
env = config["services"]["postgres"]["environment"]
for value in (env["POSTGRES_USER"], env["POSTGRES_DB"]):
    if not isinstance(value, str) or not value or "\n" in value:
        raise SystemExit("invalid PostgreSQL connection identity")
    print(value)
') || fail 'cannot resolve the selected Compose configuration'
mapfile -t connection_fields <<<"$connection"
[ "${#connection_fields[@]}" -eq 2 ] || fail 'invalid PostgreSQL connection identity'
export POSTGRES_USER=${connection_fields[0]} POSTGRES_DB=${connection_fields[1]}
export POSTGRES_CONTAINER="$container"
selected=$(timeout 30s "${compose[@]}" ps --all --quiet postgres) || fail 'cannot inspect Compose postgres'
case "$selected" in ''|*$'\n'*) fail 'select exactly one Compose postgres container' ;; esac
selected_id=$(timeout 30s docker inspect --format '{{.Id}}' "$selected") || fail 'cannot inspect selected postgres'
target_id=$(timeout 30s docker inspect --format '{{.Id}}' "$container") || fail 'cannot inspect POSTGRES_CONTAINER'
[[ "$target_id" =~ ^[0-9a-f]{64}$ ]] && [ "$target_id" = "$selected_id" ] || fail 'PostgreSQL target mismatch'
[ "$(timeout 30s docker inspect --format '{{.State.Health.Status}}' "$container")" = healthy ] ||
  fail 'PostgreSQL must already be healthy and migrated'
# Reject a changed .env that no longer describes the database's initialized identity.
# shellcheck disable=SC2016 # Expand the initialized values inside the container.
actual_connection=$(timeout 30s docker exec "$container" sh -c 'printf "%s\n%s\n" "$POSTGRES_USER" "$POSTGRES_DB"') ||
  fail 'cannot verify the initialized PostgreSQL identity'
[ "$connection" = "$actual_connection" ] || fail 'Compose and initialized PostgreSQL connection identities differ'
writers=$(timeout 30s "${compose[@]}" ps --all --quiet gateway user-service novel-service agent-service narrative-service postgres-migrate) ||
  fail 'cannot inspect application writers'
while IFS= read -r writer; do
  [ -n "$writer" ] || continue
  state=$(timeout 30s docker inspect --format '{{.State.Status}}' "$writer") || fail 'cannot inspect writer state'
  case "$state" in created|exited|dead) ;; *) fail "keep all application writers stopped; observed $state" ;; esac
done <<<"$writers"

psql() {
  timeout --kill-after=30s 630s docker exec -i -e PGOPTIONS='-c statement_timeout=600000' "$container" \
    psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -v ON_ERROR_STOP=1 -At "$@"
}
[ "$(psql -c 'SELECT NOT EXISTS (SELECT 1 FROM users) AND NOT EXISTS (SELECT 1 FROM novels)')" = t ] ||
  fail 'refusing to seed a database that already contains users or novels'
# Fail before seeding when the host-side plain dump cannot fit. This is a space
# preflight, not a reservation; PostgreSQL storage also needs operator headroom.
export TMPDIR=${TMPDIR:-/tmp}
[ -d "$TMPDIR" ] || fail 'TMPDIR must exist'
free_kib=$(df -Pk "$TMPDIR" | awk 'NR == 2 {print $4}')
if ! [[ "$free_kib" =~ ^[0-9]+$ ]] || ((free_kib * 1024 < 2 * target_bytes)); then
  fail 'TMPDIR needs at least twice the requested plain size free; use a persistent filesystem'
fi
backup_root=${BACKUP_DIR:-$repo_root/backups}
mkdir -p "$backup_root"
export BACKUP_DIR
BACKUP_DIR=$(mktemp -d "$backup_root/scale-XXXXXXXX")
report=$BACKUP_DIR/measurement.json
stage=fixture
started_ns=''
finished_ns=''
plain_bytes=0
physical_bytes=0
encrypted_bytes=0
manifest=''
status=failed
started_at=''
finish() {
  local rc=$?
  trap - EXIT
  python3 - "$report" "$status" "$stage" "$started_ns" "$finished_ns" "$started_at" "$plain_bytes" "$physical_bytes" "$encrypted_bytes" "$manifest" "$repo_root" <<'PY'
import datetime, hashlib, json, os, pathlib, subprocess, sys, time
report, status, stage, started, finished, started_at, plain, physical, encrypted, manifest, root = sys.argv[1:]
root = pathlib.Path(root)
elapsed = ((int(finished) if finished else time.monotonic_ns()) - int(started)) / 1e9 if started else None
files = ("infra/backup/scale_rehearsal.sh", "infra/backup/backup.sh", "infra/backup/restore.sh")
result = {
    "schema": "backup-scale-measurement-v1", "policy": "backup-restore-v2",
    "status": status, "stage": stage, "started_at": started_at or None,
    "finished_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "restore_to_readiness_seconds": elapsed,
    "plain_dump_bytes": int(plain), "database_physical_bytes": int(physical),
    "encrypted_dump_bytes": int(encrypted),
    "manifest_sha256": hashlib.sha256(pathlib.Path(manifest).read_bytes()).hexdigest() if manifest else None,
    "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
    "script_sha256": {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in files},
    "fixture": "legacy repeated chapter text; highly compressible",
    "host_logical_cpu_count": os.cpu_count(),
    "host_memory_bytes": os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES"),
    "reference_hardware_verified": False,
    "qualification": "requires separate 2-core / 4 GB / SSD environment evidence and review",
}
pathlib.Path(report).write_text(json.dumps(result, indent=2) + "\n")
PY
  printf 'scale: %s at %s; measurement and artifacts retained in %s\n' "$status" "$stage" "$BACKUP_DIR" >&2
  exit "$rc"
}
trap finish EXIT

# Retain the existing synthetic text. Bound generation by logical bytes instead
# of TOAST/compressed physical storage, then verify the actual artifact below.
batch_bytes=$(psql -c "SELECT pg_catalog.octet_length(pg_catalog.repeat('风暴之塔的章节内容用于容量演练。', 3000))::bigint * 100 * 20")
[[ "$batch_bytes" =~ ^[1-9][0-9]*$ ]] || fail 'cannot determine fixture batch size'
batches=$(((target_bytes + batch_bytes - 1) / batch_bytes))
psql -c "INSERT INTO users (id, email, password_hash)
  VALUES ('00000000-0000-4000-8000-00000000ffff', 'scale@test.invalid', 'x')" >/dev/null
for ((batch = 1; batch <= batches; batch++)); do
  inserted=$(psql -c "
    WITH new_novels AS (
      INSERT INTO novels (user_id, title, status, total_chapters)
      SELECT '00000000-0000-4000-8000-00000000ffff',
        'scale batch $batch #' || series, 'ready', 20
      FROM pg_catalog.generate_series(1, 100) AS series RETURNING id
    ), new_chapters AS (
      INSERT INTO chapters (novel_id, chapter_number, content, word_count)
      SELECT new_novels.id, chapter,
        pg_catalog.repeat('风暴之塔的章节内容用于容量演练。', 3000), 30000
      FROM new_novels, pg_catalog.generate_series(1, 20) AS chapter
      RETURNING pg_catalog.octet_length(content)::bigint AS content_bytes
    ) SELECT pg_catalog.sum(content_bytes) FROM new_chapters")
  [ "$inserted" = "$batch_bytes" ] || fail 'fixture batch did not insert the expected logical bytes'
  printf 'scale: fixture batch %s/%s\n' "$batch" "$batches"
done
physical_bytes=$(psql -c 'SELECT pg_catalog.pg_database_size(pg_catalog.current_database())')
[[ "$physical_bytes" =~ ^[0-9]+$ ]] || fail 'cannot measure physical database size'
stage=backup
timeout --kill-after=30s 3600s "$here/backup.sh"
shopt -s nullglob
manifests=("$BACKUP_DIR"/*.manifest)
[ "${#manifests[@]}" -eq 1 ] || fail 'backup must produce exactly one manifest in this invocation directory'
manifest=${manifests[0]}
dump=${manifest%.manifest}.dump.gz.enc
[ -f "$dump" ] || fail 'backup dump is missing'
stage=artifact_size
plain_bytes=$(timeout --kill-after=30s 3600s openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 -pass env:BACKUP_ENCRYPTION_KEY -in "$dump" |
  timeout --kill-after=30s 3600s gzip -dc | timeout --kill-after=30s 3600s wc -c) ||
  fail 'cannot decrypt and count the backup artifact'
plain_bytes=${plain_bytes//[[:space:]]/}
if ! [[ "$plain_bytes" =~ ^[0-9]+$ ]] || ((plain_bytes < target_bytes)); then
  fail 'actual plain dump is smaller than the requested size'
fi
encrypted_bytes=$(wc -c <"$dump")
encrypted_bytes=${encrypted_bytes//[[:space:]]/}

# Verification/decryption runs again inside restore and IS inside this clock.
# The artifact-size inspection above cannot replace any production restore step.
stage=restore
started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
started_ns=$(python3 -c 'import time; print(time.monotonic_ns())')
timeout --kill-after=30s 1800s "$here/restore.sh" --manifest "$manifest" --env-file "$env_file"
remaining=$(python3 - "$started_ns" <<'PY'
import math, sys, time
print(math.floor(1800 - (time.monotonic_ns() - int(sys.argv[1])) / 1e9))
PY
)
((remaining > 0)) || fail 'restore exceeded the 30-minute criterion'
stage=readiness
# Shell environment overrides --env-file; remove the stale pre-restore JWT.
unset JWT_SECRET
timeout --kill-after=30s "${remaining}s" "${compose[@]}" up -d --no-build --wait --wait-timeout "$remaining"
finished_ns=$(python3 - "$started_ns" <<'PY'
import sys, time
finished = time.monotonic_ns()
if (finished - int(sys.argv[1])) / 1e9 > 1800:
    raise SystemExit("scale: restore-to-readiness exceeded 30 minutes")
print(finished)
PY
)
stage=complete
status=passed
