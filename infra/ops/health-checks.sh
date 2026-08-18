#!/usr/bin/env bash
# Deployment health checks (H2 production-readiness, partial slice). This is
# a fail-closed health probe, NOT actionable alerting: there is no routing,
# dedup, or paging (that slice stays open - see docs/OPERATIONS.md). Run it
# from cron or before/after operations; a non-zero exit names every failing
# check, and a bounded tail of recent ERROR log lines is printed for the
# human to read.
#
# All HTTP probes go through the published nginx edge (the gateway port is
# not published on the production profile), plus per-container health.
#
# Usage: infra/ops/health-checks.sh [NGINX_URL]
set -euo pipefail
cd "$(dirname "$0")/../.."

nginx_url=${1:-http://127.0.0.1:80}
failures=0

fail() {
  printf 'health: FAIL %s\n' "$1" >&2
  failures=$((failures + 1))
}

ok() { printf 'health: ok   %s\n' "$1"; }

probe() {
  local label=$1 url=$2 expected=$3 code
  code=$(curl --connect-timeout 5 --max-time 15 --silent --output /dev/null \
    --write-out '%{http_code}' "$url" || true)
  if [ "$code" = "$expected" ]; then
    ok "$label"
  else
    fail "$label (wanted $expected, got $code)"
  fi
}

container_healthy() {
  local name=$1 status
  status=$(docker inspect --format '{{.State.Health.Status}}' "$name" 2>/dev/null || true)
  if [ "$status" = healthy ]; then
    ok "$name health"
    return 0
  fi
  if [ -z "$status" ]; then
    # No healthcheck defined: fall back to running state.
    status=$(docker inspect --format '{{.State.Status}}' "$name" 2>/dev/null || true)
    [ "$status" = running ] && { ok "$name running (no healthcheck)"; return 0; }
  fi
  fail "$name (status: ${status:-missing})"
}

# HTTP probes through the published edge.
probe 'gateway /live' "$nginx_url/live" 200
probe 'gateway /ready' "$nginx_url/ready" 200
probe 'nginx /nginx-health' "$nginx_url/nginx-health" 200

# Container health (services with healthchecks; running-state fallback).
for name in novel-gateway novel-user-service novel-novel-service \
  novel-agent-service novel-narrative-service novel-frontend novel-nginx \
  novel-postgres novel-redis; do
  container_healthy "$name"
done

# Informational: recent ERROR log lines per service, bounded.
printf 'health: recent ERROR lines (last 10 minutes):\n'
docker compose logs --since 10m --no-color 2>/dev/null \
  | grep -iE '"level"[[:space:]]*:[[:space:]]*"error"' | tail -20 || true

if [ "$failures" -gt 0 ]; then
  printf 'health: %s check(s) failed\n' "$failures" >&2
  exit 1
fi
printf 'health: all checks passed\n'
