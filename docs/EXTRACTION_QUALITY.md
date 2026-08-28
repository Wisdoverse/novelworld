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
   chapter citation within the reader's unlocked range. The `h1-eval` gate
   counts every accepted fact in the denominator: extraction-layer characters
   and relationships (chapter citations verified against the source — a
   relationship citation must not predate either endpoint's verified first
   appearance) plus canon facts (verbatim excerpts via `canon.validate`).
6. **Anti-vacuity:** an empty accepted canon, a run that rejects every input,
   or an accepted fact table emptied to pass coverage all fail the gate
   regardless of other scores.

These thresholds are versioned here and enforced by `tools/h1-eval` in live
mode. Recorded mode enforces the structural import-success gate, corpus and
rubric integrity, and calibration self-consistency; it does not claim that any
provider meets the semantic thresholds.

## Judge rubric

A live run extracts each positive case with the operator-configured provider,
then an OpenAI-compatible judge scores each category against source-grounded
expected-fact tables using a fixed rubric (match / partial / absent /
hallucinated, with provenance checks). The judge sees semantic facts and
opaque response tokens rather than fixture/runtime IDs. Expected-to-extracted
event mappings drive a deterministic relative-order check for matched corpus
events; canon validation separately rejects structurally forward causes. This
evidence remains bounded to the versioned corpus and is not an independent
claim about every possible semantic cause or death-continuity pattern. Judge
output is schema-validated with exact token coverage; an identical application-level
request is repeated once only for an invalid JSON/schema/rubric/token/
explanation contract, never for a transport failure or a valid low score.
Unregistered response-model identifiers and malformed or missing categories
fail closed. Reports record provider, configured/observed model,
corpus/rubric/prompt versions, attempt metadata, and the exact git SHA, and
contain no secrets, prompts, raw responses, or user data. Raw responses belong
only in a required fresh private evidence file outside the checkout; live
runs also require a fresh private metrics file so attempts, retries, usage,
and latency cannot be omitted from qualifying evidence.

## Change rule

Thresholds change only through a reviewed policy change approved before the
implementation judged against it; a candidate change cannot weaken its own
gate. Baseline-only reports may be recorded before thresholds are met, but
they cannot claim qualification.
