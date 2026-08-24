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
| Deployment | Operator-controlled, private, single-node Docker Compose preview on localhost, or behind an operator-managed encrypted private-network/TLS boundary | Production Compose and CI exercise one instance of each service. Internet exposure is unsupported; default Nginx has no TLS termination (the headers below are not a substitute for it); it serves baseline security headers (nosniff, clickjacking denial, same-origin referrer policy); gateway CORS is restricted to the documented preview origins (`CORS_ORIGINS`), and downstream routers stay permissive but are not browser-reachable in this envelope. The supported 0020 upgrade stops old Narrative before exposing candidate client assets, verifies world actions are retryably fail-stopped, then drains Agent before migration; zero-downtime world actions are not claimed. A durable exact-manifest marker is synced immediately before migration; every marked transition rolls that exact target forward, the target tempfile and any former current renamed as previous are synced before current replacement, promoted state is synced before marker removal, and removal is synced again. A 0020 recovery therefore never revives the older writer, while a downloaded/pre-migration candidate alone does not block current restore. The drill pins ordering and simulated process recovery; real Linux host power loss and live registry/image health remain unqualified. An installation whose active release tooling predates this control needs a control-only release before the migration release. |
| Platforms | Linux shell and Windows 10/11 launchers with Docker Compose | Local launchers stop the old project without removing named volumes before rebuild/start, forcing the one-shot migration container to rerun with old writers quiesced; the exact down-before-up order is pinned by `start.ps1 -Check`. Desktop startup stops pg0, refuses occupied service ports, embeds migrations through 0023, and applies them before spawning services. Linux paths run in CI; portable Windows/Linux/macOS GitHub Release assets remain experimental until their version-matched artifacts pass the required journey and signing gates. This is not a qualified OS/browser matrix. |
| Input | Direct UTF-8 text paste up to 5 MiB; UTF-8, BOM-marked UTF-16, or GBK TXT up to 10 MiB; EPUB or text-extractable PDF up to 20 MiB; extracted text up to 20 MiB | Parsers and limits are tested. Scanned/image-only PDFs, DRM, malformed archives, and successful semantic extraction are not promised. |
| Language | Simplified Chinese and English have deterministic chapter-splitting and lore-retrieval fixtures; authenticated readers can request a provider-backed, on-demand Simplified Chinese rendering of the visible chapter body without changing the stored source; completed translations are durably shared by exact source hash across readers of the same canonical chapter, and a PostgreSQL lease prevents concurrent cold requests from duplicating provider work; generated narrative transitions require Chinese text (English is rejected fail-closed by the validator); the UI locale is Simplified Chinese (`lang=zh-CN`, Chinese copy — residual non-normative English artifacts like `Loading...` are recorded, no English UI locale is promised) | Translation request bounds, source-bound reuse, concurrent claim fencing, source/translation switching, and failure fallback are structurally tested; a provider success followed by a database write failure can still require a later provider retry because the two systems cannot commit atomically. Translation quality is provider-dependent and has not passed a representative live-provider gate. “Any language” is unsupported. |
| Models | Operator-supplied model configuration through web setup or environment variables | Adapter compatibility and recorded fixtures do not qualify a provider/model/version combination. Provider behavior, prices, retention, and safety can change independently. |
| Scale | `single-node-v1` is a deterministic capacity policy | It is a small CI profile, not an Internet-scale or sustained-load claim. |
| Accessibility | Basic semantic status/error controls exist in the React UI | No browser, keyboard-only, screen-reader, contrast, zoom, or user-journey matrix has been qualified. |

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
| First-run administrator and model setup | Structurally verified | The first administrator is a single durable winner; web-supplied provider keys are encrypted before PostgreSQL storage and environment configuration takes precedence | H2 |
| One-click import | Accepted and structurally verified inside the input limits | Acceptance atomically commits chapters plus a pending durable job, or — with source retention enabled — the retained object plus a `source`-stage job whose claim rebuilds deterministic chapters from the retained bytes; fenced leases resume `source`/`chapters`/`enriched` work; live kill drills at the `chapters` and `enriched` boundaries pass in CI, and the S3 `source`-boundary drill passes in required CI ([PR #135](https://github.com/Wisdoverse/novelworld/pull/135)); cross-attempt provider calls stay inside `import-provider-budget-v1` (3-claim ceiling, terminal `budget_exhausted`); live semantic quality remains unqualified | H1 |
| Shared parsed novels with private user worlds | Structurally verified | A ready canonical novel can be attached from the shared catalog without uploading or parsing it again; shelf authorization, reading progress, identity, deviation mode, choices, chat/memory, and world state remain user-scoped. Deleting the uploader preserves the canonical asset for other shelves. Automatic same-content detection is not claimed; reuse is an explicit catalog action. | H1, H4 |
| Canonical world model and relationship graph | Structurally verified | Source coverage exists in deterministic tests; representative live quality is not qualified | H1, H3 |
| Character personality and authentic voice | Intended gap | Chat consumes bounded, JSON-quoted aliases, role, description, personality, background, and speaking style plus lore/memory/world context. Persona extraction is still whole-novel rather than progress-bounded, and live voice quality is unqualified | H3 |
| Generated portrait for every character | Obsolete claim | Avatar generation is a non-authoritative projection, capped at 30 characters per import, and stores provider-returned URL metadata | H0 decision recorded here; any quality slice belongs to H3 |
| Four-layer memory continuity | Intended gap | New-protocol world turns durably remain `pending` until an explicitly witnessed protagonist fact is `saved`, or eligibility is terminal `skipped`; a different key cannot advance the same user+novel while that authority slot is unresolved, while exact-key replay compensates without another world commit. Agent authenticates the deterministic private UUIDv5, full scope/shape, fixed importance, and known-field schema before fact-first insertion or retrieval; journey ingress intentionally makes zero embedding calls so optional search work cannot delay authority. Direct retrieval is chapter-bounded, reserves whole-entry budget for authenticated journey facts, and excludes generated prose and unrelated mutations. Both direct and semantic paths expose these journey facts only to a persisted `self` identity snapshot. Character mode fails closed because current memory rows lack identity provenance: it omits mid/long/permanent/semantic candidates and projection, retaining only recent chat backed by completed claims for the exact same character; legacy/unprovenanced chat remains self-only. First adoption terminal-skips pre-contract completed turns because their witness provenance cannot be proved, retains old rows, and quarantines the former permanent/importance-7/UUID-v4 producer class from direct and semantic prompts; this may hide a legitimate legacy row in that narrow class. Permanent semantic enrichment, historical fact backfill, autonomous pending reconciliation, continuous late-compensation-window guarantees, and qualified live provider/lifecycle quality remain open, so H3 is not complete | H3 |
| Branching and open-world action | Structurally verified for one player timeline | Exact committed-choice replay remains valid while a different index conflicts. A new choice commits only against the locked fingerprint used for generation and at a strictly later choice chapter; stale drafts roll back without partial rows. A choice rewrite atomically replaces an older same-chapter continuation, including legacy exact replay rebuilt from its committed consequence. `PlayerEntity` creation rejects a checkpoint behind committed branch history; competing definitions/checkpoints elect one winner, its checkpoint bounds later choices, and open-world entry rejects every new branch choice with typed conflict semantics. The first committed open-world start seals its entry context; later valid starts resume that winner and cannot rewrite it from a fresh candidate. Every new action revalidates the complete committed prefix against that checkpoint, so inconsistent legacy state conflicts before provider/world commit and cannot lower fact provenance. After entry, character chat omits choices, location, unscoped threads, player/routing IDs, and technical metadata, and sends only a provider allowlist with the numeric relationship score, character goals and canonical event, the latest four directly targeted `converse`/`ally`/`oppose` actions from a bounded 100-turn scan, and actor-listed player events. Relationship-change prose, perception, and model-generated reasons have no independent witness provenance and are omitted; actions/events are checked at both producer and consumer boundaries. Derived world context carries a canonical source high-water. After a rewind below it, player/world reads, new actions, replays, and final choice/node responses return content-free `reading_progress_behind_world`; an effective-chapter read returns immutable canon with `generated=false`, including after an in-flight generation race. The browser synchronously replaces cached player/world content with canon and refetches after progress changes. A version-2 capability header makes both mixed-version directions degrade to no world context rather than serve an unbounded view. The journey timeline distinguishes reader authority from generated prose. A terminal POST reports `saved`/`skipped`; older-response journal confirmation and user+novel-scoped same-tab `sessionStorage` preserve an ambiguous action/key until terminal. A confirmed principal transition synchronously clears private query/chat state before exposure; late old-principal mutations only invalidate/refetch active current-principal truth and never cache their response. Successful authentication keeps the confirmed principal's recovery records and removes other principals'; logout, successful account deletion, missing credentials, or confirmed `401`/`403` clears private cache and all recovery records. Transient auth/deletion failure retains them. Successful shelf removal clears only the current user+novel record; failed removal retains it, and unrelated storage remains untouched. Pre-open-world branch-to-chat continuity, exact chat/world-revision provenance, visibility beyond explicit IDs or the bounded scan, live provider/lifecycle quality, and human accessibility evidence remain unqualified | H4 |
| Novel-specific D20 advanced mode | Structurally verified, opt-in preview | The default remains narrative mode. An explicit request lazily generates one source-bound attribute/DC template per canonical novel model and shares it across authorized readers; generation is leased, fenced, and capped at three persisted claims with one logical generation call per claim (the shared transport retry policy may replay that same request). Advanced turns persist a server-owned D20 result before provider prose and replay it through the world-turn ledger. Structural target/hard-rule checks remain authoritative; arbitrary free-text reasonableness and representative live template quality are not qualified | H4 |
| Assume a canonical character identity | Fail-closed legacy compatibility path | The primary actor is an original `PlayerEntity`. Character identity is limited to in-character conversation and exact read/replay of an already committed branch result; Player/open-world endpoints, new narrative nodes, and new choices are refused before provider/write work. Character WorldState is choices-only; a persisted character chat claim never calls the Player world-context port even after a concurrent switch to self. Its history is restricted to completed turns for the exact same persisted character identity, and it omits unprovenanced mid/long/permanent/semantic memory plus derived projection; legacy/unclaimed chat is self-only. Durable node identity-provenance keys and general cross-service identity-revision fencing remain unqualified | H4 |
| No spoilers | Structurally bounded, not guaranteed | Server-owned progress filters lore and committed memory. New character facts are content-filtered to explicit witness provenance and stored at the committed session's canonical source high-water; the pre-contract permanent/importance-7/UUID-v4 producer class is retained but quarantined from every direct and semantic prompt path. Character prompt construction omits all derived world context when that high-water is missing, later than current progress after a rewind, or requested by an old consumer without the version-2 capability. World endpoints also return a content-free typed conflict while progress is behind. The supported release path drains the legacy Narrative producer and Agent consumer before migration 0021 and refuses adoption/upgrade/restore/rollback that would revive a pre-contract writer. A legitimate legacy row in the quarantined class can be hidden, and an untrusted model can still produce incorrect text | H3, H4 |
| Retry/restart without duplicate committed chat, world, or import authority | Structurally verified at persisted boundaries | Chat and world turns fence logical keys and replay committed results. One database authority slot spans an in-progress world turn and a committed `pending` projection, so only exact-key compensation proceeds until `saved`/`skipped`; post-commit identity or visibility races return content-free unknown outcome. The browser scopes the bounded action/key by user+novel in same-tab `sessionStorage`, retains it across failed post-commit confirmation, and clears it only on terminal result/rejection or principal lifecycle. Blocked storage, autonomous projection reconciliation, live dependency failure, and long-window recovery remain gaps | H1, H3, H5 |
| Complete export and deletion | Structurally verified within the documented application boundary | Provider/operator data and non-atomic backups remain outside the portable export and application erasure boundary | H2, H5 |

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
   legacy row in that narrow class. Replaying the migration
   validates the contract but MUST NOT terminally skip a new-protocol pending
   turn. The supported
   release path stops/drains the old Narrative producer before exposing the
   candidate client, confirms world actions fail with a retryable `5xx` while
   retaining their exact recovery key, and then drains Agent before this
   migration. An
   installation running release tooling that predates the barrier first needs
   a control-only release with the new script but without 0020, followed by the
   migration release. New-script adoption requires a target containing 0020;
   adoption, upgrade, restore, and rollback fail closed before activating a
   pre-0020 writer. Crossing that tooling-enforced barrier requires a separately
   approved compatibility procedure. Every marked adoption or upgrade can
   only roll the exact marker forward; a 0020 transition never revives its
   pre-0020 current writer. The marker is synced before migration, the former
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
   internal character-world context is absent. Durable identity-provenance keys
   and cross-service identity-revision fencing remain H4 gaps (SPEC §8.2).
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
