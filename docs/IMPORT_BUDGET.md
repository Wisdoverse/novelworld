# NovelWorld Import Provider Budget Policy

Version: **`import-provider-budget-v1`**. The import claim path enforces this
policy; changes to its thresholds and enforcement must land together.

This policy bounds one quantity: the provider fan-out of the ingestion
pipeline across the crash window where a provider call's receipt is
unknowable. It does not promise exactly-once provider calls; it bounds the
total spend that an unknown outcome can cause.

## Contract — `import-provider-budget-v1`

1. **Per-attempt ceiling.** `ensure_import_budget` admits at most **640
   provider calls**: three fixed-call slots (representative character
   extraction, narrative-node detection, and the bounded whole-novel event
   selection), the character- and canon-extraction scan plans, and at most 30
   avatar generations. The selector reuses `canon_extraction` telemetry and is
   skipped without a call when its complete candidate prompt exceeds 16 KiB.
2. **Attempt ceiling.** A `novel_import_jobs` row MUST NOT be claimed more
   than **3** times. Attempt counting includes the acceptance claim and every
   recovery, lease-expiry, or user-retry claim.
3. **Cross-attempt call ceiling.** Derived from (1) and (2): at most
   **3 × 640 = 1920** provider calls per import.
4. **Terminal semantics.** A claim attempt for a job already at the ceiling
   MUST mark the job terminally `failed` with failure code
   `budget_exhausted`, set the Novel to `error` with the actionable public
   message "Import provider budget exhausted; re-upload the source", and the
   job MUST never be reclaimed by the recovery scan or resumed by the retry
   endpoint. Re-uploading creates a new import with a fresh budget.
5. **Metering.** The evidence is the persisted `job.attempt`, the existing
   structured logs (attempt and failure codes), and the
   `llm-observability-v1` metrics. No new high-cardinality metric labels are
   introduced.
6. **Completed work.** Replay of a completed import MUST make no provider
   call; the kill/restart drill already asserts stub counters stay 0→0 after
   a restart.
7. **Change rule.** Thresholds change only through a reviewed policy change
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
