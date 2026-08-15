# NovelWorld SPEC Conformance Ledger

Status: **H0 candidate for `private-preview-v1`**. This ledger classifies the
reviewed specification; it is not a release certificate and does not make H0
complete.

The exact reviewed [`SPEC.md`](../SPEC.md) bytes are pinned by
[`SPEC_CONFORMANCE.sha256`](./SPEC_CONFORMANCE.sha256). A normative SPEC change
must update this ledger and checksum in the same reviewed pull request.

## Dispositions

- **Verified** — current code plus deterministic tests or checked configuration
  demonstrate the structural contract. This does not imply live quality or
  release qualification.
- **Intended gap** — the requirement remains intentional, but current evidence
  is missing or contradicts complete implementation. The named horizon owns
  its acceptance gate.
- **Obsolete/corrected** — the reviewed change removed or corrected the claim.
- **Aspirational** — the behavior is outside the declared preview and has no
  current support claim.

Rows split a subsection whenever its requirements have different states. This
avoids assigning one optimistic state to a mixed clause. `MAY`, `SHOULD`, and
`OPTIONAL` language was reviewed with the release-blocking `MUST`, `MUST NOT`,
and `REQUIRED` statements even when it does not create an unconditional duty.

## Evidence index

| ID | Evidence boundary |
|---|---|
| E0 | Current envelope and gaps: [`PRODUCT_CONTRACT.md`](./PRODUCT_CONTRACT.md) and [`ROADMAP.md`](./ROADMAP.md) |
| E1 | Schema and production repository contracts: [`init.sql`](../infra/postgres/init.sql), [`repository_contracts.rs`](../tests/integration/tests/repository_contracts.rs), [`legacy_migration.rs`](../tests/integration/tests/legacy_migration.rs) |
| E2 | Import path and validation: [`handlers`](../services/novel-service/src/application/handlers/mod.rs), [`document.rs`](../services/novel-service/src/infrastructure/document.rs), [`character_extractor.rs`](../services/novel-service/src/domain/services/character_extractor.rs), [`novel-service tests`](../services/novel-service/src/tests.rs) |
| E3 | Durable chat and memory: [`agent handlers`](../services/agent-service/src/application/handlers/mod.rs), [`memory_manager.rs`](../services/agent-service/src/domain/services/memory_manager.rs), [`pg_chat_repo.rs`](../services/agent-service/src/infrastructure/persistence/pg_chat_repo.rs), [`core_reader_loop.sh`](../tests/e2e/core_reader_loop.sh) |
| E4 | Canon and player timeline: [`world_session.rs`](../services/narrative-service/src/domain/entities/world_session.rs), [`narrative_transition.rs`](../services/narrative-service/src/domain/services/narrative_transition.rs), [`narrative tests`](../services/narrative-service/src/tests.rs), [`core_reader_loop.sh`](../tests/e2e/core_reader_loop.sh) |
| E5 | Identity and authorization: [`gateway auth`](../gateway/src/auth.rs), [`user handlers`](../services/user-service/src/application/handlers/mod.rs), [`auth_flow.rs`](../tests/integration/tests/auth_flow.rs) |
| E6 | Export and erasure: [`ACCOUNT_EXPORT.md`](./ACCOUNT_EXPORT.md), [`DATA_RETENTION.md`](./DATA_RETENTION.md), [`account_export.rs`](../tests/integration/tests/account_export.rs), [`auth_flow.rs`](../tests/integration/tests/auth_flow.rs) |
| E7 | Browser contract: [`client.ts`](../frontend/src/shared/api/client.ts), [`client.test.ts`](../frontend/src/shared/api/client.test.ts), [`BranchChoice.test.tsx`](../frontend/src/widgets/branch-choice/ui/BranchChoice.test.tsx), [`globals.css`](../frontend/src/app/styles/globals.css) |
| E8 | Runtime and security configuration: [`ci.yml`](../.github/workflows/ci.yml), [`docker-compose.yml`](../docker-compose.yml), [`nginx.conf`](../infra/nginx/nginx.conf), [`THREAT_MODEL.md`](./THREAT_MODEL.md) |

## README claim coverage

The detailed dispositions live in the product claim ledger in
[`PRODUCT_CONTRACT.md`](./PRODUCT_CONTRACT.md#product-claim-ledger). This table
proves that every core README promise has an owner rather than duplicating its
state in two places.

| README promise | Product-contract row or boundary |
|---|---|
| Import and analyze a supported book | One-click import; canonical model and relationship graph |
| Create source-grounded character agents | Character personality and authentic voice |
| Generate portraits | Generated portrait for every character |
| Resume conversations | Four-layer memory continuity; retry/restart behavior |
| Make choices and reshape the story | Branching and open-world action |
| Enter as an original player | Branching and open-world action; canonical-character identity |
| Bound future source context | No spoilers |
| Export/delete owned state | Complete export and deletion |
| One-click private deployment and setup | Current deployment envelope and operator responsibility boundary |

## Normative clause ledger

| SPEC clause and exact scope | State | Owner | Evidence or decision |
|---|---|---|---|
| Normative Language — document every implementation-defined selection | Verified | H0 | E0 and the selections below |
| §4.1.1 — bcrypt cost at least 12 | Verified | H2 | E5 |
| §4.1.5 — embeddings on long/permanent memories | Intended gap | H3 | E0, E3; production writers are not connected and permanent save may persist without an embedding |
| §4.1.6 — chat-turn status, lease, failure, and completion fields agree | Verified | H3, H5 | E1, E3 |
| §4.2 — UUID v4, UTC `TIMESTAMPTZ`, case-insensitive character deduplication | Verified | H1, H2 | E1, E2 |
| §5.1 — accepted formats, byte limits, validation, optional retention, and pending record | Verified | H1 | E0, E2 |
| §5.1 — accepted imports recoverable after process death from committed stages; retained bytes still do not imply retained-object replay | Verified | H1 | E1, E2; durable jobs, leases, and startup recovery landed in PR #116, retained-object reprocessing remains an H1 gap |
| §5.2 — current parsing stages reach `ready` or store a terminal parse error | Verified | H1 | E2 |
| §5.2 — interruption recovery, fenced attempts, and reclaim of pending or expired jobs | Verified | H1 | E1, E2; attempt-fenced claims, lease reclaim, and replay-safe backfill landed in PR #116 |
| §5.3 — non-empty, sequential chapters | Verified | H1 | E2 |
| §5.4 — structured character output validation and case-insensitive merge | Verified | H1 | E2 |
| §5.4 — complete repeated-character coverage and 50-character cost bound | Intended gap | H1 | E2 shows heuristic/model extraction without a release corpus or enforced final cap |
| §5.5 — requested world-summary fields and persistence | Verified | H1 | E2 |
| §5.5 — 2,000-character world-summary maximum | Intended gap | H1 | E2 validates non-empty output but does not enforce this maximum |
| §5.6 — bounded, schema-valid node candidates, persistence, and 2–3 choices | Verified | H1, H4 | E2, E4; only Simplified Chinese is in the current generated path |
| §5.7 — avatar failure cannot block import readiness | Verified | H1 | E0, E2 |
| §6.1 — complete persona, world, progress, identity, and deviation prompt | Intended gap | H3 | E0, E3; the current Agent boundary consumes essentially the name plus other context |
| §6.1 — voice, identity, memory, and anti-spoiler prompt instructions | Intended gap | H3, H4 | E0, E3; prompt wording is not a behavioral guarantee |
| §6.2.3 — embedded long-term records and semantic retrieval | Intended gap | H3 | E0, E3 |
| §6.2.4 — no maintenance eviction and mandatory embeddings | Intended gap | H3 | E0, E3 |
| §6.2.4 — account/novel deletion erases permanent memory | Verified | H2, H5 | E6 |
| §6.4 — exact four-layer prompt composition and context-window truncation | Intended gap | H3 | E0, E3; current fixed bounds do not implement the full stated policy |
| §6.5 — idempotency key, fencing, commit-before-done, replay, and failure semantics | Verified | H3, H5 | E1, E3 |
| §7.1 — reuse an existing choice or present the canonical node | Verified | H4 | E4 |
| §7.2 — atomically persist choice/world/chapter while source remains immutable | Verified | H4 | E1, E4 |
| §7.3 — persist consequence and complete choice-origin player chapter | Verified | H4 | E4 |
| §7.3 — live output consistently meets tone and 100–400-word prompt constraints | Intended gap | H4 | E0; no qualified live journey corpus proves model compliance |
| §7.4 — no canonical fallback after divergence; player-scoped idempotent chapters/nodes | Verified | H4 | E1, E4 |
| §7.5 — atomic per-reader world-state mutation with character UUID keys | Verified | H4 | E1, E4 |
| §7.6 — immutable canon plus append-only player timeline and provenance | Verified | H1, H4 | E1, E4 |
| §7.6 — durable original player, validated world turns, commit-before-prose, shared timeline | Verified | H4 | E4; live causal quality remains unqualified |
| §8.1 — optional character identity mode | Aspirational | H4 | E0; compatibility behavior is not a supported agency promise |
| §8.2 — same-novel character identity and no same-character conversation | Verified | H4 | E1, E3 |
| §8.2 — self-mode choices preserve canonical-character agency | Verified | H4 | E4 |
| §9.1 — setup-gated registration, password validation, bcrypt, and token issuance | Verified | H2 | E5 |
| §9.1 — RFC 5321 email-address validation | Intended gap | H2 | E5 shows only a bounded application-level shape check, not RFC conformance |
| §9.2 — case-insensitive login, bcrypt verification, sign-in time, and tokens | Verified | H2 | E5 |
| §9.3 — HS256 access token and single-use refresh rotation | Verified | H2 | E5 |
| §9.4 — authenticated routes, owner isolation, and invalid-token 401 | Verified | H2 | E5; public-profile authorization testing remains an H2 gate |
| §9.4 — administrator cross-user resource access | Obsolete/corrected | H2 | Removed: downstream services enforce acting-user ownership and do not consume the injected role as an authorization bypass |
| §10.8 — scoped, bounded, ordered, secret-free, terminal-record account export | Verified | H2, H5 | E6 |
| §10.9 — every error uses the common JSON envelope | Intended gap | H2 | Current tests cover named paths but do not prove every service/error branch |
| §11.1 — local `.env` configuration is permitted | Verified | H0 | E0 and the checked-in startup paths |
| §11.2 — every listed tuning parameter is configurable | Intended gap | H3, H5 | Several memory limits remain fixed constants; defaults are not a current support claim |
| §12.1 — required PostgreSQL extensions | Verified | H1 | E1 |
| §12.2 — required indexes | Verified | H1 | E1 |
| §12.3 — versioned migrations, replay safety, and initial application | Verified | H1, H5 | E1 |
| §12.4 — durable erasure records written atomically with deletion | Intended gap | H1 | [`BACKUP_RESTORE.md`](./BACKUP_RESTORE.md) approved; no `erasure_records` journal exists yet |
| §12.4 — idempotent erasure replay in the migration path with bounded source-key re-queue | Intended gap | H1 | Policy approved; replay is unimplemented |
| §12.4 — restored deployments never serve a deleted subject within the retention ceiling | Intended gap | H1 | Policy approved; awaits the drill change judged by `backup-restore-v1` |
| §12.4 — encrypted, integrity-verified backup artifacts and fail-closed restore | Intended gap | H1 | Policy approved; no backup or restore script exists yet |
| §12.4 — erasure records excluded from export and free of content | Intended gap | H1 | Contract defined with the journal it governs |
| §13.1 — FSD import direction | Verified | H4 | E7 and frontend CI |
| §13.2–§13.3 — standalone import route/wizard and named progress component | Obsolete/corrected | H0 | Removed implementation prescriptions; the shelf and reader own those user outcomes directly |
| §13.3 — chat preserves reading context and branch choice blocks advancement | Verified | H4 | E7 |
| §13.4 — declared visual tokens and reading typography | Verified | H4 | E7; this is not WCAG qualification |
| §13.5 — POST SSE framing, exact key reuse, commit acknowledgement, and bounded retries | Verified | H3, H4 | E3, E7 |
| §13.5 — server-owned identity/progress and retired `user_id` rejection | Verified | H2, H4 | E3, E7 |
| §14.1 — every log is structured and carries the required trace fields | Intended gap | H2, H5 | E8; no exhaustive cross-service log contract test exists |
| §14.2 — liveness/readiness separation and Gateway aggregation | Verified | H2, H5 | E8 and Production Compose Smoke |
| §14.3 — private metrics route, bounded labels, and no private content | Verified | H2, H5 | E8; representative deployment collection remains unqualified |
| §14.3 — suggested `character_id` metric label | Obsolete/corrected | H0 | Removed the high-cardinality, linkable label suggestion from the normative contract |
| §15 — internal-only services, Nginx-only host ingress, and Gateway-only application ingress | Verified | H2 | E8 |
| §15 — secret length, bcrypt, upload validation, and managed object keys | Verified | H2 | E2, E5, E8 |
| §15 — untrusted prompt boundaries and model output cannot authorize commits | Verified | H2, H3, H4 | E2, E3, E4 provide structural evidence; live adversarial qualification remains open |
| §15 — all SQL remains parameterized | Verified | H2 | Current-source review found bound persistence queries; H2 still owns automated/static and dependency gates |
| §16 and former Appendix A — duplicate implementation, test, and prompt prescriptions | Obsolete/corrected | H0 | Removed stale copies; `AGENTS.md`, runtime validators/prompts, and Roadmap issues own those changing details |

## Implementation-defined selections

- Character extraction uses a representative first/middle/last sample and a
  bounded overlapping full-text chunk scan; successful chunk results are merged
  case-insensitively.
- Prompt context currently uses the ten most recent committed messages, creates
  a mid-term summary every twenty committed messages, retrieves up to five
  semantic results when embeddings are available, and has no connected
  production promotion path for long/permanent memories.
- Source bytes are retained only when S3 is enabled. Without retention,
  extraction is request-local and an interrupted import requires re-upload.

## Change and approval process

1. A normative change names its affected product claim, owner horizon,
   compatibility and lifecycle impact, evidence gate, and rollback.
2. The change updates `SPEC.md`, this ledger, and the checksum together.
3. Thresholds and supported slices are approved before the implementation they
   judge; the judged change cannot weaken its own gate.
4. Merge plus required CI establishes **Landed** evidence only. Live quality,
   recovery, security, accessibility, deployment, and observation retain their
   Roadmap gates.

The candidate [`qualification policy`](./QUALIFICATION_POLICY.md) owns the
journey/evaluation slices and threshold process. Open H0 work after these
candidates remains independent adversarial overclaim approval and the
clean-checkout verification entry point.
