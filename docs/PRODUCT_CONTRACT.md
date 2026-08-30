# NovelWorld Product Contract

Status: **Current envelope `private-preview-v1`** (declared by the merged
reviewed change). It does not make H0 complete or qualify a public release.

This document answers one question: what can NovelWorld honestly promise now?
[`README.md`](../README.md) describes the product, [`SPEC.md`](../SPEC.md)
defines intended normative behavior, and runtime code, migrations, and tests
remain the evidence for current behavior. The
[`Roadmap`](./ROADMAP.md) owns gaps and qualification.

## Evidence terms

- **Accepted** means the current runtime validates and processes the input. It
  is not a quality or reliability claim.
- **Structurally verified** means deterministic tests exercise the contract. It
  is not live-model evidence.
- **Release-qualified** means the version-matched live, adversarial, recovery,
  security, quality, and cost gates required by the Roadmap have passed.
- **Unsupported** means no compatibility, safety, quality, or operations promise
  is made, even if part of the runtime happens to work.

No language, format, provider, model, browser, operating system, deployment, or
accessibility slice is currently release-qualified.

## Current envelope

| Dimension | Current contract | Evidence boundary |
|---|---|---|
| Deployment | Operator-controlled, private, single-node Docker Compose preview on localhost, or behind an operator-managed encrypted private-network/TLS boundary | Production Compose and CI exercise one instance of each service. Internet exposure is unsupported; default Nginx has no TLS termination (the headers below are not a substitute for it); it serves baseline security headers (nosniff, clickjacking denial, same-origin referrer policy); gateway CORS is restricted to the documented preview origins (`CORS_ORIGINS`), and downstream routers stay permissive but are not browser-reachable in this envelope. The supported 0021/0024/0025 transition stops old Narrative before exposing candidate client assets, verifies world actions are retryably fail-stopped, then stops old Novel and Agent processes before migration; zero-downtime world actions are not claimed. A durable exact-manifest marker is synced immediately before migration; every marked transition rolls that exact target forward, the target tempfile and any former current renamed as previous are synced before current replacement, promoted state is synced before marker removal, and removal is synced again. Adoption requires all three barriers, while upgrade, marked restore, and rollback reject a backwards crossing. The drill pins ordering and simulated process recovery; real Linux host power loss and live registry/image health remain unqualified. An installation whose active release tooling predates these controls needs a control-only release before the migration release. |
| Platforms | Linux shell and Windows 10/11 launchers with Docker Compose | Local launchers stop the old project without removing named volumes before rebuild/start, forcing the one-shot migration container to rerun with old writers quiesced; the exact down-before-up order is pinned by `start.ps1 -Check`. Desktop startup stops pg0, refuses occupied service ports, embeds migrations through 0025, and applies them before spawning services. Experimental desktop archives support forward migration only: reusing a post-0025 data directory with an older archive is unsupported and is not blocked by the older binary. Linux paths run in CI; portable Windows/Linux/macOS GitHub Release assets remain experimental until their version-matched artifacts pass the required journey and signing gates. This is not a qualified OS/browser matrix. |
| Input | Direct UTF-8 text paste up to 5 MiB; UTF-8, BOM-marked UTF-16, or GBK TXT up to 10 MiB; EPUB or text-extractable PDF up to 20 MiB; extracted text up to 20 MiB | Parsers and limits are tested. Scanned/image-only PDFs, DRM, malformed archives, and successful semantic extraction are not promised. |
| Language | Simplified Chinese and English have deterministic chapter-splitting and lore-retrieval fixtures; authenticated readers can request a provider-backed, on-demand Simplified Chinese rendering of the visible chapter body without changing the stored source; completed translations are durably shared by exact source hash across readers of the same canonical chapter, and a PostgreSQL lease prevents concurrent cold requests from duplicating provider work; generated narrative transitions require Chinese text (English is rejected fail-closed by the validator); the UI locale is Simplified Chinese (`lang=zh-CN`, Chinese copy — residual non-normative English artifacts like `Loading...` are recorded, no English UI locale is promised) | Translation request bounds, source-bound reuse, concurrent claim fencing, source/translation switching, and failure fallback are structurally tested; a provider success followed by a database write failure can still require a later provider retry because the two systems cannot commit atomically. Translation quality is provider-dependent and has not passed a representative live-provider gate. “Any language” is unsupported. |
| Models | Operator-managed platform configuration through protected Settings after administrator bootstrap or environment variables, with optional per-user encrypted provider configuration for signed-in readers | First run does not require or test a provider. Adapter compatibility, encrypted-at-rest storage, and recorded fixtures do not qualify a provider/model/version combination. Provider behavior, prices, retention, and safety can change independently. |
| Architecture | Static Cloud Native, DDD, and microservice source boundaries for the private `single-node-v1` topology | The versioned Rust architecture gate checks five runtime packages, reviewed local helpers and transport capabilities, layer/package edges, HTTP adapter placement, analyzable SQL and routine ownership, canonical relation/routine/view/trigger inventory, declared FK/trigger debt, and static process hooks. It does not prove database grants or isolation: one role/schema remains shared, with 20 cross-owner cascading FKs, two lifecycle-trigger accesses, five cross-owner trigger/routine bindings, ten exact historical migration audit hashes, and narrow readiness dependencies. It also does not exhaust unknown external crates or generated code, or qualify drain/timeout completeness, recovery, monitoring/alerting, replicas, horizontal scaling, or public-cloud operation. |
| Scale | `single-node-v1` is a deterministic capacity policy | It is a small CI profile, not an Internet-scale or sustained-load claim. |
| Accessibility | Basic semantic status/error controls exist in the React UI | No browser, keyboard-only, screen-reader, contrast, zoom, or user-journey matrix has been qualified. |

### L0 installation bootstrap and deferred configuration

The supported interactive server launcher requires the bundled PostgreSQL
identity at L0: it guides the role and database names, generates the database
password, writes `BOOTSTRAP_L0_COMPLETE=true` only after validation, and
automatically restarts before Docker or any business service starts. Valid
preseeded or older environments migrate without prompting; an unconfigured
non-interactive launch fails closed. PostgreSQL remains a hard readiness
dependency. The L1 JWT, runtime-encryption, and internal-service roots are
generated after restart.

The base Compose profile then persists `CACHE_MODE=postgres`, reaches readiness
without Redis or an LLM key, and elects the unique first administrator without
a provider call. The administrator configures the platform model later through
protected Settings. AI-backed operations remain unavailable with typed
`503 llm_not_configured` failures until then. Redis, per-user model keys,
monitoring, embeddings, image generation, and initially disabled S3 remain
deferred or explicit capabilities under their own contracts.

NovelWorld is not a minor-directed service. The operator is solely responsible
for restricting or authorizing access. NovelWorld does not currently provide
age assurance, rights clearance, content moderation, complaint, or takedown
operations.

## Responsibility boundary

The operator must:

- admit only trusted users, keep the service off the public Internet, and add
  transport encryption before any non-localhost access;
- have the rights or permission needed to process each uploaded work and its
  generated derivative content;
- disclose the configured model/image providers and obtain any required user
  consent before sending source excerpts, prompts, conversations, or image
  descriptions to them;
- configure provider retention, regional processing, content safety, spending
  limits, credentials, TLS/firewall controls, monitoring, backups, and deletion
  for data outside NovelWorld;
- treat generated text and images as untrusted output and review their use.

NovelWorld owns the application-layer identity, authorization, commit,
idempotency, export, and deletion contracts documented in the
[`SPEC`](../SPEC.md), [`threat model`](./THREAT_MODEL.md), and
[`retention contract`](./DATA_RETENTION.md). It cannot erase provider logs,
provider-hosted image bytes, operator logs, or backups.

## Product claim ledger

| Product claim | State now | Evidence or gap | Owner |
|---|---|---|---|
| L0 installation and first-run administrator/model setup | Structurally verified | Required PostgreSQL identity is validated and committed before one launcher restart; the first administrator is a single durable winner without a provider call; later settings keys are encrypted before PostgreSQL storage and environment configuration takes precedence | H2 |
| One-click and bounded batch import | Accepted and structurally verified inside the input limits | A request accepts one novel, or up to five uploaded files whose source bytes total at most 40 MiB. Batch acceptance commits every independent Novel, shelf/progress row, chapters or retained-source boundary, and pending durable job in one PostgreSQL transaction; it claims one job immediately and leaves the others to the existing leased recovery path. No batch aggregate, pause, cancel, or per-file metadata editor is claimed. Single-import fenced leases resume `source`/`chapters`/`enriched` work; live kill drills at the `chapters` and `enriched` boundaries pass in CI, and the S3 `source`-boundary drill passes in required CI ([PR #135](https://github.com/Wisdoverse/novelworld/pull/135)); cross-attempt provider calls stay inside `import-provider-budget-v1` (3-claim ceiling, terminal `budget_exhausted`); live semantic quality remains unqualified | H1 |
| Shared parsed novels with private user worlds | Structurally verified | A ready canonical novel can be attached from the shared catalog without uploading or parsing it again; shelf authorization, reading progress, identity, deviation mode, choices, chat/memory, and world state remain user-scoped. Deleting the uploader preserves the canonical asset for other shelves. Automatic same-content detection is not claimed; reuse is an explicit catalog action. | H1, H4 |
| Canonical world model and relationship graph | Structurally verified | Source coverage exists in deterministic tests; representative live quality is not qualified | H1, H3 |
| Character personality and authentic voice | Intended gap | Extraction remains whole-novel, but persona publication and Agent consumption are now progress-bounded: before a Ready novel reaches its final chapter, character list/detail reads expose only a source-proven canonical name with `id`, `novel_id`, and first appearance; at `Ready && current == total`, an allowlisted full persona carries `persona_source_chapter_high_water`. `system_prompt` is never public. Every new chat turn persists a validated marker no later than its chapter snapshot; a reclaimed turn requires the latest validated marker to equal the persisted marker before prompt/provider work. Pre-contract unmarked turns and their unmarked Mid/Long summaries remain exportable but cannot enter online history, prompt composition, count-triggered summaries, semantic retrieval, or completed replay. The [DeepSeek v4 Flash live baseline](./evidence/deepseek-v4-flash-live-baseline.json) exercised both persona boundaries, completed the reader journey, and passed final-SHA H3 calibration, but H1 failed and no human voice approval exists; representative live voice quality remains unqualified | H3 |
| Generated portrait for every character | Obsolete claim | Avatar generation is a non-authoritative projection, capped at 30 characters per import and six concurrent provider requests service-wide, and stores provider-returned URL metadata | H0 decision recorded here; any quality slice belongs to H3 |
| Four-layer memory continuity | Intended gap | New-protocol world turns durably remain `pending` until an explicitly witnessed protagonist fact is `saved`, or eligibility is terminal `skipped`; a different key cannot advance the same user+novel while that authority slot is unresolved. Exact-key replay and Narrative's bounded durably rotating recovery scan both compensate without another world commit, using the same identity, source-visibility, deterministic UUIDv5, Agent HTTP, and terminal-CAS path. Agent authenticates the full scope/shape, fixed importance, and known-field schema before fact-first insertion or retrieval; journey ingress intentionally makes zero embedding calls so optional search work cannot delay authority. Direct retrieval is chapter-bounded, reserves whole-entry budget for authenticated journey facts, and excludes generated prose and unrelated mutations. Both direct and semantic paths expose these journey facts only to a persisted `self` identity snapshot. New Mid/Long rows carry the maximum persona marker of every actually summarized, proven chat row; a missing, invalid, or future marker blocks consolidation and prompt use. Legacy unmarked Mid/Long rows remain exportable and deletable but are excluded from direct and semantic online paths. Character mode still fails closed because memory rows lack durable reader-identity provenance: it omits mid/long/permanent/semantic candidates and projection, retaining only progress-bounded recent chat backed by completed, persona-proven claims for the exact same character. First adoption terminal-skips pre-contract completed turns because their witness provenance cannot be proved, retains old rows, and quarantines the former permanent/importance-7/UUID-v4 producer class from direct and semantic prompts; this may hide a legitimate legacy row in that narrow class. Permanent semantic enrichment, historical fact backfill, continuous late-compensation-window guarantees, and qualified live provider/lifecycle quality remain open, so H3 is not complete | H3 |
| Branching and open-world action | Structurally verified for one player timeline | Exact committed-choice replay remains valid while a different index conflicts. A new choice commits only against the locked fingerprint used for generation and at a strictly later choice chapter; stale drafts roll back without partial rows. A choice rewrite atomically replaces an older same-chapter continuation, including legacy exact replay rebuilt from its committed consequence. `PlayerEntity` creation rejects a checkpoint behind committed branch history; competing definitions/checkpoints elect one winner, its checkpoint bounds later choices, and open-world entry rejects every new branch choice with typed conflict semantics. Before open-world entry, self-mode character chat may receive only the latest four committed transition-event summaries that explicitly list that character; Narrative filters them by its final progress read, retains only the requested user/novel/character scope plus requested-actor provenance, and omits choice text, consequence prose, locations, relationships, threads, and non-scope node/choice/world-turn identifiers. Agent independently validates scope, order, actor provenance, size, and source high-water, then projects only chapter plus summary, so no UUID reaches the provider prompt; it omits the whole branch context after a rewind. The first committed open-world start seals its entry context; later valid starts resume that winner and cannot rewrite it from a fresh candidate. Every new action revalidates the complete committed prefix against that checkpoint, so inconsistent legacy state conflicts before provider/world commit and cannot lower fact provenance. After entry, character chat omits choices, location, unscoped threads, player/routing IDs, and technical metadata, and sends only a provider allowlist with the numeric relationship score, character goals and canonical event, the latest four directly targeted `converse`/`ally`/`oppose` actions from a bounded 100-turn scan, and actor-listed player events. Relationship-change prose, perception, and model-generated reasons have no independent witness provenance and are omitted; actions/events are checked at both producer and consumer boundaries. Derived world context carries a canonical source high-water. After a rewind below it, player/world reads, new actions, replays, and final choice/node responses return content-free `reading_progress_behind_world`; an effective-chapter read returns immutable canon with `generated=false`, including after an in-flight generation race. The browser synchronously replaces cached player/world content with canon and refetches after progress changes. Version 2 preserves the old world-only internal response; version 3 adds the bounded branch/world envelope, while unsupported peers add no new branch context. The journey timeline distinguishes reader authority from generated prose. A terminal POST reports `saved`/`skipped`; older-response journal confirmation and user+novel-scoped same-tab `sessionStorage` preserve an ambiguous action/key until terminal. A confirmed principal transition synchronously clears private query/chat state before exposure; late old-principal mutations only invalidate/refetch active current-principal truth and never cache their response. Successful authentication keeps the confirmed principal's recovery records and removes other principals'; logout, successful account deletion, missing credentials, or confirmed `401`/`403` clears private cache and all recovery records. Transient auth/deletion failure retains them. Successful shelf removal clears only the current user+novel record; failed removal retains it, and unrelated storage remains untouched. Version 4 binds every new chat claim to the exact `WorldState` fingerprint returned with its one-snapshot context and rechecks it after generation; a branch or world commit during provider work leaves zero committed chat messages and no `done`. The final Narrative read and Agent commit remain separate service transactions, so cross-service linearizability is not claimed. Visibility beyond explicit IDs or the bounded scan is unsupported outside `h4-journey-qualification-v1`; the registered explicit-witness slice still lacks its judged live provider/lifecycle quality and human accessibility evidence | H4 |
| Novel-specific D20 advanced mode | Structurally verified, opt-in preview | The default remains narrative mode. An explicit request lazily generates one source-bound attribute/DC template per canonical novel model and shares it across authorized readers; generation is leased, fenced, and capped at three persisted claims with one logical generation call per claim (the shared transport retry policy may replay that same request). Advanced turns persist a server-owned D20 result before provider prose and replay it through the world-turn ledger. Structural target/hard-rule checks remain authoritative; arbitrary free-text reasonableness and representative live template quality are not qualified | H4 |
| Assume a canonical character identity | Fail-closed legacy compatibility path | The primary actor is an original `PlayerEntity`. Character identity is limited to in-character conversation and exact read/replay of a branch result already committed in self mode; Player/open-world endpoints, new or cached narrative-node perspective, new nodes, and new choices are unsupported and refused before provider/write work. Character WorldState is choices-only; a persisted character chat claim receives only an opaque causal revision from Narrative and never receives or injects Player branch/world context, even after a concurrent switch to self. Its history is restricted to completed, persona-proven turns for the exact same persisted character identity, and it omits unprovenanced mid/long/permanent/semantic memory plus derived projection. Pre-contract unmarked chat remains exportable but is absent from every online history/prompt/count/replay path. `h4-journey-qualification-v1` qualifies only this bounded compatibility smoke; durable node identity-provenance, general cross-service identity revision, and arbitrary concurrent or long-lived switching are unsupported outside that slice rather than H4-v1 release blockers | H4 |
| No spoilers | Structurally bounded, not guaranteed | Server-owned progress filters lore, committed memory, and the public character-persona read model. New character facts are content-filtered to explicit witness provenance and stored at the committed session's canonical source high-water; the pre-contract permanent/importance-7/UUID-v4 producer class is retained but quarantined from every direct and semantic prompt path. Character prompt construction omits derived context whose source high-water is missing or later than current progress. World endpoints also return a content-free typed conflict while progress is behind. The supported managed Docker path drains legacy Narrative, Novel, and Agent processes before the relevant 0021/0024/0025 semantic migrations and refuses adoption/upgrade/marked-restore/rollback paths that would revive a pre-barrier writer or reader. In particular, it cannot restore a pre-0024 Agent or Novel release after 0024, or a pre-0025 Agent release after 0025, without a separately approved compatibility procedure. A legitimate legacy row in a quarantined class can be hidden, and an untrusted model can still produce incorrect text. Relationships, world-summary surfaces, and export as a whole have not yet been proven under one equivalent progress boundary | H3, H4 |
| Retry/restart without duplicate committed chat, world, or import authority | Structurally verified at persisted boundaries | Chat and world turns fence logical keys and replay committed results. One database authority slot spans an in-progress world turn and a committed `pending` projection; exact-key replay or Narrative's bounded periodic scan retries eligible rows through the same idempotent memory fact. Identity/progress-ineligible or invalid rows remain `pending` and are durably rotated so they cannot starve later rows; post-commit identity or visibility races return a content-free unknown outcome. The browser scopes the bounded action/key by user+novel in same-tab `sessionStorage`, retains it across failed post-commit confirmation, and clears it only on terminal result/rejection or principal lifecycle. Blocked storage and the H4-v1 90-second live dependency-recovery drill remain H4 gaps; sustained long-window recovery/SLO observation remains H5 | H1, H3, H4, H5 |
| Complete export and deletion | Structurally verified within the documented application boundary | Provider/operator data and non-atomic backups remain outside the portable export and application erasure boundary | H2, H5 |

For H4, server-authoritative checks are the ones the runtime can execute:
identity/ownership, membership of supported targets, death,
location/thread availability, source-progress bounds, state revision, turn
order, idempotency, strict field shape, and numeric/list ranges. Item targeting
is unsupported and rejected as an unknown field; generated inventory is
shape-bounded state, not a canonical item catalog. Free-text `hard_rules`
prose constrains generation quality but cannot grant authority or substitute
for validation. Hostile instructions remain quoted untrusted input and cannot
bypass those listed checks. Semantic canon, spoiler, and agency correctness is
qualified through the frozen H3 plus independent-human corpus, not claimed as
general server-side natural-language enforcement. NovelWorld does not claim a
keyword blacklist or a general natural-language rules engine.

For the progress-bounded character read model, a partial response contains only
`id`, `novel_id`, a canonical `name` proven in its first-appearance source
chapter, and `first_appearance_chapter`. Alias-only evidence does not authorize
that name. Here, “source-proven” is only a conservative lexical occurrence:
ASCII names require token boundaries, non-ASCII names require at least two
Unicode scalar values, and a match covered by any known longer canonical name
or alias is rejected. It is not semantic entity recognition; representative
live extraction/identity qualification remains open. A full response is
available only when the novel is `Ready`, has a positive chapter count, and
persisted progress equals that count; it carries
`persona_source_chapter_high_water` and still omits `system_prompt`. Character
list/detail responses use `Cache-Control: private, no-store`. The browser
rebuilds both partial and full objects from an explicit field allowlist and
derives the selected character by id from the latest query result; these are
cache/race fences, not authorization or provenance checks.

A pre-contract canonical identity whose stored name is supported only by an
alias can remain readable at full completion, but after a rewind its progress
read fails closed with `reader_identity_unavailable`. New alias-only identities
cannot be selected. The browser offers an explicit switch back to self
identity; the GET path never rewrites the database. This narrow compatibility
behavior does not establish a global spoiler guarantee for relationships,
world summaries, or account export.

For the branching row, “inconsistent legacy state” includes any non-bijective
relationship between durable `user_choices` and the node-keyed JSONB choice
projection: missing, duplicate, unkeyed, malformed, or field-mismatched entries
fail closed at Player creation, open-world entry, choice commit/replay, and
world-turn reservation/completion. No request-path repair is claimed; affected
legacy data requires an explicit, auditable migration.

For the private browser-state claims, delayed protected responses are fenced by
their initiating bearer and, where known, principal. An older `401`, identity
refresh, deletion, or export cannot clear or expose the newer session; export
bytes are dropped before inspection/download after a principal change. A shared
`auth_token` change from another tab clears in-memory query/chat state and forces
authoritative re-authentication. This does not claim cross-device session push
or server-side revocation notification.

## Resolved documentation conflicts

These decisions replace contradictory prose; they do not claim the missing
runtime outcome exists.

1. **Specification authority:** `SPEC.md` is the candidate normative target,
   not a conformance certificate. A statement is current only when runtime
   evidence supports it.
2. **Formats:** paste, TXT, EPUB, and PDF are accepted within the limits above.
   File upload supports a bounded batch of up to five files and 40 MiB total;
   batch titles come from file names, with one optional shared author and mode.
   Acceptance is separate from semantic-quality qualification.
3. **Language:** language-agnostic architecture remains an aspiration. The
   current Chinese-only narrative validator prevents an any-language journey
   claim.
4. **Source retention and reprocessing:** original uploaded bytes are retained
   only when S3 is enabled. With retention, imports accept at the `source`
   stage and the claimed job replays the retained object to rebuild chapters
   before any provider work; without retention, chapter splitting stays
   request-local and re-upload remains necessary before the chapter boundary.
5. **Avatars:** NovelWorld stores provider-returned URL metadata and does not
   own or export the provider's image bytes. Avatar failure or the 30-character
   cap does not block import readiness.
6. **Narrative nodes:** detection may batch chapter summaries and choices may be
   generated when requested. Runtime choices are per-user because generation
   receives private Player/world/mode context; legacy shared nodes are accepted
   only for exact replay of that user's already-committed choice. Node cache
   writes are first-writer-wins, and a final WorldState read suppresses options
   invalidated concurrently by Player sealing, a choice, or open-world entry.
   Product behavior, not one LLM call per chapter, is the contract.
7. **Permanent memory:** permanent means exempt from normal compression or
   promotion while its account and novel exist. Its embedding is an optional
   search projection because the existing schema is nullable and direct
   retrieval remains authoritative. This changes no retention rule: account or
   novel deletion still erases the fact. Migration
   `0021_world_turn_memory_projection.sql` adds per-turn projection state,
   acknowledgement time, and one unresolved-turn authority slot. At first
   adoption, pre-contract completed turns become terminal `skipped` because
   their character-witness provenance cannot be proved; no historical fact is
   invented. Pre-contract memory rows remain available to export and ordinary
   lifecycle deletion because the exact historical protagonist cannot be
   reconstructed safely. Agent prompt consumers instead quarantine the former
   producer class (`permanent`, importance 7, UUID version nibble 4) from both
   direct and semantic retrieval; this can conservatively hide a legitimate
   legacy row in that narrow class. Replaying 0021 validates the contract but
   MUST NOT terminally skip a new-protocol pending turn. Migration
   `0024_persona_provenance.sql` adds nullable provenance markers to chat turns
   and derived memory. Existing rows deliberately stay null and remain
   exportable/deletable, while every online history, prompt, count, replay, and
   Mid/Long retrieval path excludes them. A new derived summary is written only
   after all actual source messages prove a bounded marker, and stores their
   maximum marker.

   Migration `0025_chat_world_revision.sql` adds the nullable 32-byte causal
   revision to chat claims. It preserves historical completed/failed nulls
   without inventing provenance, converts only legacy null `in_progress` rows
   to `failed/causal_revision_unavailable`, and requires every subsequent
   in-progress claim to carry an exact revision. New Agent code rejects legacy
   null replay, refreshes the revision on a safe same-key reclaim, and fences
   completion on the persisted digest.

   The supported release path stops/drains the old Narrative producer before
   exposing the candidate client, confirms world actions fail with a retryable
   `5xx` while retaining their exact recovery key, and then stops old Novel and
   Agent processes before the 0021/0024/0025 migrations. An installation running
   release tooling that predates any barrier first needs a control-only
   release with the new script that preserves every barrier already present in
   the active release and omits every barrier not yet applied, followed by
   the migration release. For a post-0024 installation adopting 0025, the
   control-only target retains 0021/0024 and omits only 0025. New-script
   adoption requires a target containing all three migrations; upgrade, marked
   restore, and rollback fail closed before crossing any barrier backwards.
   On the supported managed Docker path, a post-0024 database must not run a
   pre-0024 Agent or Novel release, and a post-0025 database must not run a
   pre-0025 Agent release; crossing those tooling-enforced barriers
   requires a separately approved compatibility procedure. Experimental desktop
   archives are forward-migration-only and must not reuse a newer data directory
   with an older archive. Every marked adoption or upgrade can only roll the exact marker
   forward. The marker is synced before migration, the former
   target tempfile and former current renamed as previous are synced before
   current replacement, promoted state is synced before marker deletion, and
   deletion is synced again. Normal restore and healthy rollback delete an
   unmarked candidate before writing the exact marker and use the same durable
   finalization barrier; new rollback reuses the schema-transition promotion
   protocol, while the older rollback marker is compatibility recovery only;
   it rejects a different candidate, tolerates a missing candidate file, and
   promotes only after migration replay and health succeed. Upgrade recovery
   preserves the former current as previous; initial adoption rejects an
   impossible previous release. Any separately
   approved compatibility procedure must preserve existing nullable-
   embedding facts and must not treat legacy generated-prose rows as
   authoritative structured facts. New journey facts use the committed
   session's unlocked-through chapter as their conservative source coordinate.
8. **Character identity:** `self` with a durable original `PlayerEntity` is the
   primary open-world mode. Character identity is a fail-closed compatibility
   mode for conversation plus exact read/replay of an already committed branch
   result. Player/open-world endpoints and every new node/choice are refused
   before provider/write work; character-mode WorldState is choices-only and
   internal V4 context contains only an opaque causal revision, never Player
   branch/world content. Durable identity-provenance keys, general cross-service
   identity-revision fencing, and arbitrary concurrent or long-lived switching
   are unsupported outside the bounded H4-v1 compatibility slice (SPEC §8.2),
   not release blockers for that slice.
9. **Prompt injection:** prompts delimit untrusted source/user content and model
   output is validated before authoritative transitions. Prompt text cannot
   guarantee model behavior or authorize an operation.
10. **Cancellation:** no user-visible cancellation guarantee exists. H1 may add
    one only with durable state and recovery semantics.

## Change and qualification rule

Changes to this envelope require a reviewed Roadmap issue and must update the
product claim, SPEC target, runtime behavior, and evidence together when they
are affected. Thresholds and supported slices must be approved before the
change they judge; a candidate cannot weaken its own gate.

The approved [`qualification policy`](./QUALIFICATION_POLICY.md) defines the
initial journey slices, hard guardrails, evidence classes, and threshold
approval process without claiming that a live slice has passed them.

The clause dispositions and their owning horizons are recorded in the
candidate [`SPEC conformance ledger`](./SPEC_CONFORMANCE.md). That ledger and
this contract do not by themselves complete H0: the clean-checkout verification
entry point and independent maintainer, product, security, accessibility, or
legal review remain separate gates where applicable.
