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
| E1 | Schema and production repository contracts: [`init.sql`](../infra/postgres/init.sql), [`0021_world_turn_memory_projection.sql`](../infra/postgres/migrations/0021_world_turn_memory_projection.sql), [`pg_world_turn_repo.rs`](../services/narrative-service/src/infrastructure/persistence/pg_world_turn_repo.rs), [`pg_narrative_repo.rs`](../services/narrative-service/src/infrastructure/persistence/pg_narrative_repo.rs), [`pg_world_state_repo.rs`](../services/narrative-service/src/infrastructure/persistence/pg_world_state_repo.rs), [`repository_contracts.rs`](../tests/integration/tests/repository_contracts.rs), [`legacy_migration.rs`](../tests/integration/tests/legacy_migration.rs) |
| E2 | Import path and validation: [`handlers`](../services/novel-service/src/application/handlers/mod.rs), [`document.rs`](../services/novel-service/src/infrastructure/document.rs), [`character_extractor.rs`](../services/novel-service/src/domain/services/character_extractor.rs), [`novel-service tests`](../services/novel-service/src/tests.rs), and deterministic malformed-input property and regression tests (text/EPUB/PDF bytes, archive-bomb and expanded-text guards) |
| E3 | Durable chat and memory: [`agent handlers`](../services/agent-service/src/application/handlers/mod.rs), [`memory_manager.rs`](../services/agent-service/src/domain/services/memory_manager.rs), [`pg_memory_repo.rs`](../services/agent-service/src/infrastructure/persistence/pg_memory_repo.rs), [`pg_chat_repo.rs`](../services/agent-service/src/infrastructure/persistence/pg_chat_repo.rs), [`Agent memory HTTP contract`](../services/agent-service/src/interface/http/mod.rs), [`core_reader_loop.sh`](../tests/e2e/core_reader_loop.sh) |
| E4 | Canon and player timeline: [`narrative handlers`](../services/narrative-service/src/application/handlers/mod.rs), [`narrative HTTP contract`](../services/narrative-service/src/interface/http/mod.rs), [`narrative_node.rs`](../services/narrative-service/src/domain/entities/narrative_node.rs), [`world_session.rs`](../services/narrative-service/src/domain/entities/world_session.rs), [`narrative_transition.rs`](../services/narrative-service/src/domain/services/narrative_transition.rs), [`narrative tests`](../services/narrative-service/src/tests.rs), [`TOCTOU and concurrency tests`](../services/narrative-service/src/toctou_tests.rs), [`core_reader_loop.sh`](../tests/e2e/core_reader_loop.sh) |
| E5 | Identity and authorization: [`gateway auth`](../gateway/src/auth.rs), [`user handlers`](../services/user-service/src/application/handlers/mod.rs), [`auth_flow.rs`](../tests/integration/tests/auth_flow.rs) |
| E6 | Export and erasure: [`ACCOUNT_EXPORT.md`](./ACCOUNT_EXPORT.md), [`DATA_RETENTION.md`](./DATA_RETENTION.md), [`account_export.rs`](../tests/integration/tests/account_export.rs), [`auth_flow.rs`](../tests/integration/tests/auth_flow.rs) |
| E7 | Browser contract: [`client.ts`](../frontend/src/shared/api/client.ts), [`client.test.ts`](../frontend/src/shared/api/client.test.ts), [`queryClient.ts`](../frontend/src/shared/api/queryClient.ts), [`readerIdentityScope.ts`](../frontend/src/shared/lib/readerIdentityScope.ts), [`readerIdentityScope.test.ts`](../frontend/src/shared/lib/readerIdentityScope.test.ts), [`narrative api tests`](../frontend/src/entities/narrative/api.test.ts), [`ReaderPage.test.tsx`](../frontend/src/pages/reader/ui/ReaderPage.test.tsx), [`BranchChoice.test.tsx`](../frontend/src/widgets/branch-choice/ui/BranchChoice.test.tsx), [`ChatPanel.test.tsx`](../frontend/src/widgets/chat-panel/ui/ChatPanel.test.tsx), [`WorldDashboard.tsx`](../frontend/src/widgets/world-dashboard/ui/WorldDashboard.tsx), [`WorldDashboard.test.tsx`](../frontend/src/widgets/world-dashboard/ui/WorldDashboard.test.tsx), [`worldTurnStorage.ts`](../frontend/src/shared/lib/worldTurnStorage.ts), [`useAuthStore.test.ts`](../frontend/src/features/auth/model/useAuthStore.test.ts), [`reflow.spec.ts`](../frontend/e2e/reflow.spec.ts), [`globals.css`](../frontend/src/app/styles/globals.css) |
| E8 | Runtime and security configuration: [`ci.yml`](../.github/workflows/ci.yml), [`docker-compose.yml`](../docker-compose.yml), [`nginx.conf`](../infra/nginx/nginx.conf), [`THREAT_MODEL.md`](./THREAT_MODEL.md), [`bad_release_drill.sh`](../tests/e2e/bad_release_drill.sh), [`release.sh`](../infra/docker/release.sh), [`release_state_drill.sh`](../tests/e2e/release_state_drill.sh) |
| E9 | Backup, restore, lineage, and erasure replay: [`BACKUP_RESTORE.md`](./BACKUP_RESTORE.md), [`0016_erasure_records.sql`](../infra/postgres/migrations/0016_erasure_records.sql), [`backup.sh`](../infra/backup/backup.sh), [`restore.sh`](../infra/backup/restore.sh), [`backup_restore.rs`](../tests/integration/tests/backup_restore.rs), [`backup_restore_drill.sh`](../tests/e2e/backup_restore_drill.sh) |
| E10 | Gateway error normalization: [`proxy.rs`](../gateway/src/proxy.rs) (`NORMALIZED_ERROR_RESPONSES` and the pinned-contract tests) |
| E11 | Supply-chain gates: [`audit.toml`](../.cargo/audit.toml), [`Cargo.lock`](../Cargo.lock), [`.gitleaks.toml`](../.gitleaks.toml), [`deny.toml`](../deny.toml), [`scan-images.sh`](../infra/security/scan-images.sh), [`generate-sboms.sh`](../infra/security/generate-sboms.sh), the direct `cargo-audit`, `gitleaks`, `cargo-deny`, and trivy CI steps, and the Dependency Policy in [`SECURITY.md`](../SECURITY.md) |

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
| Language slices (H4 issue #173) — ingestion, generated narrative, UI locale | Verified | H4 | Declared slices with evidence: SC chapter splitting (novel-service test_chapter_split fixtures), EN chapter splitting (test_chapter_split_english_headers), SC + EN lore retrieval fixtures, generated narrative requires Chinese with English rejected fail-closed (node_detector `validate_detection(&english)` is_err; `validate_player_chapter("English only")` is_err), UI locale zh-CN (`lang=zh-CN` in index.html, asserted by the browser gate's html-has-lang check; Chinese UI copy with residual non-normative English artifacts recorded in PRODUCT_CONTRACT). Evidence is structural/automated only — representative live-provider language quality remains the recorded H2/H3 gap (QUALIFICATION_POLICY) |
| §4.1.1 — bcrypt cost at least 12 | Verified | H2 | E5 |
| §4.1.5 — required long-term and optional permanent search embeddings | Intended gap | H3 | E0, E3; long-term promotion writes only correctly dimensioned embeddings. Journey-fact ingress intentionally makes zero embedding calls and remains directly retrievable without a vector; durable optional permanent enrichment, live semantic/provider quality, and retrieval relevance remain gaps |
| §4.1.6 — chat-turn status, lease, failure, and completion fields agree | Verified | H3, H5 | E1, E3 |
| §4.2 — UUID v4 by default, the namespaced journey-memory UUID v5 exception, UTC `TIMESTAMPTZ`, and case-insensitive character deduplication | Verified | H1, H2, H3 | E1, E2, E3, E4; the client world-turn key remains UUID v4 while its private projection ID is deterministically derived under a fixed UUID v5 namespace, so the client-selected turn UUID is not reused as a memory primary key |
| §5.1 — accepted formats, byte limits, validation, optional retention, and pending record | Verified | H1 | E0, E2 |
| §5.1 — accepted imports recoverable after process death from committed `source`, `chapters`, or `enriched` stages; a `source`-stage job replays its retained object to rebuild chapters | Verified | H1 | E1, E2; durable jobs, leases, and startup recovery landed in PR #116, the retained-object replay slice (#125) reads the object through the `SourceFileStorage` port before any provider call, and a live kill drill at the `source` boundary passes in required CI ([PR #135](https://github.com/Wisdoverse/novelworld/pull/135)) |
| §5.2 — current parsing stages reach `ready` or store a terminal parse error | Verified | H1 | E2 |
| §5.2 — interruption recovery, fenced attempts, and reclaim of pending or expired jobs | Verified | H1 | E1, E2; attempt-fenced claims, lease reclaim, and replay-safe backfill landed in PR #116; the source-stage chapter replacement follows the same `(novel_id, attempt)` fence (#125) |
| §5.2 — provider calls across attempts stay inside the approved `import-provider-budget-v1` policy; ceiling jobs terminate with `budget_exhausted` and are never reclaimed | Verified | H1 | E1, E2; `claim_import` refuses the (ceiling+1)-th claim and terminally marks the job in one transaction, recovery excludes it, the retry endpoint surfaces the re-upload guidance without a provider call, and the kill drill asserts its forced attempts stay inside the ceiling (#129) |
| §5.3 — non-empty, sequential chapters | Verified | H1 | E2 |
| §5.4 — structured character output validation and case-insensitive merge | Verified | H1 | E2 |
| §5.4 — at most 50 characters per novel | Verified | H1 | E2; the merged extraction truncates to the 50 most prominent characters with deterministic ordering, and unit tests cover the cap and prominence retention |
| §5.4 — extract at least all characters who appear in more than one chapter | Intended gap | H1 | E2 covers the full-text chunk scan and case-insensitive alias merge; whether live extraction actually recovers every multi-chapter character remains quality-gated |
| §5.5 — world-summary persistence and non-empty validation | Verified | H1 | E2; enrichment commits the summary atomically with the chapter count |
| §5.5 — world summary covers setting, major factions, core conflict, and unique world rules | Intended gap | H1, H3 | E2; the extraction prompt now requests all four dimensions, but prompt wording is not a behavioral guarantee; live summary coverage remains quality-gated |
| §5.5 — 2,000-character world-summary maximum | Verified | H1 | E2; validate_extraction rejects longer summaries and unit tests cover the exact boundary |
| §5.6 — bounded, schema-valid node candidates, persistence, and 2–3 choices | Verified | H1, H4 | E2, E4; only Simplified Chinese is in the current generated path |
| §5.7 — avatar failure cannot block import readiness | Verified | H1 | E0, E2 |
| §5.8 — extraction quality gates per supported positive slice (coverage, precision/hallucination, chronology, provenance, anti-vacuity, deterministic recorded evaluation) | Intended gap | H1 | E0, E2; the [`extraction-quality-v1`](./EXTRACTION_QUALITY.md) policy is approved, and its corpus and evaluator landed as [`tools/h1-eval`](../tools/h1-eval/): recorded mode proves the structural gate, corpus/rubric integrity, and calibration self-consistency in CI, and live mode enforces the versioned thresholds fail-closed; no provider/model has passed a live run |
| §6.1 — complete persona, world, progress, identity, and deviation prompt | Intended gap | H3 | E0, E3; the persona slice now flows from the extracted fields (aliases, role, description, personality, background, speaking style) as truncated, JSON-quoted data ([PR #143](https://github.com/Wisdoverse/novelworld/pull/143)). Recorded limits: syntactic-only inertness and whole-novel (non-spoiler-bounded) persona extraction are recorded, not claimed as H3 exit evidence; world/identity/deviation completeness and live quality remain gap |
| §6.1 — voice, identity, memory, and anti-spoiler prompt instructions | Intended gap | H3, H4 | E0, E3; prompt wording is not a behavioral guarantee |
| §6.2.3 — embedded long-term records and semantic retrieval | Intended gap | H3 | E0, E3; the production mid-summary path attempts embedded long-term promotion and the prompt performs bounded semantic retrieval, but live relevance and lifecycle quality remain unqualified |
| §6.2.4 — durable direct permanent retrieval plus optional embedding and no maintenance eviction | Intended gap | H3 | E0, E3, E4; deterministic tests cover authenticated fact-first insertion, exact replay, mismatch rejection, zero-call independence from slow embedding, restart and chapter scope, fixed UUIDv5, de-duplication, explicit witness filtering, strict counts/shapes, independent journey/legacy candidate buckets, whole-entry budget, and causal display order. Direct and semantic prompt paths admit journey facts only for a persisted `self` identity snapshot. Character mode omits all unprovenanced mid/long/permanent/semantic memory and projection, keeping only exact same-character recent committed chat; legacy/unclaimed chat remains self-only. First-adoption migration terminal-skips unverifiable pre-contract completed turns and retains old memory rows; both paths quarantine the former permanent/importance-7/UUID-v4 producer class, which can hide a legitimate legacy row in that narrow class. Permanent semantic enrichment, historical fact backfill, continuous late-compensation selection, and live lifecycle value remain unqualified |
| §6.2.4/§7.6 — committed-world-turn projection reaches durable `saved`/`skipped`, while `pending` replay/recovery compensates | Verified | H3, H4 | E1, E3, E4; readiness verifies exact columns, constraints, and the one-unresolved-turn unique authority slot. A new row starts `pending`; another key for the same user+novel cannot advance until explicit-witness `saved` or conclusive `skipped`. Same-key replay and Narrative's bounded durably rotating scan reuse the same identity/source-visibility checks, deterministic UUIDv5 Agent write, and terminal CAS; terminal replay avoids Agent, while scan rotation prevents an unresolved batch from starving later rows. Source-high-water, identity, and visibility races after commit preserve `pending` and return a content-free unknown/conflict |
| §6.2.4 — account/novel deletion erases permanent memory | Verified | H2, H5 | E6 |
| §6.4 — bounded four-layer prompt composition, explicit character visibility, current-progress recheck, and total prompt budget | Intended gap | H3, H4 | E0, E3, E4; deterministic tests cover source bounds, de-duplication, fail-closed aggregate size, selection of the latest four directly targeted `converse`/`ally`/`oppose` actions from a bounded 100-turn journal scan, independent Narrative/Agent action kind+target rejection, and producer/Agent-consumer event exclusion unless its actor list names the character. The provider prompt is a tested allowlist: choices, unscoped threads, player name/location, routing UUIDs, and technical metadata are excluded; character-specific state/actions/events remain. All derived context is omitted when its source high-water is absent or exceeds rewound progress. Inferred visibility, visibility beyond that scan, live relevance, and complete lifecycle quality remain unqualified |
| §6.5 — idempotency key, fencing, commit-before-done, replay, and failure semantics | Verified | H3, H5 | E1, E3 |
| §7.1 — exact committed replay or per-user first-writer node presentation with final eligibility recheck | Verified | H4 | E1, E4; runtime branch prompts contain Player/world/mode context, so canonical-source and divergent nodes are both user-owned. Deterministic tests prove two users receive distinct nodes/provider calls without private prompt crossover; uncommitted legacy shared nodes fail closed while committed ones replay without provider work; cache/provider races with Player sealing, a concurrent choice, or open-world entry return no unusable options. PostgreSQL concurrency evidence proves competing/repeated saves reload one immutable durable id/options/created-at tuple |
| §7.2 — exact same-index choice replay, different-index conflict, sealed Player checkpoint, and atomic choice/world/chapter persistence while source remains immutable | Verified | H4 | E1, E4, E7; the natural `(user_id, node_id)` key is checked before generation and under the competing world-state lock. A new choice must match the deterministic world-state fingerprint used for generation and advance strictly beyond the latest choice chapter; stale/same-or-earlier drafts conflict with no partial rows, while exact replay bypasses new-choice guards. Different Player checkpoints race to one winner, choice/open-world races admit only valid serializations, later choices cannot exceed the sealed checkpoint, and every new choice after open-world entry conflicts |
| §7.3 — persist consequence and complete choice-origin player chapter | Verified | H4 | E1, E4; a choice-origin projection atomically replaces an older same-chapter continuation, a mismatched choice projection rolls back the transaction, and legacy exact replay rebuilds missing/continuation prose from immutable canon, the anchor, and committed consequence without another provider call |
| §7.3 — live output consistently meets tone and 100–400-word prompt constraints | Intended gap | H4 | E0; no qualified live journey corpus proves model compliance |
| §7.4 — no canonical fallback after divergence; player-scoped idempotent chapters/nodes | Verified | H4 | E1, E4 |
| §7.5 — atomic per-reader world-state mutation with character UUID keys, state-fingerprint fencing, and monotonic choice chapters | Verified | H4 | E1, E4 |
| §7.6 — immutable canon plus append-only player timeline and provenance | Verified | H1, H4 | E1, E4 |
| §7.6 — durable original player, validated world turns, commit-before-prose, ordered continuation, and same-key browser recovery | Verified | H4 | E1, E4, E7; the first committed open-world entry context is first-writer-wins and later valid starts resume it. The current single-timeline prompt receives the last four committed actions and bounded narrative endings, thread actions require an open target, and v1 committed transitions remain replayable. Player/world reads, new turns, replays and final choice/node responses recheck server progress; a rewind below source high-water returns content-free `reading_progress_behind_world`, while effective chapters safely return immutable canon with `generated=false` and recheck after in-flight generation. The browser synchronously replaces cached generated prose with canon and refetches on progress change. Ambiguous delivery/lease/projection outcomes retain the original idempotency key; terminal POST includes `saved`/`skipped`, while older response shapes confirm the journal. With writable `sessionStorage`, a bounded per-user-and-novel record restores the same action/key across a same-tab reload and locks every editable control. Confirmed principal transitions synchronously clear private query/chat state; delayed old-principal mutation results only invalidate/refetch active current-principal truth and cannot cache their response. Successful authentication keeps the confirmed principal's recovery keys and removes other principals'; logout, successful account deletion, missing credentials, or confirmed `401`/`403` clears private cache and all recovery keys. Transient auth/deletion failure retains them. Successful shelf removal clears only the current user+novel key; failed removal retains it, and unrelated storage remains untouched. Blocked browser storage limits recovery to the current mount |
| §7.6 — branch, chapter, chat, journal, and world-turn consumers form one live-quality causal system | Intended gap | H4 | E0, E4, E7; after open-world entry, branch choices are deliberately omitted until they carry per-character witness provenance; directly targeted actions and producer/consumer-validated explicit-actor events form the current character context. A canonical source high-water suppresses all world-derived responses/UI after a reading rewind, and the UI distinguishes decisions/actions from generated projections. Pre-open-world branch-to-chat continuity, exact chat/world-revision provenance under concurrent commits, inferred visibility beyond explicit IDs, live provider/lifecycle evidence, a qualified long trajectory, and manual/non-author accessibility evidence remain gaps |
| §8.1/§8.2 — character-identity agency boundary defined and qualified | Intended gap | H4 | The current structural candidate rejects character identity at every Player/open-world handler before provider or write side effects, returns choices-only WorldState before high-water evaluation, and fences chat on the persisted claim: character turns never call the Player world-context port even after a concurrent switch to self, and exact-character history is filtered in PostgreSQL before pagination. Character mode omits unprovenanced mid/long/permanent/semantic memory and derived projection; self mode alone retains legacy/unclaimed chat. Prior self-mode Player/world state survives identity switches, and character mode permits only exact read/replay of an already committed branch result—never a new node or choice. Existing repository/export/browser evidence also keeps identity same-novel, portable, and separate from canon. This is not yet qualified: branch perspective and node-cache identity are not durably keyed/fenced, general cross-service reads have no identity revision, and long-lived identity-transition semantics remain unresolved |
| §8.2 — same-novel character identity and no same-character conversation | Verified | H4 | E1, E3 |
| §8.2 — self-mode choices preserve canonical-character agency | Verified | H4 | E4 |
| §9.1 — setup-gated registration, password validation, bcrypt, and token issuance | Verified | H2 | E5 |
| §9.1 — RFC 5321 email-address validation | Verified | H2 | E5; `is_valid_email` implements the RFC 5321 §4.1.2 mailbox grammar — dot-string and quoted-string local parts (with quoted-pairs), LDH domain labels (1–63 octets, no edge hyphens), or IPv4/IPv6 address literals — with the §4.5.3.1 limits (local ≤ 64, domain ≤ 255, total ≤ 254 octets) and an ASCII-only gate applied to the raw input before the whole-address lowercase normalization, so no Unicode case fold slips through; tests pin each grammar branch, every boundary, and an independently restated atext set; validation is syntactic only (no MX/delivery check — not required); dotless domains are accepted, the RFC 5321 general-address-literal (unknown standardized tags) is rejected as unsupported, and legacy accounts registered under the old check still log in because §9.2 does not re-validate |
| §9.2 — case-insensitive login, bcrypt verification, sign-in time, and tokens | Verified | H2 | E5 |
| §9.3 — HS256 access token and single-use refresh rotation | Verified | H2 | E5 |
| §9.4 — authenticated routes, owner isolation, and invalid-token 401 | Verified | H2 | E5 plus the executable gateway route/resource authorization matrix: the public-path set is pinned exactly and every protected family is checked behaviorally (missing/garbage token 401, valid token passes) against the real router; public-profile authorization testing remains an H2 gate |
| §9.4 — administrator cross-user resource access | Obsolete/corrected | H2 | Removed: downstream services enforce acting-user ownership and do not consume the injected role as an authorization bypass |
| §10.8 — scoped, bounded, ordered, secret-free, terminal-record account export | Verified | H2, H5 | E6 |
| §10.9 — every error uses the common JSON envelope | Verified | H2 | E8, E10; the gateway normalization table pins every mapped upstream status to the stable error envelope and is tested exhaustively against an independently pinned contract (drift fails the build), with unmapped fallbacks (server → `internal_error`, client → `request_error`) and Retry-After/WWW-Authenticate forwarding; every client/server upstream response flows through this normalization — already-stable envelopes pass through byte-identical — so the public API never returns a non-envelope error body; gateway-generated errors call the same envelope helper; downstream services' own named error branches remain spot-checked, and their non-envelope bodies are rewritten at the gateway; at the nginx edge, gateway-down 502/503/504 serve the same envelope — 502 verified end to end by the bad-release drill in required CI ([PR #135](https://github.com/Wisdoverse/novelworld/pull/135); stop/start with security headers intact), 503/504 are mapped by the same config but not exercised, and mid-stream SSE failures are out of scope because response headers are already sent |
| §11.1 — local `.env` configuration is permitted | Verified | H0 | E0 and the checked-in startup paths |
| §11.2 — every listed tuning parameter is configurable | Intended gap | H3, H5 | Several memory limits remain fixed constants; defaults are not a current support claim |
| §12.1 — required PostgreSQL extensions | Verified | H1 | E1 |
| §12.2 — required indexes | Verified | H1 | E1 |
| §12.3 — versioned migrations, replay safety, initial application, and schema-version barriers | Verified | H1, H5 | E1, E8; migration 0021 is explicitly transactional and replay-safe under a hostile search path. Its PostgreSQL 18 contract proves first adoption terminal-skips pre-contract completed turns without deleting legacy memory rows, a later new-protocol pending row survives replay, and both exact PostgreSQL 18 restore spellings of the new constraints reach readiness while weakened/extended constraints fail closed. Release tooling stops old Narrative before exposing the candidate client, pins the gate to a retryable world-action `5xx` that retains the exact key, then drains Agent before migration; rejection restores the current Narrative/client tuple or fail-stops the client. A durable exact-target marker is written and synced immediately before the migrator; every marked transition rolls that target forward, promoted state is synced before marker removal, and removal is synced again. A 0021 transition therefore never starts its older writer. Interrupted upgrades preserve old current as previous; initial adoption supports a missing candidate without inventing previous, mismatched candidates fail closed, and rollback refuses a pending transition or crossing the barrier. It requires a control-only first stage for old tooling and refuses every path that would activate a pre-0021 writer. Local launchers run Compose down without volume deletion before up so old writers stop and the one-shot migrator is recreated; their ordering is self-tested. The desktop list embeds 0021 and applies it after stopping pg0/validating free ports but before spawning services |
| §12.4.1 — erasure records atomic with every user or canonical-novel deletion path, identifying payload limited to subject type and UUIDs with content-free bookkeeping fields, cascade-surviving | Verified | H1 | E9; `AFTER DELETE` row triggers on `users` and `novels` write inside the deleting transaction. Shared canonical novels no longer cascade from uploader deletion; drill B exercises account deletion and explicit canonical-novel deletion independently. The journal has no foreign keys, and its column set is asserted directly: subject type and UUIDs identify, while `erased_at`, `had_source` and `source_requeued_at` are named bookkeeping carrying nothing derived from content, profile or credentials |
| §12.4.2 — migration-path replay before services start, idempotent, removes matching subject rows, exactly-once source-key re-queue per database lineage with durable per-record bookkeeping (at most one repeat per restore) | Verified | H1 | E9; replay runs in the migration container the services wait on, and both the integration tests and drill B replay it twice with no further effect. The re-queue is gated on the record's own `had_source`, recorded by the delete that could still see the key, so a key is reconstructed for a subject row this database never held — proven with no other retained-source evidence present — and never enqueued speculatively for a novel that held none. Stamps restored from the artifact's own dump are retained, so those records repeat zero re-queues; a record from a foreign sidecar source enters unstamped and re-queues once. Deletions predating the migration are declared unknowable and are not backfilled |
| §12.4.3 — artifacts embed same-snapshot erasure exports with covered-through timestamps; restore stops writes, replays the union of sources, aborts on conflicting deletion facts with the retained-source marker merging monotonically, establishes live continuation only by equality of the create-once version-4 lineage token (manifest and dump tokens must agree with asymmetric absence aborting; wholly token-less artifacts restore through the disaster gate; restores regenerate the token atomically with reachability, recording the artifact token — or its recorded absence — as parent), refuses non-empty residual windows except through attest-or-erase (every account not covered by a collected record is retained with all private state or erased; shared canonical novels remain independent subjects; collected-record accounts get automatic `replayed` attestation rows; all rows are durably recorded with subject, decision, both window bounds, the verified artifact digest inventory, operator identity, and timestamp), never contains an undecided account or serves one covered by any erasure record, rotates the JWT secret and deletes all persisted refresh tokens after verification and before services start, clears runtime configuration on final-account removal, and requires designating a retained administrator when decisions would leave none | Verified | H1 | E9; the sidecar export and the manifest's lineage token are both cut out of the single `pg_dump` stream, so manifest and dump agree by construction and the export cannot diverge from the dump; the covered-through timestamp is read immediately before that snapshot opens — never the archive-write time, and conservatively early, so a derived window is a superset. Drill C proves the token lifecycle end to end: migration replay preserves the token, two restores of one artifact produce distinct tokens each recording the artifact's as parent, a manifest disagreeing with its dump and an asymmetric absence are refused, a wholly token-less artifact restores only through the gate with an absent parent, and a failure injected before the atomic load/regenerate commit leaves no reachable data while one injected after it leaves the regenerated token — both retries face the gate. Continuation is token equality alone, so an unrelated or sibling database is gated. Collected-record accounts are excluded from the decisions the operator supplies, are rejected if named, and receive automatic `replayed` attestation rows with the full field set. Manifest and decision UUIDs, digests and timestamps are shape-checked whole-value, with timestamps additionally calendar-checked by the server that stores them, and the free-form operator identity is quote-doubled, an inverted window aborts, and the recorded inventory names only digests this run verified, labelled by what they cover |
| §12.4.4 — scripted, encrypted, integrity-verified artifacts and fail-closed restore on corrupt or unverifiable input | Verified | H1 | E9; AES-256-CBC with PBKDF2 at 200 000 iterations, a SHA-256 manifest verified before any data change, artifacts written under temporary names and renamed only once all three outputs exist, and drill negatives for a corrupted artifact, a wrong key, and tampered manifest metadata. The ≤ 30 minute RTO scale rehearsal is not part of this evidence: [`scale_rehearsal.sh`](../infra/backup/scale_rehearsal.sh) is tooling only, never runs in CI, and its recorded run remains separate release evidence |
| §12.4.5 — erasure records excluded from account export and free of source text, messages, profile data, and credentials | Verified | H1 | E9; the journal's column set is asserted directly, and both the production export port and the end-to-end export exclude erased subjects and the journal itself |
| §13.1 — FSD import direction | Verified | H4 | E7; `frontend/scripts/check-fsd.mjs` (required frontend CI via `pnpm lint:fsd`) rejects upward imports between the six recognized layer directories — alias `@/`, relative, side-effect, and dynamic imports — with a self-test proving alias-upward, relative-upward, shared-ceiling, side-effect, and dynamic cases fail while downward/same-layer and comment/string mentions pass; structural enforcement only, slice quality remains a design-review matter |
| §13.2–§13.3 — standalone import route/wizard and named progress component | Obsolete/corrected | H0 | Removed implementation prescriptions; the shelf and reader own those user outcomes directly |
| §13.3 — chat preserves reading context and branch choice blocks advancement | Verified | H4 | E7 |
| §13.4 — declared visual tokens and reading typography | Verified | H4 | E7; the browser-based gate (issue #167, required CI job `Frontend Browser A11y`) scans the critical journey (home, login, shelf, reader guided + open world, player entry, characters, settings, setup) in real Chromium with the full axe-core 4.13 WCAG 2.2 AA rule set — color-contrast computed from real styles, page-level rules (document-title, html-has-lang, meta-viewport) on the real index.html — plus a real-keyboard tab walk asserting reachability and visible focus indicators and a 320px reflow assertion; the current structural candidate passes 30 tests (9 page scans, 3 character-identity boundary tests, 3 gate-integrity tests, 8 keyboard tests, and 7 reflow tests) with zero reported axe violations. Real fixes it surfaced and landed: ChatPanel control aria-labels + 24px targets + empty-state contrast; NovelCard title as a keyboard-operable button; shelf meta/genre badge contrast; branch-choice and reader character-list purple (#6d28d9 -> #a78bfa); open-world start button and role badges (#0891b2 -> #0e7490); settings checkbox/API-key target size; a global `:focus-visible` outline. Determinism: Google Fonts are aborted by the suite so local and CI scan the same fallback-font rendering after `document.fonts.ready`; web-font contrast is recorded as a manual-review item. The jsdom axe scans (issue #165) were removed in favor of this single canonical suite. Still not WCAG qualification: manual keyboard/screen-reader/mobile review and the non-author golden journey remain open (H4), as do landmark structure on data-filled pages and web-font contrast |
| §13.5 — POST SSE framing, exact key reuse, commit acknowledgement, and bounded retries | Verified | H3, H4 | E3, E7 |
| §13.5 — server-owned identity/progress and retired `user_id` rejection | Verified | H2, H4 | E3, E7 |
| §14.1 — every log is structured and carries the required trace fields | Verified | H2, H5 | E8 plus the e2e log-contract checker: every service enters a post-init service span carrying service + trace_id (empty outside requests), the gateway middleware accepts/generates X-Trace-Id, echoes and forwards it, and each downstream trace middleware wraps requests in a span carrying it; the checker parses every service-owned stdout line for timestamp/level/message/service/trace_id, requires request-scoped trace ids per service, and proves end-to-end propagation by stamping a known X-Trace-Id and asserting the downstream service logs it |
| §14.2 — liveness/readiness separation and Gateway aggregation | Verified | H2, H5 | E8 and Production Compose Smoke |
| §14.3 — private metrics route, bounded labels, key-scoped usage visibility, and no private content | Verified | H2, H5 | E8; billable series use only a one-way actual-key fingerprint, user-service filters the current platform/personal key server-side, and readers using the platform fallback cannot query its aggregate; representative deployment collection remains unqualified |
| §14.3 — suggested `character_id` metric label | Obsolete/corrected | H0 | Removed the high-cardinality, linkable label suggestion from the normative contract |
| §15 — internal-only services, Nginx-only host ingress, and Gateway-only application ingress | Verified | H2 | E8 |
| §15 — secret length, bcrypt, upload validation, and managed object keys | Verified | H2 | E2, E5, E8 |
| §15 — untrusted prompt boundaries and model output cannot authorize commits | Verified | H2, H3, H4 | E2, E3, E4 provide structural evidence; live adversarial qualification remains open |
| §15 — all SQL remains parameterized | Verified | H2 | Current-source review found bound persistence queries; H2 still owns automated/static and dependency gates |
| §15 — known-vulnerability dependency gate | Verified | H2 | E11; the AWS SDK uses its current `default-https-client` path, so the redundant legacy `rustls` feature no longer pulls vulnerable rustls-webpki 0.101.7 or h2 0.3.27; a local cargo-audit 0.22.2 run against the current `Cargo.lock` is clean with no vulnerability ignores in `.cargo/audit.toml`; jsonwebtoken uses its `aws_lc_rs` backend so the rsa crate is not in the tree; CI runs `cargo-audit` directly against the committed `Cargo.lock` so any new advisory fails the build; informational warnings (ttf-parser unmaintained, lru unsound pop) remain non-failing and re-reviewed on chain updates; deploy-time SBOM verification, provenance/attestation, and signature gates remain open H2 items |
| §15 — committed-secret scanning gate | Verified | H2 | E11; a local gitleaks 8.24.3 scan over the full commit history is clean under `.gitleaks.toml` — the committed default rule set uses regex-escaped upstream examples so historical false-positive fixtures remain allowlisted without committing complete credential-shaped literals, plus narrow allowlists for the CI `RUNTIME_CONFIG_KEY` smoke placeholder and two static provider model names (one history-only) — and the gate's detection strength is pinned by a self-test that plants a GitHub-shaped token, asserts the scan fails, and asserts the repository stays clean; CI runs the pinned `gitleaks:v8.24.3` image directly over the checkout with the committed `.gitleaks.toml` plus the runtime-token self-test |
| §15 — dependency license and source gate | Verified | H2 | E11; a local cargo-deny 0.20.2 `check licenses sources` passes with `deny.toml` — every dependency license satisfies the explicit permissive allow set, unlicensed is denied by default (the nine workspace crates now declare `license = "MIT"` matching the repo LICENSE), and unknown registry/git sources are denied; no exceptions are needed (dual MIT/Apache-2.0 crates and r-efi's MIT OR Apache-2.0 OR LGPL-2.1-or-later are satisfied without choosing the copyleft branch); CI adds the pinned `EmbarkStudios/cargo-deny-action@v2.1.1` step (required CI green, [PR #135](https://github.com/Wisdoverse/novelworld/pull/135)); advisories remain owned by cargo-audit |
| §15 — container image scanning gate | Verified | H2 | E11; local trivy 0.68.1 scans (HIGH/CRITICAL, vuln scanner) of all six application images report zero findings, the four Dockerfile base images are now digest-pinned, and the tag pipeline (docker.yml) scans every pushed image with `--exit-code 1` so a finding fails the release (v0.1.0 pipeline green: [https://github.com/Wisdoverse/novelworld/actions/runs/32241695893], six images scanned clean); the digest-pinned infrastructure images are scanned when re-pinned through the separately approved procedure — the current pinned `pgvector/pgvector@sha256:69167330…` reports 22 findings (21 HIGH, 1 CRITICAL, CVE-2025-68121) inside its bundled gosu binary, fixed in go 1.24.13 but not yet rebuilt into the pinned image, tracked for the next infrastructure re-pin (gosu runs only as the postgres entrypoint's privilege-drop helper and does not exercise the affected Go TLS path) |
| §15 — SBOM generation | Verified | H2 | E11; the release pipeline generates one CycloneDX 1.6 SBOM per application image with the pinned trivy release and ships them with the release artifact, bound to the recorded image digest via `sboms/digests.txt`; `infra/security/generate-sboms.sh` is the local form — all six local SBOMs generated and validated (CycloneDX 1.6, 72–107 components, digest sidecar) — and the v0.1.0 release ships six pipeline-generated CycloneDX SBOMs bound to the recorded digests in `sboms/digests.txt` ([https://github.com/Wisdoverse/novelworld/releases/tag/v0.1.0], [https://github.com/Wisdoverse/novelworld/actions/runs/32241695893]); deploy-time SBOM verification, provenance/attestation, and signatures remain open |
| §16 and former Appendix A — duplicate implementation, test, and prompt prescriptions | Obsolete/corrected | H0 | Removed stale copies; `AGENTS.md`, runtime validators/prompts, and Roadmap issues own those changing details |

## Implementation-defined selections

- Character extraction uses a representative first/middle/last sample and a
  bounded overlapping full-text chunk scan; successful chunk results are merged
  case-insensitively.
- Self-mode prompt context currently retrieves two independently bounded permanent
  candidate buckets: the ten most recently created UUIDv5 journey candidates
  and ten non-v5 legacy rows by importance/recency. Rust then authenticates the
  structured candidates and budgets at most ten permanent entries in total.
  It also retrieves up to five recent/important mid memories, up to five
  semantic results after direct permanent de-duplication, and the ten most
  recent committed messages. Character mode instead retrieves only recent chat
  backed by completed claims for the exact same persisted character identity;
  it omits unprovenanced memory layers and creates no derived memory projection.
  Legacy/unclaimed chat remains available only in self mode. Self mode
  creates a mid-term summary every twenty committed messages, attempts
  best-effort mid-to-long promotion, and gives every committed open-world turn
  a durable memory-projection state. `pending` is compensated by exact-key
  replay or a bounded durably rotating Narrative scan; a different key cannot
  acquire the same user+novel authority slot, and `saved`/`skipped` are
  terminal. Both paths reuse the existing deterministic Agent write and
  terminal CAS. Projection eligibility is followed by source-high-water and
  self-identity rechecks before terminal acknowledgement, so a concurrent
  rewind or identity change returns a content-free conflict and preserves
  `pending` until a later safe scan or exact replay. Authenticated facts receive
  whole-entry budget
  before legacy prose and are then shown in causal order. A late-compensated
  early turn can still fall outside the ten-row journey-candidate window, so
  this is not a continuous long-trajectory guarantee.
- Permanent embedding policy (H0 contract decision, H3 evidence): embedding is
  an optional search projection over an authoritative directly retrievable
  fact. Compatibility relies on the already-nullable memory embedding column
  and existing direct reader; retention/deletion is unchanged. New journey
  facts use a fixed private UUID v5 namespace over the client UUID v4 turn key
  and contain only structured action/validated mutations. The prompt consumer
  also verifies that UUID against the embedded turn ID plus local
  layer/importance/character/user/novel/witness/chapter metadata, validates
  counts and character-specific action/event/relationship shapes, and requires
  the input to equal the known-field provider-safe representation. That shared
  gate runs before insertion/`saved: true` and during retrieval; unknown or
  otherwise non-equivalent protocol content is rejected and omitted rather
  than partially accepted. JSON content alone cannot claim authority. Migration
  0020 adds `world_turns` projection status, acknowledgement time, and the
  unresolved-turn uniqueness contract. Its first adoption terminal-skips
  pre-contract completed turns instead of fabricating witness facts and retains
  their memory rows for export and lifecycle deletion. Agent direct and semantic
  prompt consumers quarantine the former permanent/importance-7/UUID-v4
  producer class; later replay never skips a new pending turn. A PostgreSQL 18
  test proves first adoption, replay safety, and lossless row preservation.
  E1/E3/E4 deterministic tests cover fact-first insertion,
  concurrency, restart, scope, zero-call independence from embedding
  availability/latency, terminal replay, and
  explicit-witness skipping. Rollback must preserve existing nullable-
  embedding facts and cannot promote legacy prose into authoritative facts.
- The current character-visibility selection is explicit-ID-only: an action is
  visible only when its kind is `converse`, `ally`, or `oppose` and its target
  is the character UUID. Only that observable kind+target pair crosses the
  boundary; the reader's free-form `intent` is omitted as private motivation.
  Player-origin events use their own actor IDs; numeric
  relationship score does not expose the originating action or private intent.
  Relationship-change prose, perception, and model-generated reasons are
  omitted because they have no independent per-character witness provenance.
  Narrative and Agent both enforce the action kind+target rule, and Agent also
  checks event actor membership. The provider-facing JSON is a separate
  allowlist: it excludes player/routing UUIDs, protocol/model fields, player
  name/location, choices, and active threads; only the relationship score,
  character goals and canonical event,
  directly targeted actions, and actor-listed events remain. This model does not
  infer co-location, line of sight, hearsay, or later knowledge propagation.
  The producer scans at most 100 committed turns and returns the latest four
  matching actions in causal order; older visibility remains outside this
  structural slice. Branch choices are omitted because they do not carry
  per-character witness provenance. The producer attaches the committed
  session's unlocked-through chapter as a
  canonical source high-water; Agent omits the entire derived world view when that mark is
  absent or beyond latest persisted reading progress.
- A `PlayerEntity` checkpoint is the upper branch-choice boundary. Existing
  branch choices or chapter-tagged player events beyond a proposed checkpoint
  reject player creation; a new choice beyond the stored checkpoint rejects,
  and all new branch choices reject after open-world entry. Exact same-index
  replay is evaluated before those new-choice guards but not before the durable
  choice/projection consistency fence. Player creation, open-world entry,
  choice commit/replay, and world-turn reservation/completion require a
  one-for-one exact match between durable node-backed choices and the JSONB
  projection; missing, duplicate, unkeyed, malformed, or mismatched legacy
  entries return a typed conflict with no new authoritative write. Every new
  world action also revalidates the full committed prefix against the sealed
  entry checkpoint; inconsistent legacy state conflicts before provider
  invocation or world commit rather than projecting a fact at a lower chapter.
- Browser principal transitions fence delayed protected responses by the
  initiating bearer and, when known, user id. Tests hold an A-session `401`,
  identity refresh, account deletion, and account export across a B login: B's
  token, query/chat cache, and pending action survive, and A's export never
  reaches Blob/download APIs. An `auth_token` storage event clears query/chat
  state and requests reload/re-authentication; unchanged-token and
  refresh-token-only events are ignored.
- The exact schema-transition manifest, rather than a downloaded candidate,
  determines post-migration recovery. Every marked transition rolls that exact
  target forward; a 0020 transition cannot restore the older writer. The marker
  is synced before migration, the installed target tempfile and former current
  renamed as previous are synced before current replacement, promotion is
  synced before marker removal, and removal is synced again. Recovery accepts a
  missing candidate and preserves the former
  current as previous. Initial adoption rejects a previous release, and both
  paths reject a different candidate or a pre-0020 initial marker. Promotion
  occurs only after the idempotent migration and health gate succeed. Normal
  restore and healthy rollback first discard any unmarked candidate, write the
  exact schema marker, and clear it only after durable finalization; rollback
  reuses the same promotion path, while legacy rollback-marker recovery is
  retained only for compatibility. Static ordering plus fake-dependency state
  tests cover healthy restore/rollback and process interruption; real Linux
  host power-loss/directory-durability injection and live image health remain
  unqualified.
- Migration and Narrative readiness accept the unresolved-turn authority index
  only with exact relation/definition plus `indisunique`, `indisvalid`,
  `indisready`, and `indislive`. A failed concurrent unique build leaves an
  exact-name invalid index; the integration drill proves readiness rejects it,
  replay replaces it, all flags become true, and duplicate unresolved authority
  is rejected again.
- After an open-world source high-water exists, server reads and final
  generated responses recheck current progress. Behind that boundary, player
  entry/world state/session/turn/choice/node responses are content-free typed
  conflicts. Effective chapter reads are the exception: they return immutable
  canon with `generated=false`, recheck after in-flight generation, and reveal
  the durable player projection again only after rereading. The browser applies
  the same rule synchronously to cached prose and refetches after progress
  changes.
- World-action recovery persists one bounded, schema-validated `(action,
  Idempotency-Key)` pair per user and novel in browser `sessionStorage`. It is
  retained while a matching journal row remains projection-`pending`; a
  terminal POST reports `saved`/`skipped` directly, with journal confirmation
  retained for older response shapes. It is otherwise cleared only by terminal
  projection status or explicit terminal rejection. Authentication principal
  confirmation keeps the confirmed principal's recovery keys and removes other
  principals', while synchronously clearing private query/chat cache before the
  new principal is exposed. Delayed old-principal mutation results cannot write
  their private response into cache; they only invalidate/refetch active
  current-principal truth. Logout, successful account deletion, missing
  credentials, or a confirmed `401`/`403` clears private cache and all recovery
  keys. Transient authentication/deletion failure retains both. Successful
  shelf removal clears only the exact current
  user+novel key; failed removal retains it, and unrelated storage is preserved.
  This survives a reload/remount in that tab
  session only; it is not cross-device
  persistence or a server background-reconciliation mechanism. It requires a
  writable storage API; if the browser blocks it, the current mounted form
  still locks but remount/reload recovery is outside the supported envelope.
- Source bytes are retained only when S3 is enabled. With retention, file
  imports accept at the `source` stage and rebuild deterministic chapters from
  the retained object (format resolved from validated-bytes magic). Without
  retention, chapter splitting is request-local and an interrupted import
  requires re-upload.

## Change and approval process

1. A normative change names its affected product claim, owner horizon,
   compatibility and lifecycle impact, evidence gate, and rollback.
2. The change updates `SPEC.md`, this ledger, and the checksum together.
3. Thresholds and supported slices are approved before the implementation they
   judge; the judged change cannot weaken its own gate.
4. Merge plus required CI establishes **Landed** evidence only. Live quality,
   recovery, security, accessibility, deployment, and observation retain their
   Roadmap gates.

The approved [`qualification policy`](./QUALIFICATION_POLICY.md) owns the
journey/evaluation slices and threshold process.

## Recorded H0 reviews

| Perspective | Reviewer | Disposition | Evidence and unresolved risks |
|---|---|---|---|
| Current-truth | Fresh-context review agent, non-author | Pass with limitation | Verified the remaining-gate list, scope ownership across all H0 bullets, and that this approval is the correct minimal next outcome; fixed the envelope status-label drift. Agent-supplied, not human sign-off |
| Contract/design | Fresh-context review agent, non-author | Pass with limitation | Audited slice coverage, anti-gaming, guardrail, and evidence-class rules of the qualification policy; landed fixes — the deterministic test provider and recorded fixtures can never satisfy Baseline/Qualification provider identity, and explicit guardrails now cover non-authoritative projections served as authority and unusable failure states. Agent-supplied, not human sign-off |
| Adversarial overclaim | Fresh-context review agent | Pass with limitation | Evidence recorded in [#123](https://github.com/schorsch888/novelworld/issues/123) and fixed in [#124](https://github.com/schorsch888/novelworld/pull/124). Agent-supplied, not human sign-off |

Remaining H0 gates: the verified-dispatch `make verify` record with its
successful run URL, required CI on the final commit, and the independent
maintainer, product, security, accessibility, and legal reviews named in
[`PRODUCT_CONTRACT.md`](./PRODUCT_CONTRACT.md). These gates match the ROADMAP
H0 exit evidence list and [`review protocol`](./ROADMAP.md); the agent records
above are evidence with a recorded limitation, not human sign-off.

## Recorded H2 reviews

| Perspective | Reviewer | Disposition | Evidence and unresolved risks |
|---|---|---|---|
| Adversarial threat model | Fresh-context review agent, non-author | NO-CRITICAL-HIGH | Falsified THREAT_MODEL claims against the current code (HS256/aws_lc_rs JWT, auth matrix, identity headers never read downstream, constant-time internal tokens, settings key non-disclosure, provider SSRF allowlist, S3 key construction, nginx posture). Two Low findings fixed: the three `/internal` canon/player/world-entry routes now enforce the internal service token (the narrative client sends it), and THREAT_MODEL now states refresh tokens are stored plaintext (accepted for the self-hosted profile, recorded in SECURITY.md). Agent-supplied, not human sign-off |
| Integration contract | Local run, agent-executed | Pass | All eight integration binaries (45 tests: repository contracts, auth flows, redis cache, account export, backup/restore, legacy migration replay) pass against `docker-compose.test.yml` after the session's changes (aws_lc_rs JWT backend, internal-token wiring, RFC 5321 validation, envelope normalization). The host lacked `psql`, so the migration-contract test used a scratch container shim — CI's integration job passes on ubuntu-latest with `psql` preinstalled ([PR #135](https://github.com/Wisdoverse/novelworld/pull/135)) |
