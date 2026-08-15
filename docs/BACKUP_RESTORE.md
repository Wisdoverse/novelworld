# Backup and Restore Policy — `backup-restore-v2`

Changes from `backup-restore-v1`, each decided here ahead of the
implementation judged against them:

1. **Lineage identity is explicit.** Every database carries a
   create-once, migration-created version-4 lineage token; artifacts
   embed it with manifest-to-dump binding, and live continuation is
   established only by token equality — never inferred from row counts
   or UUID overlap, which cannot distinguish sibling restores of one
   artifact. A restore regenerates the token atomically with the data
   becoming reachable and records its parent; token-less legacy
   artifacts restore through the disaster gate.
2. **Erasure-record payload scoping.** The identifying payload stays
   limited to subject type and UUIDs; operational bookkeeping fields
   (deletion time, retained-source marker, re-queue stamp) are named and
   remain content-free.
3. **Collected records pre-decide.** Accounts already covered by a
   collected erasure record are enforced by replay and need no operator
   decision; the restore writes an automatic `replayed` attestation row
   for each, preserving restore-level audit provenance.
4. **Bookkeeping through restore.** Re-queue stamps restored from the
   artifact's own dump are retained (the same-snapshot dump preserves
   stamp/outbox consistency, so zero repeats are correct); records from
   foreign sidecar sources enter unstamped and re-queue once.
5. **Monotone marker merge.** Conflict detection aborts on disagreeing
   deletion facts; the retained-source marker merges upward as
   bookkeeping.
6. **Verified inventory.** The attestation inventory lists only verified
   digests, including the primary artifact's erasure export.

None of these weakens a recovery target, a drill bound, or the
resurrection guardrail; 1 and 6 strengthen v1, and 2–5 replace
inference-hostile v1 wording with the decided contract. Two consequences
are declared rather than hidden: a crash during a routine restore
escalates the retry to disaster gating with no RTO target, and a
token-less legacy artifact's restore cannot collect the reachable
database's export, leaving post-artifact deletions to the disaster
gate's compensating controls (undecided-erase and owner confirmation).

Status: **approved policy; implementation pending.** This document is the
versioned recovery policy required by H1 in [`ROADMAP.md`](./ROADMAP.md). It
defines the targets, procedures, and drills that the implementation change is
judged against. It is not evidence that any target is currently met; the
conformance state of each normative clause lives in
[`SPEC_CONFORMANCE.md`](./SPEC_CONFORMANCE.md), and
[`DATA_RETENTION.md`](./DATA_RETENTION.md) continues to describe current
runtime behavior. Per the threshold rule in
[`PRODUCT_CONTRACT.md`](./PRODUCT_CONTRACT.md), this policy is approved in its
own reviewed change before the drills it judges, and the judged change cannot
weaken it.

## Scope

- Deployment profile: the supported single-node private preview
  (Docker Compose, operator-controlled host).
- Authoritative store: the single PostgreSQL database. It is the only store
  this policy backs up.
- Redis is a rebuildable projection with a documented loss contract and is
  never backed up or restored.
- Opt-in S3 source objects are not part of the backup artifact. Their
  post-restore consistency is governed by erasure replay (below). An object
  orphaned inside the pre-commit upload guard window has a bounded
  occurrence window but unbounded retention until manual operator cleanup;
  bucket-listing reconciliation remains a non-goal of this version.
- Out of scope: WAL archiving, point-in-time recovery, multi-node or
  multi-region recovery, and backup scheduling infrastructure. Scheduling and
  off-host copy custody are operator duties.

## Secret custody prerequisite

The backup artifact contains no secrets. Restoring a working deployment
additionally requires the operator to preserve, outside the artifact and
under their own custody: the deployment `.env` (at minimum
`RUNTIME_CONFIG_KEY`, `INTERNAL_SERVICE_TOKEN`, and S3 credentials when
enabled) and the `BACKUP_ENCRYPTION_KEY`. A fresh host that regenerates
`RUNTIME_CONFIG_KEY` cannot decrypt a web-managed LLM key stored in the
database; the restore procedure therefore begins by installing the
preserved `.env`. `JWT_SECRET` is deliberately excluded: the restore
procedure always rotates it and deletes every persisted refresh token
before services start, so no session issued before the restore survives
one — a stale access token fails gateway signature validation and a
restored refresh token no longer exists — with no new runtime
enforcement point. Losing the
preserved secrets is a recorded recovery limitation, not a silent failure:
the restore script must state which secrets were not preserved and what
functionality that costs.

## Recovery targets

| Target | Value | Evidence that judges it |
|---|---|---|
| RPO | ≤ 24 hours | The operator schedules the scripted backup at least daily. Data committed after the newest verified backup may be lost. Drill A proves the mechanism; schedule adherence is an operator duty recorded as such, never proven by a drill. |
| RTO | ≤ 30 minutes | Applies to restores with an empty residual window. Judged by the **scale rehearsal**: a scripted restore of a synthetic database whose plain dump is at least 5 GB completes within 30 minutes on the reference envelope (2-core / 4 GB / SSD, `DEPLOY.md` minimum), measured from starting the restore procedure to the deployment passing readiness. Recorded once per policy version and re-run after any change to the backup or restore scripts. A disaster restore's duration is dominated by owner attest-or-erase decisions and carries no RTO target in this version; a crash during a routine restore escalates the retry to disaster gating, and that retry likewise carries no RTO target. |
| Drill bound | ≤ 10 minutes | CI assertion for the drill dataset defined under Drills; keeps every CI run sensitive to procedure regressions without depending on data volume. |
| Backup retention ceiling | 30 days default; operator override bounded to 7–90 days | Backups older than the ceiling must be destroyed by the operator. Values outside 7–90 days require `backup-restore-v3`. |

The RPO and RTO values are policy for the private preview profile. Public or
multi-node profiles require a new reviewed version of this document.

## Backup artifact requirements

1. Produced by the scripted procedure using `pg_dump` executed inside the
   pinned PostgreSQL container image, so client and server versions never
   skew.
2. **Embeds the erasure-record export taken from the same database snapshot
   as the dump itself** (the same `pg_dump` invocation or an export inside
   the same `REPEATABLE READ` snapshot), so the export's coverage and the
   dump's contents cannot diverge. The artifact's **covered-through
   timestamp is the snapshot time**, not the archive-write time, and is
   recorded in the manifest together with the **database lineage token**.
   The newest artifact is thereby a durable, off-database erasure source
   that survives loss of the live database.
3. Compressed, then encrypted at rest with AES-256-CBC using
   PBKDF2 (`openssl enc -aes-256-cbc -pbkdf2` with at least 200 000
   iterations) and an operator-provided `BACKUP_ENCRYPTION_KEY` of at least
   32 characters. An unencrypted backup artifact is non-conformant.
4. Accompanied by a SHA-256 checksum manifest and the covered-through
   timestamp, written at backup time.
5. Stored and rotated by the operator within the declared retention ceiling.
   NovelWorld does not schedule, upload, or prune backup artifacts.

## Restore procedure contract

The scripted restore procedure, in order:

1. **Install preserved secrets** (`.env`, `BACKUP_ENCRYPTION_KEY`) per the
   custody prerequisite.
2. **Verify** the artifact against its checksum manifest and refuse to
   proceed on any mismatch or on a missing/invalid encryption key, and
   compare the manifest's lineage token with the token inside the
   verified dump — a mismatch between present tokens or asymmetric
   absence aborts here. No data is changed before verification passes.
3. **Stop writes**: stop application services (or confirm they are not
   running) before any erasure-record export, so no deletion can commit
   between export and replacement.
4. **Collect erasure sources**: the union of (a) the erasure records
   embedded in the artifact being restored, (b) a fresh export from the
   current database when it is reachable after writes stopped **and its
   lineage token equals the artifact's** — token equality is the only
   evidence of continuation; row counts or shared UUIDs prove nothing,
   since sibling restores of one artifact share UUIDs by construction —
   and (c) the embedded erasure records of every newer artifact the
   operator holds. Records are immutable facts keyed by subject; if two
   sources disagree on any deletion fact of the same key (subject type,
   subject, owning user, deletion time), the restore aborts with an
   actionable error rather than guessing; the retained-source marker is
   monotone bookkeeping and merges upward. The newest source's
   covered-through timestamp defines the start of the **residual window**;
   its end is the moment writes stopped (or the declared failure time for a
   lost database). Deletions committed inside the window cannot be
   replayed.
5. **Load** the decrypted dump into a clean database and, atomically
   with the load becoming reachable, **regenerate the lineage token**
   (fresh version-4 UUID, artifact token recorded as parent); then
   re-insert the union of erasure sources (idempotent for identical
   rows), **rotate `JWT_SECRET`, and delete every persisted refresh
   token**, so no session issued before the restore survives it and no
   crashed attempt can present the artifact's token as live. Rotation
   and token deletion happen only after verification passes, as part of
   preparing the new deployment.
6. **Gate on the residual window.** When the current database was reachable
   in step 4 with a matching lineage token, the residual window is empty
   and the restore proceeds. When
   it is not — a disaster restore — the script **refuses to complete by
   default**. The only sanctioned continuation is **attest-or-erase**: the
   script lists every account in the restored data with its novels, and
   for each account the operator, confirming with that account's owner (or
   as the owner, for their own account), supplies one decision — retain
   the account together with the explicit list of its retained novels, or
   erase it. An account already covered by a collected erasure record is
   a pre-decided fact: replay enforces it, and no decision may retain or
   designate it. The restore records an automatic attestation row for it
   with decision `replayed`, carrying the same window bounds, inventory,
   operator identity, and timestamp, so restore-level audit provenance is
   preserved without operator burden — the attestation rows, not the
   parent marker, are what distinguish a token-less restore from a
   genesis database. A decision naming such an account is rejected, and
   the drill asserts the rejection alongside the row. The script then, before any service
   starts,
   writes erasure records for every erase-decided account and for every
   novel not on a retained account's retained list, and replays them, so
   **no subject is ever served ahead of its decision and nothing deleted
   is served at all**. The restore does not complete while any account
   outside the collected records lacks a decision. Each decision is recorded durably in the restored database —
   subject identity, decision, residual-window bounds (covered-through
   start and writes-stopped or declared-failure end), the artifact digest
   inventory used as erasure sources, an operator-supplied identity
   string, and a timestamp. This requires no runtime enforcement point:
   an unretained subject does not exist in the served deployment, stale
   access tokens fail gateway signature validation after rotation, and
   restored refresh tokens were deleted before services started. The procedure's guarantee is conditional on
   truthful decisions, as any recovery procedure is conditional on its
   inputs; a factually false retention is undetectable by construction
   and equivalent to restoring a hand-edited dump. Silent resurrection is
   prohibited in every case; there is no continuation that skips
   attest-or-erase.
7. **Deploy normally.** The standard migration path replays idempotent
   erasure before any service starts, so a restored deployment can never
   serve a subject covered by any collected erasure record.

## Lineage identity

Every database carries exactly one lineage token: a version-4 UUID created
by the standard migration path only when no token exists. Ordinary
deployments and migration replays preserve it unchanged; only a restore
regenerates it. The backup script records the token in the manifest from the same
snapshot as the dump, and
the restore compares the manifest token with the token inside the
verified dump: a mismatch between two present tokens, or one present
without the other, is tampering and aborts. A wholly token-less artifact — one produced by the scripted backup
before the lineage token existed, a set that is empty unless the script
ships ahead of the token migration — restores through the disaster gate,
because an absent token never establishes continuation; pre-policy raw
dumps without manifests remain non-conformant and fail verification
regardless. Live
continuation is established only by the reachable database's token
equalling the artifact's. The restore regenerates the token as a fresh
version-4 UUID atomically with making the restored data reachable,
recording the artifact's token as the new token's parent (recorded
absent for a token-less artifact) in the same table: a crashed or
partial restore must never present the artifact's token as a live
lineage, so a retry after any failure never faces a weaker gate than
the first attempt did. A database instantiated from a storage copy by
any means other than the scripted restore is outside this policy's
continuation contract.

## Erasure records and replay

Deletion of a user or a novel writes a durable erasure record in the same
database transaction as the authoritative delete, via `AFTER DELETE`
triggers, so every deletion path — including per-novel records under an
account cascade — is covered without service coordination. A record's
identifying payload is limited to the subject type and UUIDs (for novels,
the owning user UUID as well, so the deterministic retained-source object
key `source-files/{user_id}/{novel_id}` can be reconstructed); its
operational bookkeeping fields — deletion time, retained-source marker,
re-queue stamp — contain no content, no email, and no derived data.
Records are excluded from account export and are retained as
deletion-enforcement evidence. UUID v4 identity
guarantees replay can never affect a legitimately re-created account or
re-imported novel.

Erasure replay runs in the standard migration path on every deployment and
MUST be idempotent:

- delete any subject row matching an erasure record (cascades and the
  existing deletion triggers then re-apply downstream cleanup, including
  re-queuing the retained-source object key into `source_file_deletions`);
- re-queue the deterministic retained-source key for a novel erasure record
  whose subject row no longer exists **exactly once per record within a
  database lineage**, tracked by durable per-record bookkeeping (the
  self-consuming cleanup outbox is not that bookkeeping). A restore starts
  a new lineage with a fresh token and a recorded parent. Bookkeeping
  restored from the artifact's own dump is retained: the same-snapshot
  dump preserves stamp/outbox consistency, so zero repeats are correct.
  A record arriving from a foreign sidecar source enters unstamped and
  re-queues once; S3 object deletion is idempotent, so that bounded
  repeat is safe;
- never produce unbounded per-deployment work: replay against an
  already-clean database is a no-op apart from bounded bookkeeping;
- preserve the deletion-path invariants the interactive flow enforces:
  when replay or attest-or-erase removes the final account, the runtime
  configuration is cleared and the installation returns to first-run
  setup; when decisions would leave retained accounts without any
  administrator, the script requires the operator to designate one
  retained account as administrator before completion, recorded with
  the decisions, so the installation is never left without an
  operator.

## Drills

All three drills are release evidence for H1 and run in CI against the
supported compose topology. The **drill dataset** is the seeded end-to-end reader
journey extended to cover both deletion paths: at least three accounts
(the third exists to be covered by a collected erasure record from a
newer artifact during the disaster drill) and
three imported novels (each novel with at least two durable chapters), at
least two novels carrying retained-source keys when the drill topology
enables S3/stub storage, committed chat history, and at least one committed
world turn. One novel is deleted directly; one account owning a
retained-source novel is deleted entirely. Drill assertions reference this
dataset; production-scale RTO is judged by the scale rehearsal defined
under Recovery targets, not by these drills.

**Drill A — backup → erase → fresh-host restore.** Seed the drill dataset,
take a scripted backup, destroy the deployment including volumes while
preserving `.env` per the custody prerequisite, restore on a clean
deployment, verify sampled authoritative rows survive, and prove the same
journey continues with the existing end-to-end reader loop. The scripted
restore completes within the drill bound.

**Drill B — backup → deletion → older-backup restore.** Take a backup, then
delete both drill subjects — the direct novel deletion and the account
deletion whose cascade removes a novel with a retained-source key — then
restore the older backup and deploy with the preserved erasure records. The
deleted subjects must remain unavailable to login, reads, export, provider
work, and derived projections — the zero-tolerance guardrail in
[`QUALIFICATION_POLICY.md`](./QUALIFICATION_POLICY.md). The retained-source
keys are re-queued exactly once, and a second deployment replays cleanly:
no new re-queue, no row changes, no new provider work.

**Drill C — disaster gate.** Invoke the restore with no lineage-matching
reachable database and a non-empty residual window: the script must refuse
to complete, including when decisions cover only some undecided accounts
and when the reachable database is unrelated or a sibling lineage
(populated, even carrying its own deletion history, but with a different
lineage token). Re-run with a
complete attest-or-erase input that retains one account (with a partial
novel list) and erases the other: the restore completes; the decision
rows exist in the restored database with every required field — subject,
decision, both residual-window bounds, the verified artifact digest
inventory (dump and erasure digests), operator identity, and timestamp; the erased account and the unlisted
novel are absent from the served deployment with erasure records written
and their dependent rows (refresh tokens, world state, chat) removed by
cascade; a JWT issued before the restore is rejected after the rotation and no
pre-restore refresh token remains;
and the window bounds are computed from covered-through timestamps, not
wall-clock archive times. The drill also proves the token lifecycle:
ordinary migration replay preserves the live token; two restores of one
token-bearing artifact produce distinct tokens, each recording the
artifact's token as parent; a manifest whose token disagrees with its
dump, or where exactly one of the pair is absent, is refused; a wholly
token-less artifact restores through the disaster gate with an
absent-parent token recorded; a retry after a failure injected after the dump load and before the
atomic commit that makes the restored data reachable with its
regenerated token — and again after that commit — still faces the gate
rather than appearing lineage-matching;
and, after deleting the third account post-artifact and taking a newer
artifact held as an erasure source, the collected record covering that
account produces an automatic `replayed` attestation row carrying the
full field set, with no operator decision accepted for it.

Negative cases: a corrupted artifact and a wrong or missing encryption key
must fail closed with actionable errors before any data change; erasure
sources that disagree on a deletion fact of the same subject abort the
restore.

## Versioning

Changes to targets, artifact requirements, the restore contract, the
retention-ceiling bounds, or drill definitions require a new version of this
document approved in its own reviewed change, as `backup-restore-v3` and
onward. Possible future hardening, deliberately out of this version: an off-database
deletion-receipt append log that narrows the disaster residual window
toward zero.
