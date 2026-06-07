<div align="center">

# 📖 NovelWorld

### Turn any novel into a world you can step into

**Upload a novel → AI extracts characters → Chat with them in real time → Reshape the story with your choices**

[Quick Start](#-quick-start) · [Features](#-core-features) · [Architecture](#️-architecture) · [Docs](#-documentation)

</div>

> **For coding agents:** Start with [AGENTS.md](./AGENTS.md) (also symlinked as `CLAUDE.md`) for
> runtime contract, repo map, naming rules, and code style. The full implementation guide is in
> [IMPLEMENTATION.md](./IMPLEMENTATION.md). The normative spec is [SPEC.md](./SPEC.md).

---

## 🤔 What is this?

Imagine you just finished *The Three-Body Problem* and want to ask Ye Wenjie about her decision. Or you're reading *Harry Potter* and want to hear Snape explain himself.

**NovelWorld makes that possible.**

Upload any novel's text and NovelWorld will automatically:

1. 🔍 **Analyze the book** — detect chapter structure, extract characters, understand world lore
2. 🎭 **Create AI characters** — each with their own personality, speaking style, and memory
3. 🖼️ **Generate portraits** — AI-generated avatars based on appearance descriptions
4. 🗺️ **Build a relationship graph** — map connections between characters

Then you can:

- 💬 **Talk to any character** — they respond in their authentic voice and remember your conversations
- 🔀 **Make story choices** — your decisions change the narrative direction
- 🎭 **Assume a character's identity** — enter the world as someone from the book
- 🛡️ **No spoilers** — characters only know events up to your current chapter

---

## ✨ Core Features

### 📚 One-Click Import

Paste text or upload a file. AI handles all parsing automatically. Supports novels in any language with automatic chapter detection.

### 🧠 Characters That Remember

NovelWorld uses a **4-Layer Memory Pyramid** so every character remembers your interactions:

```
┌──────────────────────┐
│   Permanent Memory   │ ← Your name, major choices, critical events
├──────────────────────┤
│   Long-term Memory   │ ← Semantic search over past conversations
├──────────────────────┤
│   Mid-term Memory    │ ← Auto-summarized every 20 conversation turns
├──────────────────────┤
│   Short-term Memory  │ ← Recent conversation context
└──────────────────────┘
```

### 🔀 Branching Narrative

At key story moments, you're presented with 2–3 choices. Each decision:
- Generates new story developments
- Shifts character attitudes toward you
- Mutates the world state

### 🎭 Reader Identity

Choose how to enter the world:
- **As yourself** — interact with characters as an outsider
- **As a character** — assume any character's identity for a different perspective

---

## 🚀 Quick Start

### Single Docker command (simplest)

```bash
docker run -d -p 80:80 -e LLM_API_KEY=sk-your-key ghcr.io/schorsch888/novelworld
# Open http://localhost → setup wizard guides you through the rest
```

Everything included: PostgreSQL, Redis, all 5 services, Nginx, frontend. One container.

### Docker Compose (recommended for production)

```bash
git clone https://github.com/schorsch888/novelworld.git
cd novelworld
./start.sh
```

The interactive setup wizard will:
1. Check Docker is installed
2. Generate secure passwords automatically
3. Ask which LLM provider to use (OpenAI / DeepSeek / Qwen / GLM / Anthropic / Moonshot / Doubao / custom)
4. Start all services
5. Open http://localhost in your browser

That's it. Just have your API key ready.

### Development mode

<details>
<summary>Click to expand</summary>

**Prerequisites:**
- [Rust](https://rustup.rs/) ≥ 1.78
- [Docker](https://docs.docker.com/get-docker/)
- [Node.js](https://nodejs.org/) 22+ & [pnpm](https://pnpm.io/)
- OpenAI-compatible API key

```bash
# 1. Configure
cp .env.example .env
# Edit .env — set LLM_API_KEY and JWT_SECRET at minimum

# 2. Start databases
docker compose up -d postgres redis

# 3. Start backend (5 services)
cargo build --workspace
cargo run -p gateway &
cargo run -p user-service &
cargo run -p novel-service &
cargo run -p agent-service &
cargo run -p narrative-service &

# 4. Start frontend
cd frontend && pnpm install && pnpm dev
```

Open `http://localhost:5173` to get started.

</details>

### User Flow

```
Sign up → Upload novel → Wait for parsing → Start reading
                                               ↓
                                   Click character avatar → Chat
                                               ↓
                                   Hit a branch point → Choose → See consequences
```

---

## 🏗️ Architecture

```
┌─────────────────────────────────────────────┐
│                 Nginx (:80)                  │
└────────────────────┬────────────────────────┘
                     │
┌────────────────────▼────────────────────────┐
│            API Gateway (:8080)               │
│        JWT auth · routing · SSE proxy        │
└──┬──────────┬──────────┬──────────┬─────────┘
   │          │          │          │
┌──▼───┐  ┌──▼───┐  ┌───▼──┐  ┌───▼────────┐
│ User │  │Novel │  │Agent │  │ Narrative  │
│ :8001│  │:8002 │  │:8003 │  │   :8004    │
└──┬───┘  └──┬───┘  └───┬──┘  └───┬────────┘
   │         │          │         │
┌──▼─────────▼──────────▼─────────▼──────────┐
│       PostgreSQL 18 + pgvector + Redis      │
└─────────────────────────────────────────────┘
```

| Layer | Stack | Details |
|-------|-------|---------|
| Backend | Rust / Axum | 5 async microservices |
| Database | PostgreSQL 18 | pgvector semantic search, pg_trgm fuzzy matching |
| Cache | Redis 7 | Short-term memory store |
| AI | OpenAI-compatible API | Structured output + streaming, 3x exponential backoff retry |
| Frontend | React + TypeScript | Tailwind CSS, Feature-Sliced Design |
| Deploy | Docker Compose | Full stack in 7 containers |

---

## 📁 Project Structure

```
novelworld/
├── gateway/                 # API gateway (auth, routing, SSE passthrough)
├── services/
│   ├── user-service/        # Authentication (register, login, JWT)
│   ├── novel-service/       # Novel ingestion (chapter splitting, character extraction, avatars)
│   ├── agent-service/       # Character AI (memory pyramid, streaming chat)
│   └── narrative-service/   # Narrative engine (branches, choices, world state)
├── frontend/                # React app
├── infra/                   # Database schema, Nginx config
└── docker-compose.yml       # Full stack orchestration
```

---

## 📖 Documentation

| Document | Description |
|----------|-------------|
| [SPEC.md](./SPEC.md) | Full technical specification (RFC 2119) |
| [IMPLEMENTATION.md](./IMPLEMENTATION.md) | Detailed implementation guide |
| [AGENTS.md](./AGENTS.md) | Instructions for AI coding assistants |
| [DEPLOY.md](./DEPLOY.md) | Deployment guide |
| [ARCHITECTURE.md](./docs/ARCHITECTURE.md) | Architecture decisions |

---

## 🧪 Testing

```bash
cargo test --workspace    # 25 unit tests across all services
```

---

## 📄 License

[MIT](LICENSE)
