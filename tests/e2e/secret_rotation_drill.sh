#!/usr/bin/env bash
# Secret-rotation drill for the private preview (H2 exit evidence).
#
# Verifies the rotation contract recorded in SECURITY.md: after rotating
# JWT_SECRET and INTERNAL_SERVICE_TOKEN and wiping persisted refresh tokens,
# old access and refresh tokens are rejected, a new login works, and an
# account export (whose service fragments are internal-token-authenticated)
# succeeds. RUNTIME_CONFIG_KEY is deliberately NOT rotated: it is not
# rotatable in place (SECURITY.md).
#
# Usage:
#   tests/e2e/secret_rotation_drill.sh                 # against the running e2e topology
#   tests/e2e/secret_rotation_drill.sh --self-test     # verifier mutation checks, no topology
#
# One-shot: the drill leaves the stack running with rotated secrets; tear the
# stack down before running it again.
set -euo pipefail

if [ "${1:-}" = "--self-test" ]; then
  exec python3 "$(dirname "$0")/secret_rotation_verify.py" --self-test
fi
api=${E2E_API_URL:-http://127.0.0.1/api}
compose_files=${COMPOSE_FILES:-docker-compose.yml:docker-compose.e2e.yml}
email=admin@test.invalid
password='RuntimeSmokeOnly123!'
curl_cmd=(curl --connect-timeout 5 --max-time 120 --fail --silent --show-error)

json_get() {
  python3 -c "import json,sys; value=json.load(sys.stdin); print($1)"
}

http_status() {
  # Pace gateway calls: the CI drills run with RATE_LIMIT_RPS=1 (the same
  # pause the other e2e drills use).
  sleep 1.1
  curl --connect-timeout 5 --max-time 120 --silent --show-error     --output /dev/null --write-out '%{http_code}' "$@"
}

wait_gateway_healthy() {
  for _ in $(seq 1 60); do
    [ "$(docker inspect --format '{{.State.Health.Status}}' novel-gateway 2>/dev/null)" = healthy ] && return 0
    sleep 2
  done
  printf 'gateway did not become healthy\n' >&2
  return 1
}

# The stack must exist with the OLD secrets; the drill rotates them.
[ -n "${JWT_SECRET:-}" ] && [ -n "${INTERNAL_SERVICE_TOKEN:-}" ] || {
  printf 'JWT_SECRET and INTERNAL_SERVICE_TOKEN must be exported (the old values)\n' >&2
  exit 1
}

sleep 1.1
login=$("${curl_cmd[@]}" -H 'Content-Type: application/json'   --data "{\"email\":\"$email\",\"password\":\"$password\"}" "$api/auth/login")
access=$(json_get "value['access_token']" <<<"$login")
refresh=$(json_get "value['refresh_token']" <<<"$login")
sleep 1.1

# Precondition: the exported secrets must be the stack's CURRENT secrets.
pre_status=$(http_status -H "Authorization: Bearer $access" "$api/novels")
if [ "$pre_status" != 200 ]; then
  printf 'the stack is already running rotated secrets; tear it down and redeploy before re-running this drill\n' >&2
  exit 1
fi

# Rotate both rotatable secrets and recreate every service together.
old_internal_token=$INTERNAL_SERVICE_TOKEN
export JWT_SECRET=runtime-rotated-secret-at-least-32-characters
export INTERNAL_SERVICE_TOKEN=runtime-rotated-internal-token-at-least-32-chars
docker compose -f ${compose_files//:/ -f } up -d --force-recreate >/dev/null 2>&1
wait_gateway_healthy
sleep 2

# The documented procedure wipes persisted refresh tokens (opaque rows that
# would otherwise survive a JWT_SECRET rotation).
docker exec novel-postgres psql   -U "${POSTGRES_USER:-novel}" -d "${POSTGRES_DB:-novel_world}" -At   -c "DELETE FROM refresh_tokens" >/dev/null

# Negative proof that the internal token actually rotated: the OLD token
# must be rejected on the internal endpoint (reachable only on the compose
# network, hence the in-network curl container).
network=$(docker inspect novel-gateway --format '{{range $k, $v := .NetworkSettings.Networks}}{{$k}}{{end}}')
old_internal_status=$(docker run --rm --network "$network" curlimages/curl:latest \
  --silent --output /dev/null --write-out "%{http_code}" \
  -H "X-Internal-Service-Token: $old_internal_token" \
  http://user-service:8001/internal/runtime/llm)
old_access_status=$(http_status -H "Authorization: Bearer $access" "$api/novels")
old_refresh_status=$(http_status -H 'Content-Type: application/json'   --data "{\"refresh_token\":\"$refresh\"}" "$api/auth/refresh")
sleep 1.1
new_login=$("${curl_cmd[@]}" -H 'Content-Type: application/json'   --data "{\"email\":\"$email\",\"password\":\"$password\"}" "$api/auth/login")
new_token=$(json_get "value['access_token']" <<<"$new_login")
new_login_status=$(http_status -H "Authorization: Bearer $new_token" "$api/novels")
sleep 1.1
export_status=$(http_status -H "Authorization: Bearer $new_token" "$api/account/export")

python3 "$(dirname "$0")/secret_rotation_verify.py" "$old_access_status" "$old_refresh_status" "$old_internal_status" "$new_login_status" "$export_status"

printf 'secret rotation drill passed: old access=%s old refresh=%s new login=%s export=%s\n'   "$old_access_status" "$old_refresh_status" "$new_login_status" "$export_status"