use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::{oneshot, watch};
use tracing::{info, warn, Instrument};
use uuid::Uuid;

use crate::domain::entities::game_rules::{
    resolve_action_check, GameRuleTemplate, PlayerRuleProfile, ResolutionMode,
};
use crate::domain::entities::narrative_node::{
    fit_character_world_context, NarrativeChoice, NarrativeNode, WorldState, WorldStateError,
};
use crate::domain::entities::player_entity::PlayerEntity;
use crate::domain::entities::world_session::{
    build_world_turn_prompt_with_check, parse_world_turn_transition_with_check, trailing_chars,
    CharacterWorldContext, ObservedWorldAction, RecentWorldActionContext, RecentWorldTurnContext,
    WorldAction, WorldActionKind, WorldSession, MAX_CHARACTER_RECENT_ACTIONS,
    MAX_RECENT_WORLD_NARRATIVE_CHARS, MAX_RECENT_WORLD_TURNS,
};
use crate::domain::ports::{AgentMemoryPort, DiceRollerPort, LlmPort, NarrativeLlmTask};
use crate::domain::repositories::{
    BeginWorldTurn, ChapterInfo, ChapterReadRepository, CharacterBrief, ChoiceCommit,
    GameRuleTemplateRequestError, MemoryProjectionStatus, NarrativeNodeRepository, NovelInfo,
    PlayerChapter, PlayerChapterOrigin, PlayerChapterRepository, UserChoiceRecord,
    UserChoiceRepository, WorldStateRepository, WorldTurnClaim, WorldTurnJournalEntry,
    WorldTurnRepository, WorldTurnResult,
};
use crate::domain::services::narrative_engine::{
    build_branch_prompt, build_player_chapter_prompt, is_chinese_narrative, parse_generated_branch,
};
use crate::domain::services::narrative_transition::{
    build_transition_prompt, parse_transition, CanonEntityRef, NarrativeTransition,
};

const MAX_NARRATIVE_PROMPT_BYTES: usize = 32 * 1024;
const MAX_NARRATIVE_PROMPT_CHARS: usize = 16_000;
const MAX_CONSEQUENCE_BYTES: usize = 32 * 1024;
const MAX_CONSEQUENCE_CHARS: usize = 8_000;
const MAX_TRANSITION_BYTES: usize = 128 * 1024;
const MAX_PLAYER_CHAPTER_BYTES: usize = 64 * 1024;
const MAX_PLAYER_CHAPTER_CHARS: usize = 20_000;
const MAX_WORLD_TURN_PROMPT_BYTES: usize = 64 * 1024;
const MAX_WORLD_TURN_PROMPT_CHARS: usize = 32_000;
const MAX_WORLD_TRANSITION_BYTES: usize = 128 * 1024;
const WORLD_TURN_LEASE_HEARTBEAT: Duration = Duration::from_secs(30);
/// Journey memories: bounded structured fact text, fixed importance for
/// committed world turns, and bounded retries inside the turn ambiguity
/// boundary.
const MAX_JOURNEY_MEMORY_EVENT_CHARS: usize = 2_000;
const MAX_JOURNEY_MEMORY_FIELD_CHARS: usize = 128;
const MAX_JOURNEY_MEMORY_LOCATION_CHARS: usize = 200;
const WORLD_TURN_MEMORY_IMPORTANCE: i32 = 7;
const JOURNEY_MEMORY_RETRIES: usize = 2;
const MAX_WORLD_JOURNAL_ENTRIES: usize = 100;
const MEMORY_PROJECTION_RECOVERY_BATCH: usize = 10;
const MEMORY_PROJECTION_RECOVERY_INTERVAL: Duration = Duration::from_secs(30);
const JOURNEY_MEMORY_NAMESPACE: Uuid = Uuid::from_u128(0x4d5f_215d_111c_5f25_8614_71e8_5f8a_3e63);

pub(crate) fn journey_memory_id(turn_id: Uuid) -> Uuid {
    Uuid::new_v5(&JOURNEY_MEMORY_NAMESPACE, turn_id.as_bytes())
}

fn action_targets_character(action: &WorldAction, character_id: Uuid) -> bool {
    matches!(
        action.kind,
        WorldActionKind::Converse | WorldActionKind::Ally | WorldActionKind::Oppose
    ) && action
        .target_id
        .as_deref()
        .and_then(|target| Uuid::parse_str(target).ok())
        .is_some_and(|target| target == character_id)
}

fn character_witnessed_turn(
    action: &WorldAction,
    transition: &crate::domain::entities::world_session::WorldTurnTransition,
    character_id: Uuid,
) -> bool {
    action_targets_character(action, character_id)
        || transition
            .events
            .iter()
            .any(|event| event.actor_character_ids.contains(&character_id))
        || transition
            .relationship_changes
            .iter()
            .any(|change| change.character_id == character_id)
}

fn recent_character_actions(
    journal: Vec<WorldTurnJournalEntry>,
    character_id: Uuid,
    maximum_turn_number: i64,
) -> Vec<RecentWorldActionContext> {
    let mut selected = journal
        .into_iter()
        .rev()
        .filter(|entry| {
            entry.turn_number <= maximum_turn_number
                && action_targets_character(&entry.action, character_id)
        })
        .take(MAX_CHARACTER_RECENT_ACTIONS)
        .map(|entry| RecentWorldActionContext {
            turn_id: entry.turn_id,
            turn_number: entry.turn_number,
            action: ObservedWorldAction {
                kind: entry.action.kind,
                target_id: entry.action.target_id,
            },
        })
        .collect::<Vec<_>>();
    selected.reverse();
    selected
}

/// Deterministic protagonist pick: role 'protagonist', earliest first
/// appearance, stable id tiebreak. Zero protagonists => None (caller skips).
pub(crate) fn resolve_protagonist(characters: &[CharacterBrief]) -> Option<Uuid> {
    characters
        .iter()
        .filter(|character| character.role == "protagonist")
        .min_by_key(|character| {
            (
                character.first_appearance_chapter.is_none(),
                character.first_appearance_chapter,
                character.id,
            )
        })
        .map(|character| character.id)
}

/// Idempotently project a committed world-turn fact into permanent memory.
/// Eligible turns are acknowledged before the caller receives success; a
/// failed acknowledgement remains pending for exact replay or bounded recovery.
#[allow(clippy::too_many_arguments)] // Explicit projection scope.
pub(crate) async fn record_world_journey_memory(
    agent_memory: &dyn AgentMemoryPort,
    chapter_repo: &dyn ChapterReadRepository,
    memory_id: Uuid,
    user_id: Uuid,
    novel_id: Uuid,
    result: &WorldTurnResult,
) -> Result<bool> {
    let characters = chapter_repo.list_characters(novel_id, user_id).await?;
    let Some(character_id) = resolve_protagonist(&characters) else {
        tracing::debug!(%novel_id, "no protagonist found; skipping journey memory");
        return Ok(false);
    };
    if !character_witnessed_turn(&result.action, &result.transition, character_id) {
        tracing::debug!(%novel_id, %character_id, "protagonist did not witness the world turn; skipping journey memory");
        return Ok(false);
    }
    let Some((event, source_chapter_high_water)) = world_journey_fact(result, character_id)? else {
        tracing::debug!(%novel_id, %character_id, "witnessed turn had no admissible journey fact; skipping journey memory");
        return Ok(false);
    };
    let mut last_error = None;
    for attempt in 0..=JOURNEY_MEMORY_RETRIES {
        match agent_memory
            .save_permanent_memory(
                memory_id,
                character_id,
                user_id,
                novel_id,
                source_chapter_high_water,
                &event,
                WORLD_TURN_MEMORY_IMPORTANCE,
            )
            .await
        {
            Ok(()) => return Ok(true),
            Err(error) => {
                warn!(%error, attempt, %memory_id, "journey memory save failed");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("journey memory projection failed")))
}

fn leading_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn bounded_trimmed_chars(value: &str, maximum: usize) -> Option<String> {
    let bounded = leading_chars(value, maximum);
    let trimmed = bounded.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn add_journey_fact_section(
    fact: &mut serde_json::Value,
    key: &'static str,
    mut items: Vec<serde_json::Value>,
) -> Result<()> {
    while !items.is_empty() {
        fact["committed_changes"]
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("journey fact changes must be an object"))?
            .insert(key.into(), serde_json::Value::Array(items.clone()));
        if serde_json::to_string(fact)?.chars().count() <= MAX_JOURNEY_MEMORY_EVENT_CHARS {
            return Ok(());
        }
        items.pop();
    }
    fact["committed_changes"]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("journey fact changes must be an object"))?
        .remove(key);
    Ok(())
}

/// Serialize only facts with explicit provenance for one character. Generated
/// prose and unrelated mutations are deliberately excluded: witnessing one
/// part of a turn does not make the character omniscient about the whole turn.
fn world_journey_fact(
    result: &WorldTurnResult,
    character_id: Uuid,
) -> Result<Option<(String, i32)>> {
    let session = result
        .world_state
        .open_world()
        .map_err(|error| anyhow::anyhow!(error))?
        .ok_or_else(|| anyhow::anyhow!("committed world turn has no world session"))?;
    Ok(world_journey_fact_at(
        result,
        character_id,
        session.turn_number,
        session.world_time,
    )?
    .map(|fact| (fact, session.entry_context.unlocked_through_chapter)))
}

fn world_journey_fact_at(
    result: &WorldTurnResult,
    character_id: Uuid,
    turn_number: i64,
    world_time: i64,
) -> Result<Option<String>> {
    let transition = &result.transition;
    let reader_action = action_targets_character(&result.action, character_id).then(|| {
        serde_json::json!({
            "kind": result.action.kind,
            "target_id": character_id,
        })
    });
    let events = transition
        .events
        .iter()
        .filter(|event| event.actor_character_ids.contains(&character_id))
        .filter_map(|event| {
            let summary = bounded_trimmed_chars(&event.summary, MAX_JOURNEY_MEMORY_FIELD_CHARS)?;
            let location_id = event.location_id.as_deref().and_then(|location| {
                bounded_trimmed_chars(location, MAX_JOURNEY_MEMORY_LOCATION_CHARS)
                    .filter(|location| !location.chars().any(char::is_control))
            });
            Some(serde_json::json!({
                "summary": summary,
                "actor_character_ids": [character_id],
                "location_id": location_id,
            }))
        })
        .take(4)
        .collect::<Vec<_>>();
    let relationships = transition
        .relationship_changes
        .iter()
        .filter(|change| change.character_id == character_id)
        .take(4)
        .map(|change| {
            serde_json::json!({
                "character_id": change.character_id,
                "delta": change.delta,
            })
        })
        .collect::<Vec<_>>();
    let reader_action_count = usize::from(reader_action.is_some());
    let mut fact = serde_json::json!({
        "schema_version": 2,
        "source": "committed_world_turn",
        "authority": "explicit_character_witness_facts",
        "source_turn_id": result.turn_id,
        "witness_character_id": character_id,
        "turn_number": turn_number,
        "world_time": world_time,
        "canonical_checkpoint_chapter": transition.canonical_checkpoint_chapter,
        "change_counts": {
            "events": events.len(),
            "relationships": relationships.len(),
            "reader_action": reader_action_count,
        },
        "committed_changes": {},
    });
    if let Some(reader_action) = reader_action {
        fact.as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("journey fact must be an object"))?
            .insert("reader_action".into(), serde_json::to_value(reader_action)?);
    }

    add_journey_fact_section(&mut fact, "relationships", relationships)?;
    add_journey_fact_section(&mut fact, "events", events)?;
    let committed_changes = fact["committed_changes"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("journey fact changes must be an object"))?;
    let emitted_relationships = committed_changes
        .get("relationships")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let emitted_events = committed_changes
        .get("events")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    fact["change_counts"] = serde_json::json!({
        "events": emitted_events,
        "relationships": emitted_relationships,
        "reader_action": reader_action_count,
    });

    if reader_action_count == 0 && emitted_events == 0 && emitted_relationships == 0 {
        return Ok(None);
    }

    let encoded = serde_json::to_string(&fact)?;
    anyhow::ensure!(
        encoded.chars().count() <= MAX_JOURNEY_MEMORY_EVENT_CHARS,
        "journey fact exceeded its budget"
    );
    Ok(Some(encoded))
}

#[derive(Debug, thiserror::Error)]
pub enum NarrativeError {
    #[error("Resource not found")]
    NotFound,
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Conflict(String),
    #[error("A different choice is already committed")]
    ChoiceConflict,
    #[error("World turn is already in progress")]
    TurnInProgress { retry_after_seconds: u64 },
    #[error("World turn outcome is unknown; retry with the same Idempotency-Key")]
    TurnOutcomeUnknown,
    #[error("Game rule template generation is in progress")]
    GameRulesInProgress { retry_after_seconds: u64 },
    #[error("Game rule template generation budget is exhausted")]
    GameRulesExhausted,
    #[error("Reading progress is behind the committed world context")]
    ReadingProgressBehindWorld,
    #[error("Novel service is unavailable")]
    Unavailable(#[source] anyhow::Error),
    #[error("Consequence generation failed")]
    Llm(#[source] anyhow::Error),
    #[error("Narrative operation failed")]
    Internal(#[source] anyhow::Error),
}

pub type NarrativeResult<T> = std::result::Result<T, NarrativeError>;

fn map_world_state_write_error(error: anyhow::Error) -> NarrativeError {
    match error.downcast_ref::<WorldStateError>() {
        Some(WorldStateError::TimelineConflict(message)) => {
            NarrativeError::Conflict(message.clone())
        }
        _ => NarrativeError::Internal(error),
    }
}

fn require_same_choice(requested: i32, committed: i32) -> NarrativeResult<()> {
    if requested == committed {
        Ok(())
    } else {
        Err(NarrativeError::ChoiceConflict)
    }
}

fn require_choice_submission_allowed(
    requested: i32,
    committed: Option<i32>,
    choice_chapter: i32,
    player_checkpoint: Option<i32>,
    open_world_started: bool,
) -> NarrativeResult<()> {
    if let Some(committed) = committed {
        return require_same_choice(requested, committed);
    }
    if open_world_started {
        return Err(NarrativeError::Conflict(
            "Use world actions after entering the open world".into(),
        ));
    }
    if player_checkpoint.is_some_and(|checkpoint| choice_chapter > checkpoint) {
        return Err(NarrativeError::Conflict(
            "Branch choice is later than the sealed PlayerEntity checkpoint".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChoiceResult {
    pub chapter_number: i32,
    pub consequence: String,
    pub transition: NarrativeTransition,
    pub chapter_content: String,
    pub world_state: WorldState,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EffectiveChapter {
    pub chapter_number: i32,
    pub content: String,
    pub generated: bool,
}

#[derive(Debug, Clone)]
pub struct CreatePlayerEntityCommand {
    pub checkpoint_chapter: Option<i32>,
    pub name: String,
    pub background: String,
    pub capabilities: Vec<String>,
    pub location_id: String,
    pub inventory: Vec<String>,
    pub rules: PlayerRuleProfile,
}

#[derive(Debug, Serialize)]
pub struct PlayerEntry {
    pub player: Option<PlayerEntity>,
    pub checkpoint_chapter: i32,
    pub locations: Vec<CanonEntityRef>,
    pub game_rules: Option<GameRuleTemplate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenWorldView {
    pub player: PlayerEntity,
    pub session: WorldSession,
    pub world_state: WorldState,
    pub journal: Vec<WorldTurnJournalEntry>,
}

/// HTTP-facing completion view. The committed result remains the single
/// persisted payload; projection status is the separately committed journal
/// state flattened into the wire response.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorldTurnResponse {
    #[serde(flatten)]
    pub result: WorldTurnResult,
    pub memory_projection_status: MemoryProjectionStatus,
}

struct ResolvedChapter {
    canonical: ChapterInfo,
    content: String,
    generated: bool,
    origin: Option<PlayerChapterOrigin>,
}

pub struct NarrativeCommandHandler {
    pub node_repo: Arc<dyn NarrativeNodeRepository>,
    pub choice_repo: Arc<dyn UserChoiceRepository>,
    pub world_state_repo: Arc<dyn WorldStateRepository>,
    pub player_chapter_repo: Arc<dyn PlayerChapterRepository>,
    pub chapter_repo: Arc<dyn ChapterReadRepository>,
    pub world_turn_repo: Arc<dyn WorldTurnRepository>,
    pub llm: Arc<dyn LlmPort>,
    pub agent_memory: Arc<dyn AgentMemoryPort>,
    pub dice_roller: Arc<dyn DiceRollerPort>,
}

struct WorldTurnLease {
    stop: Option<oneshot::Sender<()>>,
    lost: watch::Receiver<bool>,
}

impl WorldTurnLease {
    fn start(repo: Arc<dyn WorldTurnRepository>, turn_id: Uuid, attempt: i64) -> Self {
        let (stop, mut stopped) = oneshot::channel();
        let (lost, receiver) = watch::channel(false);
        let current_span = tracing::Span::current();
        tokio::spawn(
            async move {
                let mut heartbeat = tokio::time::interval(WORLD_TURN_LEASE_HEARTBEAT);
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    _ = &mut stopped => return,
                    _ = heartbeat.tick() => {
                        match repo.renew_turn(turn_id, attempt).await {
                            Ok(true) => {}
                            Ok(false) => {
                                tracing::warn!(%turn_id, attempt, "world turn lease was fenced");
                                let _ = lost.send(true);
                                return;
                            }
                            Err(error) => {
                                tracing::error!(%turn_id, attempt, error = ?error, "world turn lease renewal failed");
                                let _ = lost.send(true);
                                return;
                            }
                        }
                    }
                }
            }
            }
            .instrument(current_span),
        );
        Self {
            stop: Some(stop),
            lost: receiver,
        }
    }

    async fn run<T>(&mut self, operation: impl Future<Output = T>) -> Option<T> {
        if *self.lost.borrow() {
            return None;
        }
        tokio::select! {
            biased;
            result = operation => Some(result),
            _ = self.wait_until_lost() => None,
        }
    }

    async fn wait_until_lost(&mut self) {
        while !*self.lost.borrow() && self.lost.changed().await.is_ok() {}
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for WorldTurnLease {
    fn drop(&mut self) {
        self.stop();
    }
}

impl NarrativeCommandHandler {
    async fn owned_novel(&self, novel_id: Uuid, user_id: Uuid) -> NarrativeResult<NovelInfo> {
        self.chapter_repo
            .get_novel_info(novel_id, user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)
    }

    async fn reader_identity_is_self(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<bool> {
        self.chapter_repo
            .reader_identity_is_self(novel_id, user_id)
            .await
            .map_err(NarrativeError::Unavailable)
    }

    async fn require_self_reader_identity(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<()> {
        if self.reader_identity_is_self(user_id, novel_id).await? {
            Ok(())
        } else {
            Err(NarrativeError::Conflict(
                "Player and open-world access require the self reader identity".into(),
            ))
        }
    }

    fn choices_only_world_state(mut world_state: WorldState) -> NarrativeResult<WorldState> {
        let choices = world_state
            .state
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .ok_or_else(|| {
                NarrativeError::Internal(anyhow::anyhow!(WorldStateError::InvalidChoices))
            })?;
        world_state.state = serde_json::json!({
            "choices": choices,
            "world_events": [],
        });
        Ok(world_state)
    }

    async fn require_world_source_visible(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        world_state: &WorldState,
    ) -> NarrativeResult<()> {
        let Some(source_chapter_high_water) = world_state
            .source_chapter_high_water()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
        else {
            return Ok(());
        };
        self.require_source_chapter_visible(user_id, novel_id, source_chapter_high_water)
            .await
    }

    async fn require_memory_projection_eligible_snapshot(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        world_state: &WorldState,
    ) -> NarrativeResult<()> {
        let source_chapter_high_water = world_state
            .source_chapter_high_water()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?;
        let progress = self
            .chapter_repo
            .get_reading_progress(novel_id, user_id)
            .await
            .map_err(|_| NarrativeError::TurnOutcomeUnknown)?;
        if !progress.reader_identity_is_self {
            return Err(NarrativeError::TurnOutcomeUnknown);
        }
        if source_chapter_high_water
            .is_some_and(|source_chapter| progress.current_chapter < source_chapter)
        {
            return Err(NarrativeError::ReadingProgressBehindWorld);
        }
        Ok(())
    }

    async fn require_source_chapter_visible(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        source_chapter: i32,
    ) -> NarrativeResult<()> {
        let current_chapter = self
            .chapter_repo
            .get_current_chapter(novel_id, user_id)
            .await
            .map_err(NarrativeError::Unavailable)?;
        if current_chapter < source_chapter {
            return Err(NarrativeError::ReadingProgressBehindWorld);
        }
        Ok(())
    }

    async fn branch_node_if_visible(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        source_chapter: i32,
        node: Option<NarrativeNode>,
    ) -> NarrativeResult<Option<NarrativeNode>> {
        if let Some(node) = &node {
            self.require_source_chapter_visible(
                user_id,
                novel_id,
                source_chapter.max(node.chapter_number),
            )
            .await?;
        }
        Ok(node)
    }

    fn uncommitted_branch_is_eligible(
        world_state: &WorldState,
        chapter_number: i32,
    ) -> NarrativeResult<bool> {
        let open_world_started = world_state
            .open_world()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .is_some();
        let beyond_player_checkpoint = world_state
            .player_entity()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .is_some_and(|player| chapter_number > player.canonical_checkpoint_chapter);
        let precedes_committed_prefix = world_state
            .latest_choice_chapter()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .is_some_and(|latest| chapter_number <= latest);
        Ok(!open_world_started && !beyond_player_checkpoint && !precedes_committed_prefix)
    }

    async fn committed_branch_node(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
    ) -> NarrativeResult<Option<NarrativeNode>> {
        let existing_choice = self
            .choice_repo
            .find_by_novel(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?
            .into_iter()
            .find(|choice| choice.chapter_number == chapter_number);
        let Some(existing_choice) = existing_choice else {
            return Ok(None);
        };
        let node = self
            .node_repo
            .find_by_id(existing_choice.node_id)
            .await
            .map_err(NarrativeError::Internal)?
            .filter(|node| {
                node.novel_id == novel_id
                    && node.chapter_number == chapter_number
                    && node.user_id.is_none_or(|owner_id| owner_id == user_id)
            })
            .ok_or(NarrativeError::NotFound)?;
        Ok(Some(node))
    }

    /// A node read or provider call can overlap Player creation, a choice, or
    /// open-world entry. Reload the committed choice and WorldState after the
    /// candidate is durable; this final state read is the response's
    /// linearization point and prevents returning options that cannot submit.
    async fn finalize_uncommitted_branch_node(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        node: Option<NarrativeNode>,
    ) -> NarrativeResult<Option<NarrativeNode>> {
        if let Some(committed) = self
            .committed_branch_node(user_id, novel_id, chapter_number)
            .await?
        {
            return self
                .branch_node_if_visible(user_id, novel_id, chapter_number, Some(committed))
                .await;
        }
        if !self.reader_identity_is_self(user_id, novel_id).await? {
            return Ok(None);
        }
        let latest_world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        if !Self::uncommitted_branch_is_eligible(&latest_world_state, chapter_number)? {
            return Ok(None);
        }
        self.branch_node_if_visible(user_id, novel_id, chapter_number, node)
            .await
    }

    async fn narrative_world_state(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<(WorldState, Option<PlayerEntity>, bool)> {
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let player = world_state
            .player_entity()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?;
        let self_identity = self.reader_identity_is_self(user_id, novel_id).await?;
        if self_identity && player.is_none() {
            return Err(NarrativeError::Conflict(
                "Create PlayerEntity before entering an original-player branch".into(),
            ));
        }
        let visible_state = if self_identity {
            world_state.clone()
        } else {
            Self::choices_only_world_state(world_state.clone())?
        };
        self.require_world_source_visible(user_id, novel_id, &visible_state)
            .await?;
        Ok((
            world_state,
            self_identity.then_some(player).flatten(),
            self_identity,
        ))
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_player_entry(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        checkpoint_chapter: Option<i32>,
    ) -> NarrativeResult<PlayerEntry> {
        if checkpoint_chapter.is_some_and(|chapter| chapter < 1) {
            return Err(NarrativeError::Validation(
                "Player checkpoint must be a positive chapter".into(),
            ));
        }
        self.owned_novel(novel_id, user_id).await?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        if let Some(player) = world_state
            .player_entity()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
        {
            let game_rules = match player.rules.mode {
                ResolutionMode::Narrative => None,
                ResolutionMode::Advanced => {
                    let template = self
                        .chapter_repo
                        .get_game_rule_template(
                            novel_id,
                            player.rules.canon_model_version.ok_or_else(|| {
                                NarrativeError::Internal(anyhow::anyhow!(
                                    "advanced player template version is missing"
                                ))
                            })?,
                            user_id,
                        )
                        .await
                        .map_err(NarrativeError::Unavailable)?
                        .ok_or_else(|| {
                            NarrativeError::Conflict(
                                "The player's game rule template is unavailable".into(),
                            )
                        })?;
                    player
                        .rules
                        .validate_against(&template)
                        .map_err(|error| NarrativeError::Conflict(error.to_string()))?;
                    Some(template)
                }
            };
            self.require_self_reader_identity(user_id, novel_id).await?;
            self.require_world_source_visible(user_id, novel_id, &world_state)
                .await?;
            self.require_self_reader_identity(user_id, novel_id).await?;
            return Ok(PlayerEntry {
                checkpoint_chapter: player.canonical_checkpoint_chapter,
                player: Some(player),
                locations: Vec::new(),
                game_rules,
            });
        }
        let context = self
            .chapter_repo
            .get_player_entry_context(novel_id, user_id, checkpoint_chapter, None)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        self.require_source_chapter_visible(user_id, novel_id, context.checkpoint_chapter)
            .await?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        Ok(PlayerEntry {
            player: None,
            checkpoint_chapter: context.checkpoint_chapter,
            locations: context.locations,
            game_rules: None,
        })
    }

    #[tracing::instrument(skip(self))]
    pub async fn request_game_rule_template(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<GameRuleTemplate> {
        self.owned_novel(novel_id, user_id).await?;
        self.chapter_repo
            .request_game_rule_template(novel_id, user_id)
            .await
            .map_err(|error| match error {
                GameRuleTemplateRequestError::InProgress {
                    retry_after_seconds,
                } => NarrativeError::GameRulesInProgress {
                    retry_after_seconds,
                },
                GameRuleTemplateRequestError::Exhausted => NarrativeError::GameRulesExhausted,
                GameRuleTemplateRequestError::Unavailable(error) => {
                    NarrativeError::Unavailable(error)
                }
            })
    }

    #[tracing::instrument(skip(self, command))]
    pub async fn create_player_entity(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        command: CreatePlayerEntityCommand,
    ) -> NarrativeResult<PlayerEntity> {
        self.owned_novel(novel_id, user_id).await?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        if command
            .checkpoint_chapter
            .is_some_and(|chapter| chapter < 1)
        {
            return Err(NarrativeError::Validation(
                "Player checkpoint must be a positive chapter".into(),
            ));
        }
        PlayerEntity::validate_definition(
            &command.name,
            &command.background,
            &command.capabilities,
            &command.location_id,
            &command.inventory,
        )
        .map_err(|error| NarrativeError::Validation(error.to_string()))?;
        command
            .rules
            .validate()
            .map_err(|error| NarrativeError::Validation(error.to_string()))?;
        let _validated_game_rules = match command.rules.mode {
            ResolutionMode::Narrative => None,
            ResolutionMode::Advanced => {
                let version = command.rules.canon_model_version.ok_or_else(|| {
                    NarrativeError::Validation("Advanced template version is required".into())
                })?;
                let template = self
                    .chapter_repo
                    .get_game_rule_template(novel_id, version, user_id)
                    .await
                    .map_err(NarrativeError::Unavailable)?
                    .ok_or_else(|| {
                        NarrativeError::Validation(
                            "Game rule template is unavailable at current progress".into(),
                        )
                    })?;
                command
                    .rules
                    .validate_against(&template)
                    .map_err(|error| NarrativeError::Validation(error.to_string()))?;
                Some(template)
            }
        };
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        if let Some(existing) = world_state
            .player_entity()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
        {
            let checkpoint_matches = command
                .checkpoint_chapter
                .is_none_or(|chapter| chapter == existing.canonical_checkpoint_chapter);
            if !checkpoint_matches
                || !existing.matches_definition(
                    &command.name,
                    &command.background,
                    &command.capabilities,
                    &command.location_id,
                    &command.inventory,
                )
                || !existing.matches_rules(&command.rules)
            {
                return Err(NarrativeError::Conflict(
                    "PlayerEntity already exists with a different definition".into(),
                ));
            }
            self.require_self_reader_identity(user_id, novel_id).await?;
            let stored = self
                .world_state_repo
                .create_player_entity(&existing)
                .await
                .map_err(map_world_state_write_error)?;
            self.require_world_source_visible(user_id, novel_id, &world_state)
                .await?;
            self.require_self_reader_identity(user_id, novel_id).await?;
            return Ok(stored);
        }
        let context = self
            .chapter_repo
            .get_player_entry_context(
                novel_id,
                user_id,
                command.checkpoint_chapter,
                Some(&command.name),
            )
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)?;
        if !context.name_available {
            return Err(NarrativeError::Validation(
                "Player name conflicts with a canonical character".into(),
            ));
        }
        if !context
            .locations
            .iter()
            .any(|location| location.id == command.location_id)
        {
            return Err(NarrativeError::Validation(
                "Player location is not visible at the unlocked checkpoint".into(),
            ));
        }
        world_state
            .validate_world_entry_checkpoint(context.checkpoint_chapter)
            .map_err(|error| NarrativeError::Conflict(error.to_string()))?;
        let candidate = PlayerEntity::new_with_rules(
            user_id,
            novel_id,
            context.checkpoint_chapter,
            command.name,
            command.background,
            command.capabilities,
            command.location_id,
            command.inventory,
            command.rules,
        )
        .map_err(|error| NarrativeError::Validation(error.to_string()))?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        let stored = self
            .world_state_repo
            .create_player_entity(&candidate)
            .await
            .map_err(map_world_state_write_error)?;
        if stored.canonical_checkpoint_chapter != candidate.canonical_checkpoint_chapter
            || !stored.matches_definition(
                &candidate.name,
                &candidate.background,
                &candidate.capabilities,
                &candidate.location_id,
                &candidate.inventory,
            )
            || !stored.matches_rules(&candidate.rules)
        {
            return Err(NarrativeError::Conflict(
                "PlayerEntity was concurrently created with a different definition".into(),
            ));
        }
        let committed_world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        self.require_world_source_visible(user_id, novel_id, &committed_world_state)
            .await?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        Ok(stored)
    }

    #[tracing::instrument(skip(self))]
    pub async fn start_open_world(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<OpenWorldView> {
        self.owned_novel(novel_id, user_id).await?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        let state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let player = state
            .player_entity()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .ok_or_else(|| {
                NarrativeError::Conflict("Create PlayerEntity before entering the world".into())
            })?;
        state
            .validate_world_entry_checkpoint(player.canonical_checkpoint_chapter)
            .map_err(|error| NarrativeError::Conflict(error.to_string()))?;
        if let Some(session) = state
            .open_world()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
        {
            self.require_self_reader_identity(user_id, novel_id).await?;
            let state = self
                .world_state_repo
                .start_open_world(
                    user_id,
                    novel_id,
                    &session.entry_context,
                    session.game_rules.as_ref(),
                )
                .await
                .map_err(map_world_state_write_error)?;
            return self.open_world_view(user_id, novel_id, state).await;
        }
        let game_rules = match player.rules.mode {
            ResolutionMode::Narrative => None,
            ResolutionMode::Advanced => {
                let version = player.rules.canon_model_version.ok_or_else(|| {
                    NarrativeError::Internal(anyhow::anyhow!(
                        "advanced player template version is missing"
                    ))
                })?;
                let template = self
                    .chapter_repo
                    .get_game_rule_template(novel_id, version, user_id)
                    .await
                    .map_err(NarrativeError::Unavailable)?
                    .ok_or_else(|| {
                        NarrativeError::Conflict(
                            "The player's game rule template is unavailable".into(),
                        )
                    })?;
                player
                    .rules
                    .validate_against(&template)
                    .map_err(|error| NarrativeError::Conflict(error.to_string()))?;
                Some(template)
            }
        };
        let context = self
            .chapter_repo
            .get_world_entry_context(novel_id, player.canonical_checkpoint_chapter, user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)?;
        if let Some(template) = &game_rules {
            if template.canon_model_version != context.model_version {
                return Err(NarrativeError::Conflict(
                    "Game rule template no longer matches the world entry canon".into(),
                ));
            }
        }
        self.require_self_reader_identity(user_id, novel_id).await?;
        let state = self
            .world_state_repo
            .start_open_world(user_id, novel_id, &context, game_rules.as_ref())
            .await
            .map_err(map_world_state_write_error)?;
        self.open_world_view(user_id, novel_id, state).await
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_open_world(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<OpenWorldView> {
        self.owned_novel(novel_id, user_id).await?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        let state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        self.open_world_view(user_id, novel_id, state).await
    }

    pub async fn get_character_world_context(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        character_id: Uuid,
    ) -> NarrativeResult<Option<CharacterWorldContext>> {
        self.owned_novel(novel_id, user_id).await?;
        if !self.reader_identity_is_self(user_id, novel_id).await? {
            return Ok(None);
        }
        let state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let Some(mut context) = state
            .character_world_context(character_id)
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
        else {
            return Ok(None);
        };
        let journal = self
            .world_turn_repo
            .journal(user_id, novel_id, MAX_WORLD_JOURNAL_ENTRIES)
            .await
            .map_err(NarrativeError::Internal)?;
        context.recent_actions =
            recent_character_actions(journal, character_id, context.turn_number);
        if !self.reader_identity_is_self(user_id, novel_id).await? {
            return Ok(None);
        }
        Ok(Some(fit_character_world_context(context)))
    }

    async fn open_world_view(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        world_state: WorldState,
    ) -> NarrativeResult<OpenWorldView> {
        let player = world_state
            .player_entity()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .ok_or(NarrativeError::NotFound)?;
        let session = world_state
            .open_world()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .ok_or(NarrativeError::NotFound)?;
        self.require_world_source_visible(user_id, novel_id, &world_state)
            .await?;
        let journal = self
            .world_turn_repo
            .journal(user_id, novel_id, MAX_WORLD_JOURNAL_ENTRIES)
            .await
            .map_err(NarrativeError::Internal)?
            .into_iter()
            .filter(|entry| entry.turn_number <= session.turn_number)
            .collect();
        self.require_self_reader_identity(user_id, novel_id).await?;
        self.require_world_source_visible(user_id, novel_id, &world_state)
            .await?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        Ok(OpenWorldView {
            player,
            session,
            world_state,
            journal,
        })
    }

    /// A successful response implies that the eligible permanent fact reached
    /// Agent. Ambiguous post-commit projection remains pending for the same-key
    /// replay path and the bounded recovery scan.
    async fn project_journey_memory(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        result: &WorldTurnResult,
    ) -> NarrativeResult<MemoryProjectionStatus> {
        match record_world_journey_memory(
            self.agent_memory.as_ref(),
            self.chapter_repo.as_ref(),
            journey_memory_id(result.turn_id),
            user_id,
            novel_id,
            result,
        )
        .await
        {
            Ok(true) => Ok(MemoryProjectionStatus::Saved),
            Ok(false) => {
                warn!(
                    turn_id = %result.turn_id,
                    %novel_id,
                    "committed world turn has no protagonist memory projection"
                );
                Ok(MemoryProjectionStatus::Skipped)
            }
            Err(error) => {
                warn!(
                    %error,
                    turn_id = %result.turn_id,
                    "committed world turn memory projection is ambiguous"
                );
                Err(NarrativeError::TurnOutcomeUnknown)
            }
        }
    }

    async fn finish_journey_memory_projection(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        result: &WorldTurnResult,
    ) -> NarrativeResult<MemoryProjectionStatus> {
        self.require_memory_projection_eligible_snapshot(user_id, novel_id, &result.world_state)
            .await?;
        let status = self
            .project_journey_memory(user_id, novel_id, result)
            .await?;
        self.require_memory_projection_eligible_snapshot(user_id, novel_id, &result.world_state)
            .await?;
        match self
            .world_turn_repo
            .finish_memory_projection(result.turn_id, user_id, novel_id, status)
            .await
        {
            Ok(true) => Ok(status),
            Ok(false) => {
                warn!(
                    turn_id = %result.turn_id,
                    ?status,
                    "world turn memory projection terminal state conflicted"
                );
                Err(NarrativeError::TurnOutcomeUnknown)
            }
            Err(error) => {
                warn!(
                    %error,
                    turn_id = %result.turn_id,
                    ?status,
                    "world turn memory projection acknowledgement is ambiguous"
                );
                Err(NarrativeError::TurnOutcomeUnknown)
            }
        }
    }

    pub(crate) async fn reconcile_pending_memory_projections_once(&self) -> Result<usize> {
        let candidates = self
            .world_turn_repo
            .rotate_pending_memory_projections(MEMORY_PROJECTION_RECOVERY_BATCH)
            .await?;
        let mut reconciled = 0;
        for result in candidates {
            let turn_id = result.turn_id;
            let user_id = result.world_state.user_id;
            let novel_id = result.world_state.novel_id;
            match self
                .finish_journey_memory_projection(user_id, novel_id, &result)
                .await
            {
                Ok(status) => {
                    reconciled += 1;
                    info!(
                        %turn_id,
                        %novel_id,
                        ?status,
                        "reconciled pending world turn memory projection"
                    );
                }
                Err(error) => warn!(
                    %turn_id,
                    %novel_id,
                    %error,
                    "pending world turn memory projection remains unresolved"
                ),
            }
        }
        Ok(reconciled)
    }

    pub fn spawn_memory_projection_recovery(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let handler = self.clone();
        let current_span = tracing::Span::current();
        tokio::spawn(
            async move {
                let mut interval = tokio::time::interval(MEMORY_PROJECTION_RECOVERY_INTERVAL);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    interval.tick().await;
                    if let Err(error) = handler.reconcile_pending_memory_projections_once().await {
                        warn!(
                            error = ?error,
                            "pending world turn memory projection scan failed"
                        );
                    }
                }
            }
            .instrument(current_span),
        )
    }

    #[tracing::instrument(skip(self, action), fields(turn_id = %turn_id))]
    pub async fn submit_world_turn(
        &self,
        turn_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        expected_turn_number: i64,
        action: WorldAction,
    ) -> NarrativeResult<WorldTurnResponse> {
        if expected_turn_number < 0 {
            return Err(NarrativeError::Validation(
                "expected_turn_number must be non-negative".into(),
            ));
        }
        let novel = self.owned_novel(novel_id, user_id).await?;
        self.require_self_reader_identity(user_id, novel_id)
            .await
            .map_err(|_| NarrativeError::TurnOutcomeUnknown)?;
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let player = world_state
            .player_entity()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .ok_or(NarrativeError::NotFound)?;
        let session = world_state
            .open_world()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .ok_or(NarrativeError::NotFound)?;
        self.require_world_source_visible(user_id, novel_id, &world_state)
            .await?;
        let request = serde_json::to_vec(&(expected_turn_number, &action))
            .map_err(|error| NarrativeError::Internal(error.into()))?;
        let request_fingerprint: [u8; 32] = Sha256::digest(&request).into();
        let resolution = match player.rules.mode {
            ResolutionMode::Narrative => None,
            ResolutionMode::Advanced => {
                let template = session.game_rules.as_ref().ok_or_else(|| {
                    NarrativeError::Conflict(
                        "Advanced game rules are missing from the session".into(),
                    )
                })?;
                let roll = self.dice_roller.roll_d20(
                    user_id,
                    novel_id,
                    expected_turn_number,
                    &request_fingerprint,
                );
                Some(
                    resolve_action_check(template, &player.rules, action.kind, roll)
                        .map_err(|error| NarrativeError::Validation(error.to_string()))?,
                )
            }
        };
        let claim = WorldTurnClaim {
            id: turn_id,
            user_id,
            novel_id,
            request_fingerprint: request_fingerprint.to_vec(),
            action,
            resolution,
            expected_turn_number,
        };
        let (claim, attempt) = match self
            .world_turn_repo
            .begin_turn(&claim)
            .await
            .map_err(map_world_state_write_error)?
        {
            BeginWorldTurn::Acquired { claim, attempt } => (*claim, attempt),
            BeginWorldTurn::Completed {
                result,
                memory_projection,
            } => {
                let result = *result;
                let memory_projection_status = if memory_projection.is_terminal() {
                    memory_projection
                } else {
                    // Anchor at the COMMITTED turn's checkpoint: the live
                    // session may have advanced since the original commit.
                    self.finish_journey_memory_projection(user_id, novel_id, &result)
                        .await?
                };
                self.require_memory_projection_eligible_snapshot(
                    user_id,
                    novel_id,
                    &result.world_state,
                )
                .await
                .map_err(|_| NarrativeError::TurnOutcomeUnknown)?;
                return Ok(WorldTurnResponse {
                    result,
                    memory_projection_status,
                });
            }
            BeginWorldTurn::InProgress {
                retry_after_seconds,
            } => {
                return Err(NarrativeError::TurnInProgress {
                    retry_after_seconds,
                })
            }
            BeginWorldTurn::Conflict => {
                return Err(NarrativeError::Conflict(
                    "Idempotency-Key conflicts with an existing world turn".into(),
                ))
            }
            BeginWorldTurn::Stale => {
                return Err(NarrativeError::Conflict(
                    "World state advanced; reload before submitting this action".into(),
                ))
            }
        };
        if session.turn_number != claim.expected_turn_number {
            self.fail_world_turn(&claim, attempt, "state_snapshot_stale")
                .await;
            return Err(NarrativeError::Conflict(
                "World state advanced; reload before submitting this action".into(),
            ));
        }
        if let Err(error) = world_state.validate_world_action(&claim.action, &session.entry_context)
        {
            self.fail_world_turn(&claim, attempt, "validation_error")
                .await;
            return Err(match error {
                WorldStateError::TimelineConflict(message) => NarrativeError::Conflict(message),
                error => NarrativeError::Validation(error.to_string()),
            });
        }

        let recent_turns = match self
            .world_turn_repo
            .journal(user_id, novel_id, MAX_RECENT_WORLD_TURNS)
            .await
        {
            Ok(turns) => turns
                .into_iter()
                .map(|turn| RecentWorldTurnContext {
                    turn_number: turn.turn_number,
                    action: turn.action,
                    rendered_narrative: trailing_chars(
                        &turn.transition.rendered_narrative,
                        MAX_RECENT_WORLD_NARRATIVE_CHARS,
                    ),
                })
                .collect::<Vec<_>>(),
            Err(error) => {
                self.fail_world_turn(&claim, attempt, "history_error").await;
                return Err(NarrativeError::Internal(error));
            }
        };

        let mut lease = WorldTurnLease::start(self.world_turn_repo.clone(), claim.id, attempt);
        let prompt = match build_world_turn_prompt_with_check(
            &novel.title,
            &player,
            &claim.action,
            &session,
            &world_state.state,
            &recent_turns,
            claim.resolution.as_ref(),
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.fail_world_turn(&claim, attempt, "prompt_error").await;
                return Err(NarrativeError::Internal(anyhow::anyhow!(error)));
            }
        };
        if prompt.len() > MAX_WORLD_TURN_PROMPT_BYTES
            || prompt.chars().count() > MAX_WORLD_TURN_PROMPT_CHARS
        {
            self.fail_world_turn(&claim, attempt, "prompt_budget").await;
            return Err(NarrativeError::Internal(anyhow::anyhow!(
                "world turn prompt exceeded its budget"
            )));
        }

        let raw = match lease
            .run(
                self.llm
                    .chat_json(user_id, NarrativeLlmTask::NarrativeTransition, &prompt),
            )
            .await
        {
            Some(Ok(raw)) => raw,
            Some(Err(error)) => {
                self.fail_world_turn(&claim, attempt, "llm_error").await;
                return Err(NarrativeError::Llm(error));
            }
            None => {
                self.fail_world_turn(&claim, attempt, "lease_lost").await;
                return Err(NarrativeError::TurnOutcomeUnknown);
            }
        };
        if raw.len() > MAX_WORLD_TRANSITION_BYTES {
            self.fail_world_turn(&claim, attempt, "response_budget")
                .await;
            return Err(NarrativeError::Llm(anyhow::anyhow!(
                "world transition exceeded its budget"
            )));
        }
        let transition = match parse_world_turn_transition_with_check(
            &raw,
            &claim.action,
            &session.entry_context,
            &session,
            claim.resolution.as_ref(),
        ) {
            Ok(transition) => transition,
            Err(error) => {
                self.fail_world_turn(&claim, attempt, "invalid_transition")
                    .await;
                return Err(NarrativeError::Llm(anyhow::anyhow!(error)));
            }
        };
        if let Err(error) = self
            .require_world_source_visible(user_id, novel_id, &world_state)
            .await
        {
            self.fail_world_turn(&claim, attempt, "progress_rewind")
                .await;
            return Err(error);
        }
        if let Err(error) = self.require_self_reader_identity(user_id, novel_id).await {
            self.fail_world_turn(&claim, attempt, "identity_changed")
                .await;
            return Err(error);
        }
        let committed = match lease
            .run(self.world_turn_repo.complete_turn(
                &claim,
                attempt,
                &transition,
                &session.entry_context,
            ))
            .await
        {
            Some(Ok(result)) => result,
            Some(Err(error)) => {
                if matches!(
                    error.downcast_ref::<WorldStateError>(),
                    Some(WorldStateError::TimelineConflict(_))
                ) {
                    match self
                        .world_turn_repo
                        .fail_turn(claim.id, attempt, "commit_error")
                        .await
                    {
                        Ok(true) => return Err(map_world_state_write_error(error)),
                        Ok(false) => {
                            tracing::error!(
                                turn_id = %claim.id,
                                attempt,
                                "timeline conflict could not fence the world turn"
                            );
                        }
                        Err(fence_error) => {
                            tracing::error!(
                                turn_id = %claim.id,
                                attempt,
                                error = ?fence_error,
                                "failed to fence a timeline-conflicted world turn"
                            );
                        }
                    }
                    return Err(NarrativeError::TurnOutcomeUnknown);
                }
                self.fail_world_turn(&claim, attempt, "commit_error").await;
                return Err(map_world_state_write_error(error));
            }
            None => {
                self.fail_world_turn(&claim, attempt, "lease_lost").await;
                return Err(NarrativeError::TurnOutcomeUnknown);
            }
        };
        lease.stop();
        // Fresh-commit path: the authoritative turn is already durable. Keep
        // its exact key ambiguous until the eligible permanent fact is
        // acknowledged, so a retry heals a crash or dependency failure.
        let memory_projection_status = self
            .finish_journey_memory_projection(user_id, novel_id, &committed)
            .await?;
        self.require_memory_projection_eligible_snapshot(user_id, novel_id, &committed.world_state)
            .await
            .map_err(|_| NarrativeError::TurnOutcomeUnknown)?;
        Ok(WorldTurnResponse {
            result: committed,
            memory_projection_status,
        })
    }

    async fn fail_world_turn(
        &self,
        claim: &WorldTurnClaim,
        attempt: i64,
        failure_code: &'static str,
    ) {
        if let Err(error) = self
            .world_turn_repo
            .fail_turn(claim.id, attempt, failure_code)
            .await
        {
            tracing::error!(turn_id = %claim.id, attempt, error = ?error, "failed to record world turn failure");
        }
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_branch_node(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
        user_id: Uuid,
    ) -> NarrativeResult<Option<NarrativeNode>> {
        if chapter_number < 1 {
            return Err(NarrativeError::Validation(
                "chapter must be at least 1".into(),
            ));
        }
        let novel_info = self.owned_novel(novel_id, user_id).await?;
        self.require_source_chapter_visible(user_id, novel_id, chapter_number)
            .await?;
        if !self.reader_identity_is_self(user_id, novel_id).await? {
            let committed = self
                .committed_branch_node(user_id, novel_id, chapter_number)
                .await?;
            return self
                .branch_node_if_visible(user_id, novel_id, chapter_number, committed)
                .await;
        }
        let (world_state, player, _) = self.narrative_world_state(user_id, novel_id).await?;
        let chapter = self
            .resolve_chapter(user_id, novel_id, chapter_number, &novel_info)
            .await?;
        if let Some(existing_node) = self
            .committed_branch_node(user_id, novel_id, chapter_number)
            .await?
        {
            return self
                .branch_node_if_visible(user_id, novel_id, chapter_number, Some(existing_node))
                .await;
        }
        if !Self::uncommitted_branch_is_eligible(&world_state, chapter_number)? {
            // A committed choice above is replayable, but an uncommitted node
            // outside the frozen branch prefix must not spend an LLM call or
            // create an option the user can never submit.
            return Ok(None);
        }
        // Branch prompts include the PlayerEntity, private world state, and
        // deviation mode. Their output is therefore never safe to share
        // between users, even when the source chapter itself is canonical.
        let node_owner = Some(user_id);
        if let Some(node) = self
            .node_repo
            .find_by_chapter(novel_id, chapter_number, node_owner)
            .await
            .map_err(NarrativeError::Internal)?
        {
            return self
                .finalize_uncommitted_branch_node(user_id, novel_id, chapter_number, Some(node))
                .await;
        }
        if !chapter.canonical.is_key_node {
            return Ok(None);
        }
        let key_node_description = chapter
            .canonical
            .key_node_description
            .as_deref()
            .filter(|description| !description.trim().is_empty())
            .ok_or_else(|| {
                NarrativeError::Internal(anyhow::anyhow!(
                    "key chapter is missing its node description"
                ))
            })?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        let prompt = build_branch_prompt(
            &novel_info.title,
            &chapter.content,
            key_node_description,
            &world_state,
            &novel_info.deviation_mode,
            player.as_ref(),
        );
        if prompt.len() > MAX_NARRATIVE_PROMPT_BYTES
            || prompt.chars().count() > MAX_NARRATIVE_PROMPT_CHARS
        {
            return Err(NarrativeError::Internal(anyhow::anyhow!(
                "branch prompt exceeded its budget"
            )));
        }
        let generated = self
            .llm
            .chat_json(user_id, NarrativeLlmTask::BranchGeneration, &prompt)
            .await
            .map_err(NarrativeError::Llm)
            .and_then(|json| {
                parse_generated_branch(&json)
                    .map_err(|error| NarrativeError::Llm(anyhow::anyhow!(error)))
            })?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        if !chapter.content.contains(&generated.anchor_quote) {
            return Err(NarrativeError::Llm(anyhow::anyhow!(
                "generated branch anchor was not present in chapter source"
            )));
        }
        let node = NarrativeNode::new(
            novel_id,
            chapter_number,
            generated.description,
            generated
                .choices
                .into_iter()
                .enumerate()
                .map(|(index, choice)| NarrativeChoice {
                    index: index as i32,
                    text: choice.text,
                    hint: choice.hint,
                    generated_consequence: None,
                })
                .collect(),
        )
        .with_anchor_quote(generated.anchor_quote)
        .for_user(user_id);
        self.node_repo
            .save(&node)
            .await
            .map_err(NarrativeError::Internal)?;
        // Reload after the upsert so concurrent first readers receive the same
        // persisted node id.
        let node = self
            .node_repo
            .find_by_chapter(novel_id, chapter_number, node_owner)
            .await
            .map_err(NarrativeError::Internal)?;
        // Progress can be rewound while any repository/provider call is in
        // flight. A node may remain durable, but no future-derived option may
        // cross the response boundary until its own source chapter is visible.
        self.finalize_uncommitted_branch_node(user_id, novel_id, chapter_number, node)
            .await
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_effective_chapter(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
    ) -> NarrativeResult<EffectiveChapter> {
        if chapter_number < 1 {
            return Err(NarrativeError::Validation(
                "chapter must be at least 1".into(),
            ));
        }
        let novel_info = self.owned_novel(novel_id, user_id).await?;
        if let Err(error) = self
            .require_source_chapter_visible(user_id, novel_id, chapter_number)
            .await
        {
            if !matches!(&error, NarrativeError::ReadingProgressBehindWorld) {
                return Err(error);
            }
            let canonical = self
                .chapter_repo
                .get_chapter(novel_id, chapter_number, user_id)
                .await
                .map_err(NarrativeError::Unavailable)?
                .ok_or(NarrativeError::NotFound)?;
            return Ok(EffectiveChapter {
                chapter_number,
                content: canonical.content,
                generated: false,
            });
        }
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let world_state = if self.reader_identity_is_self(user_id, novel_id).await? {
            world_state
        } else {
            Self::choices_only_world_state(world_state)?
        };
        match self
            .require_world_source_visible(user_id, novel_id, &world_state)
            .await
        {
            Ok(()) => {}
            Err(NarrativeError::ReadingProgressBehindWorld) => {
                // Rewinding restores the immutable source view. Never return a
                // cached/generated PlayerChapter or invoke its continuation LLM
                // until the canonical source boundary has been reread.
                let canonical = self
                    .chapter_repo
                    .get_chapter(novel_id, chapter_number, user_id)
                    .await
                    .map_err(NarrativeError::Unavailable)?
                    .ok_or(NarrativeError::NotFound)?;
                return Ok(EffectiveChapter {
                    chapter_number,
                    content: canonical.content,
                    generated: false,
                });
            }
            Err(error) => return Err(error),
        }
        let chapter = match self
            .resolve_chapter(user_id, novel_id, chapter_number, &novel_info)
            .await
        {
            Ok(chapter) => chapter,
            Err(NarrativeError::ReadingProgressBehindWorld) => {
                let canonical = self
                    .chapter_repo
                    .get_chapter(novel_id, chapter_number, user_id)
                    .await
                    .map_err(NarrativeError::Unavailable)?
                    .ok_or(NarrativeError::NotFound)?;
                return Ok(EffectiveChapter {
                    chapter_number,
                    content: canonical.content,
                    generated: false,
                });
            }
            Err(error) => return Err(error),
        };
        let current_world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let current_world_state = if self.reader_identity_is_self(user_id, novel_id).await? {
            current_world_state
        } else {
            Self::choices_only_world_state(current_world_state)?
        };
        match self
            .require_world_source_visible(user_id, novel_id, &current_world_state)
            .await
        {
            Ok(()) => {}
            Err(NarrativeError::ReadingProgressBehindWorld) => {
                return Ok(EffectiveChapter {
                    chapter_number,
                    content: chapter.canonical.content,
                    generated: false,
                });
            }
            Err(error) => return Err(error),
        }
        match self
            .require_source_chapter_visible(user_id, novel_id, chapter_number)
            .await
        {
            Ok(()) => {}
            Err(NarrativeError::ReadingProgressBehindWorld) => {
                return Ok(EffectiveChapter {
                    chapter_number,
                    content: chapter.canonical.content,
                    generated: false,
                });
            }
            Err(error) => return Err(error),
        }
        if !self.reader_identity_is_self(user_id, novel_id).await?
            && chapter.origin == Some(PlayerChapterOrigin::Continuation)
        {
            return Err(NarrativeError::Conflict(
                "Original-player continuation requires the self reader identity".into(),
            ));
        }
        Ok(EffectiveChapter {
            chapter_number,
            content: chapter.content,
            generated: chapter.generated,
        })
    }

    #[tracing::instrument(skip(self))]
    pub async fn submit_choice(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        node_id: Uuid,
        requested_choice_index: i32,
    ) -> NarrativeResult<ChoiceResult> {
        let novel_info = self.owned_novel(novel_id, user_id).await?;
        let initial_self_identity = self.reader_identity_is_self(user_id, novel_id).await?;
        let (world_state, _, _) = self.narrative_world_state(user_id, novel_id).await?;
        let node = self
            .node_repo
            .find_by_id(node_id)
            .await
            .map_err(NarrativeError::Internal)?
            .filter(|node| node.novel_id == novel_id)
            .ok_or(NarrativeError::NotFound)?;
        let existing = self
            .choice_repo
            .find_user_choice(user_id, node_id)
            .await
            .map_err(NarrativeError::Internal)?;
        match node.user_id {
            Some(owner_id) if owner_id == user_id => {}
            // A legacy shared node is readable only as the durable source of
            // an already-committed choice. It can never start a new branch.
            None if existing.is_some() => {}
            Some(_) | None => return Err(NarrativeError::NotFound),
        }
        if existing.is_none() && !initial_self_identity {
            return Err(NarrativeError::Conflict(
                "New branch choices require the self reader identity".into(),
            ));
        }
        let open_world_started = world_state
            .open_world()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .is_some();
        let player_checkpoint = world_state
            .player_entity()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .map(|player| player.canonical_checkpoint_chapter);
        require_choice_submission_allowed(
            requested_choice_index,
            existing.as_ref().map(|choice| choice.choice_index),
            node.chapter_number,
            player_checkpoint,
            open_world_started,
        )?;
        if let Some(existing) = existing.as_ref() {
            if existing.user_id != user_id
                || existing.novel_id != novel_id
                || existing.node_id != node.id
                || existing.chapter_number != node.chapter_number
            {
                return Err(NarrativeError::Internal(anyhow::anyhow!(
                    "committed choice does not match its narrative node"
                )));
            }
            warn!(
                user_id = %user_id,
                node_id = %node_id,
                choice_index = existing.choice_index,
                "replaying an existing narrative choice"
            );
            let full_content = match self
                .player_chapter_repo
                .find(user_id, novel_id, node.chapter_number)
                .await
                .map_err(NarrativeError::Internal)?
            {
                Some(chapter) if chapter.origin == PlayerChapterOrigin::Choice => chapter.content,
                Some(_) | None => {
                    let chapter = self
                        .chapter_repo
                        .get_chapter(novel_id, node.chapter_number, user_id)
                        .await
                        .map_err(NarrativeError::Unavailable)?
                        .ok_or(NarrativeError::NotFound)?;
                    rewrite_after_anchor(
                        &chapter.content,
                        node.anchor_quote.as_deref(),
                        &existing.consequence,
                    )?
                }
            };
            return self
                .commit_result(choice_draft(
                    existing,
                    world_state.fingerprint(),
                    full_content,
                ))
                .await;
        }

        self.require_self_reader_identity(user_id, novel_id).await?;

        let choice_index = requested_choice_index;
        let choice_text = usize::try_from(choice_index)
            .ok()
            .and_then(|index| node.choices.get(index))
            .map(|choice| choice.text.clone())
            .ok_or_else(|| {
                NarrativeError::Validation("choice_index is outside the node choices".into())
            })?;
        let chapter = self
            .resolve_chapter(user_id, novel_id, node.chapter_number, &novel_info)
            .await?;
        let chapter_prefix =
            chapter_prefix_through_anchor(&chapter.content, node.anchor_quote.as_deref())?;
        let canon_context = self
            .chapter_repo
            .get_canon_context(novel_id, node.chapter_number, user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or_else(|| {
                NarrativeError::Internal(anyhow::anyhow!(
                    "ready novel has no canonical story context"
                ))
            })?;
        if canon_context.checkpoint_chapter != node.chapter_number {
            return Err(NarrativeError::Validation(
                "chapter is ahead of the reader's progress".into(),
            ));
        }
        self.require_self_reader_identity(user_id, novel_id).await?;
        let prompt = build_transition_prompt(
            &novel_info.title,
            &choice_text,
            chapter_prefix,
            &world_state,
            &novel_info.deviation_mode,
            &canon_context,
        );
        if prompt.len() > MAX_NARRATIVE_PROMPT_BYTES
            || prompt.chars().count() > MAX_NARRATIVE_PROMPT_CHARS
        {
            return Err(NarrativeError::Internal(anyhow::anyhow!(
                "narrative prompt exceeded its budget"
            )));
        }

        info!(
            user_id = %user_id,
            node_id = %node_id,
            choice_index,
            "generating narrative consequence"
        );
        // Live-provider robustness: the transition LLM can drift (e.g. a
        // hallucinated or spoiling event actor), which the strict validator
        // fail-closed. Retry the stochastic call up to 3 times before giving up;
        // the strict gate (incl. spoiler/future/dead-actor protection) stays
        // intact — a spoiler is never silently accepted.
        let mut transition = None;
        let mut last_error = None;
        for attempt in 0..3 {
            let raw_transition = self
                .llm
                .chat_json(user_id, NarrativeLlmTask::NarrativeTransition, &prompt)
                .await
                .map_err(NarrativeError::Llm)?;
            if raw_transition.len() > MAX_TRANSITION_BYTES {
                return Err(NarrativeError::Llm(anyhow::anyhow!(
                    "language model transition exceeded the limit"
                )));
            }
            match parse_transition(&raw_transition, &canon_context) {
                Ok(parsed) => {
                    transition = Some(parsed);
                    break;
                }
                Err(error) => {
                    if attempt < 2 {
                        tracing::debug!(%error, attempt, "narrative transition failed validation; retrying");
                    }
                    last_error = Some(error);
                }
            }
        }
        let transition = transition.ok_or_else(|| {
            NarrativeError::Llm(anyhow::anyhow!(
                "narrative transition failed validation after 3 attempts: {:?}",
                last_error
            ))
        })?;
        let consequence = transition.rendered_narrative.clone();
        if consequence.is_empty()
            || consequence.len() > MAX_CONSEQUENCE_BYTES
            || consequence.chars().count() > MAX_CONSEQUENCE_CHARS
            || !is_chinese_narrative(&consequence)
        {
            return Err(NarrativeError::Llm(anyhow::anyhow!(
                "language model consequence was empty or exceeded the limit"
            )));
        }

        let rewritten_chapter_content = format!(
            "{}\n\n{}",
            chapter_prefix.trim_end(),
            consequence.trim_start()
        );
        self.require_self_reader_identity(user_id, novel_id).await?;
        self.commit_result(ChoiceCommit {
            user_id,
            novel_id,
            node_id,
            chapter_number: node.chapter_number,
            choice_index,
            choice_text,
            expected_world_state_fingerprint: world_state.fingerprint(),
            transition,
            rewritten_chapter_content,
        })
        .await
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_world_state(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<WorldState> {
        self.owned_novel(novel_id, user_id).await?;
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let mut world_state = if self.reader_identity_is_self(user_id, novel_id).await? {
            world_state
        } else {
            Self::choices_only_world_state(world_state)?
        };
        self.require_world_source_visible(user_id, novel_id, &world_state)
            .await?;
        if !self.reader_identity_is_self(user_id, novel_id).await? {
            world_state = Self::choices_only_world_state(world_state)?;
            self.require_world_source_visible(user_id, novel_id, &world_state)
                .await?;
        }
        Ok(world_state)
    }

    async fn commit_result(&self, draft: ChoiceCommit) -> NarrativeResult<ChoiceResult> {
        let requested_choice_index = draft.choice_index;
        let committed = self
            .choice_repo
            .commit_choice(&draft)
            .await
            .map_err(map_world_state_write_error)?;
        require_same_choice(requested_choice_index, committed.choice.choice_index)?;
        let mut response_world_state = if self
            .reader_identity_is_self(draft.user_id, draft.novel_id)
            .await?
        {
            committed.world_state
        } else {
            Self::choices_only_world_state(committed.world_state)?
        };
        // The choice commit is authoritative even if another tab rewinds
        // progress during generation. Recheck before returning any derived
        // prose/state; same-choice replay becomes visible after rereading.
        self.require_world_source_visible(draft.user_id, draft.novel_id, &response_world_state)
            .await?;
        if !self
            .reader_identity_is_self(draft.user_id, draft.novel_id)
            .await?
        {
            response_world_state = Self::choices_only_world_state(response_world_state)?;
            self.require_world_source_visible(draft.user_id, draft.novel_id, &response_world_state)
                .await?;
        }
        Ok(ChoiceResult {
            chapter_number: committed.choice.chapter_number,
            consequence: committed.choice.consequence.clone(),
            transition: committed.choice.transition,
            chapter_content: committed.player_chapter_content,
            world_state: response_world_state,
        })
    }

    async fn resolve_chapter(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        novel_info: &NovelInfo,
    ) -> NarrativeResult<ResolvedChapter> {
        let canonical = self
            .chapter_repo
            .get_chapter(novel_id, chapter_number, user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)?;
        let self_identity = self.reader_identity_is_self(user_id, novel_id).await?;
        if let Some(player_chapter) = self
            .player_chapter_repo
            .find(user_id, novel_id, chapter_number)
            .await
            .map_err(NarrativeError::Internal)?
        {
            if player_chapter.origin == PlayerChapterOrigin::Continuation {
                self.require_self_reader_identity(user_id, novel_id).await?;
            }
            return Ok(ResolvedChapter {
                canonical,
                content: player_chapter.content,
                generated: true,
                origin: Some(player_chapter.origin),
            });
        }

        let committed_choices = self
            .choice_repo
            .find_by_novel(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?
            .into_iter()
            .filter(|choice| {
                choice.chapter_number <= chapter_number && !choice.consequence.trim().is_empty()
            })
            .collect::<Vec<_>>();
        if committed_choices.is_empty() {
            return Ok(ResolvedChapter {
                content: canonical.content.clone(),
                canonical,
                generated: false,
                origin: None,
            });
        }

        if !self_identity {
            if let Some(choice) = committed_choices
                .iter()
                .find(|choice| choice.chapter_number == chapter_number)
            {
                let chapter = self.reconstruct_choice_chapter(choice).await?;
                return Ok(ResolvedChapter {
                    canonical,
                    content: chapter.content,
                    generated: true,
                    origin: Some(PlayerChapterOrigin::Choice),
                });
            }
            return Err(NarrativeError::Conflict(
                "Original-player continuation requires the self reader identity".into(),
            ));
        }

        let mut previous = self
            .player_chapter_repo
            .find_latest_before(user_id, novel_id, chapter_number)
            .await
            .map_err(NarrativeError::Internal)?;
        if previous.is_none() {
            let first_choice = committed_choices
                .iter()
                .min_by_key(|choice| choice.chapter_number)
                .expect("committed choices are non-empty");
            previous = Some(self.reconstruct_choice_chapter(first_choice).await?);
        }

        let mut previous = previous.expect("divergence chapter was reconstructed");
        if previous.chapter_number == chapter_number {
            return Ok(ResolvedChapter {
                canonical,
                content: previous.content,
                generated: true,
                origin: Some(previous.origin),
            });
        }

        if let Some(choice) = committed_choices
            .iter()
            .find(|choice| choice.chapter_number == chapter_number)
        {
            let chapter = self.reconstruct_choice_chapter(choice).await?;
            return Ok(ResolvedChapter {
                canonical,
                content: chapter.content,
                generated: true,
                origin: Some(chapter.origin),
            });
        }
        if !is_next_chapter(previous.chapter_number, chapter_number) {
            if let Some(choice) = committed_choices
                .iter()
                .find(|choice| choice.chapter_number == chapter_number - 1)
            {
                previous = self.reconstruct_choice_chapter(choice).await?;
            }
        }
        if !is_next_chapter(previous.chapter_number, chapter_number) {
            return Err(NarrativeError::Validation(
                "Request the next effective chapter before advancing further".into(),
            ));
        }

        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        previous = self
            .generate_player_chapter(
                user_id,
                novel_id,
                chapter_number,
                &previous.content,
                &canonical.content,
                novel_info,
                &world_state,
            )
            .await?;
        Ok(ResolvedChapter {
            canonical,
            content: previous.content,
            generated: true,
            origin: Some(previous.origin),
        })
    }

    async fn reconstruct_choice_chapter(
        &self,
        choice: &UserChoiceRecord,
    ) -> NarrativeResult<PlayerChapter> {
        let source = self
            .chapter_repo
            .get_chapter(choice.novel_id, choice.chapter_number, choice.user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)?;
        let node = self
            .node_repo
            .find_by_id(choice.node_id)
            .await
            .map_err(NarrativeError::Internal)?
            .filter(|node| {
                node.novel_id == choice.novel_id
                    && node
                        .user_id
                        .is_none_or(|owner_id| owner_id == choice.user_id)
            })
            .ok_or(NarrativeError::NotFound)?;
        let content = rewrite_after_anchor(
            &source.content,
            node.anchor_quote.as_deref(),
            &choice.consequence,
        )?;
        self.player_chapter_repo
            .save_if_absent(&PlayerChapter {
                user_id: choice.user_id,
                novel_id: choice.novel_id,
                chapter_number: choice.chapter_number,
                content,
                origin: PlayerChapterOrigin::Choice,
                created_at: choice.created_at,
            })
            .await
            .map_err(NarrativeError::Internal)
    }

    #[allow(clippy::too_many_arguments)]
    async fn generate_player_chapter(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        previous_content: &str,
        canonical_content: &str,
        novel_info: &NovelInfo,
        world_state: &WorldState,
    ) -> NarrativeResult<PlayerChapter> {
        self.require_self_reader_identity(user_id, novel_id).await?;
        self.require_source_chapter_visible(user_id, novel_id, chapter_number)
            .await?;
        let prompt = build_player_chapter_prompt(
            &novel_info.title,
            chapter_number,
            previous_content,
            canonical_content,
            novel_info.world_summary.as_deref(),
            world_state,
            &novel_info.deviation_mode,
        );
        if prompt.len() > MAX_NARRATIVE_PROMPT_BYTES
            || prompt.chars().count() > MAX_NARRATIVE_PROMPT_CHARS
        {
            return Err(NarrativeError::Internal(anyhow::anyhow!(
                "player chapter prompt exceeded its budget"
            )));
        }
        info!(
            user_id = %user_id,
            novel_id = %novel_id,
            chapter_number,
            "generating complete player timeline chapter"
        );
        let content = self
            .llm
            .chat_longform(
                user_id,
                "你是互动小说的玩家时间线主笔。只输出自然的简体中文小说正文。",
                &prompt,
            )
            .await
            .map_err(NarrativeError::Llm)?
            .trim()
            .to_owned();
        validate_player_chapter(&content)?;
        self.require_self_reader_identity(user_id, novel_id).await?;
        self.require_source_chapter_visible(user_id, novel_id, chapter_number)
            .await?;
        self.player_chapter_repo
            .save_if_absent(&PlayerChapter {
                user_id,
                novel_id,
                chapter_number,
                content,
                origin: PlayerChapterOrigin::Continuation,
                created_at: Utc::now(),
            })
            .await
            .map_err(NarrativeError::Internal)
    }
}

fn is_next_chapter(previous: i32, requested: i32) -> bool {
    previous.checked_add(1) == Some(requested)
}

fn choice_draft(
    existing: &UserChoiceRecord,
    expected_world_state_fingerprint: [u8; 32],
    rewritten_chapter_content: String,
) -> ChoiceCommit {
    ChoiceCommit {
        user_id: existing.user_id,
        novel_id: existing.novel_id,
        node_id: existing.node_id,
        chapter_number: existing.chapter_number,
        choice_index: existing.choice_index,
        choice_text: existing.choice_text.clone(),
        expected_world_state_fingerprint,
        transition: existing.transition.clone(),
        rewritten_chapter_content,
    }
}

fn chapter_prefix_through_anchor<'a>(
    content: &'a str,
    anchor_quote: Option<&str>,
) -> NarrativeResult<&'a str> {
    let anchor = anchor_quote
        .filter(|value| !value.is_empty())
        .ok_or_else(|| NarrativeError::Validation("narrative node has no source anchor".into()))?;
    let start = content.find(anchor).ok_or_else(|| {
        NarrativeError::Validation("narrative anchor is not present in this timeline".into())
    })?;
    Ok(&content[..start + anchor.len()])
}

fn rewrite_after_anchor(
    content: &str,
    anchor_quote: Option<&str>,
    consequence: &str,
) -> NarrativeResult<String> {
    let prefix = chapter_prefix_through_anchor(content, anchor_quote)?;
    Ok(format!(
        "{}\n\n{}",
        prefix.trim_end(),
        consequence.trim_start()
    ))
}

fn validate_player_chapter(content: &str) -> NarrativeResult<()> {
    if content.is_empty()
        || content.len() > MAX_PLAYER_CHAPTER_BYTES
        || content.chars().count() > MAX_PLAYER_CHAPTER_CHARS
        || !is_chinese_narrative(content)
    {
        return Err(NarrativeError::Llm(anyhow::anyhow!(
            "generated player chapter was empty, non-Chinese, or exceeded the limit"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod timeline_tests {
    use super::*;

    fn journal_entry(turn_number: i64, target_character_id: Uuid) -> WorldTurnJournalEntry {
        let now = Utc::now();
        WorldTurnJournalEntry {
            turn_id: Uuid::new_v4(),
            turn_number,
            memory_projection_status: MemoryProjectionStatus::Saved,
            action: WorldAction {
                kind: WorldActionKind::Converse,
                target_id: Some(target_character_id.to_string()),
                intent: format!("调查第 {turn_number} 回合"),
            },
            resolution: None,
            transition: crate::domain::entities::world_session::WorldTurnTransition {
                schema_version: 1,
                prompt_version: "world-turn-v2".into(),
                canon_model_version: 1,
                canonical_checkpoint_chapter: 1,
                rendered_narrative: format!("第 {turn_number} 回合"),
                events: vec![],
                relationship_changes: vec![],
                location_changes: vec![],
                thread_changes: vec![],
                player_location_id: None,
                inventory_additions: vec![],
                inventory_removals: vec![],
                knowledge_discoveries: vec![],
                faction_changes: vec![],
                canonical_event_change: None,
            },
            created_at: now,
            completed_at: now,
        }
    }

    fn assert_consumer_compatible_journey_fact(encoded: &str, character_id: Uuid) {
        assert!(encoded.chars().count() <= MAX_JOURNEY_MEMORY_EVENT_CHARS);
        let fact: serde_json::Value = serde_json::from_str(encoded).unwrap();
        assert_eq!(fact["schema_version"], 2);
        assert_eq!(fact["source"], "committed_world_turn");
        assert_eq!(fact["authority"], "explicit_character_witness_facts");
        assert_eq!(fact["witness_character_id"], character_id.to_string());
        let changes = fact["committed_changes"].as_object().unwrap();
        let events = changes
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let relationships = changes
            .get("relationships")
            .and_then(serde_json::Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        assert!(events.len() <= 4);
        assert!(relationships.len() <= 4);
        for event in events {
            let summary = event["summary"].as_str().unwrap();
            assert!(!summary.is_empty());
            assert_eq!(summary.trim(), summary);
            assert!(summary.chars().count() <= MAX_JOURNEY_MEMORY_FIELD_CHARS);
            assert_eq!(
                event["actor_character_ids"],
                serde_json::json!([character_id])
            );
            if let Some(location) = event["location_id"].as_str() {
                assert_eq!(location.trim(), location);
                assert!(location.chars().count() <= MAX_JOURNEY_MEMORY_LOCATION_CHARS);
                assert!(!location.chars().any(char::is_control));
            }
        }
        for relationship in relationships {
            assert_eq!(relationship["character_id"], character_id.to_string());
            assert!(relationship["delta"]
                .as_i64()
                .is_some_and(|delta| { delta != 0 && (-20..=20).contains(&delta) }));
        }
        let reader_action = fact.get("reader_action");
        if let Some(action) = reader_action {
            assert_eq!(action.as_object().unwrap().len(), 2);
            assert!(matches!(
                action["kind"].as_str().unwrap(),
                "converse" | "ally" | "oppose"
            ));
            assert_eq!(action["target_id"], character_id.to_string());
            assert!(action.get("intent").is_none());
        }
        assert_eq!(fact["change_counts"]["events"], events.len());
        assert_eq!(fact["change_counts"]["relationships"], relationships.len());
        assert_eq!(
            fact["change_counts"]["reader_action"],
            usize::from(reader_action.is_some())
        );
        assert!(reader_action.is_some() || !events.is_empty() || !relationships.is_empty());
    }

    #[test]
    fn typed_timeline_write_errors_map_to_conflicts() {
        let mapped = map_world_state_write_error(
            WorldStateError::TimelineConflict("timeline advanced".into()).into(),
        );
        assert!(matches!(
            mapped,
            NarrativeError::Conflict(message) if message == "timeline advanced"
        ));

        let mapped = map_world_state_write_error(anyhow::anyhow!("database unavailable"));
        assert!(matches!(mapped, NarrativeError::Internal(_)));
    }

    #[test]
    fn choice_replaces_everything_after_the_anchor() {
        let rewritten = rewrite_after_anchor(
            "原著开端。关键时刻。原著结局不再成立。",
            Some("关键时刻。"),
            "你介入以后，新的因果开始运转。",
        )
        .unwrap();

        assert_eq!(
            rewritten,
            "原著开端。关键时刻。\n\n你介入以后，新的因果开始运转。"
        );
        assert!(!rewritten.contains("原著结局不再成立"));
    }

    #[test]
    fn character_context_keeps_a_directed_turn_behind_unrelated_global_tail() {
        let character_id = Uuid::new_v4();
        let unrelated_character_id = Uuid::new_v4();
        let journal = (1..=5)
            .map(|turn_number| {
                journal_entry(
                    turn_number,
                    if turn_number == 1 {
                        character_id
                    } else {
                        unrelated_character_id
                    },
                )
            })
            .collect();

        let selected = recent_character_actions(journal, character_id, 5);

        assert_eq!(
            selected
                .iter()
                .map(|entry| entry.turn_number)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn character_context_keeps_only_the_latest_four_directed_turns_in_order() {
        let character_id = Uuid::new_v4();
        let journal = (1..=6)
            .map(|turn_number| journal_entry(turn_number, character_id))
            .collect();

        let selected = recent_character_actions(journal, character_id, 6);

        assert_eq!(
            selected
                .iter()
                .map(|entry| entry.turn_number)
                .collect::<Vec<_>>(),
            vec![3, 4, 5, 6]
        );
    }

    #[test]
    fn character_context_exposes_the_directed_action_but_not_private_intent() {
        let character_id = Uuid::new_v4();
        let private_marker = "PRIVATE_INTENT_PRETEND_TO_ALLY_THEN_BETRAY";
        let mut entry = journal_entry(1, character_id);
        entry.action.intent = private_marker.into();

        let selected = recent_character_actions(vec![entry], character_id, 1);
        let encoded = serde_json::to_string(&selected).unwrap();

        assert_eq!(selected[0].action.kind, WorldActionKind::Converse);
        assert_eq!(
            selected[0].action.target_id.as_deref(),
            Some(character_id.to_string().as_str())
        );
        assert!(!encoded.contains(private_marker));
        assert!(!encoded.contains("intent"));
    }

    #[test]
    fn character_context_does_not_leak_an_action_from_event_or_relationship_witnessing() {
        let character_id = Uuid::new_v4();
        let mut entry = journal_entry(1, Uuid::new_v4());
        entry.action = WorldAction {
            kind: WorldActionKind::Investigate,
            target_id: Some("sealed-archive".into()),
            intent: "秘密调查角色不应知道的档案".into(),
        };
        entry.transition.events.push(
            crate::domain::services::narrative_transition::TransitionEvent {
                summary: "角色参与了同一回合的公开事件".into(),
                actor_character_ids: vec![character_id],
                location_id: None,
            },
        );
        entry.transition.relationship_changes.push(
            crate::domain::services::narrative_transition::RelationshipChange {
                character_id,
                delta: 1,
                reason: "角色对公开事件作出反应".into(),
            },
        );

        assert!(character_witnessed_turn(
            &entry.action,
            &entry.transition,
            character_id,
        ));
        assert!(recent_character_actions(vec![entry], character_id, 1).is_empty());
    }

    #[test]
    fn permanent_journey_fact_excludes_rendered_prose_and_keeps_coordinates() {
        let user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let mut result = WorldTurnResult {
            turn_id: Uuid::new_v4(),
            action: WorldAction {
                kind: WorldActionKind::Converse,
                target_id: Some(character_id.to_string()),
                intent: "PRIVATE_INTENT_PRETEND_TO_COOPERATE_THEN_BETRAY".into(),
            },
            resolution: None,
            transition: crate::domain::entities::world_session::WorldTurnTransition {
                schema_version: 1,
                prompt_version: "world-turn-v2".into(),
                canon_model_version: 1,
                canonical_checkpoint_chapter: 3,
                rendered_narrative: "这段生成叙事绝不能成为永久事实。".into(),
                events: vec![
                    crate::domain::services::narrative_transition::TransitionEvent {
                        summary: "玩家发现北塔换防记录".into(),
                        actor_character_ids: vec![character_id],
                        location_id: Some("north-tower".into()),
                    },
                ],
                relationship_changes: vec![],
                location_changes: vec![],
                thread_changes: vec![],
                player_location_id: None,
                inventory_additions: vec![],
                inventory_removals: vec![],
                knowledge_discoveries: vec!["守门人曾修改换防表".into()],
                faction_changes: vec![],
                canonical_event_change: None,
            },
            world_state: WorldState::new(user_id, novel_id),
        };

        let entry_context = crate::domain::entities::world_session::WorldEntryContext {
            model_version: 1,
            checkpoint_chapter: 3,
            unlocked_through_chapter: 7,
            characters: vec![],
            locations: vec![],
            factions: vec![],
            hard_rules: vec![],
            dead_character_ids: vec![],
            threads: vec![],
            scheduled_events: vec![],
            character_goals: vec![],
        };
        result.world_state.state["open_world"] = serde_json::to_value(
            crate::domain::entities::world_session::WorldSession::from_context(&entry_context)
                .unwrap(),
        )
        .unwrap();
        let response = serde_json::to_value(WorldTurnResponse {
            result: result.clone(),
            memory_projection_status: MemoryProjectionStatus::Saved,
        })
        .unwrap();
        assert_eq!(response["turn_id"], result.turn_id.to_string());
        assert_eq!(response["memory_projection_status"], "saved");
        assert!(response.get("result").is_none());
        let (projected, source_chapter_high_water) =
            world_journey_fact(&result, character_id).unwrap().unwrap();
        assert_eq!(source_chapter_high_water, 7);
        assert!(!projected.contains("这段生成叙事绝不能成为永久事实"));

        let encoded = world_journey_fact_at(&result, character_id, 7, 9)
            .unwrap()
            .unwrap();
        let fact: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(fact["schema_version"], 2);
        assert_eq!(fact["source"], "committed_world_turn");
        assert_eq!(fact["source_turn_id"], result.turn_id.to_string());
        assert_eq!(fact["turn_number"], 7);
        assert_eq!(fact["world_time"], 9);
        assert_eq!(fact["witness_character_id"], character_id.to_string());
        assert_eq!(fact["reader_action"]["kind"], "converse");
        assert_eq!(fact["reader_action"]["target_id"], character_id.to_string());
        assert!(fact["reader_action"].get("intent").is_none());
        assert!(fact["committed_changes"]["knowledge_discoveries"].is_null());
        assert!(!encoded.contains("这段生成叙事绝不能成为永久事实"));
        assert!(!encoded.contains("PRIVATE_INTENT_PRETEND_TO_COOPERATE_THEN_BETRAY"));
        assert!(encoded.chars().count() <= MAX_JOURNEY_MEMORY_EVENT_CHARS);

        assert!(character_witnessed_turn(
            &result.action,
            &result.transition,
            character_id,
        ));
    }

    #[test]
    fn non_character_action_does_not_witness_a_uuid_collision() {
        let user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let entry = journal_entry(1, character_id);
        let result = WorldTurnResult {
            turn_id: entry.turn_id,
            action: WorldAction {
                kind: WorldActionKind::Travel,
                target_id: Some(character_id.to_string()),
                intent: "前往同名地点".into(),
            },
            resolution: None,
            transition: entry.transition,
            world_state: WorldState::new(user_id, novel_id),
        };

        assert!(!character_witnessed_turn(
            &result.action,
            &result.transition,
            character_id,
        ));
        assert!(world_journey_fact_at(&result, character_id, 1, 1)
            .unwrap()
            .is_none());
    }

    #[test]
    fn permanent_journey_fact_excludes_unwitnessed_secrets_from_a_mixed_turn() {
        use crate::domain::entities::world_session::{
            CanonicalEventChange, CanonicalEventStatus, FactionStandingChange,
        };
        use crate::domain::services::narrative_transition::{
            LocationChange, RelationshipChange, ThreadChange, ThreadStatus, TransitionEvent,
        };

        let user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let unrelated_character_id = Uuid::new_v4();
        let repeated = "边界内容".repeat(MAX_JOURNEY_MEMORY_FIELD_CHARS);
        let result = WorldTurnResult {
            turn_id: Uuid::new_v4(),
            action: WorldAction {
                kind: crate::domain::entities::world_session::WorldActionKind::Investigate,
                target_id: Some("north-tower".into()),
                intent: repeated.clone(),
            },
            resolution: None,
            transition: crate::domain::entities::world_session::WorldTurnTransition {
                schema_version: 1,
                prompt_version: "world-turn-v2".into(),
                canon_model_version: 1,
                canonical_checkpoint_chapter: 3,
                rendered_narrative: "不应进入事实".into(),
                events: vec![
                    TransitionEvent {
                        summary: "主角明确见证的事件".into(),
                        actor_character_ids: vec![character_id],
                        location_id: Some("north-tower".into()),
                    },
                    TransitionEvent {
                        summary: "异地角色的秘密事件".into(),
                        actor_character_ids: vec![unrelated_character_id],
                        location_id: Some(repeated.clone()),
                    },
                ],
                relationship_changes: vec![RelationshipChange {
                    character_id,
                    delta: 2,
                    reason: repeated.clone(),
                }],
                location_changes: vec![LocationChange {
                    location_id: "north-tower".into(),
                    state: repeated.clone(),
                    reason: repeated.clone(),
                }],
                thread_changes: vec![ThreadChange {
                    thread_id: "gatekeeper".into(),
                    status: ThreadStatus::Resolved,
                    description: repeated.clone(),
                }],
                player_location_id: Some("north-tower".into()),
                inventory_additions: vec![repeated.clone(); 4],
                inventory_removals: vec![repeated.clone(); 4],
                knowledge_discoveries: vec!["玩家私有发现".into()],
                faction_changes: vec![FactionStandingChange {
                    faction_id: "watch".into(),
                    delta: 1,
                    reason: repeated,
                }],
                canonical_event_change: Some(CanonicalEventChange {
                    event_id: "changing-of-the-guard".into(),
                    status: CanonicalEventStatus::Witnessed,
                    reason: "玩家亲历".into(),
                }),
            },
            world_state: WorldState::new(user_id, novel_id),
        };

        let encoded = world_journey_fact_at(&result, character_id, 7, 9)
            .unwrap()
            .unwrap();
        let fact: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        let changes = &fact["committed_changes"];

        assert_eq!(changes["relationships"][0]["delta"], 2);
        assert_eq!(changes["events"][0]["summary"], "主角明确见证的事件");
        assert!(fact["reader_action"].is_null());
        assert!(!encoded.contains("异地角色的秘密事件"));
        assert!(!encoded.contains("玩家私有发现"));
        assert!(!encoded.contains("changing-of-the-guard"));
        assert!(changes["locations"].is_null());
        assert!(changes["threads"].is_null());
        assert!(changes["factions"].is_null());
        assert!(encoded.chars().count() <= MAX_JOURNEY_MEMORY_EVENT_CHARS);
        assert!(!encoded.contains("不应进入事实"));
    }

    #[test]
    fn journey_fact_counts_match_sections_left_after_budget_trimming() {
        use crate::domain::services::narrative_transition::TransitionEvent;

        let character_id = Uuid::new_v4();
        let mut entry = journal_entry(1, character_id);
        entry.action.kind = WorldActionKind::Investigate;
        entry.action.target_id = None;
        entry.transition.events = (0..4)
            .map(|_| TransitionEvent {
                summary: "事".repeat(MAX_JOURNEY_MEMORY_FIELD_CHARS),
                actor_character_ids: vec![character_id],
                location_id: Some("l".repeat(200)),
            })
            .collect();
        let result = WorldTurnResult {
            turn_id: entry.turn_id,
            action: entry.action,
            resolution: None,
            transition: entry.transition,
            world_state: WorldState::new(Uuid::new_v4(), Uuid::new_v4()),
        };

        let encoded = world_journey_fact_at(&result, character_id, 1, 1)
            .unwrap()
            .unwrap();
        let fact: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        let changes = fact["committed_changes"].as_object().unwrap();
        let emitted_events = changes
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        let emitted_relationships = changes
            .get("relationships")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);

        assert!((1..=4).contains(&emitted_events));
        assert_eq!(emitted_relationships, 0);
        assert_eq!(fact["change_counts"]["events"], emitted_events);
        assert_eq!(
            fact["change_counts"]["relationships"],
            emitted_relationships
        );
        assert_eq!(fact["change_counts"]["reader_action"], 0);
        assert_consumer_compatible_journey_fact(&encoded, character_id);
    }

    #[test]
    fn journey_fact_omits_a_direct_action_private_intent() {
        let character_id = Uuid::new_v4();
        let mut entry = journal_entry(1, character_id);
        entry.action.target_id = Some(character_id.to_string().to_uppercase());
        entry.action.intent = "PRIVATE_INTENT_WITHHOLD_THE_REAL_PLAN".into();
        let result = WorldTurnResult {
            turn_id: entry.turn_id,
            action: entry.action,
            resolution: None,
            transition: entry.transition,
            world_state: WorldState::new(Uuid::new_v4(), Uuid::new_v4()),
        };

        let encoded = world_journey_fact_at(&result, character_id, 1, 1)
            .unwrap()
            .unwrap();
        let fact: serde_json::Value = serde_json::from_str(&encoded).unwrap();

        assert_eq!(fact["reader_action"]["kind"], "converse");
        assert_eq!(fact["reader_action"]["target_id"], character_id.to_string());
        assert!(fact["reader_action"].get("intent").is_none());
        assert!(!encoded.contains("PRIVATE_INTENT_WITHHOLD_THE_REAL_PLAN"));
        assert_consumer_compatible_journey_fact(&encoded, character_id);
    }

    #[test]
    fn journey_fact_normalizes_event_boundary_whitespace_for_the_consumer() {
        use crate::domain::services::narrative_transition::TransitionEvent;

        let character_id = Uuid::new_v4();
        let mut entry = journal_entry(1, character_id);
        entry.action.kind = WorldActionKind::Investigate;
        entry.action.target_id = None;
        entry.transition.events = vec![TransitionEvent {
            summary: "\r\n\u{3000}角色见证了事件\u{2003}\n".into(),
            actor_character_ids: vec![character_id],
            location_id: Some("\u{3000}north-tower\u{2003}".into()),
        }];
        let result = WorldTurnResult {
            turn_id: entry.turn_id,
            action: entry.action,
            resolution: None,
            transition: entry.transition,
            world_state: WorldState::new(Uuid::new_v4(), Uuid::new_v4()),
        };

        let encoded = world_journey_fact_at(&result, character_id, 1, 1)
            .unwrap()
            .unwrap();
        let fact: serde_json::Value = serde_json::from_str(&encoded).unwrap();

        assert_eq!(
            fact["committed_changes"]["events"][0]["summary"],
            "角色见证了事件"
        );
        assert_eq!(
            fact["committed_changes"]["events"][0]["location_id"],
            "north-tower"
        );
        assert_consumer_compatible_journey_fact(&encoded, character_id);
    }

    #[test]
    fn journey_fact_skips_when_all_witness_text_normalizes_to_empty() {
        use crate::domain::services::narrative_transition::TransitionEvent;

        let character_id = Uuid::new_v4();
        let mut entry = journal_entry(1, character_id);
        entry.action.kind = WorldActionKind::Investigate;
        entry.action.target_id = None;
        entry.action.intent = " \r\n\u{3000}".into();
        entry.transition.events = vec![TransitionEvent {
            summary: "\r\n\u{2003}".into(),
            actor_character_ids: vec![character_id],
            location_id: None,
        }];
        let result = WorldTurnResult {
            turn_id: entry.turn_id,
            action: entry.action,
            resolution: None,
            transition: entry.transition,
            world_state: WorldState::new(Uuid::new_v4(), Uuid::new_v4()),
        };

        assert!(world_journey_fact_at(&result, character_id, 1, 1)
            .unwrap()
            .is_none());
    }

    #[test]
    fn missing_anchor_fails_closed() {
        assert!(rewrite_after_anchor("原著正文", Some("不存在"), "新正文").is_err());
    }

    #[test]
    fn generated_player_chapter_must_be_bounded_chinese() {
        assert!(validate_player_chapter("你进入了新的时间线。").is_ok());
        assert!(validate_player_chapter("English only").is_err());
    }

    #[test]
    fn timeline_generation_advances_one_chapter_per_request() {
        assert!(is_next_chapter(9, 10));
        assert!(!is_next_chapter(8, 10));
        assert!(!is_next_chapter(i32::MAX, i32::MIN));
    }

    #[test]
    fn a_committed_choice_is_only_replayed_for_the_same_index() {
        assert!(require_same_choice(1, 1).is_ok());
        assert!(matches!(
            require_same_choice(1, 0),
            Err(NarrativeError::ChoiceConflict)
        ));
    }

    #[test]
    fn open_world_freezes_new_branch_choices_but_keeps_exact_replay() {
        assert!(require_choice_submission_allowed(1, Some(1), 8, Some(2), true).is_ok());
        assert!(matches!(
            require_choice_submission_allowed(1, Some(0), 8, Some(2), true),
            Err(NarrativeError::ChoiceConflict)
        ));
        assert!(matches!(
            require_choice_submission_allowed(1, None, 2, Some(2), true),
            Err(NarrativeError::Conflict(_))
        ));
        assert!(matches!(
            require_choice_submission_allowed(1, None, 3, Some(2), false),
            Err(NarrativeError::Conflict(_))
        ));
        assert!(require_choice_submission_allowed(1, None, 2, Some(2), false).is_ok());
        assert!(require_choice_submission_allowed(1, None, 8, None, false).is_ok());
    }

    #[test]
    fn recent_world_context_keeps_the_narrative_ending() {
        assert_eq!(trailing_chars("一二三四", 2), "三四");
    }
}
