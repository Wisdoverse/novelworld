# ADR 0004: Durable diagnostic budget authority

- Status: Accepted decision; delivery evidence is tracked in issue #320
- Date: 2026-09-07
- Owners: repository maintainers and affected service owners
- Related: [issue #320](https://github.com/Wisdoverse/novelworld/issues/320),
  [approved v5 plan](https://github.com/Wisdoverse/novelworld/issues/320#issuecomment-5567356474),
  [independent approvals](https://github.com/Wisdoverse/novelworld/issues/320#issuecomment-5567386298).
  Plan SHA256: `f61037a6b074c1b42ae86b67d039904a07b6658c598dee74a8001b8517dcc030`.

## Context

The opt-in Vision diagnostic must keep one bounded allowance across the four
paying services, retries, JSON fallback, process restart, and supported artifact
recreation. The existing H1 ledger is process-local and cannot provide that
authority. Normal unbudgeted runtime behavior remains unchanged. This ADR does
not authorize a paid run, public deployment, generic billing, or qualification.

## Decision

User-service owns one PostgreSQL budget aggregate and bounded per-attempt
receipts. These are the only two new relations and are accessed only by the
user-service adapter. The aggregate row is locked while a fresh UUIDv4 attempt
receipt is allocated and charged totals are updated. Every HTTP provider attempt
(including retry and JSON fallback) reserves immediately before dispatch and
settles only once with the complete model/input/output/cached-input tuple.

Provisioning is an explicit one-shot command against a fresh isolated database;
it is not an upsert, startup repair, request-time creation, top-up, reset, or
reopen endpoint. Ordinary startup only reads and verifies the immutable identity,
limits, and expiry. Missing, expired, sealed, unavailable, unknown, or lost
authority fails closed for new reservations. A committed grant remains charged
and may complete within the existing logical-call deadline; unknown provider
outcomes retain their reservation unless valid settlement already committed.
Settlement may finish after seal/expiry but
never reopens the budget.

A lost reserve acknowledgement means no provider dispatch and no assumed
refund or grant reuse. A lost settlement acknowledgement likewise fails closed:
the client does not reuse the grant or assume a refund, while the authoritative
receipt remains the source of truth for a later bounded reconciliation.

The wire contract is `llm-diagnostic-budget-v1`; budgeted runtime configuration
uses contract 3 with the exact budget ID, wire version, and compiled profile
digest. Ordinary configuration remains contract 2 and is not budgeted. The
fixed profile is Vision-only, DeepSeek-only, non-thinking, and uses the reviewed
official HTTPS origin. The request is bounded before reservation/network I/O to
64 messages and 262,144 serialized JSON bytes; successful control bodies are
bounded to 4 KiB. Provider input/output ceilings and integer worst-case pricing
are registered values, not invoice attribution.

Budget control uses authenticated internal HTTP under
`/internal/llm-budget/{budget_id}` for read, reserve, settle, and seal. It has no
public gateway route and exposes no prompts, source text, credentials, provider
response IDs, reader IDs, or novel IDs. Control calls have their own bounded
deadline and no automatic transport retry. Image and embedding paths deny
dispatch in this opt-in profile until separately priced; ordinary behavior is
unchanged. A shared trusted internal boundary is required across Settings,
sync chat, stream setup, fallback, and all four paying services.

## Alternatives considered

- Per-process allowances or disabling automatic restart cannot preserve one
  aggregate allowance across service restarts and recreations.
- A locked shared file adds crash, torn-write, locking, and portability
  contracts outside the existing HTTP ownership model.
- A new runtime service, Redis quota, proxy, public billing API, or generic
  quota framework is out of scope. Existing user-service ownership is the
  smallest durable authority boundary.

## Consequences

The budget survives process restart and supported artifact recreation, and
receipts provide bounded, idempotent settlement without refund reuse. This adds
an availability dependency on user-service/PostgreSQL for budgeted calls and
one reserve plus at most one settle control call per provider attempt. Receipts
are retained in the isolated database; there is no deletion/refill worker.

Database rewind/restore is outside this contract: an active diagnostic must be
sealed/stopped and cannot restore-and-resume with the same registration. Old
artifacts or mismatched capabilities/configuration are rejected on the supported
preflight path. Hostile binaries, manually started historical images and operator
removal of the registered control are outside the trusted-runtime guarantee.
An unsupported old binary cannot be patched or merely retagged and presented
as evidence of a capable upgrade artifact.

## Rollout and rollback

The binding is disabled by default and additive. The supported release tool and
journey preflight must probe every digest-pinned candidate image with network,
secrets, and data mounts disabled before migration or application startup, and
must verify the exact capability/profile digest for both artifact sets when an
upgrade is involved. A stopped/recreated compatible container is part of the
planned local proof. Any missing or mismatched capability freezes the attempt;
it does not fall back to ordinary mode or create a new budget.

Rollback retains live budget tables and authority. Before disabling the binding
or reverting below a capable artifact, the isolated attempt must be sealed/stopped.
No destructive production-data operation or paid invocation is authorized by
this ADR.

### Isolated release wiring (implementation under verification)

`LLM_DIAGNOSTIC_BUDGET_ID` and owner-only `LLM_DIAGNOSTIC_BUDGET_LIMITS` are
empty by default. The latter is strict JSON containing exactly `profile`,
`max_attempts`, `max_tokens`, `max_cost_micro_cny`, and `expires_at`. Use canonical
unquoted `KEY=value` environment-file entries, without shell expansion. The
profile is `vision-journey-diagnostic-v1`; expiry is fixed UTC seconds, not a
duration renewed on restart. Limits have no funded defaults. Individual local
processes also require explicit `USER_SERVICE_URL` and a valid shared internal
token; missing control URL never silently falls back to a loopback listener.

All four paying binaries expose exact `--diagnostic-budget-contract` before
dotenv, logging, database, listener, or provider setup. It emits only the wire
contract, profile name, and compiled profile digest. The release adapter checks
the resolved Compose configuration and executes `/app/service` from each pinned
image with that flag, no network, no injected environment or data mounts, and
bounded output/time. It captures its helper/profile before checkout. The journey
keeps the same tool files outside its changing checkout and calls the shared
`release.sh preflight MANIFEST` before direct startup/recreation; stop/cleanup
remain available even when preflight fails.

Capability creation and attached execution share a ten-second deadline. The
adapter requires the acknowledged container ID before starting the diagnostic
process, then always removes the exact name and independently verifies absence.
A lost create acknowledgement or failed cleanup is an operational failure, not
a passing incompatible-image check. Fixed phase codes may identify a failure;
they never contain image references, configuration or child output.

Only isolated cold adoption, after fresh migration, may invoke provisioning.
`diagnostic-provisioning.json` is created exclusively and file/directory-fsynced
before that command. Success atomically records completion; a failed command,
lost acknowledgement, missing marker, or incomplete marker freezes continuation.
No release recovery path deletes/retries this marker or recreates missing budget
rows. Upgrade/rollback require its exact completed registration and capable
artifacts. Scripted database restore and release restore refuse diagnostic mode.
These guards do not constitute evidence of a successful real artifact upgrade.

## Evidence

Implementation is in progress; the integrated dispatch control is not enabled
or qualified by this partial evidence. The owner now implements
`diagnostic_llm_budgets` / `diagnostic_llm_attempts`, the explicit
`user-service --provision-diagnostic-budget` command, and ordinary startup's
read-only exact-registration check. Owner configuration rejects malformed,
non-Unicode, unknown/duplicate JSON fields and noncanonical IDs; expiry uses
second-precision UTC `YYYY-MM-DDTHH:MM:SSZ`. Every durable entrypoint verifies
the compiled profile digest in infrastructure, without domain hashing adapters.

Local real-PostgreSQL tests cover concurrent bounded allocation, duplicate
grant rejection, full-tuple settlement/refund, attempt retention, seal/expiry,
missing/mismatched registrations, connection reconstruction and lock timeout.
The migration suite compares fresh-init columns, defaults, nullability and
constraints against two replays of migration 0026, including PostgreSQL 18's
NOT NULL constraints. These are owner-layer checks, not wire-loss or
whole-journey proofs. No provider is called by these tests.

The authenticated owner HTTP router now has a real-PostgreSQL test for strict
wire/scope/authentication, duplicate reservation, seal, late exact settlement,
conflicting settlement and bounded aggregate snapshots. Shared-client tests use
a test-only local provider endpoint to exercise sync/stream/retry/fallback,
request caps, lost control responses, deadline phases, missing usage, settlement
before Finished, and drop behavior. They separately cover ordinary/budgeted
runtime-config compatibility and image/embedding refusal. A subprocess test
proves missing control URL cannot reserve or dispatch. Provider-attempt/parsed
usage metrics survive a lost settlement ACK; an in-flight provider timeout is
also counted exactly once, while control-only failure counts no provider attempt.
Logical completion remains an evidence error. Those unit mocks are separate
from the combined process-level proof below.

The offline `diagnostic_budget_lifecycle.py` fixture now passes with actual
user-service Settings, PostgreSQL, and a normal Rust example linked to the
production shared client (no `cfg(test)` endpoint or shortened test deadline).
It uses only synthetic credentials and a locally trusted TLS mock accepting
the fixed official hostname, on a unique Docker internal network with no
published ports or external forwarding. Zero budget opens no provider
connection. Direct/static/runtime sync, runtime SSE, retry and JSON fallback
match exact reservation, provider and settlement counts. Raw PostgreSQL
receipts verify each reservation and complete settlement tuple. The control
proxy drops reserve/settle acknowledgements only after receiving the real
owner response; missing usage and dropped streams retain their reservations,
and failed SSE evidence never produces Finished. Recreating the owner with the
same database retains charges; both omitted and deliberately deleted empty
registrations refuse ordinary startup without row recreation. This tests owner
recreation, not a mid-journey release upgrade. Exact isolated resources are
removed after execution; mocked cleanup tests also cover Docker timeout and
continued cleanup. Independent review approved this fixture slice after adding
reserve RPC counts and cancellation/partial-failure cleanup. The backend CI
workflow now includes this gate; local passage is not a claim that CI has run.

Capability output was checked on all four actual local binaries with an empty
environment. Release/runner wiring passed bounded independent review, real
Compose syntax checks and real script/Git/marker state-machine checks with mocked
Docker. These prove ordering and refusal, not an actual image upgrade. The
offline fixture additionally packages each of the four normal service binaries
into its own test image, obtains real repository digests through a temporary
loopback registry, and invokes the production release preflight unchanged.
All four capable images pass; replacing each service position with a real
non-capable image, or changing the expected profile digest, fails. These checks
precede database startup, migration and provisioning. Subsequent owner
recreation runs the digest image without a binary mount. This image-packaged
local run passes, but is not evidence from the production release Dockerfile.

The same fixture's existing-image modes freeze source image IDs before creating
test-local registry references and verify that cleanup preserves them.
`--capability-images` performs only capability checks. `--runtime-images`, also
used by Production Compose Smoke, repeats the complete matrix on images built
with the actual `infra/docker/Dockerfile.rust-service` release profile.

The full release-built local matrix passed on source `b7eab0b`, including real
`release.sh` rejection for invalid candidate, current and previous artifacts,
unchanged running containers/ledger/receipts/provisioning marker, safe preflight
reentry, and exact cleanup with all four source image references preserved.
Its Git/state fixture is explicitly negative metadata, not an actual adoption
or two successful product versions. Failed preflight retains the attempted
checkout; reentry does not automatically restore Git HEAD. Earlier failed runs
and their corrections are retained in #320, not counted as passing evidence.

Final Rust review also found malformed successful HTTP usage could enter the
generic retry path. Budgeted sync now treats invalid completion/usage as a fixed
evidence failure before retry or JSON fallback, retaining the reservation.
Six malformed/contradictory usage cases plus an ordinary-mode retry comparison
pass. Budgeted streaming likewise returns fixed evidence errors for malformed
usage, model drift, duplicate usage, upstream failure and premature end, without
settlement or Finished; ordinary error behavior is preserved. The passing
release-built fixture includes corresponding sync/empty/SSE malformed-usage
cases, each with exactly one provider attempt and no settlement or retry.

A second successful budget-capable source version is not
manufactured for this first protocol delivery: actual different-version
successful upgrade journeys remain owned by #230. Local image/test evidence
does not replace required CI, final independent review or exact-main delivery
evidence; #320 owns their current status. No funded Diagnostic or H1/H3/H4
qualification is authorized by this structural control.
