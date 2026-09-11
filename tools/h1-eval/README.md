# Horizon 1 extraction-quality measurement

`tools/h1-eval` implements the `extraction-quality-v4` policy
([policy](../../docs/EXTRACTION_QUALITY_V4.md)) over the checked-in
`h1-synthetic-v6` corpus (`corpus/v6.json`). It is Structural, measurement-only
evidence, not formal Qualification or a provider-quality claim. Historical
policy files, the v1 corpus, frozen H4 cohort guard, and failed reports remain
immutable and are not rescored or compared.

Active identities: policy `extraction-quality-v4`, corpus `h1-synthetic-v6`,
response rubric `h1-extraction-v4`, semantic judge `h1-semantic-judge-v9`,
public report schema `4`, and private HTTP response envelope `2`.

Public reports explicitly carry these identities and distinguish gold alignment
from source-support measurements. `quality_status` is always
`measurement_only`; exit status 0 means `measurement_completed`, not that a
quality threshold passed.

Recorded mode is deterministic and required in CI:

```bash
cargo run -p h1-eval -- --recorded --git-sha "$(git rev-parse HEAD)"
```

Recorded mode proves structural extraction/splitter/canon validation, corpus and
rubric integrity, calibration self-consistency, and adversarial controls. It
does not produce source-support measurements or claim a provider meets semantic
thresholds.

Live mode reuses the production domain prompts, JSON request builder, chunk
scan/merge, first-appearance proof, canon reference canonicalization,
assembly, event-selection validation, and existing transport retry contract.
It intentionally does not reproduce application fresh-response schema-repair
loops: a schema-invalid character or canon response fails the case. Existing
event mapping, chronology, provenance, anti-vacuity, and gold thresholds remain
unchanged.

The judge receives the complete fixed case source plus semantic facts and opaque
response tokens, never fixture/runtime IDs. Complete serialized system and user
messages, including JSON escaping, must remain within 128 KiB; inputs above
that bound fail before judge I/O and are never truncated. The corpus whole-file
bound remains 256 KiB. Judge output remains capped at 800 tokens and 32 KiB
response bytes. These are contract limits; this README does not claim a live
provider has been verified against the new 800-token shape.

Each semantic case reports gold alignment independently from support. Public
gold fields are `gold_precision_percent`, `unaligned_percent`, and
`alignment_passed`; there are no bare `passed`, `observed_pass`,
`precision_percent`, or `hallucination_percent` public keys. Splitter and
malformed cases omit semantic fields and report only `case_check_passed`.
Recorded adversarial cases retain their measured alignment and expected-failure
control. No aggregate semantic pass count includes the local splitter.

The `source_support` measurement has four raw category counts (characters,
relationships, events, and world rules); each retains `supported`,
`unsupported`, and `undetermined` results in its denominator. Local exact
payload repeats are reported as
`exact_payload_repeats`; they retain category payload identity including
sequence/chapter/claim/evidence and are not semantically deduplicated. Support
and gold alignment may disagree. These counts describe judge judgments, not
independently proven truth. Non-semantic, measurement-failed and recorded cases
have no source-support result; they must not be fabricated as zero or supported.
A completed live case with low alignment retains its source-support counts.

The judge contract requires complete token coverage for all four actual output
universes and rejects old/missing/extra fields and old rubric labels. A
schema-valid low alignment score, unsupported result, or undetermined result is
completed measurement and is not retried. The one identical application-level
retry remains only for malformed JSON/schema/rubric/token/explanation or other
contract violations. Transport retries remain owned by the production LLM
client. No source-quality threshold or pass is inferred from support counts.

The evaluator's source-support and exact-repeat logic is a measurement contract,
not semantic proof. Prompt and recorded tests prove mechanics only. The
[prospective measurement design](../../docs/H1_MEASUREMENT_DESIGN.md) supplies
the distinction between gold coverage, source support, and event completeness;
it does not authorize a paid run or formal adoption by itself.

Live mode requires fresh private evidence outputs:

```bash
H1_EVAL_PROVIDER=deepseek \
LLM_API_URL=https://api.deepseek.com \
LLM_API_KEY=... \
LLM_MODEL=deepseek-flash \
H1_EVAL_ALLOWED_RESPONSE_MODELS=deepseek-flash \
cargo run -p h1-eval -- --live --git-sha "$(git rev-parse HEAD)" \
  --metrics-output /private/h1-metrics.prom \
  --private-responses-output /private/h1-responses.jsonl
```

Both paths must be absolute, fresh, outside the checkout, and inside existing
directories. Metrics retain `llm-observability-v1` attempts, retries, usage and
latency. Private response envelope schema 2 records every non-streaming HTTP
response before parsing/fallback/retry, with bounded body bytes and 0600 Unix
permissions. Raw responses and metrics can contain model output or stable
fingerprints and must never be committed; publish only a sanitized aggregate.
Missing/inconsistent usage or evidence, timeout, request/write error, or private
write failure stops further provider calls and retains the unproven reservation.

## Bounded Vision Diagnostic (opt-in)

New paid Diagnostic work is allowed only with `--live --bounded-diagnostic`, both
private output paths, and a separately registered immutable input/model/budget
under the prospective fixed `vision-diagnostic-budget-v2` profile:

```bash
H1_EVAL_PROVIDER=deepseek \
LLM_API_URL=https://api.deepseek.com \
LLM_API_KEY=... \
LLM_MODEL=deepseek-flash \
H1_EVAL_ALLOWED_RESPONSE_MODELS=deepseek-flash \
cargo run -p h1-eval -- --live --bounded-diagnostic \
  --git-sha "$(git rev-parse HEAD)" \
  --metrics-output /private/vision-diagnostic-metrics.prom \
  --private-responses-output /private/vision-diagnostic-responses.jsonl
```

This is non-qualifying evidence only. It cannot rerun or repair the frozen
formal cohort, lower thresholds, authorize a production model switch, or unlock
H4/formal Qualification. Profile ceilings and accounting remain enforced; they
are not account-wide or provider-enforced caps.

The fixed per-invocation profile permits at most 40 logical calls, 200 HTTP
attempts, 20,000,000 cumulative reserved/settled tokens and 35,000,000 micro-CNY
(CNY 35). Each logical call reserves five attempts at worst-case `2^20` input
tokens plus the request output cap, using four input and twelve output
micro-CNY per token. These coefficients conservatively cover the selected
DeepSeek V4.1 Flash peak prices under the policy's hard 10 CNY/USD bound.
Only complete metrics/private usage reconciliation releases unused reservation.
Uncertain accounting stops dispatch; restarting does not authorize another run.
The profile's 8192-token maximum for other operations does not raise the judge's
800-token cap. Revalidate provider context, output and prices at registration;
these frozen accounting assumptions are not a statement of current prices.
See [Qualification policy](../../docs/QUALIFICATION_POLICY.md) and
[`budget.rs`](src/budget.rs) for the registration and enforcement contract.

No current v4 live quality result is implied here. Fresh registration,
independent review, private evidence, and prospective approval are required
before any provider call. Existing formal Qualification remains on its frozen
v1 identity and cannot silently adopt v4.

Both modes fail closed on malformed corpus data, threshold drift, missing or
duplicate tokens, unregistered response models, non-commit SHA, dirty checkout,
provider schema violations, or evidence/accounting failures. Reports contain
versions, git SHA, provider/model identity, measurement status and sanitized
aggregates only; they contain no secrets, prompts, raw responses, or user data.
