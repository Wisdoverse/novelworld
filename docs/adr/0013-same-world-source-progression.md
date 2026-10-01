# ADR 0013: Same-world source progression

- Status: Accepted implementation contract; acceptance evidence remains tracked
- Date: 2026-10-01
- Owners: Novel, Narrative and frontend maintainers
- Related: [Issue #467](https://github.com/Wisdoverse/novelworld/issues/467)

## Context

The initial world seals the reader's entry snapshot. Resuming that world cannot
import later source even when ordinary actions change threads and reading
progress advances. A reader therefore receives action feedback while later
canonical events remain unavailable. Resetting the character or replacing the
origin would rewrite authority. Chapter navigation alone cannot repair this
missing world transition.

## Decision

Keep `entry_context`, creation checkpoint and rules immutable. Add optional
`source_context` with omitted-null serialization so unextended schema-v1 states
retain canonical bytes. The first extension creates schema v2; all
current action, visibility and context consumers use one effective-context
accessor. Historical memory recovery uses the committed result snapshot.

Novel owns monotonic `POST /progress/{novel_id}/advance`; the existing absolute
PUT still supports deliberate rewinds. Its existing internal world-entry route
adds explicit source-extension query mode for an exact pinned model and one
next chapter. Every delta catalog item carries its complete nonempty provenance.
Whole text with later evidence stays withheld, including linked definitions
and attached death effects. Original IDs never replace admitted definitions.
No latest-model fallback or private world state crosses this service boundary.
The Narrative HTTP adapter retains a two-second deadline and no automatic retry.

Narrative owns `POST /narrative/{novel_id}/world/source`, strict observed turn,
source and target coordinates, UUID-v4 key and `world_source_operations` journal.
The world and operation commit atomically. Replay retains operation metadata and
returns a freshly progress-guarded current view, never a historical restore.
Admission makes no generation call, dice roll or world-time increment. Existing
player state, event outcomes, deaths and modified threads remain authoritative.
New canonical order cannot precede an already advanced event.

Every transaction touching world and turn/source authority locks the world row
first. Reservation, reclaim, completion and supersession share that order and
connection. In-progress actions and completed/pending memory projections block
source admission. Modern action claims also fence their observed source before
provider I/O and at completion. Legacy completed keys retain their original
fingerprints and exact-result replay.

The reader automatically schedules one source admission from its latest terminal
committed turn when no scheduled or delayed event remains in the active scene.
The turn's observed source chapter must equal the current source; legacy null
coordinates use the immutable entry source. Admission changes that equality,
which prevents the same turn advancing twice across reloads without another
marker or table. Only the current user's fresh self/Player/progress/route world
view is eligible. Unresolved turns, projections, stored requests and source
operations retain their existing locks. No next-scene click is required during
normal progression; ambiguous outcomes retain explicit recovery.

Source orchestration reads fresh progress before every progress advance,
including exact-key retries, and pauses an observed automatic rewind. It sends
the optional `expected_current_chapter` snapshot to Novel. The owner atomically
requires that exact chapter, self identity and no character identity reference;
a mismatch returns `reading_progress_changed` with no update, and the frontend
does not dispatch source admission. Unguarded deliberate original-reading calls
remain compatible. No new schema is needed. Deploy Novel before the automatic
frontend; an older owner rejects the new field without writing.

Frontend orchestration remains above independent entity APIs under FSD. It
persists the bounded exact source operation before either write and establishes
the recovery lock before the first automatic progress effect. It suppresses
absolute route-progress writes until completion/recovery synchronizes the route
to the maximum of current source, operation source and freshly persisted reading
progress. A partial two-owner outcome retains its key; progress is never rolled
back to conceal it. Unknown progress blocks mutations until refetch. Explicit
rewinds synchronously hide later derived prose, catalogs and chat. Pending
recovery still allows explicit original-chapter reading through monotonic
progress/navigation, retaining the original source key until exact recovery;
this avoids trapping a rewound tab behind a concurrently expanded world.

## Alternatives considered

- Keep sealed source forever: ordinary turns cannot reach later plotted events.
- Navigate to the next chapter only: world context remains sealed.
- Rebuild or overwrite entry: destroys identity, history or diverged outcomes.
- Admit the latest model or crop future evidence: changes pinned authority or
  exposes unsupported future text.
- A generic workflow engine or a second world engine: unnecessary for this one
  bounded state transition; reuse existing repositories and action pipeline.

## Consequences

Novel and Narrative do not share a SQL transaction. A rewind after a Narrative
commit can hide the result; return a content-free unknown outcome and recover
with the same key after progress restoration. Catalog bounds and unavailable
pinned evidence fail explicitly. A safe empty delta does not imply new plotted
events, and later action/model quality remains a separate acceptance question.
Journal records participate in account export and cascade with their own world.
Reading an admitted chapter does not implicitly generate a new legacy branch
continuation in an extended world; stored chapter projections remain replayable.
No new provider configuration, credential or paid Diagnostic is required.

## Rollout and rollback

Migration 0036 is additive SQL but a semantic reader barrier. Apply it through
the existing managed release: stop Narrative before exposing candidate assets,
stop/drain old Novel and Agent, migrate under the exact durable target marker,
then start compatible services. Expose the button only with matching versions.
Desktop embeds the migration before runtime services start. Release adoption
requires all five barriers; rollback/marked restore reject crossing 0036.
Old Narrative cannot read extended schema-v2 worlds. Recover forward with the
compatible release, retaining origin, active context, journal and recovery key;
never strip fields or reset user worlds. Unextended v1 states remain readable.

## Evidence

[Issue #467](https://github.com/Wisdoverse/novelworld/issues/467) tracks the exact
source, domain/HTTP, real PostgreSQL race, reader/browser, release barrier, CI,
merge and local Docker evidence. Reviewed contracts are implementation inputs;
this ADR does not itself prove those gates. Provider quality, historical
Diagnostics and formal H4 qualification remain separate.

Implementation checks: Narrative source-domain/TOCTOU tests, Novel
`world_source_http` against disposable PostgreSQL, integration
`repository_contracts` source/turn lock tests, frontend
`world-source-progression.spec.ts`, and `release_state_drill.sh`.
`core_reader_loop.sh` exercises real service HTTP source 1 → 2, a subsequent
source event outcome, exact operation replay after restart, export and erasure
using the existing synthetic provider. `E2E_CONTAINER_PREFIX` scopes its Docker
operations for an isolated local fixture; the default CI topology remains
`novel`. Never run it against user data. The
[synthetic reader screenshot](../evidence/world-source-auto-progression.png) shows
automatic source progress without a scene button; it is not live-world acceptance
evidence.
