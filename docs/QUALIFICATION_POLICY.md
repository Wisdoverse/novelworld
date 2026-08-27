# Private-preview-v1 Qualification Policy

Version: **`private-preview-qualification-v1`**. This reviewed change approves
the policy; the baselines, thresholds, live runs, and H1–H5 gates it defines
remain open. It does not claim that any live product slice is
release-qualified.

## Decision contract

NovelWorld qualifies a user outcome, not a component or endpoint. A journey
starts after an operator completes first-admin and model setup, and is eligible
only when its source, deployment, platform, language, and provider configuration
are inside the declared [`PRODUCT_CONTRACT.md`](./PRODUCT_CONTRACT.md) envelope.
Success requires one authenticated reader to:

1. import a source and receive a non-empty, source-proven world;
2. read to an unlocked checkpoint;
3. commit and replay a character conversation;
4. enter as an original player and commit a branch or open-world action;
5. recover the same committed state after disconnect and service restart; and
6. export the owned state and complete account or novel deletion inside the
   documented boundary.

An in-envelope import that ends in an actionable, bounded failure is measured
as recoverability, not journey completion. A rejected unsupported or malformed
source is successful negative handling only when the declared error, cleanup,
and provider-call bounds hold.

## Evidence classes

| Class | Required identity | Permitted claim |
|---|---|---|
| Structural | Commit, fixture/policy version, deterministic report where applicable, and required CI | The checked implementation enforces the exercised contract |
| Baseline | Structural identity plus target environment, configured provider/model, returned model identifiers, corpus/version hashes, raw observations and aggregates, and reviewer | Observed values only; no pass threshold or support claim |
| Qualification | Pre-approved threshold policy plus a matching final-commit baseline/report and required reviews | Only the exact tested slice is release-qualified |
| Observation | Immutable deployed artifact/configuration plus the approved observation window | The deployed slice met its SLO/quality/cost policy for that window |

Branch output, a synthetic fixture, a provider connection test, or a green
capacity run cannot substitute for a live qualification report. The
deterministic test provider and recorded fixtures never satisfy the
`configured provider/model` identity that Baseline and Qualification evidence
require.

### DeepSeek v4 Flash baseline entrypoint

The baseline runner requires a clean commit and an operator-owned JSON file
outside the checkout with exactly `provider`, `api_url`, `model`,
`thinking_enabled`, and `api_key`. The accepted slice is `deepseek`,
`https://api.deepseek.com`, `deepseek-v4-flash`, with product thinking enabled.
Keep the file readable only by the operator and invoke:

```bash
tests/e2e/live_deepseek_journey.sh \
  --config /private/path/deepseek.json \
  --output-dir /private/path/novelworld-baseline \
  --git-sha "$(git rev-parse HEAD)"
```

The runner creates a unique Compose project, container prefix, volumes,
loopback port, and local image tags; it never joins or restarts the default
`novel-*` deployment and verifies that deployment's container identities and
restart counters are unchanged. It uses PostgreSQL authority without Redis or
S3, completes the ordinary-reader journey, and removes only its isolated
Compose volumes on exit. Its raw Prometheus files contain a stable usage-key
fingerprint and remain private; only the sanitized aggregate report is eligible
for review or commit.

Run H1 and H3 live evaluation separately at the same clean commit with their
documented `--metrics-output` option. The resulting reports identify returned
models and force `thinking_enabled: false` for schema-bound JSON calls. A later
evidence commit records the immutable reports, so `evaluated_git_sha` identifies
the code that ran while `evidence_commit` identifies the commit that packages
that evidence; they cannot be the same self-referential value. This entrypoint
creates baseline evidence only and does not supply the separate threshold or
human-quality approval required for Qualification.

## H4 journey qualification — `h4-journey-qualification-v1`

This policy is approved before the implementation and live runs that it
judges. It qualifies only the functional H4 journey described here; it does
not qualify public hosting, multi-replica behavior, sustained availability, or
provider pricing.

### Supported slice and cohort identity

The candidate slice is the loopback-bound production Compose profile on one
Linux Docker Engine, PostgreSQL authority, `CACHE_MODE=postgres`, no S3, a core
journey that starts with an original self-mode `PlayerEntity`, and the exact
`deepseek / deepseek-v4-flash` configuration described above. Schema-bound
evaluation has thinking disabled; product generation has thinking enabled.
Character visibility is limited to explicit actor identifiers or a directly
targeted action inside the existing bounded scan; inferred location,
line-of-sight, or relationship visibility is outside this slice.

One cohort has exactly one value for every item below:

- evaluated Git commit and clean-tree proof;
- Compose-file digest, schema barrier, Docker Engine OS/architecture/version,
  Compose version, cache/object-storage profile, and loopback binding;
- immutable application image ID and repository digest for all six application
  images, including the registered base and candidate image sets used by the
  release-upgrade drill;
- provider, API origin, configured model, the exact allowed response-model
  identifier set, product/evaluation thinking modes, and a digest of the
  allowlisted non-secret runtime configuration (never an API-key or
  credential-derived fingerprint);
- qualification, extraction, H3 semantic, LLM budget/metrics, corpus, rubric,
  prompt, runner-report, and transition-schema versions; and
- one versioned product/H1/H3 input manifest, the release base manifest, and
  one required browser, assistive-technology, viewport/device, and
  manual-review role matrix.

Base and candidate artifacts are built before cohort registration, then their
manifests and immutable image identities are locked. Artifact preparation and
its failures are CI/build evidence, not eligible product-journey attempts.

The manifest is pre-registered before any attempt. Observed response-model IDs,
actual browser/assistive-technology versions, and a consenting reviewer's
repository handle/role are evidence metadata, not post-result cohort keys. A
response-model ID outside the pre-registered set fails that attempt; it cannot
create a more favorable cohort.

Only a pre-run manifest field listed above may define a new cohort, and the new
registration records a reviewed, causally relevant behavior, policy, corpus,
runner, or runtime change. Commit-only, documentation-only, evidence metadata,
failure, latency, usage, observed identity, and reviewer-outcome changes do not
reset a failed cohort. Failed cohorts remain in the versioned ledger; changing
a commit or manifest does not turn an earlier failure into a passing
denominator.

After side-effect-free clean-checkout, configuration-shape, registered-input,
free-port, and existing-stack preflight, the runner appends an immutable
`Started` record with the cohort ID and monotonic attempt sequence. It does so
before image pull, adoption, deployment, migration, readiness, or provider
work. Invalid operator input rejected before `Started` is not eligible; every
provider, runner, pull, adoption, deployment, migration, readiness, timeout,
assertion, export/deletion, cleanup, or abandoned result after `Started` is a
failed attempt unless every gate records `Passed`.

The first failed eligible attempt makes its cohort terminal `Failed`; remaining
expensive qualification repetitions stop. Later troubleshooting is recorded as
non-qualifying `Diagnostic` work and can neither complete nor repair that
cohort. A new cohort requires the reviewed causal change above.

Public reports are allowlisted to random qualification attempt IDs, public
provider/model identifiers, checked-in fixture-manifest digests, application
image content digests, sanitized failure codes, and aggregate counts,
latencies, tokens, and costs. Private-source hashes, non-public API or registry
origins, account/novel/resource IDs, OS usernames/device IDs, and reviewer PII
remain private. A raw-content hash is linkable evidence, not anonymization; a
reviewer handle is public only with explicit consent.

### Frozen decision thresholds

| Gate | Denominator | Passing threshold |
|---|---|---|
| Same-cohort repeatability | The first three `Started` core product journeys on one final cohort | **3/3** start in self mode and complete without manual data repair. Every start is retained, including provider, runner, deployment, timeout, abandoned, and cleanup failures. Each product attempt uses fresh isolated volumes; only the upgrade inside that attempt preserves its volumes across base and candidate images. |
| H1 extraction | Three fresh live `extraction-quality-v1` runs on the same final cohort; each run contains all six registered cases | **18/18** cases pass the already-approved 80% category recall, 80% precision, at most 20% hallucination, zero chronology violations, and 100% provenance thresholds. For each registered case, only JSON parse, typed schema, rubric-version, exact-token coverage, or explanation-shape contract failure permits at most one fresh application-level judge retry with the same evaluation payload. This is a second logical `offline_evaluation` call, not a transport retry; both logical responses/calls and every underlying transport attempt/usage are retained and count under `h3-llm-budget-v2`. No Markdown extraction, token deletion, field synthesis, or semantic repair is allowed. A schema-valid low score is not retryable, and a second contract-invalid response for that case fails the run. Reports or responses are never reused between runs. |
| H3 semantic calibration | Three fresh live `h3-semantic-v1` runs on the same final cohort; each run contains all 24 registered cases | **72/72** cases pass their frozen per-dimension thresholds. Provider responses and reports are never reused between runs. Same-provider judgment remains supporting evidence only. |
| Long causal trajectory | Every eligible core product journey | Import, bounded/full persona, branch, exact branch-to-chat revision, at least 12 ordered world turns, and chat cross both production boundaries (`SHORT_TERM_LIMIT=10`, `MID_TERM_TRIGGER=20`). After Mid memory commits, restart Agent, send a later live chat, and prove by content-free selection counters/markers that the Mid candidate was selected while continuity and spoiler guardrails still pass. Exact replay, export, and deletion pass with zero canon, agency, spoiler, or state-continuity violation. UI and export retain explicit canon, reader-authority, uncertain-extraction, and generated-prose distinctions. |
| Supported release upgrade | At least one of the three core product journeys, without changing cohort candidate identity | Before registration, build and lock the exact base manifest whose `RELEASE_GIT_SHA` is a strict Git ancestor of the candidate and whose application image content/repository digests differ from the candidate in at least one service. After `Started`, supported `adopt` establishes that base as current on the attempt's fresh volumes; pull/adoption/deployment/migration/readiness failure fails the attempt. Run the first half, then record the normalized authoritative snapshot and committed-journal prefix before supported `upgrade`. After the candidate is ready on the same attempt volumes/principal, both are unchanged; the second half advances monotonically with no duplicate authority or manual database repair. Artifact-build and in-attempt pull/release/readiness timings are reported separately. #163 remains the sole checkpoint-plus-journal reducer-equivalence proof. |
| Pending projection recovery | At least one of the three core product journeys | With Agent already unavailable while Narrative and the transition provider remain available, an eligible explicitly witnessed world transition commits and its post-commit projection remains `pending`. A different key is refused before provider work. From the first successful Agent readiness probe, the existing scanner reaches **`saved` within 90 seconds**. Original-key replay returns the committed result with one world commit and no second world-transition provider call. |
| Negative/idempotency matrix | Every approved case on the final commit, plus the existing real-PostgreSQL race suite | Unknown/dead supported targets, unsupported fields, unavailable listed entity/state, future progress, invalid shapes/ranges, stale/out-of-order requests, and model transitions that violate the schema or explicitly listed identity/entity/state/progress/order checks produce zero unauthorized journal/world/chat delta. Item targeting is unsupported and rejected as an unknown field. A completed key makes zero provider calls; ambiguous in-flight work stays inside its existing lease and `h3-llm-budget-v2` transport-retry ceiling. |
| Structural final-commit gates | Final candidate commit | The existing #148/#155 negative and real-PostgreSQL race gates and #163 checkpoint-plus-committed-journal rebuild gate pass without weakening or duplicating their production validators/reducer. |
| Legacy character identity | One separate, explicitly bounded compatibility sub-slice sharing the final commit, image, provider, and registered input manifest while the path remains supported | Start from a branch committed in self mode; character mode permits progress-bounded in-character chat and exact read/replay of that result. New/cached-node perspective, new node/choice, Player, and open-world authority are unsupported and fail before provider/write work. Switching to self preserves prior self state; restart, export, and deletion complete. This one smoke does not qualify arbitrary long-lived or concurrent cross-service switching, and its evidence metadata does not reopen the core denominator. |
| Accessibility and human quality | Automated evidence on the final commit plus the complete human records | The built-app browser gate passes; issue #169 records real keyboard, named screen reader, mobile/reflow, reduced-motion, and non-author completion on merged `main`. A named non-implementer reviews the private H3/H4 outputs for character fidelity, causal continuity, spoiler/canon/agency boundaries, and failure clarity. Every violation of this policy is closed; non-violating observations are triaged without widening the qualified slice. |
| Usage and bounded work | Every provider request in every `Started` candidate-cohort attempt | Usage is present for **100%** of successful calls; each attempt and the cohort aggregate retain every retry/failure and separately satisfy `h3-llm-budget-v2`. Historical cohorts remain non-qualifying evidence and are not silently mixed into or removed from the candidate denominator. Monetary provider price and sustained cost/SLO qualification remain H5 work. |

All hard guardrails in this document remain zero-tolerance. A three-run sample
does not establish a public availability SLO; it is the minimum repeatability
evidence for this private-preview functional slice. The policy is reviewed by
**2026-11-30**, and earlier whenever an identity above changes.

### Typed rules and hostile input

Server-authoritative checks in this slice are the ones the runtime can execute:
identity and ownership, membership of supported entity targets, death,
location/thread availability, source-progress bounds, state revision, turn
order, idempotency, strict field shape, and numeric/list ranges. Item targeting
is unsupported and rejected as an unknown field; generated inventory is
shape-bounded state, not membership in a canonical item catalog. Free-text
`hard_rules` prose constrains generation quality but is not an authorization
engine.

Hostile text is accepted only as quoted, untrusted data and cannot bypass the
listed runtime checks. Semantic canon, spoiler, and agency correctness is a
zero-tolerance result in the frozen H3 and independent-human corpus; it is not
misrepresented as general server-side natural-language enforcement. A keyword
blacklist or general natural-language rules engine is neither required nor
permitted as evidence for this contract.

## Existing evidence packages

| Package | Current use | Explicit limit |
|---|---|---|
| Required [CI](../.github/workflows/ci.yml) | PRs run structural build, unit, frontend, browser accessibility, PostgreSQL/Redis, Windows launcher, and a production Compose smoke; `main`, manual verification, and release calls additionally run the deterministic recovery, outage, backup/restore, secret-rotation, and capacity drills | Not a live provider, target-environment, manual accessibility, or user-quality report |
| [`single-node-v1`](./SLOS.md) | Deterministic admission, latency, replay, persistence, and Redis bounds on recorded CI hardware | Not a public-traffic or sustained availability SLO |
| [`h3-synthetic-v1`](../tools/h3-eval/README.md) | Positive/adversarial calibration for extraction coverage, chronology, causality, character consistency, spoilers, memory, coherence, and replay | Recorded judgments do not qualify a provider/model or representative novel corpus |
| [DeepSeek v4 Flash live baseline](./evidence/deepseek-v4-flash-live-baseline.json) | At exact code SHA `ddefaa8c023019fb2cbf6b279444215795ef5f48`, an isolated production-Compose reader journey completed, including branch-to-chat revision binding, 12 world turns, restart recovery, export, and deletion; H3 passed 24/24 final-SHA samples | H1 failed 5/6 slices, human quality approval is absent, and no qualification claim is made |
| [`h3-llm-budget-v2`](../tools/llm-budget/policy-v2.json) | Metrics-schema, closed operation set, token-ceiling, retry/error, latency, missing-usage, and full release-window sampling contract | Checked-in metrics are synthetic; provider price and live unit cost are not qualified |
| [`THREAT_MODEL.md`](./THREAT_MODEL.md) | Current assets, trust boundaries, attacker stories, severity, and accepted external boundaries | A repository review is not host configuration or incident-response evidence |
| [`DATA_RETENTION.md`](./DATA_RETENTION.md) and [`ACCOUNT_EXPORT.md`](./ACCOUNT_EXPORT.md) | Application retention, erasure, provider/operator boundary, and export completeness | Not backup restore, provider deletion, or target-environment verification |

## Independent-dimension slices

Every dimension has at least one normal and adversarial case before its slice
can qualify. Cases may share a journey when the combination is risk-selected;
passing one dimension does not qualify its untested combinations.

| Dimension | Normal slice | Adversarial slice | Current state / owner |
|---|---|---|---|
| Deployment | Localhost, one production Compose instance, fresh volumes, operator-controlled secrets | Dependency loss, restart, occupied capacity, non-local bind/TLS and permissive-CORS review | Structural only; H2 qualifies the private boundary and any future public profile |
| Platform | Linux full journey; Windows launcher initialization | Missing/invalid prerequisites, interrupted startup, path and encoding edge cases | Launcher structural evidence only; H2/H4 own platform/browser qualification |
| Input format and encoding | Paste UTF-8; TXT UTF-8, BOM UTF-16, GBK; text EPUB; text PDF at declared sizes | Oversize, invalid encoding, malformed/traversal or decompression-heavy EPUB, scanned/encrypted/malformed PDF | Parser acceptance only; H1 owns recoverability and extraction quality |
| Language | Simplified Chinese and English chapter/lore fixtures; generated-world journey explicitly Chinese | Mixed-script headers, ambiguous boundaries, hostile instructions, future-spoiler text | Structural only; H1/H3/H4 own representative live quality |
| Provider and model | Exact provider, model, response model, prompt/corpus/policy versions recorded | Timeout, retry-after, malformed JSON/SSE, silent EOF, hostile text, changed response model, missing usage | DeepSeek v4 Flash has a baseline, but H1 failed and no provider/model is qualified; H1/H2/H3/H4 own the remaining functional evidence |
| Scale and cost | `single-node-v1` workload and `h3-llm-budget-v2` schema/ceilings | Admission overflow, retry amplification, missing usage, provider failure, cost ceiling breach | Deterministic only; H2/H3/H4 own bounded functional usage and H5 owns live spend/SLO observation |
| Accessibility and browser | Critical journey with keyboard, focus, names/roles/status, zoom, motion, and supported viewport/browser record | Failed request/retry, streaming announcement, modal/focus escape, narrow viewport, reduced motion | Baseline missing; H4 owns WCAG 2.2 AA qualification |

## Risk-selected intersections

The initial suite uses these intersections rather than a format × encoding ×
language × provider × browser Cartesian product:

- Simplified Chinese GBK TXT exercises legacy decoding plus Chinese splitting;
- English BOM UTF-16 TXT exercises multibyte decoding plus English splitting;
- Chinese EPUB and English text PDF exercise both structured document parsers;
- UTF-8 paste at the accepted boundary exercises the JSON/body-limit path;
- the largest accepted source uses the deterministic provider before any live
  run to bound fan-out and rejected-work cost;
- hostile source instructions cross import, lore, chat, and world-transition
  validation without crossing authority;
- disconnect/restart crosses chat and world commit/replay before export/delete;
- one keyboard/screen-reader qualification crosses operator setup and then the
  reader's import failure, chat stream, required branch choice, and deletion
  confirmation.

A reviewer adds a combination only when shared parsing, model, UI, privacy, or
cost risk makes the existing intersections insufficient.

## Journey measurements

| Measure | Denominator | Success record | Threshold state |
|---|---|---|---|
| Eligible journey completion | Attempts initiated by an eligible reader on the qualified deployment with a source/configuration inside the declared slice | All six journey stages complete for one principal without manual data repair | `h4-journey-qualification-v1`: the first three same-cohort attempts must pass 3/3 |
| Recoverable import | All attempts with a source/configuration inside the declared import slice | Terminal ready, or actionable terminal failure that can safely retry/re-upload without duplicate authority | Baseline-only |
| Extraction quality | Labeled expected canon facts in a legally usable corpus | Non-empty accepted canon with coverage, precision, hallucination, chronology, causality, and provenance scores | The approved [`extraction-quality-v1`](./EXTRACTION_QUALITY.md) gate is live: DeepSeek v4 Flash failed 5/6 final-SHA slices, so no provider/model has passed |
| Character/world quality | Human-calibrated live conversation and trajectory cases | Character fidelity, memory relevance, multi-turn and causal coherence meet the approved rubric | `h4-journey-qualification-v1` requires 72/72 same-cohort H3 cases plus named non-implementer review; current evidence lacks that human approval |
| Latency and recovery | All eligible operations for the exact environment; success latency and timeout/failure counts remain separate | First token, durable completion, replay, restart, restore, and rollback observations | H4 approves the 90-second pending-projection bound and requires supported restart/upgrade completion; sustained live SLO remains H5 |
| Unit cost | Attempts and successful operations, both retained | Provider calls, retries, tokens, missing usage, and priced billable classes per operation | H4 requires 100% successful-call usage and `h3-llm-budget-v2` ceilings; monetary price and sustained unit-cost qualification remain H5 |

Baseline reports preserve failures and rejected work. They may not discard
timeouts, retries, unsupported cases, or provider calls to improve a metric.

## Hard guardrails

Qualification requires zero accepted violations in the release corpus for:

- forged or cross-user identity, access, prompt context, export, or deletion;
- future-source disclosure by a committed retrieval/transition path or in
  user-visible model output;
- mutation of source canon, generated output silently becoming canon, or a
  model-proposed transition bypassing schema or an explicitly listed typed
  identity/entity/state/progress/order validation;
- a reader action controlling a canonical character outside an explicitly
  qualified compatibility slice;
- duplicate authoritative commit or provider call on completed-key replay;
- completion emitted before the authoritative transaction commits;
- a non-authoritative projection (cache, search index, derived media, or
  generated prose) served as authoritative state;
- a deleted subject becoming available to login, reads, export, provider work,
  or derived projections after the approved restore/erasure procedure;
- secrets, novel text, prompts, conversations, user identities, or linkable
  production resource IDs in reports and product telemetry;
- an inaccessible, misleading, or unrecoverable failure state on the critical
  journey;
- unbounded provider fan-out, retry amplification, or work after rejection.

Any unresolved Critical or High security finding blocks qualification. Lower
accepted risks require an owner, rationale, review date, and exact affected
slice. A hard guardrail cannot be traded against average quality, latency, or
cost.

## Threshold approval and change rules

1. A baseline-only change freezes corpus, dimensions, report schema, sampling,
   and environment identity, then records raw observations and aggregates; it
   sets no pass target.
2. A separate reviewed policy change sets thresholds before the implementation
   or provider/model change it judges. It records rationale, minimum sample
   size, hard guardrails, rollback, and expiry/review date.
3. The judged change cannot lower its policy, remove failed samples, change the
   supported slice, or substitute a faster host/provider to pass.
4. Policy, corpus, prompt, model, provider, parser, or material environment
   changes invalidate only the affected evidence. Versioned inputs receive a
   new version; environment changes require a new identity and report.
5. Recorded reports that claim deterministic output run twice and must be
   byte-identical. Live and measured-capacity reports retain response-model or
   environment identity and bounded aggregate evidence; they are not expected
   to be byte-identical.
6. Reports contain no private content. Locally aggregated product telemetry
   requires explicit consent before collection outside the deployment.
7. Once any judged attempt reaches `Started`, changing this decision contract
   requires a new policy version. The old cohort remains in its versioned
   ledger and cannot be reclassified or reused by the new version; reverting
   policy text cannot turn a failure into qualification.

## Review and decision

Current-truth, contract/design, adversarial, and final-evidence reviews record
reviewer, final commit, policy/corpus versions, exact slice, unresolved risks,
and disposition. The implementer may perform an adversarial pass but cannot
represent it as independent human approval.

H0 approval of this policy freezes the measurement contract only. H1–H5 still
own the baselines, thresholds, live runs, recovery/security/accessibility
evidence, deployment, and observation required for their exit states.
