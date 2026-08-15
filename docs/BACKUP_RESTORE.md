# Backup and Restore Policy — `backup-restore-v1`

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
under their own custody: the deployment `.env` (at minimum `JWT_SECRET`,
`RUNTIME_CONFIG_KEY`, `INTERNAL_SERVICE_TOKEN`, and S3 credentials when
enabled) and the `BACKUP_ENCRYPTION_KEY`. A fresh host that regenerates
`RUNTIME_CONFIG_KEY` cannot decrypt a web-managed LLM key stored in the
database, and a regenerated `JWT_SECRET` invalidates sessions; the restore
procedure therefore begins by installing the preserved `.env`. Losing the
preserved secrets is a recorded recovery limitation, not a silent failure:
the restore script must state which secrets were not preserved and what
functionality that costs.

## Recovery targets

| Target | Value | Meaning |
|---|---|---|
| RPO | ≤ 24 hours | The operator schedules the scripted backup at least daily. Data committed after the newest verified backup may be lost. The drill validates the mechanism; adherence to the schedule is an operator duty recorded as such, not proven by the drill. |
| RTO | ≤ 30 minutes | Operator-executed scripted restore on the reference envelope (2-core / 4 GB / SSD, `DEPLOY.md` minimum) from a verified backup at supported preview scale (a database within the `DEPLOY.md` 20–50 GB disk sizing), measured from starting the restore procedure to the deployment passing readiness. |
| Drill bound | ≤ 10 minutes | CI assertion for the drill dataset defined under Drills; keeps the drill sensitive to procedure regressions without depending on production data volume. |
| Backup retention ceiling | 30 days default; operator override bounded to 7–90 days | Backups older than the ceiling must be destroyed by the operator. Values outside 7–90 days require `backup-restore-v2`. |

The RPO and RTO values are policy for the private preview profile. Public or
multi-node profiles require a new reviewed version of this document.

## Backup artifact requirements

1. Produced by the scripted procedure using `pg_dump` executed inside the
   pinned PostgreSQL container image, so client and server versions never
   skew.
2. **Embeds the erasure-record export**: every artifact carries a separate
   dump of `erasure_records` as of backup time. The newest artifact is
   thereby a durable, off-database erasure source that survives loss of the
   live database.
3. Compressed, then encrypted at rest with AES-256-CBC using
   PBKDF2 (`openssl enc -aes-256-cbc -pbkdf2` with at least 200 000
   iterations) and an operator-provided `BACKUP_ENCRYPTION_KEY` of at least
   32 characters. An unencrypted backup artifact is non-conformant.
4. Accompanied by a SHA-256 checksum manifest and a timestamp, written at
   backup time.
5. Stored and rotated by the operator within the declared retention ceiling.
   NovelWorld does not schedule, upload, or prune backup artifacts.

## Restore procedure contract

The scripted restore procedure, in order:

1. **Install preserved secrets** (`.env`, `BACKUP_ENCRYPTION_KEY`) per the
   custody prerequisite.
2. **Verify** the artifact against its checksum manifest and refuse to
   proceed on any mismatch or on a missing/invalid encryption key. No data
   is changed before verification passes.
3. **Stop writes**: stop application services (or confirm they are not
   running) before any erasure-record export, so no deletion can commit
   between export and replacement.
4. **Collect erasure sources**: the union of (a) the erasure records
   embedded in the artifact being restored, (b) a fresh export from the
   current database when it is still reachable after writes stopped, and
   (c) the embedded erasure records of every newer artifact the operator
   holds. The newest source's timestamp defines the **residual window**:
   deletions committed after it cannot be replayed.
5. **Load** the decrypted dump into a clean database and re-insert the
   union of erasure sources (idempotent by primary key).
6. **Gate on the residual window**: when the current database was reachable
   in step 4, the residual window is empty and the restore proceeds. In a
   disaster restore it is not: the script must state the window's start
   time and refuse to complete unless the operator explicitly accepts it
   with a recorded flag. The acceptance — window bounds and timestamp — is
   written to the deployment log. Silent resurrection is prohibited;
   explicitly recorded acceptance of a residual window no longer than the
   RPO interval is the documented disaster fallback for this profile. The
   retention ceiling bounds how old a restorable backup — and therefore how
   stale its embedded erasure source — can be.
7. **Deploy normally.** The standard migration path replays idempotent
   erasure before any service starts, so a restored deployment can never
   serve a subject with a preserved erasure record.

## Erasure records and replay

Deletion of a user or a novel writes a durable erasure record in the same
database transaction as the authoritative delete, via `AFTER DELETE`
triggers, so every deletion path — including per-novel records under an
account cascade — is covered without service coordination. A record contains
only the subject type and UUIDs (for novels, the owning user UUID as well,
so the deterministic retained-source object key
`source-files/{user_id}/{novel_id}` can be reconstructed). Records contain
no content, no email, and no derived data; they are excluded from account
export and are retained as deletion-enforcement evidence. UUID v4 identity
guarantees replay can never affect a legitimately re-created account or
re-imported novel.

Erasure replay runs in the standard migration path on every deployment and
MUST be idempotent:

- delete any subject row matching an erasure record (cascades and the
  existing deletion triggers then re-apply downstream cleanup, including
  re-queuing the retained-source object key into `source_file_deletions`);
- re-queue the deterministic retained-source key for a novel erasure record
  whose subject row no longer exists **exactly once per record across all
  deployments**, tracked by durable per-record bookkeeping (the
  self-consuming cleanup outbox is not that bookkeeping), so a restore
  cannot silently resurrect source bytes in S3 and repeated deployments do
  not re-issue provider deletes;
- never produce unbounded per-deployment work: replay against an
  already-clean database is a no-op apart from bounded bookkeeping.

## Drills

Both drills are release evidence for H1 and run in CI against the supported
compose topology. The **drill dataset** is the seeded end-to-end reader
journey: at least one account, one imported novel with at least two durable
chapters and a retained-source key when S3/stub storage is enabled in the
drill topology, committed chat history, and at least one committed world
turn. Drill assertions reference this dataset; they do not claim
production-scale RTO, which remains the operator-procedure target above.

**Drill A — backup → erase → fresh-host restore.** Seed the drill dataset,
take a scripted backup, destroy the deployment including volumes while
preserving `.env` per the custody prerequisite, restore on a clean
deployment, verify sampled authoritative rows survive, and prove the same
journey continues with the existing end-to-end reader loop. The scripted
restore completes within the drill bound.

**Drill B — backup → deletion → older-backup restore.** Take a backup, then
delete **both** drill subjects — one direct novel deletion and one account
deletion whose cascade removes a novel with a retained-source key — then
restore the older backup and deploy with the preserved erasure records. The
deleted subjects must remain unavailable to login, reads, export, provider
work, and derived projections — the zero-tolerance guardrail in
[`QUALIFICATION_POLICY.md`](./QUALIFICATION_POLICY.md). The retained-source
keys are re-queued exactly once, and a second deployment replays cleanly:
no new re-queue, no row changes, no new provider work.

**Disaster gate check.** A restore invoked without any reachable current
database and without the acceptance flag must refuse to complete; with the
flag, it must record the residual window in the deployment log.

Negative cases: a corrupted artifact and a wrong or missing encryption key
must fail closed with actionable errors before any data change.

## Versioning

Changes to targets, artifact requirements, the restore contract, the
retention-ceiling bounds, or drill definitions require a new version of this
document approved in its own reviewed change, as `backup-restore-v2` and
onward.
