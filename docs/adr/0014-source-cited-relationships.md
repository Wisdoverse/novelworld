# ADR 0014: Source-cited relationships at saved reading progress

- Status: Accepted implementation contract; acceptance evidence remains tracked
- Date: 2026-10-06
- Owners: Novel and frontend maintainers
- Related: [Issue #549](https://github.com/Wisdoverse/novelworld/issues/549)

## Context

The legacy relationship table has no chapter evidence. Its public graph must
wait for exact full-book progress. Canon v1 already stores each relationship's
complete first extracted description and chapter excerpts. The extractor keeps
that first fact rather than merging later descriptions. Reuse those facts to
show cited relationships before the reader finishes the book.

## Decision

Add the read-only `GET /novels/{id}/relationships/source-v1` projection. Novel
continues to own Canon, source chapters, shelves, and reading progress. Select
immutable model version 1; do not fall back to a newer model.

Authorize the acting reader's Ready novel and saved progress. Return a whole
relationship only when every citation is unlocked and matches its source
chapter. Use existing lexical first-appearance checks for both endpoint names.
Do not crop evidence or truncate descriptions. Keep at most 256 endpoint names,
256 relationships, and eight citations per relationship. Reuse Canon text
validation. Reject missing, malformed, or oversized authority.

After all source reads, recheck shelf ownership, Ready status, total chapters,
and unchanged progress. Use one five-second deadline, no automatic retries, and
`private, no-store` for success and failure. No provider call or new durable
reader data is needed. The legacy full-book graph and internal Agent grounding
keep their existing shapes and exact-full boundary.

The route uses Novel's existing idempotent chapter-1 progress initialization if
the progress row is absent. It never advances that record or saves a graph.

The Characters page shows escaped source text and fixed chapter links in an
accessible list. Query keys include principal, novel, and saved chapter. The
client validates scope, bounds, and an explicit field allowlist. It withholds
the list while progress is unknown, loading, refetching, or in error.

## Alternatives considered

- Publish legacy rows earlier: they have no chapter evidence.
- Re-extract relationships or add a new table: existing Canon facts suffice.
- Crop mixed early and later citations: this would authorize unsupported text.
- Add a graph library: a cited list uses existing UI components.

## Consequences

The list describes extracted source facts, not relationship evolution or
private world state. Source matching does not prove semantic accuracy. Existing
live quality, export-wide spoiler safety, and H3/H4 qualification remain open.
Final validation is a read boundary, not a transaction shared with later reader
actions. A concurrent progress change causes a content-free failure.

## Rollout and rollback

Deploy compatible Novel before the frontend. An older Novel returns no new
view; the client displays its localized load error. Rollback reverts the route
and UI. No migration, saved-state rewrite, or provider configuration is needed.

## Evidence

Domain tests check complete visible citations, malformed evidence, bounds, and
private-field omission. Handler tests check ownership and a rewind after source
I/O. `world_source_http` uses real PostgreSQL and production routes to check
independent reader progress, source matching, rewind, and legacy compatibility.
Frontend tests check scope and cached-progress failure; `source-relationships`
browser tests check English, Chinese, citation links, and accessibility.
Issue #549 records local results, non-author review, final CI, and merge.
