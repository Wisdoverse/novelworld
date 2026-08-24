# NovelWorld Agent Instructions

NovelWorld is a full-stack platform that transforms novels into interactive
worlds. It combines a Rust microservice backend (Axum), PostgreSQL with
pgvector, an optional Redis projection, and a React/TypeScript frontend following Feature-Sliced
Design.

`CLAUDE.md` is a symlink to this file. Keep this file as the canonical local
agent entrypoint.

## Current Runtime Contract

- Backend ownership is Rust. Five services in the workspace: `gateway`,
  `user-service`, `novel-service`, `agent-service`, `narrative-service`.
- The React app in `frontend/` talks to the gateway on `:8080` over HTTP.
  SSE streaming is used for character conversations.
- All services share a single PostgreSQL 18 database with pgvector, pg_trgm,
  and uuid-ossp extensions.
- Redis is an explicit optional short-term projection for agent-service;
  PostgreSQL-backed operation is the default minimum profile.
- LLM calls go to an OpenAI-compatible API. All calls implement retry
  (3 retries, exponential backoff 1s/2s/4s, Retry-After header support).
- JWT authentication flows through the gateway. Downstream services receive
  `X-User-Id` and `X-User-Role` headers injected by the gateway middleware.

Runtime shape:

```text
Browser (Vite dev server or Nginx static)
  → Gateway (:8080) — JWT validation, SSE passthrough
  → User Service (:8001) — auth, tokens
  → Novel Service (:8002) — ingestion, parsing, progress
  → Agent Service (:8003) — memory, chat, SSE streaming
  → Narrative Service (:8004) — branches, choices, world state
  → PostgreSQL (:5432) / optional Redis (:6379) / optional S3-compatible object storage
```

Data flow for a conversation turn:

```text
Browser POST /api/chat/:characterId/stream
  → Gateway validates JWT, injects X-User-Id
  → Agent Service reserves the Idempotency-Key and snapshots server-side progress
  → Agent Service retrieves committed chat and the available memory layers
  → Agent Service retrieves relevant lore up to the reader's current chapter
  → Agent Service builds system prompt (character + lore + memories + anti-spoiler)
  → Agent Service streams LLM response via SSE
  → Atomically store both messages and complete the turn
  → Emit done only after commit; then project Redis memory/compression
```

## Repository Map

- `gateway/` — Axum API gateway. JWT middleware, reverse proxy, SSE passthrough.
- `services/user-service/` — Authentication. Register, login, JWT, refresh tokens.
- `services/novel-service/` — Novel ingestion pipeline. Chapter splitting
  (regex + LLM fallback), character extraction, relationship graph, avatar
  generation, reading progress.
- `services/agent-service/` — Character AI. Durable chat, memory retrieval and
  projections (short/mid/long/permanent schema), SSE streaming, compression.
- `services/narrative-service/` — Branch logic. Narrative nodes, choice
  submission, consequence generation, world state mutations.
- `frontend/` — React/TypeScript/Tailwind app. Feature-Sliced Design.
- `infra/postgres/` — Schema (`init.sql`), explicit development seed fixture, extensions.
- `infra/nginx/` — Reverse proxy config with SSE support.
- `docs/` — Architecture docs.

The current supported product/deployment envelope and evidence limits are in
`docs/PRODUCT_CONTRACT.md`.

Each Rust service follows layered architecture:

```text
src/
├── main.rs              — bootstrap, middleware, server
├── domain/
│   ├── entities/        — aggregates, value objects
│   ├── repositories/    — trait definitions (ports)
│   └── services/        — domain logic
├── application/
│   ├── commands/        — command DTOs
│   └── handlers/        — use-case orchestration
├── infrastructure/
│   ├── persistence/     — PostgreSQL implementations (adapters)
│   ├── cache/           — Redis (agent-service only)
│   └── llm/             — OpenAI-compatible client
└── interface/
    └── http/            — Axum routes, request/response DTOs
```

## Naming Rules

- `Novel` — an uploaded book being processed.
- `Chapter` — a section of a novel, identified by `novel_id` + `chapter_number`.
- `Character` — an extracted fictional person, exposed as an AI agent.
- `Memory` — a stored fact about a character-user interaction. Layered:
  optional `short` projection (Redis), `mid` (PG summary), `long` (PG +
  pgvector), `permanent`. PostgreSQL remains authoritative.
- `NarrativeNode` — a branch point in the story with multiple choices.
- `WorldState` — JSONB document tracking a reader's choices and relationships.
- `ReadingProgress` — a reader's position and identity within a novel.

## Commands

Rust:

```bash
cargo build --workspace
cargo check --workspace
cargo test --workspace
cargo run --locked -p architecture-check -- self-test
cargo run --locked -p architecture-check -- check
cargo run -p gateway
cargo run -p user-service
cargo run -p novel-service
cargo run -p agent-service
cargo run -p narrative-service
cargo test -p novel-service                      # single service
cargo test -p novel-service test_chapter_split   # single test
```

Frontend:

```bash
cd frontend
pnpm install
pnpm dev
pnpm build
pnpm lint
pnpm lint:fsd
pnpm type-check
```

Docker:

```bash
docker compose up -d postgres                # minimum infrastructure
# Explicit Redis projection: the supported launcher derives these together.
export CACHE_MODE=redis REDIS_PASSWORD="Aa0._~-Z$(openssl rand -hex 16)"
export REDIS_URL="redis://:${REDIS_PASSWORD}@redis:6379"
docker compose --profile redis up -d postgres redis
docker compose up --build                     # full stack
docker compose -f docker-compose.yml up -d    # production
```

## Architecture Constraints

- **Backend: Cloud Native + DDD + Microservice architecture is mandatory within
  the current private `single-node-v1` contract.** Every change must respect:
  - Domain code depends only on domain code and pure data types. Application
    code orchestrates domain ports; database, Redis, object storage, model,
    password, and outbound HTTP implementations live in infrastructure.
  - Services communicate over HTTP. Inter-service clients live under
    `infrastructure/http/`; runtime packages never depend on another runtime
    package.
  - Each relation has one owner in
    `tools/architecture/table-ownership-v1.json`. Runtime SQL may access only
    owner relations or an exact, fingerprinted readiness exception. The shared
    schema's 20 cross-owner foreign keys, two lifecycle-trigger accesses, and
    five cross-owner trigger/routine bindings are declared migration debt. Ten
    historical migrations with executable `DO` bodies are locked by normalized
    full-file hashes rather than claimed as semantically parsed. The gate blocks
    undeclared or stale debt; any declared growth changes the versioned policy
    and must be justified in review. It does not claim database-level isolation.
    Views, routines, and trigger bindings are checked transitively against the
    same ownership policy.
  - Authoritative state is externalized. Process-local state is limited to
    disposable caches, projections, readiness caches, and bounded admission;
    it must not become the only copy of a committed fact.
  - Runtime configuration and secrets are externalized. Every process exposes
    separate liveness/readiness, structured JSON tracing, a metrics endpoint,
    and graceful SIGINT/SIGTERM handling. Every new or changed dependency call
    states its deadline and retry contract; existing unqualified timeout/drain
    paths remain documented gaps, not precedent. Retried side effects require
    idempotency, a durable claim/lease, or an explicit non-retry contract.
  - `cargo run --locked -p architecture-check -- check` is the blocking static
    gate. Passing it proves only the rules it scans; it does not prove drain
    completion, timeout coverage, recovery, alerting, database privileges,
    multi-replica correctness, horizontal scaling, or public-cloud readiness.
- **Frontend: Feature-Sliced Design (FSD) is mandatory.** Import rules:
  - `app` → `pages` → `widgets` → `features` → `entities` → `shared`.
  - `pages`, `widgets`, `features`, and `entities` are sliced layers. Every
    cross-layer import into one of their slices must use that slice's root
    `index.ts`/`index.tsx` public API (for example, `@/features/auth`), never
    `ui/`, `model/`, `api/`, or another private path below the slice root.
  - Imports never point upward, and one slice must not import another slice in
    the same layer. Code within a slice may use relative imports to its own
    private segments.
  - Root entry modules may only bootstrap `app`. The non-sliced `app` and
    `shared` layers have the minimum composition exception for their own
    internal relative imports. They still cannot bypass a sliced-layer public
    API or violate the downward layer direction.
  - The rule covers every TypeScript/TSX module under `frontend/src`, including
    tests and source-side mocks. Static and type-only imports, import types,
    literal dynamic imports, re-exports, `require`/import-equals, and literal
    Vitest/Jest module APIs (`mock`, `doMock`, unmocking, and actual/mock
    loaders), aliases, and relative paths are evaluated as architecture edges.
  - `pnpm lint:fsd` has no legacy allowlist. Any reported boundary violation
    blocks merge.

Violating these constraints is a blocking issue — fix before merging.

## Code Style

### Rust

- Use `sqlx::query_as::<_, RowStruct>(...)` for SELECT queries.
- Repository traits in `domain/repositories/`, implementations in
  `infrastructure/persistence/`.
- Enum-to-string conversion via `to_str()`/`from_str()` methods, not Display.
- All LLM calls go through domain port traits (`LlmPort`/`TextSummarizer`) with built-in retry.
- SSE responses use `axum::response::Sse` with `async_stream`.
- Error handling: `anyhow::Result` for application code, `thiserror` for
  domain errors.

### Frontend

- Feature-Sliced Design follows the mandatory public-API, slice-isolation, and
  full-source rules in [Architecture Constraints](#architecture-constraints).
  `pnpm lint:fsd` is the blocking structural gate; passing it does not prove
  runtime behavior, semantic ownership, or product correctness.
- State: Zustand for client state, TanStack Query for server state.
- API: All calls through `shared/api/client.ts` (axios with JWT interceptor).
- SSE: Custom `createChatStream()` in `shared/api/client.ts` using fetch +
  ReadableStream (not EventSource — POST not supported).
- Styling: Tailwind CSS with custom design tokens in `app/styles/globals.css`.
- Path alias: `@/` maps to `src/`.

## Database

Schema lives in `infra/postgres/init.sql`. Key tables:

| Table | Purpose |
|-------|---------|
| `users` | Auth, profiles |
| `novels` | Uploaded books, parse status |
| `user_novels` | Per-user shelf access to shared canonical novels |
| `novel_import_jobs` | Durable import stage, attempt, lease, and terminal state |
| `chapters` | Split chapter content |
| `chapter_chunks` | Derived chunks for spoiler-bounded lore retrieval |
| `chapter_translations` | Source-bound, lease-fenced Simplified Chinese chapter translations |
| `characters` | Extracted characters with system prompts |
| `character_memories` | Layered memory records + optional pgvector embeddings |
| `character_relationships` | Entity relationship graph between characters |
| `chat_messages` | Conversation history |
| `chat_turns` | Idempotency, lease, and commit state for conversation turns |
| `world_turns` | Idempotency, lease, audit, and exact replay state for open-world turns |
| `narrative_nodes` | Branch points with JSONB choices |
| `user_choices` | Reader's branch decisions |
| `world_states` | JSONB world state per reader per novel |
| `reading_progress` | Chapter position, reader identity |
| `refresh_tokens` | JWT refresh token storage |

IDs are UUID v4 by default. The committed-world-turn journey-memory projection
is the explicit exception: it uses a private, fixed UUID v5 namespace so a
pending replay addresses the same durable fact. All timestamps are TIMESTAMPTZ
(UTC).

## Environment Variables

Copy `.env.example` to `.env`. Bootstrap/runtime surfaces:

- `JWT_SECRET` — min 32 chars
- `RUNTIME_CONFIG_KEY` — 64-character hex key generated by `start.sh`/`start.cmd`
- `INTERNAL_SERVICE_TOKEN` — generated service-to-service configuration token
- `DATABASE_URL` — PostgreSQL connection string
- `CACHE_MODE` — `postgres` (default) or explicit `redis`
- `REDIS_PASSWORD` / `REDIS_URL` — required together only for Redis mode;
  supported launch/release tools derive the URL from the persisted mode/password

`LLM_API_KEY` is optional: set it for an operator-managed environment override,
or leave it empty, create the first administrator without a provider call, and
complete DeepSeek/OpenAI setup later in protected Settings.

`S3_ENABLED` is optional. When true, configure `S3_BUCKET` and `S3_REGION`;
`S3_ENDPOINT` and path-style addressing support S3-compatible providers. Use
either explicit `S3_ACCESS_KEY`/`S3_SECRET_KEY` credentials or the standard AWS
credential provider chain.

See `.env.example` for the full list with defaults.

## Testing

Use the narrow commands above while iterating. Before review, follow the
affected-gate matrix in [`CONTRIBUTING.md`](./CONTRIBUTING.md#verification);
CI remains the authoritative required gate.

## GitHub Project Governance

- `docs/ROADMAP.md` owns product direction, invariants, horizon ordering, and
  exit criteria. [NovelWorld Roadmap](https://github.com/users/schorsch888/projects/2)
  owns execution status, horizon assignment, and priority.
- Roadmap work starts from the roadmap Issue Form. One issue represents one
  independently mergeable outcome and records scope, non-goals, invariants,
  acceptance evidence, dependencies, and rollback. Do not pre-create
  speculative work for a horizon that has not started.
- Add active roadmap issues and their pull requests to the Project. Set
  `Horizon`, `Priority`, and `Status`; use `In Progress` only while work is
  actively owned.
- Roadmap pull requests must link their issue with `Closes #<issue>`. `Done`
  means the final commit is merged to `main` and required CI is green. A pushed
  branch, open pull request, or delegated auto-merge is not done.
- Update the roadmap status only when its stated evidence or exit criteria are
  true. Record blockers on the issue instead of reporting optimistic status.

## Gotchas

- Use `sqlx::query()` with `.bind()`, NOT `sqlx::query!()` macro — no DATABASE_URL at compile time.
- `deadpool-redis 0.23` requires `redis 1.2`. `redis::AsyncCommands` uses `isize` for range params.
- `sqlx 0.9` renamed feature: `runtime-tokio-rustls` → `runtime-tokio` + `tls-rustls`.
- Novel `domain_events` field must be `pub` for infrastructure reconstruction from DB rows.
- Chapter splitter filters out chapters < 100 chars — test data must be long enough.
- `axum 0.8` wildcard routes use `{*path}` syntax, not `*path`.
- Gateway SSE proxy must NOT set Content-Length — use `Body::from_stream()` for passthrough.

## DDD Rules

- Domain layer (`domain/`) must never import from `application/`,
  `infrastructure/`, or `interface/`, or a concrete external adapter crate.
- Application handlers hold `Arc<dyn Port>`, not `Arc<ConcreteType>`.
- Port traits live in `domain/ports.rs`. Infra types implement them.
- Services must not query another service's owned business relations. Use HTTP
  adapters (`infrastructure/http/`) for cross-service behavior. The current
  shared-schema foreign keys, lifecycle triggers, and exact readiness exceptions
  are explicit debt, not permission for new coupling.
- `NOVEL_SERVICE_URL` env var for agent-service and narrative-service to call novel-service.
- Value object serialization (`to_str`/`from_str`) belongs in `domain/value_objects/`, not in persistence layer.

## Inter-Service Communication

- Gateway injects `X-User-Id` and `X-User-Role` headers from JWT claims.
- Downstream services extract user identity from these headers, never from JWT directly.
- novel-service exposes `GET /characters/:id` for agent-service lookups.
- All LLM calls use domain port traits with 3x exponential backoff retry.

## Known Gaps (Not Yet Implemented)

Product, quality, recovery, and release gaps are owned by `docs/ROADMAP.md` and
the active GitHub Project issue. Do not infer completion from this section or
from a merged structural-control PR.

## Security Notes

- Never commit `.env`, credentials, or API keys.
- All SQL uses parameterized queries (no string interpolation).
- JWT tokens expire per `AUTH_ACCESS_TOKEN_EXPIRY` (default 1h).
- Refresh tokens are server-side, bounded, and expire; they are currently stored as opaque values.
- User input is passed to LLM prompts — the system prompt includes behavioral
  constraints to mitigate prompt injection, but this is defense-in-depth, not
  a guarantee.
- Passwords hashed with bcrypt, cost factor 12.
