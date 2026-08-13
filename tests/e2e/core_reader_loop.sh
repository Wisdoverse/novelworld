#!/usr/bin/env bash
set -euo pipefail

api=${E2E_API_URL:-http://127.0.0.1/api}
email=admin@test.invalid
password='RuntimeSmokeOnly123!'
source_file=$(mktemp)
trap 'rm -f "$source_file"' EXIT
curl_cmd=(curl --connect-timeout 5 --max-time 120 --fail --silent --show-error)

json_get() {
  python3 -c "import json,sys; value=json.load(sys.stdin); print($1)"
}

pause() {
  sleep 1.1
}

printf '%s\n' \
  '第一章 风暴前夜' \
  '林岚握紧手中的旧地图，望向被风暴笼罩的北塔，决定在天黑前寻找失踪的守门人。边城的钟声连续响了三次，街道上的人们纷纷关紧门窗。林岚仍站在石桥中央，逐一核对地图上的暗号，并请你留意河岸新出现的足迹。远处的塔灯忽明忽暗，仿佛有人正用最后的力气发出求救信号。你们约定不替彼此作决定，却要共同承担进入风暴的后果。' \
  '第二章 北塔回声' \
  '北塔的石门布满潮湿苔痕，林岚在门边发现守门人留下的铜铃。铃身刻着通往地下回廊的路线，也写明只有彼此信任的同行者才能安全通过。风暴压低了天空，城墙上的火把依次熄灭。你与林岚交换各自找到的线索，确认失踪并非意外。塔内传来沉重脚步，旧地图上从未标注的房间正在缓缓开启，而边城的命运也随这一刻发生变化。' \
  >"$source_file"

pause
login=$("${curl_cmd[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"email\":\"$email\",\"password\":\"$password\"}" \
  "$api/auth/login")
token=$(json_get "value['access_token']" <<<"$login")
auth=(-H "Authorization: Bearer $token")

pause
upload=$("${curl_cmd[@]}" "${auth[@]}" \
  -F 'title=风暴之塔' \
  -F 'author=E2E' \
  -F 'deviation_mode=creative' \
  -F "file=@$source_file;filename=storm.txt;type=text/plain" \
  "$api/novels/upload")
novel_id=$(json_get "value['novel_id']" <<<"$upload")

retried=false
for _ in $(seq 1 45); do
  sleep 2
  status=$("${curl_cmd[@]}" "${auth[@]}" "$api/novels/$novel_id/status")
  state=$(json_get "value['status']" <<<"$status")
  [ "$state" = ready ] && break
  if [ "$state" = error ]; then
    [ "$retried" = false ] || { printf 'novel retry failed: %s\n' "$status" >&2; exit 1; }
    count=$(docker exec novel-postgres psql \
      -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
      -c "SELECT COUNT(*) FROM canon_story_models WHERE novel_id = '$novel_id'")
    [ "$count" = 0 ]
    pause
    "${curl_cmd[@]}" --output /dev/null "${auth[@]}" -X POST "$api/novels/$novel_id/retry"
    retried=true
  fi
done
[ "$state" = ready ] || { printf 'novel did not become ready: %s\n' "$status" >&2; exit 1; }
[ "$retried" = true ]

canon_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT model_version || ':' || schema_version || ':' || prompt_version || ':' || md5(content::text) FROM canon_story_models WHERE novel_id = '$novel_id'")
[[ "$canon_snapshot" == 1:1:canon-chunk-v1:* ]]

pause
chapters=$("${curl_cmd[@]}" "${auth[@]}" "$api/novels/$novel_id/chapters")
python3 -c 'import json,sys; chapters=json.load(sys.stdin); assert len(chapters)==2' <<<"$chapters"

pause
progress=$("${curl_cmd[@]}" "${auth[@]}" "$api/progress/$novel_id")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['current_chapter']==1" <<<"$progress"

pause
"${curl_cmd[@]}" --output /dev/null "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' \
  --data '{"identity_type":"self","identity_name":"云舟","character_id":null}' \
  "$api/progress/$novel_id/identity"

pause
characters=$("${curl_cmd[@]}" "${auth[@]}" "$api/novels/$novel_id/characters")
character_id=$(json_get "value[0]['id']" <<<"$characters")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value[0]['name']=='林岚'" <<<"$characters"

turn_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
pause
stream=$("${curl_cmd[@]}" --no-buffer "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $turn_id" \
  --data "{\"message\":\"你还记得我吗？\",\"novel_id\":\"$novel_id\"}" \
  "$api/chat/$character_id/stream")
grep -Fq '林岚记得你，也愿意继续同行。' <<<"$stream"
grep -Fq 'event: done' <<<"$stream"
grep -Fq '"committed":true' <<<"$stream"

pause
history=$("${curl_cmd[@]}" "${auth[@]}" "$api/chat/$character_id/history")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['count']==2; assert {m['role'] for m in value['messages']}=={'user','character'}" <<<"$history"

pause
node=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/1")
node_id=$(json_get "value['id']" <<<"$node")

pause
choice=$("${curl_cmd[@]}" "${auth[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"novel_id\":\"$novel_id\",\"node_id\":\"$node_id\",\"choice_index\":0}" \
  "$api/narrative/choose")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['chapter_number']==1; assert len(value['world_state']['state']['choices'])==1" <<<"$choice"

pause
chapter_two=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/chapters/2")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['generated'] is True; assert '北塔深处' in value['content']" <<<"$chapter_two"
chapter_hash=$(printf '%s' "$chapter_two" | sha256sum | cut -d' ' -f1)

pause
"${curl_cmd[@]}" --output /dev/null "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' --data '{"current_chapter":2}' \
  "$api/progress/$novel_id"

docker restart novel-user-service novel-novel-service novel-agent-service novel-narrative-service novel-gateway >/dev/null
for _ in $(seq 1 60); do
  [ "$(docker inspect --format '{{.State.Health.Status}}' novel-gateway 2>/dev/null)" = healthy ] && break
  sleep 2
done
[ "$(docker inspect --format '{{.State.Health.Status}}' novel-gateway)" = healthy ]

resumed_canon_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT model_version || ':' || schema_version || ':' || prompt_version || ':' || md5(content::text) FROM canon_story_models WHERE novel_id = '$novel_id'")
[ "$resumed_canon_snapshot" = "$canon_snapshot" ]

pause
resumed_progress=$("${curl_cmd[@]}" "${auth[@]}" "$api/progress/$novel_id")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['current_chapter']==2; assert value['reader_identity']=='云舟'" <<<"$resumed_progress"

pause
resumed_chapter=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/chapters/2")
[ "$(printf '%s' "$resumed_chapter" | sha256sum | cut -d' ' -f1)" = "$chapter_hash" ]

pause
replay=$("${curl_cmd[@]}" "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $turn_id" \
  --data "{\"message\":\"你还记得我吗？\",\"novel_id\":\"$novel_id\"}" \
  "$api/chat/$character_id")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['committed'] is True; assert value['replayed'] is True" <<<"$replay"

pause
replayed_choice=$("${curl_cmd[@]}" "${auth[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"novel_id\":\"$novel_id\",\"node_id\":\"$node_id\",\"choice_index\":0}" \
  "$api/narrative/choose")
python3 -c "import json,sys; value=json.load(sys.stdin); assert len(value['world_state']['state']['choices'])==1" <<<"$replayed_choice"

printf 'core reader loop resumed after restart: novel=%s turn=%s\n' "$novel_id" "$turn_id"
