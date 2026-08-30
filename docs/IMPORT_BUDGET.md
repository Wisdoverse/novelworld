# NovelWorld Import Provider Budget Policy

Version: **`import-provider-budget-v1`**. The import claim path enforces this
policy; changes to its thresholds and enforcement must land together.

This policy bounds application-port dispatches by the ingestion pipeline
across the crash window where a provider outcome is unknowable. It does not
promise exactly-once execution, cap exact spend, or count physical HTTP
requests. A fresh `LlmPort::chat_json` dispatch and an `ImagePort::generate`
dispatch are the units enforced here; transport behavior is described
separately below.

## Contract — `import-provider-budget-v1`

1. **Per-attempt application-dispatch ceiling.** The import reserves at most
   **640 application dispatches**: 610 fresh `LlmPort::chat_json` dispatches
   plus at most 30 `ImagePort::generate` dispatches. The atomic LLM token is
   consumed immediately before every port dispatch, including JSON/schema/
   evidence retries and chapter-boundary repair, so concurrency cannot exceed
   the 610 ceiling. The avatar cap reserves the other 30 slots.
   `ensure_import_budget` also rejects a source whose deterministic baseline
   scan plan cannot fit. For the gate's single-chapter 5 MiB boundary fixture,
   that forecast is **4 fixed + 221 character scans + 328 canon scans + 30
   images = 583**, which leaves 57 LLM tokens for validation or boundary-repair
   dispatches. Chapter count is also part of the scan plan, so a highly
   fragmented source can be rejected below the byte envelope. The four fixed
   slots are representative character extraction, narrative-node detection,
   and up to two whole-novel event-selection responses. Event selection is
   skipped without a dispatch when its complete candidate prompt exceeds 16
   KiB.
2. **Attempt ceiling.** A `novel_import_jobs` row MUST NOT be claimed more
   than **3** times. Attempt counting includes the acceptance claim and every
   recovery, lease-expiry, or user-retry claim.
3. **Cross-attempt application-dispatch ceiling.** Derived from (1) and (2):
   at most **3 × 640 = 1920 application-port dispatches** per import. This is
   not a physical HTTP-request or exact-spend ceiling.
4. **Terminal semantics.** A claim attempt for a job already at the ceiling
   MUST mark the job terminally `failed` with failure code
   `budget_exhausted`, set the Novel to `error` with the actionable public
   message "Import provider budget exhausted; re-upload the source", and the
   job MUST never be reclaimed by the recovery scan or resumed by the retry
   endpoint. Re-uploading creates a new import with a fresh budget.
5. **Metering and retry proof boundary.** The enforcement evidence is the
   per-claim atomic 610-token LLM-dispatch budget plus the reserved 30 image
   slots. Cross-attempt evidence is the persisted `job.attempt`; structured
   logs and `llm-observability-v1` metrics remain operational evidence. Local
   validation loops are bounded to three fresh responses (initial + two) for
   representative, character-scan, narrative-node, and chapter-boundary JSON;
   three fresh responses for each canon chunk's JSON/evidence gate; and two
   fresh responses for event selection. All consume the same global 610-token
   budget. Separately, the current shared LLM client bounds one port dispatch
   to an initial transport invocation plus at most three retryable invocations
   and at most one JSON-mode fallback invocation. Those transport invocations
   are not additional application-budget tokens, and this policy does not
   claim a wire-level request count. No new high-cardinality metric labels are
   introduced.
6. **Completed work.** Replay of a completed import MUST make no application
   provider dispatch; the kill/restart drill already asserts stub counters
   stay 0→0 after a restart.
7. **Change rule.** Thresholds change only through a reviewed policy change
   approved before the implementation judged against it; a candidate change
   cannot weaken its own gate.

## Acceptance evidence that judges this policy

- The kill/restart drill (`tests/e2e/ingestion_recovery.sh`) forces two
  attempts per novel (one hard kill each at the `chapters` and `enriched`
  boundaries) and its verifier asserts the resulting `attempt <= 3`.
- An integration test seeds a job claimed three times, proves the fourth
  claim marks `budget_exhausted` and no application provider dispatch occurs,
  proves recovery never reclaims it, and proves the retry endpoint returns the
  re-upload guidance without an application provider dispatch.
- A unit test forces a schema retry after the runtime token is exhausted and
  proves the retry is rejected before the LLM port is dispatched.

## Non-goals

- Per-principal and time-window spend ceilings for public profiles (H2/H3).
- Provider-side idempotency keys (unavailable on the configured providers).
- Changing the per-attempt call budget, the avatar cap, or the golden loop's
  two-attempt retry expectations.
