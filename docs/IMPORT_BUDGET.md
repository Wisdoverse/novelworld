# NovelWorld Import Provider Budget Policy

Version: **`import-provider-budget-v1`**. This reviewed change approves the
policy; the enforcement that is judged against it lands in a separate change
and may not weaken these thresholds.

This policy bounds one quantity: the provider fan-out of the ingestion
pipeline across the crash window where a provider call's receipt is
unknowable. It does not promise exactly-once provider calls; it bounds the
total spend that an unknown outcome can cause.

## Current truth (baseline)

- Per attempt, `ensure_import_budget` admits at most **640 provider calls**:
  two mandatory calls, the character-extraction scan plan, the canon-extraction
  scan plan, and at most 30 avatar generations.
- Cross-attempt fan-out is currently **unbounded**: the startup/30-second
  recovery scan and the retry endpoint can claim a job any number of times,
  each claim re-issuing up to the full per-attempt budget.
- The configured providers (DeepSeek/OpenAI chat-completions and image
  generation) offer **no request-level idempotency key**, so the
  "bound, meter, and budget" branch of the H1 scope applies; provider-side
  idempotency is recorded as unavailable, not promised.

## Contract — `import-provider-budget-v1`

1. **Attempt ceiling.** A `novel_import_jobs` row MUST NOT be claimed more
   than **3** times. Attempt counting includes the acceptance claim and every
   recovery, lease-expiry, or user-retry claim.
2. **Cross-attempt call ceiling.** Derived from (1) and the per-attempt
   ceiling: at most **3 × 640 = 1920** provider calls per import.
3. **Terminal semantics.** A claim attempt for a job already at the ceiling
   MUST mark the job terminally `failed` with failure code
   `budget_exhausted`, set the Novel to `error` with the actionable public
   message "Import provider budget exhausted; re-upload the source", and the
   job MUST never be reclaimed by the recovery scan or resumed by the retry
   endpoint. Re-uploading creates a new import with a fresh budget.
4. **Metering.** The evidence is the persisted `job.attempt`, the existing
   structured logs (attempt and failure codes), and the
   `llm-observability-v1` metrics. No new high-cardinality metric labels are
   introduced.
5. **Completed work.** Replay of a completed import MUST make no provider
   call; the kill/restart drill already asserts stub counters stay 0→0 after
   a restart.
6. **Change rule.** Thresholds change only through a reviewed policy change
   approved before the implementation judged against it; a candidate change
   cannot weaken its own gate.

## Acceptance evidence that judges this policy

- The kill/restart drill (`tests/e2e/ingestion_recovery.sh`) forces two
  attempts per novel (one hard kill each at the `chapters` and `enriched`
  boundaries) and its verifier asserts the resulting `attempt <= 3`.
- An integration test seeds a job claimed three times, proves the fourth
  claim marks `budget_exhausted` and no provider call occurs, proves recovery
  never reclaims it, and proves the retry endpoint returns the re-upload
  guidance without a provider call.

## Non-goals

- Per-principal and time-window spend ceilings for public profiles (H2/H3).
- Provider-side idempotency keys (unavailable on the configured providers).
- Changing the per-attempt call budget, the avatar cap, or the golden loop's
  two-attempt retry expectations.
