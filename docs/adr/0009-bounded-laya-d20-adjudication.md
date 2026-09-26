# ADR 0009: Bounded Laya (Jev) adjudication for advanced D20 turns

- Status: Accepted (bounded structural private preview)
- Date: 2026-09-26
- Owners: narrative-service and frontend owners
- Related: [ADR 0001](./0001-source-bound-advanced-game-rules.md), [ADR 0006](./0006-optional-laya-action-hints.md), [advanced rules contract](../ADVANCED_RULES_PLAN.md), and [issue #418](https://github.com/Wisdoverse/novelworld/issues/418)
- Supersedes: Only ADR 0001's exclusion of per-action model adjudication, and only for the bounded preview described here. All other ADR 0001 decisions remain in force.

## Context

The server already owns action eligibility, hard validation, template DCs,
server-derived dice, and durable world-turn replay. A bounded semantic
classification can add contextual feasibility and difficulty while keeping
those authority boundaries. Laya and Jev are the same decision capability in
this product; `LAYA_API_URL`, `LAYA_API_KEY`, and implementation identifiers
retain the existing Laya names. The existing action-type hint remains a
separate user-confirmed feature.

This exception is an optional, structurally bounded preview. It does not claim
that confidence is calibrated or that the service understands arbitrary
Chinese D20 actions correctly. Semantic quality, paid comparison against
direct DeepSeek structured judgment, and human approval remain unqualified and
separately authorized work.

## Decision

Call adjudication only for a new advanced-mode world turn when both existing
Laya endpoint and key are configured. Narrative-mode turns never call it. The
existing Laya client and endpoint are reused with the existing four-slot
admission limit; the request keeps the existing `multilingual` model. There is
no new service, dependency, or database migration. The classifier call uses a
300 ms connect timeout, 2 s total timeout, 16 KiB response limit, and no HTTP
retries.

After existing ownership, source-visibility, turn, target, and hard validation,
recheck player identity and progress immediately before classification and
again after its response. Build an allowlisted JSON context containing only:
intent; action kind; selected target display name; current location display
name; player background, capabilities, and inventory; current action attribute
label, description, and score; template base DC; and visible hard-rule
descriptions. Quote all included strings as untrusted data. Resolve target
display names from the authorized entry context according to action kind. If
an `investigate` target is ambiguous between a location and thread with the
same identifier, or exists only as a scheduled event, do not call the
classifier and use template fallback. Do not send scope UUIDs, full novel text,
history, future-event lists, dice, or results. Serialize the context before the
request; above 8 KiB, make zero classifier calls and use the template fallback.
The context is private reader/world data crossing an external provider
boundary; provider-side retention remains governed by the operator's agreement.

Accept only one bounded category: `impossible`, `automatic_success`, `easy`,
`standard`, or `hard`. These category names map to persisted metadata decision
tags `impossible`, `automatic_success`, `easy_check`, `standard_check`, and
`hard_check`. A valid choice must have probability at least 0.8; explicit
uncertainty or lower confidence abstains to the template check. This probability
is an uncalibrated threshold, not a quality or calibration claim. `easy` uses
`base DC - 5`, `standard` uses `base DC`, and `hard` uses `base DC + 5`,
clamped to 5–30.
`automatic_success` skips the check. `impossible` yields a failed outcome. No
die is presented as evidence for either no-check outcome. Automatic success
still passes every existing server validation; impossible follows failed-action
normalization and clears action-granted mutations, while time and scheduled
canon events may still advance. Dice generation and the final result remain
server-owned.

The first PostgreSQL world-turn claim stores a pending resolution. A logical
idempotency key permits at most one classifier attempt. The final decision,
resolved DC, and result must be written by fenced compare-and-set before any
DeepSeek narrative prose. On lease reclaim, a pending decision settles to the
template fallback without another classifier request. Legacy rows and final
decisions are never reclassified. If the database outcome is unknown or the
lease is lost, stop before prose. If prose fails after the decision is frozen,
same-key replay uses that frozen decision and does not redraw the die.

Classifier absence, uncertainty, malformed/oversized output, timeout, overload,
or other failure settles to the existing template check. These paths do not
disable advanced mode or weaken a validation rule. Unsetting either Laya
setting disables new classifier calls and selects this fallback for new turns;
already-frozen decisions remain unchanged. The optional `ActionCheck`
adjudication metadata uses schema version 1 and is stored in the existing JSONB
resolution. The world-turn prompt advances to `world-turn-v3` while preserving
`world-turn-v1` and `world-turn-v2` replay compatibility; the rules template
remains `novel-game-rules-v1`. Older
binaries do not understand the new advanced metadata and must fail closed;
restore support by forward-deploying the compatible version.

## Consequences

The Laya (Jev) provider receives a narrow but sensitive snapshot of player and
world display facts. NovelWorld can bound what it sends but cannot enforce the
provider's retention or training policy. The response may influence only the
check category/DC or a bounded no-check result; it cannot authorize an action,
change a target, override progress or hard rules, generate prose, or supply a
roll.

Pending recovery and exact-key replay do not repeat a semantic decision. A
frozen adjudication is auditable with the existing world-turn resolution and
account export. There is no database migration, new durable store, or separate
adjudication service. This decision does not qualify narrative quality, latency,
cost, or semantic correctness.

## Acceptance evidence and limits

- Local checks passed for two real-PostgreSQL CAS/fencing/commit/reclaim/replay
  tests, the 165-test Narrative suite, and the 252-test frontend suite.
- Independent non-author code review passed the bounded data disclosure,
  legacy fail-closed behavior, unchanged hard validation, and no-call narrative
  boundaries.

Local workspace tests (596), PostgreSQL/Redis integration tests (92),
Clippy with warnings denied, DDD/FSD gates, frontend lint/type/build and
dependency audit, and browser checks (52) passed. The frontend unit suite
passed with two workers; the default highly concurrent run timed out in an
unchanged lazy-route test. CI, merge, and deployment status are tracked
separately in issue #418 and are not asserted here. Acceptance covers the bounded structural preview only. No provider
benchmark or human semantic-quality approval is required for this decision;
those remain separate and unqualified.
