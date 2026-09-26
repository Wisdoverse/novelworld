# Advanced novel rules: current behavior and evaluation plan

This document owns the D20 preview's responsibilities and the proposed semantic
judgment evaluation. The implementation below is structurally verified; live
template/narrative quality and arbitrary free-text judgment remain unqualified.
[ADR 0001](./adr/0001-source-bound-advanced-game-rules.md) owns the accepted
runtime boundary. The evaluation proposal below does not change that decision.

## Outcome

Provide an optional rules-forward open-world mode without changing the default
narrative experience. A novel-specific rule template is generated once, owned
by novel-service, and reused by every authorized reader. Advanced player turns
are resolved by a server-owned D20 check before the existing narrative model
renders the outcome.

## Current D20 responsibilities

This is a novel-specific, D20-inspired preview, not a complete D&D rules engine.

| Responsibility | Current owner and behavior |
|---|---|
| Attributes and check difficulty | Novel Service owns one immutable, source-backed template per canon model version. Each supported action type maps to a template attribute and DC; the runtime does not ask a model to assign a new DC for each free-text intent. |
| Eligibility and hard constraints | Narrative Service enforces identity, ownership, reading progress, supported targets, world state, and turn ordering. A successful roll never bypasses these checks. |
| Die and result | Narrative's secret-derived die is bound to user, novel, turn number, and request fingerprint. The domain computes `modifier = floor((score - 10) / 2)` and success as `d20 + modifier >= DC`. The claimed result is persisted before prose and replayed for the same key. |
| Narration | H3/H4 journey generation uses DeepSeek under the repository's provider policy. It receives the resolved check and proposes prose/transitions; server validation remains the commit authority. Failed checks discard action-granted mutations, while world time and scheduled mainline events may still advance. |
| Optional action suggestion | Laya receives bounded intent and candidate action-type descriptions only. The reader must choose/confirm the type and target and submit manually. A hint cannot authorize an action, set its DC, roll the die, or determine success. |
| Semantic referee | No Jev/Laya per-action semantic judge is connected. Free-text feasibility, context-sensitive difficulty, tactical combat, and full D&D rules are not delivered by this preview. |

Implementation entrypoints:
[template validation/progress](../services/novel-service/src/domain/entities/game_rule_template.rs),
[check resolver](../services/narrative-service/src/domain/entities/game_rules.rs),
[dice adapter](../services/narrative-service/src/infrastructure/dice.rs),
[world-turn orchestration](../services/narrative-service/src/application/handlers/mod.rs),
[turn persistence](../services/narrative-service/src/infrastructure/persistence/pg_world_turn_repo.rs),
[transition validation](../services/narrative-service/src/domain/entities/world_session.rs),
and [Laya hints](./adr/0006-optional-laya-action-hints.md).

If a cited template chapter is still locked, both services preserve the
content-free `422 game_rules_unavailable_at_progress`. The Chinese entry form
guides the reader to continue reading or use narrative mode; this is not a
generic Novel Service outage. Template generation failure remains a separate
failure case. Reader profiles and journeys stay private even when readers reuse
the same canonical novel and ready template.

## Proposed Jev evaluation — not an integration commitment

Jev's [official description](https://typesafe.ai/) presents bounded typed
decisions with probabilities/confidence. This makes it a candidate for a narrow
classification experiment; that suitability is an inference, not evidence that
it judges Chinese D20 play better than DeepSeek. Laya's current compatible hint
API also does not establish equivalent semantic judgment quality.

Start with the simplest direct DeepSeek structured-judgment baseline. Before any
paid execution, review the exact registration and obtain authorization for its
provider/model, corpus, budget, and stopping conditions. Freeze human-labeled
Chinese action examples from authorized novels, including impossible actions,
ambiguous intent, ordinary actions, hostile instructions, and locked-chapter
traps. Give each candidate identical bounded, progress-authorized context and
compare blinded judgments for action type, feasibility, and whether a check is
needed. Any difficulty band is an offline candidate, not a runtime DC change.

Measure incorrect approvals, useful abstention, canon/spoiler/agency violations,
latency, cost, and context disclosure. Register the rubric and meaningful
improvement threshold before execution; do not invent a passing threshold after
seeing results. An uncertain decision must abstain to manual confirmation or a
predefined baseline, never grant new authority. Confidence describes a model's
judgment, not a character's probability of passing a D20 check.

Only measured benefit can justify a runtime proposal. Such a proposal needs an
ADR replacing the per-action-adjudication exclusion, a reviewed data boundary,
deadlines and retry/idempotency rules, and persisted/replayed decisions before
any narration. Hard validation and dice arithmetic remain server-owned. No
provider benchmark or qualification result is claimed by this documentation.

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
  impossible free-text intent. Full semantic adjudication of arbitrary prose is
  not claimed by this slice.
- Advanced world actions use `d20 + attribute modifier` against the template DC.
  The authoritative roll and modifier breakdown are persisted before the LLM
  result is accepted and are replayed exactly for the same idempotency key.
- The LLM receives the authoritative check outcome and may render prose and
  propose already-validated state transitions; it cannot choose or change the
  roll, DC, attribute, or success result.
- Every attribute and action rule carries source chapters. Template facts are
  filtered by the reader's server-owned progress. Advanced mode is unavailable
  when filtering would leave an incomplete action mapping.
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
   one replica performs provider work for a novel/model version.
3. Add repository ports for claim, renew/complete/fail, and ready reads. PostgreSQL
   remains an adapter; application handlers depend only on the repository trait.
4. Generate the template on first explicit advanced-mode request from the
   immutable canonical story model, not per user and not on ordinary novel import.
   Validate model output before publishing the immutable ready template.
5. Expose authenticated internal endpoints to request/read the progress-filtered
   template. Do not permit narrative-service to read novel-service tables.
6. Bound prompt and response sizes, allow at most one logical generation call
   per leased claim and three persisted claims per template, renew the lease
   while the provider call is in flight, and emit generation latency/outcome
   logs and existing LLM-operation metrics. Shared transport retries remain the
   retry policy for that one logical call and do not create another generation.
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

Disable advanced-mode template requests before rollback. Narrative profiles omit
the new optional player/session fields when serialized, so state written by this
version retains the previous binary's exact JSON shape. Ready templates and the
nullable world-turn resolution column are additive and can be ignored. Existing
advanced profiles intentionally retain their rule fields: the previous binary
rejects them rather than silently executing them as narrative turns. Restore
those readers by forward-deploying this version again; no down migration or data
rewrite is required.

## Review record

Pre-implementation review removed a separate rules microservice, executable
formula DSL, per-action adjudication call, and per-reader templates. It also
made progress safety, exact version binding, leases, provider budgets, and the
meaning of a successful check explicit before code was written.

Post-implementation review fixed three correctness edges: progress now exposes
an exact immutable template or none (never a changing shape under one version),
technical provider failures cannot be used to reroll the same action/state, and
failed template generation has a three-claim logical-generation ceiling (one
logical provider call per claim; bounded transport retries may replay that same
request). The default narrative path performs no template generation or dice
work.
