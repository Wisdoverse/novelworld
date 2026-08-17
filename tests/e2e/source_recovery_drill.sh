#!/usr/bin/env bash
# Live kill/restart drill for the S3-retained 'source' import boundary
# (H1 exit evidence, closing the last open persisted-ingestion boundary).
#
# With source retention enabled, acceptance commits the retained object plus a
# 'source'-stage job with no chapters, and the claimed worker rebuilds
# deterministic chapters from S3 before any provider call. The drill parks the
# worker deterministically at the fenced chapter commit by holding an
# EXCLUSIVE lock on the chapters table (the replay INSERT blocks on it), then
# hard-kills novel-service there, restarts it, waits out the real lease fence,
# and proves the import resumes from the retained object to exactly one
# authoritative result without re-upload: chapter content must hash to the
# uploaded source, and completed work must make no new provider call.
#
# Usage:
#   tests/e2e/source_recovery_drill.sh                 # against the S3 e2e topology
#   tests/e2e/source_recovery_drill.sh --self-test     # verifier mutation checks, no topology
set -euo pipefail

if [ "${1:-}" = "--self-test" ]; then
  python3 - <<'PY'
import sys


def verify(record):
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
    require("chapters rebuilt from the retained object",
            record["chapter_md5s"] == record["expected_md5s"])
    require("exactly one character", record["character_count"] == 1)
    require("exactly one canon model", record["canon_count"] == 1)
    require("kill landed at the source boundary",
            record["kill_stage"] == "source" and record["chapters_at_kill"] == 0)
    require("no provider calls for completed work",
            record["calls_after"] == record["calls_before"] == 0)
    require("retry on ready rejected", record["retry_status"] == 409)
    return problems


base = {
    "phase": "source",
    "job_status": "completed",
    "failure_code": None,
    "novel_status": "ready",
    "chapter_count": 2,
    "total_chapters": 2,
    "expected_md5s": "aa,bb",
    "chapter_md5s": "aa,bb",
    "character_count": 1,
    "canon_count": 1,
    "kill_stage": "source",
    "chapters_at_kill": 0,
    "calls_before": 0,
    "calls_after": 0,
    "retry_status": 409,
    "attempt": 2,
}

tampered = [
    ("duplicated chapters must fail", dict(base, chapter_count=3)),
    ("false ready must fail", dict(base, novel_status="parsing")),
    ("chapters not from the retained object must fail",
     dict(base, chapter_md5s="cc,dd")),
    ("kill outside the source boundary must fail",
     dict(base, kill_stage="chapters")),
    ("chapters committed at kill must fail", dict(base, chapters_at_kill=2)),
    ("duplicate canon must fail", dict(base, canon_count=2)),
    ("provider call for completed work must fail",
     dict(base, calls_after=1)),
    ("ready import retry must fail", dict(base, retry_status=200)),
    ("attempt beyond the budget ceiling must fail", dict(base, attempt=4)),
]
for label, record in tampered:
    if not verify(record):
        print(f"self-test failed: {label} passed the weakened verifier")
        sys.exit(1)
print("source recovery verifier self-test passed")
PY
  exit 0
fi

api=${E2E_API_URL:-http://127.0.0.1/api}
email=admin@test.invalid
password='RuntimeSmokeOnly123!'
source_file="source_recovery_source_$$.txt"
lock_pid=''
trap 'release_lock; rm -f "$source_file"' EXIT
curl_cmd=(curl --connect-timeout 5 --max-time 120 --fail --silent --show-error)
stub=${E2E_STUB_URL:-http://127.0.0.1:18080}

chapter_one='第一章 风暴前夜'
chapter_one_body='林岚握紧手中的旧地图，望向被风暴笼罩的北塔，决定在天黑前寻找失踪的守门人。边城的钟声连续响了三次，街道上的人们纷纷关紧门窗。林岚仍站在石桥中央，逐一核对地图上的暗号，并请你留意河岸新出现的足迹。远处的塔灯忽明忽暗，仿佛有人正用最后的力气发出求救信号。你们约定不替彼此作决定，却要共同承担进入风暴的后果。'
chapter_two='第二章 北塔回声'
chapter_two_body='北塔的石门布满潮湿苔痕，林岚在门边发现守门人留下的铜铃。铃身刻着通往地下回廊的路线，也写明只有彼此信任的同行者才能安全通过。风暴压低了天空，城墙上的火把依次熄灭。你与林岚交换各自找到的线索，确认失踪并非意外。塔内传来沉重脚步，旧地图上从未标注的房间正在缓缓开启，而边城的命运也随这一刻发生变化。'

printf '%s\n' "$chapter_one" "$chapter_one_body" '' "$chapter_two" "$chapter_two_body" \
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

# The drill parks the source-stage replay at its fenced chapter commit by
# holding an EXCLUSIVE lock on chapters: the replay INSERT blocks, the job
# stays at source:in_progress with zero chapters, and the hard kill lands
# deterministically inside the source boundary without any production hook.
lock_chapters() {
  docker exec novel-postgres psql \
    -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
    -c "BEGIN; LOCK TABLE chapters IN EXCLUSIVE MODE; SELECT pg_sleep(600);" \
    >/dev/null 2>&1 &
  lock_pid=$!
}

release_lock() {
  if [ -n "$lock_pid" ]; then
    kill "$lock_pid" 2>/dev/null || true
    lock_pid=''
  fi
  docker exec novel-postgres psql \
    -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At \
    -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE query LIKE '%pg_sleep(600)%' AND pid <> pg_backend_pid();" \
    >/dev/null 2>&1 || true
}

wait_replay_blocked() {
  for _ in $(seq 1 300); do
    [ "$(db "SELECT COUNT(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query ILIKE '%chapters%'")" -ge 1 ] && return 0
    sleep 0.1
  done
  printf 'replay did not reach the parked chapter commit\n' >&2
  return 1
}

# 'docker kill' counts as an unexpected exit, so the 'unless-stopped' restart
# policy would race the persisted-state assertions; suspend it for the window.
hard_kill() {
  docker update --restart=no novel-novel-service >/dev/null
  docker kill novel-novel-service >/dev/null
}

hard_start() {
  docker start novel-novel-service >/dev/null
  docker update --restart=unless-stopped novel-novel-service >/dev/null
}

phase_record() {
  local novel_id=$1 expected_md5s=$2 kill_stage=$3 chapters_at_kill=$4 calls_before=$5 calls_after=$6 retry_status=$7 state
  state=$(db "SELECT \
        job.attempt || ':' || \
        job.status || ':' || COALESCE(job.failure_code, '-') || ':' || \
        novel.status::text || ':' || \
        (SELECT COUNT(*) FROM chapters WHERE novel_id = novel.id) || ':' || \
        novel.total_chapters || ':' || \
        COALESCE((SELECT string_agg(md5(content), ',' ORDER BY chapter_number) FROM chapters WHERE novel_id = novel.id), '-') || ':' || \
        (SELECT COUNT(*) FROM characters WHERE novel_id = novel.id) || ':' || \
        (SELECT COUNT(*) FROM canon_story_models WHERE novel_id = novel.id) \
      FROM novel_import_jobs AS job JOIN novels AS novel ON novel.id = job.novel_id \
      WHERE novel.id = '$novel_id'")
  python3 - "$state" "$expected_md5s" "$kill_stage" "$chapters_at_kill" "$calls_before" "$calls_after" "$retry_status" <<'PY'
import json
import sys

state, expected_md5s, kill_stage, chapters_at_kill = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
calls_before, calls_after, retry_status = sys.argv[5], sys.argv[6], sys.argv[7]
attempt, job_status, failure_code, novel_status, chapter_count, \
    total_chapters, chapter_md5s, character_count, canon_count = state.split(":")
print(json.dumps({
    "phase": "source",
    "attempt": int(attempt),
    "job_status": job_status,
    "failure_code": None if failure_code == "-" else failure_code,
    "novel_status": novel_status,
    "chapter_count": int(chapter_count),
    "total_chapters": int(total_chapters),
    "expected_md5s": expected_md5s,
    "chapter_md5s": chapter_md5s,
    "character_count": int(character_count),
    "canon_count": int(canon_count),
    "kill_stage": kill_stage,
    "chapters_at_kill": int(chapters_at_kill),
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
require("chapters rebuilt from the retained object",
        record["chapter_md5s"] == record["expected_md5s"])
require("exactly one character", record["character_count"] == 1)
require("exactly one canon model", record["canon_count"] == 1)
require("kill landed at the source boundary",
        record["kill_stage"] == "source" and record["chapters_at_kill"] == 0)
require("no provider calls for completed work",
        record["calls_after"] == record["calls_before"] == 0)
require("retry on ready rejected", record["retry_status"] == 409)

if problems:
    print("source recovery failed: " + ", ".join(problems))
    sys.exit(1)
print("source recovery phase verified: " + record["phase"])
PY
}

# A fresh S3 stack has no first administrator; the drill is self-contained.
setup_status=$(curl --silent "$api/setup/status" | json_get "value['configured']")
if [ "$setup_status" != True ]; then
  sleep 1.1
  curl --fail --silent -H 'Content-Type: application/json' \
    --data "{\"email\":\"$email\",\"password\":\"$password\",\"name\":\"Runtime Admin\"}" \
    "$api/setup/init" >/dev/null
fi
sleep 1.1

login=$("${curl_cmd[@]}" \
  -H 'Content-Type: application/json' \
  --data "{\"email\":\"$email\",\"password\":\"$password\"}" \
  "$api/auth/login")
token=$(json_get "value['access_token']" <<<"$login")
auth=(-H "Authorization: Bearer $token")
sleep 1.1

stub_reset '{"delays_ms":{},"failures_remaining":{}}'
lock_chapters
sleep 0.5

upload=$("${curl_cmd[@]}" "${auth[@]}" \
  -F 'title=恢复源' \
  -F "file=@$source_file;filename=storm.txt;type=text/plain" \
  "$api/novels/upload")
novel_id=$(json_get "value['novel_id']" <<<"$upload")

# Acceptance committed the source-stage job atomically with zero chapters.
[ "$(db "SELECT stage || ':' || status FROM novel_import_jobs WHERE novel_id = '$novel_id'")" = "source:in_progress" ]
chapters_at_kill=$(db "SELECT COUNT(*) FROM chapters WHERE novel_id = '$novel_id'")
[ "$chapters_at_kill" = 0 ]

# The replay now blocks on the parked chapter commit; kill it there.
wait_replay_blocked
hard_kill
[ "$(db "SELECT stage || ':' || status FROM novel_import_jobs WHERE novel_id = '$novel_id'")" = "source:in_progress" ]
[ "$(db "SELECT COUNT(*) FROM chapters WHERE novel_id = '$novel_id'")" = 0 ]

release_lock
hard_start
wait_gateway_healthy
wait_ready "$novel_id" "$(($(date +%s) + 300))"

expected_md5s=$(python3 - "$chapter_one" "$chapter_one_body" "$chapter_two" "$chapter_two_body" <<'PY'
import hashlib
import sys

chapter_one, chapter_one_body, chapter_two, chapter_two_body = sys.argv[1:5]
first = chapter_one + "\n" + chapter_one_body
second = chapter_two + "\n" + chapter_two_body
print(hashlib.md5(first.encode()).hexdigest() + "," + hashlib.md5(second.encode()).hexdigest())
PY
)

# Completed work replays without a provider call on the S3 topology.
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
  -X POST "$api/novels/$novel_id/retry")

verify_phase "$(phase_record "$novel_id" "$expected_md5s" source "$chapters_at_kill" "$calls_before" "$calls_after" "$retry_status")"

printf 'source recovery drill passed: kill at source boundary for %s, chapters rebuilt from S3, completed replay calls=%s->%s retry=%s\n' \
  "$novel_id" "$calls_before" "$calls_after" "$retry_status"
