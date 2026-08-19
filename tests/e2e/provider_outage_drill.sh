#!/usr/bin/env bash
# Provider-outage drill (H2 incident-response scope): with the LLM provider
# unreachable, an import fails closed - a bounded, source-free public error,
# no crash, services stay healthy - and the deployment recovers (retry
# succeeds) once the provider returns. Also pins the settings
# non-disclosure surface: the settings API never returns key material.
# Credential rotation itself stays provider-gated and is recorded as such
# in SECURITY.md and DEPLOYMENT_PROFILE.md.
#
# Re-runnable on a first-run deployment or one whose admin is
# admin@test.invalid (the CI-seeded account); the stub is reset first, and
# the drill deletes its novels and its own admin, leaving the deployment in
# first-run state for the next drill.
set -euo pipefail
cd "$(dirname "$0")/../.."

api=${E2E_API_URL:-http://127.0.0.1/api}
stub=${E2E_STUB_URL:-http://127.0.0.1:18080}
email=admin@test.invalid
password='RuntimeSmokeOnly123!'
work=$(mktemp -d)
stub_was_stopped=0
cleanup() {
  rm -rf "$work"
  if [ "$stub_was_stopped" -eq 1 ] &&
    [ "$(docker inspect --format '{{.State.Running}}' "$container" 2>/dev/null)" = false ]; then
    docker start "$container" >/dev/null 2>&1 || true
    printf 'drill: note: restored the stopped provider stub\n' >&2
  fi
  # Best-effort return to first-run state if the drill created the admin.
  if [ -n "${token:-}" ] && [ "$admin_created" -eq 1 ]; then
    curl --silent --output /dev/null --max-time 10 \
      -H "Authorization: Bearer $token" -X DELETE "$api/auth/me" || true
  fi
}
trap cleanup EXIT
admin_created=0
curl_cmd=(curl --connect-timeout 5 --max-time 120 --fail --silent --show-error)

json_get() { python3 -c "import json,sys; value=json.load(sys.stdin); print($1)"; }

# The CI drills run with RATE_LIMIT_RPS=1; pace standalone API calls like the
# other e2e drills (the status polls already sleep 2s per attempt).
pause() { sleep 1.1; }

check() {
  if [ "$2" != "$3" ]; then
    printf 'drill: FAIL %s: expected [%s], got [%s]\n' "$1" "$2" "$3" >&2
    exit 1
  fi
  printf 'drill: ok   %s = %s\n' "$1" "$3"
}

stub_container() { docker ps --format '{{.Names}}' | grep 'llm-stub' | head -1; }

stub_reset() {
  for _ in $(seq 1 50); do
    active=$(curl --silent "$stub/__control__/stats" | json_get "sum(value['active'].values())" || true)
    [ "$active" = 0 ] && break
    sleep 0.2
  done
  curl --fail --silent -H 'Content-Type: application/json' \
    --data '{"delays_ms":{},"failures_remaining":{}}' "$stub/__control__/reset" >/dev/null
}

db() {
  docker exec novel-postgres psql -U "${POSTGRES_USER:-novel}" \
    -d "${POSTGRES_DB:-novel_world}" -At -v ON_ERROR_STOP=1 -c "$1"
}

wait_gateway_healthy() {
  for _ in $(seq 1 90); do
    [ "$(docker inspect --format '{{.State.Health.Status}}' novel-gateway 2>/dev/null)" = healthy ] && return 0
    sleep 2
  done
  docker compose ps --all >&2
  printf 'drill: the gateway never became healthy\n' >&2
  exit 1
}

write_source() {
  printf '%s\n' \
    '第一章 风暴前夜' \
    '林岚握紧手中的旧地图，望向被风暴笼罩的北塔，决定在天黑前寻找失踪的守门人。边城的钟声连续响了三次，街道上的人们纷纷关紧门窗。林岚仍站在石桥中央，逐一核对地图上的暗号，并请你留意河岸新出现的足迹。远处的塔灯忽明忽暗，仿佛有人正用最后的力气发出求救信号。你们约定不替彼此作决定，却要共同承担进入风暴的后果。' \
    '第二章 北塔回声' \
    '北塔的石门布满潮湿苔痕，林岚在门边发现守门人留下的铜铃。铃身刻着通往地下回廊的路线，也写明只有彼此信任的同行者才能安全通过。风暴压低了天空，城墙上的火把依次熄灭。你与林岚交换各自找到的线索，确认失踪并非意外。塔内传来沉重脚步，旧地图上从未标注的房间正在缓缓开启，而边城的命运也随这一刻发生变化。' \
    >"$1"
}

upload_novel() {
  local upload novel_id
  upload=$("${curl_cmd[@]}" "${auth[@]}" \
    -F "title=$1" \
    -F 'author=E2E' \
    -F 'deviation_mode=creative' \
    -F "file=@$2;filename=storm.txt;type=text/plain" \
    "$api/novels/upload")
  novel_id=$(json_get "value['novel_id']" <<<"$upload")
  printf '%s' "$novel_id"
}

novel_status() {
  json_get "value['status']" <<<"$("${curl_cmd[@]}" "${auth[@]}" "$api/novels/$1/status")"
}

wait_status() {
  local novel_id=$1 wanted=$2 attempts=$3 state=""
  for _ in $(seq 1 "$attempts"); do
    sleep 2
    state=$(novel_status "$novel_id")
    [ "$state" = "$wanted" ] && return 0
  done
  printf 'drill: novel %s stayed [%s] instead of [%s]\n' "$novel_id" "$state" "$wanted" >&2
  return 1
}

# ---- preconditions ---------------------------------------------------------
wait_gateway_healthy

# The shared gateway rate limiter can be cold right after the previous
# drill's last request (CI runs with RATE_LIMIT_RPS=1), so retry 429s.
for _ in $(seq 1 5); do
  pause
  setup_status=$("${curl_cmd[@]}" "$api/setup/status") && break
  setup_status=""
done
[ -n "$setup_status" ] || { printf 'drill: setup/status kept returning 429\n' >&2; exit 1; }
admin_configured=$(json_get "value['admin_configured']" <<<"$setup_status")
if [ "$admin_configured" != True ]; then
  pause
  status_code=$(curl --silent --output /dev/null --write-out '%{http_code}' \
    -H 'Content-Type: application/json' \
    --data "{\"email\":\"$email\",\"password\":\"$password\",\"name\":\"Runtime Admin\"}" \
    "$api/setup/init")
  [ "$status_code" = 201 ] || { printf 'drill: setup/init returned %s\n' "$status_code" >&2; exit 1; }
  admin_created=1
fi
pause
login=$("${curl_cmd[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"email\":\"$email\",\"password\":\"$password\"}" \
  "$api/auth/login")
token=$(json_get "value['access_token']" <<<"$login")
auth=(-H "Authorization: Bearer $token")

container=$(stub_container)
[ -n "$container" ] || { printf 'drill: llm-stub container not running\n' >&2; exit 1; }
stub_reset

# ---- baseline: a normal import completes while the provider is up --------
pause
source_a="$work/a.txt"
write_source "$source_a"
novel_a=$(upload_novel 'outage-baseline' "$source_a")
wait_status "$novel_a" ready 45 || exit 1
pause
check 'baseline import completed' "$(novel_status "$novel_a")" ready

# ---- outage: the provider disappears mid-flight ---------------------------
printf 'drill: stopping the LLM provider (simulated outage)\n'
docker stop "$container" >/dev/null
stub_was_stopped=1
pause
source_b="$work/b.txt"
write_source "$source_b"
novel_b=$(upload_novel 'outage-under-provider-failure' "$source_b")
wait_status "$novel_b" error 80 || exit 1
pause
check 'outage import reached the terminal error state' "$(novel_status "$novel_b")" error
check 'outage import failed with the bounded code' \
  "$(db "SELECT failure_code FROM novel_import_jobs WHERE novel_id = '$novel_b'")" processing_failed
check 'the import service stays healthy during the outage' \
  "$(docker inspect --format '{{.State.Health.Status}}' novel-novel-service)" healthy
pause
check 'baseline novel is untouched by the outage' "$(novel_status "$novel_a")" ready

# ---- non-disclosure: the settings API never returns key material ----------
pause
settings=$("${curl_cmd[@]}" "${auth[@]}" "$api/settings/llm")
python3 -c "import json,sys; body=json.load(sys.stdin); assert 'api_key' not in body, body; assert isinstance(body.get('api_key_configured'), bool)" <<<"$settings"
printf 'drill: ok   settings response carries no key material\n'

# ---- recovery: the provider returns and the import retries -----------------
printf 'drill: starting the LLM provider again\n'
docker start "$container" >/dev/null
for _ in $(seq 1 30); do
  curl --fail --silent --output /dev/null "$stub/health" && break
  sleep 1
done
stub_reset
pause
retry_status=$(curl --silent --output /dev/null --write-out '%{http_code}' \
  "${auth[@]}" -X POST "$api/novels/$novel_b/retry")
check 'failed import accepts a retry' "$retry_status" 202
wait_status "$novel_b" ready 45 || exit 1
pause
check 'recovered import completed after retry' "$(novel_status "$novel_b")" ready

# ---- cleanup ---------------------------------------------------------------
for novel_id in "$novel_a" "$novel_b"; do
  pause
  check "cleanup delete returns 204" \
    "$(curl --silent --output /dev/null --write-out '%{http_code}' "${auth[@]}" -X DELETE "$api/novels/$novel_id")" 204
done
# Return the deployment to first-run state (the next drill's precondition).
pause
check 'cleanup deletes the drill admin' \
  "$(curl --silent --output /dev/null --write-out '%{http_code}' "${auth[@]}" -X DELETE "$api/auth/me")" 204

printf 'drill: provider-outage drill passed\n'
