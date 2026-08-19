#!/usr/bin/env bash
# Bad-release drill (H2 incident-response scope): the gateway goes down
# mid-flight, the nginx edge keeps serving the common SPEC 10.9 error
# envelope instead of a default HTML page, and the deployment recovers when
# the healthy release returns.
#
# The edge error pages mirror the status->code mapping the gateway pins in
# gateway/src/proxy.rs (NORMALIZED_ERROR_RESPONSES). Only 502 is exercised
# here (docker stop = connect refused); 503/504 are mapped by the same
# config but are not drillable without upstream health marks or a
# proxy_read_timeout hang, and mid-stream SSE failures are out of scope
# (response headers are already sent). Re-runnable: stop/start is
# idempotent and no data is mutated.
set -euo pipefail
cd "$(dirname "$0")/../.."

nginx_url=${E2E_NGINX_URL:-http://127.0.0.1}
compose_files=${COMPOSE_FILES:-docker-compose.yml}

work=$(mktemp -d)
gateway_was_stopped=0
cleanup() {
  rm -rf "$work"
  if [ "$gateway_was_stopped" -eq 1 ] &&
    [ "$(docker inspect --format '{{.State.Running}}' novel-gateway 2>/dev/null)" = false ]; then
    docker start novel-gateway >/dev/null 2>&1 || true
    printf 'drill: note: restored the stopped gateway\n' >&2
  fi
}
trap cleanup EXIT

check() {
  if [ "$2" != "$3" ]; then
    printf 'drill: FAIL %s: expected [%s], got [%s]\n' "$1" "$2" "$3" >&2
    exit 1
  fi
  printf 'drill: ok   %s = %s\n' "$1" "$3"
}

wait_healthy() {
  for _ in $(seq 1 90); do
    if [ "$(docker inspect --format '{{.State.Health.Status}}' novel-gateway 2>/dev/null)" = healthy ]; then
      return 0
    fi
    sleep 2
  done
  docker compose -f ${compose_files//:/ -f } ps --all >&2
  printf 'drill: the gateway never became healthy\n' >&2
  exit 1
}

headers() {
  curl --connect-timeout 5 --max-time 30 --silent --head "$nginx_url/api/setup/status"
}

# Precondition: the gateway is healthy and reachable through the edge.
wait_healthy
check 'gateway reachable through the edge before the outage' \
  "$(curl --connect-timeout 5 --max-time 30 --silent --output /dev/null --write-out '%{http_code}' "$nginx_url/live")" 200

printf 'drill: stopping the gateway (simulated bad release / dependency failure)\n'
docker stop novel-gateway >/dev/null

# Mark the outage for the EXIT trap so a failed run cannot leave the
# gateway down.
gateway_was_stopped=1

# The first connect after the stop can pay the bridge ARP-resolution cost,
# so poll until the edge reports the outage instead of one fixed warm-up.
for _ in $(seq 1 30); do
  status=$(curl --connect-timeout 5 --max-time 10 --silent --output "$work/body" --write-out '%{http_code}' "$nginx_url/api/setup/status" || true)
  [ "$status" = 502 ] && break
  sleep 2
done

status=$(curl --connect-timeout 5 --max-time 30 --silent --output "$work/body" --write-out '%{http_code}' "$nginx_url/api/setup/status")
check 'edge status while the gateway is down' "$status" 502
check 'edge error body is the stable envelope' "$(cat "$work/body")" \
  '{"error":{"code":"bad_gateway","message":"Upstream service returned an invalid response"}}'
headers >"$work/headers"
check 'edge error content type' "$(grep -i '^content-type:' "$work/headers" | tr -d '\r' | tr 'A-Z' 'a-z')" 'content-type: application/json'
check 'edge error keeps the nosniff header' "$(grep -i '^x-content-type-options:' "$work/headers" | tr -d '\r' | tr 'A-Z' 'a-z')" 'x-content-type-options: nosniff'

printf 'drill: starting the gateway again (rollback to the healthy release)\n'
docker start novel-gateway >/dev/null
wait_healthy
check 'gateway reachable through the edge after recovery' \
  "$(curl --connect-timeout 5 --max-time 30 --silent --output /dev/null --write-out '%{http_code}' "$nginx_url/live")" 200

printf 'drill: bad-release drill passed\n'
