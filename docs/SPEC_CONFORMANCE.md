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
| E2 | Import path and validation: [`handlers`](../services/novel-service/src/application/handlers/mod.rs), [`document.rs`](../services/novel-service/src/infrastructure/document.rs), [`character_extractor.rs`](../services/novel-service/src/domain/services/character_extractor.rs), [`novel-service tests`](../services/novel-service/src/tests.rs), and deterministic malformed-input property and regression tests (text/EPUB/PDF bytes, archive-bomb and expanded-text guards) |
| E3 | Durable chat and memory: [`agent handlers`](../services/agent-service/src/application/handlers/mod.rs), [`memory_manager.rs`](../services/agent-service/src/domain/services/memory_manager.rs), [`pg_chat_repo.rs`](../services/agent-service/src/infrastructure/persistence/pg_chat_repo.rs), [`core_reader_loop.sh`](../tests/e2e/core_reader_loop.sh) |
| E4 | Canon and player timeline: [`world_session.rs`](../services/narrative-service/src/domain/entities/world_session.rs), [`narrative_transition.rs`](../services/narrative-service/src/domain/services/narrative_transition.rs), [`narrative tests`](../services/narrative-service/src/tests.rs), [`core_reader_loop.sh`](../tests/e2e/core_reader_loop.sh) |
| E5 | Identity and authorization: [`gateway auth`](../gateway/src/auth.rs), [`user handlers`](../services/user-service/src/application/handlers/mod.rs), [`auth_flow.rs`](../tests/integration/tests/auth_flow.rs) |
| E6 | Export and erasure: [`ACCOUNT_EXPORT.md`](./ACCOUNT_EXPORT.md), [`DATA_RETENTION.md`](./DATA_RETENTION.md), [`account_export.rs`](../tests/integration/tests/account_export.rs), [`auth_flow.rs`](../tests/integration/tests/auth_flow.rs) |
| E7 | Browser contract: [`client.ts`](../frontend/src/shared/api/client.ts), [`client.test.ts`](../frontend/src/shared/api/client.test.ts), [`BranchChoice.test.tsx`](../frontend/src/widgets/branch-choice/ui/BranchChoice.test.tsx), [`globals.css`](../frontend/src/app/styles/globals.css) |
| E8 | Runtime and security configuration: [`ci.yml`](../.github/workflows/ci.yml), [`docker-compose.yml`](../docker-compose.yml), [`nginx.conf`](../infra/nginx/nginx.conf), [`THREAT_MODEL.md`](./THREAT_MODEL.md), [`bad_release_drill.sh`](../tests/e2e/bad_release_drill.sh), [`release.sh`](../infra/docker/release.sh), [`release_state_drill.sh`](../tests/e2e/release_state_drill.sh) |
| E9 | Backup, restore, lineage, and erasure replay: [`BACKUP_RESTORE.md`](./BACKUP_RESTORE.md), [`0016_erasure_records.sql`](../infra/postgres/migrations/0016_erasure_records.sql), [`backup.sh`](../infra/backup/backup.sh), [`restore.sh`](../infra/backup/restore.sh), [`backup_restore.rs`](../tests/integration/tests/backup_restore.rs), [`backup_restore_drill.sh`](../tests/e2e/backup_restore_drill.sh) |
| E10 | Gateway error normalization: [`proxy.rs`](../gateway/src/proxy.rs) (`NORMALIZED_ERROR_RESPONSES` and the pinned-contract tests) |
| E11 | Supply-chain gates: [`audit.toml`](../.cargo/audit.toml), [`Cargo.lock`](../Cargo.lock), [`.gitleaks.toml`](../.gitleaks.toml), [`deny.toml`](../deny.toml), [`scan-images.sh`](../infra/security/scan-images.sh), [`generate-sboms.sh`](../infra/security/generate-sboms.sh), the `rustsec/audit-check`, `gitleaks`, `cargo-deny`, and trivy CI steps, and the Dependency Policy in [`SECURITY.md`](../SECURITY.md) |

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
| §12.3 — versioned migrations, replay safety, and initial application | Verified | H1, H5 | E1 |
| §12.4.1 — erasure records atomic with every deletion path, identifying payload limited to subject type and UUIDs with content-free bookkeeping fields, per-novel records under account cascade, cascade-surviving | Verified | H1 | E9; `AFTER DELETE` row triggers on `users` and `novels` write inside the deleting transaction, the journal has no foreign keys so a cascade cannot remove its own evidence, and drill B exercises the direct and cascade paths. The column set is asserted directly: subject type and UUIDs identify, while `erased_at`, `had_source` and `source_requeued_at` are named bookkeeping carrying nothing derived from content, profile or credentials |
| §12.4.2 — migration-path replay before services start, idempotent, removes matching subject rows, exactly-once source-key re-queue per database lineage with durable per-record bookkeeping (at most one repeat per restore) | Verified | H1 | E9; replay runs in the migration container the services wait on, and both the integration tests and drill B replay it twice with no further effect. The re-queue is gated on the record's own `had_source`, recorded by the delete that could still see the key, so a key is reconstructed for a subject row this database never held — proven with no other retained-source evidence present — and never enqueued speculatively for a novel that held none. Stamps restored from the artifact's own dump are retained, so those records repeat zero re-queues; a record from a foreign sidecar source enters unstamped and re-queues once. Deletions predating the migration are declared unknowable and are not backfilled |
| §12.4.3 — artifacts embed same-snapshot erasure exports with covered-through timestamps; restore stops writes, replays the union of sources, aborts on conflicting deletion facts with the retained-source marker merging monotonically, establishes live continuation only by equality of the create-once version-4 lineage token (manifest and dump tokens must agree with asymmetric absence aborting; wholly token-less artifacts restore through the disaster gate; restores regenerate the token atomically with reachability, recording the artifact token — or its recorded absence — as parent), refuses non-empty residual windows except through attest-or-erase (every account not covered by a collected record decided retain-with-listed-novels or erase; collected-record accounts get automatic `replayed` attestation rows; erasure records written and replayed for erased accounts and unlisted novels before services start; all rows durably recorded with subject, decision, both window bounds, the verified artifact digest inventory, operator identity, and timestamp), never contains an undecided subject or serves one covered by any erasure record, rotates the JWT secret and deletes all persisted refresh tokens after verification and before services start, clears runtime configuration on final-account removal, and requires designating a retained administrator when decisions would leave none | Verified | H1 | E9; the sidecar export and the manifest's lineage token are both cut out of the single `pg_dump` stream, so manifest and dump agree by construction and the export cannot diverge from the dump; the covered-through timestamp is read immediately before that snapshot opens — never the archive-write time, and conservatively early, so a derived window is a superset. Drill C proves the token lifecycle end to end: migration replay preserves the token, two restores of one artifact produce distinct tokens each recording the artifact's as parent, a manifest disagreeing with its dump and an asymmetric absence are refused, a wholly token-less artifact restores only through the gate with an absent parent, and a failure injected before the atomic load/regenerate commit leaves no reachable data while one injected after it leaves the regenerated token — both retries face the gate. Continuation is token equality alone, so an unrelated or sibling database is gated. Collected-record accounts are excluded from the decisions the operator supplies and from the prompt's novel inventory, are rejected if named, and receive automatic `replayed` attestation rows with the full field set. Manifest and decision UUIDs, digests and timestamps are shape-checked whole-value, with timestamps additionally calendar-checked by the server that stores them, and the free-form operator identity is quote-doubled, an inverted window aborts, and the recorded inventory names only digests this run verified, labelled by what they cover |
| §12.4.4 — scripted, encrypted, integrity-verified artifacts and fail-closed restore on corrupt or unverifiable input | Verified | H1 | E9; AES-256-CBC with PBKDF2 at 200 000 iterations, a SHA-256 manifest verified before any data change, artifacts written under temporary names and renamed only once all three outputs exist, and drill negatives for a corrupted artifact, a wrong key, and tampered manifest metadata. The ≤ 30 minute RTO scale rehearsal is not part of this evidence: [`scale_rehearsal.sh`](../infra/backup/scale_rehearsal.sh) is tooling only, never runs in CI, and its recorded run remains separate release evidence |
| §12.4.5 — erasure records excluded from account export and free of source text, messages, profile data, and credentials | Verified | H1 | E9; the journal's column set is asserted directly, and both the production export port and the end-to-end export exclude erased subjects and the journal itself |
| §13.1 — FSD import direction | Intended gap | H4 | E7; the slice structure exists but no import-direction lint or test enforces it — frontend CI builds but does not verify boundaries |
| §13.2–§13.3 — standalone import route/wizard and named progress component | Obsolete/corrected | H0 | Removed implementation prescriptions; the shelf and reader own those user outcomes directly |
| §13.3 — chat preserves reading context and branch choice blocks advancement | Verified | H4 | E7 |
| §13.4 — declared visual tokens and reading typography | Verified | H4 | E7; this is not WCAG qualification |
| §13.5 — POST SSE framing, exact key reuse, commit acknowledgement, and bounded retries | Verified | H3, H4 | E3, E7 |
| §13.5 — server-owned identity/progress and retired `user_id` rejection | Verified | H2, H4 | E3, E7 |
| §14.1 — every log is structured and carries the required trace fields | Verified | H2, H5 | E8 plus the e2e log-contract checker: every service enters a post-init service span carrying service + trace_id (empty outside requests), the gateway middleware accepts/generates X-Trace-Id, echoes and forwards it, and each downstream trace middleware wraps requests in a span carrying it; the checker parses every service-owned stdout line for timestamp/level/message/service/trace_id, requires request-scoped trace ids per service, and proves end-to-end propagation by stamping a known X-Trace-Id and asserting the downstream service logs it |
| §14.2 — liveness/readiness separation and Gateway aggregation | Verified | H2, H5 | E8 and Production Compose Smoke |
| §14.3 — private metrics route, bounded labels, and no private content | Verified | H2, H5 | E8; representative deployment collection remains unqualified |
| §14.3 — suggested `character_id` metric label | Obsolete/corrected | H0 | Removed the high-cardinality, linkable label suggestion from the normative contract |
| §15 — internal-only services, Nginx-only host ingress, and Gateway-only application ingress | Verified | H2 | E8 |
| §15 — secret length, bcrypt, upload validation, and managed object keys | Verified | H2 | E2, E5, E8 |
| §15 — untrusted prompt boundaries and model output cannot authorize commits | Verified | H2, H3, H4 | E2, E3, E4 provide structural evidence; live adversarial qualification remains open |
| §15 — all SQL remains parameterized | Verified | H2 | Current-source review found bound persistence queries; H2 still owns automated/static and dependency gates |
| §15 — known-vulnerability dependency gate | Verified | H2 | E11; a local cargo-audit 0.22.2 run against the current `Cargo.lock` is clean under `.cargo/audit.toml`, which records the four acknowledged advisories (rustls-webpki 0.101.7 and h2 0.3.27) with rationale (transitive through the already-latest AWS SDK TLS chain — no patched 0.101 release; name-constraint findings need a misissued certificate, CRL-parsing panic documented, and h2's unbounded empty DATA frames require a hostile HTTP/2 server); jsonwebtoken was switched to its `aws_lc_rs` backend so the rsa crate is not in the tree at all; CI adds the live `rustsec/audit-check@v2.0.0` step so any newly reported advisory fails the build (required CI green, [PR #135](https://github.com/Wisdoverse/novelworld/pull/135)); informational warnings (ttf-parser unmaintained, lru unsound pop) remain non-failing and re-reviewed on chain updates; deploy-time SBOM verification, provenance/attestation, and signature gates remain open H2 items |
| §15 — committed-secret scanning gate | Verified | H2 | E11; a local gitleaks 8.24.3 scan over the full commit history is clean under `.gitleaks.toml` — the committed default rule set plus an allowlist of two deliberate test fixtures (the CI `RUNTIME_CONFIG_KEY` smoke placeholder and two static provider model names, one history-only) — and the gate's detection strength is pinned by a self-test that plants a GitHub-shaped token, asserts the scan fails, and asserts the repository stays clean; CI runs the pinned `gitleaks:v8.24.3` image directly over the checkout with the committed `.gitleaks.toml` (the action was dropped: it requires a paid org license) plus the runtime-token self-test (required CI green, [PR #135](https://github.com/Wisdoverse/novelworld/pull/135)) |
| §15 — dependency license and source gate | Verified | H2 | E11; a local cargo-deny 0.20.2 `check licenses sources` passes with `deny.toml` — every dependency license satisfies the explicit permissive allow set, unlicensed is denied by default (the nine workspace crates now declare `license = "MIT"` matching the repo LICENSE), and unknown registry/git sources are denied; no exceptions are needed (dual MIT/Apache-2.0 crates and r-efi's MIT OR Apache-2.0 OR LGPL-2.1-or-later are satisfied without choosing the copyleft branch); CI adds the pinned `EmbarkStudios/cargo-deny-action@v2.1.1` step (required CI green, [PR #135](https://github.com/Wisdoverse/novelworld/pull/135)); advisories remain owned by cargo-audit |
| §15 — container image scanning gate | Verified | H2 | E11; local trivy 0.68.1 scans (HIGH/CRITICAL, vuln scanner) of all six application images report zero findings, the four Dockerfile base images are now digest-pinned, and the tag pipeline (docker.yml) scans every pushed image with `--exit-code 1` so a finding fails the release (release-pipeline run pending: no v-tag cut yet); the digest-pinned infrastructure images are scanned when re-pinned through the separately approved procedure — the current pinned `pgvector/pgvector@sha256:69167330…` reports 22 findings (21 HIGH, 1 CRITICAL, CVE-2025-68121) inside its bundled gosu binary, fixed in go 1.24.13 but not yet rebuilt into the pinned image, tracked for the next infrastructure re-pin (gosu runs only as the postgres entrypoint's privilege-drop helper and does not exercise the affected Go TLS path) |
| §15 — SBOM generation | Verified | H2 | E11; the release pipeline generates one CycloneDX 1.6 SBOM per application image with the pinned trivy release and ships them with the release artifact, bound to the recorded image digest via `sboms/digests.txt`; `infra/security/generate-sboms.sh` is the local form — all six local SBOMs generated and validated (CycloneDX 1.6, 72–107 components, digest sidecar) — release-pipeline run pending (no v-tag cut yet); deploy-time SBOM verification, provenance/attestation, and signatures remain open |
| §16 and former Appendix A — duplicate implementation, test, and prompt prescriptions | Obsolete/corrected | H0 | Removed stale copies; `AGENTS.md`, runtime validators/prompts, and Roadmap issues own those changing details |

## Implementation-defined selections

- Character extraction uses a representative first/middle/last sample and a
  bounded overlapping full-text chunk scan; successful chunk results are merged
  case-insensitively.
- Prompt context currently uses the ten most recent committed messages, creates
  a mid-term summary every twenty committed messages, retrieves up to five
  semantic results when embeddings are available, and has no connected
  production promotion path for long/permanent memories.
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
