# NovelWorld

**Transform any novel into a living, interactive world.**

NovelWorld ingests arbitrary novel text, extracts its characters and narrative structure via LLM
analysis, and exposes each character as a stateful AI agent that readers can converse with in
real time. Readers influence the story through branch choices, carry persistent memories with
every character across sessions, and enter the world as themselves or as a character from the book.

> **For coding agents:** This README is your primary implementation guide. Read it top to bottom
> before writing a single line of code. The companion [SPEC.md](./SPEC.md) contains the full
> normative specification with RFC 2119 language. When this README and SPEC.md conflict, SPEC.md
> takes precedence.

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [Repository Layout](#2-repository-layout)
3. [Technology Stack](#3-technology-stack)
4. [Getting Started](#4-getting-started)
5. [Service Implementation Guide](#5-service-implementation-guide)
6. [Frontend Implementation Guide](#6-frontend-implementation-guide)
7. [Database Schema](#7-database-schema)
8. [LLM Integration Patterns](#8-llm-integration-patterns)
9. [Memory Pyramid Implementation](#9-memory-pyramid-implementation)
10. [SSE Streaming Contract](#10-sse-streaming-contract)
11. [Testing Strategy](#11-testing-strategy)
12. [Deployment](#12-deployment)
13. [Environment Variables Reference](#13-environment-variables-reference)
14. [Implementation Checklist](#14-implementation-checklist)

---

## 1. Architecture Overview

NovelWorld is composed of five Rust microservices behind an Axum API gateway, a React frontend
following Feature-Sliced Design, PostgreSQL 18 with pgvector, Redis, and S3-compatible object
storage.

```
                         ┌─────────────────────────────────────────┐
                         │              Nginx (port 80/443)         │
                         └────────────────────┬────────────────────┘
                                              │
                         ┌────────────────────▼────────────────────┐
                         │          API Gateway  :8080              │
                         │   JWT validation · routing · SSE proxy   │
                         └──┬──────┬──────┬──────┬──────┬──────────┘
                            │      │      │      │      │
              ┌─────────────▼┐ ┌───▼───┐ ┌▼────┐ ┌────▼──────┐ ┌──▼──────────┐
              │ User Service │ │Novel  │ │Agent│ │Narrative  │ │(future)     │
              │    :8081     │ │Service│ │Svc  │ │Service    │ │             │
              │              │ │ :8082 │ │:8083│ │  :8084    │ │             │
              └──────┬───────┘ └───┬───┘ └──┬──┘ └────┬──────┘ └─────────────┘
                     │             │         │          │
              ┌──────▼─────────────▼─────────▼──────────▼──────┐
              │              PostgreSQL 18  :5432               │
              │         (pgvector · uuid-ossp · pg_trgm)        │
              └─────────────────────────────────────────────────┘
              ┌──────────────────────┐  ┌──────────────────────┐
              │     Redis  :6379     │  │  S3-compatible store  │
              │  (short-term memory) │  │  (files · avatars)    │
              └──────────────────────┘  └──────────────────────┘
```

**Data flow for a conversation turn:**

1. Browser sends `POST /api/chat/:characterId/stream` with `{ message, chapterNum }`.
2. Gateway validates JWT, injects `user_id`, forwards to Agent Service.
3. Agent Service retrieves memories from all four pyramid layers.
4. Agent Service builds the system prompt (character identity + memories + world state).
5. Agent Service calls the LLM with `stream: true`.
6. Agent Service proxies SSE `delta` events back through the Gateway to the browser.
7. After the stream ends, Agent Service writes a new short-term memory and triggers compression
   if the threshold is exceeded.

---

## 2. Repository Layout

```
novel-world-rust/
├── SPEC.md                         ← Normative specification (RFC 2119)
├── README.md                       ← This file
├── Cargo.toml                      ← Workspace root
├── docker-compose.yml              ← Full stack for local development
├── docker-compose.prod.yml         ← Production overrides
├── .env.example                    ← Environment variable template
│
├── gateway/                        ← API Gateway (Axum)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                 ← Server bootstrap, middleware stack
│       ├── auth.rs                 ← JWT extraction and validation middleware
│       ├── proxy.rs                ← Reverse proxy and SSE passthrough
│       └── routes.rs               ← Route table
│
├── services/
│   ├── user-service/               ← Authentication and user management
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── domain/
│   │       │   ├── entities/user.rs
│   │       │   ├── value_objects/email.rs
│   │       │   ├── value_objects/password.rs
│   │       │   └── repositories/user_repo.rs
│   │       ├── application/
│   │       │   ├── commands/register.rs
│   │       │   ├── commands/login.rs
│   │       │   └── handlers/auth_handler.rs
│   │       ├── infrastructure/
│   │       │   ├── persistence/pg_user_repo.rs
│   │       │   └── auth/jwt.rs
│   │       └── interface/
│   │           └── http/mod.rs
│   │
│   ├── novel-service/              ← Novel ingestion and parsing pipeline
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── domain/
│   │       │   ├── entities/novel.rs
│   │       │   ├── entities/chapter.rs
│   │       │   ├── entities/character.rs
│   │       │   ├── services/chapter_splitter.rs
│   │       │   ├── services/character_extractor.rs
│   │       │   ├── services/world_summarizer.rs
│   │       │   └── services/node_detector.rs
│   │       ├── application/
│   │       │   ├── commands/import_novel.rs
│   │       │   └── handlers/parse_pipeline.rs
│   │       ├── infrastructure/
│   │       │   ├── persistence/pg_novel_repo.rs
│   │       │   ├── llm/client.rs
│   │       │   ├── llm/image.rs
│   │       │   └── storage/s3.rs
│   │       └── interface/
│   │           └── http/mod.rs
│   │
│   ├── agent-service/              ← Character AI, memory pyramid, SSE streaming
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── domain/
│   │       │   ├── entities/memory.rs
│   │       │   ├── entities/chat_message.rs
│   │       │   ├── services/memory_manager.rs
│   │       │   ├── services/prompt_builder.rs
│   │       │   └── repositories/memory_repo.rs
│   │       ├── application/
│   │       │   ├── commands/chat.rs
│   │       │   └── handlers/agent_handler.rs
│   │       ├── infrastructure/
│   │       │   ├── persistence/pg_memory_repo.rs
│   │       │   ├── cache/redis_memory.rs
│   │       │   └── llm/stream_client.rs
│   │       └── interface/
│   │           └── http/mod.rs
│   │
│   ├── narrative-service/          ← Branch logic and world state
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── domain/
│   │       │   ├── entities/narrative_node.rs
│   │       │   ├── entities/world_state.rs
│   │       │   ├── entities/user_choice.rs
│   │       │   └── services/narrative_engine.rs
│   │       ├── application/
│   │       │   ├── commands/submit_choice.rs
│   │       │   └── handlers/narrative_handler.rs
│   │       ├── infrastructure/
│   │       │   └── persistence/pg_narrative_repo.rs
│   │       └── interface/
│   │           └── http/mod.rs
│   │
│   └── user-service/               ← (see above)
│
├── frontend/                       ← React + TypeScript (FSD)
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html
│   └── src/
│       ├── app/                    ← Bootstrap, routing, providers, global CSS
│       ├── pages/                  ← Page composition
│       ├── widgets/                ← Self-contained UI blocks
│       ├── features/               ← User interaction scenarios
│       ├── entities/               ← Business entity models + API hooks
│       └── shared/                 ← UI kit, API client, utilities
│
└── infra/
    ├── postgres/
    │   ├── init.sql                ← Schema + extensions + indexes
    │   └── seed.sql                ← Development seed data
    ├── nginx/
    │   └── nginx.conf
    └── redis/
        └── redis.conf
```

---

## 3. Technology Stack

| Layer | Technology | Version | Notes |
|---|---|---|---|
| Backend language | Rust | stable (≥ 1.78) | Use `rustup` to install |
| HTTP framework | Axum | 0.7 | Async, tower-compatible |
| Async runtime | Tokio | 1.x | `full` feature |
| Database ORM | SQLx | 0.8 | Compile-time query checking |
| Database | PostgreSQL | 18 | Requires pgvector, uuid-ossp, pg_trgm |
| Vector search | pgvector | 0.7+ | HNSW index |
| Cache | Redis | 7+ | Via `redis` crate |
| Object storage | S3-compatible | — | Via `aws-sdk-s3` |
| JWT | `jsonwebtoken` | 9.x | HMAC-SHA256 |
| Password hashing | `bcrypt` | 0.15 | Cost factor ≥ 12 |
| HTTP client | `reqwest` | 0.12 | For LLM API calls |
| Serialization | `serde_json` | 1.x | All JSON I/O |
| Frontend framework | React | 19 | With TypeScript |
| Frontend build | Vite | 6 | |
| Frontend routing | React Router | 6 | |
| Frontend state | Zustand | 4 | Per-feature stores |
| Frontend data | TanStack Query | 5 | Server state management |
| Frontend styling | Tailwind CSS | 4 | With custom design tokens |
| Frontend UI | Radix UI | latest | Accessible primitives |
| Container | Docker | 27+ | Multi-stage builds |
| Orchestration | Docker Compose | v2 | Development and production |

---

## 4. Getting Started

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable

# Install Docker and Docker Compose
# Follow https://docs.docker.com/get-docker/

# Install Node.js 22+ and pnpm
curl -fsSL https://fnm.vercel.app/install | bash
fnm use 22
npm install -g pnpm
```

### Local Development Setup

```bash
# 1. Clone the repository
git clone <repo-url> novel-world-rust
cd novel-world-rust

# 2. Copy and fill environment variables
cp .env.example .env
# Edit .env — at minimum set: LLM_API_KEY, JWT_SECRET, S3_* credentials

# 3. Start infrastructure (PostgreSQL, Redis, MinIO)
docker compose up -d postgres redis minio

# 4. Apply database schema
docker compose exec postgres psql -U novelworld -d novelworld -f /docker-entrypoint-initdb.d/init.sql

# 5. Build and run all services
cargo build --workspace
cargo run -p gateway &
cargo run -p user-service &
cargo run -p novel-service &
cargo run -p agent-service &
cargo run -p narrative-service &

# 6. Start the frontend
cd frontend
pnpm install
pnpm dev
```

The application is now available at `http://localhost:5173` (frontend) and `http://localhost:8080`
(API gateway).

### Docker Compose Full Stack

```bash
# Build and start everything
docker compose up --build

# The stack exposes:
# http://localhost:80   → Nginx (frontend + API proxy)
# http://localhost:8080 → API Gateway (direct access)
# http://localhost:9001 → MinIO console (object storage UI)
```

---

## 5. Service Implementation Guide

### 5.1 User Service

The User Service owns all authentication state. It is the only service that issues JWTs.

**Key implementation points:**

The `register` handler must validate email format with a regex or the `email_address` crate,
check uniqueness with a case-insensitive query (`WHERE LOWER(email) = LOWER($1)`), hash the
password with `bcrypt::hash(password, 12)`, and return both an access token and a refresh token.

The `login` handler must use `bcrypt::verify` for password comparison. Timing-safe comparison is
handled by bcrypt internally; do not add additional constant-time logic.

JWT issuance uses the `jsonwebtoken` crate. The access token payload must include `sub` (user UUID
as string), `role`, `iat`, and `exp`. Sign with `Algorithm::HS256` using the `JWT_SECRET`
environment variable.

```rust
// Example JWT claims structure
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,       // user UUID
    pub role: String,      // "user" or "admin"
    pub iat: usize,        // issued at (Unix seconds)
    pub exp: usize,        // expiry (Unix seconds)
}
```

The refresh token is a random 256-bit hex string stored in a `refresh_tokens` table with an
expiry timestamp. On `POST /api/auth/refresh`, validate the token, check expiry, and issue a new
access token.

### 5.2 Novel Service

The Novel Service orchestrates the most complex async pipeline in the system. The parsing pipeline
runs as a Tokio background task spawned after the file is stored.

**Pipeline orchestration pattern:**

```rust
// In the import handler, after storing the file:
let novel_id = novel.id;
let pool = pool.clone();
let llm = llm_client.clone();
let s3 = s3_client.clone();

tokio::spawn(async move {
    if let Err(e) = run_parse_pipeline(novel_id, pool, llm, s3).await {
        tracing::error!("Parse pipeline failed for {novel_id}: {e}");
        update_novel_status(&pool, novel_id, NovelStatus::Error, Some(e.to_string())).await.ok();
    }
});
```

**Chapter splitting** should first attempt the regex-based heuristics defined in SPEC.md §5.3.
Only fall back to the LLM if fewer than 2 boundaries are found. The LLM fallback sends the first
8000 tokens (approximate: first 32000 bytes) and requests a JSON array of chapter boundaries.

**Character extraction** sends the full novel text in chunks if it exceeds the LLM context window.
For very long novels, send chapter summaries instead of full text. Always use structured output
(`response_format: json_schema`) to ensure parseable responses.

**Avatar generation** runs concurrently for all characters using `tokio::join_all` or a bounded
`FuturesUnordered`. Limit concurrency to 3 simultaneous image generation requests to avoid rate
limiting.

### 5.3 Agent Service

The Agent Service is the most performance-sensitive service. Every conversation turn must complete
the full memory retrieval, prompt construction, and LLM stream proxy within the SSE connection
lifetime.

**SSE handler pattern with Axum:**

```rust
use axum::response::sse::{Event, Sse};
use futures::stream::{self, Stream};
use tokio_stream::StreamExt;

pub async fn chat_stream(
    State(state): State<AppState>,
    Extension(user_id): Extension<Uuid>,
    Path(character_id): Path<Uuid>,
    Json(body): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = async_stream::stream! {
        // 1. Retrieve memories
        let memories = state.memory_manager
            .retrieve_for_prompt(character_id, user_id, &body.message)
            .await
            .unwrap_or_default();

        // 2. Build prompt
        let messages = state.prompt_builder
            .build(character_id, user_id, &memories, &body.message)
            .await;

        // 3. Stream LLM response
        let mut llm_stream = state.llm_client.stream_chat(messages).await;
        let mut full_response = String::new();

        while let Some(chunk) = llm_stream.next().await {
            match chunk {
                Ok(delta) => {
                    full_response.push_str(&delta);
                    yield Ok(Event::default()
                        .event("delta")
                        .data(serde_json::json!({ "content": delta }).to_string()));
                }
                Err(e) => {
                    yield Ok(Event::default()
                        .event("error")
                        .data(serde_json::json!({ "code": "llm_error", "message": e.to_string() }).to_string()));
                    return;
                }
            }
        }

        // 4. Post-stream: store messages, update memory
        state.after_turn(character_id, user_id, &body.message, &full_response).await.ok();

        yield Ok(Event::default().event("done").data("{}"));
    };

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}
```

**Memory retrieval** must query all four layers in parallel using `tokio::join!`:

```rust
let (permanent, long_term, mid_term, short_term) = tokio::join!(
    self.repo.get_permanent(character_id, user_id),
    self.repo.search_long_term(character_id, user_id, &query_embedding, 5),
    self.repo.get_mid_term(character_id, user_id, 3),
    self.redis.get_short_term(character_id, user_id, 10),
);
```

### 5.4 Narrative Service

The Narrative Service manages world state mutations. All writes to `world_states.state` must use
PostgreSQL's `jsonb_set` function or a full-document update within a transaction to prevent
partial writes.

**Atomic world state update:**

```sql
UPDATE world_states
SET state = jsonb_set(
    jsonb_set(state, '{choices}', state->'choices' || $1::jsonb),
    '{updated_at}', to_jsonb(NOW())
),
updated_at = NOW()
WHERE user_id = $2 AND novel_id = $3
```

**Consequence generation** should include the reader's prior choices from `WorldState.state.choices`
in the LLM prompt to ensure narrative continuity. Limit the included history to the last 5 choices
to bound prompt size.

### 5.5 API Gateway

The Gateway must handle SSE passthrough without buffering. In Axum, this requires forwarding the
upstream response body as a stream. The critical constraint is that the Gateway must not set a
`Content-Length` header on SSE responses, as the length is unknown.

The JWT middleware extracts the `Authorization: Bearer <token>` header, validates the token, and
injects the `user_id` and `role` as request extensions:

```rust
pub async fn jwt_middleware(
    State(secret): State<String>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let claims = decode::<Claims>(token, &DecodingKey::from_secret(secret.as_bytes()), &Validation::default())
        .map_err(|_| StatusCode::UNAUTHORIZED)?
        .claims;

    req.extensions_mut().insert(Uuid::parse_str(&claims.sub).map_err(|_| StatusCode::UNAUTHORIZED)?);
    req.extensions_mut().insert(claims.role);
    Ok(next.run(req).await)
}
```

---

## 6. Frontend Implementation Guide

The frontend follows Feature-Sliced Design (FSD). Every coding agent working on the frontend must
understand the layer hierarchy before adding any file.

### 6.1 FSD Layer Rules

| Layer | Can import from | Cannot import from |
|---|---|---|
| `app` | All layers | — |
| `pages` | `widgets`, `features`, `entities`, `shared` | `app` |
| `widgets` | `features`, `entities`, `shared` | `app`, `pages` |
| `features` | `entities`, `shared` | `app`, `pages`, `widgets` |
| `entities` | `shared` | `app`, `pages`, `widgets`, `features` |
| `shared` | — (no internal imports) | All upper layers |

Violations of these import rules will cause circular dependency errors. Use ESLint with
`eslint-plugin-boundaries` to enforce them.

### 6.2 API Client (shared layer)

All API calls go through a single client instance in `shared/api/client.ts`. The client
automatically attaches the JWT from localStorage and handles 401 responses by redirecting to
login.

```typescript
// shared/api/client.ts
const BASE_URL = import.meta.env.VITE_API_URL ?? 'http://localhost:8080';

export async function apiRequest<T>(
  path: string,
  options: RequestInit = {}
): Promise<T> {
  const token = localStorage.getItem('access_token');
  const res = await fetch(`${BASE_URL}${path}`, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...options.headers,
    },
  });
  if (res.status === 401) {
    localStorage.removeItem('access_token');
    window.location.href = '/login';
    throw new Error('Unauthorized');
  }
  if (!res.ok) {
    const err = await res.json().catch(() => ({ error: { message: res.statusText } }));
    throw new Error(err.error?.message ?? 'Request failed');
  }
  return res.json();
}
```

### 6.3 SSE Client (shared layer)

The SSE client for character conversations is implemented as a custom hook in
`shared/api/useCharacterStream.ts`. It uses the `fetch` API with `ReadableStream` rather than
`EventSource` because `EventSource` does not support `POST` requests.

```typescript
// shared/api/useCharacterStream.ts
export function useCharacterStream(characterId: string) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [streaming, setStreaming] = useState(false);

  const sendMessage = useCallback(async (content: string, chapterNum: number) => {
    setStreaming(true);
    const token = localStorage.getItem('access_token');
    const res = await fetch(`/api/chat/${characterId}/stream`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
      body: JSON.stringify({ message: content, chapterNum }),
    });

    const reader = res.body!.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let assistantContent = '';

    // Optimistically add the assistant message placeholder
    setMessages(prev => [...prev, { role: 'character', content: '' }]);

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop() ?? '';

      for (const line of lines) {
        if (line.startsWith('data: ')) {
          const data = JSON.parse(line.slice(6));
          if (data.content) {
            assistantContent += data.content;
            setMessages(prev => {
              const next = [...prev];
              next[next.length - 1] = { role: 'character', content: assistantContent };
              return next;
            });
          }
        }
      }
    }
    setStreaming(false);
  }, [characterId]);

  return { messages, streaming, sendMessage };
}
```

### 6.4 Chat Panel Widget

The `ChatPanel` widget slides in from the right without displacing the reader's scroll position.
It uses a CSS `position: fixed` overlay with a `transform: translateX` transition.

The panel must:
- Preserve the reader's scroll position when opened and closed.
- Show a character avatar, name, and role badge in the header.
- Render streaming text with a blinking cursor during generation.
- Support keyboard shortcut `Escape` to close.
- Maintain its own scroll position independently of the page.

### 6.5 Branch Choice Widget

The `BranchChoice` widget appears as a full-width card between chapter content and the "next
chapter" button. It must:
- Block the reader from advancing to the next chapter until a choice is made.
- Show the `NarrativeNode.description` above the choice buttons.
- Animate the consequence text in after the choice is submitted.
- Persist the choice in local state so it survives page refresh (via TanStack Query cache).

### 6.6 Design Tokens

Apply the cosmic theme by adding these tokens to `app/styles/globals.css`:

```css
@import url('https://fonts.googleapis.com/css2?family=Cinzel:wght@400;600;700&family=Inter:wght@300;400;500;600&family=Noto+Serif+SC:wght@400;500;700&display=swap');

:root {
  --color-void: #03040a;
  --color-cosmos: #080d1f;
  --color-nebula: #0f1535;
  --color-stardust: #1a2040;
  --color-aurora: #6d28d9;
  --color-aurora-light: #8b5cf6;
  --color-nova: #06b6d4;
  --color-nova-glow: #22d3ee;
  --color-starlight: #e2e8f0;
  --color-moonbeam: #94a3b8;
  --color-comet: #475569;
  --font-display: 'Cinzel', serif;
  --font-body: 'Inter', sans-serif;
  --font-reading: 'Noto Serif SC', serif;
  --glow-nova: 0 0 20px rgba(6, 182, 212, 0.4);
  --glow-aurora: 0 0 30px rgba(109, 40, 217, 0.3);
}

body {
  background: linear-gradient(135deg, var(--color-void) 0%, var(--color-cosmos) 100%);
  color: var(--color-starlight);
  font-family: var(--font-body);
  min-height: 100vh;
}
```

---

## 7. Database Schema

The full schema is in `infra/postgres/init.sql`. Below is the logical entity relationship summary.

```
users ─────────────────────────────────────────────────────────┐
  │                                                             │
  ├── novels (user_id FK)                                       │
  │     ├── chapters (novel_id FK)                              │
  │     │     └── narrative_nodes (novel_id FK, chapter_number) │
  │     └── characters (novel_id FK)                            │
  │           └── character_memories (character_id FK, ─────────┤ user_id FK)
  │                                                             │
  ├── reading_progress (user_id FK, novel_id FK)                │
  ├── world_states (user_id FK, novel_id FK)                    │
  ├── user_choices (user_id FK, novel_id FK, node_id FK)        │
  └── chat_messages (user_id FK, character_id FK, novel_id FK)  │
                                                                │
users ◄────────────────────────────────────────────────────────┘
```

### Key Schema Decisions

**UUID v4 everywhere.** All primary keys are `UUID` generated with `uuid_generate_v4()`. This
avoids sequential ID enumeration attacks and simplifies sharding if needed later.

**TIMESTAMPTZ for all timestamps.** Store and retrieve in UTC. The frontend converts to local
time using `new Date(utcString).toLocaleString()`.

**JSONB for flexible structures.** `world_states.state`, `characters.traits`, and
`narrative_nodes.choices` use JSONB to avoid premature schema rigidity.

**pgvector for memory embeddings.** The `character_memories.embedding` column is `vector(1536)`,
matching the output dimension of `text-embedding-3-small`. The HNSW index uses cosine distance
(`vector_cosine_ops`) with `m=16` and `ef_construction=64`.

---

## 8. LLM Integration Patterns

All LLM calls follow the same pattern: build a `messages` array, call the API, parse the response.

### 8.1 Structured Output

For parsing pipeline calls (character extraction, chapter splitting, node detection), always
request structured JSON output:

```rust
// In infrastructure/llm/client.rs
pub async fn structured_call<T: DeserializeOwned>(
    &self,
    messages: Vec<Message>,
    schema: serde_json::Value,
) -> Result<T> {
    let body = serde_json::json!({
        "model": self.model,
        "messages": messages,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "response",
                "strict": true,
                "schema": schema
            }
        }
    });

    let res: serde_json::Value = self.http.post(&self.url)
        .bearer_auth(&self.api_key)
        .json(&body)
        .send().await?
        .json().await?;

    let content = res["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow!("No content in LLM response"))?;

    serde_json::from_str(content).map_err(Into::into)
}
```

### 8.2 Streaming Call

For agent conversation turns, use the streaming API:

```rust
pub async fn stream_chat(
    &self,
    messages: Vec<Message>,
) -> impl Stream<Item = Result<String>> {
    let body = serde_json::json!({
        "model": self.model,
        "messages": messages,
        "stream": true
    });

    let response = self.http.post(&self.url)
        .bearer_auth(&self.api_key)
        .json(&body)
        .send().await
        .expect("LLM request failed");

    let byte_stream = response.bytes_stream();

    async_stream::stream! {
        let mut buf = String::new();
        tokio::pin!(byte_stream);

        while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            for line in buf.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" { return; }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(delta) = v["choices"][0]["delta"]["content"].as_str() {
                            yield Ok(delta.to_string());
                        }
                    }
                }
            }
            // Keep only the last incomplete line in the buffer
            buf = buf.lines().last().unwrap_or("").to_string();
        }
    }
}
```

### 8.3 Retry Policy

All LLM calls must implement a retry policy for transient errors (HTTP 429, 500, 502, 503):

- Maximum 3 retries.
- Exponential backoff: 1s, 2s, 4s.
- On HTTP 429, respect the `Retry-After` header if present.
- After 3 failures, return an `llm_error` to the caller.

---

## 9. Memory Pyramid Implementation

The `MemoryManager` struct in `agent-service/src/domain/services/memory_manager.rs` is the core
of the agent system. It must implement the following interface:

```rust
pub struct MemoryManager {
    pg: Arc<PgPool>,
    redis: Arc<RedisClient>,
    llm: Arc<LlmClient>,
    embedding: Arc<EmbeddingClient>,
    config: MemoryConfig,
}

impl MemoryManager {
    /// Add a raw conversation turn to the short-term layer.
    pub async fn add_short_term(&self, character_id: Uuid, user_id: Uuid, content: &str, importance: u8) -> Result<()>;

    /// Retrieve memories from all layers for prompt construction.
    /// Returns memories ordered for insertion into the prompt (permanent first, then long, mid, short).
    pub async fn retrieve_for_prompt(&self, character_id: Uuid, user_id: Uuid, query: &str) -> Result<MemoryBundle>;

    /// Compress short-term memories into a mid-term summary.
    /// Called automatically when short-term count exceeds config.compress_threshold.
    pub async fn compress(&self, character_id: Uuid, user_id: Uuid) -> Result<()>;

    /// Add a permanent memory (reader name, major choices, critical events).
    pub async fn add_permanent(&self, character_id: Uuid, user_id: Uuid, content: &str, importance: u8) -> Result<()>;

    /// Get the count of short-term memories for a (character, user) pair.
    pub async fn short_term_count(&self, character_id: Uuid, user_id: Uuid) -> Result<usize>;
}
```

**Compression trigger:** After every `add_short_term` call, check `short_term_count`. If it
exceeds `config.compress_threshold` (default: 15), spawn a background task to run `compress`.
Do not await the compression task in the hot path; it runs concurrently.

**Embedding generation:** When promoting a memory to the `long` or `permanent` layer, generate
an embedding by calling `POST /v1/embeddings` with the memory content. Store the resulting
`vector(1536)` in `character_memories.embedding`.

**Semantic search:** Use pgvector's `<=>` operator for cosine distance:

```sql
SELECT id, content, importance, 1 - (embedding <=> $1::vector) AS similarity
FROM character_memories
WHERE character_id = $2 AND user_id = $3 AND layer = 'long'
ORDER BY embedding <=> $1::vector
LIMIT $4;
```

---

## 10. SSE Streaming Contract

The SSE stream from `/api/chat/:characterId/stream` follows this exact event sequence:

```
POST /api/chat/:characterId/stream
Content-Type: application/json
Authorization: Bearer <token>

{ "message": "Hello, who are you?", "chapterNum": 3 }

---

HTTP/1.1 200 OK
Content-Type: text/event-stream
Cache-Control: no-cache
X-Accel-Buffering: no

event: delta
data: {"content":"I"}

event: delta
data: {"content":" am"}

event: delta
data: {"content":" Hermione"}

... (more delta events) ...

event: done
data: {"usage":{"input_tokens":342,"output_tokens":87}}
```

On error at any point:

```
event: error
data: {"code":"llm_error","message":"The upstream LLM service returned a 429 error."}
```

The frontend must handle all three event types. The `done` event signals that the full response
has been received and post-processing (memory write, world state update) has completed.

---

## 11. Testing Strategy

### 11.1 Unit Tests

Each service must have unit tests for its domain layer. Domain logic must be testable without
database or network access by using trait objects for repositories and LLM clients.

**Required unit tests:**

```
user-service:
  - email validation (valid, invalid, edge cases)
  - password hashing and verification
  - JWT issuance and validation
  - JWT expiry handling

novel-service:
  - chapter splitter: Chinese headers, English headers, number headers, LLM fallback
  - character deduplication by name (case-insensitive)
  - world summary truncation at 2000 chars

agent-service:
  - memory pyramid: add short-term, trigger compression at threshold
  - prompt construction: memory ordering (permanent first)
  - prompt truncation when context window exceeded
  - anti-spoiler: memories from future chapters excluded

narrative-service:
  - choice index bounds validation
  - world state JSONB merge correctness
```

### 11.2 Integration Tests

Integration tests run against a real PostgreSQL and Redis instance (use Docker in CI).

**Required integration tests:**

```
auth flow: register → login → access protected endpoint → refresh → logout
novel import: paste text → poll status until ready → list chapters → list characters
agent conversation: send message → receive SSE stream → verify message stored
branch choice: get node → submit choice → verify world state updated
memory compression: add 16 short-term memories → verify compression triggered → verify mid-term created
```

### 11.3 Running Tests

```bash
# Unit tests (no external dependencies)
cargo test --workspace

# Integration tests (requires running infrastructure)
docker compose up -d postgres redis
DATABASE_URL=postgres://novelworld:novelworld@localhost:5432/novelworld \
REDIS_URL=redis://localhost:6379 \
cargo test --workspace -- --include-ignored

# Frontend tests
cd frontend && pnpm test
```

---

## 12. Deployment

### 12.1 Production Docker Compose

```bash
# Copy and configure production environment
cp .env.example .env.prod
# Edit .env.prod with production credentials

# Build production images
docker compose -f docker-compose.yml -f docker-compose.prod.yml build

# Start the stack
docker compose -f docker-compose.yml -f docker-compose.prod.yml up -d

# Verify all services are healthy
curl http://localhost:8080/health
```

### 12.2 Service Port Assignments

| Service | Internal Port | External (dev only) |
|---|---|---|
| Nginx | 80, 443 | 80, 443 |
| API Gateway | 8080 | 8080 |
| User Service | 8081 | — |
| Novel Service | 8082 | — |
| Agent Service | 8083 | — |
| Narrative Service | 8084 | — |
| PostgreSQL | 5432 | 5432 |
| Redis | 6379 | 6379 |
| MinIO API | 9000 | 9000 |
| MinIO Console | 9001 | 9001 |

In production, only Nginx (80/443) and the MinIO console (9001, if needed) should be exposed.
All other ports are internal to the Docker network.

### 12.3 Database Backup

```bash
# Backup
docker compose exec postgres pg_dump -U novelworld novelworld | gzip > backup_$(date +%Y%m%d).sql.gz

# Restore
gunzip -c backup_20240101.sql.gz | docker compose exec -T postgres psql -U novelworld novelworld
```

---

## 13. Environment Variables Reference

Copy `.env.example` to `.env` and fill in all required values before starting any service.

```bash
# ─── Database ────────────────────────────────────────────────────────────────
DATABASE_URL=postgres://novelworld:novelworld@postgres:5432/novelworld

# ─── Redis ───────────────────────────────────────────────────────────────────
REDIS_URL=redis://redis:6379

# ─── Authentication ──────────────────────────────────────────────────────────
JWT_SECRET=<min-32-char-random-string>           # REQUIRED
AUTH_ACCESS_TOKEN_EXPIRY=3600                    # seconds (default: 1 hour)
AUTH_REFRESH_TOKEN_EXPIRY=604800                 # seconds (default: 7 days)

# ─── LLM ─────────────────────────────────────────────────────────────────────
LLM_API_URL=https://api.openai.com              # REQUIRED
LLM_API_KEY=sk-...                              # REQUIRED
LLM_MODEL=gpt-4o-mini                           # REQUIRED

# ─── Embeddings ──────────────────────────────────────────────────────────────
EMBEDDING_API_URL=https://api.openai.com        # REQUIRED
EMBEDDING_API_KEY=sk-...                        # REQUIRED (can be same as LLM_API_KEY)
EMBEDDING_MODEL=text-embedding-3-small          # REQUIRED

# ─── Image Generation ────────────────────────────────────────────────────────
IMAGE_GEN_API_URL=https://api.openai.com        # REQUIRED
IMAGE_GEN_API_KEY=sk-...                        # REQUIRED
IMAGE_GEN_MODEL=dall-e-3                        # default: dall-e-3

# ─── Object Storage (S3-compatible) ──────────────────────────────────────────
S3_ENDPOINT=http://minio:9000                   # REQUIRED
S3_BUCKET=novelworld                            # REQUIRED
S3_ACCESS_KEY=minioadmin                        # REQUIRED
S3_SECRET_KEY=minioadmin                        # REQUIRED
S3_REGION=us-east-1                             # default: us-east-1

# ─── Memory Tuning ───────────────────────────────────────────────────────────
MEMORY_SHORT_TERM_LIMIT=20
MEMORY_COMPRESS_THRESHOLD=15
MEMORY_MID_TERM_LIMIT=50
MEMORY_LONG_TERM_K=5
AGENT_CONTEXT_WINDOW_TURNS=20

# ─── Upload Limits ───────────────────────────────────────────────────────────
UPLOAD_MAX_TXT_BYTES=10485760                   # 10 MB
UPLOAD_MAX_PDF_BYTES=20971520                   # 20 MB
UPLOAD_MAX_PASTE_BYTES=5242880                  # 5 MB
PARSE_MAX_CHARACTERS=50

# ─── Service Ports ───────────────────────────────────────────────────────────
GATEWAY_PORT=8080
USER_SERVICE_PORT=8081
NOVEL_SERVICE_PORT=8082
AGENT_SERVICE_PORT=8083
NARRATIVE_SERVICE_PORT=8084

# ─── Frontend ────────────────────────────────────────────────────────────────
VITE_API_URL=http://localhost:8080
```

---

## 14. Implementation Checklist

Use this checklist to track progress. A coding agent should complete items in order.

### Phase 1: Foundation

- [ ] PostgreSQL schema applied (`infra/postgres/init.sql`)
- [ ] pgvector, uuid-ossp, pg_trgm extensions installed
- [ ] All tables created with correct indexes
- [ ] Cargo workspace compiles without errors

### Phase 2: User Service

- [ ] `POST /api/auth/register` — validates email, hashes password, issues tokens
- [ ] `POST /api/auth/login` — verifies password, issues tokens
- [ ] `POST /api/auth/refresh` — validates refresh token, issues new access token
- [ ] `GET /api/auth/me` — returns current user from JWT
- [ ] `POST /api/auth/logout` — invalidates refresh token
- [ ] Unit tests: email validation, JWT issuance, bcrypt verification

### Phase 3: Gateway

- [ ] JWT middleware extracts and validates Bearer tokens
- [ ] Routes all `/api/*` paths to correct downstream services
- [ ] SSE responses proxied without buffering
- [ ] `GET /health` aggregates downstream health

### Phase 4: Novel Service

- [ ] `POST /api/novels/upload` — stores file in S3, creates Novel record, enqueues pipeline
- [ ] `POST /api/novels` — accepts text paste, creates Novel record, enqueues pipeline
- [ ] Parsing pipeline: chapter splitting (regex + LLM fallback)
- [ ] Parsing pipeline: character extraction with structured output
- [ ] Parsing pipeline: world summary generation
- [ ] Parsing pipeline: narrative node detection
- [ ] `GET /api/novels/:id/status` — returns current parse status
- [ ] `GET /api/novels/:id/chapters` — returns chapter list
- [ ] `GET /api/novels/:id/chapters/:num` — returns full chapter content
- [ ] `GET /api/novels/:id/characters` — returns character list
- [ ] Unit tests: chapter splitter, character deduplication

### Phase 5: Agent Service (Core)

- [ ] `MemoryManager.add_short_term` — stores in Redis and PostgreSQL
- [ ] `MemoryManager.retrieve_for_prompt` — retrieves from all four layers in parallel
- [ ] `PromptBuilder.build` — assembles system prompt with character identity and memories
- [ ] `POST /api/chat/:characterId/stream` — streams LLM response via SSE
- [ ] Post-stream: stores ChatMessage records
- [ ] Post-stream: creates short-term memory entry
- [ ] `GET /api/chat/:characterId/history` — returns conversation history
- [ ] Unit tests: memory ordering, prompt truncation, anti-spoiler constraint

### Phase 6: Agent Service (Memory Compression)

- [ ] `MemoryManager.compress` — summarizes short-term into mid-term via LLM
- [ ] Compression auto-triggered when short-term count exceeds threshold
- [ ] `MemoryManager.add_permanent` — stores with embedding
- [ ] Long-term semantic search via pgvector cosine similarity
- [ ] Unit tests: compression trigger, embedding storage, semantic retrieval

### Phase 7: Narrative Service

- [ ] `GET /api/narrative/:novelId/:chapter` — returns branch node or null
- [ ] `POST /api/narrative/choose` — stores choice, generates consequence, updates world state
- [ ] `GET /api/narrative/:novelId/world-state` — returns reader's world state
- [ ] World state JSONB updates are atomic
- [ ] Unit tests: choice bounds validation, world state merge

### Phase 8: Progress and Identity

- [ ] `GET /api/progress/:novelId` — returns reading progress
- [ ] `PUT /api/progress/:novelId` — updates current chapter
- [ ] `PUT /api/progress/:novelId/identity` — sets reader identity and mode
- [ ] Identity constraints enforced (cannot adopt current conversation partner)

### Phase 9: Avatar Generation

- [ ] `POST /api/characters/:id/generate-avatar` — triggers avatar generation
- [ ] Avatar generation runs concurrently (max 3 parallel) during parsing pipeline
- [ ] Avatar stored in S3, URL saved to character record
- [ ] Avatar failure does not block novel from reaching `ready` status

### Phase 10: Frontend

- [ ] FSD directory structure created
- [ ] Design tokens applied in `globals.css`
- [ ] `HomePage` — landing page with login/register CTA
- [ ] `ShelfPage` — novel library with import button
- [ ] `ImportPage` — file upload and text paste with progress indicator
- [ ] `ReaderPage` — chapter content with `ChatPanel` and `BranchChoice`
- [ ] `CharactersPage` — character gallery
- [ ] `ChatPanel` widget — SSE streaming, slide-in, keyboard shortcut
- [ ] `BranchChoice` widget — blocks chapter advancement, animates consequence
- [ ] `CharacterCard` widget — avatar, name, role badge, talk button
- [ ] SSE client with reconnect and exponential backoff
- [ ] JWT stored in localStorage, attached to all API requests
- [ ] 401 responses redirect to login

### Phase 11: Testing and Hardening

- [ ] All unit tests passing (`cargo test --workspace`)
- [ ] Integration tests passing against Docker infrastructure
- [ ] Frontend tests passing (`pnpm test`)
- [ ] LLM retry policy implemented (3 retries, exponential backoff)
- [ ] File upload MIME type and size validation
- [ ] SQL injection prevention (parameterized queries throughout)
- [ ] Prompt injection mitigation (user input sanitized before LLM)
- [ ] `GET /health` returns correct status for all services

---

## Contributing

When adding a new feature, follow this sequence:

1. Update `SPEC.md` with normative requirements for the feature.
2. Update the database schema in `infra/postgres/init.sql` and create a migration file.
3. Implement the domain layer (entities, value objects, repository traits) first.
4. Implement the infrastructure layer (PostgreSQL, Redis, S3 implementations).
5. Implement the application layer (command handlers).
6. Implement the interface layer (HTTP routes).
7. Write unit and integration tests.
8. Update this README's checklist.

---

## License

MIT
