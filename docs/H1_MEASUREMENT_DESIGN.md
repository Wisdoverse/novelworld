# H1 semantic measurement: prospective design

The [v4 measurement contract](EXTRACTION_QUALITY_V4.md) implements the distinction
below as separate gold alignment and judge-reported source observations, not
a new source-quality threshold or a model quality result. The historical
[v3 policy](EXTRACTION_QUALITY_V3.md), old corpus and reports remain frozen;
gold arithmetic is preserved under new version identities. There is no
authorization for a paid run or retrospective rescore here.

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
| Source: “Mei handed Bo the map. Bo did not burn it.” Gold: the handover and retained map. | “Mei handed Bo the map, and Bo burned it.” | Mixed true/false assertion: partial gold coverage at most, never source-supported as a whole. |

## Smallest implementation path and acceptance

V4 keeps the existing evaluator and gold thresholds, adds exact-payload repeat
counts without deduplication, and preserves unknown source judgments. It does
not invent a source-quality threshold to fit #229 or reinterpret its failed
result. Offline tests cover the affected evaluator/rubric paths and existing
mapping/chronology/empty-output guards.

Mechanical checks can prove identifier uniqueness, shapes, arithmetic and
locatable quotations. Independent semantic review must judge entailment and
material completeness; a prompt instruction or recorded answer cannot prove
that a live judge follows it. Real acceptance requires independently audited,
new version-bound evidence, with disagreements recorded rather than hidden by
an aggregate pass. Any provider run needs its own prospective immutable
registration and enforceable budget under the existing qualification policy.

The implementation of a measurement contract does not establish semantic
reliability. H1 measurement reliability, #229's model quality and #222's formal
qualification remain unresolved; this document closes none of those outcomes.
