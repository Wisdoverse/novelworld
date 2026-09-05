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

Live mode reuses the production domain prompts, JSON request builder, chunk
scan/merge, first-appearance proof, canon reference canonicalization,
assembly, and validation. It intentionally does not reproduce the
application handler's fresh-response schema-repair loops: a schema-invalid
character or canon response fails the qualification case. An
OpenAI-compatible judge scores each category against the source-grounded
expected-fact tables with the fixed rubric (match / partial / absent /
hallucinated). Judge inputs contain semantic facts and opaque response tokens,
not fixture or runtime IDs. Expected-to-extracted event mappings make
relative event order a deterministic check; production canon validation
separately rejects structurally forward causes. The evidence is bounded to the
versioned corpus facts and does not independently prove every possible
semantic cause/death-continuity pattern. Thresholds remain exactly as
versioned, and the hallucination ceiling rounds up so no fraction above the
policy bound can pass.

The application makes one judge request. It repeats that identical request
once only when the response violates the judge JSON/schema/rubric/token/
explanation contract. It does not add a retry for a transport failure or a
valid low score. The production LLM client retains its documented transport
retry contract.

```bash
H1_EVAL_PROVIDER=deepseek \
LLM_API_URL=https://api.deepseek.com \
LLM_API_KEY=... \
LLM_MODEL=deepseek-v4-flash \
H1_EVAL_ALLOWED_RESPONSE_MODELS=deepseek-v4-flash \
cargo run -p h1-eval -- --live --git-sha "$(git rev-parse HEAD)" \
  --metrics-output /private/h1-metrics.prom \
  --private-responses-output /private/h1-responses.jsonl
```

Every live run requires both evidence outputs. `--metrics-output` retains the
existing `llm-observability-v1` counters and latency summaries, including
failed attempts and retries. The report records
`thinking_enabled: false` because these schema-bound JSON calls deliberately
disable DeepSeek thinking. Production character and canon extraction also use
temperature 0.0 to make qualification and accepted imports deterministic. Raw
metrics contain a stable usage-key fingerprint;
keep them in the private evidence directory. Both output paths must be
absolute, outside the Git checkout, inside existing directories, and fresh:
the evaluator creates each file exclusively before any provider call.
`--private-responses-output` records every non-streaming HTTP response before
parsing, JSON fallback or transport retry, and flushes each JSONL record. Private
schema version 2 stores `sequence`, `case_id`, `operation`, `logical_attempt`,
`http_status`, `complete` and exact `body` bytes (a JSON integer array). The body
is capped at 1 MiB; truncated, oversized or deadline-cancelled bodies retain the
available prefix with `complete: false`. No envelope exists when a request fails
before response headers. Files are created exclusively; Unix permissions are 0600. Writes use synchronous
local filesystem I/O with no retry; the HTTP deadline cannot preempt a stuck
filesystem, and flush is not an fsync durability guarantee.

Every complete successful envelope, including an empty JSON-mode reply, must
contain an allowed model and valid usage. Invalid or incomplete evidence, a
private-write failure or a total timeout stops further provider calls across the
run. Complete HTTP errors remain subject to the existing bounded retry policy.
`private_response_count` counts flushed HTTP envelopes, including failed replies;
it is not the logical-request or judge-attempt denominator. The public model set
includes all validated observed aliases, even before fallback or a later failure.
Rejected-envelope tokens may be absent from metrics: reconcile the private bytes
when diagnosing cost; such a run cannot qualify. These private files may contain model output
or stable fingerprints and must never be committed; publish only the
sanitized aggregate produced for the reviewed evidence package. The public
report records only the configured model, allowlisted observed response-model
identifiers, attempt counts, typed failure codes, and aggregate scores.

Both modes fail closed on malformed corpus data, threshold drift from the
policy, missing or duplicate judge tokens, an unregistered response-model
identifier, a non-commit SHA, a dirty checkout, or a provider that violates
the extraction schema. Reports record corpus/rubric/prompt versions, the git
SHA, provider/model identity, and no secrets, prompts, raw responses, or user
data.

Malformed and unsupported inputs are labeled and never scored. The empty and
gapped-provenance labels exercise the production splitter and canon validator
directly; the oversized, invalid-encoding, and unsupported-format labels
assert the declared-limit contracts whose production rejection paths are
covered by the novel-service parser and handler tests. The GBK and BOM UTF-16
slices store decoded text with their slice identity; the decode paths
themselves are exercised by the novel-service document tests.
