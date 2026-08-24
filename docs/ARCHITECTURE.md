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

Services communicate over HTTP and never read another service's tables. A
single PostgreSQL instance is a deployment choice, not shared data ownership.
External databases, Redis, object storage, model providers, and HTTP services
are reached through domain ports and infrastructure adapters.

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

Rust services use `domain -> application -> infrastructure/interface` ports and
adapters. Domain code cannot import infrastructure or HTTP types. Cross-service
reads use HTTP adapters.

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
