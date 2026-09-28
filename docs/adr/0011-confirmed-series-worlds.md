# ADR 0011: User-confirmed series worlds and frozen basic rules

- Status: Accepted structural preview
- Date: 2026-09-26
- Owners: Novel and Narrative service maintainers
- Related: [Issue #426](https://github.com/Wisdoverse/novelworld/issues/426),
  [evidence-backed matching #438](https://github.com/Wisdoverse/novelworld/issues/438),
  [arbitrary-title matching #440](https://github.com/Wisdoverse/novelworld/issues/440),
  [deferred D20 rules #442](https://github.com/Wisdoverse/novelworld/issues/442)

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
definition contains a reader-confirmed, bounded background and a stable source
book identity. Creating it atomically associates its source book; an already
associated source returns a conflict. If an exact ready v2 basic template for
the current source Canon exists, creation freezes it. Otherwise the definition
is pending D20 rules and immediately supports the shared background. Creation
never generates a template, takes a generation claim, refills a budget, or
retries terminal work.

A later explicit source-book D20 request uses the existing bounded generation
claim. Once a ready source template exists, Novel alone binds that exact
template to a pending definition in one owner-scoped, idempotent transition.
No target member generates substitute rules while the source is pending. The
source identity and confirmed background never change; a nonempty snapshot is
immutable. Existing ready definitions and previously frozen Narrative sessions
are not rewritten.

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
title/author/genre and candidate names, plus at most six deduplicated
chapter-1 location/faction names of at most 40 characters. Every valid citation
must be in chapter 1 and at least one must contain the name. Descriptions,
excerpts, world-rule prose, later-chapter clues and plot are excluded; evidence
names are not echoed in the public suggestion. Scope IDs and novel text are
never sent. Unknown, low-confidence, malformed, busy and unavailable results require
manual selection. A probability of 0.8 is conservative abstention, not evidence
of calibrated accuracy. Laya never generates setting prose or authorizes writes.

Confirmed memberships organize candidates. The arbitrary-title revision
removes the title-stem gate, unconfirmed title-family grouping, and pre-Canon
title ranking. Novel privately ranks a bounded Ready shelf by source-cited whole-book
location/faction names, after verifying that a citation contains each name.
Those later-book names remain inside Novel: they are absent from Laya and
DeepSeek inputs, public suggestions, and cached result bodies. A scan that
exceeds its fixed limit, or overlapping/ambiguous evidence, abstains. Only
when Laya normally abstains can a unique strong private-evidence candidate be
shown as a separately labeled server heuristic. Confirmed groups require
agreement from every member; reader confirmation is still the only
association authority. This does not rewrite prior decisions or associations.

Ordinary recognition calls Laya, then may use the bounded internal heuristic
only after a normal Laya abstention. A separate reader-clicked DeepSeek
second opinion uses a dedicated `series_matching` operation and the existing
credential/accounting path. One resolved configuration supplies both the cache
provider/model identity and the physical dispatch. Non-DeepSeek providers and
Diagnostic-bound configurations are refused before reserve or dispatch. The
30 s total deadline includes configuration lookup. The 512-token total output
ceiling includes reasoning when the resolved user configuration enables it;
truncated or invalid output safely abstains without increasing the allowance.
HTTP retries, empty-JSON
fallback and application repair are disabled.

Novel-owned PostgreSQL suggestions bind the reader, target, provider evidence,
candidates, method and policy. Laya's key also binds a digest of private
ranking evidence; DeepSeek's key does not change when its exact prompt and
candidate scope are unchanged. A unique durable claim allows one dispatch;
concurrent requests reuse the state. Failed, interrupted or unknown outcomes
are never reclaimed for another attempt. Result-only requests (`check_only=true`)
never insert a claim or dispatch, even after configuration/evidence changes.
Only validated candidate/status/reason
results are stored, not raw responses or model rationale. Cache hits revalidate
current shelf access and Ready sources. Shelf removal and account erasure
remove the cache; account export includes safe decision metadata. Existing
frozen Diagnostic profiles, budgets, series snapshots and D20 replays are unchanged.

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
not source text. Migration 0033 backfills a stable source ID for existing
definitions, permits a missing template for new background-only definitions,
and narrows the trigger to allow only one empty-to-ready snapshot bind. Account
export includes definitions and associations; account
deletion erases both through owner-scoped cascades. Migration 0032 adds the
Novel-owned suggestion cache with a cascading shelf FK and no new cross-owner
relation. It is applied before the matching service starts.

## Rollout and rollback

Apply 0031, 0032, and 0033 through the normal migration path with ingress/writers quiesced and
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
