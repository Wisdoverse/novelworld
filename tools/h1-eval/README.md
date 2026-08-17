# Horizon 1 extraction-quality evaluation gate

This tool implements the `extraction-quality-v1` policy
(`docs/EXTRACTION_QUALITY.md`) over the checked-in synthetic corpus
`corpus/v1.json` without writing runtime data.

Recorded mode is deterministic and required in CI:

```bash
cargo run -p h1-eval -- --recorded --git-sha "$(git rev-parse HEAD)"
```

It proves the production structural gate (the deterministic chapter splitter,
the extraction schema validator, and the canon-model validator), corpus and
rubric integrity (versions and thresholds must equal the policy's), and
calibration self-consistency (each recorded calibration artifact must meet
every policy threshold, and each adversarial mutation must fail it). It does
**not** claim that a current provider meets the semantic thresholds.

Live mode runs the full production two-stage extraction (character extraction
prompt, then per-chunk canon extraction prompts) through the configured
provider, then one OpenAI-compatible judge call scores each category against
the expected-fact tables with the fixed rubric (match / partial / absent /
hallucinated); provenance and chronology are enforced deterministically on the
provider output. Thresholds are enforced exactly as versioned:

```bash
H1_EVAL_PROVIDER=openai \
LLM_API_URL=https://api.openai.com \
LLM_API_KEY=... \
LLM_MODEL=gpt-4o-mini \
cargo run -p h1-eval -- --live --git-sha "$(git rev-parse HEAD)"
```

Both modes fail closed on malformed corpus data, threshold drift from the
policy, missing judge categories, a non-commit SHA, a dirty checkout, or a
provider that violates the extraction schema. Reports record corpus/rubric
versions, the git SHA, provider/model identity, and no secrets, prompts, or
user data.

Malformed and unsupported inputs are labeled, never scored: empty documents,
oversized inputs above the declared TXT limit, invalid UTF-8 bytes, gapped
provenance, and formats outside the declared quality slices each reach their
expected bounded error label.
