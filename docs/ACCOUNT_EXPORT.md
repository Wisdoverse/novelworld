# Account Export Contract

`GET /api/account/export` downloads the acting user's portable NovelWorld data
as newline-delimited JSON. The Gateway derives the subject only from the
validated JWT; callers cannot select another account.

## Wire format: `account-export-v1`

Successful responses use `application/x-ndjson`, `Cache-Control: no-store`, an
attachment filename containing only the user UUID and UTC date, and no
`Content-Length`. Records appear in this order:

1. one `manifest` with `schema: account-export-v1`, the user UUID, creation
   time, `snapshot: service-local`, and the ordered service list;
2. `service_start`, zero or more `record` lines, then `service_complete` for
   each of `user`, `novel`, `agent`, and `narrative`;
3. one final `complete` record with `schema: account-export-v1`.

The final record is the only completeness proof. A dependency error, timeout,
or client disconnect terminates the stream without it. Consumers must not
present or import a file whose final non-empty line is not that record.

Every `record` has this shape:

```json
{"type":"record","service":"novel","kind":"chapter","data":{}}
```

Text is serialized as JSON, so embedded newlines and other untrusted characters
never create additional records.

## Included records

| Service | Kinds |
|---|---|
| user | `profile` |
| novel | `novel`, `chapter`, `character`, `character_relationship`, `canon_story_model`, `reading_progress` |
| agent | `chat_message`, `character_memory` |
| narrative | `narrative_node`, `user_choice`, `world_state`, `player_chapter`, `world_turn` |

Profile fields include identity, email, display/avatar metadata, role,
verification state, and account timestamps. Novel and character records retain
provider-returned asset URLs, not provider-hosted bytes. Narrative nodes include
the user's own nodes and shared/canonical nodes referenced by that user's
choices.

World-turn records include the portable action, status, committed transition
and exact replay result when present. Explicitly excluded data includes password hashes, access and refresh tokens,
runtime LLM keys, internal service tokens, source object keys, chat-turn
and world-turn fingerprints, leases, and failure codes, vector embeddings and memory access metadata,
Redis projections, chapter search chunks, provider logs/asset bytes, and
operator backups.

## Consistency and resource limits

Each data service owns an internal-token-authenticated endpoint and reads its
allowlisted records with one ordered PostgreSQL statement. That statement has a
consistent service-local snapshot and streams rows with backpressure. The four
snapshots are taken sequentially, so the export is portable data, not a
distributed point-in-time backup.

Each Gateway process permits at most two simultaneous exports and enforces one
15-minute deadline across all fragments. It streams with bounded server memory
and stores no export artifact, temporary file, or queued job. Dropping the
client stream releases the permit and downstream database query.
