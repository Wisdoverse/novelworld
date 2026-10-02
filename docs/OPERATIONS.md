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

For missing cost estimates, check the exact metric provider/model against the
[official pricing snapshot and overrides](./LLM_PRICING.md), then Prometheus
availability/retention. Native currencies need no exchange rate. Unknown prices
and missing usage are distinct from zero cost; a subscription quote is not an
account bill. Invalid price configuration fails User Service startup; remove
the conflicting override and recheck `/ready` rather than changing Diagnostic
budgets or frozen evidence.

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
  503 assertions in [single-node-v1 capacity contract](#single-node-v1-slo-and-capacity-contract).
- **Log contract** — [`log_contract.py`](../tests/e2e/log_contract.py) checks the §14.1 shape and
  trace propagation and request outcome fields.
- **Capacity profile** — [single-node-v1 capacity contract](#single-node-v1-slo-and-capacity-contract) Run locally section; the recorded CI run is
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

`422 game_rules_unavailable_at_progress` means a v1 template cites chapters
beyond the reader's current progress. Novel Service and Narrative Service
preserve this content-free rejection; it is not a dependency outage. V2 chapter
references are provenance only, not an unlock gate; v2 still requires a ready
novel and positive progress, and all narrative context and hard-rule guards
remain enforced. Continue reading before requesting v1 rules again, or
disable the advanced option to enter in narrative mode. Do not change reading
progress administratively or regenerate a ready template to bypass visibility.

The frontend displays this condition in Chinese and does not automatically
retry it. A genuine transport failure still returns `503 service_unavailable`.
The deterministic browser reproduction is
`pnpm exec playwright test e2e/advanced-rules.spec.ts` from `frontend` after a
frontend build; it uses fixtures and makes no provider calls.

### Advanced rules rollback and recovery

Unsetting either Laya setting stops new classifier calls and selects template
fallback for new advanced turns; frozen decisions are unchanged. Before rolling
back application code, disable new advanced-template requests. Template storage
and world-turn resolution columns are additive, but older binaries do not
understand advanced metadata and cannot safely read v4 timeline transitions.
New advanced profiles also require a compatible reader. Once a v4 turn commits,
narrative and advanced timelines may contain canon event timestamps, and
advanced scores may have evolved. Do not strip fields, rewrite scores, or
relabel transitions. Restore service by forward-deploying a compatible release;
there is no down migration, and the immutable template format does not change.

### Ambiguous world-action responses

The reader polls the original turn key with a read-only confirmation request.
A durably failed row unlocks only after the latest world refresh succeeds;
unknown, active, and committed-pending outcomes retain the original key.
Confirmation must not be retried as a generation request. Use the separate
explicit resume only when the original action still needs processing.
Narrative transition rejections log `failure_code=invalid_transition` with a
static `rejection_class` (`json` or `semantic`). These categories do not record
model text or establish the historical failure's exact cause.

## Ownership and escalation

The private self-hosted profile has a single operator (the deployment owner,
as recorded in the [deployment profile decisions](./ARCHITECTURE.md#deployment-profile-decisions)).
The operator owns detection and response. There is no on-call rotation or paging.
The vulnerability-reporting channel in [`SECURITY.md`](../SECURITY.md) is for
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
  5%; check `RATE_LIMIT_RPS` and single-node-v1 capacity contract. **Known gap:** the nginx edge's
  per-client 429s never reach the gateway, so they are not visible here.
- **HighErrorRatio** (warning) — gateway 5xx above 2%; go to the
  bad-release or provider-outage playbook.

`infra/monitoring/drill.sh` verifies the profile: the rules are valid and
provably fire (promtool unit tests), every target scrapes, the
instance-down alert fires and resolves against a live service stop/start,
and Grafana serves the dashboard.

## Single-node-v1 SLO and capacity contract

This contract decides whether NovelWorld's current single-node production
topology is sufficient. It does not predict internet-scale traffic and does not
authorize infrastructure merely because a test is green.

### Applicability

`single-node-v1` explicitly selects `cache_mode=redis` and runs one production
Compose instance of Gateway, each Rust service, PostgreSQL, Redis, Nginx, and the
deterministic test-only LLM provider. It is not evidence for the minimum
`CACHE_MODE=postgres` profile.
The capacity load enters through Gateway's loopback port so Nginx's intentional
20 requests/second per-client abuse limit is not mistaken for application
capacity. Existing production smoke checks continue to verify Nginx itself.

The CI report records host/cgroup CPU and memory limits, platform, commit,
policy version, every raw latency sample, provider call/active/peak counts, and
each pass/fail predicate. It must contain no bearer token, password, provider
key, database password, or Redis password.

### Workload and objectives

| Surface | Versioned workload | Objective |
|---|---|---|
| Import | Three distinct users release >=16 KiB TXT uploads together | Each receives either 202 within 1 s or typed `429 upload_capacity_busy` within 1 s. At least one is accepted; every accepted novel becomes ready within 120 s. Any observed 429 owns no persisted novel and adds no provider work. Fixture retries use a bounded local one-second backoff; upload 429 does not require the parser-overload 503 header. The runner records whether overload was observed and does not infer parser concurrency from completed upload responses. |
| Agent stream | Nine distinct users release one SSE chat turn together; the provider holds stream setup for 1 s | Eight commit; p95 first event <=2.5 s; one receives retryable 503 within 1 s; provider stream peak is eight. |
| World turn | Eight independent first turns release together; the provider holds generation for 1 s | All eight commit exactly once; p95 completion <=3 s; every timeline advances 0 -> 1; provider world-turn peak is eight. |
| Failure/replay | The provider returns one invalid world transition | No state advances; retrying the same UUIDv4 idempotency key commits once; a completed replay is byte-identical and adds no provider call. |
| Database-backed read | One timeline contains 100 committed world turns; eight closed batches issue 128 reads at concurrency 16 | 100% return the 100-turn state and journal; p95 <=750 ms. |
| Redis projection | One character has 60 committed chat turns | PostgreSQL contains all 120 messages; after projection settles Redis contains exactly the newest 50 messages and `MEMORY USAGE` is <=256 KiB. |

The profile uses nine authenticated users and nine independent novels so
per-user admission cannot make shared-capacity results look better than they
are. Fixture creation and warm-up are excluded from latency samples.

### Measurement rules

- Concurrent work uses a barrier and starts timing at the shared release.
- Read load is eight closed batches of 16, preventing a client-side queue from
  hiding latency through coordinated omission.
- p95 is nearest-rank: sorted sample `ceil(0.95 * n) - 1`.
- Expected 503 overload responses are asserted separately and never counted as
  successful in-profile requests.
- Provider delay is test-only and exactly 1,000 ms for stream/world phases; the
  report preserves raw end-to-end latency instead of subtracting that delay.
- HTTP success is insufficient: the runner checks committed turn numbers,
  journal size, exact replay, provider call deltas, PostgreSQL rows, and Redis
  length/memory.
- Every run starts with empty PostgreSQL and Redis volumes and unique fixture
  identifiers. CI always tears the stack down.

### Decision rule

A passing report keeps the current architecture. It does not justify a durable
queue, physical database split, replicas, partitioning, CDN/object storage, or
orchestration.

Likewise, a passing static architecture check prevents known source-boundary
regressions; it does not qualify database isolation, graceful drain, timeout
coverage, monitoring/alerting, replicas, or horizontal scaling. Those outcomes
need separate runtime or migration evidence.

A failure must name the failed predicate and retain the report. Open a narrow
follow-up only after reproducing it. Prefer tuning or removing work inside the
current component first. Any infrastructure proposal must state the measured
bottleneck, expected improvement, migration cost, and rollback. Do not weaken a
threshold merely to restore green CI; change the policy version when a product
requirement genuinely changes. Compare reports only on comparable recorded
hardware; a faster machine is not evidence that a slower deployment meets the
same contract.

### Run locally

From an empty test topology:

```bash
export CACHE_MODE=redis
export REDIS_PASSWORD="Aa0._~-Z$(openssl rand -hex 16)"
export REDIS_URL="redis://:${REDIS_PASSWORD}@redis:6379"
RATE_LIMIT_RPS=500 docker compose -f docker-compose.yml -f docker-compose.e2e.yml \
  --profile redis up -d --build --wait
python3 tools/capacity/run.py \
  --policy tools/capacity/policy-v1.json \
  --report /tmp/novelworld-capacity-report.json
```

The runner uses only the Python standard library. `python3
tools/capacity/run.py --self-test` verifies policy validation and nearest-rank
calculation without starting services.
## Deferred to H5

Journey SLIs, the initial SLO/error budget, alert notification
routing/dedup/paging (the rules fire; nothing pages yet), and postmortem
tooling.

### Uploads while parsing is busy

Parsing capacity is independent of upload acceptance. New accepted books remain
`pending` in `novel_import_jobs` with attempt zero until the existing worker can
claim them. `429 upload_capacity_busy` means short-lived upload preparation is
busy; `503 import_capacity_busy` remains a bounded retry admission response.
A 202 confirms durable acceptance, not completion or model quality. The UI can
select 50 books and submits bounded batches sequentially; partial acceptance
preserves confirmed books. After an unknown batch response, check the shelf
before resubmitting those files. PostgreSQL owns the queue; Redis is optional
projection/cache infrastructure. Original files are retained only when the
S3-compatible storage configuration is enabled.

### Optional RustFS storage and Redis projection

[Deployment configuration](../DEPLOY.md#显式启用-rustfs-原文件存储) covers the
operator-managed RustFS endpoint, application credentials, prefix permissions,
and the separate object-storage backup boundary. RustFS is not provisioned by
the repository Compose stack. Novel Service `/ready` includes `HeadBucket`
when S3 is enabled; the health script does not independently manage or qualify
an external RustFS deployment. Check both the operator's storage health and an
application-account PUT/GET/DELETE probe, including denied out-of-scope access.
An unreachable endpoint, missing bucket, or denied HEAD must be resolved before
accepting retained-source uploads; never treat lost source bytes as recoverable
from Redis.

With `CACHE_MODE=redis`, confirm authenticated Redis readiness and Agent Service
readiness after recreation. Redis holds disposable message projections, while
PostgreSQL owns imports, leases, and committed turns. Enabling either optional
dependency does not backfill old source files, rerun terminal imports, implement
automatic content deduplication, or qualify model quality. Ready shared-catalog
attachment is the current reuse path; replacing a failed shelf entry must
preserve terminal import history and use a separately authorized successful
import/attachment followed by supported shelf removal.

### Advanced D20 entry reports unavailable

Inspect the typed response before treating this as a service outage.
`422 game_rules_unavailable_at_progress` for v1 means the immutable template
references chapters the reader has not unlocked. V2 citations are provenance,
not an unlock decision. Continue reading or choose narrative mode for the v1
progress response; do not regenerate the template, advance stored progress, or
retry provider work to bypass that boundary. `422 game_rule_sources_unavailable`
means no usable source-backed mechanic was found or bounded input exceeded
32 KiB. That preflight failure takes no generation claim and dispatches no
provider request; more reading does not automatically fix missing mechanics or
the input bound. Choose narrative mode or wait for corrected canonical source.
For v2, claim admission uses one five-second PostgreSQL transaction attempt;
never retry an uncertain commit or dispatch provider work after unknown outcome.
An ambiguous durable claim may remain and is not budget-refilled. Before applying
0030, the release target must contain all five required barriers (0021/0024/0025/0030/0036);
old Novel and Narrative writers must both be stopped and drained, then restarted
as compatible versions. Template generation failure and dependency failures are
separate cases. Migration 0031 adds series tables through the normal managed
migration path and does not add a release barrier. Migration 0033 permits a
background-only series with a pending D20 snapshot; it also follows the normal
managed migration path. `409 series_rule_source_unavailable` for a pending
target member means the selected source book has not yet supplied a ready v2
template. Migration 0035 adds default-off series contribution consent. Apply it
before using the matching Novel/frontend version; desktop embeds it. Community
suggestions read live membership pairs without model dispatch, and withdrawal
or account deletion removes contributions from subsequent reads. A suggestion
is a grouping hint, not a verified common setting. No operator key or new
environment variable is needed; see [ADR 0012](./adr/0012-opt-in-community-series.md).
Migration 0034 allows the reader to associate books before confirming
a shared background; `409 series_background_pending` means shared D20 is not
available until that one-time confirmation. Narrative mode remains available.
The reader may explicitly generate
rules for the source book in series management; do not generate target-book
substitutes, reset a terminal generation claim, or treat the 409 as an outage.
Upgrade Novel, Narrative,
and frontend together; after series-bound state exists, rollback to an
application that cannot read it is unsupported. The [D20 responsibility and
evaluation plan](./adr/0009-bounded-laya-d20-adjudication.md) explains the optional Laya (Jev) classification preview and its fallback. A
missing or failing classifier is not this 422 and falls back to the template
check for a new advanced turn; frozen decisions replay without another call.
Both names refer to the same decision capability; configuration still uses
`LAYA_API_URL` and `LAYA_API_KEY`. No semantic-quality qualification is implied.

An original player may enter with no initial place. Deploy Narrative, Agent,
and frontend together while ingress is quiesced. Once a `null` player location
is stored, an older runtime that requires a place cannot safely read it;
recovery requires a compatible forward release.

### Migration 0002 rejects a legacy progress row

Migration 0002 replay must preserve import-created progress metadata for
`pending`, `parsing`, or `error` novels that have no effective chapter within
their advertised `total_chapters`; `current_chapter = 1` does not prove a
chapter exists. It must still fail closed when a `ready` or missing/unrecognized
status has no effective chapter. Only normalize against an existing effective
chapter. Before a managed release replays migrations, take and verify a
PostgreSQL backup. Do not delete or rewrite progress rows to make migration 0002
pass. If the error persists, collect only aggregate counts grouped by novel
status, advertised chapter count, actual chapter shape, and progress validity;
keep book, reader, and chapter content out of logs and review artifacts.

The one-click `start.sh` builds images before `docker compose down`, so a build
failure leaves existing containers running. After a successful build it stops
old writers, then starts the full stack with `up --no-build`, including migration
replay. It is not an ingress-only recovery command.
For an existing stopped edge container, follow the single-container Nginx
restart procedure in [DEPLOY.md](../DEPLOY.md); do not start dependent services
as a side effect.

Migration 0019 detects first adoption by checking whether `user_novels` exists
before creating it. It captures that decision in a transaction-local setting
while holding the `novels` lock. A missing relation permits the initial
uploader-shelf backfill; an existing relation, even an empty one, is preserved
as the adopted state so replay cannot resurrect an intentional detach. This is
not a general repair for an arbitrary partial historical migration. The
`full_migration_replay_preserves_incomplete_progress_and_terminal_imports` integration regression
covers repeated full migration replay and detached-shelf preservation. Issue
#424 tracks the final CI and live deployment evidence.

### Series recognition abstains

A low-confidence Laya result is a valid abstention, not an association failure.
Check the configured endpoint/version and the owned Ready candidate set.
Book titles may be arbitrary and authors absent; neither is a required match
key. Novel can rank candidates using internally stored, source-cited whole-book
location/faction names. Only a unique strong local candidate after a normal
Laya abstention is shown, labeled as a server heuristic and still requiring
reader confirmation. Tied, incomplete-Canon or oversized Ready shelves
abstain; use manual selection rather than retrying paid work. Recognition sends only the reviewed
metadata and bounded chapter-1 location/faction names; it does not send
raw excerpts, summaries or later plot. The optional pinned private endpoint
rejects state overflow instead of silently truncating it. A runtime upgrade
alone does not qualify matching accuracy.

The dialog offers manual association and an explicit DeepSeek second opinion.
Ordinary recognition never triggers a paid fallback. The second opinion requires
an actual DeepSeek configuration and no Diagnostic binding, has a 30 s total
deadline and one physical attempt, with at most 512 total output tokens including
reasoning when enabled, and caches only a validated candidate or
fixed reason. Pending/unknown claims are not reclaimed after restart; do not
delete them to force a retry. Result-only queries (`check_only=true`) never
create a claim or dispatch, even when inputs have changed. Laya endpoint changes
change cache identity; for an in-place model upgrade, use a new endpoint identity
to avoid retaining prior-version suggestions. Repeated requests reuse state while rechecking
current shelf permissions and Ready sources. Apply 0032 before starting the new
Novel service. Shelf/account deletion and safe export include this owner state.
See [ADR 0011](./adr/0011-confirmed-series-worlds.md) and
[arbitrary-title matching #440](https://github.com/Wisdoverse/novelworld/issues/440) for
source, test, CI, deployment and semantic-quality evidence limits.

### Existing world cannot reach later source scenes

Inspect the effective source chapter separately from the immutable entry
checkpoint and current turn number. After the current turn reaches terminal memory projection and no
scheduled/delayed event remains, the reader automatically admits one next source
chapter. Its turn source must match the current effective source, so reloads
cannot advance the same turn twice. Admission is provider-free; subsequent
ordinary actions consume newly eligible events. Inspect user/novel identity,
progress/route agreement and pending recovery if automatic admission is absent. An empty safe
Canon delta is not proof of newly plotted events.

`reading_progress_changed` means the exact progress snapshot or self identity
changed before the guarded owner update. No progress write or source dispatch
occurred from that attempt. Recover the latest state; restore a rewound original
reading boundary only through the deliberate original-reading recovery. Update
Novel before the automatic frontend: older Novel versions refuse the new guard
field without admitting source.

`world_source_busy` means a turn or its pending memory projection retains
authority; finish exact turn recovery first. `world_source_changed` requires a
fresh world view and a new deliberate action after resolving the old operation.
`world_source_order_conflict` means new pinned extraction order would precede an
already advanced event; do not reorder committed history.
`world_source_unavailable` requires the exact pinned chapter/model or bounded
safe definitions; do not substitute latest Canon or crop provenance.
`world_source_outcome_unknown` retains the original key and command until
progress is restored and exact replay returns current guarded truth. A Novel
advance followed by Narrative failure is a partial two-owner outcome; never
rewind progress or reset the world to hide it.

Apply 0036 before enabling matching Novel/Narrative/frontend source admission.
The managed release drain/marker sequence remains mandatory. Older Narrative
cannot read extended schema-v2 worlds; recover by forward deployment. Source
journal rows are included in export and cascade with their owning world.
