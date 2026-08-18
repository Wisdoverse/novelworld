# Operations Runbook

Version: **`operations-v1`**. This is the minimum production-readiness
**partial slice** for the private self-hosted profile: health checks, the
playbook index, and ownership. The remaining H2 subset — dashboards,
journey SLIs, the initial SLO/error budget, and actionable alerting with
routing/paging — is explicitly **not yet landed** (ROADMAP H2 scope and H5
own that work).

## Service map and health checks

| Surface | Probe |
|---|---|
| Gateway | `/live`, `/ready`, `/health` on the gateway port; `/metrics` (Prometheus text, internal only) |
| Edge | nginx `/nginx-health` |
| Services | per-service `/health` + `/ready` (see each `interface/http`), docker healthchecks |
| Data | `docker inspect … novel-postgres`, `novel-redis` healthchecks |

`infra/ops/health-checks.sh` probes the gateway and edge endpoints
(through the published nginx edge) and every container's health, fails
non-zero naming each failing check, and prints a bounded tail of recent
ERROR log lines (structured stdout via `docker compose logs`). The
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

## Deferred to H5

Dashboards, journey SLIs, the initial SLO/error budget, actionable alerts
with routing/dedup/paging, and postmortem tooling.
