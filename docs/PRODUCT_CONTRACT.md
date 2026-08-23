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
| Deployment | Operator-controlled, private, single-node Docker Compose preview on localhost, or behind an operator-managed encrypted private-network/TLS boundary | Production Compose and CI exercise one instance of each service. Internet exposure is unsupported; default Nginx has no TLS termination (the headers below are not a substitute for it); it serves baseline security headers (nosniff, clickjacking denial, same-origin referrer policy); gateway CORS is restricted to the documented preview origins (`CORS_ORIGINS`), and downstream routers stay permissive but are not browser-reachable in this envelope. |
| Platforms | Linux shell and Windows 10/11 launchers with Docker Compose | Linux paths run in CI; `start.ps1 -Check` runs on Windows CI. Portable Windows/Linux/macOS GitHub Release assets are experimental until their version-matched artifacts pass the required journey and signing gates. This is not a qualified OS/browser matrix. |
| Input | Direct UTF-8 text paste up to 5 MiB; UTF-8, BOM-marked UTF-16, or GBK TXT up to 10 MiB; EPUB or text-extractable PDF up to 20 MiB; extracted text up to 20 MiB | Parsers and limits are tested. Scanned/image-only PDFs, DRM, malformed archives, and successful semantic extraction are not promised. |
| Language | Simplified Chinese and English have deterministic chapter-splitting and lore-retrieval fixtures; authenticated readers can request a provider-backed, on-demand Simplified Chinese rendering of the visible chapter body without changing the stored source; generated narrative transitions require Chinese text (English is rejected fail-closed by the validator); the UI locale is Simplified Chinese (`lang=zh-CN`, Chinese copy — residual non-normative English artifacts like `Loading...` are recorded, no English UI locale is promised) | Translation request bounds, source/translation switching, and failure fallback are structurally tested; translation quality is provider-dependent and has not passed a representative live-provider gate. “Any language” is unsupported. |
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
| Four-layer memory continuity | Intended gap | Durable messages and mid-term summaries are written; mid-term summaries promote best-effort into embedded long-term memory, and committed open-world turns can write permanent journey memory. Promotion/retrieval quality, skipped writes, and full lifecycle evidence remain unqualified | H3 |
| Branching and open-world action | Structurally verified for one player timeline | Choice replay is exact for the committed index and conflicts on a different index. World turns validate open thread targets, provide generation with a bounded tail of committed actions and narrative endings, and preserve the logical-turn key while the outcome is ambiguous. Deterministic evidence does not prove that chat, branches, chapters, and world actions form one live-quality causal system | H4 |
| Assume a canonical character identity | Accepted legacy/experimental path, unsupported as a product promise | The primary open-world contract is an original `PlayerEntity`; character identity is bounded to in-character conversation and branch choices, with no open-world mutation authority | H4 |
| No spoilers | Structurally bounded, not guaranteed | Server-owned progress filters lore and committed memory, but an untrusted model can still produce incorrect text | H3, H4 |
| Retry/restart without duplicate committed chat, world, or import authority | Structurally verified at persisted boundaries | Import attempts fence source-stage chapter replacement, chapter-node, character/relationship, enrichment, and canon commits; live kill drills at the `chapters` and `enriched` boundaries pass in CI; cross-attempt provider calls stay inside `import-provider-budget-v1`; live dependency and long-window recovery evidence remains a H1/H5 gate | H1, H5 |
| Complete export and deletion | Structurally verified within the documented application boundary | Provider/operator data and non-atomic backups remain outside the portable export and application erasure boundary | H2, H5 |

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
   generated when requested. Product behavior, not one LLM call per chapter, is
   the contract.
7. **Permanent memory:** permanent means exempt from normal compression or
   promotion while its account and novel exist. Account or novel deletion still
   erases it.
8. **Character identity:** `self` with a durable original `PlayerEntity` is the
   primary open-world mode. Character identity is a compatibility mode for
   conversation and branch choices only: it MUST NOT create a `PlayerEntity`,
   enter the open world, submit world turns, or hold world-journal or
   relationship/faction/location mutation authority (boundary in SPEC §8.2).
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
