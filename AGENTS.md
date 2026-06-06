# NovelWorld Agent Instructions

NovelWorld is a full-stack platform that transforms novels into interactive
worlds. It combines a Rust microservice backend (Axum), PostgreSQL with
pgvector, Redis, and a React/TypeScript frontend following Feature-Sliced
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
- Redis is used for short-term memory caching in the agent-service.
- LLM calls go to an OpenAI-compatible API. All calls implement retry
  (3 attempts, exponential backoff 1s/2s/4s, Retry-After header support).
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
  → PostgreSQL (:5432) / Redis (:6379)
```

Data flow for a conversation turn:

```text
Browser POST /api/chat/:characterId/stream
  → Gateway validates JWT, injects X-User-Id
  → Agent Service retrieves 4-layer memory pyramid
  → Agent Service builds system prompt (character + memories + anti-spoiler)
  → Agent Service streams LLM response via SSE
  → Post-stream: store messages, update short-term memory, trigger compression
```

## Repository Map

- `gateway/` — Axum API gateway. JWT middleware, reverse proxy, SSE passthrough.
- `services/user-service/` — Authentication. Register, login, JWT, refresh tokens.
- `services/novel-service/` — Novel ingestion pipeline. Chapter splitting
  (regex + LLM fallback), character extraction, relationship graph, avatar
  generation, reading progress.
- `services/agent-service/` — Character AI. 4-layer memory pyramid
  (short/mid/long/permanent), SSE streaming, memory compression.
- `services/narrative-service/` — Branch logic. Narrative nodes, choice
  submission, consequence generation, world state mutations.
- `frontend/` — React/TypeScript/Tailwind app. Feature-Sliced Design.
- `infra/postgres/` — Schema (`init.sql`), seed data, extensions.
- `infra/nginx/` — Reverse proxy config with SSE support.
- `docs/` — Architecture docs.

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
  `short` (Redis), `mid` (PG summary), `long` (PG + pgvector), `permanent`.
- `NarrativeNode` — a branch point in the story with multiple choices.
- `WorldState` — JSONB document tracking a reader's choices and relationships.
- `ReadingProgress` — a reader's position and identity within a novel.

## Commands

Rust:

```bash
cargo build --workspace
cargo check --workspace
cargo test --workspace
cargo run -p gateway
cargo run -p user-service
cargo run -p novel-service
cargo run -p agent-service
cargo run -p narrative-service
```

Frontend:

```bash
cd frontend
pnpm install
pnpm dev
pnpm build
pnpm lint
pnpm type-check
```

Docker:

```bash
docker compose up -d postgres redis          # infrastructure only
docker compose up --build                     # full stack
docker compose -f docker-compose.yml up -d    # production
```

## Code Style

### Rust

- Use `sqlx::query` with `.bind()` params, not `sqlx::query!` macro (no
  compile-time DB required).
- Use `sqlx::query_as::<_, RowStruct>(...)` for SELECT queries.
- Repository traits in `domain/repositories/`, implementations in
  `infrastructure/persistence/`.
- Enum-to-string conversion via `to_str()`/`from_str()` methods, not Display.
- All LLM calls go through the `LlmClient` struct with built-in retry.
- SSE responses use `axum::response::Sse` with `async_stream`.
- Error handling: `anyhow::Result` for application code, `thiserror` for
  domain errors.

### Frontend

- Feature-Sliced Design: `app` → `pages` → `widgets` → `features` →
  `entities` → `shared`. Never import upward.
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
| `chapters` | Split chapter content |
| `characters` | Extracted characters with system prompts |
| `character_memories` | 4-layer memory pyramid + pgvector embeddings |
| `character_relationships` | Entity relationship graph between characters |
| `chat_messages` | Conversation history |
| `narrative_nodes` | Branch points with JSONB choices |
| `user_choices` | Reader's branch decisions |
| `world_states` | JSONB world state per reader per novel |
| `reading_progress` | Chapter position, reader identity |
| `refresh_tokens` | JWT refresh token storage |

All IDs are UUID v4. All timestamps are TIMESTAMPTZ (UTC).

## Environment Variables

Copy `.env.example` to `.env`. Required:

- `JWT_SECRET` — min 32 chars
- `LLM_API_KEY` — OpenAI-compatible API key
- `DATABASE_URL` — PostgreSQL connection string
- `REDIS_URL` — Redis connection string

See `.env.example` for the full list with defaults.

## Testing

25 unit tests across all services. Run with `cargo test --workspace`.

Tests cover: email validation, JWT roundtrip, bcrypt verification, chapter
splitting (Chinese/English/fallback), novel status transitions, memory layer
ordering, anti-spoiler chapter filtering, narrative choice bounds, world state
relationship clamping.

Integration tests require running PostgreSQL and Redis (use Docker Compose).

## Gotchas

- Use `sqlx::query()` with `.bind()`, NOT `sqlx::query!()` macro — no DATABASE_URL at compile time.
- `deadpool-redis 0.23` requires `redis 1.2`. `redis::AsyncCommands` uses `isize` for range params.
- `sqlx 0.9` renamed feature: `runtime-tokio-rustls` → `runtime-tokio` + `tls-rustls`.
- Novel `domain_events` field must be `pub` for infrastructure reconstruction from DB rows.
- Chapter splitter filters out chapters < 100 chars — test data must be long enough.
- `axum 0.8` wildcard routes use `{*path}` syntax, not `*path`.
- Gateway SSE proxy must NOT set Content-Length — use `Body::from_stream()` for passthrough.

## DDD Rules

- Domain layer (`domain/`) must never import from `infrastructure/` or `interface/`.
- Application handlers hold `Arc<dyn Port>`, not `Arc<ConcreteType>`.
- Port traits live in `domain/ports.rs`. Infra types implement them.
- Services must NOT share database tables. Use HTTP adapters (`infrastructure/http/`) for cross-service queries.
- `NOVEL_SERVICE_URL` env var for agent-service and narrative-service to call novel-service.
- Value object serialization (`to_str`/`from_str`) belongs in `domain/value_objects/`, not in persistence layer.

## Inter-Service Communication

- Gateway injects `X-User-Id` and `X-User-Role` headers from JWT claims.
- Downstream services extract user identity from these headers, never from JWT directly.
- novel-service exposes `GET /characters/:id` for agent-service lookups.
- All LLM calls use domain port traits with 3x exponential backoff retry.

## Security Notes

- Never commit `.env`, credentials, or API keys.
- All SQL uses parameterized queries (no string interpolation).
- JWT tokens expire per `AUTH_ACCESS_TOKEN_EXPIRY` (default 1h).
- Refresh tokens stored hashed, with expiry.
- User input is passed to LLM prompts — the system prompt includes behavioral
  constraints to mitigate prompt injection, but this is defense-in-depth, not
  a guarantee.
- Passwords hashed with bcrypt, cost factor 12.
