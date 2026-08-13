# NovelWorld Long-Term Engineering Roadmap

This roadmap orders work by risk and evidence, not by feature count or calendar
promises. A horizon starts only when the previous horizon's exit criteria are
measured and met.

## Delivery status

Last reviewed: 2026-08-13. "Complete" means merged to `main` with required CI
green; it does not claim that an operator has deployed the release.

| Horizon | Status | Evidence / next gate |
|---|---|---|
| 0 — Trust boundary | Complete | [PR #72](https://github.com/schorsch888/novelworld/pull/72) merged as `6b62c44`; ownership, principal, and boundary checks are in required CI. |
| 1 — Reliability | Complete | PR #72 completed atomic chat/narrative persistence, durable setup, migrations, readiness, and release checks; CI passed 5/5. |
| 2 — Core reader loop | Complete | PR #76 gates upload -> parse -> read -> chat -> choose -> resume across a full process restart; all required CI checks passed. |
| 3 — Canonical world model | In progress | PRs [#79](https://github.com/schorsch888/novelworld/pull/79), [#84](https://github.com/schorsch888/novelworld/pull/84), [#86](https://github.com/schorsch888/novelworld/pull/86), [#88](https://github.com/schorsch888/novelworld/pull/88), and [#90](https://github.com/schorsch888/novelworld/pull/90) establish source-complete canon, validated transitions, exact replay, player-scoped generated chapters, the offline quality gate, and the durable original `PlayerEntity`; [PR #92](https://github.com/schorsch888/novelworld/pull/92) merged provider observability and release budgets, and [PR #94](https://github.com/schorsch888/novelworld/pull/94) merged complete deletion and explicit retention. [Issue #95](https://github.com/schorsch888/novelworld/issues/95) tracks the account-export merge gate; the threat-model gate remains. |
| 4 — Living open world | Queued | Starts after the canonical model and structured transition quality gates pass. |
| 5 — Measured scale | Queued | Starts only for a named SLO or measured bottleneck. |

## Product and system invariants

These remain true through every horizon:

1. Authenticated identity comes only from Gateway-verified JWT claims. Request
   bodies, paths, and query strings never choose the acting user.
2. A user can access only novels and derived resources they own. Cross-service
   calls carry that identity and fail closed when ownership cannot be proven.
3. Domain code has no infrastructure or HTTP dependencies. Service data is
   reached through its owning service, not another service's tables.
4. A conversation turn is durable only after both messages are stored. A
   narrative choice is idempotent and cannot be attached to a different novel.
5. Spoiler boundaries use server-side reading progress; the browser cannot move
   the model's knowledge boundary independently of persisted progress.
6. Every externally visible contract has a regression check. Required CI gates
   cannot be allowed to fail.
7. Changes are observable and reversible before they are scaled.
8. Canon is an immutable, source-backed baseline. Reader choices and generated
   events are stored as overlays; generated prose never silently rewrites
   extracted canon facts.

## Horizon 0: restore the trust boundary

Baseline evidence showed resource endpoints that trusted caller-supplied
`user_id` and novel reads that did not verify ownership. PR #72 closed these
gaps before feature work proceeds.

- Derive the acting user from `X-User-Id` on every downstream endpoint.
- Enforce novel ownership before returning novels, chapters, characters,
  relationships, parse status, progress, memories, or world state.
- Propagate the acting user on agent-service and narrative-service calls to
  novel-service.
- Reject character/novel and narrative-node/novel mismatches.
- Remove caller-supplied identity fields from browser and API request contracts.
- Put token issuance and chat completion behind domain ports; application code
  must not import concrete JWT or LLM adapters.
- Add focused regression checks for the principal and ownership invariants.

Exit criteria: the protected-resource matrix is owner-scoped, forged identity
fields are rejected, targeted tests pass, and workspace checks introduce no new
failure.

## Horizon 1: make the current product reliable

- Replace string-matched application errors with typed domain/application
  errors and the specified stable API error envelope.
- Bound pagination, message size, chapter position, and all LLM prompt inputs at
  trust boundaries.
- Make chat persistence and narrative choice/world-state mutation atomic or
  explicitly recoverable and idempotent.
- Make setup state durable and truthful across restarts; never imply that an API
  key was saved when it was not.
- Repair the existing formatting baseline, make formatting and Clippy required
  CI gates, and run real PostgreSQL/Redis integration tests in CI.
- Add readiness checks that verify dependencies separately from liveness.

Exit criteria: required CI is green, restart/failure-path tests pass, and the
documented API matches runtime behavior.

## Horizon 2: complete the core reader loop

- Accept bounded TXT, EPUB, and PDF sources through a domain-owned extraction
  port, preserving EPUB spine order and rejecting malformed or unsafe archives.
- Route the public journey entry to registration and authenticated readers to
  their shelf, with navigation covered by a UI regression test.
- Replace the reader's mock branch node with the narrative-service contract.
- Connect the existing authenticated progress, reader-identity, history, and
  memory contracts into a restart-safe resume flow.
- Complete memory controls, narrative consequence rendering, and accessible
  loading/error/retry states in the existing FSD slices.
- Move ingestion to durable background work with explicit status transitions,
  retry policy, cancellation, and idempotency.
- Keep provider settings administrator-only and allowlisted. Structured DeepSeek
  extraction uses non-thinking JSON mode; optional conversational thinking uses
  the Responses API and preserves semantic SSE completion/failure events.
- Arrange repeated extraction instructions as stable prompt prefixes so
  DeepSeek's automatic context cache can reuse them; expose cache-hit tokens in
  the provider cost metrics rather than claiming guaranteed cache hits.
- Store uploaded source files only when retention/reprocessing requirements are
  defined; otherwise keep direct extraction and document the limit.

Exit criteria: upload -> parse -> read -> chat -> choose -> resume works after a
process restart and is covered by an end-to-end test.

## Horizon 3: build the canonical world model and its quality gates

A completed source novel is treated as a finite mainline from which a reusable
world can be derived. This horizon builds that durable source of truth before
adding unrestricted exploration.

- Extract a versioned `CanonStoryModel` containing story arcs, ordered events,
  locations, factions, world rules, character goals, relationships, deaths,
  unresolved threads, and the canonical ending snapshot. Every item retains
  source chapter provenance and confidence.
- Represent the mainline as a causal event graph, not only chapter summaries.
  A player timeline references a canonical checkpoint and stores append-only
  deviations without copying or mutating the canonical graph.
- Create a durable `PlayerEntity` that is not part of source canon. It carries
  the user's chosen name, background, capabilities, location, inventory,
  relationships, faction standing, and discovered knowledge into the world.
- Replace prose-only consequences with a validated structured transition
  (`events`, `relationship_changes`, `location_changes`, `thread_changes`, and
  rendered narrative). Apply the transition and the user choice atomically.
- Treat the first player choice as a causal boundary: preserve source chapters
  unchanged, replace the rest of the divergence chapter, and regenerate every
  subsequent chapter as a user-scoped `PlayerChapter`. Canonical chapter prose
  becomes reference material only; it is never displayed as the active timeline
  when its preconditions have been invalidated.
- Persist generated chapters as deterministic read projections of committed
  world state. Fail closed on generation errors, generate chapters in order,
  and isolate post-divergence narrative nodes by player.
- Version prompts and structured-output schemas; reject transitions that refer
  to unknown entities, violate hard world rules, resurrect characters without
  an explicit allowed mechanism, or cross the reader's spoiler boundary.
- Build offline evaluation sets for extraction coverage, chronology, causal
  consistency, character consistency, spoiler leakage, memory relevance, and
  multi-turn narrative coherence.
- Add per-provider latency, error, retry, token, cache-hit, and cost metrics
  with explicit budgets.
- Add deletion/export workflows and explicit retention for source text,
  messages, memories, canonical models, timelines, and generated assets.
- Threat-model prompt injection, untrusted files, SSRF-capable provider config,
  model data leakage, and abuse; close findings with regression tests.

Exit criteria: a completed novel can be deterministically rebuilt into a
versioned, source-cited canonical graph; structured branch transitions replay
to the same world state; and releases meet defined coherence, privacy, security,
latency, and cost budgets.

## Horizon 4: let the player enter the completed novel's living world

- Insert the user's `PlayerEntity` into an unlocked canonical checkpoint as a
  new person in the world. The primary experience never asks the player to make
  decisions on behalf of Liu Bei, Cao Cao, or another canonical character.
- Keep the canonical mainline running as scheduled world events. Canonical
  characters continue to pursue their own goals; player actions may witness,
  assist, obstruct, delay, redirect, or make an event's preconditions false.
- Create an open-world session from `canonical checkpoint + PlayerEntity +
  player timeline`, with durable world time, location, inventory/capabilities,
  faction state, character goals, active threads, and discovered knowledge.
- Generate actions for the player: travel, investigate, converse, ally, oppose,
  resolve a thread, or pursue a player-authored goal. The model proposes
  transitions; domain validation decides what can commit.
- Make every world turn idempotent, atomic, replayable, and auditable. Narrative
  prose is a rendering of the committed transition, never the authoritative
  state itself.
- Give canonical character agents the same committed timeline and their own
  goals, knowledge, and perception of the player so conversations, quests,
  relationships, and world events cannot contradict one another.
- Add a timeline/journal, location view, active-thread list, relationship view,
  and a visible distinction between canonical history and reader-created
  history.
- Support multiple named timelines per novel only after one-timeline resume,
  export, deletion, and conflict behavior are reliable.

Exit criteria: create player -> enter an unlocked mainline checkpoint -> affect
a canonical event without controlling its characters -> complete multiple
world turns -> service restart -> exact resume is covered end to end, while
canon provenance and the player's divergence remain inspectable.

## Horizon 5: scale only the measured bottleneck

- Establish SLOs and capacity tests for upload throughput, stream concurrency,
  world-turn throughput, database latency, Redis memory, and LLM-provider
  saturation.
- Split PostgreSQL ownership physically only when contention, isolation, or
  independent operations justify migration cost.
- Add a durable queue only when ingestion reliability or measured concurrency
  exceeds the in-process worker design.
- Add replicas, partitioning, CDN/object storage, or orchestration only against a
  measured bottleneck and with a rollback plan.

Exit criteria: load and failure-injection tests demonstrate the target SLOs at
the required traffic and data volume.

## Explicit non-goals until evidence changes

- No Kafka/event bus for synchronous request flows.
- No Kubernetes service mesh or multi-region control plane.
- No second vector database while pgvector meets measured recall and latency.
- No generic plugin framework, workflow engine, or repository abstraction with
  a single consumer.
- No unconstrained "keep writing forever" mode whose prose is treated as world
  state. Open-world prose must render a validated, committed transition.

Each becomes eligible only when a named SLO, compliance requirement, or measured
bottleneck cannot be met by the current platform.
