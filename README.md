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
- 🛡️ **Bound source context** — server-owned reading progress bounds lore and committed memory; whole-novel persona extraction remains a recorded limitation

---

## ✨ Core Features

### 📚 One-Click Import

Paste up to 5 MiB of text; upload UTF-8, BOM-marked UTF-16, or GBK TXT up to
10 MiB; or upload EPUB or text-extractable PDF up to 20 MiB. Simplified Chinese
and English have deterministic structural coverage; generated narrative
transitions require Chinese text, and the UI locale is Simplified Chinese
(`lang=zh-CN`); no language/model pair is release-qualified.

### 🧠 Durable Conversations

PostgreSQL keeps committed chat history. Mid-term summaries can promote into
embedded long-term memory. A committed open-world turn also carries a durable
`pending` memory-projection state until it reaches terminal `saved` or
`skipped`: the same logical-turn key compensates a pending projection, while a
terminal replay returns the committed result without depending on the Agent
Service. After projection returns `saved`/`skipped`, Narrative rechecks the
committed source high-water before terminalizing the journal; a concurrent
rewind returns a content-free typed conflict and leaves the row `pending` for
same-key compensation after progress is restored. Eligible permanent journey
facts use a private namespaced UUID v5 and store only the observable kind and
target of `converse`, `ally`, or `oppose` directed at the protagonist—never the
reader's free-form private intent—plus events and relationship changes that explicitly name
that protagonist, and are atomically stored without waiting on an embedding
provider. Optional permanent-memory semantic enrichment is not implemented in
this candidate. The Agent authenticates a structured fact from its UUID v5 and
complete local scope/shape metadata before inserting it or returning
`saved: true`; malformed ingress returns `422` without a repository write.
The same gate authenticates retrieval, and reserves permanent
prompt budget for the newest authenticated fact before legacy prose. Unrelated knowledge, inventory,
location, thread, faction, canon-event, and generated-prose changes are not
promoted into character memory. A missing protagonist or a turn the
protagonist did not explicitly witness is recorded as `skipped`, not invented
as character knowledge. At first adoption, migration marks pre-contract
completed turns terminal `skipped` because their witness provenance cannot be
proved, but retains their old rows for export and normal lifecycle deletion.
Agent prompt consumers quarantine the former producer class—permanent,
importance 7, UUID version nibble 4—instead of guessing a historical
protagonist and deleting data. This may conservatively hide a legitimate legacy
memory in that narrow class; no historical fact is fabricated. Under the new contract,
a different logical key cannot advance the same reader world while an earlier
committed turn is still `pending`; only exact-key compensation can close it.
There is no autonomous scan for abandoned `pending` rows; live
semantic/lifecycle quality and the complete H3 exit evidence remain open.

### 🔀 Branching Narrative

At key story moments, you're presented with 2–3 choices. Each committed
decision generates new story developments and may shift character attitudes or
mutate the world state. Creating an original player seals an upper chapter
boundary for that branch prefix: a later branch choice is rejected, and once
the open world starts every new branch choice is rejected. An exact replay of
an already committed choice remains valid. New choice consequences commit only
if the locked world state still matches the snapshot used for generation and
their chapter advances the committed choice prefix. A choice rewrite replaces
an older same-chapter continuation projection atomically; conflicts leave no
partial choice, chapter, or world-state update.

### 🎭 Reader Identity

The primary mode creates an original `PlayerEntity` so canonical characters
retain their own agency. A legacy character-identity path remains available for
compatibility: it supports in-character conversation and exact replay/read of
an already committed branch result only. New nodes/choices and every
Player/open-world endpoint remain self-only even if a prior `PlayerEntity` is
retained. Character-mode WorldState reads expose choices only, and internal
character-world context is omitted based on the chat turn's persisted identity
snapshot, so a concurrent switch to self cannot re-enable the lookup.
Authenticated self-mode journey facts are
also excluded from both direct and semantic memory blocks in character mode;
until memory rows carry durable identity provenance, character mode also omits
mid-, long-, permanent-, and semantic-memory blocks and does not create derived
summaries. Its conversation continuity is limited to recent committed chat
whose persisted turn claim names the exact same character identity; legacy or
unprovenanced chat remains available only in self mode. SPEC §8.2 defines the
boundary.

### 🧭 Living Open World

Create an original player at an unlocked chapter compatible with the already
committed branch prefix, then travel, investigate, converse, ally, oppose,
advance an open thread (which may resolve when the narrated facts justify it),
or pursue your own goal. Canonical characters retain their own agency. Every
turn is validated, committed once, auditable, and resumed exactly after a
service restart in the structurally tested single-timeline path. New turns
receive a bounded tail of committed actions and narrative projections. The
first start commits its world-entry context as the winner; a later valid
start request resumes that session instead of rewriting it from fresh canon.
Each new action revalidates that the complete committed choice/player-event
prefix still fits the sealed checkpoint, so inconsistent legacy data conflicts
before a provider call or world commit instead of lowering fact provenance.
The same transaction also requires every durable branch choice and its
node-keyed WorldState projection to match one-for-one; missing, duplicate,
unkeyed, or malformed legacy projections fail closed instead of being guessed
or late-applied after the world is sealed.
Once the open world has begun, character chat receives only the latest four
character-directed (`converse`, `ally`, or `oppose`) actions selected from a
bounded 100-turn scan, plus player-origin events whose actor list names the
character and that character's numeric relationship score. Relationship-change
prose, perception, reasons, and branch choices are omitted because they do not
yet carry independent per-character witness provenance. The provider
view is an allowlist: route/player UUIDs, technical metadata, player name and
location, branch choices, and unscoped active threads are excluded. Narrative and
Agent boundaries independently reject an action whose kind/target does not
address the requested character, and Agent also rejects a player event that
does not name it. Derived world context carries the highest canonical
source chapter that could have influenced it and is omitted when that boundary
is missing or later than current server-owned progress, including after a
rewind. While progress is behind, player/world reads, new actions, and exact
replays return `reading_progress_behind_world` without derived content or a
provider call. Effective-chapter reads instead return immutable canon with
`generated=false`; the browser synchronously replaces cached player prose with
canon and refetches after progress changes. Restoring progress makes the exact
player timeline available again. Permanent journey facts use the same conservative chapter boundary.
The journey timeline labels reader decisions separately from generated prose.

Before sending a world action, the browser stores the validated action and its
logical-turn key per user and novel in `sessionStorage`. An ambiguous result
locks new actions and a same-tab reload restores the exact request for confirmation; a
matching journal entry clears it only after memory projection is terminal
`saved`/`skipped`, while an explicit rejection of the original POST also clears
it. A terminal POST response carries that projection status directly; journal
confirmation remains an older-response compatibility path. Failure of the subsequent confirmation refresh, including a 4xx response,
remains ambiguous and keeps the action/key locked. The key
is scoped by user and novel. This recovery is limited to the current browser
session and requires writable `sessionStorage`; when the browser blocks that
storage, only the current mounted form remains locked and remount recovery is
not promised. Successful first-run setup, login, registration, or session confirmation removes
other principals' NovelWorld pending-turn records but retains the confirmed
principal's exact recovery records. The same principal boundary cancels and
clears the in-memory server-query cache before the new identity is exposed;
late mutation success can only invalidate/refetch active current-user queries,
not write a previous user's private response into cache. Logout, successful
account deletion, missing credentials, or a confirmed `401`/`403` clears both
private query cache and all such recovery records; transient authentication
failure and failed deletion retain them. Unrelated session data is preserved.
That clearing is credential-fenced: a delayed response from an older bearer
cannot clear or replace a newer login. Delayed account deletion and export are
also fenced by both initiating token and user; stale export bytes are dropped
before inspection or download. An `auth_token` change from another tab clears
this tab's query/chat state and reloads it for authoritative re-authentication
before private UI can continue.
It is not autonomous server reconciliation.
Successful removal of one novel from the current shelf clears only that
user-and-novel pending record; a failed removal retains it, and records for
other users or novels are untouched.
Pre-open-world branch-to-chat continuity, exact chat/world revision provenance,
visibility beyond explicit IDs, continuous-window selection under late memory compensation, live
provider/lifecycle evidence, and human accessibility qualification remain open
H3/H4 work.

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

Application-semantic migrations 0021 and 0024 use a maintenance window: the
managed release path stops the old Narrative producer before exposing candidate
client assets, verifies that world actions fail with a retryable `5xx` while
preserving their recovery key, then stops old Novel and Agent processes before
migrating. Zero-downtime world actions are not claimed.
An installation running older release tooling must first activate a
control-only release containing the new release script but neither barrier,
then use that script for the migration release. New-script adoption requires a
target containing both barriers; upgrade, marked restore, and rollback refuse a
schema downgrade across either one. A durable schema-transition manifest is written
only immediately before the migrator runs and is cleared only after release
state promotion. Normal restore and healthy rollback discard any unmarked
candidate before writing that exact marker, then use the same finalization
barrier as adoption and upgrade. The marker is flushed before migration;
promoted state is flushed before marker deletion, and deletion is flushed
again. On upgrade or rollback, the former `current` is renamed to `previous`
and flushed before the new `current` is installed; on every promotion, the
target tempfile is flushed before it is renamed to `current`. Thus neither the
target content nor rollback authority can lag a durable active-manifest
directory entry. A legacy `rollback.pending` pair is still recovered for
compatibility, but new rollback operations use the schema-transition protocol.
Merely
downloading a candidate cannot block restoration of the current release. Any
marked transition rolls the exact marked manifest forward; a 0021/0024
transition therefore never revives the older writer or reader. Recovery is idempotent even when the
downloaded candidate file is gone, and promotes it only after health succeeds.
An interrupted upgrade preserves the former
`current` as `previous`; initial adoption creates no fictitious predecessor.
The drill covers ordering and process-crash recovery with fake dependencies;
real Linux filesystem power-loss injection and live registry/image health are
still release evidence gaps.

On the first interactive launch, the script guides the required L0 PostgreSQL
user and database name, generates the database password, writes the completion
marker last, and automatically restarts itself before Docker or any business
service starts. Valid preseeded or existing `.env` files migrate without a
prompt; an unconfigured non-interactive launch fails with preseed instructions.

After L0, the server startup scripts check Docker, generate the JWT/runtime
encryption/internal-service L1 roots, run `docker compose down` without
`--volumes`, then build and start the selected profile with `--wait`. This preserves
named-volume data while stopping old writers and removing the old one-shot
migration container so every local pull/restart reapplies migrations before
services start. A fresh install persists `CACHE_MODE=postgres`, starts no Redis,
and opens `http://localhost` only after readiness. The operator creates the first
administrator immediately and may configure the model later in Settings. Redis
projection is an explicit opt-in: persist `CACHE_MODE=redis` plus a non-placeholder
`REDIS_PASSWORD`, then rerun the launcher so it derives the profile and URL together.

### 2. Portable desktop — no Docker or external NovelWorld server

The experimental desktop build packages the React/FSD interface, all five Rust
services, and a local PostgreSQL 18 + pgvector runtime. Redis is not
shipped because it is only a reconstructable cache; the desktop adapter reads
authoritative conversation state from PostgreSQL.

Official `v*` tags publish the server manifest, SBOMs, checksums, and every
desktop archive together on [GitHub Releases](https://github.com/Wisdoverse/novelworld/releases).
The workflow can also be run manually to produce temporary Actions artifacts.

| Platform | Portable artifact | Launch |
|----------|-------------------|--------|
| Windows 10/11 x64 | `novelworld-windows-x64-portable.zip` | Extract, then double-click `NovelWorld.exe` |
| Linux x64 | `novelworld-linux-x64-appimage.tar.gz` | Extract, then double-click the AppImage |
| macOS Apple Silicon | `novelworld-macos-arm64-app.zip` | Extract, then double-click `NovelWorld.app` |
| macOS Intel | `novelworld-macos-x64-app.zip` | Extract, then double-click `NovelWorld.app` |

The player does not install Docker, PostgreSQL, Redis, Node.js, or Rust, and the
app never connects to an external NovelWorld server. All application services,
data, and generated secrets stay on the player's computer in the operating
system's per-user application data directory. AI features require Internet
access to the configured model provider and an API key only when they are used.
Desktop startup stops its named embedded database, refuses occupied service
ports, applies every embedded migration through 0024, and only then starts the
five services; the migration therefore runs without an old local writer.
Desktop archives are experimental and forward-migration-only: do not reuse a
post-0024 application-data directory with an older archive. Older binaries do
not contain the current downgrade guard.

The current artifacts are unsigned engineering builds. Windows SmartScreen and
macOS Gatekeeper may warn on first launch; public distribution requires platform
code-signing certificates and macOS notarization.

### Platform support

| Mode | Windows | Linux | macOS |
|------|---------|-------|-------|
| Docker server | `start.cmd` | `./start.sh` | Not qualified |
| Portable desktop | x64 engineering build | x64 AppImage engineering build | Apple Silicon and Intel engineering builds |

No default application account is installed. A key entered later in protected Settings is
sent only to your server, encrypted before PostgreSQL persistence, and never
written to browser storage. Advanced operators can still use `LLM_API_*` in
`.env`; environment configuration takes precedence over protected platform Settings.
Signed-in readers may optionally store their own encrypted provider key; their
requests and visible usage then follow that key, while readers without one use
the platform key without seeing its aggregate cost.

### Development mode

<details>
<summary>Click to expand</summary>

**Prerequisites:**
- Current stable [Rust](https://rustup.rs/) (the locked dependency graph currently requires ≥ 1.94.1)
- [Docker](https://docs.docker.com/get-docker/)
- [Node.js](https://nodejs.org/) 22+ & [pnpm](https://pnpm.io/)
- OpenAI-compatible API key (optional until an AI-backed operation is used)

```bash
# 1. Configure
cp .env.example .env
# Set the generated server secrets. LLM_API_KEY is optional when using web setup.

# 2. Start the minimum database profile
docker compose up -d postgres
# Optional Redis projection: use the supported launcher, or explicitly select
# CACHE_MODE=redis + REDIS_PASSWORD + REDIS_URL and --profile redis together.

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
│ PostgreSQL 18 + pgvector · optional Redis   │
└─────────────────────────────────────────────┘
```

| Layer | Stack | Details |
|-------|-------|---------|
| Backend | Rust / Axum | 5 async microservices |
| Database | PostgreSQL 18 | pgvector semantic search, pg_trgm fuzzy matching |
| Cache | Optional Redis | Reconstructable recent-message projection; PostgreSQL is the base profile |
| AI | Operator-configured provider | Structured output + streaming, bounded retry |
| Frontend | React + TypeScript | Tailwind CSS, Feature-Sliced Design |
| Server deploy | Docker Compose | 8 base containers, optional Redis, plus a migration job |
| Desktop deploy | Tauri portable bundle | Same five services on loopback + bundled pg0; no Docker |

The five Rust runtimes have statically enforced DDD, HTTP, and relation-owner
boundaries, but the current private `single-node-v1` deployment still uses one
PostgreSQL schema/role, 20 declared cross-owner foreign keys, two exact
lifecycle-trigger accesses, five cross-owner trigger/routine bindings, and ten
full-file-hash debts for historical executable migrations. This is not a claim
of physical database isolation, replica safety, or horizontal scaling; see the
[architecture evidence limits](./docs/ARCHITECTURE.md#code-boundaries).

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
