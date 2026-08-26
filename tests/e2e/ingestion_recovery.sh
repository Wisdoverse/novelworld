#!/usr/bin/env bash
# Live kill/restart drill for the durable import boundaries (H1 exit evidence).
#
# Hard-kills novel-service at the `chapters` and `enriched` stages while a
# provider call is in flight, restarts it, waits out the real lease fence, and
# proves the import resumes to exactly one authoritative result without
# re-upload. Provider calls are metered through the LLM stub's control
# endpoints, and completed work is proven to make no new provider call.
#
# Usage:
#   tests/e2e/ingestion_recovery.sh                 # against the running compose topology
#   tests/e2e/ingestion_recovery.sh --self-test     # verifier mutation checks, no topology
set -euo pipefail

if [ "${1:-}" = "--self-test" ]; then
  python3 - <<'PY'
import sys


def verify(record):
    problems = []

    def require(name, condition):
        if not condition:
            problems.append(name)

    phase = record["phase"]
    require("job completed", record["job_status"] == "completed")
    require("no failure code", record["failure_code"] is None)
    require("novel ready", record["novel_status"] == "ready")
    require("attempts inside import-provider-budget-v1",
            1 <= record["attempt"] <= 3)
    require("chapters exactly committed total",
            record["chapter_count"] == record["total_chapters"] == 2)
    require("exactly one character", record["character_count"] == 1)
    require("exactly one canon model", record["canon_count"] == 1)
    if phase == "enriched":
        require("character snapshot stable across resume",
                record["character_md5"] == record["character_md5_before"])
    if phase == "completed":
        require("no provider calls for completed work",
                record["calls_after"] == record["calls_before"] == 0)
        require("retry on ready rejected", record["retry_status"] == 409)
    return problems


base = {
    "phase": "chapters",
    "job_status": "completed",
    "failure_code": None,
    "novel_status": "ready",
    "chapter_count": 2,
    "total_chapters": 2,
    "character_count": 1,
    "canon_count": 1,
    "character_md5": "abc",
    "character_md5_before": "abc",
    "calls_before": 0,
    "calls_after": 0,
    "retry_status": 409,
    "attempt": 2,
}

tampered = [
    ("duplicated chapters must fail", dict(base, phase="chapters", chapter_count=3)),
    ("false ready must fail", dict(base, phase="chapters", novel_status="parsing")),
    ("duplicate canon must fail", dict(base, phase="enriched", canon_count=2)),
    ("character drift must fail", dict(base, phase="enriched", character_md5="xyz")),
    ("provider call for completed work must fail",
     dict(base, phase="completed", calls_after=1)),
    ("ready import retry must fail", dict(base, phase="completed", retry_status=200)),
    ("attempt beyond the budget ceiling must fail", dict(base, attempt=4)),
]
for label, record in tampered:
    if not verify(record):
        print(f"self-test failed: {label} passed the weakened verifier")
        sys.exit(1)
print("ingestion recovery verifier self-test passed")
PY
  exit 0
fi

api=${E2E_API_URL:-http://127.0.0.1/api}
email=admin@test.invalid
password='RuntimeSmokeOnly123!'
source_file="ingestion_recovery_source_$$.txt"
trap 'rm -f "$source_file"' EXIT
curl_cmd=(curl --connect-timeout 5 --max-time 120 --fail --silent --show-error)
stub=${E2E_STUB_URL:-http://127.0.0.1:18080}

printf '%s\n' \
  '第一章 风暴前夜' \
  '林岚握紧手中的旧地图，望向被风暴笼罩的北塔，决定在天黑前寻找失踪的守门人。边城的钟声连续响了三次，街道上的人们纷纷关紧门窗。林岚仍站在石桥中央，逐一核对地图上的暗号，并请你留意河岸新出现的足迹。远处的塔灯忽明忽暗，仿佛有人正用最后的力气发出求救信号。你们约定不替彼此作决定，却要共同承担进入风暴的后果。' \
  '第二章 北塔回声' \
  '北塔的石门布满潮湿苔痕，林岚在门边发现守门人留下的铜铃。铃身刻着通往地下回廊的路线，也写明只有彼此信任的同行者才能安全通过。风暴压低了天空，城墙上的火把依次熄灭。你与林岚交换各自找到的线索，确认失踪并非意外。塔内传来沉重脚步，旧地图上从未标注的房间正在缓缓开启，而边城的命运也随这一刻发生变化。' \
  >"$source_file"

json_get() {
  python3 -c "import json,sys; value=json.load(sys.stdin); print($1)"
}

stub_active() {
  curl --silent "$stub/__control__/stats" | json_get "sum(value['active'].values())"
}

stub_calls() {
  curl --silent "$stub/__control__/stats" | json_get "sum(value['calls'].values())"
}

stub_reset() {
  local payload=$1
  for _ in $(seq 1 100); do
    [ "$(stub_active)" = 0 ] && break
    sleep 0.2
  done
  curl --fail --silent -H 'Content-Type: application/json' \
    --data "$payload" "$stub/__control__/reset" >/dev/null
}

db() {
  docker exec novel-postgres psql \
    -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At -v ON_ERROR_STOP=1 \
    -c "$1"
}

wait_gateway_healthy() {
  for _ in $(seq 1 60); do
    [ "$(docker inspect --format '{{.State.Health.Status}}' novel-gateway 2>/dev/null)" = healthy ] && return 0
    sleep 2
  done
  printf 'gateway did not become healthy\n' >&2
  return 1
}

wait_job_stage() {
  local novel_id=$1 stage=$2 deadline=$3 state
  while [ "$(date +%s)" -lt "$deadline" ]; do
    state=$(db "SELECT stage || ':' || status FROM novel_import_jobs WHERE novel_id = '$novel_id'")
    [ "$state" = "$stage:in_progress" ] && return 0
    sleep 0.05
  done
  return 1
}

wait_ready() {
  local novel_id=$1 deadline=$2 state
  while [ "$(date +%s)" -lt "$deadline" ]; do
    state=$(db "SELECT status::text FROM novels WHERE id = '$novel_id'")
    [ "$state" = ready ] && return 0
    if [ "$state" = error ]; then
      printf 'novel %s failed: %s\n' "$novel_id" \
        "$(db "SELECT parse_error FROM novels WHERE id = '$novel_id'")" >&2
      return 1
    fi
    sleep 1
  done
  printf 'novel %s did not become ready\n' "$novel_id" >&2
  return 1
}

phase_record() {
  local novel_id=$1 phase=$2 character_md5_before=$3 calls_before=$4 calls_after=$5 retry_status=$6 state
  state=$(db "SELECT \
        job.attempt || ':' || \
        job.stage || ':' || job.status || ':' || COALESCE(job.failure_code, '-') || ':' || \
        novel.status::text || ':' || \
        (SELECT COUNT(*) FROM chapters WHERE novel_id = novel.id) || ':' || \
        novel.total_chapters || ':' || \
        (SELECT COUNT(*) FROM characters WHERE novel_id = novel.id) || ':' || \
        (SELECT COUNT(*) FROM canon_story_models WHERE novel_id = novel.id) || ':' || \
        COALESCE((SELECT md5(string_agg(id::text || ':' || name, ',' ORDER BY id)) \
                  FROM characters WHERE novel_id = novel.id), '-') \
      FROM novel_import_jobs AS job JOIN novels AS novel ON novel.id = job.novel_id \
      WHERE novel.id = '$novel_id'")
  python3 - "$state" "$phase" "$character_md5_before" "$calls_before" "$calls_after" "$retry_status" <<'PY'
import json
import sys

state, phase, character_md5_before = sys.argv[1], sys.argv[2], sys.argv[3]
calls_before, calls_after, retry_status = sys.argv[4], sys.argv[5], sys.argv[6]
attempt, job_stage, job_status, failure_code, novel_status, chapter_count, \
    total_chapters, character_count, canon_count, character_md5 = state.split(":")
print(json.dumps({
    "phase": phase,
    "attempt": int(attempt),
    "job_stage": job_stage,
    "job_status": job_status,
    "failure_code": None if failure_code == "-" else failure_code,
    "novel_status": novel_status,
    "chapter_count": int(chapter_count),
    "total_chapters": int(total_chapters),
    "character_count": int(character_count),
    "canon_count": int(canon_count),
    "character_md5": character_md5,
    "character_md5_before": character_md5_before,
    "calls_before": int(calls_before),
    "calls_after": int(calls_after),
    "retry_status": int(retry_status),
}))
PY
}

verify_phase() {
  python3 - "$1" <<'PY'
import json
import sys

record = json.loads(sys.argv[1])
problems = []


def require(name, condition):
    if not condition:
        problems.append(name)


require("job completed", record["job_status"] == "completed")
require("no failure code", record["failure_code"] is None)
require("novel ready", record["novel_status"] == "ready")
require("attempts inside import-provider-budget-v1",
        1 <= record["attempt"] <= 3)
require("chapters exactly committed total",
        record["chapter_count"] == record["total_chapters"] == 2)
require("exactly one character", record["character_count"] == 1)
require("exactly one canon model", record["canon_count"] == 1)
if record["phase"] == "enriched":
    require("character snapshot stable across resume",
            record["character_md5"] == record["character_md5_before"])
if record["phase"] == "completed":
    require("no provider calls for completed work",
            record["calls_after"] == record["calls_before"] == 0)
    require("retry on ready rejected", record["retry_status"] == 409)

if problems:
    print("ingestion recovery failed: " + ", ".join(problems))
    sys.exit(1)
print("ingestion recovery phase verified: " + record["phase"])
PY
}

sleep 1.1
login=$("${curl_cmd[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"email\":\"$email\",\"password\":\"$password\"}" \
  "$api/auth/login")
token=$(json_get "value['access_token']" <<<"$login")
auth=(-H "Authorization: Bearer $token")
sleep 1.1

# `docker kill` counts as an unexpected exit, so the `unless-stopped` restart
# policy would race the persisted-state assertions; suspend it for the window.
hard_kill() {
  docker update --restart=no novel-novel-service >/dev/null
  docker kill novel-novel-service >/dev/null
}

hard_start() {
  docker start novel-novel-service >/dev/null
  docker update --restart=unless-stopped novel-novel-service >/dev/null
}

# ---- Phase A: hard kill at the `chapters` boundary -------------------------
stub_reset '{"delays_ms":{"characters":3000},"failures_remaining":{}}'
upload=$("${curl_cmd[@]}" "${auth[@]}" \
  -F 'title=恢复甲' \
  -F "file=@$source_file;filename=storm.txt;type=text/plain" \
  "$api/novels/upload")
novel_a=$(json_get "value['novel_id']" <<<"$upload")
[ "$(db "SELECT COUNT(*) FROM chapters WHERE novel_id = '$novel_a'")" = 2 ]
wait_job_stage "$novel_a" chapters "$(($(date +%s) + 60))"
hard_kill
# The persisted boundary survives the kill: chapters committed, job in progress.
[ "$(db "SELECT stage || ':' || status FROM novel_import_jobs WHERE novel_id = '$novel_a'")" = "chapters:in_progress" ]
[ "$(db "SELECT COUNT(*) FROM chapters WHERE novel_id = '$novel_a'")" = 2 ]

hard_start
wait_gateway_healthy
wait_ready "$novel_a" "$(($(date +%s) + 300))"
verify_phase "$(phase_record "$novel_a" chapters - 0 0 409)"

# ---- Phase B: hard kill at the `enriched` boundary -------------------------
stub_reset '{"delays_ms":{"canon":3000},"failures_remaining":{}}'
sleep 1.1
upload=$("${curl_cmd[@]}" "${auth[@]}" \
  -F 'title=恢复乙' \
  -F "file=@$source_file;filename=storm.txt;type=text/plain" \
  "$api/novels/upload")
novel_b=$(json_get "value['novel_id']" <<<"$upload")
wait_job_stage "$novel_b" enriched "$(($(date +%s) + 120))"
character_md5_before=$(db "SELECT md5(string_agg(id::text || ':' || name, ',' ORDER BY id)) FROM characters WHERE novel_id = '$novel_b'")
hard_kill
[ "$(db "SELECT stage || ':' || status FROM novel_import_jobs WHERE novel_id = '$novel_b'")" = "enriched:in_progress" ]
[ "$(db "SELECT COUNT(*) FROM characters WHERE novel_id = '$novel_b'")" = 1 ]

hard_start
wait_gateway_healthy
wait_ready "$novel_b" "$(($(date +%s) + 300))"
verify_phase "$(phase_record "$novel_b" enriched "$character_md5_before" 0 0 409)"

# ---- Phase C: completed work replays without a provider call ---------------
stub_reset '{"delays_ms":{},"failures_remaining":{}}'
calls_before=$(stub_calls)
[ "$calls_before" = 0 ]
docker restart novel-novel-service >/dev/null
wait_gateway_healthy
sleep 5
calls_after=$(stub_calls)
sleep 1.1
retry_status=$(curl --connect-timeout 5 --max-time 120 --silent --show-error \
  --output /dev/null --write-out '%{http_code}' "${auth[@]}" \
  -X POST "$api/novels/$novel_a/retry")
verify_phase "$(phase_record "$novel_a" completed - "$calls_before" "$calls_after" "$retry_status")"

# Restore the golden reader loop's failure injections: one canon response
# exercises bounded validation recovery, while three narrative responses
# exhaust validation retries and prove the atomic 502 path.
stub_reset '{"delays_ms":{},"failures_remaining":{"canon":1,"narrative_transition":3,"world_turn":0}}'

printf 'ingestion recovery drills passed: kill at chapters=%s enriched=%s, completed replay calls=%s->%s retry=%s\n' \
  "$novel_a" "$novel_b" "$calls_before" "$calls_after" "$retry_status"
