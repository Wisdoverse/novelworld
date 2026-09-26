# ADR 0011: User-confirmed series worlds and frozen basic rules

- Status: Accepted structural preview
- Date: 2026-09-26
- Owners: Novel and Narrative service maintainers
- Related: [Issue #426](https://github.com/Wisdoverse/novelworld/issues/426)

## Context

Books in one series can share a setting and basic D20 definitions. Automatic
association can merge unrelated worlds, and reusing chapter numbers from a
different book can falsify provenance. Existing player rules and committed
checks must remain immutable when the shelf association changes.

## Decision

Novel Service owns private `user_world_series` definitions and
`user_novel_world_series` shelf associations. Laya (Jev) suggests a candidate;
only an authenticated reader's explicit confirmation creates or associates a
series. No global canonical novel is changed. The immutable revision-1
definition contains a reader-confirmed, bounded background and an exact ready
v2 basic template snapshot. Creating it atomically associates its source book;
an already associated source returns a conflict. Creation never generates a
template, takes a generation claim, refills a budget, or retries terminal work.

Source template novel/version/chapter identities remain real source identities.
The `series-game-rules-v1` wrapper carries series ID, revision and target novel
separately. New profiles must match the current association; existing profiles
resolve their exact private definition even after reassociation or source shelf
removal. Narrative owns player allocations, frozen session setting and checks;
it reaches Novel through HTTP and never reads Novel tables. Target-book canon,
progress, entity authorization and hard rules remain independent and authoritative.
Pure narrative sessions may freeze the confirmed setting without enabling D20.
All background text is untrusted, quoted data, not permission or canon.

The optional matcher uses the existing external Laya configuration, a 300 ms
connect / 2 s total deadline, four local admission slots, no HTTP retries,
8 KiB request / 16 KiB response limits and at most eight candidates. It sends
only title/author/genre and candidate names/metadata, never scope IDs or novel
text. Unknown, low-confidence, malformed, busy and unavailable results require
manual selection. A probability of 0.8 is conservative abstention, not evidence
of calibrated accuracy. Laya never generates setting prose or authorizes writes.

## Alternatives considered

Manual association alone is simpler and remains available. Laya is optional
because the requested automatic recognition must still abstain safely. A
global series ID on shared novels was rejected because one reader's choice
must not alter another reader's worlds. Relabeling the source template as the
target book was rejected because it destroys provenance.

## Consequences and lifecycle

Migration 0031 adds two Novel-owned tables and an immutable-update trigger.
The owner-user cascading FK is declared single-node schema debt (21 total),
not database-role isolation. Shelf removal deletes only that association;
definitions and frozen sessions remain until account erasure. Source IDs have
no cascading source FK so deleting the source cannot destroy another book's
frozen mechanics. Snapshots contain fixed dictionaries/numbers/provenance,
not source text. Account export includes definitions and associations; account
deletion erases both through owner-scoped cascades.

## Rollout and rollback

Apply 0031 through the normal migration path with ingress/writers quiesced and
deploy matching Novel, Narrative and frontend versions together. This additive
schema does not replace the existing 0021/0024/0025/0030 release barriers.
Older peers reject the new prompt version; mixed versions cannot support the
series journey. Once series-bound state exists, returning to an application
that cannot read it is unsupported. Recover by rolling forward with compatible
services; never rewrite old profiles, checks, frozen Diagnostics or source canon.

## Evidence

The owning domain, HTTP adapter and disposable PostgreSQL tests exercise
permissions, provenance, immutable snapshots, reassociation and claim avoidance.
The architecture and FSD gates check source boundaries. These are structural
evidence only; live Laya recognition quality, production rollout, recovery and
multi-replica operation remain unqualified.
