# ADR 0001: Source-bound shared rules and server-owned D20 resolution

- Status: Accepted
- Date: 2026-08-23
- Owners: novel-service, narrative-service, and frontend owners
- Related: [`../ADVANCED_RULES_PLAN.md`](../ADVANCED_RULES_PLAN.md); [roadmap issue #202](https://github.com/Wisdoverse/novelworld/issues/202)

## Context

Readers want an optional rules-driven journey whose attributes fit a novel's
world, while narrative play remains the default. Generating rules for every
reader or action would repeat provider work. Treating model prose as an
authoritative roll or state transition would violate the untrusted-model,
spoiler, idempotency, and service-ownership boundaries.

This decision covers shared rule generation, player allocation, action checks,
and committed world-turn effects. Tactical combat, a general rule DSL,
per-action model adjudication, and live semantic-quality claims are out of
scope.

## Decision

The novel-service owns one immutable `GameRuleTemplate` row per novel and
canonical-model version. Schema and prompt versions are metadata bound into
that row; supporting a new schema or prompt for the same canonical model needs
an explicit migration or canonical-model version bump. The service builds the
template only from cited canon facts, validates all model output, and exposes it
to narrative-service through authenticated internal HTTP. A leased and fenced
PostgreSQL row coordinates replicas. One leased claim performs one logical
generation call; at most three claims are allowed for a template. Ready rows
cannot be updated.

The narrative-service owns the player's selected attribute values, the exact
template snapshot bound to a world session, server-side D20 resolution, and the
world-turn ledger. The roll is derived with a service secret from user, novel,
turn number, and action fingerprint; it is persisted before provider prose so
retrying the same action/state cannot reroll it. The browser never supplies the
roll or success result.

An advanced failure is authoritative: before validation and commit, the domain
normalizer replaces model-proposed events with one neutral failed-attempt event
and clears relationship, location, thread, player-location, inventory,
knowledge, faction, and explicit canon-event mutations. World time and the
scheduled canon mainline may still advance deterministically. A successful
check still passes the existing hard-rule and source-context validators.

Narrative player/session fields use serde defaults and are omitted while in the
legacy narrative state. Advanced fields remain explicit. The public API adds
optional fields and keeps narrative requests valid.

## Alternatives considered

- A new rules microservice was rejected because the existing novel and
  narrative ownership boundaries already cover generation and resolution.
- Per-action LLM adjudication was rejected because it increases latency, cost,
  nondeterminism, and prompt-injection exposure.
- A general formula/DSL engine was rejected because eight existing action kinds
  need only a bounded attribute and DC mapping.
- Client-generated randomness was rejected because it permits rerolls and
  forged outcomes.
- Allowing failure complications from model-generated mutations was rejected
  because their direction cannot be verified structurally.

## Consequences

Templates are reused across authorized readers, while player values and worlds
remain private. Generation adds one derived table and world turns add one
nullable resolution column. The services gain a versioned internal HTTP
contract but do not share tables.

The model may still narrate semantically poor prose; only the committed state is
authoritative. Failed advanced actions cannot create model-selected partial
state changes. Provider transport retries may replay the same logical request,
but the persisted three-claim budget prevents unbounded semantic regeneration.

Generation outcome and latency logs plus the existing LLM-operation metrics are
the operational signals. The world-turn journal records the exact check and
normalized transition for audit and replay.

## Rollout and rollback

Ship the mode behind an opt-in frontend control. Keep narrative mode as the
default and monitor generation failures, validation failures, elapsed time, and
world-turn commit errors. There is no runtime kill switch in this slice; abort
requires deploying code that hides the advanced control and rejects the template
request route.

The migration is additive and has no down migration. Narrative profiles omit
new player/session fields, retaining the previous binary's exact JSON shape.
Advanced profiles deliberately fail closed on a previous binary because its
strict deserializer does not know the rule fields. Forward-deploy this version
again to recover those readers; immutable templates and persisted rolls remain
available.

## Evidence

- Domain tests validate template identity, source citations, allocation bounds,
  D20 arithmetic, deterministic rolling, and failed-check normalization.
- Repository/integration tests cover lease fencing, immutable rows, legacy
  migration replay, and persisted checks.
- Domain serialization tests pin the previous narrative player/session JSON
  shape.
- Frontend tests cover the default narrative request, advanced allocation,
  action preview, journal display, and cross-novel state reset.
- CI is authoritative for PostgreSQL migration replay, Rust/clippy, frontend
  FSD/type/lint/test/build, browser accessibility, and production topology.
