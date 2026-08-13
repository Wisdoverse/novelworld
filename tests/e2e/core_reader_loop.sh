#!/usr/bin/env bash
set -euo pipefail

api=${E2E_API_URL:-http://127.0.0.1/api}
public_url=${E2E_PUBLIC_URL:-http://127.0.0.1}
email=admin@test.invalid
password='RuntimeSmokeOnly123!'
source_file=$(mktemp)
metrics_file=$(mktemp)
trap 'rm -f "$source_file" "$metrics_file"' EXIT
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
refresh_token=$(json_get "value['refresh_token']" <<<"$login")
user_id=$(json_get "value['user']['id']" <<<"$login")
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
blocked_branch_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  "$api/narrative/$novel_id/1")
[ "$blocked_branch_status" = 409 ]

pause
player_entry=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/player-entry")
location_id=$(json_get "value['locations'][0]['id']" <<<"$player_entry")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['player'] is None; assert value['checkpoint_chapter']==1; assert len(value['locations'])==1" <<<"$player_entry"

pause
player=$("${curl_cmd[@]}" "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' \
  --data "{\"name\":\"云舟\",\"background\":\"来自边城的地图学徒。\",\"capabilities\":[\"辨认古地图\"],\"location_id\":\"$location_id\",\"inventory\":[\"旧地图\"]}" \
  "$api/narrative/$novel_id/player-entry")
player_id=$(json_get "value['player']['id']" <<<"$player")
python3 -c "import json,sys; value=json.load(sys.stdin); player=value['player']; assert value['checkpoint_chapter']==1; assert player['name']=='云舟'; assert player['location_id']=='$location_id'; assert player['relationships']=={}" <<<"$player"

pause
same_player=$("${curl_cmd[@]}" "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' \
  --data "{\"name\":\"云舟\",\"background\":\"来自边城的地图学徒。\",\"capabilities\":[\"辨认古地图\"],\"location_id\":\"$location_id\",\"inventory\":[\"旧地图\"]}" \
  "$api/narrative/$novel_id/player-entry")
[ "$(json_get "value['player']['id']" <<<"$same_player")" = "$player_id" ]
pause
conflict_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' \
  --data "{\"name\":\"另一名玩家\",\"background\":\"来自另一条时间线。\",\"capabilities\":[\"观察\"],\"location_id\":\"$location_id\",\"inventory\":[]}" \
  "$api/narrative/$novel_id/player-entry")
[ "$conflict_status" = 409 ]
player_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT md5((state -> 'player_entity')::text) FROM world_states WHERE user_id = (SELECT id FROM users WHERE email = '$email') AND novel_id = '$novel_id'")
[[ "$player_snapshot" =~ ^[0-9a-f]{32}$ ]]

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
failed_choice_file=$(mktemp)
trap 'rm -f "$source_file" "$failed_choice_file" "$metrics_file"' EXIT
failed_choice_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output "$failed_choice_file" --write-out '%{http_code}' "${auth[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"novel_id\":\"$novel_id\",\"node_id\":\"$node_id\",\"choice_index\":0}" \
  "$api/narrative/choose")
[ "$failed_choice_status" = 502 ]
failed_writes=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT (SELECT COUNT(*) FROM user_choices WHERE user_id = (SELECT id FROM users WHERE email = '$email') AND node_id = '$node_id') || ':' || (SELECT COUNT(*) FROM player_chapters WHERE user_id = (SELECT id FROM users WHERE email = '$email') AND novel_id = '$novel_id' AND chapter_number = 1) || ':' || (SELECT jsonb_array_length(state -> 'choices') FROM world_states WHERE user_id = (SELECT id FROM users WHERE email = '$email') AND novel_id = '$novel_id')")
[ "$failed_writes" = 0:0:0 ]

pause
choice=$("${curl_cmd[@]}" "${auth[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"novel_id\":\"$novel_id\",\"node_id\":\"$node_id\",\"choice_index\":0}" \
  "$api/narrative/choose")
python3 -c "import json,sys; value=json.load(sys.stdin); transition=value['transition']; state=value['world_state']['state']; assert value['chapter_number']==1; assert transition['schema_version']==1; assert transition['canon_model_version']==1; assert transition['canonical_checkpoint_chapter']==1; assert value['consequence']==transition['rendered_narrative']; assert len(state['choices'])==1; assert len(state['world_events'])==1; assert 'relationships' not in state; assert len(state['player_entity']['relationships'])==1; assert len(state['locations'])==1; assert len(state['threads'])==1" <<<"$choice"
player_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT md5((state -> 'player_entity')::text) FROM world_states WHERE user_id = (SELECT id FROM users WHERE email = '$email') AND novel_id = '$novel_id'")

transition_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT md5(transition::text) || ':' || md5((SELECT state::text FROM world_states WHERE user_id = user_choices.user_id AND novel_id = user_choices.novel_id)) FROM user_choices WHERE node_id = '$node_id'")

pause
chapter_two=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/chapters/2")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['generated'] is True; assert '北塔深处' in value['content']" <<<"$chapter_two"
chapter_hash=$(printf '%s' "$chapter_two" | sha256sum | cut -d' ' -f1)

for target in \
  'novel-user-service:8001' \
  'novel-novel-service:8002' \
  'novel-agent-service:8003' \
  'novel-narrative-service:8004'; do
  container=${target%%:*}
  port=${target##*:}
  docker exec "$container" curl --fail --silent "http://127.0.0.1:$port/metrics" >>"$metrics_file"
  printf '\n' >>"$metrics_file"
done
grep -Fq 'type="cached_input"' "$metrics_file"
python3 tools/llm-budget/verify.py \
  --policy tools/llm-budget/policy-v1.json \
  --metrics "$metrics_file" \
  --commit "$(git rev-parse HEAD)"
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "$public_url/metrics")" = 404

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
resumed_transition_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT md5(transition::text) || ':' || md5((SELECT state::text FROM world_states WHERE user_id = user_choices.user_id AND novel_id = user_choices.novel_id)) FROM user_choices WHERE node_id = '$node_id'")
[ "$resumed_transition_snapshot" = "$transition_snapshot" ]
resumed_player_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT md5((state -> 'player_entity')::text) FROM world_states WHERE user_id = (SELECT id FROM users WHERE email = '$email') AND novel_id = '$novel_id'")
[ "$resumed_player_snapshot" = "$player_snapshot" ]

pause
resumed_player=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/player-entry")
[ "$(json_get "value['player']['id']" <<<"$resumed_player")" = "$player_id" ]

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
python3 -c "import json,sys; value=json.load(sys.stdin); state=value['world_state']['state']; assert len(state['choices'])==1; assert len(state['world_events'])==1; assert 'relationships' not in state; assert next(iter(state['player_entity']['relationships'].values()))['score']==55" <<<"$replayed_choice"

delete_novel_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
delete_character_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
delete_message_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -v ON_ERROR_STOP=1 \
  -c "INSERT INTO novels (id, user_id, title, status) VALUES ('$delete_novel_id', '$user_id', 'Deletion contract', 'ready'); INSERT INTO characters (id, novel_id, name) VALUES ('$delete_character_id', '$delete_novel_id', 'Deletion witness');" >/dev/null
delete_cache_message=$(python3 -c "import json; print(json.dumps({'id':'$delete_message_id','turn_id':None,'user_id':'$user_id','character_id':'$delete_character_id','novel_id':'$delete_novel_id','role':'user','content':'delete this projection','reader_identity':None,'chapter_context':1,'created_at':'2026-01-01T00:00:00Z'}, separators=(',',':')))")
docker exec novel-redis redis-cli --no-auth-warning -a "${REDIS_PASSWORD:-runtime-redis-only}" \
  LPUSH "chat:$delete_character_id:$user_id" "$delete_cache_message" >/dev/null
pause
test "$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  -X DELETE "$api/novels/$delete_novel_id")" = 204
test "$(docker exec novel-redis redis-cli --no-auth-warning -a "${REDIS_PASSWORD:-runtime-redis-only}" EXISTS "chat:$delete_character_id:$user_id")" = 0
test "$(docker exec novel-redis redis-cli --no-auth-warning -a "${REDIS_PASSWORD:-runtime-redis-only}" EXISTS "chat:$character_id:$user_id")" = 1
test "$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT (SELECT COUNT(*) FROM novels WHERE id = '$delete_novel_id') || ':' || (SELECT COUNT(*) FROM characters WHERE id = '$delete_character_id')")" = 0:0
pause
test "$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  -X DELETE "$api/novels/$delete_novel_id")" = 404

test "$(docker exec novel-redis redis-cli --no-auth-warning -a "${REDIS_PASSWORD:-runtime-redis-only}" EXISTS "chat:$character_id:$user_id")" = 1
test "$(docker exec novel-agent-service curl --silent --output /dev/null --write-out '%{http_code}' \
  -X DELETE -H 'X-Internal-Service-Token: wrong-token' \
  "http://127.0.0.1:8003/internal/privacy/users/$user_id")" = 401
pause
test "$(curl --silent --output /dev/null --write-out '%{http_code}' \
  "${auth[@]}" -X DELETE "$api/internal/privacy/users/$user_id")" = 404
pause
test "$(curl --silent --output /dev/null --write-out '%{http_code}' \
  -X DELETE "$api/auth/me")" = 401

pause
test "$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  -X DELETE "$api/auth/me")" = 204
pause
test "$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  -X DELETE "$api/auth/me")" = 204
test "$(docker exec novel-redis redis-cli --no-auth-warning -a "${REDIS_PASSWORD:-runtime-redis-only}" EXISTS "chat:$character_id:$user_id")" = 0

erased_counts=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT (SELECT COUNT(*) FROM users) || ':' || (SELECT COUNT(*) FROM novels) || ':' || (SELECT COUNT(*) FROM chapters) || ':' || (SELECT COUNT(*) FROM chapter_chunks) || ':' || (SELECT COUNT(*) FROM characters) || ':' || (SELECT COUNT(*) FROM character_relationships) || ':' || (SELECT COUNT(*) FROM character_memories) || ':' || (SELECT COUNT(*) FROM chat_turns) || ':' || (SELECT COUNT(*) FROM chat_messages) || ':' || (SELECT COUNT(*) FROM narrative_nodes) || ':' || (SELECT COUNT(*) FROM user_choices) || ':' || (SELECT COUNT(*) FROM world_states) || ':' || (SELECT COUNT(*) FROM player_chapters) || ':' || (SELECT COUNT(*) FROM canon_story_models) || ':' || (SELECT COUNT(*) FROM reading_progress) || ':' || (SELECT COUNT(*) FROM refresh_tokens) || ':' || (SELECT COUNT(*) FROM runtime_llm_config)")
[ "$erased_counts" = 0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0:0 ]

pause
test "$(curl --silent --output /dev/null --write-out '%{http_code}' \
  -H 'Content-Type: application/json' \
  --data "{\"email\":\"$email\",\"password\":\"$password\"}" \
  "$api/auth/login")" = 401
pause
test "$(curl --silent --output /dev/null --write-out '%{http_code}' \
  -H 'Content-Type: application/json' \
  --data "{\"refresh_token\":\"$refresh_token\"}" \
  "$api/auth/refresh")" = 401
pause
setup_status=$("${curl_cmd[@]}" "$api/setup/status")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['configured'] is False; assert value['admin_configured'] is False" <<<"$setup_status"

printf 'core reader loop resumed and account data erased: novel=%s turn=%s\n' "$novel_id" "$turn_id"
