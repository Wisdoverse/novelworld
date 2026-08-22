<div align="center">

# 📖 NovelWorld

### Turn a supported novel into a world you can step into

**Upload a novel → AI extracts characters → Chat with them in real time → Reshape the story with your choices**

[Quick Start](#-quick-start) · [Platform Support](#platform-support) · [Features](#-core-features) · [Architecture](#️-architecture) · [Docs](#-documentation)

</div>

> **For coding agents:** Start with [AGENTS.md](./AGENTS.md) (also symlinked as `CLAUDE.md`) for
> runtime contract, repo map, naming rules, and code style. [SPEC.md](./SPEC.md) is the candidate
> normative target; runtime code and tests remain conformance evidence.

> **Current status:** NovelWorld is an operator-controlled private self-hosted
> preview, not a public hosted service. Accepted inputs and landed controls are
> not universal language, model, quality, scale, or accessibility guarantees.
> See the [product contract](./docs/PRODUCT_CONTRACT.md).

---

## 🤔 What is this?

Imagine you just finished *The Three-Body Problem* and want to ask Ye Wenjie about her decision. Or you're reading *Harry Potter* and want to hear Snape explain himself.

**NovelWorld makes that possible.**

Inside the current preview envelope, upload or paste a novel and NovelWorld can:

1. 🔍 **Analyze the book** — detect chapter structure, extract characters, understand world lore
2. 🎭 **Create AI characters** — extracted characters with source-bounded conversation context
3. 🖼️ **Generate portraits** — optional provider-hosted avatars for a bounded character set
4. 🗺️ **Build a relationship graph** — map connections between characters

Then you can:

- 💬 **Talk to extracted characters** — committed conversations resume across sessions
- 🔀 **Make story choices** — your decisions change the narrative direction
- 🧭 **Enter a living world** — choose an unlocked checkpoint, act as your own player, and inspect canon versus player-created history
- 🎭 **Use the primary self mode** — enter as an original player; character identity remains experimental
- 🛡️ **Bound source context** — server-owned reading progress excludes future lore from prompts

---

## ✨ Core Features

### 📚 One-Click Import

Paste up to 5 MiB of text; upload UTF-8, BOM-marked UTF-16, or GBK TXT up to
10 MiB; or upload EPUB or text-extractable PDF up to 20 MiB. Simplified Chinese
and English have deterministic structural coverage; generated narrative
transitions require Chinese text, and the UI locale is Simplified Chinese
(`lang=zh-CN`); no language/model pair is release-qualified.

### 🧠 Durable Conversations

PostgreSQL keeps committed chat history and the current runtime creates
mid-term summaries. The four-layer memory model remains the target contract;
production writers for long-term and permanent memories are an H3 gap.

### 🔀 Branching Narrative

At key story moments, you're presented with 2–3 choices. Each decision:
- Generates new story developments
- Shifts character attitudes toward you
- Mutates the world state

### 🎭 Reader Identity

The primary mode creates an original `PlayerEntity` so canonical characters
retain their own agency. A legacy character-identity path remains available for
compatibility: it supports in-character conversation and branch choices only,
with no open-world agency (no `PlayerEntity`, world turns, or journal) — the
boundary is defined in SPEC §8.2.

### 🧭 Living Open World

Create an original player at any unlocked chapter, then travel, investigate,
converse, ally, oppose, resolve a thread, or pursue your own goal. Canonical
characters retain their own agency. Every turn is validated, committed once,
auditable, and resumed exactly after a service restart in the structurally
tested single-timeline path; live causal quality is not yet qualified.

---

## 🚀 Quick Start

NovelWorld has two deliberately separate runtime modes.

### 1. Server deployment — Docker Compose

Use this mode for a self-hosted server. On Windows 10/11, install and start
[Docker Desktop](https://docs.docker.com/desktop/setup/install/windows-install/),
then double-click `start.cmd` or run:

```bat
start.cmd
```

On Linux, install Docker Engine and Docker Compose v2, then run:

```bash
git clone https://github.com/schorsch888/novelworld.git
cd novelworld
./start.sh
```

Keep the default preview on localhost. Remote access requires an
operator-managed encrypted tunnel or TLS boundary; the current stack is not
qualified for direct public-Internet hosting.

The server startup scripts check Docker, generate secrets, start all services,
open `http://localhost`, and guide the operator through model and first-admin
setup.

### 2. Portable desktop — no Docker or external NovelWorld server

The experimental desktop build packages the React/FSD interface, all five Rust
services, and a local PostgreSQL 18 + pgvector runtime. Redis is not
shipped because it is only a reconstructable cache; the desktop adapter reads
authoritative conversation state from PostgreSQL.

Trigger the **Desktop Portable Builds** GitHub Actions workflow, then download
and extract the artifact and its contained platform archive:

| Platform | Portable artifact | Launch |
|----------|-------------------|--------|
| Windows 10/11 x64 | `novelworld-windows-x64-portable` | Double-click `NovelWorld.exe` |
| Linux x64 | `novelworld-linux-x64-appimage` | Extract the `.tar.gz`, then double-click the AppImage |
| macOS Apple Silicon | `novelworld-macos-arm64-app` | Double-click `NovelWorld.app` |
| macOS Intel | `novelworld-macos-x64-app` | Double-click `NovelWorld.app` |

The player does not install Docker, PostgreSQL, Redis, Node.js, or Rust, and the
app never connects to an external NovelWorld server. All application services,
data, and generated secrets stay on the player's computer in the operating
system's per-user application data directory. AI features still require
Internet access to the configured model provider and an API key on first launch.

The current artifacts are unsigned engineering builds. Windows SmartScreen and
macOS Gatekeeper may warn on first launch; public distribution requires platform
code-signing certificates and macOS notarization.

### Platform support

| Mode | Windows | Linux | macOS |
|------|---------|-------|-------|
| Docker server | `start.cmd` | `./start.sh` | Not qualified |
| Portable desktop | x64 engineering build | x64 AppImage engineering build | Apple Silicon and Intel engineering builds |

No default application account is installed. A key entered in the setup page is
sent only to your server, encrypted before PostgreSQL persistence, and never
written to browser storage. Advanced operators can still use `LLM_API_*` in
`.env`; environment configuration takes precedence over the web setup.

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
# Set the generated server secrets. LLM_API_KEY is optional when using web setup.

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
| Cache | Redis | Reconstructable recent-message projection |
| AI | Operator-configured provider | Structured output + streaming, bounded retry |
| Frontend | React + TypeScript | Tailwind CSS, Feature-Sliced Design |
| Server deploy | Docker Compose | 9 long-running containers plus a migration job |
| Desktop deploy | Tauri portable bundle | Same five services on loopback + bundled pg0; no Docker |

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
│   └── src-tauri/           # Portable desktop shell and local runtime supervisor
├── infra/                   # Database schema, Nginx config
└── docker-compose.yml       # Full stack orchestration
```

---

## 📖 Documentation

| Document | Description |
|----------|-------------|
| [Documentation index](./docs/README.md) | Source-of-truth map, complete catalog, and maintenance standard |
| [SPEC.md](./SPEC.md) | Candidate normative specification (RFC 2119) |
| [SPEC_CONFORMANCE.md](./docs/SPEC_CONFORMANCE.md) | Clause dispositions, owners, and evidence boundaries |
| [PRODUCT_CONTRACT.md](./docs/PRODUCT_CONTRACT.md) | Current supported envelope, responsibility boundary, and claim ledger |
| [AGENTS.md](./AGENTS.md) | Instructions for AI coding assistants |
| [DEPLOY.md](./DEPLOY.md) | Deployment guide |
| [ARCHITECTURE.md](./docs/ARCHITECTURE.md) | Architecture decisions |
| [ROADMAP.md](./docs/ROADMAP.md) | Evidence-gated engineering horizons |
| [QUALIFICATION_POLICY.md](./docs/QUALIFICATION_POLICY.md) | Versioned journey, slice, guardrail, and threshold-approval rules |
| [SLOS.md](./docs/SLOS.md) | Versioned single-node capacity and scaling decision contract |
| [DATA_RETENTION.md](./docs/DATA_RETENTION.md) | Data retention, erasure, and provider boundaries |
| [ACCOUNT_EXPORT.md](./docs/ACCOUNT_EXPORT.md) | Versioned account export wire contract and completeness rules |

---

## 🧪 Testing

Use the commands and affected-gate matrix in
[CONTRIBUTING.md](./CONTRIBUTING.md#verification).

---

## 📄 License

[MIT](LICENSE)
