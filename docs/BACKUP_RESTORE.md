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
  post-restore consistency is governed by erasure replay (below); orphaned
  objects created inside the pre-commit upload guard window remain a
  documented bounded limitation.
- Out of scope: WAL archiving, point-in-time recovery, multi-node or
  multi-region recovery, and backup scheduling infrastructure. Scheduling and
  off-host copy custody are operator duties.

## Recovery targets

| Target | Value | Meaning |
|---|---|---|
| RPO | ≤ 24 hours | The operator schedules the scripted backup at least daily. Data committed after the newest verified backup may be lost. |
| RTO | ≤ 30 minutes | Operator-executed scripted restore on the reference envelope (2-core / 4 GB / SSD, `DEPLOY.md` minimum) from a verified backup at the supported preview scale, measured from starting the restore procedure to the deployment passing readiness. |
| Drill bound | ≤ 10 minutes | CI drill assertion for the drill dataset; keeps the drill sensitive to procedure regressions without depending on production data volume. |
| Backup retention ceiling | 30 days by default; operator-declared override | Backups older than the ceiling must be destroyed by the operator. The ceiling bounds the resurrection window for disaster restores that predate every preserved erasure record. |

The RPO and RTO values are policy for the private preview profile. Public or
multi-node profiles require a new reviewed version of this document.

## Backup artifact requirements

1. Produced by the scripted procedure using `pg_dump` executed inside the
   pinned PostgreSQL container image, so client and server versions never
   skew.
2. Compressed, then encrypted at rest with AES-256-CBC using
   PBKDF2 (`openssl enc -aes-256-cbc -pbkdf2` with at least 200 000
   iterations) and an operator-provided `BACKUP_ENCRYPTION_KEY` of at least
   32 characters. An unencrypted backup artifact is non-conformant.
3. Accompanied by a SHA-256 checksum manifest and a timestamp, written at
   backup time.
4. Stored and rotated by the operator within the declared retention ceiling.
   NovelWorld does not schedule, upload, or prune backup artifacts.

## Restore procedure contract

The scripted restore procedure, in order:

1. **Verify** the artifact against its checksum manifest and refuse to
   proceed on any mismatch or on a missing/invalid encryption key. No data
   is changed before verification passes.
2. **Preserve erasure records**: if a current database is reachable, export
   its `erasure_records` before replacing anything, and re-insert them after
   the restore (idempotent by primary key). In a disaster restore where no
   current database exists, preserved erasure records may be unavailable;
   the retention ceiling then bounds the resurrection window.
3. **Load** the decrypted dump into a clean database.
4. **Deploy normally.** The standard migration path replays idempotent
   erasure before any service starts, so a restored deployment can never
   serve a deleted subject (see below).

## Erasure records and replay

Deletion of a user or a novel writes a durable erasure record in the same
database transaction as the authoritative delete, via `AFTER DELETE`
triggers, so every deletion path — including account cascades — is covered
without service coordination. A record contains only the subject type and
UUIDs (for novels, the owning user UUID as well, so the deterministic
retained-source object key `source-files/{user_id}/{novel_id}` can be
reconstructed). Records contain no content, no email, and no derived data;
they are excluded from account export and are retained as deletion-
enforcement evidence. UUID v4 identity guarantees replay can never affect a
legitimately re-created account or re-imported novel.

Erasure replay runs in the standard migration path on every deployment and
MUST be idempotent:

- delete any subject row matching an erasure record (cascades and the
  existing deletion triggers then re-apply downstream cleanup, including
  re-queuing the retained-source object key into `source_file_deletions`);
- re-queue the deterministic retained-source key for novel erasure records
  whose subject row no longer exists, at most once per record, so a restore
  cannot silently resurrect source bytes in S3;
- never produce unbounded per-deployment work: replay against an
  already-clean database is a no-op apart from bounded bookkeeping.

## Drills

Both drills are release evidence for H1 and run in CI against the supported
compose topology.

**Drill A — backup → erase → fresh-host restore.** Seed a journey, take a
scripted backup, destroy the deployment including volumes, restore on a
clean deployment, verify sampled authoritative rows survive, and prove the
same journey continues with the existing end-to-end reader loop. The
scripted restore completes within the drill bound.

**Drill B — backup → deletion → older-backup restore.** Take a backup, then
delete a subject (account or novel), then restore the older backup and
deploy normally with preserved erasure records. The deleted subject must
remain unavailable to login, reads, export, provider work, and derived
projections — the zero-tolerance guardrail in
[`QUALIFICATION_POLICY.md`](./QUALIFICATION_POLICY.md) — and the
retained-source key is re-queued for deletion exactly once.

Negative cases: a corrupted artifact and a wrong or missing encryption key
must fail closed with actionable errors before any data change.

## Versioning

Changes to targets, artifact requirements, the restore contract, or drill
definitions require a new version of this document approved in its own
reviewed change, as `backup-restore-v2` and onward.
