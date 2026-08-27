# ADR 0003: H4 private-preview journey qualification

- Status: Accepted
- Date: 2026-08-28
- Owners: repository maintainers and H4 component owners
- Related: [`../QUALIFICATION_POLICY.md`](../QUALIFICATION_POLICY.md); [roadmap issue #228](https://github.com/Wisdoverse/novelworld/issues/228)

## Context

The existing policy defines evidence classes and zero-tolerance guardrails,
but leaves H4 journey completion, recovery, and live quality thresholds at
Baseline-only. One DeepSeek run completed the product journey while another
attempt on the same code failed with a provider error; H1 failed five of six
cases. The current evidence also lacks a supported release upgrade, a live
committed-pending projection window, independent human quality approval, and
manual accessibility evidence.

Without a pre-approved cohort and denominator, later runs could discard
failures, change artifacts, or tune criteria against results. This decision is
limited to the loopback, private single-node functional journey. Public-cloud,
multi-replica, sustained SLO, and provider-price qualification are out of
scope.

## Decision

Adopt `h4-journey-qualification-v1` as the H4 release-evidence contract. One
pre-registered cohort manifest pins the clean evaluated commit; base and
candidate release manifests and application image identities; schema and
Compose/runtime identity; provider/model/thinking identity plus the exact
allowed response-model set; non-secret configuration digest; registered input
and browser/assistive-technology matrices; and every applicable
policy/corpus/rubric/prompt/report/schema version. Observed response IDs and
actual browser/assistive-technology/reviewer metadata cannot split a cohort
after results are known. Base and candidate artifacts are built and locked
before registration; build preparation is CI/artifact evidence, not an eligible
journey attempt.

The first three `Started` product journeys on a final cohort must pass 3/3.
Three same-cohort H1 runs must pass all 18 cases, and three H3 runs must pass all
72 cases. Every product run crosses the 10-message recent-history and 20-message
Mid-memory boundaries; a post-Mid Agent restart followed by later live chat must
show content-free evidence that the Mid candidate was selected. At least one
product run crosses the supported ancestor-to-candidate release path. Its exact
base manifest is pre-registered before `Started`, has a strict-ancestor
`RELEASE_GIT_SHA`, and differs in at least one application image digest. After
`Started`, supported adoption establishes the base as current on fresh attempt
volumes, so any adoption/deployment/migration/readiness failure counts. Its
normalized authoritative snapshot and journal prefix are unchanged immediately
after supported candidate upgrade/readiness, then the journey continues
monotonically. #163 remains the sole reducer equivalence proof.

At least one product run also starts an eligible, explicitly witnessed world
transition with Agent already unavailable while Narrative and its transition
provider remain available. The transition commits and its post-commit projection
remains `pending`. From the first successful Agent readiness probe, the
existing scanner has 90 seconds—three normal 30-second scan periods—to reach
`saved` without a second world commit or transition-provider call.
An attempt is registered after clean/config/source/port/existing-stack
preflight and before deployment or provider work, so every later deployment,
provider, assertion, lifecycle, and cleanup failure stays in the denominator.
The first failure terminally fails the cohort and stops later expensive
qualification repetitions; troubleshooting is non-qualifying Diagnostic work.
A new cohort needs a reviewed causally relevant behavior, policy, corpus,
runner, or runtime change. A commit-only, documentation-only, or attempt/evidence
metadata change cannot reset failure.

For each registered H1 case, only JSON parse, typed schema, rubric-version,
exact-token coverage, or explanation-shape contract failure permits at most one
fresh application-level judge retry with the same evaluation payload. This is a
second logical `offline_evaluation` call, not a transport retry; both logical
responses/calls and every underlying transport attempt/usage are retained and
count under `h3-llm-budget-v2`. Markdown extraction, token deletion, field
synthesis, and semantic repair are forbidden. A schema-valid low score is not
retryable, and a second contract-invalid response for that case fails the run.

Runtime authority remains limited to executable checks. Free-text rules and
hostile instructions are untrusted generation input, not authorization. The
server rejects unknown/dead supported targets, unsupported fields, unavailable
listed location/thread state, future progress, invalid shapes/ranges, and
stale/order/idempotency violations. Item targeting is unsupported; generated
inventory is shape-bounded rather than checked against a canon catalog.
Semantic canon, spoiler, and agency correctness is zero-tolerance in frozen H3
plus independent-human evidence, not claimed as general server-side natural-
language enforcement. No keyword blacklist or general rules engine is added.

The retained character-identity compatibility path receives one separate,
bounded sub-slice sharing the core commit/images/provider/input manifest. It
starts from a self-mode committed branch and qualifies only progress-bounded
chat plus exact replay; new/cached-node perspective and new choice/Player/world
authority are unsupported. It does not qualify arbitrary long-lived or
concurrent identity switching. Final approval also requires the existing
built-browser gate, issue #169's named human accessibility/non-author record,
and independent human review of private H3/H4 outputs.

## Alternatives considered

- One green run was rejected because the existing same-code 1/2 product result
  already demonstrates variance.
- A statistical availability program was rejected because three runs cannot
  establish an SLO and H5 owns sustained observation.
- Averaging H1/H3 scores was rejected because per-case thresholds and hard
  guardrails must not be averaged away.
- Retiring character compatibility inside this policy was rejected because it
  would be a separate product/runtime decision; the current path remains
  explicitly bounded and therefore needs its own journey.
- A natural-language rules engine or hostile-keyword filter was rejected as
  unverifiable authority and unnecessary architecture.

## Consequences

Qualification becomes more expensive: one final cohort requires three product,
three H1, and three H3 runs plus human review. The cost buys a fixed denominator
and minimal repeatability evidence, not a public reliability claim. A single
failed eligible attempt fails that cohort and remains visible; a later candidate
must record the reviewed, causally relevant change that creates a new cohort.

The 90-second projection target is now a release gate for this slice. General
latency, public availability, and monetary unit cost remain H5 work. Successful
provider calls must still have 100% usage reporting and all operations remain
inside `h3-llm-budget-v2`.

No service boundary, schema, data ownership, runtime dependency, or public API
changes. Public reports are limited to random attempt IDs, public
provider/model identity, checked-in fixture-manifest and application-image
digests, sanitized failure codes, and aggregates. Private-source hashes,
non-public origins, UUIDs, OS/device identity, reviewer PII, secrets, and
content remain private; raw-content hashes are not treated as anonymization.

## Rollout and rollback

Merge this policy before implementation or judged provider runs. Implement the
H1, release/recovery runner, and accessibility prerequisites as separate
roadmap changes. Register the final cohort before its first eligible attempt,
then append every attempt in order.

Before a judged change depends on this decision, rollback is a normal revert.
After any judged attempt reaches `Started`, changing or reverting the policy
requires a new policy version. The v1 cohort remains in its ledger, cannot be
reclassified or reused, and a failure cannot become qualification. Runtime and
persisted data are unchanged.

## Evidence

- `tools/h1-eval --recorded` and `tools/h3-eval --recorded` remain the
  deterministic structural/calibration gates.
- The existing DeepSeek baseline records the same-code completed and failed
  product attempts, H1 1/6, H3 24/24, and the absent human approval.
- Issues #148, #155, and #163 own the typed negative, real-PostgreSQL race, and
  checkpoint-plus-journal reducer evidence; this decision does not duplicate
  them.
- Issues #169, #229, #230, #231, and #222 own the remaining human,
  implementation, and final-evidence work.
