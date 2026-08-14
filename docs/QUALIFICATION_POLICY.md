# Private-preview-v1 Qualification Policy

Status: **H0 candidate `private-preview-qualification-v1`**. This policy
defines evidence and approval rules; it does not claim that any live product
slice is release-qualified.

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
capacity run cannot substitute for a live qualification report.

## Existing evidence packages

| Package | Current use | Explicit limit |
|---|---|---|
| Required [CI](../.github/workflows/ci.yml) | Structural build, unit, frontend, PostgreSQL/Redis, Windows launcher, and production Compose evidence | Not a live provider, browser, accessibility, recovery, or user-quality report |
| [`single-node-v1`](./SLOS.md) | Deterministic admission, latency, replay, persistence, and Redis bounds on recorded CI hardware | Not a public-traffic or sustained availability SLO |
| [`h3-synthetic-v1`](../tools/h3-eval/README.md) | Positive/adversarial calibration for extraction coverage, chronology, causality, character consistency, spoilers, memory, coherence, and replay | Recorded judgments do not qualify a provider/model or representative novel corpus |
| [`h3-llm-budget-v1`](../tools/llm-budget/policy-v1.json) | Metrics-schema, token-ceiling, retry/error, latency, and missing-usage contract | Checked-in metrics are synthetic; provider price and live unit cost are not qualified |
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
| Provider and model | Exact provider, model, response model, prompt/corpus/policy versions recorded | Timeout, retry-after, malformed JSON/SSE, silent EOF, hostile text, changed response model, missing usage | No provider/model qualified; H2/H3 own live evidence |
| Scale and cost | `single-node-v1` workload and `h3-llm-budget-v1` schema/ceilings | Admission overflow, retry amplification, missing usage, provider failure, cost ceiling breach | Deterministic only; H2/H3/H5 own live spend and SLO observation |
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
| Eligible journey completion | Attempts initiated by an eligible reader on the qualified deployment with a source/configuration inside the declared slice | All six journey stages complete for one principal without manual data repair | Baseline-only |
| Recoverable import | All attempts with a source/configuration inside the declared import slice | Terminal ready, or actionable terminal failure that can safely retry/re-upload without duplicate authority | Baseline-only |
| Extraction quality | Labeled expected canon facts in a legally usable corpus | Non-empty accepted canon with coverage, precision, hallucination, chronology, causality, and provenance scores | Synthetic calibration only; H1 live threshold unapproved |
| Character/world quality | Human-calibrated live conversation and trajectory cases | Character fidelity, memory relevance, multi-turn and causal coherence meet the approved rubric | Synthetic calibration only; H3/H4 live threshold unapproved |
| Latency and recovery | All eligible operations for the exact environment; success latency and timeout/failure counts remain separate | First token, durable completion, replay, restart, restore, and rollback observations | `single-node-v1` structural thresholds only; live SLO unapproved |
| Unit cost | Attempts and successful operations, both retained | Provider calls, retries, tokens, missing usage, and priced billable classes per operation | Schema/ceilings structural; live price and cost threshold unapproved |

Baseline reports preserve failures and rejected work. They may not discard
timeouts, retries, unsupported cases, or provider calls to improve a metric.

## Hard guardrails

Qualification requires zero accepted violations in the release corpus for:

- forged or cross-user identity, access, prompt context, export, or deletion;
- future-source disclosure by a committed retrieval/transition path or in
  user-visible model output;
- mutation of source canon, generated output silently becoming canon, or a
  model-proposed transition bypassing schema/entity/hard-rule validation;
- a reader action controlling a canonical character outside an explicitly
  qualified compatibility slice;
- duplicate authoritative commit or provider call on completed-key replay;
- completion emitted before the authoritative transaction commits;
- a deleted subject becoming available to login, reads, export, provider work,
  or derived projections after the approved restore/erasure procedure;
- secrets, novel text, prompts, conversations, user identities, or linkable
  production resource IDs in reports and product telemetry;
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

## Review and decision

Current-truth, contract/design, adversarial, and final-evidence reviews record
reviewer, final commit, policy/corpus versions, exact slice, unresolved risks,
and disposition. The implementer may perform an adversarial pass but cannot
represent it as independent human approval.

H0 approval of this policy freezes the measurement contract only. H1–H5 still
own the baselines, thresholds, live runs, recovery/security/accessibility
evidence, deployment, and observation required for their exit states.
