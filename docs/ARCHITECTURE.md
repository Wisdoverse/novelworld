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

## Ownership

- **Gateway** authenticates external requests, injects `X-User-Id` and
  `X-User-Role`, applies public routing/admission, and preserves SSE framing.
- **User Service** owns users, password hashes, JWT/refresh tokens, first-run
  configuration, and account-level privacy orchestration.
- **Novel Service** owns novels, chapters/chunks, characters, canon models,
  reading progress, source ingestion, and optional retained source objects.
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

## Code boundaries

Rust services use `domain -> application -> infrastructure/interface` ports and
adapters. Domain code cannot import infrastructure or HTTP types. Cross-service
reads use HTTP adapters.

The frontend follows Feature-Sliced Design:

```text
app -> pages -> widgets -> features -> entities -> shared
```

Imports only point downward. Server state uses TanStack Query, client state uses
Zustand, and API/SSE traffic goes through `frontend/src/shared/api/client.ts`.

## Change rule

Preserve identity, ownership, immutable canon, server-owned spoiler bounds,
commit-before-completion, idempotent replay, and data lifecycle unless a
reviewed contract change explicitly replaces them. Do not add a queue, service,
database, cache, or orchestrator without a measured constraint that the current
design cannot meet.
