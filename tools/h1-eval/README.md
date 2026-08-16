# Horizon 1 extraction quality gate

This tool judges the checked-in `h1-extraction-v1` corpus against the
approved [`extraction-quality-v1`](../../docs/EXTRACTION_QUALITY.md) policy
without writing runtime data.

Recorded mode is deterministic and required in CI:

```bash
cargo run -p h1-eval -- --recorded --git-sha "$(git rev-parse HEAD)"
```

It proves, on the production extractor and splitter from novel-service:

- the import-success gate for the supported positive slices (zh/en TXT in
  UTF-8, GBK, and BOM UTF-16) — contiguous non-blank chapters with the
  expected anchors;
- the labeled malformed/unsupported error contract (invalid UTF-16, empty
  document, unsupported type, oversized input);
- the curated rubric calibration (coverage, precision/hallucination,
  chronology, provenance, anti-vacuity verdicts).

It does **not** claim that any provider meets the semantic thresholds; live
scoring of a provider run against the corpus's expected-fact tables is a
separate reviewed slice, and its thresholds are fixed by the policy change
that approved them.

`--self-test` mutates the corpus and thresholds in memory and proves the
gate fails closed on each weakening.

Both modes fail closed on missing categories, lowered thresholds, malformed
corpus or calibration fixtures, empty expected-fact tables, duplicate case
ids, or a non-commit SHA.
