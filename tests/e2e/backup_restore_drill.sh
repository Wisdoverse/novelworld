#!/usr/bin/env bash
# Drills A, B and C plus the negative cases of backup-restore-v1
# (docs/BACKUP_RESTORE.md, "Drills"), against the supported compose topology.
#
# Runs after tests/e2e/core_reader_loop.sh, which leaves the deployment in
# first-run state, and seeds its own drill dataset: two accounts, three novels
# with at least two durable chapters each, two retained-source keys, committed
# chat history and a committed world turn.
#
# Retained-source coverage limit: the end-to-end topology has no S3 stub and
# runs with S3_ENABLED=false, where novel-service refuses to start while the
# deletion outbox is non-empty. The drill therefore asserts the retained-source
# re-queue on the database (the outbox rows and their per-record bookkeeping)
# and drains the outbox — exactly what the S3 cleanup worker would do — before
# services start. Object-level deletion is covered by novel-service's own tests.
set -euo pipefail
cd "$(dirname "$0")/../.."

api=${E2E_API_URL:-http://127.0.0.1/api}
stub=${E2E_LLM_STUB_URL:-http://127.0.0.1:18080}
password='RuntimeSmokeOnly123!'
admin_email=drill-admin@test.invalid
reader_email=drill-reader@test.invalid

export COMPOSE_FILE=${COMPOSE_FILE:-docker-compose.yml:docker-compose.e2e.yml}
export BACKUP_ENCRYPTION_KEY=${BACKUP_ENCRYPTION_KEY:-drill-backup-key-at-least-32-characters-long}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export BACKUP_DIR=$work/backups
env_file=$work/.env
{
  printf 'JWT_SECRET=%s\n' "${JWT_SECRET:?JWT_SECRET is required}"
  printf 'RUNTIME_CONFIG_KEY=%s\n' "${RUNTIME_CONFIG_KEY:-}"
  printf 'INTERNAL_SERVICE_TOKEN=%s\n' "${INTERNAL_SERVICE_TOKEN:-}"
} >"$env_file"

curl_cmd=(curl --connect-timeout 5 --max-time 120 --fail --silent --show-error)

json_get() { python3 -c "import json,sys; value=json.load(sys.stdin); print($1)"; }
pause() { sleep 1.1; }

psql() {
  docker exec -i novel-postgres psql -U "${POSTGRES_USER:-novel}" \
    -d "${POSTGRES_DB:-novel_world}" -At -v ON_ERROR_STOP=1 "$@"
}

check() {
  if [ "$2" != "$3" ]; then
    printf 'drill: FAIL %s: expected [%s], got [%s]\n' "$1" "$2" "$3" >&2
    exit 1
  fi
  printf 'drill: ok   %s = %s\n' "$1" "$3"
}

refuses() {
  local label=$1 status=0
  shift
  set +e
  "$@" >"$work/refusal.txt" 2>&1
  status=$?
  set -e
  if [ "$status" -eq 0 ]; then
    printf 'drill: FAIL %s: the restore completed but had to refuse\n' "$label" >&2
    cat "$work/refusal.txt" >&2
    exit 1
  fi
  printf 'drill: ok   %s refused (exit %s)\n' "$label" "$status"
}

refusal_says() {
  if ! grep -Fq "$1" "$work/refusal.txt"; then
    printf 'drill: FAIL refusal did not mention [%s]:\n' "$1" >&2
    cat "$work/refusal.txt" >&2
    exit 1
  fi
}

wait_healthy() {
  for _ in $(seq 1 90); do
    if [ "$(docker inspect --format '{{.State.Health.Status}}' novel-gateway 2>/dev/null)" = healthy ]; then
      return 0
    fi
    sleep 2
  done
  docker compose ps --all >&2
  printf 'drill: the topology never became healthy\n' >&2
  exit 1
}

http_status() {
  pause
  curl --connect-timeout 5 --max-time 120 --silent --output /dev/null \
    --write-out '%{http_code}' "$@"
}

stub_calls() {
  curl --silent "$stub/__control__/stats" |
    python3 -c 'import json,sys; print(sum(json.load(sys.stdin)["calls"].values()))'
}

# The S3 cleanup worker owns the outbox in a deployment that enables S3; with S3
# disabled the drill drains it so novel-service can start (see the header note).
drain_outbox() {
  psql -c "DELETE FROM source_file_deletions" >/dev/null
  psql -c "UPDATE novels SET original_file_key = NULL WHERE original_file_key LIKE 'source-files/%'" >/dev/null
}

set_source_keys() {
  psql -c "UPDATE novels SET original_file_key = 'source-files/' || user_id || '/' || id
             WHERE id IN ('$novel_a2', '$novel_b1')" >/dev/null
}

# ─── Phase 0: seed the drill dataset ───────────────────────────────────────

printf 'drill: seeding the drill dataset\n'
for _ in 1 2 3; do
  if "${curl_cmd[@]}" --output /dev/null -X POST -H 'Content-Type: application/json' \
    --data '{}' "$stub/__control__/reset"; then
    break
  fi
  sleep 2
done

source_file=$work/novel.txt
printf '%s\n' \
  '第一章 风暴前夜' \
  '林岚握紧手中的旧地图，望向被风暴笼罩的北塔，决定在天黑前寻找失踪的守门人。边城的钟声连续响了三次，街道上的人们纷纷关紧门窗。林岚仍站在石桥中央，逐一核对地图上的暗号，并请你留意河岸新出现的足迹。远处的塔灯忽明忽暗，仿佛有人正用最后的力气发出求救信号。你们约定不替彼此作决定，却要共同承担进入风暴的后果。' \
  '第二章 北塔回声' \
  '北塔的石门布满潮湿苔痕，林岚在门边发现守门人留下的铜铃。铃身刻着通往地下回廊的路线，也写明只有彼此信任的同行者才能安全通过。风暴压低了天空，城墙上的火把依次熄灭。你与林岚交换各自找到的线索，确认失踪并非意外。塔内传来沉重脚步，旧地图上从未标注的房间正在缓缓开启，而边城的命运也随这一刻发生变化。' \
  >"$source_file"

pause
setup=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$admin_email\",\"password\":\"$password\",\"name\":\"Drill Admin\"}" \
  "$api/setup/init")
admin_token=$(json_get "value['access_token']" <<<"$setup")
admin_id=$(json_get "value['user']['id']" <<<"$setup")
admin_auth=(-H "Authorization: Bearer $admin_token")

pause
reader=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$reader_email\",\"password\":\"$password\",\"name\":\"Drill Reader\"}" \
  "$api/auth/register")
reader_id=$(json_get "value['user']['id']" <<<"$reader")
reader_refresh=$(json_get "value['refresh_token']" <<<"$reader")

pause
upload=$("${curl_cmd[@]}" "${admin_auth[@]}" \
  -F 'title=风暴之塔' -F 'author=Drill' -F 'deviation_mode=creative' \
  -F "file=@$source_file;filename=storm.txt;type=text/plain" \
  "$api/novels/upload")
novel_a1=$(json_get "value['novel_id']" <<<"$upload")
for _ in $(seq 1 60); do
  sleep 2
  state=$(json_get "value['status']" <<<"$("${curl_cmd[@]}" "${admin_auth[@]}" \
    "$api/novels/$novel_a1/status")")
  [ "$state" = ready ] && break
  [ "$state" = error ] && { printf 'drill: import failed\n' >&2; exit 1; }
done
check 'imported novel status' ready "$state"

pause
"${curl_cmd[@]}" --output /dev/null "${admin_auth[@]}" -X PUT \
  -H 'Content-Type: application/json' --data '{"current_chapter":2}' "$api/progress/$novel_a1"
pause
entry=$("${curl_cmd[@]}" "${admin_auth[@]}" \
  "$api/narrative/$novel_a1/player-entry?checkpoint_chapter=1")
location_id=$(json_get "value['locations'][0]['id']" <<<"$entry")
pause
"${curl_cmd[@]}" --output /dev/null "${admin_auth[@]}" -X PUT \
  -H 'Content-Type: application/json' \
  --data "{\"checkpoint_chapter\":1,\"name\":\"云舟\",\"background\":\"来自边城的地图学徒。\",\"capabilities\":[\"辨认古地图\"],\"location_id\":\"$location_id\",\"inventory\":[\"旧地图\"]}" \
  "$api/narrative/$novel_a1/player-entry"
pause
characters=$("${curl_cmd[@]}" "${admin_auth[@]}" "$api/novels/$novel_a1/characters")
character_id=$(json_get "value[0]['id']" <<<"$characters")
pause
node_id=$(json_get "value['id']" <<<"$("${curl_cmd[@]}" "${admin_auth[@]}" "$api/narrative/$novel_a1/1")")
pause
"${curl_cmd[@]}" --output /dev/null "${admin_auth[@]}" -H 'Content-Type: application/json' \
  --data "{\"novel_id\":\"$novel_a1\",\"node_id\":\"$node_id\",\"choice_index\":0}" \
  "$api/narrative/choose"
pause
"${curl_cmd[@]}" --output /dev/null "${admin_auth[@]}" -X POST "$api/narrative/$novel_a1/world"
pause
"${curl_cmd[@]}" --output /dev/null "${admin_auth[@]}" \
  -H 'Content-Type: application/json' \
  -H "Idempotency-Key: $(python3 -c 'import uuid; print(uuid.uuid4())')" \
  --data '{"kind":"pursue_goal","target_id":null,"intent":"绘制地下回廊并寻找守门人的踪迹"}' \
  "$api/narrative/$novel_a1/world/turns"
pause
chat=$("${curl_cmd[@]}" --no-buffer "${admin_auth[@]}" \
  -H 'Content-Type: application/json' \
  -H "Idempotency-Key: $(python3 -c 'import uuid; print(uuid.uuid4())')" \
  --data "{\"message\":\"你还记得我吗？\",\"novel_id\":\"$novel_a1\"}" \
  "$api/chat/$character_id/stream")
grep -Fq 'event: done' <<<"$chat"

# Two more novels with durable chapters: one more for the admin (deleted
# directly in drill B) and one for the reader (removed by the account cascade).
novel_a2=$(python3 -c 'import uuid; print(uuid.uuid4())')
novel_b1=$(python3 -c 'import uuid; print(uuid.uuid4())')
psql -c "
  INSERT INTO novels (id, user_id, title, status, total_chapters)
  VALUES ('$novel_a2', '$admin_id', 'Drill novel A2', 'ready', 2),
         ('$novel_b1', '$reader_id', 'Drill novel B1', 'ready', 2);
  INSERT INTO chapters (novel_id, chapter_number, content)
  VALUES ('$novel_a2', 1, 'A2 首章的持久内容'), ('$novel_a2', 2, 'A2 次章的持久内容'),
         ('$novel_b1', 1, 'B1 首章的持久内容'), ('$novel_b1', 2, 'B1 次章的持久内容');
  INSERT INTO characters (novel_id, name) VALUES ('$novel_b1', '守门人');
  INSERT INTO world_states (user_id, novel_id) VALUES ('$reader_id', '$novel_b1');" >/dev/null

check 'dataset accounts' 2 "$(psql -c "SELECT COUNT(*) FROM users")"
check 'dataset novels' 3 "$(psql -c "SELECT COUNT(*) FROM novels")"
check 'dataset chapters' 6 "$(psql -c "SELECT COUNT(*) FROM chapters")"
check 'dataset chat messages' 2 "$(psql -c "SELECT COUNT(*) FROM chat_messages")"
check 'dataset world turns' 1 \
  "$(psql -c "SELECT COUNT(*) FROM world_turns WHERE status = 'completed'")"

# Retained-source keys, set after the services started: novel-service refuses to
# start with S3 disabled while any source key or outbox row exists.
set_source_keys
check 'retained source keys' 2 \
  "$(psql -c "SELECT COUNT(*) FROM novels WHERE original_file_key LIKE 'source-files/%'")"

# ─── Backup ────────────────────────────────────────────────────────────────

infra/backup/backup.sh
manifest_one=$(ls -t "$BACKUP_DIR"/*.manifest | head -1)
artifact_one=$(dirname "$manifest_one")/$(awk -F= '$1 == "dump" {print $2}' "$manifest_one")
covered_one=$(awk -F= '$1 == "covered_through" { sub(/^[^=]*=/, ""); print }' "$manifest_one")
[ -n "$covered_one" ]
check 'artifact is encrypted' 1 \
  "$(head -c 8 "$artifact_one" | grep -c Salted || true)"
if grep -aq 'CREATE TABLE' "$artifact_one"; then
  printf 'drill: FAIL the artifact contains plaintext SQL\n' >&2
  exit 1
fi

# ─── Negative cases (nothing may change before verification passes) ────────

before_negatives=$(psql -c "SELECT (SELECT COUNT(*) FROM users) || ':' || (SELECT COUNT(*) FROM novels)")
mkdir -p "$work/corrupt"
cp "$BACKUP_DIR"/* "$work/corrupt/"
corrupt_manifest=$work/corrupt/$(basename "$manifest_one")
corrupt_dump=$work/corrupt/$(basename "$artifact_one")
printf 'tamper' | dd of="$corrupt_dump" bs=1 seek=64 conv=notrunc status=none
refuses 'corrupted artifact' infra/backup/restore.sh --manifest "$corrupt_manifest" \
  --env-file "$env_file"
refusal_says 'checksum mismatch'
refuses 'wrong encryption key' env BACKUP_ENCRYPTION_KEY=another-key-of-at-least-32-characters \
  infra/backup/restore.sh --manifest "$manifest_one" --env-file "$env_file"
refusal_says 'cannot decrypt'
check 'negatives changed nothing' "$before_negatives" \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM users) || ':' || (SELECT COUNT(*) FROM novels)")"

# ─── Drill A: backup → erase → fresh-host restore ──────────────────────────

printf 'drill: A — destroying the deployment including volumes\n'
sampled_before=$(psql -c "
  SELECT (SELECT COUNT(*) FROM users) || ':' || (SELECT COUNT(*) FROM novels) || ':' ||
         (SELECT COUNT(*) FROM chapters) || ':' || (SELECT COUNT(*) FROM characters) || ':' ||
         (SELECT COUNT(*) FROM chat_messages) || ':' || (SELECT COUNT(*) FROM world_turns) || ':' ||
         (SELECT COUNT(*) FROM reading_progress) || ':' ||
         (SELECT COUNT(*) FROM novels WHERE original_file_key LIKE 'source-files/%')")
stale_token=$admin_token
docker compose down -v >/dev/null 2>&1
docker compose up -d postgres >/dev/null 2>&1
for _ in $(seq 1 60); do
  docker exec novel-postgres pg_isready -U "${POSTGRES_USER:-novel}" >/dev/null 2>&1 && break
  sleep 2
done

cat >"$work/decisions-retain-all" <<EOF
operator=backup-restore-v1 drill A
retain $admin_id novels=$novel_a1,$novel_a2
retain $reader_id novels=$novel_b1
EOF
restore_started=$(date +%s)
infra/backup/restore.sh --manifest "$manifest_one" --decisions "$work/decisions-retain-all" \
  --env-file "$env_file" --i-stopped-writes
restore_seconds=$(($(date +%s) - restore_started))
printf 'drill: A — scripted restore took %s seconds\n' "$restore_seconds"
if [ "$restore_seconds" -gt 600 ]; then
  printf 'drill: FAIL the restore exceeded the 10 minute drill bound\n' >&2
  exit 1
fi
check 'A sampled rows survived' "$sampled_before" "$(psql -c "
  SELECT (SELECT COUNT(*) FROM users) || ':' || (SELECT COUNT(*) FROM novels) || ':' ||
         (SELECT COUNT(*) FROM chapters) || ':' || (SELECT COUNT(*) FROM characters) || ':' ||
         (SELECT COUNT(*) FROM chat_messages) || ':' || (SELECT COUNT(*) FROM world_turns) || ':' ||
         (SELECT COUNT(*) FROM reading_progress) || ':' ||
         (SELECT COUNT(*) FROM novels WHERE original_file_key LIKE 'source-files/%')")"
check 'A retained sources survived' \
  "source-files/$admin_id/$novel_a2" \
  "$(psql -c "SELECT original_file_key FROM novels WHERE id = '$novel_a2'")"
check 'A attestations' 'retain:2' \
  "$(psql -c "SELECT decision || ':' || COUNT(*) FROM restore_attestations GROUP BY decision")"
check 'A refresh tokens cleared' 0 "$(psql -c "SELECT COUNT(*) FROM refresh_tokens")"

JWT_SECRET=$(awk -F= '$1 == "JWT_SECRET" { print $2 }' "$env_file")
export JWT_SECRET
drain_outbox
docker compose up -d >/dev/null 2>&1
wait_healthy

check 'A pre-restore access token rejected' 401 \
  "$(http_status -H "Authorization: Bearer $stale_token" "$api/auth/me")"
pause
login=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$admin_email\",\"password\":\"$password\"}" "$api/auth/login")
admin_token=$(json_get "value['access_token']" <<<"$login")
admin_auth=(-H "Authorization: Bearer $admin_token")
check 'A pre-restore refresh token rejected' 401 \
  "$(http_status -H 'Content-Type: application/json' \
    --data "{\"refresh_token\":\"$reader_refresh\"}" "$api/auth/refresh")"

pause
chapters=$("${curl_cmd[@]}" "${admin_auth[@]}" "$api/novels/$novel_a1/chapters")
check 'A journey chapters' 2 "$(json_get 'len(value)' <<<"$chapters")"
pause
history=$("${curl_cmd[@]}" "${admin_auth[@]}" "$api/chat/$character_id/history")
check 'A journey chat history' 2 "$(json_get "value['count']" <<<"$history")"
pause
world=$("${curl_cmd[@]}" "${admin_auth[@]}" "$api/narrative/$novel_a1/world")
check 'A journey world turn' 1 "$(json_get "value['session']['turn_number']" <<<"$world")"
pause
resumed_chat=$("${curl_cmd[@]}" --no-buffer "${admin_auth[@]}" \
  -H 'Content-Type: application/json' \
  -H "Idempotency-Key: $(python3 -c 'import uuid; print(uuid.uuid4())')" \
  --data "{\"message\":\"我们继续吧。\",\"novel_id\":\"$novel_a1\"}" \
  "$api/chat/$character_id/stream")
grep -Fq 'event: done' <<<"$resumed_chat"
printf 'drill: A — passed\n'

# ─── Drill B: backup → deletion → older-backup restore ─────────────────────

set_source_keys
novel_a3=$(python3 -c 'import uuid; print(uuid.uuid4())')
# A novel created and deleted after the artifact was taken: its subject row is
# in no dump, so replay can only reconstruct its object key from the record.
psql -c "
  INSERT INTO novels (id, user_id, title, status, original_file_key)
  VALUES ('$novel_a3', '$admin_id', 'Drill novel A3', 'ready',
          'source-files/$admin_id/$novel_a3');
  DELETE FROM novels WHERE id = '$novel_a3';" >/dev/null

check 'B delete novel directly' 204 \
  "$(http_status "${admin_auth[@]}" -X DELETE "$api/novels/$novel_a2")"
pause
reader_login=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$reader_email\",\"password\":\"$password\"}" "$api/auth/login")
reader_token=$(json_get "value['access_token']" <<<"$reader_login")
check 'B delete account with cascade' 204 \
  "$(http_status -H "Authorization: Bearer $reader_token" -X DELETE "$api/auth/me")"
check 'B erasure records written' 'novel:3|user:1' \
  "$(psql -c "SELECT subject_type || ':' || COUNT(*) FROM erasure_records
                WHERE subject_id IN ('$novel_a2','$novel_a3','$novel_b1','$reader_id')
                GROUP BY subject_type ORDER BY 1" | paste -sd'|')"

infra/backup/backup.sh
manifest_two=$(ls -t "$BACKUP_DIR"/*.manifest | head -1)
[ "$manifest_two" != "$manifest_one" ]

calls_before_restore=$(stub_calls)
printf 'drill: B — restoring the older artifact over the live database\n'
docker compose stop gateway user-service novel-service agent-service narrative-service >/dev/null 2>&1
# The current database is reachable, so its erasure export closes the residual
# window and no attest-or-erase decision is required.
infra/backup/restore.sh --manifest "$manifest_one" --env-file "$env_file" --i-stopped-writes

check 'B erased subjects stay deleted' '0:0:0:0' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM users WHERE id = '$reader_id') || ':' ||
                     (SELECT COUNT(*) FROM novels WHERE id = '$novel_a2') || ':' ||
                     (SELECT COUNT(*) FROM novels WHERE id = '$novel_b1') || ':' ||
                     (SELECT COUNT(*) FROM world_states WHERE user_id = '$reader_id')")"
check 'B surviving journey intact' "1:$novel_a1" \
  "$(psql -c "SELECT COUNT(*) || ':' || MIN(id::text) FROM novels")"
check 'B retained sources re-queued' '3' \
  "$(psql -c "SELECT COUNT(*) FROM source_file_deletions WHERE object_key IN
                ('source-files/$admin_id/$novel_a2',
                 'source-files/$reader_id/$novel_b1',
                 'source-files/$admin_id/$novel_a3')")"
check 'B every retained-source record is bookkept' 0 \
  "$(psql -c "SELECT COUNT(*) FROM erasure_records
                WHERE subject_type = 'novel' AND had_source
                  AND source_requeued_at IS NULL")"
# Novels that never held a retained source — including the core reader loop's —
# are never enqueued speculatively, which is what keeps an S3-disabled
# deployment startable.
check 'B no speculative re-queue' 0 \
  "$(psql -c "SELECT COUNT(*) FROM erasure_records
                WHERE NOT had_source AND source_requeued_at IS NOT NULL")"

replay_snapshot=$(psql -c "
  SELECT md5(string_agg(subject_type || subject_id || user_id || erased_at || had_source ||
                        COALESCE(source_requeued_at::text, '-'), ',' ORDER BY subject_type, subject_id))
    FROM erasure_records")
outbox_snapshot=$(psql -c "SELECT md5(string_agg(object_key, ',' ORDER BY object_key))
                             FROM source_file_deletions")
docker compose run --rm postgres-migrate >/dev/null
check 'B second deployment replays cleanly' "$replay_snapshot" "$(psql -c "
  SELECT md5(string_agg(subject_type || subject_id || user_id || erased_at || had_source ||
                        COALESCE(source_requeued_at::text, '-'), ',' ORDER BY subject_type, subject_id))
    FROM erasure_records")"
check 'B second deployment re-queues nothing' "$outbox_snapshot" \
  "$(psql -c "SELECT md5(string_agg(object_key, ',' ORDER BY object_key)) FROM source_file_deletions")"

JWT_SECRET=$(awk -F= '$1 == "JWT_SECRET" { print $2 }' "$env_file")
export JWT_SECRET
drain_outbox
docker compose up -d >/dev/null 2>&1
wait_healthy

check 'B deleted account cannot log in' 401 \
  "$(http_status -H 'Content-Type: application/json' \
    --data "{\"email\":\"$reader_email\",\"password\":\"$password\"}" "$api/auth/login")"
pause
login=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$admin_email\",\"password\":\"$password\"}" "$api/auth/login")
admin_token=$(json_get "value['access_token']" <<<"$login")
admin_auth=(-H "Authorization: Bearer $admin_token")
check 'B deleted novel is not readable' 404 \
  "$(http_status "${admin_auth[@]}" "$api/novels/$novel_a2/chapters")"
pause
"${curl_cmd[@]}" "${admin_auth[@]}" --output "$work/export.ndjson" "$api/account/export"
if grep -Fq "$novel_a2" "$work/export.ndjson"; then
  printf 'drill: FAIL the export still contains the deleted novel\n' >&2
  exit 1
fi
if grep -Fqi 'erasure' "$work/export.ndjson"; then
  printf 'drill: FAIL the export exposes erasure records\n' >&2
  exit 1
fi
check 'B no provider work during the restore' "$calls_before_restore" "$(stub_calls)"
printf 'drill: B — passed\n'

# ─── Negative case: erasure sources that disagree ──────────────────────────

mkdir -p "$work/conflicting"
cp "$(dirname "$manifest_two")"/"$(basename "$manifest_two")" "$work/conflicting/"
conflict_manifest=$work/conflicting/$(basename "$manifest_two")
conflict_erasure_name=$(awk -F= '$1 == "erasure" {print $2}' "$manifest_two")
openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 -salt -pass env:BACKUP_ENCRYPTION_KEY \
  -in "$(dirname "$manifest_two")/$conflict_erasure_name" | gzip -dc >"$work/conflict-source.tsv"
if [ ! -s "$work/conflict-source.tsv" ]; then
  printf 'drill: FAIL the newer artifact carries no erasure record to disagree about\n' >&2
  exit 1
fi
awk -F'\t' 'BEGIN { OFS = FS } { if (NR == 1) $4 = "2000-01-01 00:00:00+00"; print }' \
  "$work/conflict-source.tsv" |
  gzip -9 -c |
  openssl enc -aes-256-cbc -pbkdf2 -iter 200000 -salt -pass env:BACKUP_ENCRYPTION_KEY \
    -out "$work/conflicting/$conflict_erasure_name"
awk -F= -v digest="$(sha256sum "$work/conflicting/$conflict_erasure_name" | cut -d' ' -f1)" \
  '$1 == "erasure_sha256" { print "erasure_sha256=" digest; next } { print }' \
  "$manifest_two" >"$conflict_manifest"
docker compose stop gateway user-service novel-service agent-service narrative-service >/dev/null 2>&1
refuses 'conflicting erasure sources' infra/backup/restore.sh --manifest "$manifest_two" \
  --newer-artifact "$conflict_manifest" --env-file "$env_file"
refusal_says 'conflicting erasure records'

# ─── Drill C: disaster gate ────────────────────────────────────────────────

printf 'drill: C — restoring with no reachable current database\n'
docker compose down -v >/dev/null 2>&1
docker compose up -d postgres >/dev/null 2>&1
for _ in $(seq 1 60); do
  docker exec novel-postgres pg_isready -U "${POSTGRES_USER:-novel}" >/dev/null 2>&1 && break
  sleep 2
done

refuses 'C undecided restore' infra/backup/restore.sh --manifest "$manifest_one" \
  --env-file "$env_file"
refusal_says 'refusing to complete a disaster restore'
refusal_says "$reader_id"
check 'C refusal changed nothing' '0:0' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM users) || ':' ||
                     (SELECT COUNT(*) FROM restore_attestations)")"

cat >"$work/decisions-partial" <<EOF
operator=backup-restore-v1 drill C
retain $admin_id novels=$novel_a1
EOF
refuses 'C partial decisions' infra/backup/restore.sh --manifest "$manifest_one" \
  --decisions "$work/decisions-partial" --env-file "$env_file"
refusal_says 'every restored account needs a decision'
refusal_says "$reader_id"

cat >"$work/decisions-no-admin" <<EOF
operator=backup-restore-v1 drill C
erase $admin_id
retain $reader_id novels=$novel_b1
EOF
refuses 'C decisions leaving no administrator' infra/backup/restore.sh \
  --manifest "$manifest_one" --decisions "$work/decisions-no-admin" --env-file "$env_file"
refusal_says 'leave no administrator'
check 'C refusals changed nothing' '0:0' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM users) || ':' ||
                     (SELECT COUNT(*) FROM restore_attestations)")"

cat >"$work/decisions-complete" <<EOF
# One account retained with a partial novel list, the other erased.
operator=backup-restore-v1 drill C
retain $admin_id novels=$novel_a1
erase $reader_id
EOF
failure_time=$(date -u +'%Y-%m-%d %H:%M:%S+00')
infra/backup/restore.sh --manifest "$manifest_one" --decisions "$work/decisions-complete" \
  --declared-failure-time "$failure_time" --env-file "$env_file"

check 'C erased account is gone' 0 "$(psql -c "SELECT COUNT(*) FROM users WHERE id = '$reader_id'")"
check 'C unlisted novel is gone' 0 "$(psql -c "SELECT COUNT(*) FROM novels WHERE id = '$novel_a2'")"
check 'C retained novel survived' 1 "$(psql -c "SELECT COUNT(*) FROM novels WHERE id = '$novel_a1'")"
check 'C decisions wrote erasure records' '1:1:1' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM erasure_records
                        WHERE subject_type = 'user' AND subject_id = '$reader_id') || ':' ||
                     (SELECT COUNT(*) FROM erasure_records
                        WHERE subject_type = 'novel' AND subject_id = '$novel_a2') || ':' ||
                     (SELECT COUNT(*) FROM erasure_records
                        WHERE subject_type = 'novel' AND subject_id = '$novel_b1')")"
check 'C cascade removed dependent rows' '0:0:0' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM world_states WHERE user_id = '$reader_id') || ':' ||
                     (SELECT COUNT(*) FROM chapters WHERE novel_id = '$novel_a2') || ':' ||
                     (SELECT COUNT(*) FROM refresh_tokens)")"
check 'C attestation fields' \
  "$admin_id|retain|$covered_one|$failure_time|backup-restore-v1 drill C|true|true" \
  "$(psql -c "SELECT subject_id || '|' || decision || '|' ||
                     to_char(window_start AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US+00') || '|' ||
                     to_char(window_end AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS+00') || '|' ||
                     operator_identity || '|' ||
                     (artifact_inventory <> '') || '|' || (recorded_at IS NOT NULL)
                FROM restore_attestations WHERE decision = 'retain'")"
check 'C erase decision recorded' "$reader_id" \
  "$(psql -c "SELECT subject_id FROM restore_attestations WHERE decision = 'erase'")"
check 'C attestation names the artifact digest' 1 \
  "$(psql -c "SELECT COUNT(*) FROM restore_attestations
                WHERE artifact_inventory = '$(awk -F= '$1 == "dump_sha256" {print $2}' "$manifest_one")'
                  AND decision = 'retain'")"

JWT_SECRET=$(awk -F= '$1 == "JWT_SECRET" { print $2 }' "$env_file")
export JWT_SECRET
drain_outbox
docker compose up -d >/dev/null 2>&1
wait_healthy

check 'C pre-restore access token rejected' 401 \
  "$(http_status -H "Authorization: Bearer $admin_token" "$api/auth/me")"
check 'C erased account cannot log in' 401 \
  "$(http_status -H 'Content-Type: application/json' \
    --data "{\"email\":\"$reader_email\",\"password\":\"$password\"}" "$api/auth/login")"
pause
login=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$admin_email\",\"password\":\"$password\"}" "$api/auth/login")
admin_token=$(json_get "value['access_token']" <<<"$login")
admin_auth=(-H "Authorization: Bearer $admin_token")
check 'C unlisted novel is not served' 404 \
  "$(http_status "${admin_auth[@]}" "$api/novels/$novel_a2/chapters")"
check 'C retained novel is served' 200 \
  "$(http_status "${admin_auth[@]}" "$api/novels/$novel_a1/chapters")"
printf 'drill: C — passed\n'

printf 'drill: backup-restore-v1 drills A, B, C and the negative cases passed\n'
