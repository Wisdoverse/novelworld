# Data Retention and Erasure

This document describes the current NovelWorld runtime contract. Retention is
event-based unless a duration is stated: data remains while its account or novel
exists, then the owning delete workflow removes it.

## NovelWorld-owned data

| Data | Storage and retention | Erasure |
|---|---|---|
| Uploaded TXT, EPUB, or PDF bytes | Held only while the upload request extracts text. NovelWorld does not persist the original file or write `original_file_key` in the current runtime. | Released after request processing; there is no stored upload object to delete. |
| Extracted source text and lore chunks | PostgreSQL `chapters` and `chapter_chunks`, for the lifetime of the novel. | `DELETE /api/novels/{id}` or account deletion. |
| Characters, relationships, and canonical models | PostgreSQL, for the lifetime of the novel. Avatar bytes are not copied into NovelWorld; only provider-returned URL metadata is stored. | Novel or account deletion removes the rows and URL metadata. |
| Chat turns and messages | PostgreSQL, for the lifetime of the novel/account. PostgreSQL is authoritative. | Novel or account deletion. |
| Character memories | PostgreSQL, for the lifetime of the novel/account. | Novel or account deletion. The existing per-character short-memory action clears only the Redis projection, not durable history. |
| Short-memory projection | Redis lists, at most 50 messages per character/user pair. There is no time-based TTL; PostgreSQL remains the source of truth. | Internal cleanup establishes a tombstone before authoritative deletion, then removes matching keys. Account cleanup removes every matching user key; novel cleanup preserves other novels and users. |
| Deletion tombstones | Redis keys containing only a user UUID or user/novel UUID pair, retained for one hour. They contain no source text, message, profile, or model data. | Expire automatically. They prevent an already-committed asynchronous projection from recreating deleted cache data. |
| Choices, world state, narrative nodes, reading progress, and player timelines | PostgreSQL, for the lifetime of the novel/account. | Novel or account deletion. |
| Generated chapter prose | PostgreSQL `player_chapters`, for the lifetime of the novel/account. | Novel or account deletion. |
| User profile and refresh tokens | PostgreSQL, for the account lifetime. A refresh token is also removed by logout or when an expired token is presented. | Account deletion. |
| Web-managed LLM API key | Encrypted in the singleton PostgreSQL runtime configuration until replaced. | Removed when the final account is deleted. Environment-managed keys remain under operator control outside NovelWorld. |

Database foreign keys perform the authoritative cascade. Redis cleanup is a
service-owned operation: user-service and novel-service do not read or delete
agent-service tables directly.

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
  provide a restore or recycle-bin workflow.

## Data outside NovelWorld

Configured model and image providers may receive source excerpts, prompts,
messages, and image-generation descriptions. NovelWorld cannot delete provider
logs or provider-hosted image bytes unless that provider offers and the operator
configures a separate deletion contract. Their retention is governed by the
operator's provider agreement.

Container logs, database snapshots, volume copies, and external backups are also
operator-owned. NovelWorld does not create or prune backups, so operators must
apply their own retention and deletion schedules to those copies.

## Account export

Settings and `GET /api/account/export` provide the acting user a versioned
`account-export-v1` NDJSON download. The Gateway composes ordered, service-owned
HTTP fragments with bounded memory; it never reads downstream tables. A final
`complete` record is required before the browser saves the file.

The export contains profile metadata; owned novels, source chapters,
characters, relationships, canon models, and reading progress; durable messages
and memory content; and relevant narrative nodes, choices/transitions, world
state, and player chapters. It excludes credentials, tokens, runtime model
keys, internal operational state, embeddings, Redis/search projections, and
data held only by providers or operators. Each service uses its own statement
snapshot, so this is a portability export rather than a globally atomic backup.
The complete wire contract and consumer rules are in
[ACCOUNT_EXPORT.md](./ACCOUNT_EXPORT.md).
