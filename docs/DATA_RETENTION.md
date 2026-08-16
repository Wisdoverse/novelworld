# Data Retention and Erasure

This document describes the current NovelWorld runtime contract. Retention is
event-based unless a duration is stated: data remains while its account or novel
exists, then the owning delete workflow removes it.

## NovelWorld-owned data

| Data | Storage and retention | Erasure |
|---|---|---|
| Uploaded TXT, EPUB, or PDF bytes | With `S3_ENABLED=true`, stored privately under a server-generated key for the lifetime of the novel and read back during import recovery to replay chapter splitting before any provider work. With S3 disabled, held only while the request extracts text. | Novel/account deletion atomically queues the object key in PostgreSQL; novel-service deletes it asynchronously and retries with bounded backoff until S3 acknowledges deletion. |
| Extracted source text and lore chunks | PostgreSQL `chapters` and `chapter_chunks`, for the lifetime of the novel. | `DELETE /api/novels/{id}` or account deletion. |
| Import job metadata | PostgreSQL `novel_import_jobs` stores only stage, status, attempt, lease, and failure code for the lifetime of the novel; it contains no source or model output. | Novel or account deletion cascades the row. It is internal operational state and is excluded from account export. |
| Characters, relationships, and canonical models | PostgreSQL, for the lifetime of the novel. Avatar bytes are not copied into NovelWorld; only provider-returned URL metadata is stored. | Novel or account deletion removes the rows and URL metadata. |
| Chat turns and messages | PostgreSQL, for the lifetime of the novel/account. PostgreSQL is authoritative. | Novel or account deletion. |
| Character memories | PostgreSQL, for the lifetime of the novel/account. | Novel or account deletion. The existing per-character short-memory action clears only the Redis projection, not durable history. |
| Short-memory projection | Redis lists, at most 50 messages per character/user pair. There is no time-based TTL; PostgreSQL remains the source of truth. | Internal cleanup establishes a tombstone before authoritative deletion, then removes matching keys. Account cleanup removes every matching user key; novel cleanup preserves other novels and users. |
| Deletion tombstones | Redis keys containing only a user UUID or user/novel UUID pair, retained for one hour. They contain no source text, message, profile, or model data. | Expire automatically. They prevent an already-committed asynchronous projection from recreating deleted cache data. |
| Choices, world state, narrative nodes, reading progress, player timelines, and world-turn audit/replay records | PostgreSQL, for the lifetime of the novel/account. Failed world turns retain their action and status but not a model transition/result. | Novel or account deletion. `world_turns` cascade through their owning world state. |
| Generated chapter prose | PostgreSQL `player_chapters`, for the lifetime of the novel/account. | Novel or account deletion. |
| User profile and refresh tokens | PostgreSQL, for the account lifetime. A refresh token is atomically replaced after successful refresh and removed by logout or when expired. | Account deletion. The approved [`BACKUP_RESTORE.md`](./BACKUP_RESTORE.md) procedure additionally deletes every refresh token during a restore, so sessions never survive one. |
| Web-managed LLM API key | Encrypted in the singleton PostgreSQL runtime configuration until replaced. | Removed when the final account is deleted. Environment-managed keys remain under operator control outside NovelWorld. |
| Erasure records | PostgreSQL `erasure_records` stores only a subject type, the subject UUID, the owning user UUID, the deletion time, and the re-queue bookkeeping, written by `AFTER DELETE` triggers in the same transaction as the deletion. No content, email, or derived data. Retained indefinitely as deletion-enforcement evidence: they are what stops an older restored backup from resurrecting a deleted subject. | Not erased — a record outlives its subject by design. It is internal operational state and is excluded from account export. |
| Database lineage token | PostgreSQL `database_lineage` holds exactly one row: a version-4 UUID, the UUID of the artifact a restore came from (absent for a genesis database), and a creation timestamp. No account, novel, or content identity. Created once by the migration path and retained for the lifetime of the database; only a restore replaces it. | Not erased — it is what lets a restore tell its own lineage from a sibling restore of the same artifact. Internal operational state, excluded from account export. |
| Restore attestations | PostgreSQL `restore_attestations` stores the attest-or-erase decisions of a disaster restore: subject UUID, decision, both residual-window bounds, the artifact digest inventory, an operator identity string, and a timestamp. Retained indefinitely with the database it describes. | Not erased. Internal operational state, excluded from account export. |

Database foreign keys perform the authoritative cascade. Redis cleanup is a
service-owned operation: user-service and novel-service do not read or delete
agent-service tables directly.

S3 is opt-in and belongs to novel-service. Enabling it requires a pre-created
private bucket. Upload acceptance requires a successful object write and stores
only a server-generated key in PostgreSQL. Disabling S3 while stored objects or
pending deletion records exist is rejected at startup. Original object bytes and
keys are not included in account export; the extracted source chapters remain
portable through `account-export-v1`. The service identity needs `s3:ListBucket`
on that bucket for readiness and `s3:PutObject`/`s3:DeleteObject` only on the
`source-files/*` prefix.

## Delete behavior

- The acting account always comes from the Gateway-validated JWT. A request body
  or path cannot select a different account for deletion.
- Account deletion is available in Settings and as `DELETE /api/auth/me`.
- The only administrator cannot delete their account while other users remain,
  because that would leave the installation without an operator. A sole final
  account can delete itself and return the installation to first-run setup.
- A required Redis cleanup failure returns `503` before authoritative deletion.
  The tombstone atomically rejects delayed cache and derived-memory projections,
  closing the concurrent-chat window without a second scan. Repeating deletion
  is state-idempotent; a deleted account returns `204`, while a deleted novel is
  no longer an owned resource and returns `404`.
- Deleted application data is intentionally unrecoverable. NovelWorld does not
  provide a per-item restore or recycle-bin workflow. Whole-database operator
  recovery is governed separately by the approved
  [`BACKUP_RESTORE.md`](./BACKUP_RESTORE.md) policy, whose erasure-replay
  contract exists precisely so that restoring an older backup cannot bring a
  deleted subject back; its implementation state is tracked in
  [`SPEC_CONFORMANCE.md`](./SPEC_CONFORMANCE.md) (§12.4).
- S3 deletion is eventually complete rather than transactionally coupled to
  PostgreSQL. The durable `source_file_deletions` outbox survives novel and
  account cascades and service restarts; `/ready` fails while an enabled S3
  bucket is unavailable.

## Data outside NovelWorld

Configured model and image providers may receive source excerpts, prompts,
messages, and image-generation descriptions. NovelWorld cannot delete provider
logs or provider-hosted image bytes unless that provider offers and the operator
configures a separate deletion contract. Their retention is governed by the
operator's provider agreement.

Container logs, database snapshots, volume copies, and external backups are also
operator-owned. NovelWorld does not schedule, upload, or prune backups, so
operators must apply their own retention and deletion schedules to those copies,
bounded by the retention ceiling in
[`BACKUP_RESTORE.md`](./BACKUP_RESTORE.md) once that policy's scripted backup is
implemented.

## Account export

Settings and `GET /api/account/export` provide the acting user a versioned
`account-export-v1` NDJSON download. The Gateway composes ordered, service-owned
HTTP fragments with bounded memory; it never reads downstream tables. A final
`complete` record is required before the browser saves the file.

The export contains profile metadata; owned novels, source chapters,
characters, relationships, canon models, and reading progress; durable messages
and memory content; and relevant narrative nodes, choices/transitions, world
state, player chapters, and world-turn actions/transitions/results. It excludes credentials, tokens, runtime model
keys, internal operational state, embeddings, Redis/search projections, and
data held only by providers or operators. Each service uses its own statement
snapshot, so this is a portability export rather than a globally atomic backup.
The complete wire contract and consumer rules are in
[ACCOUNT_EXPORT.md](./ACCOUNT_EXPORT.md).
