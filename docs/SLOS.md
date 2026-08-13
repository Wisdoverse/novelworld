# Single-node-v1 SLO and capacity contract

This contract decides whether NovelWorld's current single-node production
topology is sufficient. It does not predict internet-scale traffic and does not
authorize infrastructure merely because a test is green.

## Applicability

`single-node-v1` runs one production Compose instance of Gateway, each Rust
service, PostgreSQL, Redis, Nginx, and the deterministic test-only LLM provider.
The capacity load enters through Gateway's loopback port so Nginx's intentional
20 requests/second per-client abuse limit is not mistaken for application
capacity. Existing production smoke checks continue to verify Nginx itself.

The CI report records host/cgroup CPU and memory limits, platform, commit,
policy version, every raw latency sample, provider call/active/peak counts, and
each pass/fail predicate. It must contain no bearer token, password, provider
key, database password, or Redis password.

## Workload and objectives

| Surface | Versioned workload | Objective |
|---|---|---|
| Import | Three distinct users release >=16 KiB TXT uploads together | Exactly two receive 202 within 1 s; one receives 503 within 1 s, owns no persisted novel, and causes no provider work; accepted novels become ready within 120 s. |
| Agent stream | Nine distinct users release one SSE chat turn together; the provider holds stream setup for 1 s | Eight commit; p95 first event <=2.5 s; one receives retryable 503 within 1 s; provider stream peak is eight. |
| World turn | Eight independent first turns release together; the provider holds generation for 1 s | All eight commit exactly once; p95 completion <=3 s; every timeline advances 0 -> 1; provider world-turn peak is eight. |
| Failure/replay | The provider returns one invalid world transition | No state advances; retrying the same UUIDv4 idempotency key commits once; a completed replay is byte-identical and adds no provider call. |
| Database-backed read | One timeline contains 100 committed world turns; eight closed batches issue 128 reads at concurrency 16 | 100% return the 100-turn state and journal; p95 <=750 ms. |
| Redis projection | One character has 60 committed chat turns | PostgreSQL contains all 120 messages; after projection settles Redis contains exactly the newest 50 messages and `MEMORY USAGE` is <=256 KiB. |

The profile uses nine authenticated users and nine independent novels so
per-user admission cannot make shared-capacity results look better than they
are. Fixture creation and warm-up are excluded from latency samples.

## Measurement rules

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

## Decision rule

A passing report keeps the current architecture. It does not justify a durable
queue, physical database split, replicas, partitioning, CDN/object storage, or
orchestration.

A failure must name the failed predicate and retain the report. Open a narrow
follow-up only after reproducing it. Prefer tuning or removing work inside the
current component first. Any infrastructure proposal must state the measured
bottleneck, expected improvement, migration cost, and rollback. Do not weaken a
threshold merely to restore green CI; change the policy version when a product
requirement genuinely changes. Compare reports only on comparable recorded
hardware; a faster machine is not evidence that a slower deployment meets the
same contract.

## Run locally

From an empty test topology:

```bash
RATE_LIMIT_RPS=500 docker compose -f docker-compose.yml -f docker-compose.e2e.yml up -d --build
python3 tools/capacity/run.py \
  --policy tools/capacity/policy-v1.json \
  --report /tmp/novelworld-capacity-report.json
```

The runner uses only the Python standard library. `python3
tools/capacity/run.py --self-test` verifies policy validation and nearest-rank
calculation without starting services.
