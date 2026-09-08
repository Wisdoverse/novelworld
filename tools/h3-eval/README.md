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

Live baseline runs may add `--metrics-output <path>` to retain the existing
`llm-observability-v1` counters and latency summaries, including failed
attempts and retries. The path is live-only, and the report records
`thinking_enabled: false` because these schema-bound JSON calls deliberately
disable DeepSeek thinking. Raw metrics contain a stable usage-key fingerprint;
keep them in the private evidence directory and commit only a sanitized
aggregate. Every successful HTTP envelope is checked before fallback: models
must be valid identifiers and usage must be present and valid. The report retains
all observed model aliases, including earlier fallback responses and failed
judgments. Evidence failure stops subsequent live cases. The ordinary live interface does not retain raw
bodies; H1's separate private JSONL capture is required for H1 qualification.
The explicit registered H3 Diagnostic below has its own private capture contract.

Both modes fail closed on missing categories, malformed judge output, lowered
thresholds, incomplete samples, unsupported versions, or a non-commit SHA.
Release enforcement of a live report is deliberately a later roadmap slice.

## Separately registered Vision calibration Diagnostic

A reviewed private-iteration calibration can use the explicit Linux-only path:

```text
h3-eval --live --git-sha <exact-clean-commit> --diagnostic-registration <absolute-private-registration.json>
```

This interface is tooling, not permission to call a provider. A future run needs
its own independently approved, immutable registration. It cannot share an H4
run, namespace, allowance, or evidence, and cannot resume a consumed invocation.
The ordinary recorded/live interfaces above retain their existing contracts.

The corpus still has 24 cases: sixteen deterministic checks and eight live
semantic judgments. The eight requests retain the same prompts, rubric,
thresholds, temperature zero, thinking disabled, and 800-token output ceiling.
There is no application-level judge retry or case selection. Shared transport
retains at most three retries and one empty-JSON fallback, with at most five
HTTP attempts per logical call. A calibration result does not establish
production conversation quality, durable-memory usefulness, or formal H3's
separate 72/72 final-cohort requirement.

Set `H3_EVAL_PROVIDER=deepseek`, `LLM_API_URL=https://api.deepseek.com`, and
`LLM_MODEL=deepseek-v4-flash-vision-exp`; the approved execution mechanism supplies
`LLM_API_KEY` without putting it in the registration. Runtime
`LLM_DIAGNOSTIC_BUDGET_ID`/`LLM_DIAGNOSTIC_BUDGET_LIMITS` must be absent or empty.
This standalone control does not provision or reuse a service-owned allowance.
It performs no account, balance, or model-inventory query.

The registration is a strict JSON object (no unknown or duplicate fields):

| Field | Exact contract |
| --- | --- |
| `schema_version`, `profile` | `1`, `h3-vision-calibration-diagnostic-v1` |
| `hypothesis` | Nonempty, printable, at most 2,000 bytes |
| `git_sha`, `executable_sha256` | Exact clean commit and SHA256 of the running executable |
| `source_sha256` | Exact path-to-SHA256 map for the eight files listed below |
| `prompt_sha256` | SHA256 of compact JSON containing the eight request message arrays in corpus order, each message serialized as `role` then `content` |
| `provider`, `api_url`, `model` | `deepseek`, `https://api.deepseek.com`, `deepseek-v4-flash-vision-exp` |
| `allowed_response_models` | Exactly `["deepseek-v4-flash-vision-exp"]` |
| `semantic_cases` | The eight `semantic_cases` IDs from the unchanged corpus, in its original order |
| `limits` | Exactly `{"logical_calls":8,"attempts":40,"tokens":20000000,"cost_micro_cny":35000000}` |
| `not_before_unix`, `expires_unix` | Integer UTC Unix seconds; current time must be in this half-open interval, whose length is at most 86,400 seconds |
| `max_lifetime_seconds` | Exactly `3600`; the earlier registration expiry also stops work |
| `output_directory` | Fresh absolute path under an existing owned private directory outside Git |

The exact `source_sha256` keys are `tools/h3-eval/src/main.rs`,
`tools/h3-eval/src/budget.rs`, `tools/h3-eval/src/diagnostic.rs`,
`tools/h3-eval/corpus/v1.json`, `tools/h3-eval/Cargo.toml`, `Cargo.toml`,
`Cargo.lock`, and `tools/llm-budget/policy-v2.json`. The commit binds the remaining
tracked dependencies; the executable digest binds the actual reviewed binary.
Do not generate registration values by copying a previous paid run.

The registration is an owned 0600 regular file under a 0700 parent. Its parent
and the output parent must be outside any enclosing Git checkout, with no
symlink path components. The runner checks each exact parent through Git with
inherited Git overrides cleared. Diagnostic checkout commands trust only the
canonical invocation directory through a process-local `safe.directory` setting;
ordinary recorded/live commands preserve their prior scoped Git environment.
It validates identities before reading the key,
creates the output directory exclusively, and syncs `started.json` before any
possible paid I/O. Even an empty directory left by a crash remains consumed.
There is no reset, cleanup, refill, or resume option.

The local control adapts H1's conservative reservation and complete-usage
reconciliation. Before each logical call it syncs a reservation for all five
possible HTTP attempts to `control.jsonl`. It syncs settlement before releasing
unused allowance. Each attempt is conservatively quoted at at most 1,048,576
input tokens and 800 output tokens, using three and nine micro-CNY per respective
token. These are inherited control assumptions, not a claim about current
invoiced prices; the later paid registration must independently verify official
context/pricing assumptions. Outer limits may stop the run before eight calls
complete; they are never increased to fill the sample count.

The observer retains responses before validating complete envelopes, exact
returned model, and usage bounds, including responses that could trigger JSON
fallback. Missing or invalid evidence stops further provider calls. An ordinary
transport error may use the remaining already-reserved retries in that logical
call; if any attempt remains unaccounted afterward, the full ticket remains
charged and no later logical case runs. A complete but schema-invalid judgment
fails its case without repair or another judge call; subsequent fixed cases may
continue only with complete accounting. Independent semantic disagreements stay
separate from the evaluator's original scores.

Private output is fixed: `started.json`, `control.jsonl`, `responses.jsonl`,
`metrics.prom`, `report.json`, and `terminal.json`. Raw envelopes contain the
complete judge response and case linkage; no redundant prose capture is needed.
The Diagnostic aggregate has schema version 2, mode `diagnostic`, and an explicit
Diagnostic profile; it is not a formal schema-1 live report. It omits raw prose,
explanations, private paths and registration/run identifiers. Raw responses and
metrics remain private, including on failure; metrics contain a usage-key
fingerprint. A kill can leave no terminal file: the consumed directory and
synced reservation journal remain authoritative, with no execution recovery.

Bounds are enforced before writes: 1 MiB raw HTTP body, at most forty envelopes,
4 MiB + 8 KiB per serialized response record, and 161 MiB total raw JSONL. Other
caps are 32 KiB registration, 4 KiB Started, 64 KiB terminal, 256 KiB report,
1 MiB metrics, and 32 control records of at most 4 KiB each. Total namespace
artifacts stay below 163 MiB. Overflow or failed writes/syncs fail closed.
