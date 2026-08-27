# Horizon 1 extraction-quality evaluation gate

This tool implements the extraction-quality-v1 policy
(docs/EXTRACTION_QUALITY.md) over the checked-in synthetic corpus
corpus/v1.json without writing runtime data.

Recorded mode is deterministic and required in CI:

```bash
cargo run -p h1-eval -- --recorded --git-sha "$(git rev-parse HEAD)"
```

It proves the production structural gate (the deterministic chapter splitter,
the extraction schema validator, and the canon-model validator), corpus and
rubric integrity (versions, composition minimums, and thresholds must equal
the policy's), and calibration self-consistency: each recorded calibration
artifact must meet every policy threshold, and each adversarial mutation must
fail the exact threshold it targets (per-category coverage, precision and the
hallucination ceiling, provenance, chronology). It does **not** claim that a
current provider meets the semantic thresholds.

Live mode runs the production two-stage extraction contract — the production
character-extraction prompt (with chunk scan and merge when the source
exceeds the sample window, and first appearances verified against the split
chapters), the per-chunk canon-extraction prompts with production assembly and
validation — then one OpenAI-compatible judge call scores each category
against the expected-fact tables with the fixed rubric (match / partial /
absent / hallucinated). Provenance and chronology are enforced
deterministically on the provider output. Thresholds are enforced exactly as
versioned, and the hallucination ceiling rounds up so no fraction above the
policy bound can pass:

```bash
H1_EVAL_PROVIDER=openai \
LLM_API_URL=https://api.openai.com \
LLM_API_KEY=... \
LLM_MODEL=gpt-4o-mini \
cargo run -p h1-eval -- --live --git-sha "$(git rev-parse HEAD)"
```

Live baseline runs may add `--metrics-output <path>` to retain the existing
`llm-observability-v1` counters and latency summaries, including failed
attempts and retries. The path is live-only, and the report records
`thinking_enabled: false` because these schema-bound JSON calls deliberately
disable DeepSeek thinking. Raw metrics contain a stable usage-key fingerprint;
keep them in the private evidence directory and commit only the sanitized
aggregate produced for the reviewed evidence package.

Both modes fail closed on malformed corpus data, threshold drift from the
policy, missing judge categories, a non-commit SHA, a dirty checkout, or a
provider that violates the extraction schema. Reports record corpus/rubric
versions, the git SHA, provider/model identity, and no secrets, prompts, or
user data.

Malformed and unsupported inputs are labeled and never scored. The empty and
gapped-provenance labels exercise the production splitter and canon validator
directly; the oversized, invalid-encoding, and unsupported-format labels
assert the declared-limit contracts whose production rejection paths are
covered by the novel-service parser and handler tests. The GBK and BOM UTF-16
slices store decoded text with their slice identity; the decode paths
themselves are exercised by the novel-service document tests.
