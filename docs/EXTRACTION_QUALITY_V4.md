# H1 v4: gold alignment and source-support measurement

This prospective contract governs the existing `tools/h1-eval` evaluator.
It is measurement-only, not source-quality Qualification. It neither changes
the frozen #229 outcome nor adopts a new formal H4 cohort. Any paid run still
requires a separate immutable registration and enforceable budget under
[Qualification policy](QUALIFICATION_POLICY.md).

## Identity and compatibility

| Surface | Identity |
|---|---|
| Policy | `extraction-quality-v4` |
| Corpus | `h1-synthetic-v6`, `tools/h1-eval/corpus/v6.json` |
| Rubric | `h1-extraction-v4` |
| Judge prompt | `h1-semantic-judge-v9` |
| Public report / private response envelope | 4 / 2 |

The new corpus copies `corpus/v1.json`, changing only policy/corpus/rubric
identities. Sources, expected facts, recorded artifacts, thresholds and case
composition are unchanged. Previous policies, corpus bytes and reports remain
frozen. There is one evaluator, not a compatibility scoring path; historical
results retain their original source revision. Consumers must explicitly adopt
schema 4 and must not interpret successful execution as quality approval.

## Gold alignment

The [v3 gold arithmetic](EXTRACTION_QUALITY_V3.md) stays fixed: per-category
coverage at least 80%, global gold precision at least 80%, unaligned fraction
at most 20% (rounded up), zero mapped chronology violations and 100%
provenance. Empty denominators do not pass vacuously. Public names are
`gold_precision_percent`, `unaligned_percent` and `alignment_passed`; serialized
thresholds use `gold_precision_percent` and `unaligned_max_percent`. Corpus
input threshold keys remain unchanged for identity-only copying.

Expected facts retain `match` / `partial` / `absent`. Extracted characters,
relationships and world rules use `match` / `unaligned`, replacing the old
`hallucinated` wire label without changing its arithmetic. Old labels reject.
Expected event Match/Partial maps one-to-one to actual extracted event tokens;
Match alone earns full-event recall, while either mapping earns extracted
alignment. All actual events, including unmapped supported finer-grained
events, remain in the denominator. Other events or context cannot supply a
missing material part of a mapped event. Faithful translation remains valid.
World-rule supports still require literal excerpts of the referenced extracted
description; this proves locatability, not entailment. Existing cardinality,
mapping, chronology and provenance checks remain in force.

## Source support

The judge receives the complete fixed `case.source`, plus expected and actual
semantic payloads with opaque tokens. Source text is untrusted data, not
instructions. It is not reconstructed from the extractor's selected quotes.

Each extracted character/relationship/world-rule row adds required `support`.
Events add `extracted_event_support: [{extracted, support}]`, covering every
actual event, including unmapped ones. Each of the four actual token universes
must appear exactly once. Missing, duplicate, unknown, wrong-category tokens,
unknown labels and extra fields reject; no defaults or response repair.

- `supported`: the complete assertion is entailed, including attribution,
  uncertainty, negation, conditions, chronology and all material parts.
- `unsupported`: contradicted assertions, unjustified unconditional promotions
  and mixed true/false assertions. A genuine quoted sentence alone is not proof.
- `undetermined`: the complete source cannot resolve the assertion.

Gold alignment and source support are independent; `unaligned + supported` and
`match + unsupported` are valid observations. Validators must not force their
agreement. Each category reports only raw `total`, `supported`, `unsupported`,
`undetermined` and `exact_payload_repeats`. The three support counts sum to
the actual total, never dropping uncertain or repeated outputs. These are
judge-reported counts, not independently established truth.

Exact repeats are computed locally from same-category extracted JSON payloads,
excluding only the opaque token and preserving sequence, chapter, claim and
evidence. Count additional identical entries; do not deduplicate denominators.
This detects exact payload repetition, not semantic duplication. There is no
support ratio, ranking, source threshold or source PASS.

## Execution, bounds and reports

Serialize the complete system/user message list and reject above 128 KiB UTF-8
before judge dispatch, accounting for JSON escaping. Never truncate. This is
not a cap on HTTP headers or provider-added serialization. The existing 256 KiB
whole-corpus bound precedes extraction; generated judge input can exceed its
own limit later, and previous extraction usage must remain recorded.

Judge output remains capped at 800 tokens and the parsed response at 32 KiB.
An identical judge request may repeat once only for JSON/schema/rubric/token/
explanation violations. Existing transport retries and diagnostic reservations
remain unchanged. Valid low alignment, unsupported or undetermined observations
do not trigger retries. Offline shape tests do not establish that a live judge
can reliably produce this expanded response within 800 tokens.

Top-level `quality_status` is always `measurement_only`.
`measurement_completed` controls CLI exit 0: all required measurement and
evidence operations completed. `measurement_failures` lists failed case checks;
`alignment_failures` separately lists measured semantic cases below the gold
contract. In live mode, valid low alignment retains its source counts and can
complete measurement. Schema, provider, input, provenance, accounting and
output failures remain nonzero. An output failure can abort before a report is
available; prior private HTTP evidence is not discarded.

Each case has `case_check_passed`. Recorded mode requires every calibration,
negative-control mechanism and malformed/splitter check to meet expectations.
Its semantic cases expose their actual alignment and `expected_alignment_pass`,
even when an intentionally failing alignment correctly passes a negative
control. Recorded mode never fabricates source judgments. Splitter/malformed
cases and cases without a semantic measurement omit semantic score fields;
they do not contribute a synthetic semantic pass. Invalid live provenance
omits source counts. No public bare `passed` or `observed_pass` remains.

## Evidence limits and rollback

The [measurement design examples](H1_MEASUREMENT_DESIGN.md) state expected
semantics. Deterministic tests prove schema, accounting and request/report
mechanics; supplied judgments cannot prove a model follows the rubric. Semantic
reliability, independent source audits and any future qualification threshold
need new version-bound evidence. Preserve disagreements rather than relabeling
them as success. H1 quality and H4 release recovery remain separate open work.

Only necessary contracts and usage guidance belong in Git. Raw provider data,
credentials and acceptance reports remain private and outside the checkout.
Rollback is a revert of this tool/docs revision, never conversion of new
measurements into old identities or retrospective rescoring.
