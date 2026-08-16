# NovelWorld Extraction Quality Policy

Version: **`extraction-quality-v1`**. This reviewed change approves the policy;
the corpus and evaluator judged against it land in a separate change and may
not weaken these thresholds.

This policy defines the non-vacuous quality gates that an import's accepted
canon must meet for each supported positive slice, and the labels that keep
rejection from passing the gate.

## Supported slices

| Slice | Positive cases | Adversarial cases | Evidence boundary |
|---|---|---|---|
| Simplified Chinese TXT | UTF-8 and GBK sources with expected characters, relationships, events, and world rules | Mixed-script chapter headers, hostile instructions in source text, future-spoiler text beyond the unlocked chapter | Live reports are provider-scoped; no provider/model is qualified here |
| English TXT | UTF-8 and BOM UTF-16 sources with the same expected-fact tables | Ambiguous chapter boundaries, hostile instructions, spoiler text | Same boundary |

Malformed and unsupported inputs are labeled separately from quality
judgment: invalid encoding, empty documents, oversized inputs, gapped
chapters, and unsupported types must each reach their expected bounded,
actionable error; they count toward import-success evidence but never toward
extraction scores. EPUB/PDF remain parser-acceptance evidence until a corpus
owns their quality slices.

## Metrics and thresholds — `extraction-quality-v1`

Live runs against the versioned corpus's expected-fact tables must meet, per
supported slice:

1. **Import success:** 100% of positive cases must yield a non-empty,
   structurally valid accepted canon (chapters contiguous, at least one
   character, canon model present). Recorded mode proves the same gate on the
   production extractor/splitter deterministically in CI.
2. **Coverage:** at least **80%** recall of expected facts per category —
   characters, relationships, events, world rules — each category judged
   separately; one weak category fails the slice.
3. **Precision:** at least **80%** of accepted facts must match an expected
   fact; at most **20%** of accepted facts may be hallucinated (no expected
   match).
4. **Chronology and causality:** **zero** accepted causal or chronological
   violations (an event caused by a later event, a death followed by
   dialogue, or order contradicting the expected sequence).
5. **Provenance:** **100%** of accepted canon facts must carry a valid source
   chapter citation within the reader's unlocked range.
6. **Anti-vacuity:** an empty accepted canon, a run that rejects every input,
   or an accepted fact table emptied to pass coverage all fail the gate
   regardless of other scores.

These thresholds are versioned here and enforced by `tools/h1-eval` in live
mode. Recorded mode enforces the structural import-success gate, corpus and
rubric integrity, and calibration self-consistency; it does not claim that any
provider meets the semantic thresholds.

## Judge rubric

A live run extracts each positive case with the operator-configured provider,
then an OpenAI-compatible judge scores each category against the expected-fact
tables using a fixed rubric (match / partial / absent / hallucinated, with
provenance checks). Judge output is schema-validated; malformed or missing
categories fail closed. Reports record provider, model, corpus/rubric
versions, and the exact git SHA, and contain no secrets, prompts, or user
data.

## Change rule

Thresholds change only through a reviewed policy change approved before the
implementation judged against it; a candidate change cannot weaken its own
gate. Baseline-only reports may be recorded before thresholds are met, but
they cannot claim qualification.
