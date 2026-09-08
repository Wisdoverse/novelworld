# H1 semantic measurement: prospective design

Status: design only, not an adopted scoring policy or quality result. This
addresses the distinction between gold alignment and source support described
below, not the frozen provider failure itself. Current
[v3 policy](EXTRACTION_QUALITY_V3.md), corpus,
rubric, thresholds, denominators and historical reports remain unchanged.
There is no authorization for a paid run or retrospective rescore here.

## Three distinct questions

- **Gold coverage:** which expected facts did the output cover? An omission is
  not a fabrication; a supported additional event is not additional gold recall.
- **Source support:** does the source entail the output's assertion, including
  speaker, uncertainty, scope and conditions? A real citation alone is not proof.
- **Event completeness:** does the single mapped event contain every material
  part of the expected event in its own fields? Other events or story context
  cannot repair a partial mapping. Faithful translation does not need verbatim text.

The current one-to-one v3 calculation counts unmapped extracted events under
its inherited hallucination label. That number must not be presented as a
direct count of unsupported assertions. Conversely, correcting that label
must not excuse a genuinely unsupported rule or an incomplete event match.

## Fresh synthetic counterexamples

These examples are newly authored, not copied from private provider output.
They specify semantic expectations, not a new judge response schema.

| Short source and gold target | Extracted assertion | Expected disposition |
|---|---|---|
| Source: “At dusk Mei reached the dock and handed Bo the sealed map.” Gold: that whole event. | “At dusk Mei arrived at the dock and gave Bo the sealed map.” | Full gold coverage, source-supported, complete event. |
| Same source and gold. | “Mei possessed a sealed map.” | Source-supported state, but not a complete match for arrival and handover; no full-event recall. |
| Same source. Gold covers the arrival and handover. | A complete matching event plus an additional event: “Bo received the sealed map.” | Gold covered once; the extra event is supported finer granularity, not another gold match and not a fabricated fact. Duplication remains separately visible. |
| Source: “The unsigned letter claimed that every gate opens at dawn. Mei doubted it.” Gold: the letter's claim and Mei's doubt. | “Every gate always opens at dawn.” | Unsupported unconditional world rule; drops attribution and uncertainty. A literal quotation from the letter would not entail this rule. |
| Source: “雨停后，林舟把钥匙交给阿宁。” Gold: after the rain stopped, Lin Zhou handed A Ning the key. | “Once the rain ended, Lin Zhou gave A Ning the key.” | Complete source-supported cross-language match; spelling/wording differences alone are not omissions. |
| Source: “Bo touched the bell. The door stayed shut.” Gold: touching the bell and the unchanged door. | “Touching the bell opened the door,” citing those exact sentences. | Real, locatable citation but contradicted assertion; fails source support and complete gold coverage. |

## Smallest implementation path and acceptance

Keep the existing evaluator. First review a versioned policy proposal that
separates the three questions above, explicitly defines split/duplicate-event
treatment, denominators and anti-vacuity, and declares prospective thresholds.
Do not change thresholds to fit #229 or reinterpret its existing failed result.
Then update only the affected evaluator/rubric paths with offline tests for
these counterexamples and the existing mapping/chronology/empty-output guards.

Mechanical checks can prove identifier uniqueness, shapes, arithmetic and
locatable quotations. Independent semantic review must judge entailment and
material completeness; a prompt instruction or recorded answer cannot prove
that a live judge follows it. Real acceptance requires independently audited,
new version-bound evidence, with disagreements recorded rather than hidden by
an aggregate pass. Any provider run needs its own prospective immutable
registration and enforceable budget under the existing qualification policy.

Until that work passes, this is a reviewed design direction only. H1 semantic
measurement reliability, #229's model quality and #222's formal qualification
remain unresolved; this document closes none of those outcomes.
