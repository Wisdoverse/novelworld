# NovelWorld Service Specification

Status: H0 candidate `private-preview-v1`

Purpose: Define a platform that transforms a supported novel into an interactive world where readers engage
with AI-driven character agents, influence narrative branches, and maintain persistent memory across
sessions.

This document is the intended normative target, not evidence that the current
runtime conforms to every clause. The current supported envelope, known gaps,
and claim dispositions are recorded in
[`docs/PRODUCT_CONTRACT.md`](./docs/PRODUCT_CONTRACT.md). Runtime code,
migrations, and tests remain the source of truth for current behavior. The
clause-by-clause review state is recorded in
[`docs/SPEC_CONFORMANCE.md`](./docs/SPEC_CONFORMANCE.md).

## Normative Language

The key words `MUST`, `MUST NOT`, `REQUIRED`, `SHOULD`, `SHOULD NOT`, `RECOMMENDED`, `MAY`, and
`OPTIONAL` in this document are to be interpreted as described in RFC 2119.

`Implementation-defined` means the behavior is part of the implementation contract, but this
specification does not prescribe one universal policy. Implementations MUST document the selected
behavior.

`LLM` refers to a Large Language Model reached through a configured provider
adapter. A working adapter does not by itself qualify a provider or model.

---

## 1. Problem Statement

NovelWorld is a platform that ingests supported novel text, extracts its characters and world
structure via LLM analysis, and exposes each character as a stateful AI agent that readers can
converse with. The platform solves five problems:

- It turns static novel text into a living, interactive world without requiring author involvement.
- It isolates each reader's experience so that one reader's choices and conversations do not affect
  another reader's world state.
- It maintains per-reader, per-character memory across sessions using a four-layer memory pyramid,
  so character relationships evolve naturally over time.
- It surfaces key narrative branch points and lets readers make choices that diverge the story,
  with the LLM generating canon-consistent consequences.
- It can generate bounded, optional character portraits from textual appearance descriptions;
  missing portraits do not remove a character or block import readiness.

Important boundary:

- NovelWorld is a reader-facing interactive platform, not an authoring tool.
- The platform does not modify the source novel text; it only generates derivative interactive
  content layered on top of it.
- A reader's choices affect only their own world state, not the canonical novel text.

---

## 2. Goals and Non-Goals

### 2.1 Goals

- Accept novel text via file upload (TXT, EPUB, PDF) or direct paste and parse it into chapters and
  characters without manual annotation.
- Generate a character agent for every extracted character, with personality, background, and
  speaking style derived from the source text.
- Maintain a four-layer memory pyramid per reader per character: short-term, mid-term, long-term,
  and permanent layers with automatic compression and semantic retrieval.
- Stream character dialogue responses via Server-Sent Events (SSE) so readers see text appear
  progressively.
- Present branch choice nodes at key chapters and persist the reader's selections in a world state
  document.
- Use an original `PlayerEntity` as the primary identity; retain character identity only as a
  compatibility path for conversation and branch choices, with the agency boundary in §8.2.
- Generate character avatar images from appearance descriptions using an image generation API.
- Enforce per-reader memory isolation so no reader can observe another reader's conversation
  history or world state.
- Provide a user account system with JWT-based authentication and bcrypt password hashing.

### 2.2 Non-Goals

- Modifying or annotating the source novel text.
- Multi-reader shared world state or collaborative sessions.
- Real-time multiplayer interaction between readers.
- Authoring tools for creating original novels.
- Prescribing a specific LLM provider. Operators may configure compatible
  endpoints, but only separately qualified provider/model slices are supported.
- Built-in content moderation in the private-preview profile. A public profile
  remains blocked on the H2 policy and enforcement controls.

---

## 3. System Overview

### 3.1 Main Components

1. `API Gateway`
   - Receives all inbound HTTP requests.
   - Validates JWT tokens and injects authenticated user context.
   - Routes requests to downstream microservices.
   - Proxies SSE streams without buffering.

2. `User Service`
   - Manages user registration, login, and token issuance.
   - Stores bcrypt-hashed passwords.
   - Issues and validates JWT access tokens.

3. `Novel Service`
   - Accepts novel uploads and text pastes.
   - Orchestrates LLM-based parsing: chapter splitting, character extraction, world summary
     generation.
   - Triggers bounded avatar generation for eligible extracted characters.
   - Stores parsed artefacts in PostgreSQL and, when enabled, original upload bytes in object
     storage.

4. `Agent Service`
   - Manages character agent sessions.
   - Builds LLM prompts from character profile, world state, and memory layers.
   - Streams LLM responses back to the caller via SSE.
   - Projects committed turns into bounded recent context and periodic mid-term summaries.
   - Retrieves any available long-term and permanent records without implying that current writers
     populate them.

5. `Narrative Service`
   - Identifies key branch nodes in chapters.
   - Presents choice options to readers.
   - Persists reader choices in the world state document.
   - Generates LLM-derived consequence text for each choice.

6. `Database` (PostgreSQL 18)
   - Single source of truth for all structured data.
   - Uses `pgvector` extension for semantic memory retrieval.

7. `Object Storage` (optional, S3-compatible)
   - Retains original uploaded bytes only when the operator enables source retention.
   - Uses server-generated storage keys. Provider-hosted avatar bytes remain outside NovelWorld;
     only returned URL metadata is stored.

8. `Cache` (Redis)
   - Stores a bounded, reconstructable recent-message projection.

### 3.2 Abstraction Layers

NovelWorld is easiest to implement when organized into these layers:

1. `Identity Layer` (user auth)
   - JWT issuance, validation, and refresh.
   - Password hashing and verification.

2. `Ingestion Layer` (novel parsing pipeline)
   - File parsing, chapter splitting, character extraction, world summary, avatar generation.

3. `Agent Layer` (character AI)
   - Memory pyramid management, prompt construction, LLM invocation, SSE streaming.

4. `Narrative Layer` (branch logic)
   - Node detection, choice presentation, world state mutation, consequence generation.

5. `Storage Layer` (persistence)
   - PostgreSQL query helpers, Redis cache helpers, S3 upload/download helpers.

6. `Gateway Layer` (routing and auth)
   - JWT middleware, request routing, SSE proxy, CORS handling.

### 3.3 External Dependencies

- A configured LLM provider adapter.
- Optionally, configured image-generation and embedding adapters.
- PostgreSQL 18 with the `pgvector` and `uuid-ossp` extensions installed.
- Redis 7 or later.
- Optionally, an S3-compatible object storage endpoint for retained source bytes.

---

## 4. Core Domain Model

### 4.1 Entities

#### 4.1.1 User

Fields:

- `id` (UUID v4)
  - Stable primary key.
- `email` (string, max 320 chars)
  - Unique. Used as login identifier.
- `password_hash` (string)
  - bcrypt hash, cost factor MUST be at least 12.
- `name` (string or null)
- `avatar_url` (string or null)
- `role` (enum: `user` | `admin`)
  - Default: `user`.
- `email_verified` (boolean)
  - Default: `false`.
- `created_at` (timestamptz)
- `updated_at` (timestamptz)
- `last_sign_in` (timestamptz or null)

#### 4.1.2 Novel

Fields:

- `id` (UUID v4)
- `user_id` (UUID, foreign key → User)
  - The reader who imported this novel.
- `title` (string, max 500 chars)
- `author` (string or null)
- `cover_url` (string or null)
- `description` (string or null)
- `world_summary` (string or null)
  - LLM-generated summary of the novel's world, factions, and rules.
- `genre` (string or null)
- `total_chapters` (integer)
  - Set after parsing completes.
- `status` (enum: `pending` | `parsing` | `ready` | `error`)
  - `pending`: uploaded but not yet parsed.
  - `parsing`: LLM pipeline is running.
  - `ready`: accepted chapters and the current extraction results are committed.
  - `error`: pipeline failed; `parse_error` contains the reason.
- `parse_error` (string or null)
- `deviation_mode` (enum: `canon` | `creative` | `remix`)
  - Controls how strictly the agent adheres to source text. Default: `canon`.
- `original_file_key` (string or null)
  - S3 key of the uploaded source file.
- `created_at` (timestamptz)
- `updated_at` (timestamptz)

#### 4.1.3 Chapter

Fields:

- `id` (UUID v4)
- `user_id` (UUID, nullable foreign key → User)
  - `null` identifies a canonical pre-divergence node.
  - Non-null nodes belong exclusively to one player's generated timeline.
- `novel_id` (UUID, foreign key → Novel)
- `chapter_number` (integer)
  - 1-indexed. Unique within a novel.
- `title` (string or null)
- `content` (text)
  - Full chapter text as extracted from the source.
- `summary` (string or null)
  - LLM-generated one-paragraph summary.
- `is_key_node` (boolean)
  - `true` if the chapter contains a narrative branch point.
- `key_node_description` (string or null)
  - Human-readable description of the branch point, used in the choice UI.
- `word_count` (integer)
- `created_at` (timestamptz)

#### 4.1.4 Character

Fields:

- `id` (UUID v4)
- `novel_id` (UUID, foreign key → Novel)
- `name` (string, max 200 chars)
- `aliases` (array of strings)
  - Alternative names used in the text.
- `role` (enum: `protagonist` | `antagonist` | `supporting` | `minor`)
- `description` (string or null)
  - Narrative description as extracted from the text.
- `personality` (string or null)
  - Structured personality summary used in agent system prompts.
- `background` (string or null)
  - Character backstory.
- `speaking_style` (string or null)
  - Description of how the character speaks, used in agent system prompts.
- `appearance` (string or null)
  - Physical appearance description, used as the avatar generation prompt.
- `avatar_url` (string or null)
  - URL of the generated avatar image.
- `avatar_status` (enum: `pending` | `generating` | `ready` | `error`)
- `first_appearance_chapter` (integer or null)
- `traits` (JSONB)
  - Extensible key-value store for additional character attributes.
- `created_at` (timestamptz)
- `updated_at` (timestamptz)

#### 4.1.5 CharacterMemory

One memory record in the four-layer pyramid. Memories are scoped to a `(character_id, user_id)`
pair so that each reader maintains a private relationship with each character.

Fields:

- `id` (UUID)
  - Version 4 by default. A permanent fact projected from a committed world
    turn uses a deterministic UUID v5 derived from that turn's UUID v4 under a
    private fixed namespace; it MUST NOT reuse the client-controlled turn UUID
    directly.
- `character_id` (UUID, foreign key → Character)
- `user_id` (UUID, foreign key → User)
- `layer` (enum: `short` | `mid` | `long` | `permanent`)
- `content` (text)
  - Memory data. A new committed-world-turn permanent fact is bounded JSON
    containing only facts with explicit provenance for its target character:
    the reader action only when its kind is character-directed (`converse`,
    `ally`, or `oppose`) and its target is that character UUID, events only
    when their actor list names that character, and relationship mutations
    only for that character. A UUID collision in a location or thread target
    MUST NOT count as character witness. Other turn mutations and generated
    narrative prose MUST NOT be promoted into that character's memory.
    A consumer MUST NOT classify content as an authoritative committed-world
    fact from JSON fields alone. It MUST also verify the deterministic UUID v5
    against the embedded source-turn UUID and the local layer, importance,
    character/user/novel, witness, and chapter-scope metadata; an unverified
    row that claims the committed-world protocol MUST be rejected at ingress
    before persistence and omitted from authoritative retrieval. The internal
    write endpoint MUST return success only after this complete gate passes and
    the canonical fact is inserted or an exact durable replay is confirmed;
    malformed counts/shape, witness or scope drift, a non-deterministic ID,
    unknown fields, and importance other than 7 return `422` with no write.
- `importance` (integer, 1–10)
  - Higher values survive compression longer.
- `embedding` (vector(1536) or null)
  - Semantic embedding for similarity search. REQUIRED for `long`; optional for
    `permanent`, whose durable fact remains directly retrievable without it.
- `access_count` (integer)
  - Incremented each time this memory is retrieved during prompt construction.
- `last_accessed` (timestamptz or null)
- `expires_at` (timestamptz or null)
  - Set only for `short` layer entries. Null means no expiry.
- `created_at` (timestamptz)

#### 4.1.6 ChatTurn

The durable idempotency and fencing record for one logical conversation turn.

Fields:

- `id` (client-generated UUID v4 from `Idempotency-Key`)
- `user_id`, `character_id`, `novel_id` (UUID ownership scope)
- `request_fingerprint` (32-byte message fingerprint; plaintext is not stored while pending)
- `chapter_context`, `reader_identity`, `reader_identity_type`,
  `reader_character_id`, `deviation_mode` (immutable server-side prompt snapshot)
- `status` (`in_progress` | `completed` | `failed`)
- `attempt` (positive fencing token)
- `lease_expires_at`, `failure_code`, `created_at`, `updated_at`, `completed_at`

The state fields MUST agree with the status: only `in_progress` has a lease,
only `failed` has a failure code, and only `completed` has `completed_at`.
At most one `in_progress` turn may exist for a
`(user_id, character_id, novel_id)` conversation, so a following turn builds
its prompt only after the preceding pair is committed. An expired abandoned
turn may be marked `superseded` when a new key claims that conversation.

#### 4.1.7 ChatMessage

One message in a conversation between a reader and a character agent.

Fields:

- `id` (UUID v4)
- `character_id` (UUID, foreign key → Character)
- `user_id` (UUID, foreign key → User)
- `novel_id` (UUID, foreign key → Novel)
- `turn_id` (UUID or null, foreign key → ChatTurn)
  - Null only for messages written before the idempotent-turn migration or by a
    rolled-back compatible service version.
- `role` (enum: `user` | `character`)
- `content` (text)
- `reader_identity` (string or null)
- `chapter_context` (integer or null)
  - The reader's current chapter at the time of the message.
- `created_at` (timestamptz)

Constraint: unique on `(turn_id, role)` where `turn_id IS NOT NULL`.

#### 4.1.8 NarrativeNode

A branch point within a chapter.

Fields:

- `id` (UUID v4)
- `novel_id` (UUID, foreign key → Novel)
- `chapter_number` (integer)
- `description` (text)
  - Situation description shown to the reader before the choices.
- `choices` (JSONB array of `NarrativeChoice`)
  - Each `NarrativeChoice` has:
    - `index` (integer, 0-based)
    - `text` (string) — the choice label shown to the reader
    - `consequence_hint` (string or null) — brief hint about the consequence
- `created_at` (timestamptz)

#### 4.1.9 PlayerChapter

A persisted prose projection of one chapter in a player's forked timeline.
The source `Chapter` remains immutable; this projection is rebuilt from
committed player state and is never shared between users.

Fields:

- `id` (UUID v4)
- `user_id` (UUID, foreign key → User)
- `novel_id` (UUID, foreign key → Novel)
- `chapter_number` (integer)
- `content` (text) — the complete effective chapter shown to this player
- `origin` (`choice` | `continuation`)
- `created_at`, `updated_at` (timestamptz)

Constraint: unique on `(user_id, novel_id, chapter_number)`.

#### 4.1.10 UserChoice

A reader's selection at a NarrativeNode.

Fields:

- `id` (UUID v4)
- `user_id` (UUID, foreign key → User)
- `novel_id` (UUID, foreign key → Novel)
- `node_id` (UUID, foreign key → NarrativeNode)
- `chapter_number` (integer)
- `choice_index` (integer)
- `choice_text` (string)
  - Snapshot of the choice label at the time of selection.
- `consequence` (string or null)
  - LLM-generated consequence narrative.
- `created_at` (timestamptz)

#### 4.1.11 WorldState

Aggregated state of a reader's journey through a novel.

Fields:

- `id` (UUID v4)
- `user_id` (UUID, foreign key → User)
- `novel_id` (UUID, foreign key → Novel)
- `state` (JSONB)
  - Structure:
    ```json
    {
      "choices": [{ "chapter": 3, "choice_index": 1, "choice_text": "..." }],
      "relationships": { "<character_id>": { "affinity": 7, "trust": 5 } },
      "world_events": ["event description 1", "event description 2"]
    }
    ```
- `updated_at` (timestamptz)

Constraint: unique on `(user_id, novel_id)`.

#### 4.1.11 ReadingProgress

A reader's current position in a novel.

Fields:

- `id` (UUID v4)
- `user_id` (UUID, foreign key → User)
- `novel_id` (UUID, foreign key → Novel)
- `current_chapter` (integer)
  - Default: 1.
- `reader_identity` (string or null)
  - The name the reader uses when entering the world.
- `reader_identity_type` (enum: `self` | `character`)
  - `self`: reader enters as themselves.
  - `character`: reader adopts a character's identity.
- `reader_character_id` (UUID or null, foreign key → Character)
  - Set only when `reader_identity_type = character`.
- `deviation_mode` (enum: `canon` | `creative` | `remix`)
- `last_read_at` (timestamptz)
- `created_at` (timestamptz)

Constraint: unique on `(user_id, novel_id)`.

### 4.2 Normalization Rules

- All UUIDs MUST be version 4 unless otherwise specified. The deterministic
  permanent-memory projection ID in §4.1.5 is the only current version-5
  exception.
- All timestamps MUST be stored as `TIMESTAMPTZ` (UTC-normalized).
- `chapter_number` is 1-indexed throughout the system.
- Character `name` comparisons for deduplication MUST be case-insensitive.
- `deviation_mode` defaults to `canon` at all levels unless explicitly overridden by the reader.

---

## 5. Novel Ingestion Pipeline

### 5.1 File Acceptance

The Novel Service MUST accept:

- Plain text files (`.txt`) in UTF-8, BOM-marked UTF-16, or GBK up to 10 MiB.
- EPUB files (`.epub`) up to 20 MiB.
- PDF files (`.pdf`) up to 20 MiB.
- Direct text paste payloads up to 5 MiB (UTF-8 encoded JSON string body).

On receipt, the service MUST:

1. Validate the declared type, size, and extractable content before accepting the import.
2. If source retention is enabled, store uploaded bytes under a server-generated managed key with
   durable cleanup intent. With retention disabled, extraction is request-local.
3. With retention disabled, split and validate deterministic chapters before acceptance. With
   retention enabled, the claimed import job rebuilds deterministic chapters from the retained
   object before any provider-backed enrichment.
4. Atomically commit the `pending` Novel and a `novel_import_jobs` row — with chapters/chunks when
   splitting preceded acceptance, otherwise at the `source` stage — and cancel the retained
   object's cleanup intent in the same transaction. Attempt an immediate claim with a renewable
   lease; startup recovery claims any job left pending. Return `202` and the Novel ID only after
   the durable boundary commits.

An accepted import MUST be recoverable after process death from its committed `source`,
`chapters`, or `enriched` stage. A job committed at `source` MUST replay the retained object to
rebuild chapters; a missing or unreadable retained object fails the job with actionable re-upload
guidance and never advances the stage.

### 5.2 Parsing Pipeline

Provider-backed enrichment runs asynchronously after file acceptance. It MUST:

0. When the committed stage is `source`, read the retained object for the Novel's managed key,
   re-extract and re-split deterministic chapters within the acceptance bounds, and atomically
   commit the chapters and the `chapters` stage advance behind the `(novel_id, attempt)` fence
   before any provider call.
1. Set `status = parsing`.
2. Claim exactly one current attempt; renew its lease during external work and fence every
   authoritative write by `(novel_id, attempt)`.
3. Atomically replace the character and relationship snapshot produced by the Character Extractor
   (see §5.4).
4. Identify key narrative nodes using the Node Detector (see §5.6); retries MUST NOT duplicate
   authoritative node state.
5. Persist the world summary and chapter count, advancing the durable stage to `enriched`.
6. Commit a source-validated canonical model before advancing the job and Novel to `completed` and
   `ready` in one transaction.
7. Start bounded avatar generation for eligible characters (see §5.7) as a non-authoritative
   projection that cannot block readiness.

On an unrecoverable current-attempt error, the pipeline MUST atomically set the job to `failed` and
the Novel to `error`, store an actionable public message in `parse_error`, and keep detailed
provider/internal errors in logs rather than exposing them to readers. Pending or expired
in-progress jobs MUST be reclaimed after restart; completed jobs MUST NOT call a provider again.
Provider calls across attempts MUST stay inside the approved import-provider budget policy
(`docs/IMPORT_BUDGET.md`): a job at the attempt ceiling MUST be marked terminally `failed` with
`budget_exhausted` and actionable re-upload guidance, and MUST NOT be reclaimed or retried.

### 5.3 Chapter Splitter

Input: full novel text (string).

Algorithm:

1. Attempt pattern-based splitting using the following heuristics in order:
   - Lines matching `^(第[零一二三四五六七八九十百千万\d]+[章节回]|Chapter\s+\d+|CHAPTER\s+\d+)` are
     chapter boundaries.
   - Lines matching `^\s*\d+\s*$` (standalone numbers) are chapter boundaries.
   - Paragraphs separated by two or more blank lines, where the paragraph is fewer than 100 chars,
     are chapter boundaries.
2. If fewer than 2 chapter boundaries are detected, fall back to LLM-based splitting:
   - Send the first 8000 tokens of the text to the LLM with a structured prompt requesting a JSON
     array of `{ chapter_number, title, start_offset, end_offset }` objects.
3. Each chapter MUST have a non-empty `content` field after trimming.
4. Chapters MUST be numbered sequentially starting from 1.

### 5.4 Character Extractor

Input: full novel text or per-chapter summaries (implementation-defined).

The extractor MUST invoke the LLM with a structured output schema requesting:

```json
{
  "characters": [
    {
      "name": "string",
      "aliases": ["string"],
      "role": "protagonist|antagonist|supporting|minor",
      "description": "string",
      "personality": "string",
      "background": "string",
      "speaking_style": "string",
      "appearance": "string",
      "first_appearance_chapter": "integer or null"
    }
  ],
  "relationships": [
    {
      "from_character": "string",
      "to_character": "string",
      "relationship_type": "string",
      "description": "string",
      "strength": "integer 0-100"
    }
  ],
  "world_summary": "string",
  "genre": "string"
}
```

The extractor MUST:

- Deduplicate characters by name (case-insensitive) and merge aliases.
- Extract at least all characters who appear in more than one chapter.
- Return at most 50 characters per novel to bound LLM cost.

### 5.5 World Summarizer

Input: the novel title and the bounded representative sample used by character
extraction (§5.4). The current implementation returns the world summary in the
same structured extraction response rather than making a second provider call.

The extraction request MUST request a world summary covering:

- Setting (time period, geography, society).
- Major factions or groups.
- Core conflict.
- Unique world rules (magic systems, technology, etc.) if applicable.

The summary MUST be stored in `novels.world_summary` and MUST NOT exceed 2000 characters.

### 5.6 Node Detector

Input: bounded chapter summaries with their real chapter numbers.

The Node Detector MUST return bounded candidate branch points. It MAY batch or
sample chapter summaries; the product contract does not require one provider
call per chapter. Model output MUST use a structured schema:

```json
{
  "nodes": [
    {
      "chapter_number": "integer",
      "description": "string",
      "choices": [
        { "text": "string", "hint": "string" }
      ]
    }
  ]
}
```

For each accepted node, the service MUST:

- Set `chapters.is_key_node = true`.
- Store the `description` in `chapters.key_node_description`.
- Let the Narrative Service validate and persist the user-visible node and
  choices when they are requested.

The number of choices per node MUST be between 2 and 3 inclusive. Current
reader-facing node text is Simplified Chinese; other languages remain outside
the supported journey.

### 5.7 Avatar Generation

For each eligible extracted character with a non-null `appearance` field, up to
the documented cost cap of 30 characters per import:

1. Set `characters.avatar_status = generating`.
2. Construct a bounded image-generation prompt from the `appearance` field.
3. Invoke the configured image adapter; rendering parameters are adapter-owned.
4. Store the provider-returned URL as non-authoritative metadata and set
   `characters.avatar_status = ready`.
5. On failure, set `characters.avatar_status = error`. Avatar failure MUST NOT block the novel
   from reaching `status = ready`.

Characters beyond the cap remain available without generated avatars. The
provider owns image-byte retention and deletion unless a future reviewed change
adds a NovelWorld-owned media lifecycle.

### 5.8 Extraction Quality Gates

Each supported positive extraction slice MUST meet the versioned
`extraction-quality` policy (`docs/EXTRACTION_QUALITY.md`) before it can claim
qualification:

1. Every positive case MUST produce a non-empty, structurally valid accepted
   canon; malformed and unsupported inputs MUST reach their expected bounded,
   actionable errors and are labeled separately.
2. Accepted canon facts MUST meet the approved per-category coverage,
   precision/hallucination, chronology, and provenance thresholds on the
   versioned corpus.
3. Rejecting every input or producing an empty accepted canon MUST NOT pass
   any gate.
4. Recorded evaluation MUST be deterministic and required in CI; live
   evaluation MUST record provider, model, corpus/rubric versions, and the
   exact commit, and MUST fail closed on malformed judge output.

---

## 6. Character Agent System

### 6.1 Agent Identity

Each character agent derives its identity from the Character entity. The agent system prompt MUST
include:

- The character's `name` and `aliases`.
- The character's `role` in the story.
- The character's `personality`, `background`, and `speaking_style`.
- The novel's `world_summary`.
- The reader's current `chapter_number` from `ReadingProgress`.
- The reader's `reader_identity` and `reader_identity_type`.
- The reader's `deviation_mode`.

The system prompt MUST instruct the character to:

- Respond only in the character's established voice and speaking style.
- Not reveal plot events that occur after the reader's current chapter (anti-spoiler constraint).
- Acknowledge the reader's identity (self or character) appropriately.
- Incorporate relevant memories naturally without breaking character.

### 6.2 Memory Pyramid

The memory pyramid has four layers. Each layer has distinct characteristics:

| Layer | Storage | Max Entries | Retrieval | Expiry |
|---|---|---|---|---|
| `short` | PostgreSQL messages + bounded Redis projection | 50 projected messages | Recency | Account/novel lifecycle |
| `mid` | PostgreSQL | Implementation-defined | Recency + Importance | Account/novel lifecycle |
| `long` | PostgreSQL + pgvector | Implementation-defined | Semantic similarity | Account/novel lifecycle |
| `permanent` | PostgreSQL + optional pgvector projection | Implementation-defined | Bounded direct candidates (journey recency plus legacy importance/recency) + optional semantic similarity | No automatic eviction; account/novel lifecycle still applies |

#### 6.2.1 Short-Term Layer

- Contains raw conversation turns from the current and recent sessions.
- Persisted to PostgreSQL for durability; Redis holds only a bounded projection and has no
  time-based expiry.
- The current runtime creates a mid-term summary every 20 committed messages. A later contract may
  make this threshold configurable.

#### 6.2.2 Mid-Term Layer

- Contains compressed summaries of past conversation sessions.
- Created by the Compression Pipeline from short-term entries.
- Retrieved by recency and importance score during prompt construction.
- Each created mid-term summary attempts best-effort promotion to a 1536-dim
  embedded long-term entry. An unavailable or incompatible embedding leaves the
  mid-term summary intact.

#### 6.2.3 Long-Term Layer

- Contains semantically indexed memories of significant events and relationship milestones.
- Each entry MUST have an `embedding` vector.
- Retrieved via cosine similarity search using `pgvector`.

#### 6.2.4 Permanent Layer

- Contains immutable facts: the reader's name, major choices, and critical relationship events.
- Entries in this layer MUST NOT be compressed or removed by memory maintenance.
- Novel or account deletion MUST erase them under the data-lifecycle contract.
- The committed fact MUST be stored before embedding generation. Embedding is
  an optional search projection; provider failure or an incompatible vector
  MUST NOT discard the fact.
- Retrieved directly within the reader's chapter boundary and, when a valid
  embedding exists, optionally via cosine similarity. Directly included IDs
  MUST NOT be duplicated by semantic retrieval.
- A world-turn fact's chapter coordinate MUST be the committed session's
  `entry_context.unlocked_through_chapter`, not merely its player checkpoint,
  because any validated mutation may have been influenced by canon up to that
  source high-water mark. A later reading-progress rewind MUST therefore
  exclude that fact until the boundary is unlocked again.
- At first adoption, upgrade migration MUST mark every pre-contract completed
  world turn terminal `skipped`; old output has no trustworthy character-
  witness provenance and MUST NOT be promoted into a new structured fact.
  Migration MUST retain pre-contract memory rows for export and ordinary
  lifecycle deletion rather than guess an unavailable historical protagonist.
  Every Agent prompt-consumer path MUST quarantine the old producer class
  (`permanent`, importance 7, UUID version nibble 4) from direct and semantic
  retrieval. The quarantine deliberately does not require an RFC UUID variant,
  because the former producer accepted any UUID with version nibble 4. This
   conservative boundary can hide a legitimate legacy row in the same narrow
   class, but MUST NOT delete it or fabricate a replacement historical fact.
- An authenticated committed-world-turn fact MUST enter direct or semantic
  prompt memory only when the persisted conversation-turn identity snapshot is
  `self`. Until memory records carry durable reader-identity provenance, a
  `character` turn MUST omit every direct `mid`/`long`/`permanent` and semantic
  memory candidate and MUST NOT create a summary or other derived memory
  projection. This deliberately sacrifices unprovable continuity rather than
  leak self-mode or another-character context.
- A committed-world-turn fact MUST be scoped to the deterministic protagonist
  only when that protagonist is explicitly named by a direct UUID action
  target, an event's `actor_character_ids`, or a relationship mutation. No
  protagonist, or no such witness evidence, produces a terminal `skipped`
  projection rather than inferred character knowledge. The stored payload MUST
  then be filtered to that same evidence: a directly targeted action's
  observable kind and target UUID, events
  naming the protagonist as actor, and that protagonist's relationship
  changes only. The reader's free-form `intent` is private motivation, not
  witnessed dialogue, and MUST NOT cross into character memory. Witnessing one part of a turn MUST NOT expose unrelated event,
  knowledge, inventory, location, thread, faction, or canon-event mutations.

### 6.3 Compression Pipeline

The current mid-term projection is triggered every 20 committed messages.

Steps:

1. Retrieve the twenty most recent committed `ChatMessage` records for the
   `(character_id, user_id, novel_id)` conversation in chronological order.
2. Send those messages to the LLM with a prompt requesting a concise summary of the key events,
   emotional tone, and relationship developments.
3. Store the summary as a new `mid` layer entry with bounded importance.
4. Keep committed chat messages in PostgreSQL; the Redis projection remains bounded independently.
5. Attempt best-effort 1536-dim long-term promotion of the new mid summary.
   Separately, every committed open-world turn stores a durable projection
   status on its `world_turns` row: `pending` until a protagonist-scoped
   permanent fact is acknowledged as `saved`, or eligibility is conclusively
   recorded as `skipped`. A same-key replay or autonomous recovery pass MUST
   attempt to compensate eligible `pending` rows using the same deterministic memory ID. Narrative MUST
   scan a bounded batch periodically and durably rotate attempted rows so a
   persistently ineligible or unavailable oldest batch cannot starve later
   pending turns. Narrative may record `saved` only after Agent
   returns success for a fully authenticated insert or exact durable replay;
   parsing a JSON-shaped payload or reserving an unauthenticated row is not an
   acknowledgement. `saved` and `skipped` are terminal and MUST
   replay the committed result without an Agent Service call. After projection
   returns either terminal candidate but before acknowledging it on the turn
   row, Narrative MUST recheck both persisted `self` identity and the committed
   world's source high-water from one current server-owned progress snapshot.
   A concurrent rewind MUST return the content-free
   `reading_progress_behind_world` error and leave the row `pending`; restoring
   progress allows the same key or a later recovery pass to compensate without
   another world generation or commit.

### 6.4 Prompt Construction

Before invoking the LLM for a conversation turn, the Agent Service MUST construct the prompt as
follows:

1. **System prompt**: character identity block (§6.1).
2. **Memory data blocks**: in `self` mode, bounded JSON explicitly labeled as
   untrusted data, never as instructions:
   - Bounded directly retrieved `permanent` entries within the server-owned
     chapter boundary. Authenticated committed-world facts are selected before
     legacy prose consumes the permanent-memory character budget; at least the
     newest eligible fact MUST remain whole when one fits the block, and the
     selected facts are emitted in causal turn order after any legacy text.
   - Bounded `long`/`permanent` semantic results when a compatible query
     embedding is available, excluding permanent IDs already included directly.
   - Bounded recent/important `mid` entries.
   A `character` turn MUST omit this entire block until each candidate has
   durable identity provenance, and MUST NOT project that turn into the
   unscoped memory layers.
3. **Conversation history**: bounded recent committed PostgreSQL `ChatMessage`
   records within the chapter boundary, in chronological order. Scope MUST be
   applied in PostgreSQL before ordering and limiting: `self` admits completed
   self claims plus legacy messages without a persisted identity claim, while
   `character` admits only completed claims whose `reader_character_id` exactly
   matches that turn's persisted identity snapshot. Failed, incomplete,
   other-character, self, and unprovenanced rows MUST be absent from a character
   prompt. Redis is only a projection and is not prompt authority.
4. **Source and reader context**: bounded chapter-visible lore, persisted
   progress, identity, and deviation mode.
5. **World context**: Agent MUST attempt this lookup only when the persisted
   conversation-turn identity snapshot is `self`; a `character` turn MUST omit
   the lookup even if the current downstream identity changes concurrently.
   After open-world entry, Narrative may return a bounded,
   server-committed transport DTO. After validating its scope and bounds, Agent
   MUST build a separate provider-facing allowlist containing only world
   time/turn, that character's numeric relationship score and goals, a canonical
   event naming that character, directly targeted recent player actions, and
   actor-listed recent player events. Routing UUIDs, protocol/model metadata,
   player name/location, branch choices, active threads, relationship-change
   prose, perception, and model-generated reasons MUST NOT be sent to the
   provider. Those fields have no independent per-character witness provenance
   in this version and are therefore omitted rather than
   treated as character knowledge. Structured action fields are authoritative
   facts; rendered narrative fields are labeled as generated projections and
  cannot override structured state or agency. Free-form player `intent` is not
  character-visible and MUST be omitted. A recent action supplied
   to a character MUST have a character-directed kind (`converse`, `ally`, or
   `oppose`) whose target is that character UUID. Events use their separately
   filtered fields, while relationship context exposes only its numeric score,
   rather than exposing an originating action or private intent. A supplied player-origin event MUST list the character in its own
   `actor_character_ids`, and the Agent consumer MUST reject an event that does
   not name the requested character. The world view MUST carry the committed
   session's `entry_context.unlocked_through_chapter` as its canonical source
   high-water mark. If that mark is absent during a rolling deploy or exceeds
   current server-owned reading progress, the Agent MUST omit the entire world
   view before provider invocation, including after reading progress is
   rewound. Agent requests for this view MUST declare
   `X-World-Context-Version: 2`; Narrative MUST return no view to an
   authenticated older consumer that omits or changes that version. Thus either
   mixed-version direction degrades to no world context rather than exposing an
   unbounded view. Narrative and Agent MUST each enforce the character-directed
   action kind and target UUID before provider invocation. The current
   structural visibility model does not infer line of sight, location
   propagation, or social knowledge beyond explicit IDs.
6. **User message**: the current reader input.

Each data block has a fixed character/item bound and the complete prompt MUST
remain within the Agent Service prompt budget. A request that still exceeds the
total budget fails closed before provider invocation.

### 6.5 Streaming Response

The Agent Service MUST stream the LLM response to the caller via SSE. Every new
turn request MUST include `Idempotency-Key: <UUID v4>`. The key identifies one
logical turn and MUST be reused for transport retries; reusing it with a
different user, character, novel, or message returns `409 idempotency_conflict`.

SSE event format:

```
event: delta
data: {"content": "<token>"}

event: done
data: {"turn_id":"<UUID>","committed":true,"replayed":false}
```

On error:

```
event: error
data: {"code":"<error_code>","message":"<human-readable message>","turn_id":"<UUID>"}
```

`done` is a commit acknowledgement, not merely an upstream EOF marker. The
Agent Service MUST NOT emit it until a single PostgreSQL transaction has:

1. Fenced the active `chat_turns.attempt`.
2. Stored exactly one `user` and one `character` `ChatMessage` for the turn.
3. Marked the turn `completed`.

Client disconnect MUST stop delivery only; the in-process producer continues to
the durable boundary. A provider error, malformed frame, or EOF without the
provider's explicit terminal event MUST fail the attempt and MUST NOT be
followed by `done`. A completed key is replayed without another LLM call. An
active key returns `409 turn_in_progress` with `Retry-After`; a failed or
expired attempt may be reclaimed with a higher fencing attempt.

Redis and memory-compression updates are derived, best-effort projections after
the PostgreSQL commit. Prompt history MUST be read from committed PostgreSQL
messages so Redis lag or restart cannot erase the immediately preceding turn.

---

## 7. Narrative Branch System

### 7.1 Node Presentation

When a reader advances to a chapter where `is_key_node = true`, the Narrative Service MUST:

1. Check whether the reader has already made a choice at this node by querying `UserChoice` for
   `(user_id, node_id)`.
2. If a choice exists, return the existing choice and consequence without re-presenting options.
3. If no choice exists, generate/cache the `NarrativeNode` under
   `(user_id, novel_id, chapter_number)` and return its `description` and
   `choices` array. Runtime nodes MUST be player-scoped because their prompt
   includes PlayerEntity, private WorldState, and deviation mode; an
   uncommitted legacy shared node MUST NOT start a new choice.
4. Node persistence is first-writer-wins. A competing generation MUST reload
   and return the durable node without replacing its id, description, anchor,
   choices, or creation time.
5. Before returning uncommitted options, reload the latest WorldState and omit
   them if open-world entry started, the Player checkpoint sealed the chapter,
   or the committed choice prefix reached the chapter. A committed same-user
   legacy choice remains exactly replayable.

### 7.2 Choice Submission

When a reader submits a choice:

1. Validate that `choice_index` is within the bounds of `NarrativeNode.choices`.
2. Treat `(user_id, node_id)` as the natural operation key. If that key already
   contains the same `choice_index`, replay the committed choice, world state,
   and effective chapter without another provider call. If it contains a
   different index, return `409 choice_conflict` and reload the committed
   choice, world state, and effective chapter; the
   submitted index MUST NOT overwrite or masquerade as the committed choice.
3. Snapshot a deterministic fingerprint of the `WorldState` used for
   generation, then invoke the LLM to generate replacement prose from the
   exact inline anchor through the end of the current chapter (see §7.3).
4. Under the same `WorldState` row lock used for persistence, require a new
   choice's fingerprint to still match and its chapter to be strictly later
   than the latest committed choice chapter. A stale, same-chapter, or backward
   draft MUST return `409` without leaving a `UserChoice`, state mutation, or
   `PlayerChapter`. Exact same-index replay bypasses these new-choice guards.
5. Atomically persist the `UserChoice`, updated `WorldState`, and the complete
   current `PlayerChapter`. If an earlier continuation projection occupies the
   same chapter, the committed choice rewrite MUST replace it atomically. An
   existing choice projection may only replay when it matches the committed
   result; any other projection conflict MUST roll back the whole transaction.
   The source `Chapter` MUST remain unchanged.
6. Optionally update `WorldState.state.world_events` if the consequence implies a world-level event.
7. Return both the consequence and the persisted effective chapter to the reader.

### 7.3 Consequence Generation

Input: novel world summary, chapter content, choice text, and the reader's prior choices from
`WorldState`.

The LLM MUST be prompted to generate a consequence narrative that:

- Is consistent with the novel's world and tone.
- Acknowledges the reader's prior choices where relevant.
- Is between 100 and 400 words.
- Ends with a clear transition to the next chapter.

The consequence MUST be stored in `UserChoice.consequence`, and the resulting
complete chapter MUST be stored as a `PlayerChapter` with `origin = choice`.
For an already committed choice whose legacy same-chapter projection is absent
or has `origin = continuation`, exact replay MUST reconstruct the chapter from
the immutable source, inline node anchor, and committed consequence, then
atomically restore the `choice` projection without another provider call.

### 7.4 Full Chapter Regeneration After Divergence

The first committed choice establishes a causal boundary. From the next chapter
onward, the original chapter text MUST NOT be displayed as the player's current
story. For each requested chapter, the Narrative Service MUST:

1. Return an already persisted `PlayerChapter` when one exists.
2. Require the immediately preceding player chapter, preventing gaps in the
   generated timeline.
3. Generate the entire chapter from the previous player chapter, committed
   `WorldState`, novel world summary, and the original chapter as reference-only
   source material.
4. Remove or rewrite canonical events whose preconditions no longer hold; it
   MUST NOT silently reset causality to the source novel.
5. Persist the winning result idempotently under
   `(user_id, novel_id, chapter_number)` before displaying it.
6. Fail closed when generation is unavailable rather than fall back to original
   prose that contradicts the player's timeline.

All runtime-generated narrative nodes MUST be player-scoped. Two users MUST NOT
receive or mutate the same generated node even before divergence because Player
identity, private world state, and deviation mode already affect the prompt.

### 7.5 World State Consistency

The Narrative Service MUST ensure that:

- `WorldState` is created on first access for a `(user_id, novel_id)` pair if it does not exist.
- All mutations to `WorldState.state` are atomic (use database transactions or optimistic locking).
- The `relationships` map in `WorldState.state` uses `character_id` (UUID string) as keys.
- Provider-generated branch transitions are committed only against the exact
  deterministic state fingerprint from which they were generated, and new
  choices advance strictly by chapter under the same row lock.
- `user_choices` is the durable branch-decision authority. Every element of
  `WorldState.state.choices` MUST be a node-keyed projection of exactly one
  durable choice, and every durable choice MUST have exactly one projection
  with matching node, chapter, index, text, consequence, canon-model version,
  and canonical checkpoint. Player creation, open-world entry, choice commit
  or replay, and world-turn reservation or completion MUST validate this
  bijection under their transaction lock. Missing, duplicate, unkeyed,
  malformed, or mismatched projections MUST return a typed conflict before a
  provider call or new authoritative write; exact replay MUST NOT bypass it.

### 7.6 Canonical Mainline and Open-World Evolution

A completed source novel MAY be transformed into a living world, but generated
prose MUST NOT become the authoritative state. The Narrative Service MUST keep
two distinct layers:

1. `CanonStoryModel`: an immutable, versioned, source-backed graph of story
   arcs, ordered events, locations, factions, world rules, character states,
   unresolved threads, and the canonical ending. Extracted entities and events
   MUST retain chapter provenance.
2. `PlayerTimeline`: an append-only overlay beginning at a canonical checkpoint
   and containing only the player's committed actions and validated world
   transitions.

Generated `PlayerChapter` prose is a durable read projection of committed
timeline state, not the authoritative transition log. It may be regenerated by
future migration tooling, while the source `Chapter` and canonical graph remain
immutable.

The user MUST enter as a durable `PlayerEntity`: a new person who does not exist
in source canon and has a chosen identity, background, capabilities, location,
inventory, relationships, faction standing, and discovered knowledge. The
primary interaction MUST describe actions taken by this player. It MUST NOT ask
the player to choose actions on behalf of canonical characters.

An open-world session is reconstructed from a canonical checkpoint plus its
player entity and player timeline. Canonical events continue when their
preconditions remain true, and canonical characters act according to their own
goals and knowledge. Player actions MAY observe, assist, obstruct, delay, or
redirect those events. A world turn MUST produce a structured transition containing
event, relationship, location, and thread changes alongside the narrative
rendering. The transition MUST be schema-valid, entity-valid, spoiler-bounded,
idempotent, and atomically committed before its prose is shown as complete.
The first successfully committed start MUST seal its `WorldSession.entry_context`.
A later valid start request is a resume hint and MUST return that persisted
winner without replacing it with a freshly derived candidate context.

For one reader and novel, committed world turns form a single total order. A
new request MUST carry the `expected_turn_number` of the world view from which
the reader chose its action. Narrative MUST use that client-observed revision
in the durable claim and reject a different logical key when authoritative
state has advanced; it MUST NOT silently reinterpret a delayed action as the
next turn. Exact-key replay matches the persisted expected revision and remains
valid after its own commit. A new turn MUST be generated from current authoritative state plus a bounded,
ordered tail of committed turn actions and narrative endings so it continues
from what the reader was last shown. Thread-advance and thread-resolution
actions MUST target a currently open thread, and the committed transition MUST
update that exact thread. This player timeline uses the PostgreSQL journal;
a separate graph database is not required.

The browser MUST keep the original world-turn `Idempotency-Key` when no
response arrives, a `5xx` is returned, the turn is still in progress, or the
server reports `409 turn_outcome_unknown`. A retry MUST reuse that key until a
committed replay or a terminal rejection establishes the outcome.

Players MAY enter at any checkpoint already unlocked by server-side reading
progress only when every already committed branch choice and chapter-tagged
player event is at or before that checkpoint. Creating the `PlayerEntity`
seals this checkpoint as the upper bound for later branch-choice commits. A new
choice after open-world entry MUST be rejected; a request for the already
committed choice index MUST still replay, while a different index MUST
conflict. These rules MUST be repeated under the same database lock that
persists player or choice state so a race cannot create an invalid prefix.
Every new world action MUST also revalidate the stored entry checkpoint against
the complete committed choice/player-event prefix before provider invocation
or world commit. A legacy or externally restored state whose prefix extends
beyond that checkpoint MUST fail with a conflict; it MUST NOT project a fact at
the lower checkpoint chapter.

Future canon remains spoiler-bounded. Character agents, exploration, scheduled
canon events, and future narrative turns MUST consume the same committed player
timeline subject to the explicit character-visibility rule in §6.4.

Every player-entry/world-state/world-session read, new turn, exact turn replay,
and choice or branch response MUST recheck server-owned reading progress against
the session's canonical source high-water immediately before returning
world-derived state or prose. If progress has been rewound below that boundary,
those endpoints MUST return `409 reading_progress_behind_world`, MUST NOT
include the derived result, and a new world turn MUST NOT reserve a key or invoke
the provider. A choice or node generated concurrently with the rewind may remain
durable but MUST NOT cross the response boundary until progress is restored.
The effective-chapter endpoint is the safe exception: while behind, it MUST
return the immutable source chapter with `generated = false`, MUST NOT generate
or return a player projection, and MUST recheck after any in-flight generation
before responding. The browser MUST synchronously replace cached generated
chapter/world content with source content while behind and refetch after every
progress change. Generated-chapter cache identity MUST include the active
reader identity and server-owned progress boundary. While route and persisted
progress differ, the browser MUST use their lower chapter for character
availability, hide every generated chapter, and close a chat whose character
is not confirmed visible at that boundary. Restoring progress to the boundary
makes exact player results available again.

The `world_turns` row MUST durably record `memory_projection_status` as
`pending`, `saved`, or `skipped`. A fresh committed turn does not return success
until its projection reaches a terminal state; failure after the authoritative
turn commit is an unknown outcome, not a rollback of that turn. Replaying the
same logical-turn key MUST attempt to compensate eligible `pending` rows idempotently. Replaying
`saved`/`skipped` MUST return the exact committed result without recontacting
the Agent Service. Narrative MUST also run bounded autonomous compensation for
committed `pending` rows and durably rotate attempted scan positions so one
unrecoverable batch cannot starve later turns. While a committed turn remains `pending`, the persistence
boundary MUST refuse every different logical-turn key for the same user and
novel; the unresolved turn and an in-progress turn share one database-
serialized authority slot. Before contacting Agent and again before changing
`pending` to a terminal status, Narrative MUST recheck persisted `self` identity
and the committed result's source high-water from one server-owned progress
snapshot. If progress concurrently
falls behind, the request MUST return content-free
`reading_progress_behind_world`, keep the row `pending`, and allow the same key
or a later recovery pass to finish after progress is restored without
regenerating or recommitting the world turn. Dependency failure, identity
change, ambiguous acknowledgement, and invalid persisted data MUST also leave
the row `pending` rather than discard a committed fact.
The successful world-turn POST MUST include its terminal
`memory_projection_status`. A browser that receives that terminal response MAY
clear the pending request immediately; compatibility with an older response
shape MAY confirm the journal row before clearing.

Before sending a world turn, the browser MUST store the bounded, validated
`(action, expected_turn_number, Idempotency-Key)` tuple per `(user, novel)` in
`sessionStorage`. While
the result is ambiguous it MUST prevent a different action, restore the same
tuple across a same-tab reload or remount, and offer only same-key confirmation.
A matching committed journal row remains ambiguous while
`memory_projection_status = pending`; the browser MUST clear the stored request
only after that row reaches `saved`/`skipped`, or after an explicit terminal
rejection of the original POST establishes that no commit occurred. When local
storage is absent but the authoritative bounded journal contains its unique
`pending` row, the browser MUST reconstruct the exact action, turn UUID, and
`turn_number - 1` expected revision, lock the form, and offer same-key
compensation. A failed
confirmation refresh after a successful POST remains ambiguous regardless of
that refresh response's status. `sessionStorage` covers the interval before a
commit becomes journal-visible; committed-pending recovery does not depend on
the original tab. While a matching request remains pending, the current page
MUST poll the authoritative view at a bounded fixed interval and stop after a
terminal state. Same-key user confirmation remains a compatibility fallback
alongside the autonomous server scan.
When first-run setup, login, registration, or session confirmation establishes the authenticated
principal, the browser MUST remove NovelWorld pending world-turn records for
every other principal while retaining exact recovery records for the confirmed
principal. Before exposing that principal, it MUST also cancel and clear the
in-memory server-query cache so a private response from the previous principal
cannot satisfy the new principal's query. An authenticated mutation that
finishes after this boundary MUST NOT insert its response into that cache;
successful mutations invalidate and refetch only queries active under the
current authenticated principal. On logout, successful account deletion,
absence of an authentication token, or a confirmed `401`/`403` session
invalidation, the browser MUST clear the server-query cache and remove every
NovelWorld pending record before exposing the unauthenticated state. A
transient authentication failure or failed account-deletion request MUST retain
the applicable cache and recovery records. Every transition MUST preserve
unrelated `sessionStorage` keys.
Every protected asynchronous browser operation MUST remain bound to the access
token and, when already known, principal that initiated it. A late `401` or
`403` MAY invalidate the global session only when the request's actual bearer
token still equals the current token. A late identity refresh or account
deletion MUST NOT clear, replace, or expose a newer principal's auth state,
query/chat cache, or recovery records. Account-export bytes MUST be discarded
before inspection, Blob creation, or download if either the token or principal
changed while the request was in flight. When another browsing context changes
the shared `auth_token`, the current context MUST synchronously clear private
query and chat state and reload/re-authenticate before issuing or presenting
principal-scoped work; a refresh-token-only event or unchanged token MUST NOT
trigger that boundary.
After a novel is successfully removed from the authenticated user's shelf, the
browser MUST remove only that principal-and-novel pending record. A failed
novel-removal request MUST retain it, and neither outcome may alter pending
records belonging to another principal or novel.

---

## 8. Player Identity System

### 8.1 Identity Types

Users MAY choose one of two identity modes. The primary mode is `self`, whose
existing wire name is retained for compatibility:

- `self`: The user creates an original `PlayerEntity` and enters the world as a
  new person. The display name does not have to be the user's real identity.
  Agents address the player in second person and canonical characters perceive
  the player as another person in their world.
- `character`: A legacy compatibility mode in which the reader adopts a
  character identity for conversation. Until narrative nodes carry durable
  identity provenance, a character-identity reader MAY replay an already
  committed branch result but MUST NOT receive or submit a new branch. They
  also MUST NOT create a `PlayerEntity`, enter the open world, submit world
  turns, read or mutate the world journal, or hold relationship/faction/location
  mutation authority. The open world and every new narrative decision remain
  exclusive to `self` mode with a durable `PlayerEntity`.

### 8.2 Identity Constraints

- If `reader_identity_type = character`, `reader_character_id` MUST reference a Character that
  belongs to the same novel.
- A reader MUST NOT adopt the identity of the character they are currently conversing with.
- Identity changes take effect immediately for new conversation turns; they do not retroactively
  alter existing `ChatMessage` records.
- Character-identity readers MUST be refused every Player/open-world endpoint: reading or creating
  the Player entry, reading or starting the open-world session, and submitting or replaying a world
  turn. This remains true when a previously created `PlayerEntity` is retained; the refusal MUST be
  a conflict error, never a silent no-op.
- Character-identity readers MUST NOT receive an uncommitted narrative node or
  submit a new narrative choice. The server MUST fail closed before provider or
  persistence work; only exact read/replay of an already committed choice is
  allowed until node persistence is keyed by durable identity provenance.
- A character-identity chat turn MUST be fenced by the identity snapshot stored
  with its idempotent claim: Agent MUST neither request nor inject Player world
  context even if the reader concurrently switches to `self` before generation.
- The same persisted claim is the authority for conversation visibility. A
  character-identity prompt and history read MUST include only messages backed
  by completed claims for the exact same `reader_character_id`, with the scope
  predicate applied before pagination. Self, another character, failed or
  incomplete turns, and legacy messages without an identity claim MUST be
  omitted. Self-mode prompt/history reads MAY retain legacy unclaimed messages
  for compatibility.
- Until memory records gain equally durable identity provenance, character mode
  MUST return no mid/long/permanent/semantic memory, MUST expose no such records
  through the memory read endpoint, and MUST NOT create summary or promotion
  projections. Exact same-character committed recent chat is its only current
  cross-turn memory source.
- A general WorldState read in character mode MUST expose only the reader's `choices` plus the
  required empty `world_events` field. This choices-only projection MUST happen before source
  high-water validation, so hidden Player/open-world state neither leaks nor blocks a valid branch
  recovery. Internal character-world-context reads in character mode MUST return no content.
- Switching identity from `character` to `self` (or back) MUST NOT create or destroy a
  `PlayerEntity`; the `self`-mode open world only exists once the reader explicitly creates one.
- Character-identity data (identity type, name, character id) is portable account data: it MUST
  be included in account export and MUST NOT enter the canonical story model.
- In `self` mode, narrative choices MUST be actions performed by the
  `PlayerEntity`; they MUST NOT transfer control of a canonical character to the
  player.

### 8.3 Deviation Modes

| Mode | Agent Behavior |
|---|---|
| `canon` | Strictly follows the source text. Agent refuses to speculate beyond established facts. |
| `creative` | Allows the agent to extrapolate plausibly within the world's rules. |
| `remix` | Agent may introduce new plot elements while maintaining character consistency. |

---

## 9. Authentication and Authorization

### 9.1 Registration

Input: `email`, `password`, `name` (optional).

The User Service MUST:

0. Reject registration with `setup_required` until the initial administrator exists.
1. Validate that `email` is a valid RFC 5321 address.
2. Validate that `password` is at least 8 characters.
3. Check that no existing user has the same `email` (case-insensitive).
4. Hash the password with bcrypt at cost factor 12 or higher.
5. Create the User record.
6. Return a JWT access token and refresh token.

### 9.2 Login

Input: `email`, `password`.

The User Service MUST:

1. Look up the user by `email` (case-insensitive).
2. Verify the password against `password_hash` using bcrypt.
3. Update `last_sign_in`.
4. Return a JWT access token (expiry: `auth.access_token_expiry_seconds`, default: 3600) and a
   refresh token (expiry: `auth.refresh_token_expiry_seconds`, default: 604800).

### 9.3 JWT Structure

Access token claims:

- `sub` (string): user UUID.
- `role` (string): user role.
- `iat` (integer): issued-at Unix timestamp.
- `exp` (integer): expiry Unix timestamp.

The JWT MUST be signed with HMAC-SHA256 using the `JWT_SECRET` environment variable.

A refresh token is single-use: a successful refresh atomically consumes it and
returns both a new access token and a replacement refresh token.

### 9.4 Authorization Rules

- All application endpoints under `/api` except setup status/init, the
  deprecated setup LLM probe,
  `POST /api/auth/register`, `POST /api/auth/login`, and
  `POST /api/auth/refresh` MUST present a valid JWT. Setup init succeeds only while
  the `users` table is empty and atomically creates one administrator, its
  refresh token, and (when not supplied by the environment) an encrypted model
  configuration. Anonymous setup only accepts provider presets with fixed
  HTTPS endpoints; it never accepts an arbitrary URL. Process probes and the
  internal metrics routes follow §14 instead of application authentication.
- A user MAY only access novels, characters, memories, and world states that belong to their own
  `user_id`.
- The Gateway MUST reject requests with expired or invalid JWTs with HTTP 401.

---

## 10. API Contract

All application endpoints below are prefixed with `/api/`. The Gateway routes
them to the appropriate downstream service; process probes and internal service
routes are outside this public contract.

### 10.1 Authentication Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| GET | `/api/setup/status` | User | None | Administrator and model readiness (`contract: 3`) |
| POST | `/api/setup/init` | User | None | Validate AI settings and atomically create the initial configuration |
| POST | `/api/auth/register` | User | None | Register new user |
| POST | `/api/auth/login` | User | None | Login, returns tokens |
| POST | `/api/auth/refresh` | User | Refresh token | Atomically rotate and issue new access and refresh tokens |
| GET | `/api/auth/me` | User | JWT | Current user profile |
| DELETE | `/api/auth/me` | User | JWT | Permanently delete the acting account and owned application data |
| POST | `/api/auth/logout` | User | JWT | Invalidate refresh token |
| GET | `/api/settings/llm` | User | JWT | Read the acting scope without returning a secret: platform for administrators; personal when configured for other users, otherwise platform fallback metadata |
| PUT | `/api/settings/llm` | User | JWT | Validate and update the platform configuration for administrators or the acting user's encrypted personal configuration |
| GET | `/api/settings/llm/usage` | User | JWT | Read key-scoped usage: platform for administrators, personal for other users; return 403 when no personal key is configured |
| GET | `/api/account/export` | Gateway | JWT | Stream the acting user's complete `account-export-v1` NDJSON data |

### 10.2 Novel Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| GET | `/api/novels` | Novel | JWT | List user's novels |
| POST | `/api/novels` | Novel | JWT | Import novel (text paste) |
| POST | `/api/novels/upload` | Novel | JWT | Upload one bounded TXT, EPUB, or PDF file |
| GET | `/api/novels/:id` | Novel | JWT | Novel detail |
| GET | `/api/novels/:id/status` | Novel | JWT | Parse status (poll) |
| POST | `/api/novels/:id/retry` | Novel | JWT | Retry an owned failed import from its last committed durable stage; re-upload is required when no chapters remain |
| POST | `/api/novels/:id/lore/search` | Novel | JWT | Search owned, progress-bounded source lore |
| GET | `/api/novels/:id/relationships` | Novel | JWT | Source-extracted character relationships |
| DELETE | `/api/novels/:id` | Novel | JWT | Delete novel |

### 10.3 Chapter Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| GET | `/api/novels/:id/chapters` | Novel | JWT | Chapter list (id, number, title, is_key_node) |
| GET | `/api/novels/:id/chapters/:num` | Novel | JWT | Full chapter content |

### 10.4 Character Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| GET | `/api/novels/:id/characters` | Novel | JWT | Character list |
| GET | `/api/characters/:id` | Novel | JWT | Character detail |

### 10.5 Agent Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| POST | `/api/chat/:characterId/stream` | Agent | JWT | Stream conversation turn (SSE) |
| POST | `/api/chat/:characterId` | Agent | JWT | Complete one non-streaming conversation turn |
| GET | `/api/chat/:characterId/history` | Agent | JWT | Conversation history |
| GET | `/api/memories/:characterId` | Agent | JWT | Memory layer summary |
| DELETE | `/api/memories/:characterId/short` | Agent | JWT | Clear the reconstructable short-term projection |

### 10.6 Narrative Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| GET | `/api/narrative/:novelId/:chapter` | Narrative | JWT | Get branch node for chapter |
| GET | `/api/narrative/:novelId/chapters/:chapter` | Narrative | JWT | Get or generate the player's effective full chapter |
| GET | `/api/narrative/:novelId/player-entry` | Narrative | JWT | Read checkpoint options and the current original player |
| PUT | `/api/narrative/:novelId/player-entry` | Narrative | JWT | Create the original player at an unlocked checkpoint |
| POST | `/api/narrative/choose` | Narrative | JWT | Submit choice |
| GET | `/api/narrative/:novelId/world-state` | Narrative | JWT | Reader's world state |
| GET | `/api/narrative/:novelId/world` | Narrative | JWT | Read the current open-world session |
| POST | `/api/narrative/:novelId/world` | Narrative | JWT | Start the open-world session |
| POST | `/api/narrative/:novelId/world/turns` | Narrative | JWT | Commit an idempotent player action |

### 10.7 Progress Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| GET | `/api/progress/:novelId` | Novel | JWT | Reading progress |
| PUT | `/api/progress/:novelId` | Novel | JWT | Update chapter position |
| PUT | `/api/progress/:novelId/identity` | Novel | JWT | Set reader identity |

### 10.8 Account Export

`GET /api/account/export` MUST derive the subject from the Gateway-validated
JWT, compose internal-token-authenticated fragments in `user`, `novel`, `agent`,
and `narrative` order, and stream with bounded server memory. The response MUST
use `application/x-ndjson`, `Cache-Control: no-store`, and no `Content-Length`.
It MUST begin with an `account-export-v1` manifest and emit `complete` only after
all four service fragments finish. Clients MUST reject a file without that
terminal record. Queries MUST use explicit field allowlists and deterministic
ordering; credentials, tokens, runtime keys, chat-turn operational state,
embeddings, Redis/search projections, and external provider/operator data MUST
be excluded. Each fragment is a service-local statement snapshot, not a
distributed point-in-time backup. The Gateway MUST bound concurrency and total
elapsed work.

### 10.9 Error Response Format

All error responses MUST use the following JSON structure:

```json
{
  "error": {
    "code": "<machine-readable error code>",
    "message": "<human-readable description>"
  }
}
```

Standard error codes:

| Code | HTTP Status | Meaning |
|---|---|---|
| `unauthorized` | 401 | Missing or invalid JWT |
| `forbidden` | 403 | Valid JWT but insufficient permission |
| `not_found` | 404 | Resource does not exist |
| `conflict` | 409 | Unique constraint violation |
| `setup_required` | 409 | Initial administrator must be created first |
| `turn_in_progress` | 409 | The same idempotent turn is still being produced |
| `turn_outcome_unknown` | 409 | The world-turn lease was lost; retry the logical turn with the same key |
| `idempotency_conflict` | 409 | The key was reused for a different request |
| `choice_conflict` | 409 | A different choice is already committed for this reader and node |
| `validation_error` | 422 | Request body failed validation |
| `client_upgrade_required` | 426 | A stale chat client omitted the idempotency contract |
| `rate_limited` | 429 | Request rate exceeded |
| `payload_too_large` | 413 | Request or upload exceeded its byte limit |
| `unsupported_media_type` | 415 | Upload type is not accepted |
| `parse_error` | 422 | Novel parsing pipeline failed |
| `llm_error` | 502 | Upstream LLM API returned an error |
| `storage_error` | 502 | Object storage operation failed |
| `bad_gateway` | 502 | Upstream response was invalid |
| `service_unavailable` | 503 | A required dependency is unavailable |
| `capacity_unavailable` | 503 | Bounded local work capacity is busy; retry later |
| `internal_error` | 500 | Unexpected server error |

---

## 11. Configuration

### 11.1 Environment Variables

All services read configuration from environment variables. No configuration file format is
mandated; implementations MAY use `.env` files for local development.

Runtime variables (some are required only when their integration is enabled):

| Variable | Service | Description |
|---|---|---|
| `DATABASE_URL` | All | PostgreSQL connection string |
| `REDIS_URL` | User, Agent | Redis connection string |
| `JWT_SECRET` | User, Gateway | HMAC-SHA256 signing key, min 32 chars |
| `LLM_API_URL` | Novel, Agent, Narrative | LLM API base URL |
| `LLM_API_KEY` | User, Novel, Agent, Narrative | Optional environment override for LLM authentication |
| `LLM_MODEL` | User, Novel, Agent, Narrative | Model identifier for the environment override |
| `RUNTIME_CONFIG_KEY` | User | 32-byte hex key for encrypting web-provided credentials |
| `INTERNAL_SERVICE_TOKEN` | Gateway, User, Novel, Agent, Narrative | Authenticates internal runtime configuration and account-export reads |
| `IMAGE_GEN_API_URL` | Novel | Image generation API base URL |
| `IMAGE_GEN_API_KEY` | Novel | Image generation API key |
| `EMBEDDING_API_URL` | Agent | Embedding API base URL |
| `EMBEDDING_API_KEY` | Agent | Embedding API key |
| `EMBEDDING_MODEL` | Agent | Embedding model identifier |
| `S3_ENDPOINT` | Novel | S3-compatible endpoint URL |
| `S3_BUCKET` | Novel | Bucket name |
| `S3_ACCESS_KEY` | Novel | Access key ID |
| `S3_SECRET_KEY` | Novel | Secret access key |
| `SERVICE_PORT` | All | Port the service listens on |

### 11.2 Tunable Parameters

The following parameters SHOULD be configurable via environment variables with the listed defaults:

| Parameter | Env Variable | Default | Description |
|---|---|---|---|
| Access token expiry | `AUTH_ACCESS_TOKEN_EXPIRY` | `3600` | Seconds |
| Refresh token expiry | `AUTH_REFRESH_TOKEN_EXPIRY` | `604800` | Seconds |
| Short-term memory limit | `MEMORY_SHORT_TERM_LIMIT` | `20` | Max entries before compression |
| Compression threshold | `MEMORY_COMPRESS_THRESHOLD` | `15` | Trigger compression at this count |
| Mid-term memory limit | `MEMORY_MID_TERM_LIMIT` | `50` | Max mid-term entries |
| Context window turns | `AGENT_CONTEXT_WINDOW_TURNS` | `20` | Chat history turns in prompt |
| Long-term K | `MEMORY_LONG_TERM_K` | `5` | Top-K semantic results |
| Max file size (TXT) | `UPLOAD_MAX_TXT_BYTES` | `10485760` | 10 MB |
| Max file size (PDF) | `UPLOAD_MAX_PDF_BYTES` | `20971520` | 20 MB |
| Max paste size | `UPLOAD_MAX_PASTE_BYTES` | `5242880` | 5 MB |
| Max characters per novel | `PARSE_MAX_CHARACTERS` | `50` | Character extraction cap |

---

## 12. Database Schema Requirements

### 12.1 Extensions

The PostgreSQL database MUST have the following extensions installed:

- `uuid-ossp` — for `uuid_generate_v4()`.
- `pg_trgm` — for trigram-based fuzzy search on `novels.title` and `characters.name`.
- `vector` (pgvector) — for semantic similarity search on `character_memories.embedding`.

### 12.2 Index Requirements

Implementations MUST create the following indexes:

- `users(email)` — unique B-tree.
- `novels(user_id)` — B-tree.
- `novels(status)` — B-tree.
- `novels(title)` — GIN trigram.
- `chapters(novel_id)` — B-tree.
- `chapters(novel_id, is_key_node)` — partial B-tree where `is_key_node = true`.
- `characters(novel_id)` — B-tree.
- `characters(name)` — GIN trigram.
- `character_memories(character_id, user_id)` — B-tree.
- `character_memories(character_id, user_id, layer)` — B-tree.
- `character_memories(embedding)` — HNSW with `vector_cosine_ops`, `m=16`, `ef_construction=64`.
- `chat_messages(character_id, user_id, created_at DESC)` — B-tree.
- `chat_messages(turn_id, role)` where `turn_id IS NOT NULL` — unique B-tree.
- `chat_turns(user_id, character_id, novel_id)` where `status = in_progress` — unique B-tree.
- `world_states(user_id, novel_id)` — unique B-tree.
- `reading_progress(user_id, novel_id)` — unique B-tree.

### 12.3 Migrations

Implementations MUST apply schema changes through versioned migration files. Migrations MUST be
idempotent where possible. The initial migration MUST be applied before the first service start.
Migration 0021 MUST be one explicit transaction. Its first-adoption backfill
MUST terminally skip pre-contract completed turns before replacing the old
in-progress-only uniqueness rule with a single unresolved-turn authority rule;
replaying 0020 MUST NOT skip a new-protocol `pending` turn.
The unresolved-turn authority index is accepted only when it belongs to
`public.world_turns`, has the exact predicate/columns, and PostgreSQL reports it
unique, valid, ready, and live. Migration replay MUST replace an exact-name but
non-enforcing index, and Narrative readiness MUST fail while any of those
catalog flags is false.
Before applying migration 0021, the supported release path MUST first stop and
drain the Narrative Service world-turn producer before candidate client assets
are exposed, then stop and drain the Agent Service memory-projection consumer.
During the client contract gate, a candidate world action MUST receive a
retryable `5xx` from the stopped producer rather than a terminal validation
response from an older DTO, so its exact recovery key remains retained.
Zero-downtime world actions are not claimed. Only after both services quiesce may the repair run, so first-adoption
backfill and activation of the consumer quarantine share one clean release
boundary.

An installation whose active release script predates this drain/barrier logic
MUST use a two-stage managed upgrade: first activate a control-only release
that contains the new release script but not migration 0021, then use that
script to deploy the migration and matching services. Initial adoption through
the new script MUST select a release that already contains migration 0021.
Adoption, upgrade, restore, and rollback tooling MUST fail closed rather than
activate a pre-0020 writer after the contract is present. Crossing that barrier
requires a separately approved database compatibility procedure.
The release state MUST persist the exact target manifest immediately before
starting the migration phase and MUST clear that transition marker only after
the deployed manifest is durably promoted. A successful storage sync of the
marker MUST precede migration. Promotion and staged-candidate removal MUST be
synced before marker deletion, and that deletion MUST be synced again; a sync
failure MUST leave the marker recoverable. Restore/rollback schema barriers
MUST use this marker, not the mere presence of a downloaded candidate: a
failure before the marker is written may restore the current release, while an
interrupted marked transition MUST NOT activate an incompatible older writer.
Restore MUST roll every marked transition forward to its exact manifest. When
that manifest is not yet `current`, it MUST preserve the former `current` as
`previous` and promote only after migration replay and service health succeed;
when it is already `current`, recovery MUST retain the existing `previous`.
Whenever an upgrade has a former `current`, its rename to `previous` MUST be
successfully storage-synced before the target manifest replaces `current`, so
no crash can make the target durable while losing its rollback authority.
For both initial adoption and upgrade, the fully installed target tempfile MUST
be included in a successful storage sync before it is renamed to `current`, so
a durable directory entry cannot name an unpersisted target file.
If initial adoption is interrupted after the marker is written but before a
`current` manifest exists, restore MUST roll forward the exact marked manifest,
MUST reject a different candidate or an impossible previous release, MUST work
when the candidate file is absent, and MUST promote `current` only after the
idempotent migration and service health checks succeed.
Before normal restore or rollback writes a transition marker, it MUST remove
any unmarked staged candidate. Normal restore and rollback MUST use the same
exact-target marker and durable finalization barrier as adoption and upgrade;
rollback MUST also use the same target-tempfile and previous-manifest promotion
protocol. A legacy rollback marker MAY be recovered for compatibility, but new
rollback operations MUST NOT create a second recovery authority alongside the
schema-transition marker.

The local Docker launchers MUST stop the existing Compose project without
removing named volumes before starting the rebuilt stack. This MUST remove the
old one-shot migration container and quiesce old writers so migrations rerun
before dependent services. The portable desktop runtime MUST stop its named
embedded database, refuse startup while old service ports remain occupied,
apply its embedded migrations through 0020, and only then spawn application
services.

### 12.4 Backup, Restore, and Erasure Replay

The authoritative PostgreSQL database is recoverable under the versioned policy in
[`docs/BACKUP_RESTORE.md`](docs/BACKUP_RESTORE.md). Implementations MUST satisfy:

1. Deleting a user or a novel MUST write a durable erasure record in the same database
   transaction as the authoritative delete. The record's identifying payload is limited to
   subject type and UUIDs (including the owning user UUID for novels); its operational
   bookkeeping fields — deletion time, retained-source marker, re-queue stamp — MUST NOT carry
   source text, message content, profile data, or credentials. Records MUST be written for
   every deletion path including per-novel records under an account cascade and MUST survive
   account and novel cascades.
2. Erasure replay MUST run in the standard migration path before services start, MUST be
   idempotent across deployments, MUST remove any subject row matching an erasure record, and
   MUST re-queue the deterministic retained-source object key for an erased novel whose subject
   row no longer exists exactly once per record within a database lineage, tracked by durable
   per-record bookkeeping; a restore starts a new lineage, so it MAY cause at most one
   additional re-queue per record.
3. Every database carries exactly one lineage token: a version-4 UUID created by the standard
   migration path only when none exists; ordinary deployments preserve it, and only a restore
   regenerates it — as a fresh version-4 UUID, atomically with the restored data becoming
   reachable, recording the artifact's token as parent (recorded absent for a token-less
   artifact), so no crashed or partial restore can present the artifact's token as live.
   Backup artifacts MUST embed an erasure-record export taken from the same database snapshot
   as the dump, and record that snapshot's covered-through timestamp and the lineage token.
   The restore procedure MUST abort when the manifest token and the token inside the verified
   dump mismatch or exactly one of them is absent; a wholly token-less artifact MUST restore
   through the disaster gate, because an absent token never establishes continuation. The
   restore procedure MUST stop application writes before exporting
   erasure records, MUST treat a reachable database as live continuation only when its
   lineage token equals the artifact's, MUST replay the union of every available erasure
   source, MUST abort when sources disagree on a deletion fact of the same subject (the
   retained-source marker merges monotonically upward as bookkeeping), and MUST refuse to
   complete a restore whose residual window — from the newest source's covered-through
   timestamp to the writes-stopped or declared-failure time — is non-empty. The only
   sanctioned continuation is attest-or-erase: every restored account not already covered by
   a collected erasure record receives a retain-with-listed-novels or erase decision before
   completion; an account covered by a collected record is the pre-decided fact replay
   enforces — no decision may name it, and the restore records an automatic
   `replayed` attestation row for it. Erasure records are written and replayed for every
   erase-decided account and every unlisted novel before services start, and each decision or
   `replayed` row is durably recorded in the restored database with subject identity,
   decision, both residual-window bounds, the verified artifact digest inventory, an operator
   identity string, and a timestamp. The deployment MUST NOT contain an undecided subject and
   MUST NOT serve any subject covered by a collected or decision-written erasure record. Every restore MUST, after verification passes and before services start, rotate the
   JWT secret and delete every persisted refresh token, so no pre-restore session survives.
   Replay and attest-or-erase MUST preserve the deletion-path invariants: removing the final
   account clears the runtime configuration, and decisions that would leave retained accounts
   without an administrator require the operator to designate a retained account as
   administrator before completion, recorded with the decisions. Silent resurrection is
   prohibited in every case.
4. Backup artifacts MUST be produced by the scripted procedure, encrypted at rest, and verified
   against their integrity manifest before any restore changes data; corrupt or unverifiable
   artifacts MUST fail closed without side effects.
5. Erasure records are internal operational state: they MUST NOT appear in account export and
   MUST NOT contain source text, message content, profile data, or credentials.

---

## 13. Frontend Specification

### 13.1 Architecture

The frontend MUST follow Feature-Sliced Design (FSD) with the following layers:

```
src/
  app/        — Application bootstrap, routing, global providers, global CSS
  pages/      — Page-level composition components
  widgets/    — Self-contained UI blocks with their own data fetching
  features/   — User interaction scenarios (import, chat, choose, identity)
  entities/   — Business entity models and their API hooks
  shared/     — UI kit, API client, utility functions, type definitions
```

### 13.2 Required Pages

| Route | Component | Description |
|---|---|---|
| `/` | `HomePage` | Landing page with product introduction and login/register CTA |
| `/login`, `/register` | `LoginPage` | Sign-in and registration modes |
| `/shelf` | `ShelfPage` | User's novel library with import button and progress indicators |
| `/reader/:novelId/:chapterNum` | `ReaderPage` | Chapter reader with slide-in chat panel |
| `/characters/:novelId` | `CharactersPage` | Character gallery for a novel |
| `/settings` | `SettingsPage` | User profile and identity settings |

First-run setup renders `SetupPage` before authenticated application routing;
it is a state gate rather than a public URL.

### 13.3 Required Widgets

- `ChatPanel` — Slide-in panel with SSE-streamed character conversation. MUST support opening
  without interrupting the reader's scroll position.
- `BranchChoice` — Modal or inline card presenting narrative choices. MUST block chapter
  advancement until a choice is made.
- `CharacterCard` — Avatar, name, role badge, and "Talk" button.
- Shelf import controls — upload/paste plus parsing progress in the existing
  shelf flow; no standalone wizard component is required.
- Reader progress — chapter position in the reader header; no standalone
  progress component is required.

### 13.4 Visual Theme

The frontend MUST implement the following design tokens:

```css
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
```

The background MUST use a deep space gradient from `--color-void` to `--color-cosmos`. The reading
area MUST use `--font-reading` at a minimum size of 18px with a line height of 1.8.

### 13.5 SSE Client Contract

The frontend SSE client MUST:

1. Generate one UUID v4 per logical turn, send it in `Idempotency-Key`, and keep
   the same key for every automatic or manual retry.
2. Open a `POST` request to `/api/chat/:characterId/stream` with the user message in the body.
3. Incrementally decode UTF-8 and SSE frames across arbitrary network chunks.
4. Parse `event: delta` JSON and append `data.content` to the displayed message.
5. Finalize only on `event: done` whose JSON has the matching `turn_id` and
   `committed: true`; a bare EOF is an error.
6. On `event: error`, discard partial output, display the error, and retain the
   key for an explicit retry.
7. Retry network, 5xx, and `turn_in_progress` failures with exponential backoff
   (max 3 retries), clearing partial output before each attempt.

During the rolling upgrade the client MUST also accept the previous unnamed
delta frames and empty `done` event. This compatibility parser does not weaken
the new protocol: once a named v2 event is observed, a structured commit
acknowledgement is required.

The Agent Service MUST derive the user, current chapter, and reader identity from authenticated
server-side state. Browser-supplied context is never authoritative. Requests containing the retired
body `user_id` marker MUST fail before LLM invocation with HTTP 426 and error code
`client_upgrade_required`; the client must refresh before retrying.

---

## 14. Observability

### 14.1 Structured Logging

All services MUST emit structured JSON logs to stdout. Each log entry MUST include:

- `timestamp` (ISO 8601)
- `level` (`debug` | `info` | `warn` | `error`)
- `service` (service name)
- `message` (string)
- `trace_id` (string or null) — propagated from the `X-Trace-Id` request header.

### 14.2 Health Endpoints

Each downstream service MUST expose `GET /health` as a process-liveness probe
and `GET /ready` as a dependency-readiness probe. Liveness returns HTTP 200
while the process can serve requests; readiness returns HTTP 503 whenever a
required dependency is unavailable.

The Gateway MUST expose `GET /live` for its own liveness. Its `/health` and
`/ready` endpoints MUST aggregate downstream readiness and return HTTP 503 if
any required service is unavailable. Gateway probe and metrics endpoints MUST
not require application authentication or consume business rate-limit capacity.
Deployments MAY restrict `/metrics` to their internal monitoring network.

### 14.3 Metrics

Each LLM-calling service exposes Prometheus-compatible metrics at its internal
`GET /metrics`. The public Nginx route MUST remain unavailable. The bounded
`llm-observability-v1` contract includes:

- `novelworld_llm_requests_started_total` and `novelworld_llm_requests_total`
- `novelworld_llm_attempts_total` and `novelworld_llm_retries_total`
- request, attempt, stream-setup, and first-token latency histograms
- `novelworld_llm_usage_reports_total`, `novelworld_llm_tokens_total`, and
  `novelworld_llm_billable_tokens_total`
- actual and static per-operation output-token ceilings

LLM labels MUST come from bounded configuration and MUST NOT include prompts,
raw URLs/errors, secrets, principals, or resource identifiers. Cost is derived
at query time from billable token classes and operator-maintained current
provider pricing. User-facing presentation uses CNY when the UI language starts
with `zh`, and USD otherwise; exchange rates are configuration, not embedded
constants. Billable-token series identify the credential that actually paid for
the request only through a stable one-way SHA-256 fingerprint; raw API keys and
user identifiers MUST NOT appear in metrics or Prometheus queries. Administrators
may view usage for the platform credential. A non-administrator may view usage
only after configuring a personal credential, and only for that credential;
falling back to the platform credential does not grant access to platform usage.
The checked-in versioned release policy is the source of truth for H3 budgets.

---

## 15. Security Requirements

- All inter-service communication MUST occur on an internal network not exposed to the public
  internet.
- The deployment ingress (Nginx in the current Compose profile) MUST be the
  only component with a host-published port, and application traffic MUST pass
  through the Gateway behind it.
- JWT secrets MUST be at least 32 characters and MUST NOT be committed to version control.
- Passwords MUST be hashed with bcrypt at cost factor 12 or higher.
- File uploads MUST be validated for MIME type and size before storage.
- Source text and user input passed to an LLM MUST be explicitly delimited and treated as untrusted.
  Prompt wording MUST NOT be relied on for authorization or commit validity; model-derived
  transitions require bounded schema and domain validation.
- Object storage keys MUST be server-generated under the managed
  `source-files/<user_id>/<novel_id>` namespace; uploaded filenames MUST NOT
  influence a key.
- Database queries MUST use parameterized statements; string interpolation into SQL is forbidden.

---

## 16. Implementation Guidance

This specification does not duplicate implementation order, dependency advice,
test inventories, or prompt templates. [`AGENTS.md`](./AGENTS.md) owns current
engineering constraints; runtime code owns the exact prompts, validators, and
tests. Roadmap issues define the next approved implementation slice.

---

## Appendix A: Glossary

| Term | Definition |
|---|---|
| Agent | An AI persona derived from a novel character, capable of conversing with readers. |
| Branch node | A chapter that contains a narrative choice point. |
| Canon mode | Deviation mode where the agent strictly follows the source text. |
| Compression pipeline | The process of summarizing short-term memories into mid-term memories. |
| Deviation mode | Reader-configurable setting controlling how strictly the agent adheres to canon. |
| Memory pyramid | The four-layer hierarchical memory system (short, mid, long, permanent). |
| Reader identity | The persona the reader adopts when entering the novel world. |
| World state | The accumulated record of a reader's choices and their consequences. |
