# Horizon 3 offline evaluation gate

This tool evaluates the checked-in synthetic Horizon 3 corpus without writing
runtime data.

Recorded mode is deterministic and required in CI:

```bash
cargo run -p h3-eval -- --recorded --git-sha "$(git rev-parse HEAD)"
```

It proves the production structural validators, replay behavior, report
contract, and curated semantic-rubric calibration. It does **not** claim that a
current provider meets semantic quality thresholds.

Live mode runs the same semantic calibration cases through an
OpenAI-compatible judge and records the provider/model identity:

```bash
H3_EVAL_PROVIDER=openai \
LLM_API_URL=https://api.openai.com \
LLM_API_KEY=... \
LLM_MODEL=gpt-4o-mini \
cargo run -p h3-eval -- --live --git-sha "$(git rev-parse HEAD)"
```

Both modes fail closed on missing categories, malformed judge output, lowered
thresholds, incomplete samples, unsupported versions, or a non-commit SHA.
Release enforcement of a live report is deliberately a later roadmap slice.
