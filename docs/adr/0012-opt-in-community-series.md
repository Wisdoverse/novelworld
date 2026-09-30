# ADR 0012: Opt-in community series suggestions

- Status: Accepted structural preview
- Date: 2026-09-30
- Owners: Novel Service and frontend maintainers
- Related: [Issue #460](https://github.com/Wisdoverse/novelworld/issues/460)

## Context

Readers repeatedly organize the same shared-catalog books. Their confirmed
associations can suggest a group to another reader without sharing a private
series or invoking a model. ADR 0011 keeps definitions, backgrounds, rules,
player state and association authority private; this decision adds only an
explicitly consented aggregation of current membership relationships.

## Decision

Novel owns `world_series_contributions`. Absence means disabled; an owner-only
GET/PUT at `/novels/world-series/{id}/contribution` reads or sets consent.
PUT is idempotent and serialized with series deletion. Opt-in covers current
and future members of that private series. Withdrawal deletes the consent row.
A composite FK to the reader-owned definition removes consent on account
erasure without introducing a new cross-service FK. Account export includes
the acting reader's current consent.

An explicit POST to `/novels/{id}/world-series/community-suggestion` reads
current memberships in a PostgreSQL statement snapshot. It neither writes
associations nor dispatches model work nor persists a recommendation cache.
Each exact pair of canonical novel UUIDs needs at least three distinct opted-in
donor accounts, excluding the requesting account. A donor placing the pair in
different opted-in series makes that pair ambiguous; this is an abstention
signal, not a validated negative fact. Separate uploads or editions are not
merged by title, filename or inferred similarity. Readers reuse a canonical
identity through explicit shared-catalog attachment.

Candidates are restricted to the requesting reader's Ready shelf. An existing
private-series candidate requires direct qualifying evidence between the target
and every current member; missing/not-Ready members disqualify it. There is no
transitive graph inference. Multiple eligible groups or an already-associated
target abstain. The existing 128-other-book ceiling applies; overflow abstains.
The response uses `method: community`, `reason: community_consensus` for a
suggestion, and only recipient-owned candidate metadata. It includes no donor
IDs, counts, names, backgrounds, player state, raw evidence or other books.
`Cache-Control: no-store` and fresh aggregation preserve withdrawal semantics.

Database work has a five-second outer deadline; aggregate and consent-write
transactions additionally use four-second statement and three-second lock
timeouts. There are no automatic retries. Failure leaves manual selection and
existing recognition available. The recipient explicitly confirms any group
and separately confirms a shared background. A group is not proof of common
world facts. Existing frozen sessions remain unchanged.

## Alternatives considered

- Private-only matching preserves the old boundary but cannot reuse consented
  community evidence.
- Public series/backgrounds expose more than is needed for a grouping hint.
- Cached vote graphs, fuzzy identity merging and model training add identity,
  withdrawal and consistency problems without demonstrated need.

## Consequences

Current memberships and consent are authoritative; regrouping, unlinking,
shelf removal and account deletion change recommendations on the next read.
No worker or materialized aggregate needs reconciliation. Direct aggregation
cost grows with contributors; revisit only after measuring query latency.
Three accounts are not three verified humans. Accuracy, resistance to Sybil
accounts, representative quality and scale remain unqualified. Aggregate
thresholds reduce incidental exposure but do not establish anonymization.

## Rollout and rollback

Apply additive migration 0035 before starting the matching Novel/frontend pair;
desktop startup embeds it. Existing readers contribute nothing until opt-in.
An older frontend simply lacks the new controls. To roll back, stop the new
frontend/API paths and retain the additive table; no background, rules or
session binding is changed. Reapplying the migration is safe. No public
deployment or provider execution is authorized by this feature.

## Evidence

- Backend: `services/novel-service/tests/support/world_series_contract.rs` and
  `services/novel-service/tests/support/community_series_contract.rs`.
- Migration: `tests/integration/tests/legacy_migration.rs`.
- HTTP routes: `services/novel-service/tests/community_series_http.rs`; run
  `TEST_DATABASE_URL=<disposable-db> cargo test --locked -p novel-service --test community_series_http -- --ignored`.
  Required integration CI runs it against the migrated disposable database.
- Browser: `frontend/e2e/community-series.spec.ts`.
- Lifecycle: [Data retention](../DATA_RETENTION.md) and
  [account export](../ACCOUNT_EXPORT.md).

Local tests, required CI, deployment and representative recommendation quality
are separate evidence classes; structural preview does not qualify the latter.
