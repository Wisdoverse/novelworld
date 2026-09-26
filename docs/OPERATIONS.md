# Operations Runbook

Version: **`operations-v1`**. This is the minimum production-readiness
**partial slice** for the private self-hosted profile: health checks, the
playbook index, ownership, Prometheus collectors, a Grafana dashboard, and
alert rules. The remaining H2 subset — journey SLIs, the initial
SLO/error budget, and alert notification routing/paging — is explicitly
**not yet landed** (ROADMAP H2 scope and H5 own that work).

## Service map and health checks

For model connection failures, verify the selected provider, region and API/plan
Key against [LLM provider configuration](./LLM_PROVIDERS.md). Settings switches
require a freshly entered Key. A successful bounded probe proves connectivity,
not live model quality or remaining subscription quota.

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
  trace propagation and request outcome fields.
- **Capacity profile** — [`SLOS.md`](./SLOS.md) Run locally section; the recorded CI run is
  the qualification gate.

## Log levels and incident lookup

All five Rust processes emit JSON to stdout. Set `RUST_LOG` in `.env` to tune
verbosity at the next service recreation; Compose defaults to `info`. For a
focused investigation, use `RUST_LOG=info,novel_service=debug` (replace the
module target for another service). `reqwest` and `tower_http` remain disabled
because their default traces can contain full URLs. Restore `info` after the
investigation. `debug` can be high volume and is not a privacy exception.

The base and optional monitoring Compose services use Docker's `local` logging
driver, including Redis and local embedding profiles. Its default rotation
retains up to five 20 MB files per container (with compression); older logs
are discarded. `docker logs` and
`docker compose logs` remain available. Existing containers pick up this
setting when recreated. See [Docker's local driver reference](https://docs.docker.com/engine/logging/drivers/local/).

The public Nginx edge emits JSON request outcomes with status and upstream
timing, but no raw URI, client IP, Referer, or User-Agent. Its `trace_id`
comes from the Gateway response and can be empty for requests stopped at the
edge. The frontend Nginx does not duplicate access logs. Both Nginx error logs
use `crit`: lower-severity upstream errors automatically include the original
request line and client address. Critical errors can still carry request
context, so restrict Docker log access and treat the retained logs as
sensitive. Use the edge status and upstream timing for routine failures.

- `ERROR`: a failed operation or HTTP 5xx; investigate using `trace_id`,
  `route`, `status`, and nearby fixed error codes.
- `WARN`: degraded or retried work, including HTTP 429. Normal client 4xx
  remains `INFO` and is searchable by numeric `status`.
- `INFO`: lifecycle and normal request completion. `DEBUG`: bounded internal
  decisions needed for a focused investigation.

For a failed novel import, `error_code` is the public-safe category. The
associated provider-rejection `WARN` records `provider_http_status` for common
upstream 4xx responses. HTTP `402` indicates an account-balance problem;
verify the provider account before asking the reader to re-import. A locally
rejected oversized response does not claim an upstream HTTP status. Never log
the provider response body or credentials.

Completion `elapsed_ms` ends when response headers are produced; it excludes
the rest of a streaming response. `route` is an Axum template or `unmatched`,
never the raw path. Caller trace IDs are bounded and sanitized at every HTTP
ingress. Keep credentials, email, names, novel content, prompt/response bodies,
raw URLs, query strings, and arbitrary headers out of application logs at every
level. The Nginx critical-error limitation is described above.

For a recent error on the local host, inspect the service's structured output:

```bash
docker logs novel-novel-service --since 30m 2>&1 \
  | jq -c 'select(.level == "ERROR") | (.spans // [] | map(select(has("service"))) | .[-1] // {}) as $ctx | {timestamp,service: $ctx.service,trace_id: $ctx.trace_id,fields}'
```

Use the same `trace_id` with `docker logs` for the Gateway and the downstream
service. This is local, rotating log inspection; a central collector, archive
retention, and paging integration are not yet qualified for the supported profile.

## D20 rules blocked by reading progress

`422 game_rules_unavailable_at_progress` means the shared immutable rules
template cites chapters beyond the reader's current progress. Novel Service
and Narrative Service preserve this content-free rejection; it is not a
dependency outage. Continue reading before requesting the rules again, or
disable the advanced option to enter in narrative mode. Do not change reading
progress administratively or regenerate a ready template to bypass visibility.

The frontend displays this condition in Chinese and does not automatically
retry it. A genuine transport failure still returns `503 service_unavailable`.
The deterministic browser reproduction is
`pnpm exec playwright test e2e/advanced-rules.spec.ts` from `frontend` after a
frontend build; it uses fixtures and makes no provider calls.

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
