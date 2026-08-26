#!/usr/bin/env bash
set -euo pipefail

api=${E2E_API_URL:-http://127.0.0.1/api}
public_url=${E2E_PUBLIC_URL:-http://127.0.0.1}
stub=${E2E_STUB_URL:-http://127.0.0.1:18080}
email=admin@test.invalid
password='RuntimeSmokeOnly123!'
source_file=$(mktemp)
metrics_file=$(mktemp)
account_export_file=$(mktemp)
account_export_headers=$(mktemp)
trap 'rm -f "$source_file" "$metrics_file" "$account_export_file" "$account_export_headers"' EXIT
curl_cmd=(curl --connect-timeout 5 --max-time 120 --fail --silent --show-error)

json_get() {
  python3 -c "import json,sys; value=json.load(sys.stdin); print($1)"
}

journey_memory_id() {
  python3 -c 'import hashlib,sys,uuid; namespace=uuid.UUID("4d5f215d-111c-5f25-8614-71e85f8a3e63"); source=uuid.UUID(sys.argv[1]); value=bytearray(hashlib.sha1(namespace.bytes+source.bytes).digest()[:16]); value[6]=(value[6]&15)|80; value[8]=(value[8]&63)|128; print(uuid.UUID(bytes=bytes(value)))' "$1"
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
old_refresh_token=$refresh_token

pause
rotated=$("${curl_cmd[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"refresh_token\":\"$old_refresh_token\"}" \
  "$api/auth/refresh")
token=$(json_get "value['access_token']" <<<"$rotated")
refresh_token=$(json_get "value['refresh_token']" <<<"$rotated")
auth=(-H "Authorization: Bearer $token")

pause
test "$(curl --silent --output /dev/null --write-out '%{http_code}' \
  -H 'Content-Type: application/json' \
  --data "{\"refresh_token\":\"$old_refresh_token\"}" \
  "$api/auth/refresh")" = 401

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
[ "$retried" = false ]
[ "$("${curl_cmd[@]}" "$stub/__control__/stats" | json_get "value['failures_remaining']['canon']")" = 0 ]

canon_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT model_version || ':' || schema_version || ':' || prompt_version || ':' || md5(content::text) FROM canon_story_models WHERE novel_id = '$novel_id'")
[[ "$canon_snapshot" == 1:1:canon-chunk-v3:* ]]

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
"${curl_cmd[@]}" --output /dev/null "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' --data '{"current_chapter":2}' \
  "$api/progress/$novel_id"

pause
blocked_branch_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  "$api/narrative/$novel_id/1")
[ "$blocked_branch_status" = 409 ]

pause
player_entry=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/player-entry?checkpoint_chapter=1")
location_id=$(json_get "value['locations'][0]['id']" <<<"$player_entry")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['player'] is None; assert value['checkpoint_chapter']==1; assert len(value['locations'])==1" <<<"$player_entry"
player_definition="{\"checkpoint_chapter\":1,\"name\":\"云舟\",\"background\":\"来自边城的地图学徒。\",\"capabilities\":[\"辨认古地图\"],\"location_id\":\"$location_id\",\"inventory\":[\"旧地图\"]}"

pause
player=$("${curl_cmd[@]}" "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' \
  --data "$player_definition" \
  "$api/narrative/$novel_id/player-entry")
player_id=$(json_get "value['player']['id']" <<<"$player")
python3 -c "import json,sys; value=json.load(sys.stdin); player=value['player']; assert value['checkpoint_chapter']==1; assert player['name']=='云舟'; assert player['location_id']=='$location_id'; assert player['relationships']=={}" <<<"$player"

pause
same_player=$("${curl_cmd[@]}" "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' \
  --data "$player_definition" \
  "$api/narrative/$novel_id/player-entry")
[ "$(json_get "value['player']['id']" <<<"$same_player")" = "$player_id" ]
pause
conflict_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' \
  --data "{\"checkpoint_chapter\":1,\"name\":\"另一名玩家\",\"background\":\"来自另一条时间线。\",\"capabilities\":[\"观察\"],\"location_id\":\"$location_id\",\"inventory\":[]}" \
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
choice_conflict_file=$(mktemp)
rewind_response_file=$(mktemp)
trap 'rm -f "$source_file" "$failed_choice_file" "$choice_conflict_file" "$rewind_response_file" "$metrics_file" "$account_export_file" "$account_export_headers"' EXIT
failed_choice_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output "$failed_choice_file" --write-out '%{http_code}' "${auth[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"novel_id\":\"$novel_id\",\"node_id\":\"$node_id\",\"choice_index\":0}" \
  "$api/narrative/choose")
[ "$failed_choice_status" = 502 ]
[ "$(json_get "value['error']['code']" <"$failed_choice_file")" = llm_error ]
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
transition_calls_before=$(curl --silent "$stub/__control__/stats" |
  json_get "value['calls'].get('narrative_transition', 0)")
choice_conflict_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output "$choice_conflict_file" --write-out '%{http_code}' "${auth[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"novel_id\":\"$novel_id\",\"node_id\":\"$node_id\",\"choice_index\":1}" \
  "$api/narrative/choose")
[ "$choice_conflict_status" = 409 ]
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['error']['code']=='choice_conflict'" \
  <"$choice_conflict_file"
transition_calls_after=$(curl --silent "$stub/__control__/stats" |
  json_get "value['calls'].get('narrative_transition', 0)")
[ "$transition_calls_after" = "$transition_calls_before" ]
conflict_transition_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT md5(transition::text) || ':' || md5((SELECT state::text FROM world_states WHERE user_id = user_choices.user_id AND novel_id = user_choices.novel_id)) FROM user_choices WHERE node_id = '$node_id'")
[ "$conflict_transition_snapshot" = "$transition_snapshot" ]

pause
chapter_two=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/chapters/2")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['generated'] is True; assert '北塔深处' in value['content']" <<<"$chapter_two"
chapter_hash=$(printf '%s' "$chapter_two" | sha256sum | cut -d' ' -f1)

pause
open_world=$("${curl_cmd[@]}" "${auth[@]}" -X POST "$api/narrative/$novel_id/world")
canon_event_id=$(json_get "value['session']['canonical_events'][0]['id']" <<<"$open_world")
python3 -c "import json,sys; value=json.load(sys.stdin); session=value['session']; assert value['player']['id']=='$player_id'; assert session['entry_context']['checkpoint_chapter']==1; assert session['entry_context']['unlocked_through_chapter']==2; assert len(session['canonical_events'])==1; assert session['canonical_events'][0]['source_chapters']==[2]; assert session['canonical_events'][0]['status']=='scheduled'; assert session['turn_number']==0" <<<"$open_world"

pause
same_open_world=$("${curl_cmd[@]}" "${auth[@]}" -X POST "$api/narrative/$novel_id/world")
[ "$(printf '%s' "$same_open_world" | sha256sum | cut -d' ' -f1)" = "$(printf '%s' "$open_world" | sha256sum | cut -d' ' -f1)" ]

world_turn_one_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
world_action_one="{\"expected_turn_number\":0,\"kind\":\"investigate\",\"target_id\":\"$canon_event_id\",\"intent\":\"查清北塔换防并阻止伏击\"}"
pause
world_turn_one=$("${curl_cmd[@]}" "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $world_turn_one_id" \
  --data "$world_action_one" "$api/narrative/$novel_id/world/turns")
world_turn_one_hash=$(printf '%s' "$world_turn_one" | sha256sum | cut -d' ' -f1)
python3 -c "import json,sys; value=json.load(sys.stdin); transition=value['transition']; session=value['world_state']['state']['open_world']; assert value['turn_id']=='$world_turn_one_id'; assert value['memory_projection_status']=='saved'; assert transition['canonical_event_change']['event_id']=='$canon_event_id'; assert transition['canonical_event_change']['status']=='obstructed'; assert transition['events'][0]['actor_character_ids']==['$character_id']; assert session['turn_number']==1; assert session['canonical_events'][0]['status']=='obstructed'" <<<"$world_turn_one"
journey_memory_one_id=$(journey_memory_id "$world_turn_one_id")
journey_memory_one=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT jsonb_build_object('id', id, 'character_id', character_id, 'user_id', user_id, 'novel_id', novel_id, 'layer', layer, 'importance', importance, 'chapter_number', chapter_number, 'fact', content::jsonb)::text FROM character_memories WHERE id = '$journey_memory_one_id'")
python3 -c "import json,sys; value=json.load(sys.stdin); fact=value['fact']; assert value['id']=='$journey_memory_one_id'; assert value['character_id']=='$character_id'; assert value['user_id']=='$user_id'; assert value['novel_id']=='$novel_id'; assert value['layer']=='permanent'; assert value['importance']==7; assert value['chapter_number']==2; assert fact['schema_version']==2; assert fact['source']=='committed_world_turn'; assert fact['authority']=='explicit_character_witness_facts'; assert fact['source_turn_id']=='$world_turn_one_id'; assert fact['witness_character_id']=='$character_id'; assert fact['turn_number']==1; assert fact['world_time']==1; assert fact['change_counts']=={'events':1,'relationships':1,'reader_action':0}" <<<"$journey_memory_one"

pause
replayed_world_turn_one=$("${curl_cmd[@]}" "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $world_turn_one_id" \
  --data "$world_action_one" "$api/narrative/$novel_id/world/turns")
[ "$(printf '%s' "$replayed_world_turn_one" | sha256sum | cut -d' ' -f1)" = "$world_turn_one_hash" ]

pause
test "$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $world_turn_one_id" \
  --data "{\"expected_turn_number\":0,\"kind\":\"investigate\",\"target_id\":\"$canon_event_id\",\"intent\":\"提交冲突的另一项行动\"}" \
  "$api/narrative/$novel_id/world/turns")" = 409

world_turn_two_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
world_action_two='{"expected_turn_number":1,"kind":"pursue_goal","target_id":null,"intent":"绘制地下回廊并寻找守门人的踪迹"}'
pause
world_turn_two=$("${curl_cmd[@]}" "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $world_turn_two_id" \
  --data "$world_action_two" "$api/narrative/$novel_id/world/turns")
python3 -c "import json,sys; value=json.load(sys.stdin); session=value['world_state']['state']['open_world']; assert value['turn_id']=='$world_turn_two_id'; assert value['memory_projection_status']=='saved'; assert value['transition']['canonical_event_change'] is None; assert session['turn_number']==2; assert session['world_time']==2" <<<"$world_turn_two"

pause
world_view=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/world")
python3 -c "import json,sys; value=json.load(sys.stdin); state=value['world_state']['state']; assert value['session']['turn_number']==2; assert value['session']['canonical_events'][0]['status']=='obstructed'; assert [entry['turn_number'] for entry in value['journal']]==[1,2]; assert len([event for event in state['world_events'] if isinstance(event,dict) and event.get('origin')=='player'])==2" <<<"$world_view"
world_view_hash=$(printf '%s' "$world_view" | sha256sum | cut -d' ' -f1)
player_entity_hash=$(python3 -c 'import hashlib,json,sys; value=json.load(sys.stdin); print(hashlib.sha256(json.dumps(value["player"],ensure_ascii=False,sort_keys=True,separators=(",",":")).encode()).hexdigest())' <<<"$world_view")
player_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT md5((state -> 'player_entity')::text) FROM world_states WHERE user_id = (SELECT id FROM users WHERE email = '$email') AND novel_id = '$novel_id'")
transition_snapshot=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT md5(transition::text) || ':' || md5((SELECT state::text FROM world_states WHERE user_id = user_choices.user_id AND novel_id = user_choices.novel_id)) FROM user_choices WHERE node_id = '$node_id'")

pause
"${curl_cmd[@]}" --output /dev/null "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' --data '{"current_chapter":1}' \
  "$api/progress/$novel_id"

pause
rewind_world_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output "$rewind_response_file" --write-out '%{http_code}' "${auth[@]}" \
  "$api/narrative/$novel_id/world")
[ "$rewind_world_status" = 409 ]
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['error']['code']=='reading_progress_behind_world'; assert not ({'session','journal','world_state'} & value.keys())" \
  <"$rewind_response_file"

pause
rewind_state_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output "$rewind_response_file" --write-out '%{http_code}' "${auth[@]}" \
  "$api/narrative/$novel_id/world-state")
[ "$rewind_state_status" = 409 ]
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['error']['code']=='reading_progress_behind_world'; assert 'state' not in value" \
  <"$rewind_response_file"

pause
rewind_player_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output "$rewind_response_file" --write-out '%{http_code}' "${auth[@]}" \
  "$api/narrative/$novel_id/player-entry")
[ "$rewind_player_status" = 409 ]
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['error']['code']=='reading_progress_behind_world'; assert not ({'player','locations','checkpoint_chapter'} & value.keys())" \
  <"$rewind_response_file"

pause
rewind_player_replay_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output "$rewind_response_file" --write-out '%{http_code}' "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' --data "$player_definition" \
  "$api/narrative/$novel_id/player-entry")
[ "$rewind_player_replay_status" = 409 ]
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['error']['code']=='reading_progress_behind_world'; assert not ({'player','locations','checkpoint_chapter'} & value.keys())" \
  <"$rewind_response_file"

pause
rewind_effective_chapter=$("${curl_cmd[@]}" "${auth[@]}" \
  "$api/narrative/$novel_id/chapters/2")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['generated'] is False; assert '北塔深处' not in value['content']" \
  <<<"$rewind_effective_chapter"

pause
rewind_replay_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output "$rewind_response_file" --write-out '%{http_code}' "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $world_turn_one_id" \
  --data "$world_action_one" "$api/narrative/$novel_id/world/turns")
[ "$rewind_replay_status" = 409 ]
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['error']['code']=='reading_progress_behind_world'; assert not ({'turn_id','transition','world_state'} & value.keys())" \
  <"$rewind_response_file"

rewind_turn_count_before=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT COUNT(*) FROM world_turns WHERE user_id = '$user_id' AND novel_id = '$novel_id'")
rewind_llm_calls_before=$(curl --silent "$stub/__control__/stats" |
  json_get "value['calls'].get('world_turn', 0)")
rewind_new_turn_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
pause
rewind_new_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output "$rewind_response_file" --write-out '%{http_code}' "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $rewind_new_turn_id" \
  --data '{"expected_turn_number":2,"kind":"pursue_goal","target_id":null,"intent":"回退后不应执行的行动"}' \
  "$api/narrative/$novel_id/world/turns")
[ "$rewind_new_status" = 409 ]
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['error']['code']=='reading_progress_behind_world'" \
  <"$rewind_response_file"
rewind_turn_count_after=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT COUNT(*) FROM world_turns WHERE user_id = '$user_id' AND novel_id = '$novel_id'")
rewind_llm_calls_after=$(curl --silent "$stub/__control__/stats" |
  json_get "value['calls'].get('world_turn', 0)")
[ "$rewind_turn_count_after" = "$rewind_turn_count_before" ]
[ "$rewind_llm_calls_after" = "$rewind_llm_calls_before" ]

pause
"${curl_cmd[@]}" --output /dev/null "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' --data '{"current_chapter":2}' \
  "$api/progress/$novel_id"
pause
restored_player=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/player-entry")
[ "$(python3 -c 'import hashlib,json,sys; value=json.load(sys.stdin); print(hashlib.sha256(json.dumps(value["player"],ensure_ascii=False,sort_keys=True,separators=(",",":")).encode()).hexdigest())' <<<"$restored_player")" = "$player_entity_hash" ]
pause
restored_player_replay=$("${curl_cmd[@]}" "${auth[@]}" \
  -X PUT -H 'Content-Type: application/json' --data "$player_definition" \
  "$api/narrative/$novel_id/player-entry")
[ "$(python3 -c 'import hashlib,json,sys; value=json.load(sys.stdin); print(hashlib.sha256(json.dumps(value["player"],ensure_ascii=False,sort_keys=True,separators=(",",":")).encode()).hexdigest())' <<<"$restored_player_replay")" = "$player_entity_hash" ]
pause
restored_effective_chapter=$("${curl_cmd[@]}" "${auth[@]}" \
  "$api/narrative/$novel_id/chapters/2")
[ "$(printf '%s' "$restored_effective_chapter" | sha256sum | cut -d' ' -f1)" = "$chapter_hash" ]
pause
restored_world_turn_one=$("${curl_cmd[@]}" "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $world_turn_one_id" \
  --data "$world_action_one" "$api/narrative/$novel_id/world/turns")
[ "$(printf '%s' "$restored_world_turn_one" | sha256sum | cut -d' ' -f1)" = "$world_turn_one_hash" ]

world_chat_turn_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
pause
world_chat=$("${curl_cmd[@]}" --no-buffer "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $world_chat_turn_id" \
  --data "{\"message\":\"这两回合之后你准备做什么？\",\"novel_id\":\"$novel_id\"}" \
  "$api/chat/$character_id/stream")
grep -Fq '林岚知道你已经改变了两回合的世界，也会依照自己的目标继续调查。' <<<"$world_chat"
grep -Fq 'event: done' <<<"$world_chat"

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
  --policy tools/llm-budget/policy-v2.json \
  --metrics "$metrics_file" \
  --commit "$(git rev-parse HEAD)"
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "$public_url/metrics")" = 404

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
resumed_world=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/world")
[ "$(printf '%s' "$resumed_world" | sha256sum | cut -d' ' -f1)" = "$world_view_hash" ]

pause
resumed_world_replay=$("${curl_cmd[@]}" "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $world_turn_one_id" \
  --data "$world_action_one" "$api/narrative/$novel_id/world/turns")
[ "$(printf '%s' "$resumed_world_replay" | sha256sum | cut -d' ' -f1)" = "$world_turn_one_hash" ]

resumed_world_chat_turn_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
pause
resumed_world_chat=$("${curl_cmd[@]}" --no-buffer "${auth[@]}" \
  -H 'Content-Type: application/json' -H "Idempotency-Key: $resumed_world_chat_turn_id" \
  --data "{\"message\":\"重启后，你还记得我在北塔作出的选择吗？\",\"novel_id\":\"$novel_id\"}" \
  "$api/chat/$character_id/stream")
grep -Fq '林岚知道你已经改变了两回合的世界，也会依照自己的目标继续调查。' <<<"$resumed_world_chat"
grep -Fq 'event: done' <<<"$resumed_world_chat"

pause
current_world=$("${curl_cmd[@]}" "${auth[@]}" "$api/narrative/$novel_id/world")
python3 -c "import json,sys; value=json.load(sys.stdin); assert value['session']['turn_number']==2; assert len(value['journal'])==2" <<<"$current_world"

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
python3 -c "import json,sys; value=json.load(sys.stdin); state=value['world_state']['state']; assert len(state['choices'])==1; assert len(state['world_events'])==3; assert len([event for event in state['world_events'] if event.get('origin')=='player'])==2; assert 'relationships' not in state; assert next(iter(state['player_entity']['relationships'].values()))['score']==57; assert state['open_world']['turn_number']==2" <<<"$replayed_choice"

privacy_turn_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -v ON_ERROR_STOP=1 \
  -c "UPDATE novels SET original_file_key = 'SENTINEL_E2E_OBJECT_KEY' WHERE id = '$novel_id'; INSERT INTO character_relationships (novel_id, from_character_id, to_character_id, relationship_type, description) VALUES ('$novel_id', '$character_id', '$character_id', 'self', 'Portable E2E relationship'); INSERT INTO character_memories (character_id, user_id, novel_id, layer, content, importance, chapter_number) VALUES ('$character_id', '$user_id', '$novel_id', 'long', 'Portable E2E memory', 8, 1); INSERT INTO chat_turns (id, user_id, character_id, novel_id, request_fingerprint, chapter_context, reader_identity_type, deviation_mode, status, failure_code) VALUES ('$privacy_turn_id', '$user_id', '$character_id', '$novel_id', decode(repeat('5a', 32), 'hex'), 1, 'self', 'canon', 'failed', 'SENTINEL_E2E_CHAT_FAILURE');" >/dev/null

for target in \
  'novel-user-service:8001' \
  'novel-novel-service:8002' \
  'novel-agent-service:8003' \
  'novel-narrative-service:8004'; do
  container=${target%%:*}
  port=${target##*:}
  test "$(docker exec "$container" curl --silent --output /dev/null --write-out '%{http_code}' \
    -H 'X-Internal-Service-Token: wrong-token' \
    "http://127.0.0.1:$port/internal/privacy/users/$user_id/export")" = 401
done
pause
"${curl_cmd[@]}" "${auth[@]}" --dump-header "$account_export_headers" \
  --output "$account_export_file" "$api/account/export"
grep -Eiq '^content-type: application/x-ndjson' "$account_export_headers"
grep -Eiq '^cache-control: no-store' "$account_export_headers"
grep -Eiq '^content-disposition: attachment;' "$account_export_headers"
! grep -Eiq '^content-length:' "$account_export_headers"
python3 - "$account_export_file" "$user_id" "$refresh_token" "$token" <<'PY'
import json
import os
import pathlib
import sys

path, user_id, refresh_token, access_token = sys.argv[1:]
raw = pathlib.Path(path).read_text()
records = [json.loads(line) for line in raw.splitlines() if line]
assert records[0]["type"] == "manifest"
assert records[0]["schema"] == "account-export-v1"
assert records[0]["user_id"] == user_id
assert records[0]["snapshot"] == "service-local"
assert records[-1] == {
    "schema": "account-export-v1",
    "services": ["user", "novel", "agent", "narrative"],
    "type": "complete",
}

expected_services = ["user", "novel", "agent", "narrative"]
completed = []
active = None
kinds = set()
all_keys = set()

def visit(value):
    if isinstance(value, dict):
        all_keys.update(value)
        for child in value.values():
            visit(child)
    elif isinstance(value, list):
        for child in value:
            visit(child)

for record in records[1:-1]:
    visit(record)
    event = record["type"]
    if event == "service_start":
        assert active is None
        active = record["service"]
        assert active == expected_services[len(completed)]
    elif event == "record":
        assert record["service"] == active
        kinds.add(record["kind"])
    elif event == "service_complete":
        assert record["service"] == active
        completed.append(active)
        active = None
    else:
        raise AssertionError(event)

assert active is None
assert completed == expected_services
assert {
    "profile", "novel", "chapter", "character", "character_relationship",
    "canon_story_model", "reading_progress", "chat_message", "character_memory",
    "narrative_node", "user_choice", "world_state", "player_chapter", "world_turn",
} <= kinds
assert "Portable E2E relationship" in raw
assert "Portable E2E memory" in raw
for secret in [
    "SENTINEL_E2E_OBJECT_KEY",
    "SENTINEL_E2E_CHAT_FAILURE",
    "RuntimeSmokeOnly123!",
    refresh_token,
    access_token,
    os.environ["JWT_SECRET"],
    os.environ["INTERNAL_SERVICE_TOKEN"],
    os.environ["LLM_API_KEY"],
]:
    assert secret not in raw
assert {
    "password_hash", "original_file_key", "request_fingerprint", "failure_code",
    "embedding", "access_count", "last_accessed", "expires_at",
}.isdisjoint(all_keys)
PY
pause
test "$(curl --silent --output /dev/null --write-out '%{http_code}' \
  "${auth[@]}" "$api/internal/privacy/users/$user_id/export")" = 404
pause
test "$(curl --path-as-is --silent --output /dev/null --write-out '%{http_code}' \
  "${auth[@]}" -H "X-Internal-Service-Token: $INTERNAL_SERVICE_TOKEN" \
  "$api/users/%2e%2e/internal/privacy/users/$user_id/export")" = 404

delete_novel_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
delete_character_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
delete_message_id=$(python3 -c 'import uuid; print(uuid.uuid4())')
docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -v ON_ERROR_STOP=1 \
  -c "INSERT INTO novels (id, user_id, title, status) VALUES ('$delete_novel_id', '$user_id', 'Deletion contract', 'ready'); INSERT INTO user_novels (user_id, novel_id) VALUES ('$user_id', '$delete_novel_id'); INSERT INTO characters (id, novel_id, name) VALUES ('$delete_character_id', '$delete_novel_id', 'Deletion witness');" >/dev/null
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
  -c "SELECT (SELECT COUNT(*) FROM novels WHERE id = '$delete_novel_id') || ':' || (SELECT COUNT(*) FROM characters WHERE id = '$delete_character_id') || ':' || (SELECT COUNT(*) FROM user_novels WHERE user_id = '$user_id' AND novel_id = '$delete_novel_id')")" = 1:1:0
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

erased_private_counts=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT (SELECT COUNT(*) FROM users WHERE id = '$user_id') || ':' || (SELECT COUNT(*) FROM user_novels WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM character_memories WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM chat_turns WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM chat_messages WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM narrative_nodes WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM user_choices WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM world_states WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM world_turns WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM player_chapters WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM reading_progress WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM refresh_tokens WHERE user_id = '$user_id') || ':' || (SELECT COUNT(*) FROM user_llm_configs WHERE user_id = '$user_id')")
[ "$erased_private_counts" = 0:0:0:0:0:0:0:0:0:0:0:0:0 ]
test "$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT COUNT(*) FROM runtime_llm_config")" = 0
retained_canonical_counts=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT (SELECT COUNT(*) FROM novels WHERE id IN ('$novel_id', '$delete_novel_id')) || ':' || (SELECT COUNT(*) FROM novel_import_jobs WHERE novel_id = '$novel_id') || ':' || (SELECT COUNT(*) FROM chapters WHERE novel_id = '$novel_id') || ':' || (SELECT COUNT(*) FROM chapter_chunks AS chunk JOIN chapters AS chapter ON chapter.id = chunk.chapter_id WHERE chapter.novel_id = '$novel_id') || ':' || (SELECT COUNT(*) FROM characters WHERE novel_id IN ('$novel_id', '$delete_novel_id')) || ':' || (SELECT COUNT(*) FROM character_relationships WHERE novel_id = '$novel_id') || ':' || (SELECT COUNT(*) FROM canon_story_models WHERE novel_id = '$novel_id')")
[ "$retained_canonical_counts" = 2:1:2:2:2:1:1 ]
erasure_counts=$(docker exec novel-postgres psql \
  -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
  -c "SELECT (SELECT COUNT(*) FROM erasure_records WHERE subject_type = 'user' AND subject_id = '$user_id') || ':' || (SELECT COUNT(*) FROM erasure_records WHERE subject_type = 'novel' AND subject_id IN ('$novel_id', '$delete_novel_id'))")
[ "$erasure_counts" = 1:0 ]

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

printf 'core reader loop and living world resumed, then account data erased: novel=%s chat_turn=%s world_turns=%s,%s\n' "$novel_id" "$turn_id" "$world_turn_one_id" "$world_turn_two_id"
