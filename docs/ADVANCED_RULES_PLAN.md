# Advanced novel rules: D20 adjudication boundaries

This document owns the D20 preview's responsibilities, the bounded Laya (Jev)
adjudication contract, and its qualification limits. [ADR 0001](./adr/0001-source-bound-advanced-game-rules.md)
continues to own template, server validation, and dice authority;
[ADR 0009](./adr/0009-bounded-laya-d20-adjudication.md) records the accepted
bounded structural semantic-adjudication exception. Live adjudication quality
remains unqualified.

Naming: Laya and Jev refer to the same decision capability in NovelWorld. This
document uses **Laya (Jev)**; existing `LAYA_API_URL` / `LAYA_API_KEY` configuration
and implementation identifiers retain the Laya name. The current action hint
and semantic adjudication are different uses of that same capability.

## Outcome

Provide an optional rules-forward open-world mode without changing the default
narrative experience. Novel Service owns immutable game-rule templates bound to
the exact canonical-model version, schema version 1, and prompt version. Readers
reuse only the exact bound template. Existing v1 bindings retain their chapter
unlock behavior; newly created v2 bindings use the bounded basic vocabulary and
source-selection behavior below. Advanced player turns are resolved by a
server-owned D20 check before the existing narrative model renders the outcome.

## Current D20 responsibilities

This is a novel-specific, D20-inspired preview, not a complete D&D rules engine.

| Responsibility | Current owner and behavior |
|---|---|
| Attributes and check difficulty | Novel Service owns immutable templates identified by canon-model version, schema version 1, and exact prompt version (`novel-game-rules-v1` or `novel-game-rules-v2`). V1 keeps model-authored labels and its cited-chapter unlock behavior. V2 selects 3–6 capabilities from the fixed vocabulary below using bounded, stably ordered whole-book `world_rules`; server dictionaries supply labels/descriptions, and its citations are provenance only. Each supported action maps to an attribute and base DC. For configured adjudication, Laya (Jev) can select only a difficulty band; code maps easy/standard/hard to base DC −5/base DC/base DC +5, clamped to 5–30. |
| Eligibility and hard constraints | Narrative Service enforces identity, ownership, reading progress, supported targets, world state, and turn ordering. A successful roll never bypasses these checks. |
| Die and result | For a check, Narrative's secret-derived die is bound to user, novel, turn number, and request fingerprint. The domain computes `modifier = floor((score - 10) / 2)` and success as `d20 + modifier >= DC`. For `impossible` or `automatic_success`, there is no check result and no die is shown as evidence; the frozen category and server code determine the failed or automatic-success result. The resolution is persisted before prose and replayed for the same key. |
| Narration | H3/H4 journey generation uses DeepSeek under the repository's provider policy. It receives the resolved check and proposes prose/transitions; server validation remains the commit authority. Failed checks discard action-granted mutations, while world time and scheduled mainline events may still advance. |
| Optional action suggestion | Laya (Jev) receives bounded intent and candidate action-type descriptions only. The reader chooses/confirms the type and target and submits manually. A hint cannot authorize an action, set its DC, roll the die, or determine success. |
| Optional semantic adjudication | In advanced mode only, paired `LAYA_API_URL` and `LAYA_API_KEY` enable one bounded classification for a new turn. It may select impossible, automatic success, or a difficulty band; uncertainty and all unavailable/error paths fall back to the existing template check. Its confidence is not calibrated. It cannot bypass hard validation, choose an arbitrary DC, or choose a check's die/result. |
| Persistence and replay | The adjudication and resulting resolution are fenced with the existing world-turn claim. A frozen result is reused on replay; a reclaimed pending classification is not called again and falls back. Unknown database outcome or lost fencing stops before prose. |
| Semantic and game limits | This preview does not implement tactical combat, classes, spells, multiplayer fairness, arbitrary free-text semantic guarantees, or a full D&D rules engine. |

Implementation entrypoints:
[template validation/progress](../services/novel-service/src/domain/entities/game_rule_template.rs),
[check resolver](../services/narrative-service/src/domain/entities/game_rules.rs),
[dice adapter](../services/narrative-service/src/infrastructure/dice.rs),
[world-turn orchestration](../services/narrative-service/src/application/handlers/mod.rs),
[turn persistence](../services/narrative-service/src/infrastructure/persistence/pg_world_turn_repo.rs),
[transition validation](../services/narrative-service/src/domain/entities/world_session.rs),
and [Laya (Jev) action hints](./adr/0006-optional-laya-action-hints.md). The
bounded turn-classification contract is in
[ADR 0009](./adr/0009-bounded-laya-d20-adjudication.md).

For exact prompt version `novel-game-rules-v1`, a cited template chapter still
locked produces content-free `422 game_rules_unavailable_at_progress`; reading
further can make that bound v1 template visible. Version `novel-game-rules-v2`
uses chapter citations as provenance only, but requires a ready novel and
positive reading progress. If no source-backed world-rule mechanic can support
a template, or the bounded prompt would exceed 32 KiB, Novel Service returns
`422 game_rule_sources_unavailable` before taking a generation claim or calling
a provider. Continuing to read does not automatically repair missing mechanics
or the size bound; use narrative mode or wait for an independently corrected
canonical source. This is not a generic service outage. Reader profiles and
journeys stay private across exact template versions.

## Bounded Laya (Jev) turn adjudication

This is an optional, unqualified preview, not an authority transfer. It runs
only for a new advanced-mode turn with both existing Laya settings configured;
ordinary narrative turns never call it. The existing action-type hint remains a
separate user-triggered suggestion. Both use the same Laya (Jev) decision
capability and bounded HTTP client.

The classifier receives only a JSON-quoted, untrusted allowlist: player intent,
action kind, selected target display name, current location, player background,
abilities and inventory, the current action attribute label/description/score,
the template base DC, and hard-rule descriptions. Names are resolved from the
authorized entry context according to action kind. If an `investigate` target
could refer to both a location and a thread with the same identifier, or exists
only as a scheduled event, target resolution is ambiguous/unavailable: do not
call the classifier and use the template fallback. No scope UUID, full
novel text, history, future-event list, die, or check result is sent. The
serialized context is capped at 8 KiB; an oversized context makes zero
classifier calls and uses the template fallback. The classifier call has a 300
ms connect timeout, 2 s total timeout, 16 KiB response cap, and no HTTP retries.
Identity and reading progress are rechecked immediately before and after the
classification call; existing ownership, turn, target, and hard validation
remain in force.

The response is one bounded category: `impossible`, `automatic_success`, `easy`,
`standard`, or `hard`. Persisted metadata uses the exact decision tags
`impossible`, `automatic_success`, `easy_check`, `standard_check`, and
`hard_check`. Only a valid choice with probability at least 0.8 is
accepted; probability is an uncalibrated abstention heuristic. Missing
configuration, errors, malformed/oversized responses, uncertainty, or
probability below 0.8 use the template DC. `easy`, `standard`, and `hard` select
base DC minus 5, base DC, or base DC plus 5, clamped to 5–30. Automatic success
and impossible are no-check outcomes: the UI does not show a die as evidence
for either result. Impossible follows the existing failed-action normalization, clearing
action-granted mutations while world time and scheduled canon events may still
advance. Automatic success still passes every structural, target, ownership,
progress, and hard-rule check.

The world-turn row is first claimed with adjudication `pending`. One logical key
gets at most one classification attempt. A final decision, resolved DC, and
check result are frozen by fenced compare-and-set before DeepSeek prose starts.
A reclaimed pending row uses the template fallback without another Laya call;
legacy rows remain on the existing behavior and already-final rows are never
reclassified. An unknown database result or lost lease stops processing. If
prose fails after the decision is frozen, same-key replay reuses it. Disabling
Laya configuration stops new calls and selects template fallback; it does not
rewrite frozen results. World-turn prompts advance to version 3 while retaining
version 1 and 2 replay compatibility. Game-rule schema version is 1 for both
supported prompt versions, `novel-game-rules-v1` and `novel-game-rules-v2`;
profiles and sessions resolve their exact bound prompt version. Adjudication
metadata also has schema version 1. An older binary may not understand this
advanced metadata, so rollback of those journeys is fail-closed and recovery
requires a forward deploy.

## Versioned basic rules from whole-book world rules

Template v1 remains immutable for profiles and sessions bound to it. New basic
rules use schema version 1 with exact prompt version `novel-game-rules-v2`; the
source-bound template prompt remains `novel-game-rules-v1`. V2 receives at most
64 deterministically ordered canon `world_rules` and selects 3–6 source-backed
capabilities from the twelve-key vocabulary below, plus bounded numeric values
and source chapter references. This is a novel-specific subset, not a fixed set
of six attributes. The server supplies every label and description from its
fixed dictionary; provider-authored labels, descriptions, or plot prose are
rejected. This is a compact basic capability set, not a full rules engine or a
claim that extraction found every rule.

The complete available vocabulary is fixed; each generated template uses only
the source-supported subset:

| Key | Server label | Basic capability |
|---|---|---|
| `root` | 根骨 | Body aptitude and basic resilience |
| `agility` | 身法 | Movement, evasion, and coordination |
| `vigor` | 力道 | Force, impact, and sustained stamina |
| `insight` | 悟性 | Understanding, reasoning, and learning |
| `fortune` | 福缘 | Chance and environmental opportunities |
| `strategy` | 谋略 | Situational analysis and planning |
| `command` | 统御 | Organizing and coordinating action |
| `loyalty` | 义理 | Commitments, trust, and mutual aid |
| `resolve` | 心志 | Enduring pressure and maintaining intent |
| `influence` | 交涉 | Communication, persuasion, and negotiation |
| `knowledge` | 学识 | Applying acquired knowledge |
| `craft` | 技艺 | Practical and tool-based skills |

These keys describe capabilities only. They do not reveal hidden plot facts:
chapter references record provenance and do not unlock narrative content. Existing progress guards on real narrative context, targets, events,
and hard constraints remain in force; v2 templates require a ready novel and
positive reading progress. Every existing profile/session continues using its
exact bound prompt version; existing v1 bindings remain on v1. The canonical
novel's three-claim generation ceiling is shared across prompt versions and
serialized by its parent-canon lock; changing prompt versions does not replenish
attempts.

Migration 0030 is incompatible with the old Novel Service writer. Deployment
must stop and drain both Novel and Narrative writers before applying it, then
start compatible versions together. This rollout plumbing and offline checks do
not establish merge, deployment, paid-provider, or semantic-quality evidence.

## Qualification remains separate

Structural implementation does not qualify semantic quality. No paid model
comparison or human semantic-quality approval is claimed. Any such evaluation
requires separate authorization for the exact registration, provider/model,
corpus, budget, and stopping conditions. Compare Laya (Jev) with direct DeepSeek
structured judgment on a frozen human-labeled corpus of authorized novel
actions, including impossible, ambiguous, ordinary, hostile, and
locked-chapter cases. Measure incorrect approvals, useful abstention,
canon/spoiler/agency violations, latency, cost, and context disclosure; register
the rubric and improvement threshold before running. Confidence remains
uncalibrated and never stands for a character's D20 success chance.

The Laya adjudication slice introduces no new service or dependency; its
category and base DC use the existing world-turn resolution record. It adds no
database migration of its own. The separate v2 basic-template change does add
migration 0030. Provider probability and raw request/response bodies are not
persisted.

## Product contract

- Narrative mode remains the default and preserves existing API behaviour.
- Advanced mode is opt-in while creating the original `PlayerEntity`.
- A template contains 3-6 source-backed attributes and one check rule for every
  supported `WorldActionKind`.
- The player receives template defaults and may redistribute points within the
  template budget. The server validates the final allocation.
- Existing structural world validation runs before provider work or commit.
  The die never makes an invalid target, dead character, future entity, stale turn, or
  unavailable thread valid. A successful check means the best feasible outcome
  within the supplied hard rules; it never authorizes the literal wording of an
  impossible free-text intent. The bounded adjudicator may classify the supplied
  context, but arbitrary free-text correctness is not guaranteed.
- Advanced world actions use `d20 + attribute modifier` against the template DC.
  The authoritative roll and modifier breakdown are persisted before the LLM
  result is accepted and are replayed exactly for the same idempotency key.
- The LLM receives the authoritative check outcome and may render prose and
  propose already-validated state transitions; it cannot choose or change the
  roll, DC, attribute, or success result.
- Every attribute and action rule carries source chapters. For v1, template
  facts are filtered by the reader's server-owned progress; v1 advanced mode is
  unavailable when filtering would leave an incomplete action mapping. V2
  citations establish provenance only and do not gate the fixed basic template;
  its actual narrative context and action eligibility remain progress-guarded.
- Player profiles and world sessions bind an exact `canon_model_version` and
  template schema/prompt version. A newer canon model never silently changes an
  existing journey's rules.

## Backend design

### novel-service ownership

1. Add a `GameRuleTemplate` aggregate with strict validation, provenance chapter
   references, versioned prompt/schema metadata, bounded attribute counts, and a
   complete action-to-attribute/DC mapping.
2. Add a novel-service-owned `novel_game_rule_templates` table. Rows use a
   generating/ready/failed state, attempt fencing, and an expiring lease so only
   one replica performs provider work for an exact novel/model/prompt-version
   identity. V2 claim accounting is additionally serialized across prompt
   variants by canonical novel and model version.
3. Add repository ports for claim, renew/complete/fail, and ready reads. PostgreSQL
   remains an adapter; application handlers depend only on the repository trait.
4. Generate the template on first explicit advanced-mode request from the
   immutable canonical story model, not per user and not on ordinary novel import.
   Validate model output before publishing the immutable ready template.
5. Expose authenticated internal endpoints to request/read the progress-filtered
   template. Do not permit narrative-service to read novel-service tables.
6. Bound prompt and response sizes. The current writer permits at most three persisted claims per canonical novel
   and canon-model version across both prompt versions, including consumed v1
   claims, serialized under the parent-canon lock. Claim admission has a fixed five-second database-transaction deadline,
   no retry, and must not dispatch provider work after an unknown commit outcome;
   an ambiguous outcome may leave a durable generating claim and does not refill
   the budget. Renew the generation lease while provider work is in flight. The
   existing LLM adapter, provider-call deadlines, response limits, and retry
   policy remain unchanged; admission timeout does not authorize dispatch.
   Shared transport retries remain within one logical claim.
7. A failed or unavailable template affects only the explicit advanced request.
   It never changes novel readiness and never blocks narrative-mode entry.

### narrative-service ownership

1. Extend the existing novel HTTP port with request/read game-rule operations.
2. Keep `PlayerEntity` backward compatible with a default narrative-mode rules
   profile. Advanced profiles store only the selected template identity and
   validated attribute values, not a private template copy.
3. Add a domain action-check resolver. It consumes `PlayerEntity`, the template,
   a validated `WorldAction`, and a D20 value; it has no HTTP, database, or random
   dependency.
4. Add a `DiceRoller` domain port. The infrastructure adapter derives an
   unpredictable D20 from the internal runtime secret, world turn number, and
   action fingerprint. A provider failure followed by a new idempotency key
   therefore cannot reroll the same action against the same world state.
5. Persist the computed resolution on the in-progress `world_turns` claim. The
   request fingerprint covers only the client action; retries return the stored
   claim resolution even if a process or secret changes.
6. Include the resolution in completed results and journal entries, and in the
   world-turn prompt. Preserve legacy rows through optional/defaulted fields.
7. Resolve existing journeys against their bound model/template version. Return
   a conflict rather than substituting the latest template when that version is
   unavailable.

## Frontend design (FSD)

- `shared/types`: wire types only.
- `entities/narrative`: query/mutation hooks and wire contracts.
- `features/player-entry`: opt-in advanced-mode controls and accessible attribute
  allocation; it imports only entities/shared.
- `features/world-action`: show the applicable attribute/DC before submission.
- `widgets/world-dashboard`: compose the character sheet, action feature, and
  journal; display the authoritative persisted roll and outcome.
- `pages/reader`: coordinate existing feature/widget placement only. No rules or
  check calculations live in the page.

## Verification

1. Domain tests: strict template validation, progress filtering, point budget,
   action mapping, D20 boundaries, success/failure, and narrative-mode fallback.
2. Repository contract tests: one generation owner, concurrent in-progress
   response, expired lease recovery, attempt fencing, immutable ready template,
   and persisted/replayed action checks.
3. Application/interface tests: authorization, optional generation, player
   allocation validation, idempotency, prompt binding, and legacy JSON.
4. Frontend tests: default narrative mode unchanged, advanced template request,
   accessible allocation, action preview, and journaled roll display.
5. Run `cargo fmt --all --check`, targeted Rust tests, narrative/novel service
   tests, frontend unit tests, lint/type-check, and the affected integration gate.

## Non-goals

- Tactical maps, initiative, combat rounds, classes, spell slots, party control,
  multiplayer fairness, competitive rewards, or a general-purpose D&D engine.
- Allowing generated executable formulas or client-supplied roll outcomes.
- Generating a separate template for every reader or every action.
- Claiming deterministic semantic understanding of arbitrary free-text intent;
  generated prose remains an untrusted projection behind authoritative state
  validation.

## Rollback

Unset either Laya setting to stop new adjudicator calls; new advanced turns then
use the template fallback, and already-frozen results remain unchanged. Before
rolling back the implementation, disable advanced-mode template requests.
Narrative profiles omit
the new optional player/session fields when serialized, so state written by this
version retains the previous binary's exact JSON shape. Ready templates and the
nullable world-turn resolution column are additive and can be ignored. Existing
advanced profiles and resolutions with adjudication metadata intentionally fail
closed on a previous binary rather than silently becoming narrative turns.
Restore those readers by forward-deploying this version again; no down migration
or data rewrite is required. The game-rule template format is unchanged.

## Review record

Pre-implementation review removed a separate rules microservice, executable
formula DSL, unbounded per-action adjudication, and per-reader templates. It also
made progress safety, exact version binding, leases, provider budgets, and the
meaning of a successful check explicit before code was written.

Post-implementation review fixed three correctness edges: progress now exposes
an exact immutable template or none (never a changing shape under one version),
technical provider failures cannot be used to reroll the same action/state, and
failed v1 template generation has a three-claim logical-generation ceiling (one
logical provider call per claim; bounded transport retries may replay that same
request). V2 uses the shared cross-prompt canonical budget described above. The
default narrative path performs no template generation or dice work.

ADR 0009 accepts only the bounded adjudication exception to ADR 0001 as a
structural private preview. The recorded local checks and independent code
review apply to that v1/Laya implementation, not to v2 templates or migration
0030. V2 still requires its own runtime and required-CI evidence; no live-provider
or deployment result is asserted here. CI, merge, and deployment status for ADR
0009 remain tracked separately in
[#418](https://github.com/Wisdoverse/novelworld/issues/418).
