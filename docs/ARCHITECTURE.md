# NovelWorld Architecture

This is the current high-level boundary map. [`AGENTS.md`](../AGENTS.md) owns
the detailed repository layout and coding rules; runtime code, migrations, and
tests own current behavior. Product support and evidence limits are in the
[`product contract`](./PRODUCT_CONTRACT.md).

```text
Browser (React static app)
  -> Nginx (:80, private-preview HTTP)
  -> Gateway (:8080, JWT validation and SSE passthrough)
     -> User Service (:8001)
     -> Novel Service (:8002)
     -> Agent Service (:8003)
     -> Narrative Service (:8004)
  -> PostgreSQL 18 + pgvector
  -> Redis
  -> optional private S3-compatible source storage
  -> operator-configured model and image providers
```

The default Compose stack is a private single-node preview. It does not provide
the TLS, CORS, abuse, policy, recovery, or operational qualification required
for public Internet hosting.

## Quality attributes

The architecture is optimized for correctness and recoverability inside the
private single-node profile:

- **Consistency:** PostgreSQL commits are the success boundary for user-visible
  turns; caches and generated assets cannot make an uncommitted turn complete.
- **Replay safety:** accepted asynchronous work and model-backed turns use
  durable jobs or idempotency keys so retries do not duplicate committed work.
- **Privacy:** identity is derived at the gateway, data ownership remains
  service-local, and deletion/export behavior is explicit in lifecycle
  contracts.
- **Availability:** liveness is process-local, readiness includes required
  dependencies, and overload fails with bounded retryable responses instead of
  accepting unbounded work.
- **Operability:** structured logs, health/readiness probes, Prometheus metrics,
  release manifests, and recovery drills are the supported diagnostic surface.

The measured objectives and topology decision live in [`SLOS.md`](./SLOS.md).
These attributes do not imply multi-region availability, zero data loss, or
public-service qualification.

## Ownership

- **Gateway** authenticates external requests, injects `X-User-Id` and
  `X-User-Role`, applies public routing/admission, and preserves SSE framing.
- **User Service** owns users, password hashes, JWT/refresh tokens, first-run
  configuration, and account-level privacy orchestration.
- **Novel Service** owns shared canonical novels, per-user shelf associations,
  chapters/chunks, characters, canon models, reading progress, source ingestion,
  and optional retained source objects.
- **Agent Service** owns chat turns/messages, memory records, and the bounded
  Redis message projection.
- **Narrative Service** owns narrative nodes, player entities/timelines, world
  state, choices, and the world-turn audit/replay ledger.

Services communicate over HTTP; inter-service clients live under
`infrastructure/http/`. Runtime business SQL is restricted to the relation
owner declared in `tools/architecture/table-ownership-v1.json`. A single
PostgreSQL instance is a deployment choice, not shared business ownership.
External databases, Redis, object storage, model providers, password hashing,
and HTTP services are reached through domain ports and infrastructure adapters.

The shared schema still contains 20 cross-owner `ON DELETE CASCADE` foreign
keys. The user and novel deletion triggers also make two exact writes into the
platform-owned erasure journal; five cross-owner trigger/routine bindings cover
those hooks and the shared row-local timestamp routine. Novel Service readiness
has exact, declared checks against platform lineage/erasure relations and the
User Service deletion trigger. Ten historical migrations with executable `DO`
bodies are pinned by normalized full-file hashes because their effects are not
claimed as semantically parsed. These are single-node migration/audit debt, not
proof of database isolation. The static gate pins FK identities,
trigger/function/relation/access/body/definition hashes, and readiness SQL
fingerprints. It rejects undeclared additions and
stale entries; declaring new debt changes the versioned policy and remains a
blocking review decision. Removing debt requires a separately reviewed ownership migration,
deletion/lifecycle design, and recovery evidence.

## Authoritative state

PostgreSQL is authoritative for application state. Redis, provider-hosted
avatars, generated prose, and search/cache data are projections with explicit
loss or reconstruction boundaries. Optional S3 retention owns original upload
bytes. Before returning `202`, import acceptance atomically commits deterministic
chapters and a PostgreSQL job; claims use renewable leases, and restart recovery
resumes from the `chapters` or `enriched` boundary. The runtime does not yet read
retained S3 objects for full reprocessing.

Chat and world turns reserve a UUID idempotency key, perform model work outside
the transaction, validate bounded output, and emit success only after the
authoritative transaction commits. Completed keys replay without another model
call. Source canon remains immutable; player actions commit to a user-owned
timeline overlay.

## Failure semantics

- A required dependency failure makes the affected service not ready while its
  liveness endpoint remains available for diagnosis.
- Provider failures are bounded by timeout and retry policy; incomplete work
  must not be reported as committed success.
- Redis loss may reduce short-term recall until reconstruction but cannot erase
  committed PostgreSQL chat history.
- A failed release keeps the last promoted release manifest authoritative.
  Database migrations are forward-compatible; rollback never runs a down
  migration.
- Backup/restore is the recovery boundary for authoritative database loss. Its
  evidence and targets are defined in [`BACKUP_RESTORE.md`](./BACKUP_RESTORE.md).

Detailed operator actions belong in [`OPERATIONS.md`](./OPERATIONS.md) and must
change in the same pull request as the failure behavior they describe.

## Code boundaries

Rust services use ports and adapters with the enforced dependency matrix:

```text
domain         -> domain
application    -> application, domain
infrastructure -> infrastructure, domain
interface      -> interface, application, domain
main/lib/root  -> composition only
```

`cargo run --locked -p architecture-check -- check` parses all five runtime
packages with Rust syntax awareness, checks package and layer edges, confines
service HTTP clients to infrastructure adapters, evaluates fail-closed SQL and
known routine calls against the owner manifest, reconciles canonical
relation/view/routine/trigger/FK dependencies, and checks each runtime's static
liveness, readiness, JSON tracing, metrics, and graceful-signal hooks. Tests
nested inside a DDD layer retain layer and SQL checks; root `cfg(test)`
composition modules remain SQL-scanned but have no DDD layer assignment.
Test-only or unreachable files cannot satisfy a runtime hook.

### Backend verification contract

| Invariant | Blocking evidence | Evidence limit |
|---|---|---|
| Domain and application purity | The architecture checker follows each crate's reachable module graph, enforces the layer matrix, rejects unknown production layers, and allows only reviewed pure/orchestration crates in domain/application code. | Static source structure does not prove that a domain rule is behaviorally correct. |
| HTTP and data ownership | Cargo graph checks reject runtime-to-runtime dependencies and unreviewed local/path helpers; source checks constrain reviewed raw/non-HTTP transports and keep service HTTP clients in `infrastructure/http/`; SQL and known routine calls are resolved against the owner manifest; views and routine-call closures are checked transitively; relation/routine inventory, trigger bindings, and cross-owner FK/trigger debt must match exactly. | One PostgreSQL role/schema, 20 acknowledged cascading FKs, two lifecycle-trigger accesses, five cross-owner trigger/routine bindings, ten full-file historical migration audit debts, and the exact readiness exceptions remain. Unknown external crates, proc-macro expansion, generated code, and runtime behavior are not exhaustively proven by source scanning. |
| External configuration | Each production runtime must contain the policy's environment-backed configuration markers; launcher validation and the production Compose smoke exercise required values. | This does not prove secret-manager integration, rotation, or every optional provider combination. |
| Liveness and readiness | Static checks require distinct handlers and local/dependency response semantics; the required Production Compose Smoke starts the topology and verifies dependency-failure transitions. | Probe presence does not prove every dependency failure mode or long-running degradation path. |
| Graceful termination | Static checks require Axum graceful shutdown wired to SIGINT and SIGTERM. | Readiness draining, background-task joins, bounded drain deadlines, and forced-termination drills are not yet qualified. |
| Observability | Static checks require JSON tracing, request trace propagation, and `/metrics`; the main-branch runtime drill validates the structured log contract. | A supported collector, dashboards, alerts, paging ownership, and cardinality budgets are not yet qualified. |
| Durable state and retry safety | Authoritative facts must live in PostgreSQL or configured object storage. A changed retried side effect must carry an idempotency/lease/non-retry contract and a failure/replay test named in its pull request. Existing turn, ingestion, and recovery suites are the scoped evidence. | No static scan can prove that every process-local value is disposable or that every external side effect is exactly-once. |
| Scale | `single-node-v1` capacity and recovery gates are the only current qualification. | Multi-replica safety, horizontal scaling, public-cloud operation, and Internet exposure are unsupported until separately measured and reviewed. |

The result is static structural evidence only. It does not prove database
roles/grants, long-request or background-task drain, complete upstream/SQL
timeouts, metrics collection/alerting, fault recovery, load capacity,
multi-replica correctness, horizontal scaling, or public-cloud readiness. The
supported topology remains one instance of each process in private
`single-node-v1` Compose.

The frontend follows Feature-Sliced Design:

```text
app -> pages -> widgets -> features -> entities -> shared
```

`pages`, `widgets`, `features`, and `entities` are sliced layers. Cross-layer
consumers address a slice only through its root `index.ts`/`index.tsx` public
API; no consumer reaches into another slice's `ui`, `model`, `api`, or other
private path. Imports never point upward, and same-layer slices do not depend on
one another. Relative imports are private to the current slice.

Root entry modules may only bootstrap `app`. The non-sliced `app` and `shared`
layers have the minimum composition exception for their own internal relative
imports; this does not permit bypassing a sliced-layer public API. The blocking
`pnpm lint:fsd` gate analyzes every TypeScript/TSX module under `frontend/src`
with no legacy allowlist, including tests and source-side mocks. It resolves static
and type-only imports, import types, literal dynamic imports, re-exports,
`require`/import-equals, literal Vitest/Jest module APIs (`mock`, `doMock`,
unmocking, and actual/mock loaders), aliases, and relative dependency edges. It
is a structural import check, not evidence of semantic ownership, runtime
loading, or product correctness.

Server state uses TanStack Query, client state uses Zustand, and API/SSE traffic
goes through `frontend/src/shared/api/client.ts`.

## Change rule

Preserve identity, ownership, immutable canon, server-owned spoiler bounds,
commit-before-completion, idempotent replay, and data lifecycle unless a
reviewed contract change explicitly replaces them. Do not add a queue, service,
database, cache, or orchestrator without a measured constraint that the current
design cannot meet.

A change to service boundaries, data ownership, trust boundaries, public
contracts, consistency semantics, or availability targets requires an
[architecture decision record](./adr/0000-template.md). The record must include
alternatives, rollout and rollback, and linked evidence; ordinary local design
choices remain in the pull request.
