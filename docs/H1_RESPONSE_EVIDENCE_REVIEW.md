# H1 provider-response evidence correction

Issue: [#262](https://github.com/Wisdoverse/novelworld/issues/262). Base:
`8df0d7c6a03582d61e6046e2a77cb27bf605e4f4`.

## Plan and design review

The accepted [plan and implementer design review](https://github.com/Wisdoverse/novelworld/issues/262#issuecomment-5549638666)
keeps production prompts, corpus, judge thresholds, model selection and retry
schedules fixed. A loopback server demonstrated the defect: an empty JSON-mode
HTTP 200 from an unregistered model without usage was followed by a valid final
response. The original H1 sink and model set retained only the final response.
The base's 448 workspace tests and 19/19 recorded H1 cases did not detect this.

Capture must run before transport parsing or fallback. An array attached only
to the final ChatResponse cannot stop a failed write or invalid first envelope
before another provider call. The implementation therefore uses an opt-in,
request-scoped callback on the existing bounded body reader. It adds no service,
dependency, runtime raw logger, provider route or persistence abstraction.

## Implementer adversarial review

Reviewed paths and resulting behavior:

- Non-streaming OpenAI-compatible Chat Completions and DeepSeek Responses both
  capture successful and error HTTP envelopes. Embedding and SSE retain their
  existing readers; a streaming request with the new observer fails before I/O.
- H1 representative character extraction, character chunk scans, canon scans,
  both event-selection attempts and both semantic-judge attempts attach the
  observer before sending. JSON fallback and transport retries keep the same
  callback context. Sequence numbers identify each HTTP envelope; logical
  attempts retain their existing meaning.
- H1 flushes exact bounded bytes before inspecting the envelope. Wrong models,
  malformed successful envelopes, missing/invalid usage, partial bodies and
  write failures invalidate the run before fallback or later cases. Private
  records include rejected models; the public set contains only allowed models.
- A total timeout can cancel the body future without returning a read error.
  The capture's drop path retains the prefix with `complete: false`. Timeout
  before headers produces no invented envelope and is a terminal evidence error;
  H1 and H3 stop subsequent calls.
- H3 collects all valid observed model IDs before fallback and preserves them
  even if the judgment later fails. It has no H1-style raw artifact and does not
  gain H1 qualification from this change.
- Normal runtime JSON fallback logs the earlier response model and counts known
  tokens once. If the empty successful response has no usage, the logical
  success has one missing-usage report, even when the final response has usage.
  `present + missing == successes` and the original retry schedule are retained.
- Callback errors expose a constant typed error, never callback error text or
  raw provider bytes. Private JSONL v2 is outside the checkout, created
  exclusively and mode 0600 on Unix. Its byte array preserves invalid UTF-8.

Review-discovered corrections included replacing an invalid metric assertion
(model observations are tracing, not Prometheus labels), retaining partial
bodies during cancellation, and making a pre-header timeout invalidate later
cases. No known blocking implementation finding remains from this implementer
review. Independent review and required CI are still merge gates.

## Runnable evidence

`cargo test --locked -p llm-client -p h1-eval -p h3-eval` includes loopback
regressions for wrong-model and missing-usage first replies; malformed,
truncated, oversized and timeout-cancelled bodies; actual private-write failure;
valid JSON fallback plus a semantic-judge retry; both non-streaming HTTP routes;
HTTP error capture with Retry-After; exact logical usage/token accounting; and
H3's full model set and stopping subsequent cases. Existing production prompt
and evaluator contract tests remain in that command.

The affected gate matrix is `cargo fmt`, architecture self-test/check, workspace
check/test/Clippy excluding the external integration suite, the seven Python
budget tests, and both recorded evaluators run twice at a clean commit. Exact
results and commit identity belong in the linked PR and issue, not an inferred
claim from this source review.

One local build failed when the shared sccache could not find a dependency
`.rmeta`. Validation moved to an isolated Cargo target with sccache disabled;
this infrastructure failure is not counted as a passing run.

## Proof limits and recovery

All new provider calls in these regressions are synthetic loopback requests.
There is no paid generation, provider-account query, live qualification,
production deployment, independent approval or human accessibility evidence.
#236 remains blocked until this correction is merged and a fresh clean-main
registration and operator authorization are in place. #229's same-model stop
remains in force.

Rejected-envelope tokens may be absent from runtime counters because validation
runs first. Reconcile private bytes for diagnosis; never qualify such a run.
The 1 MiB cap bounds each retained body, not total disk use. Writes flush through
ordinary synchronous local filesystem I/O with no retry; the async HTTP deadline
cannot preempt a stuck filesystem. Process or host loss is not an fsync-backed
artifact durability guarantee.

Rollback is a revert of this commit series; there are no migrations. Do not
resume H1 qualification on the reverted final-response-only behavior. Retain
existing private evidence and regenerate qualification artifacts after the
corrected clean main has been registered. Rollback has been source-reviewed,
not deployed or exercised against a live provider.
