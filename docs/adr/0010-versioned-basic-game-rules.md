# ADR 0010: Versioned basic game-rule templates

- Status: Accepted (structural private preview)
- Date: 2026-09-26
- Owners: Novel Service, Narrative Service, and release owners
- Related: [ADR 0001](./0001-source-bound-advanced-game-rules.md), [ADR 0009](./0009-bounded-laya-d20-adjudication.md), [advanced rules plan](../ADVANCED_RULES_PLAN.md)
- Supersedes: ADR 0001 only for v2 template identity, source-selection, progress semantics, and shared generation-budget decisions. Immutable v1 bindings remain supported.

## Context

Template v1's chapter citations are part of template availability, which can make
basic player capabilities wait for later source chapters. The product needs a
small stable rules vocabulary grounded in extracted whole-book world rules,
without exposing later narrative facts, accepting provider prose as trusted
rules, refilling paid-call budgets by changing prompt versions, or changing
existing sessions.

## Decision

Keep every existing profile/session bound to its exact template identity; in
particular, existing v1 readers remain on v1. New basic templates use schema
version 1 with exact prompt version `novel-game-rules-v2` (the established
source-bound template prompt remains `novel-game-rules-v1`). Generation input
is limited to 64 stably ordered canonical `world_rules`; output can select only
keys in a fixed twelve-entry
vocabulary, bounded numeric values, and source chapter references.
The server provides user-facing labels and descriptions from fixed dictionaries.
Unknown/duplicate keys, unbounded values, invalid references, and provider-authored
free text are rejected. No plot prose is an output field.

V2 source references are provenance only, not an unlock gate. A v2 template
requires a ready novel and positive reading progress. Every existing narrative
context, target, event, hard-rule, ownership, and timeline guard stays in force;
this decision changes which basic capabilities can be established, not access
to story facts. It does not claim complete world-rule extraction or semantic
quality.

At most three generation claims are available per canonical novel and
canon-model version across all prompt versions. A parent-canon lock serializes
that shared budget; prompt-version changes do not replenish it. Claim admission
runs in a PostgreSQL transaction with a fixed five-second deadline and no
retry. If commit outcome is unknown, do not dispatch provider work; the durable
claim may remain ambiguous and does not replenish the budget. Existing
provider-call adapter, deadlines, response limits, and retry behavior remain
unchanged. Existing v1 rows and exact reader bindings remain addressable and are
not rewritten.

## Compatibility and rollout

Migration 0030 is incompatible with the old Novel Service writer. Stop and drain
both Novel and Narrative writers before applying it; start compatible versions
together after the barrier passes. Preserve existing v1 profiles and sessions.
Do not roll back to an old writer against the migrated schema; recover by
forward deployment. Release scripts and desktop migration embedding must include
0030 and reject adoption or upgrade without it.

## Consequences

The fixed vocabulary limits provider influence and makes client-visible wording
server-owned. Whole-book rule metadata can establish basic capabilities earlier,
while chapter references no longer gate those mechanics. A bad extraction or
incorrect allowed selection can still produce a poor capability template.
This ADR records a design decision; it is not evidence of a paid run, semantic
evaluation, qualification, merge, or deployment. Those statuses must be updated
from their own evidence when available.

## Evidence

Offline release-state drill and source-level checks are required for this
implementation. Runtime, migration, and service acceptance remain owned by their
code/tests and must be reported separately from this ADR.
