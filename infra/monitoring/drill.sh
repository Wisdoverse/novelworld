#!/usr/bin/env bash
# Monitoring-profile drill (H2 production-readiness): proves the alert rules
# are valid and fire, every scrape target is up, the instance-down alert
# really fires and resolves, and Grafana serves the provisioned dashboard.
# Requires the monitoring overlay on top of the running stack and
# GRAFANA_ADMIN_PASSWORD exported.
set -euo pipefail
cd "$(dirname "$0")/../.."

grafana_url=${GRAFANA_URL:-http://127.0.0.1:13000}
password=${GRAFANA_ADMIN_PASSWORD:?GRAFANA_ADMIN_PASSWORD must be exported}

query_prometheus() {
  local encoded=$1
  docker exec novel-prometheus wget -qO- \
    "http://127.0.0.1:9090/api/v1/query?query=$encoded" || true
}

alerts_query='ALERTS%7Balertstate%3D%22firing%22%2Calertname%3D%22InstanceDown%22%7D'

# 1. The alert rules must be valid PromQL and provably fire.
docker run --rm --entrypoint promtool -v "$PWD/infra/monitoring:/rules:ro" \
  prom/prometheus:v3.14.0@sha256:5ce7540c3c00ef4ab0c9d2c995c6a5b9c421f44b4a115d97a2c7af3b1c21cbb0 \
  test rules /rules/alert-tests.yml 2>&1 | tail -2
printf 'drill: ok   all alert rules are valid and fire on synthetic input\n'

# 2. Every scrape target up (retry while Prometheus settles).
for _ in $(seq 1 20); do
  targets=$(docker exec novel-prometheus wget -qO- \
    "http://127.0.0.1:9090/api/v1/targets?state=active" || true)
  if python3 -c "import json,sys; d=json.load(sys.stdin); t=d['data']['activeTargets']; assert all(x['health']=='up' for x in t), t; print('targets up:', len(t))" <<<"$targets" 2>/dev/null; then
    break
  fi
  sleep 2
done
python3 -c "import json,sys; d=json.load(sys.stdin); t=d['data']['activeTargets']; assert all(x['health']=='up' for x in t), t" <<<"$targets"

# 3. The instance-down alert fires when a service stops and resolves after.
printf 'drill: stopping the narrative service\n'
docker stop novel-narrative-service >/dev/null
for _ in $(seq 1 30); do
  sleep 2
  alerts=$(query_prometheus "$alerts_query")
  if python3 -c "import json,sys; d=json.load(sys.stdin); assert d['data']['result']" <<<"$alerts" 2>/dev/null; then
    break
  fi
done
python3 -c "import json,sys; d=json.load(sys.stdin); results=d['data']['result']; assert results, 'InstanceDown never fired'; print('firing:', sorted({r['metric'].get('instance','') for r in results}))" <<<"$alerts"
docker start novel-narrative-service >/dev/null
for _ in $(seq 1 60); do
  sleep 2
  alerts=$(query_prometheus "$alerts_query")
  if python3 -c "import json,sys; d=json.load(sys.stdin); assert not d['data']['result']" <<<"$alerts" 2>/dev/null; then
    printf 'drill: ok   InstanceDown resolved after the service returned\n'
    break
  fi
done
python3 -c "import json,sys; d=json.load(sys.stdin); assert not d['data']['result'], 'InstanceDown did not resolve'" <<<"$alerts"

# 4. Grafana serves the provisioned dashboard.
dashboard=$(curl --silent --fail --user "admin:$password" \
  "$grafana_url/api/dashboards/uid/novelworld-overview")
python3 -c "import json,sys; d=json.load(sys.stdin); assert d['dashboard']['title']=='NovelWorld Overview'; print('panels:', len(d['dashboard']['panels']))" <<<"$dashboard"

printf 'drill: monitoring-profile drill passed\n'
