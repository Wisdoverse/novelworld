# NovelWorld Service Specification

Status: Draft v1 (language-agnostic)

Purpose: Define a platform that transforms any novel into an interactive world where readers engage
with AI-driven character agents, influence narrative branches, and maintain persistent memory across
sessions.

## Normative Language

The key words `MUST`, `MUST NOT`, `REQUIRED`, `SHOULD`, `SHOULD NOT`, `RECOMMENDED`, `MAY`, and
`OPTIONAL` in this document are to be interpreted as described in RFC 2119.

`Implementation-defined` means the behavior is part of the implementation contract, but this
specification does not prescribe one universal policy. Implementations MUST document the selected
behavior.

`LLM` refers to any Large Language Model reachable via an OpenAI-compatible chat completions API.

---

## 1. Problem Statement

NovelWorld is a platform that ingests arbitrary novel text, extracts its characters and world
structure via LLM analysis, and exposes each character as a stateful AI agent that readers can
converse with. The platform solves five problems:

- It turns static novel text into a living, interactive world without requiring author involvement.
- It isolates each reader's experience so that one reader's choices and conversations do not affect
  another reader's world state.
- It maintains per-reader, per-character memory across sessions using a four-layer memory pyramid,
  so character relationships evolve naturally over time.
- It surfaces key narrative branch points and lets readers make choices that diverge the story,
  with the LLM generating canon-consistent consequences.
- It generates character avatars from textual appearance descriptions so every character has a
  visual identity without manual artwork.

Important boundary:

- NovelWorld is a reader-facing interactive platform, not an authoring tool.
- The platform does not modify the source novel text; it only generates derivative interactive
  content layered on top of it.
- A reader's choices affect only their own world state, not the canonical novel text.

---

## 2. Goals and Non-Goals

### 2.1 Goals

- Accept novel text via file upload (TXT, PDF) or direct paste and parse it into chapters and
  characters without manual annotation.
- Generate a character agent for every extracted character, with personality, background, and
  speaking style derived from the source text.
- Maintain a four-layer memory pyramid per reader per character: short-term, mid-term, long-term,
  and permanent layers with automatic compression and semantic retrieval.
- Stream character dialogue responses via Server-Sent Events (SSE) so readers see text appear
  progressively.
- Present branch choice nodes at key chapters and persist the reader's selections in a world state
  document.
- Allow readers to adopt either their own identity or a character's identity when entering the
  world.
- Generate character avatar images from appearance descriptions using an image generation API.
- Enforce per-reader memory isolation so no reader can observe another reader's conversation
  history or world state.
- Provide a user account system with JWT-based authentication and bcrypt password hashing.

### 2.2 Non-Goals

- Modifying or annotating the source novel text.
- Multi-reader shared world state or collaborative sessions.
- Real-time multiplayer interaction between readers.
- Authoring tools for creating original novels.
- Prescribing a specific LLM provider; any OpenAI-compatible endpoint is acceptable.
- Built-in content moderation beyond what the configured LLM provides.

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
   - Triggers avatar generation for extracted characters.
   - Stores parsed artefacts in the database and novel files in object storage.

4. `Agent Service`
   - Manages character agent sessions.
   - Builds LLM prompts from character profile, world state, and memory layers.
   - Streams LLM responses back to the caller via SSE.
   - Writes new memories after each conversation turn.
   - Compresses short-term memories when the layer exceeds its configured threshold.

5. `Narrative Service`
   - Identifies key branch nodes in chapters.
   - Presents choice options to readers.
   - Persists reader choices in the world state document.
   - Generates LLM-derived consequence text for each choice.

6. `Database` (PostgreSQL 18)
   - Single source of truth for all structured data.
   - Uses `pgvector` extension for semantic memory retrieval.

7. `Object Storage` (S3-compatible)
   - Stores uploaded novel files and generated avatar images.
   - All file references in the database are storage keys, not local paths.

8. `Cache` (Redis)
   - Stores short-term memory entries for fast access during active conversations.
   - Caches parsed chapter content to reduce database load.

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

- An LLM reachable via an OpenAI-compatible chat completions API (`/v1/chat/completions`).
- An image generation API reachable via an OpenAI-compatible endpoint (`/v1/images/generations`).
- An embedding API reachable via an OpenAI-compatible endpoint (`/v1/embeddings`).
- PostgreSQL 18 with the `pgvector` and `uuid-ossp` extensions installed.
- Redis 7 or later.
- An S3-compatible object storage endpoint.

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
  - `ready`: all chapters and characters extracted.
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

- `id` (UUID v4)
- `character_id` (UUID, foreign key → Character)
- `user_id` (UUID, foreign key → User)
- `layer` (enum: `short` | `mid` | `long` | `permanent`)
- `content` (text)
  - Natural language description of the memory.
- `importance` (integer, 1–10)
  - Higher values survive compression longer.
- `embedding` (vector(1536) or null)
  - Semantic embedding for similarity search. REQUIRED for `long` and `permanent` layers.
- `access_count` (integer)
  - Incremented each time this memory is retrieved during prompt construction.
- `last_accessed` (timestamptz or null)
- `expires_at` (timestamptz or null)
  - Set only for `short` layer entries. Null means no expiry.
- `created_at` (timestamptz)

#### 4.1.6 ChatMessage

One turn in a conversation between a reader and a character agent.

Fields:

- `id` (UUID v4)
- `character_id` (UUID, foreign key → Character)
- `user_id` (UUID, foreign key → User)
- `novel_id` (UUID, foreign key → Novel)
- `role` (enum: `user` | `character`)
- `content` (text)
- `chapter_num` (integer or null)
  - The reader's current chapter at the time of the message.
- `created_at` (timestamptz)

#### 4.1.7 NarrativeNode

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

#### 4.1.8 UserChoice

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

#### 4.1.9 WorldState

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

#### 4.1.10 ReadingProgress

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

- All UUIDs MUST be version 4 unless otherwise specified.
- All timestamps MUST be stored as `TIMESTAMPTZ` (UTC-normalized).
- `chapter_number` is 1-indexed throughout the system.
- Character `name` comparisons for deduplication MUST be case-insensitive.
- `deviation_mode` defaults to `canon` at all levels unless explicitly overridden by the reader.

---

## 5. Novel Ingestion Pipeline

### 5.1 File Acceptance

The Novel Service MUST accept:

- Plain text files (`.txt`) up to 10 MB.
- PDF files (`.pdf`) up to 20 MB.
- Direct text paste payloads up to 5 MB (UTF-8 encoded JSON string body).

On receipt, the service MUST:

1. Store the raw file in object storage under the key `novels/<user_id>/<novel_id>/source.<ext>`.
2. Create a Novel record with `status = pending`.
3. Return the novel ID to the caller immediately.
4. Enqueue the parsing pipeline asynchronously.

### 5.2 Parsing Pipeline

The parsing pipeline runs asynchronously after file acceptance. It MUST:

1. Set `status = parsing`.
2. Extract plain text from the source file (PDF text extraction, TXT read).
3. Split the text into chapters using the Chapter Splitter (see §5.3).
4. For each chapter, store a Chapter record with `content` and `word_count`.
5. Extract characters using the Character Extractor (see §5.4).
6. Generate a world summary using the World Summarizer (see §5.5).
7. Identify key narrative nodes using the Node Detector (see §5.6).
8. Enqueue avatar generation for each character (see §5.7).
9. Set `status = ready` and `total_chapters = <count>`.

On any unrecoverable error, the pipeline MUST set `status = error` and store the error message in
`parse_error`.

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
[
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
]
```

The extractor MUST:

- Deduplicate characters by name (case-insensitive) and merge aliases.
- Extract at least all characters who appear in more than one chapter.
- Return at most 50 characters per novel to bound LLM cost.

### 5.5 World Summarizer

Input: novel title, author, and the first 4000 tokens of the novel text.

The summarizer MUST invoke the LLM and request a world summary covering:

- Setting (time period, geography, society).
- Major factions or groups.
- Core conflict.
- Unique world rules (magic systems, technology, etc.) if applicable.

The summary MUST be stored in `novels.world_summary` and MUST NOT exceed 2000 characters.

### 5.6 Node Detector

Input: chapter content and chapter number.

For each chapter, the Node Detector MUST invoke the LLM to determine whether the chapter contains
a narrative branch point. The LLM response MUST use a structured schema:

```json
{
  "is_key_node": "boolean",
  "description": "string or null",
  "choices": [
    { "index": 0, "text": "string", "consequence_hint": "string or null" }
  ]
}
```

If `is_key_node = true`, the service MUST:

- Set `chapters.is_key_node = true`.
- Store the `description` in `chapters.key_node_description`.
- Create a `NarrativeNode` record with the returned choices.

The number of choices per node MUST be between 2 and 4 inclusive.

### 5.7 Avatar Generation

For each extracted character with a non-null `appearance` field:

1. Set `characters.avatar_status = generating`.
2. Construct an image generation prompt from the `appearance` field, prefixed with the instruction:
   `"Character portrait illustration, detailed, fantasy art style: "`.
3. Invoke the image generation API with size `512x512` and `n=1`.
4. Upload the returned image to object storage under the key
   `avatars/<novel_id>/<character_id>.png`.
5. Set `characters.avatar_url` to the storage URL and `characters.avatar_status = ready`.
6. On failure, set `characters.avatar_status = error`. Avatar failure MUST NOT block the novel
   from reaching `status = ready`.

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
| `short` | Redis + PostgreSQL | Configurable (default 20) | Recency | Configurable TTL |
| `mid` | PostgreSQL | Configurable (default 50) | Recency + Importance | None |
| `long` | PostgreSQL + pgvector | Unbounded | Semantic similarity | None |
| `permanent` | PostgreSQL + pgvector | Unbounded | Semantic similarity + Importance | Never |

#### 6.2.1 Short-Term Layer

- Contains raw conversation turns from the current and recent sessions.
- Stored in Redis with a TTL equal to `memory.short_term_ttl_seconds` (default: 86400).
- Also persisted to PostgreSQL for durability.
- When the count of short-term entries for a `(character_id, user_id)` pair exceeds
  `memory.compress_threshold` (default: 15), the Compression Pipeline (§6.3) MUST be triggered.

#### 6.2.2 Mid-Term Layer

- Contains compressed summaries of past conversation sessions.
- Created by the Compression Pipeline from short-term entries.
- Retrieved by recency and importance score during prompt construction.
- When the count of mid-term entries exceeds `memory.mid_term_limit` (default: 50), the oldest
  low-importance entries are promoted to long-term or discarded.

#### 6.2.3 Long-Term Layer

- Contains semantically indexed memories of significant events and relationship milestones.
- Each entry MUST have an `embedding` vector.
- Retrieved via cosine similarity search using `pgvector`.

#### 6.2.4 Permanent Layer

- Contains immutable facts: the reader's name, major choices, and critical relationship events.
- Entries in this layer MUST NOT be deleted or compressed.
- Each entry MUST have an `embedding` vector.
- Retrieved via cosine similarity search, weighted by `importance`.

### 6.3 Compression Pipeline

Triggered when `short` layer count exceeds `memory.compress_threshold`.

Steps:

1. Retrieve all `short` layer entries for the `(character_id, user_id)` pair, ordered by
   `created_at` ascending.
2. Send the entries to the LLM with a prompt requesting a concise summary of the key events,
   emotional tone, and relationship developments.
3. Store the summary as a new `mid` layer entry with `importance` derived from the LLM's assessment
   (1–10).
4. Delete the compressed `short` layer entries from both Redis and PostgreSQL.
5. If the summary contains any permanent facts (character name, major choices), extract them and
   store as `permanent` layer entries with embeddings.

### 6.4 Prompt Construction

Before invoking the LLM for a conversation turn, the Agent Service MUST construct the prompt as
follows:

1. **System prompt**: character identity block (§6.1).
2. **Memory block**: retrieved memories formatted as a `<memories>` XML block:
   - All `permanent` layer entries (always included).
   - Top-K `long` layer entries by cosine similarity to the current user message (K = 5).
   - Most recent N `mid` layer entries (N = 3).
   - Most recent M `short` layer entries (M = `memory.short_term_limit`, default 10).
3. **World state block**: the reader's `WorldState.state` formatted as a `<world_state>` XML block.
4. **Conversation history**: the last `agent.context_window_turns` (default: 20) `ChatMessage`
   records for this `(character_id, user_id)` pair, in chronological order.
5. **User message**: the current reader input.

The total prompt MUST NOT exceed the LLM's context window. If it does, the Agent Service MUST
truncate mid-term and long-term memory blocks first, then short-term blocks, preserving the most
recent entries.

### 6.5 Streaming Response

The Agent Service MUST stream the LLM response to the caller via SSE.

SSE event format:

```
event: delta
data: {"content": "<token>"}

event: done
data: {"usage": {"input_tokens": N, "output_tokens": M}}
```

On error:

```
event: error
data: {"code": "<error_code>", "message": "<human-readable message>"}
```

After the stream completes, the Agent Service MUST:

1. Store the complete response as a `ChatMessage` with `role = character`.
2. Store the user's input as a `ChatMessage` with `role = user` (if not already stored).
3. Create a new `short` layer `CharacterMemory` entry summarizing the turn.
4. Trigger the Compression Pipeline if the short-term threshold is exceeded.
5. Update `WorldState.state.relationships` if the LLM response implies a relationship change
   (implementation-defined heuristic).

---

## 7. Narrative Branch System

### 7.1 Node Presentation

When a reader advances to a chapter where `is_key_node = true`, the Narrative Service MUST:

1. Check whether the reader has already made a choice at this node by querying `UserChoice` for
   `(user_id, node_id)`.
2. If a choice exists, return the existing choice and consequence without re-presenting options.
3. If no choice exists, return the `NarrativeNode` with its `description` and `choices` array.

### 7.2 Choice Submission

When a reader submits a choice:

1. Validate that `choice_index` is within the bounds of `NarrativeNode.choices`.
2. Store a `UserChoice` record.
3. Invoke the LLM to generate a consequence narrative (see §7.3).
4. Update `WorldState.state.choices` by appending the new choice.
5. Optionally update `WorldState.state.world_events` if the consequence implies a world-level event.
6. Return the consequence text to the reader.

### 7.3 Consequence Generation

Input: novel world summary, chapter content, choice text, and the reader's prior choices from
`WorldState`.

The LLM MUST be prompted to generate a consequence narrative that:

- Is consistent with the novel's world and tone.
- Acknowledges the reader's prior choices where relevant.
- Is between 100 and 400 words.
- Ends with a clear transition to the next chapter.

The consequence MUST be stored in `UserChoice.consequence`.

### 7.4 World State Consistency

The Narrative Service MUST ensure that:

- `WorldState` is created on first access for a `(user_id, novel_id)` pair if it does not exist.
- All mutations to `WorldState.state` are atomic (use database transactions or optimistic locking).
- The `relationships` map in `WorldState.state` uses `character_id` (UUID string) as keys.

---

## 8. Reader Identity System

### 8.1 Identity Types

Readers MAY choose one of two identity modes:

- `self`: The reader enters the world as themselves. The agent system prompt uses the reader's
  `reader_identity` name and addresses them in second person.
- `character`: The reader adopts a character's identity. The agent system prompt acknowledges the
  reader as that character and adjusts the dynamic accordingly.

### 8.2 Identity Constraints

- If `reader_identity_type = character`, `reader_character_id` MUST reference a Character that
  belongs to the same novel.
- A reader MUST NOT adopt the identity of the character they are currently conversing with.
- Identity changes take effect immediately for new conversation turns; they do not retroactively
  alter existing `ChatMessage` records.

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

### 9.4 Authorization Rules

- All endpoints except `POST /api/auth/register` and `POST /api/auth/login` REQUIRE a valid JWT.
- A user MAY only access novels, characters, memories, and world states that belong to their own
  `user_id`.
- Admin users MAY access all resources.
- The Gateway MUST reject requests with expired or invalid JWTs with HTTP 401.

---

## 10. API Contract

All endpoints are prefixed with `/api/`. The Gateway routes requests to the appropriate downstream
service.

### 10.1 Authentication Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| POST | `/api/auth/register` | User | None | Register new user |
| POST | `/api/auth/login` | User | None | Login, returns tokens |
| POST | `/api/auth/refresh` | User | Refresh token | Issue new access token |
| GET | `/api/auth/me` | User | JWT | Current user profile |
| POST | `/api/auth/logout` | User | JWT | Invalidate refresh token |

### 10.2 Novel Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| GET | `/api/novels` | Novel | JWT | List user's novels |
| POST | `/api/novels` | Novel | JWT | Import novel (text paste) |
| POST | `/api/novels/upload` | Novel | JWT | Upload TXT or PDF file |
| GET | `/api/novels/:id` | Novel | JWT | Novel detail |
| GET | `/api/novels/:id/status` | Novel | JWT | Parse status (poll) |
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
| POST | `/api/characters/:id/generate-avatar` | Novel | JWT | Trigger avatar regeneration |

### 10.5 Agent Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| POST | `/api/chat/:characterId/stream` | Agent | JWT | Stream conversation turn (SSE) |
| GET | `/api/chat/:characterId/history` | Agent | JWT | Conversation history |
| DELETE | `/api/chat/:characterId/history` | Agent | JWT | Clear conversation history |
| GET | `/api/chat/:characterId/memories` | Agent | JWT | Memory layer summary |

### 10.6 Narrative Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| GET | `/api/narrative/:novelId/:chapter` | Narrative | JWT | Get branch node for chapter |
| POST | `/api/narrative/choose` | Narrative | JWT | Submit choice |
| GET | `/api/narrative/:novelId/world-state` | Narrative | JWT | Reader's world state |

### 10.7 Progress Endpoints

| Method | Path | Service | Auth | Description |
|---|---|---|---|---|
| GET | `/api/progress/:novelId` | Novel | JWT | Reading progress |
| PUT | `/api/progress/:novelId` | Novel | JWT | Update chapter position |
| PUT | `/api/progress/:novelId/identity` | Novel | JWT | Set reader identity |

### 10.8 Error Response Format

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
| `validation_error` | 422 | Request body failed validation |
| `parse_error` | 422 | Novel parsing pipeline failed |
| `llm_error` | 502 | Upstream LLM API returned an error |
| `storage_error` | 502 | Object storage operation failed |
| `internal_error` | 500 | Unexpected server error |

---

## 11. Configuration

### 11.1 Environment Variables

All services read configuration from environment variables. No configuration file format is
mandated; implementations MAY use `.env` files for local development.

Required variables:

| Variable | Service | Description |
|---|---|---|
| `DATABASE_URL` | All | PostgreSQL connection string |
| `REDIS_URL` | User, Agent | Redis connection string |
| `JWT_SECRET` | User, Gateway | HMAC-SHA256 signing key, min 32 chars |
| `LLM_API_URL` | Novel, Agent, Narrative | LLM API base URL |
| `LLM_API_KEY` | Novel, Agent, Narrative | LLM API authentication key |
| `LLM_MODEL` | Novel, Agent, Narrative | Model identifier |
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
- `world_states(user_id, novel_id)` — unique B-tree.
- `reading_progress(user_id, novel_id)` — unique B-tree.

### 12.3 Migrations

Implementations MUST apply schema changes through versioned migration files. Migrations MUST be
idempotent where possible. The initial migration MUST be applied before the first service start.

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
| `/shelf` | `ShelfPage` | User's novel library with import button and progress indicators |
| `/import` | `ImportPage` | Novel import form (file upload or text paste) |
| `/reader/:novelId/:chapterNum` | `ReaderPage` | Chapter reader with slide-in chat panel |
| `/characters/:novelId` | `CharactersPage` | Character gallery for a novel |
| `/settings` | `SettingsPage` | User profile and identity settings |

### 13.3 Required Widgets

- `ChatPanel` — Slide-in panel with SSE-streamed character conversation. MUST support opening
  without interrupting the reader's scroll position.
- `BranchChoice` — Modal or inline card presenting narrative choices. MUST block chapter
  advancement until a choice is made.
- `CharacterCard` — Avatar, name, role badge, and "Talk" button.
- `ImportWizard` — Multi-step form: upload/paste → parsing progress → character review.
- `ProgressBar` — Chapter progress indicator in the reader header.

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

1. Open a `POST` request to `/api/chat/:characterId/stream` with the user message in the body.
2. Parse `event: delta` events and append `data.content` to the displayed message.
3. On `event: done`, finalize the message and display token usage if desired.
4. On `event: error`, display the error message and close the stream.
5. Implement a reconnect strategy with exponential backoff (max 3 retries) on network errors.

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

Each service MUST expose `GET /health` returning HTTP 200 with:

```json
{ "status": "ok", "service": "<service-name>", "version": "<semver>" }
```

The Gateway's `/health` endpoint MUST aggregate health from all downstream services and return HTTP
503 if any service is unhealthy.

### 14.3 Metrics (OPTIONAL)

Implementations MAY expose Prometheus-compatible metrics at `GET /metrics`. Recommended metrics:

- `novelworld_llm_requests_total` (counter, labels: `service`, `model`, `status`)
- `novelworld_llm_tokens_total` (counter, labels: `service`, `model`, `type`)
- `novelworld_memory_compression_total` (counter, labels: `character_id`)
- `novelworld_active_streams` (gauge)
- `novelworld_novel_parse_duration_seconds` (histogram)

---

## 15. Security Requirements

- All inter-service communication MUST occur on an internal network not exposed to the public
  internet.
- The Gateway MUST be the only service with a public-facing port.
- JWT secrets MUST be at least 32 characters and MUST NOT be committed to version control.
- Passwords MUST be hashed with bcrypt at cost factor 12 or higher.
- File uploads MUST be validated for MIME type and size before storage.
- LLM prompts MUST NOT include raw user-supplied content without sanitization to prevent prompt
  injection.
- Object storage keys MUST be scoped to `<user_id>/` prefixes to prevent cross-user access.
- Database queries MUST use parameterized statements; string interpolation into SQL is forbidden.

---

## 16. Implementation Notes for Coding Agents

This section provides non-normative guidance for coding agents implementing this specification.

### 16.1 Recommended Implementation Order

1. Database schema and migrations (§12).
2. User Service: registration, login, JWT (§9).
3. Gateway: JWT middleware, routing skeleton (§3.1).
4. Novel Service: file upload, chapter splitting, character extraction (§5).
5. Agent Service: prompt construction, SSE streaming, short-term memory (§6.4, §6.5).
6. Narrative Service: node detection, choice submission, world state (§7).
7. Memory compression pipeline (§6.3).
8. Long-term and permanent memory with pgvector (§6.2.3, §6.2.4).
9. Avatar generation (§5.7).
10. Frontend: FSD structure, pages, widgets, SSE client (§13).

### 16.2 LLM Prompt Design

All LLM calls that require structured output MUST use `response_format: { type: "json_schema" }`
when the model supports it, or instruct the model to return only valid JSON in the system prompt
when it does not. Implementations MUST validate the returned JSON against the expected schema and
retry once on parse failure before returning an error.

### 16.3 Rust Implementation Notes

When implementing in Rust:

- Use `axum` for HTTP servers and SSE.
- Use `sqlx` with the `postgres` feature for database access.
- Use `redis` crate for Redis operations.
- Use `aws-sdk-s3` for object storage.
- Use `reqwest` for LLM API calls.
- Use `serde_json` for JSON serialization.
- Use `jsonwebtoken` for JWT operations.
- Use `bcrypt` crate for password hashing.
- Structure each service as a Cargo workspace member.
- Use `tokio` as the async runtime.
- Implement the four-layer memory pyramid as a `MemoryManager` struct with methods:
  `add_short_term`, `compress`, `retrieve_for_prompt`, `add_permanent`.

### 16.4 Testing Requirements

Implementations MUST include:

- Unit tests for the Chapter Splitter covering at least: Chinese chapter headers, English chapter
  headers, standalone number headers, and the LLM fallback path.
- Unit tests for the Memory Manager covering: short-term insertion, compression trigger, and
  prompt retrieval ordering.
- Integration tests for the authentication flow: register, login, token refresh, and logout.
- Integration tests for the novel parsing pipeline using a short sample text.

---

## Appendix A: Sample LLM Prompts

### A.1 Character Extraction Prompt

```
You are analyzing a novel to extract its characters. Return a JSON array of character objects.
Each object must have these fields:
- name (string): the character's primary name
- aliases (array of strings): alternative names used in the text
- role (string): one of "protagonist", "antagonist", "supporting", "minor"
- description (string): 1-2 sentence narrative description
- personality (string): key personality traits, comma-separated
- background (string): brief backstory as revealed in the text
- speaking_style (string): how the character speaks (formal/casual, verbose/terse, etc.)
- appearance (string): physical description for portrait generation
- first_appearance_chapter (integer or null): chapter number of first appearance

Extract all characters who appear in more than one scene. Return at most 50 characters.
Return only valid JSON, no markdown fences.
```

### A.2 Character Agent System Prompt Template

```
You are {{character.name}}, a character from the novel "{{novel.title}}".

## Your Identity
Role: {{character.role}}
Personality: {{character.personality}}
Background: {{character.background}}
Speaking style: {{character.speaking_style}}

## World Context
{{novel.world_summary}}

## Current Situation
The reader is at chapter {{reader.current_chapter}} of {{novel.total_chapters}}.
{{#if reader.identity_type == "self"}}
You are speaking with {{reader.identity}}, a visitor to your world.
{{else}}
You are speaking with someone who has taken on the role of {{reader.identity}}.
{{/if}}

## Constraints
- Stay in character at all times.
- Do not reveal events from chapters after chapter {{reader.current_chapter}}.
- Speak in your established voice and style.
- Deviation mode: {{reader.deviation_mode}}

<memories>
{{memories}}
</memories>

<world_state>
{{world_state}}
</world_state>
```

### A.3 Memory Compression Prompt

```
The following is a conversation history between a reader and the character {{character.name}}.
Summarize the key events, emotional developments, and relationship changes in 2-3 sentences.
Focus on information that would be important for the character to remember in future conversations.
Rate the importance of this summary on a scale of 1-10.
Return JSON: { "summary": "...", "importance": N }
Return only valid JSON, no markdown fences.

Conversation:
{{conversation_turns}}
```

---

## Appendix B: Glossary

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
