#!/usr/bin/env bash
# Drills A, B and C plus the negative cases of backup-restore-v2
# (docs/BACKUP_RESTORE.md, "Drills"), against the supported compose topology.
#
# Runs after the provider-outage drill (which follows core_reader_loop.sh),
# with the deployment left in first-run state, and seeds its own drill dataset
# through the real import path:
# three accounts — the third exists to be deleted after the artifact and covered
# by a collected erasure record — five fixture novels with two durable chapters
# each, three retained-source keys, committed chat history and a committed world
# turn. Shared canonical novels retained by earlier drills are valid background
# state and must survive every backup/restore phase.
#
# Under v2 every restore regenerates the database's lineage token, so an
# artifact can establish continuation only against the lineage that produced it:
# drill A restores onto a destroyed host and therefore always goes through the
# disaster gate, and drill B restores an artifact taken from the lineage drill A
# created.
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
third_email=drill-third@test.invalid

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

lineage_token() { psql -c "SELECT token FROM database_lineage"; }
lineage_parent() { psql -c "SELECT COALESCE(parent::text, 'absent') FROM database_lineage"; }
manifest_field() { awk -F= -v key="$2" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$1"; }

# The S3 cleanup worker owns the outbox in a deployment that enables S3; with S3
# disabled the drill drains it so novel-service can start (see the header note).
drain_outbox() {
  psql -c "DELETE FROM source_file_deletions" >/dev/null
  psql -c "UPDATE novels SET original_file_key = NULL WHERE original_file_key LIKE 'source-files/%'" >/dev/null
}

# Every drill novel goes through the real upload path, so the fixture is five
# imported novels with durable chapters, characters and canon models.
import_novel() { # import_novel TOKEN TITLE -> novel id on stdout
  local token=$1 title=$2 novel state
  pause
  novel=$(json_get "value['novel_id']" <<<"$("${curl_cmd[@]}" -H "Authorization: Bearer $token" \
    -F "title=$title" -F 'author=Drill' -F 'deviation_mode=creative' \
    -F "file=@$source_file;filename=storm.txt;type=text/plain" \
    "$api/novels/upload")")
  for _ in $(seq 1 60); do
    sleep 2
    state=$(json_get "value['status']" <<<"$("${curl_cmd[@]}" -H "Authorization: Bearer $token" \
      "$api/novels/$novel/status")")
    [ "$state" = ready ] && break
    if [ "$state" = error ]; then
      printf 'drill: import of %s failed\n' "$title" >&2
      exit 1
    fi
  done
  [ "$state" = ready ] || { printf 'drill: import of %s never became ready\n' "$title" >&2; exit 1; }
  printf '%s' "$novel"
}

set_source_keys() {
  psql -c "UPDATE novels SET original_file_key = 'source-files/' || user_id || '/' || id
             WHERE id IN ('$novel_a2', '$novel_b1', '$novel_b2')" >/dev/null
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
reader_token=$(json_get "value['access_token']" <<<"$reader")
reader_refresh=$(json_get "value['refresh_token']" <<<"$reader")

pause
third=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$third_email\",\"password\":\"$password\",\"name\":\"Drill Third\"}" \
  "$api/auth/register")
third_id=$(json_get "value['user']['id']" <<<"$third")
third_token=$(json_get "value['access_token']" <<<"$third")

novel_a1=$(import_novel "$admin_token" '风暴之塔')
novel_a2=$(import_novel "$admin_token" '风暴之塔 II')
novel_b1=$(import_novel "$reader_token" '边城旧事')
novel_b2=$(import_novel "$reader_token" '边城旧事 II')
novel_c1=$(import_novel "$third_token" '第三账户的书')
fixture_novel_ids="'$novel_a1','$novel_a2','$novel_b1','$novel_b2','$novel_c1'"
fixture_user_ids="'$admin_id','$reader_id','$third_id'"
background_novel_count=$(psql -c "SELECT COUNT(*) FROM novels WHERE id NOT IN ($fixture_novel_ids)")

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
  --data '{"expected_turn_number":0,"kind":"pursue_goal","target_id":null,"intent":"绘制地下回廊并寻找守门人的踪迹"}' \
  "$api/narrative/$novel_a1/world/turns"
pause
chat=$("${curl_cmd[@]}" --no-buffer "${admin_auth[@]}" \
  -H 'Content-Type: application/json' \
  -H "Idempotency-Key: $(python3 -c 'import uuid; print(uuid.uuid4())')" \
  --data "{\"message\":\"你还记得我吗？\",\"novel_id\":\"$novel_a1\"}" \
  "$api/chat/$character_id/stream")
grep -Fq 'event: done' <<<"$chat"

# The reader's dependent rows for the account-cascade assertions later; the
# shared canonical novels themselves all came through the import path above.
psql -c "INSERT INTO world_states (user_id, novel_id)
         VALUES ('$reader_id', '$novel_b1')" >/dev/null

check 'dataset accounts' 3 \
  "$(psql -c "SELECT COUNT(*) FROM users WHERE id IN ($fixture_user_ids)")"
check 'dataset novels' 5 \
  "$(psql -c "SELECT COUNT(*) FROM novels WHERE id IN ($fixture_novel_ids)")"
check 'dataset chapters' 10 \
  "$(psql -c "SELECT COUNT(*) FROM chapters WHERE novel_id IN ($fixture_novel_ids)")"
check 'dataset imports are complete' 5 \
  "$(psql -c "SELECT COUNT(*) FROM novel_import_jobs
                WHERE novel_id IN ($fixture_novel_ids) AND status = 'completed'")"
check 'dataset has one lineage token' 1 \
  "$(psql -c "SELECT COUNT(*) FROM database_lineage WHERE parent IS NULL")"
check 'dataset chat messages' 2 \
  "$(psql -c "SELECT COUNT(*) FROM chat_messages WHERE user_id IN ($fixture_user_ids)")"
check 'dataset world turns' 1 \
  "$(psql -c "SELECT COUNT(*) FROM world_turns
                WHERE user_id IN ($fixture_user_ids) AND status = 'completed'")"

# Retained-source keys, set after the services started: novel-service refuses to
# start with S3 disabled while any source key or outbox row exists.
set_source_keys
check 'retained source keys' 3 \
  "$(psql -c "SELECT COUNT(*) FROM novels
                WHERE id IN ($fixture_novel_ids)
                  AND original_file_key LIKE 'source-files/%'")"

# ─── Backup ────────────────────────────────────────────────────────────────

infra/backup/backup.sh
manifest_one=$(ls -t "$BACKUP_DIR"/*.manifest | head -1)
check 'artifact records the live lineage token' "$(lineage_token)" \
  "$(manifest_field "$manifest_one" lineage_token)"
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
# Manifest metadata reaches SQL literals, so it is shape-checked at the boundary.
mkdir -p "$work/tampered"
cp "$BACKUP_DIR"/* "$work/tampered/"
tampered_manifest=$work/tampered/$(basename "$manifest_one")
awk -F= "\$1 == \"covered_through\" { print \"covered_through=2026-01-01 00:00:00+00'; DROP TABLE users; --\"; next } { print }" \
  "$manifest_one" >"$tampered_manifest"
refuses 'tampered covered-through metadata' infra/backup/restore.sh \
  --manifest "$tampered_manifest" --env-file "$env_file"
refusal_says 'is not a'
# Manifest-to-dump token binding: the manifest is the editable half.
awk -F= '$1 == "lineage_token" { print "lineage_token=00000000-0000-4000-8000-000000000000"; next } { print }' \
  "$manifest_one" >"$work/tampered/token.manifest"
refuses 'manifest token disagreeing with the dump' infra/backup/restore.sh \
  --manifest "$work/tampered/token.manifest" --env-file "$env_file"
refusal_says 'disagrees with'
grep -v '^lineage_token=' "$manifest_one" >"$work/tampered/half-token.manifest"
refuses 'artifact with a token on only one side' infra/backup/restore.sh \
  --manifest "$work/tampered/half-token.manifest" --env-file "$env_file"
refusal_says 'in only one of its manifest and dump'
check 'metadata negatives changed nothing' "$before_negatives" \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM users) || ':' || (SELECT COUNT(*) FROM novels)")"
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
docker compose up -d --wait --wait-timeout 180 postgres >/dev/null

# Destroying the volumes leaves no lineage to match, so drill A is a disaster
# restore under v2 and must carry a complete decision set.
cat >"$work/decisions-retain-all" <<EOF
operator=backup-restore-v2 drill A
retain $admin_id
retain $reader_id
retain $third_id
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
check 'A attestations' 'retain:3' \
  "$(psql -c "SELECT decision || ':' || COUNT(*) FROM restore_attestations GROUP BY decision")"
check 'A regenerated the lineage token' "$(manifest_field "$manifest_one" lineage_token)" \
  "$(lineage_parent)"
[ "$(lineage_token)" != "$(manifest_field "$manifest_one" lineage_token)" ]
artifact_one_lineage=$(lineage_token)
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

# ─── Post-A: the third account's deletion and the newer artifact ───────────
# Deleted after artifact one, so a newer artifact carries its erasure record as
# a collected source for drill C. The same artifact is drill B's older backup:
# it belongs to the lineage drill A's restore created, which is what lets drill
# B establish continuation at all.

set_source_keys
pause
third_login=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$third_email\",\"password\":\"$password\"}" "$api/auth/login")
check 'third account deleted after the artifact' 204 \
  "$(http_status -H "Authorization: Bearer $(json_get "value['access_token']" <<<"$third_login")" \
    -X DELETE "$api/auth/me")"
check 'third account erasure records' 'user:1' \
  "$(psql -c "SELECT subject_type || ':' || COUNT(*) FROM erasure_records
                WHERE subject_id IN ('$third_id', '$novel_c1')
                GROUP BY subject_type ORDER BY 1" | paste -sd'|')"
infra/backup/backup.sh
manifest_newer=$(ls -t "$BACKUP_DIR"/*.manifest | head -1)
check 'newer artifact belongs to the restored lineage' "$(lineage_token)" \
  "$(manifest_field "$manifest_newer" lineage_token)"

# ─── Drill B: backup → deletion → older-backup restore ─────────────────────


novel_a3=$(python3 -c 'import uuid; print(uuid.uuid4())')
# A novel created and deleted after the artifact was taken: its subject row is
# in no dump, so replay can only reconstruct its object key from the record.
psql -c "
  INSERT INTO novels (id, user_id, title, status, original_file_key)
  VALUES ('$novel_a3', '$admin_id', 'Drill novel A3', 'ready',
          'source-files/$admin_id/$novel_a3');
  DELETE FROM novels WHERE id = '$novel_a3';" >/dev/null

psql -c "DELETE FROM novels WHERE id = '$novel_a2'" >/dev/null
check 'B delete canonical novel directly' 0 \
  "$(psql -c "SELECT COUNT(*) FROM novels WHERE id = '$novel_a2'")"
pause
reader_login=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$reader_email\",\"password\":\"$password\"}" "$api/auth/login")
reader_token=$(json_get "value['access_token']" <<<"$reader_login")
check 'B delete account and private world' 204 \
  "$(http_status -H "Authorization: Bearer $reader_token" -X DELETE "$api/auth/me")"
check 'B erasure records written' 'novel:2|user:1' \
  "$(psql -c "SELECT subject_type || ':' || COUNT(*) FROM erasure_records
                WHERE subject_id IN ('$novel_a2','$novel_a3','$novel_b1','$novel_b2','$reader_id')
                GROUP BY subject_type ORDER BY 1" | paste -sd'|')"

infra/backup/backup.sh
manifest_two=$(ls -t "$BACKUP_DIR"/*.manifest | head -1)
[ "$manifest_two" != "$manifest_newer" ]

calls_before_restore=$(stub_calls)
printf 'drill: B — restoring the older artifact over the live database\n'
docker compose stop gateway user-service novel-service agent-service narrative-service >/dev/null 2>&1
# The current database is reachable and carries this artifact's lineage token,
# so its erasure export closes the residual window and no attest-or-erase
# decision is required.
infra/backup/restore.sh --manifest "$manifest_newer" --env-file "$env_file" --i-stopped-writes
check 'B recorded the restored artifact as parent' \
  "$(manifest_field "$manifest_newer" lineage_token)" "$(lineage_parent)"

check 'B erased subjects stay deleted and shared novels survive' '0:0:1:1:0' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM users WHERE id = '$reader_id') || ':' ||
                     (SELECT COUNT(*) FROM novels WHERE id = '$novel_a2') || ':' ||
                     (SELECT COUNT(*) FROM novels WHERE id = '$novel_b1') || ':' ||
                     (SELECT COUNT(*) FROM novels WHERE id = '$novel_b2') || ':' ||
                     (SELECT COUNT(*) FROM world_states WHERE user_id = '$reader_id')")"
check 'B surviving canonical novels intact' "$((background_novel_count + 4))" \
  "$(psql -c "SELECT COUNT(*) FROM novels")"
check 'B primary journey novel intact' 1 \
  "$(psql -c "SELECT COUNT(*) FROM novels WHERE id = '$novel_a1'")"
check 'B third account stays erased across the restore' 0 \
  "$(psql -c "SELECT COUNT(*) FROM users WHERE id = '$third_id'")"
check 'B explicitly deleted retained sources re-queued' '2' \
  "$(psql -c "SELECT COUNT(*) FROM source_file_deletions WHERE object_key IN
                ('source-files/$admin_id/$novel_a2',
                 'source-files/$reader_id/$novel_b1',
                 'source-files/$reader_id/$novel_b2',
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

printf 'drill: C — restoring with no lineage-matching reachable database\n'
fresh_postgres() {
  docker compose down -v >/dev/null 2>&1
  docker compose up -d --wait --wait-timeout 180 postgres >/dev/null
}
fresh_postgres

refuses 'C undecided restore' infra/backup/restore.sh --manifest "$manifest_one" \
  --env-file "$env_file"
refusal_says 'refusing to complete a disaster restore'
refusal_says "$reader_id"
check 'C refusal changed nothing' '0:0' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM users) || ':' ||
                     (SELECT COUNT(*) FROM restore_attestations)")"

# A populated replacement database with its own deletion history is not this
# lineage: only token equality is continuation, and a genesis token never
# equals an artifact's.
psql -c "INSERT INTO users (email, password_hash) VALUES ('unrelated@test.invalid', 'x');
         DELETE FROM users WHERE email = 'unrelated@test.invalid'" >/dev/null
check 'C unrelated database has its own journal row' 1 \
  "$(psql -c "SELECT COUNT(*) FROM erasure_records")"
refuses 'C unrelated live lineage is not this lineage' infra/backup/restore.sh \
  --manifest "$manifest_one" --env-file "$env_file"
refusal_says 'refusing to complete a disaster restore'

cat >"$work/decisions-partial" <<EOF
operator=backup-restore-v2 drill C
retain $admin_id
EOF
refuses 'C partial decisions' infra/backup/restore.sh --manifest "$manifest_one" \
  --decisions "$work/decisions-partial" --env-file "$env_file"
refusal_says 'every restored account needs a decision'
refusal_says "$reader_id"

cat >"$work/decisions-no-admin" <<EOF
operator=backup-restore-v2 drill C
erase $admin_id
retain $reader_id
retain $third_id
EOF
refuses 'C decisions leaving no administrator' infra/backup/restore.sh \
  --manifest "$manifest_one" --decisions "$work/decisions-no-admin" --env-file "$env_file"
refusal_says 'leave no administrator'

# The third account is covered by the newer artifact's collected record: replay
# enforces it, so it neither needs nor accepts a decision.
refuses 'C undecided restore with a newer erasure source' infra/backup/restore.sh \
  --manifest "$manifest_one" --newer-artifact "$manifest_newer" --env-file "$env_file"
refusal_says "$admin_id"
if grep -Fq "  $third_id role=" "$work/refusal.txt"; then
  printf 'drill: FAIL a collected-erasure account was still listed as undecided\n' >&2
  exit 1
fi
if grep -Fq "$novel_c1" "$work/refusal.txt"; then
  printf 'drill: FAIL a collected-erasure novel was advertised as retainable\n' >&2
  exit 1
fi
cat >"$work/decisions-override" <<EOF
operator=backup-restore-v2 drill C
erase $admin_id
retain $reader_id
admin=$reader_id
erase $third_id
EOF
refuses 'C deciding a collected-erasure account' infra/backup/restore.sh \
  --manifest "$manifest_one" --newer-artifact "$manifest_newer" \
  --decisions "$work/decisions-override" --env-file "$env_file"
refusal_says 'no decision may name it'
check 'C refusals changed nothing' '0:0' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM users) || ':' ||
                     (SELECT COUNT(*) FROM restore_attestations)")"

# A wholly token-less artifact — one written before the lineage migration —
# restores only through the disaster gate and records an absent parent.
mkdir -p "$work/tokenless"
cp "$manifest_one" "$work/tokenless/"
tokenless_manifest=$work/tokenless/$(basename "$manifest_one")
tokenless_dump=tokenless.dump.gz.enc
cp "$(dirname "$manifest_one")/$(manifest_field "$manifest_one" erasure)" "$work/tokenless/"
openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 -salt -pass env:BACKUP_ENCRYPTION_KEY \
  -in "$(dirname "$manifest_one")/$(manifest_field "$manifest_one" dump)" | gzip -dc |
  awk '/^COPY public\.database_lineage \(/ { inside = 1 }
       inside && $0 == "\\." { inside = 0; print; next }
       inside && !/^COPY/ { next }
       { print }' |
  gzip -9 -c |
  openssl enc -aes-256-cbc -pbkdf2 -iter 200000 -salt -pass env:BACKUP_ENCRYPTION_KEY \
    -out "$work/tokenless/$tokenless_dump"
awk -F= -v dump="$tokenless_dump" \
  -v digest="$(sha256sum "$work/tokenless/$tokenless_dump" | cut -d' ' -f1)" '
  $1 == "lineage_token" { next }
  $1 == "dump" { print "dump=" dump; next }
  $1 == "dump_sha256" { print "dump_sha256=" digest; next }
  { print }' "$manifest_one" >"$tokenless_manifest"
refuses 'C token-less artifact faces the gate' infra/backup/restore.sh \
  --manifest "$tokenless_manifest" --env-file "$env_file"
refusal_says 'refusing to complete a disaster restore'
cat >"$work/decisions-tokenless" <<EOF
operator=backup-restore-v2 drill C token-less
erase $admin_id
erase $reader_id
erase $third_id
EOF
infra/backup/restore.sh --manifest "$tokenless_manifest" \
  --decisions "$work/decisions-tokenless" --env-file "$env_file" >/dev/null
check 'C token-less restore records an absent parent' absent "$(lineage_parent)"
[ -n "$(lineage_token)" ]

# The sanctioned continuation: erase the administrator's account, retain the
# reader and all of that reader's private worlds, designate the retained account as the
# administrator the installation would otherwise lack, and let replay enforce
# the collected record covering the third account.
cat >"$work/decisions-complete" <<EOF
# One account retained, one erased, one pre-decided.
operator=backup-restore-v2 drill C
erase $admin_id
retain $reader_id
admin=$reader_id
EOF
failure_time=$(date -u +'%Y-%m-%d %H:%M:%S+00')
infra/backup/restore.sh --manifest "$manifest_one" --newer-artifact "$manifest_newer" \
  --decisions "$work/decisions-complete" --declared-failure-time "$failure_time" \
  --env-file "$env_file"
first_restore_token=$(lineage_token)

check 'C erased account is gone' 0 "$(psql -c "SELECT COUNT(*) FROM users WHERE id = '$admin_id'")"
check 'C collected-record account is gone' 0 \
  "$(psql -c "SELECT COUNT(*) FROM users WHERE id = '$third_id'")"
check 'C collected account canonical novel survived' 1 \
  "$(psql -c "SELECT COUNT(*) FROM novels WHERE id = '$novel_c1'")"
check 'C retained account second canonical novel survived' 1 "$(psql -c "SELECT COUNT(*) FROM novels WHERE id = '$novel_b2'")"
check 'C retained novel survived' 1 "$(psql -c "SELECT COUNT(*) FROM novels WHERE id = '$novel_b1'")"
check 'C designated administrator promoted' 'admin' \
  "$(psql -c "SELECT role FROM users WHERE id = '$reader_id'")"
check 'C decisions wrote only account erasure records' '1:0:0' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM erasure_records
                        WHERE subject_type = 'user' AND subject_id = '$admin_id') || ':' ||
                     (SELECT COUNT(*) FROM erasure_records
                        WHERE subject_type = 'novel' AND subject_id = '$novel_b2') || ':' ||
                     (SELECT COUNT(*) FROM erasure_records
                        WHERE subject_type = 'novel' AND subject_id = '$novel_a1')")"
check 'C account cascade removed only erased-user data' '0:2:0:0' \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM chat_messages) || ':' ||
                     (SELECT COUNT(*) FROM chapters WHERE novel_id = '$novel_b2') || ':' ||
                     (SELECT COUNT(*) FROM world_states WHERE user_id = '$admin_id') || ':' ||
                     (SELECT COUNT(*) FROM refresh_tokens)")"
# The window starts at the NEWEST source's covered-through — here the newer
# artifact's, not the restored artifact's — and ends at the declared failure.
newest_covered=$(manifest_field "$manifest_newer" covered_through)
check 'C attestation fields' \
  "$reader_id|retain|$newest_covered|$failure_time|backup-restore-v2 drill C|true|true|true" \
  "$(psql -c "SELECT subject_id || '|' || decision || '|' ||
                     to_char(window_start AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US+00') || '|' ||
                     to_char(window_end AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS+00') || '|' ||
                     operator_identity || '|' || designated_admin || '|' ||
                     (artifact_inventory <> '') || '|' || (recorded_at IS NOT NULL)
                FROM restore_attestations WHERE decision = 'retain'")"
check 'C erase decision recorded' "$admin_id|false" \
  "$(psql -c "SELECT subject_id || '|' || designated_admin FROM restore_attestations
                WHERE decision = 'erase'")"
# The collected record is audited as a restore-level fact with the full field
# set, without an operator decision.
check 'C replayed attestation recorded' \
  "$third_id|$newest_covered|$failure_time|backup-restore-v2 drill C|false|true|true" \
  "$(psql -c "SELECT subject_id || '|' ||
                     to_char(window_start AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US+00') || '|' ||
                     to_char(window_end AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS+00') || '|' ||
                     operator_identity || '|' || designated_admin || '|' ||
                     (artifact_inventory <> '') || '|' || (recorded_at IS NOT NULL)
                FROM restore_attestations WHERE decision = 'replayed'")"
check 'C inventory lists only verified digests' \
  "dump:$(manifest_field "$manifest_one" dump_sha256),erasure:$(manifest_field "$manifest_one" erasure_sha256),erasure:$(manifest_field "$manifest_newer" erasure_sha256)" \
  "$(psql -c "SELECT DISTINCT artifact_inventory FROM restore_attestations")"
check 'C recorded the artifact token as parent' \
  "$(manifest_field "$manifest_one" lineage_token)" "$(lineage_parent)"

# Ordinary migration replay preserves the lineage; only a restore replaces it.
docker compose run --rm postgres-migrate >/dev/null
check 'C migration replay preserves the lineage token' "$first_restore_token" "$(lineage_token)"

JWT_SECRET=$(awk -F= '$1 == "JWT_SECRET" { print $2 }' "$env_file")
export JWT_SECRET
drain_outbox
docker compose up -d >/dev/null 2>&1
wait_healthy

check 'C pre-restore access token rejected' 401 \
  "$(http_status -H "Authorization: Bearer $admin_token" "$api/auth/me")"
check 'C erased account cannot log in' 401 \
  "$(http_status -H 'Content-Type: application/json' \
    --data "{\"email\":\"$admin_email\",\"password\":\"$password\"}" "$api/auth/login")"
check 'C collected-record account cannot log in' 401 \
  "$(http_status -H 'Content-Type: application/json' \
    --data "{\"email\":\"$third_email\",\"password\":\"$password\"}" "$api/auth/login")"
pause
login=$("${curl_cmd[@]}" -H 'Content-Type: application/json' \
  --data "{\"email\":\"$reader_email\",\"password\":\"$password\"}" "$api/auth/login")
reader_token=$(json_get "value['access_token']" <<<"$login")
reader_auth=(-H "Authorization: Bearer $reader_token")
check 'C retained account second novel is served' 200 \
  "$(http_status "${reader_auth[@]}" "$api/novels/$novel_b2/chapters")"
check 'C retained novel is served' 200 \
  "$(http_status "${reader_auth[@]}" "$api/novels/$novel_b1/chapters")"

# A second restore of the same artifact is a sibling of the first, not its
# continuation: it faces the gate, and its token is distinct with the same
# recorded parent.
docker compose stop gateway user-service novel-service agent-service narrative-service >/dev/null 2>&1
refuses 'C sibling restore of one artifact is not continuation' infra/backup/restore.sh \
  --manifest "$manifest_one" --newer-artifact "$manifest_newer" --env-file "$env_file"
refusal_says 'refusing to complete a disaster restore'
infra/backup/restore.sh --manifest "$manifest_one" --newer-artifact "$manifest_newer" \
  --decisions "$work/decisions-complete" --declared-failure-time "$failure_time" \
  --env-file "$env_file" >/dev/null
check 'C two restores of one artifact differ' true \
  "$([ "$(lineage_token)" != "$first_restore_token" ] && echo true || echo false)"
check 'C both restores record the artifact as parent' \
  "$(manifest_field "$manifest_one" lineage_token)" "$(lineage_parent)"

# A crash before the atomic commit leaves no reachable data and no token, and a
# crash after it leaves the regenerated token: either way the retry faces the
# gate rather than presenting the artifact's token as live.
refuses 'C failure injected before the atomic commit' \
  env RESTORE_FAIL_BEFORE_COMMIT=1 infra/backup/restore.sh --manifest "$manifest_one" \
  --newer-artifact "$manifest_newer" --decisions "$work/decisions-complete" \
  --declared-failure-time "$failure_time" --env-file "$env_file"
refusal_says 'injected pre-commit failure'
check 'C aborted load left no reachable data' 0 \
  "$(psql -c "SELECT COUNT(*) FROM pg_tables WHERE schemaname = 'public'")"
refuses 'C retry after the pre-commit failure faces the gate' infra/backup/restore.sh \
  --manifest "$manifest_one" --newer-artifact "$manifest_newer" --env-file "$env_file"
refusal_says 'refusing to complete a disaster restore'
# Absence is not equality: a database with no token and an artifact with no
# token are not one lineage.
refuses 'C token-less artifact over a token-less database' infra/backup/restore.sh \
  --manifest "$tokenless_manifest" --env-file "$env_file"
refusal_says 'refusing to complete a disaster restore'

refuses 'C failure injected after the atomic commit' \
  env RESTORE_FAIL_AFTER_COMMIT=1 infra/backup/restore.sh --manifest "$manifest_one" \
  --newer-artifact "$manifest_newer" --decisions "$work/decisions-complete" \
  --declared-failure-time "$failure_time" --env-file "$env_file"
refusal_says 'injected post-commit failure'
check 'C committed load already carries a regenerated token' false \
  "$([ "$(lineage_token)" = "$(manifest_field "$manifest_one" lineage_token)" ] && echo true || echo false)"
refuses 'C retry after the post-commit failure faces the gate' infra/backup/restore.sh \
  --manifest "$manifest_one" --newer-artifact "$manifest_newer" --env-file "$env_file"
refusal_says 'refusing to complete a disaster restore'

# Erasing every account is a sanctioned outcome, not a stuck restore: no
# administrator is designated and the installation returns to first-run setup.
fresh_postgres
cat >"$work/decisions-erase-all" <<EOF
operator=backup-restore-v2 drill C erase-all
erase $admin_id
erase $reader_id
erase $third_id
EOF
infra/backup/restore.sh --manifest "$manifest_one" --decisions "$work/decisions-erase-all" \
  --env-file "$env_file" >/dev/null
check 'C erase-all removes accounts but retains shared canonical novels' \
  "0:$((background_novel_count + 5)):0" \
  "$(psql -c "SELECT (SELECT COUNT(*) FROM users) || ':' ||
                     (SELECT COUNT(*) FROM novels) || ':' ||
                     (SELECT COUNT(*) FROM runtime_llm_config)")"
check 'C erase-all recorded every decision' 3 \
  "$(psql -c "SELECT COUNT(*) FROM restore_attestations WHERE decision = 'erase'")"
check 'C erase-all designated nobody' 0 \
  "$(psql -c "SELECT COUNT(*) FROM restore_attestations WHERE designated_admin")"
printf 'drill: C — passed\n'

printf 'drill: backup-restore-v2 drills A, B, C and the negative cases passed\n'
