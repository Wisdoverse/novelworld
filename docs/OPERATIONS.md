# Operations Runbook

Version: **`operations-v1`**. This is the minimum production-readiness
**partial slice** for the private self-hosted profile: health checks, the
playbook index, ownership, Prometheus collectors, a Grafana dashboard, and
alert rules. The remaining H2 subset — journey SLIs, the initial
SLO/error budget, and alert notification routing/paging — is explicitly
**not yet landed** (ROADMAP H2 scope and H5 own that work).

## Service map and health checks

| Surface | Probe |
|---|---|
| Gateway | `/live`, `/ready`, `/health` on the gateway port; `/metrics` (Prometheus text, internal only) |
| Edge | nginx `/nginx-health` |
| Services | per-service `/health` + `/ready` (see each `interface/http`), docker healthchecks |
| Data | PostgreSQL health always; Redis health only when persisted `CACHE_MODE=redis` |

`infra/ops/health-checks.sh` reads the persisted cache mode, derives the same
Compose profile/URL selection as the launch and release tools, probes the gateway
and edge endpoints (through the published nginx edge) and every required container's health, fails
non-zero naming each failing check, and prints a bounded tail of recent
ERROR log lines (structured stdout via `docker compose logs`). The
PostgreSQL mode fails if Redis is unexpectedly running; Redis mode fails on a
missing/placeholder credential or an unhealthy Redis container. No credential is logged. The
per-service `/health` and `/metrics` endpoints are the hand-debugging
surface, not scripted probes. Cron example:

```bash
*/5 * * * * /srv/novelworld/infra/ops/health-checks.sh http://127.0.0.1:80 >> /var/log/novelworld-health.log 2>&1 || true
```

This is a fail-closed health probe, not actionable alerting: no routing,
dedup, or paging exists yet.

## Playbook index

- **Bad release / dependency failure** — [`bad_release_drill.sh`](../tests/e2e/bad_release_drill.sh) is the
  practice; the recovery is rollback via `infra/docker/release.sh`
  ([`SECURITY.md`](../SECURITY.md) Release Rollback).
- **Secret rotation** — [`SECURITY.md`](../SECURITY.md) Secret Rotation + its e2e drill.
- **Provider outage** — fail-closed import, retry after the provider
  returns; [`provider_outage_drill.sh`](../tests/e2e/provider_outage_drill.sh).
- **Backup / restore** — [`BACKUP_RESTORE.md`](./BACKUP_RESTORE.md) drills A/B/C; RTO ≤ 30 minutes.
- **Overload** — the landed admission controls: nginx per-client rate limit
  plus gateway `RATE_LIMIT_RPS` ([`SECURITY.md`](../SECURITY.md)); capacity contract and
  503 assertions in [`SLOS.md`](./SLOS.md).
- **Log contract** — [`log_contract.py`](../tests/e2e/log_contract.py) checks the §14.1 shape and
  trace propagation.
- **Capacity profile** — [`SLOS.md`](./SLOS.md) Run locally section; the recorded CI run is
  the qualification gate.

## Ownership and escalation

The private self-hosted profile has a single operator (the deployment
owner, [`DEPLOYMENT_PROFILE.md`](./DEPLOYMENT_PROFILE.md)): the operator owns
detection and response. There is no on-call rotation and no paging. The
vulnerability-reporting channel in [`SECURITY.md`](../SECURITY.md) is for
security reports, not operational escalation; incidents are the operator's
to triage against this runbook.

## Monitoring

Optional overlay: `docker compose -f docker-compose.yml -f docker-compose.monitoring.yml up -d`
with `GRAFANA_ADMIN_PASSWORD` set. Prometheus scrapes the gateway and the
four services, evaluates [`alerts.yml`](../infra/monitoring/alerts.yml), and
Grafana serves the provisioned NovelWorld Overview dashboard on
`GRAFANA_HTTP_PORT` (default `127.0.0.1:13000`). The alerts:

- **InstanceDown** (critical) — a service stopped being scraped;
  restart it, then run the health checks.
- **GatewayRateLimitRejections** (warning) — the gateway's own 429s above
  5%; check `RATE_LIMIT_RPS` and SLOS.md. **Known gap:** the nginx edge's
  per-client 429s never reach the gateway, so they are not visible here.
- **HighErrorRatio** (warning) — gateway 5xx above 2%; go to the
  bad-release or provider-outage playbook.

`infra/monitoring/drill.sh` verifies the profile: the rules are valid and
provably fire (promtool unit tests), every target scrapes, the
instance-down alert fires and resolves against a live service stop/start,
and Grafana serves the dashboard.

## Deferred to H5

Journey SLIs, the initial SLO/error budget, alert notification
routing/dedup/paging (the rules fire; nothing pages yet), and postmortem
tooling.
