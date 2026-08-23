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
use crate::domain::entities::narrative_node::{NarrativeChoice, NarrativeNode, WorldState};
use crate::domain::entities::player_entity::PlayerEntity;
use crate::domain::entities::world_session::{
    build_world_turn_prompt_with_check, parse_world_turn_transition_with_check, trailing_chars,
    CharacterWorldContext, RecentWorldTurnContext, WorldAction, WorldSession,
    MAX_RECENT_WORLD_NARRATIVE_CHARS, MAX_RECENT_WORLD_TURNS,
};
use crate::domain::ports::{AgentMemoryPort, DiceRollerPort, LlmPort, NarrativeLlmTask};
use crate::domain::repositories::{
    BeginWorldTurn, ChapterInfo, ChapterReadRepository, CharacterBrief, ChoiceCommit,
    GameRuleTemplateRequestError, NarrativeNodeRepository, NovelInfo, PlayerChapter,
    PlayerChapterOrigin, PlayerChapterRepository, UserChoiceRecord, UserChoiceRepository,
    WorldStateRepository, WorldTurnClaim, WorldTurnJournalEntry, WorldTurnRepository,
    WorldTurnResult,
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
/// Journey memories: bounded event text, fixed importance for committed world
/// turns, and bounded best-effort retries (a memory loss never fails a turn).
const MAX_JOURNEY_MEMORY_EVENT_CHARS: usize = 2_000;
const WORLD_TURN_MEMORY_IMPORTANCE: i32 = 7;
const JOURNEY_MEMORY_RETRIES: usize = 2;

/// Deterministic protagonist pick: role 'protagonist', earliest first
/// appearance, stable id tiebreak. Zero protagonists => None (caller skips).
pub(crate) fn resolve_protagonist(characters: &[CharacterBrief]) -> Option<Uuid> {
    characters
        .iter()
        .filter(|character| character.role == "protagonist")
        .min_by_key(|character| {
            (
                character.first_appearance_chapter.unwrap_or(0),
                character.id,
            )
        })
        .map(|character| character.id)
}

/// Best-effort projection of a committed world turn into the permanent memory
/// layer: idempotent (memory id = turn id), fire-and-forget with bounded
/// retries, and it never fails or delays the caller's turn result.
pub(crate) async fn record_world_journey_memory(
    agent_memory: &dyn AgentMemoryPort,
    chapter_repo: &dyn ChapterReadRepository,
    turn_id: Uuid,
    user_id: Uuid,
    novel_id: Uuid,
    checkpoint_chapter: i32,
    event: &str,
) {
    let characters = match chapter_repo.list_characters(novel_id, user_id).await {
        Ok(characters) => characters,
        Err(error) => {
            tracing::debug!(%error, %novel_id, "character list unavailable; skipping journey memory");
            return;
        }
    };
    let Some(character_id) = resolve_protagonist(&characters) else {
        tracing::debug!(%novel_id, "no protagonist found; skipping journey memory");
        return;
    };
    let event: String = event.chars().take(MAX_JOURNEY_MEMORY_EVENT_CHARS).collect();
    for attempt in 0..=JOURNEY_MEMORY_RETRIES {
        match agent_memory
            .save_permanent_memory(
                turn_id,
                character_id,
                user_id,
                novel_id,
                checkpoint_chapter,
                &event,
                WORLD_TURN_MEMORY_IMPORTANCE,
            )
            .await
        {
            Ok(()) => return,
            Err(error) => {
                warn!(%error, attempt, %turn_id, "journey memory save failed");
            }
        }
    }
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
    #[error("Novel service is unavailable")]
    Unavailable(#[source] anyhow::Error),
    #[error("Consequence generation failed")]
    Llm(#[source] anyhow::Error),
    #[error("Narrative operation failed")]
    Internal(#[source] anyhow::Error),
}

pub type NarrativeResult<T> = std::result::Result<T, NarrativeError>;

fn require_same_choice(requested: i32, committed: i32) -> NarrativeResult<()> {
    if requested == committed {
        Ok(())
    } else {
        Err(NarrativeError::ChoiceConflict)
    }
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

struct ResolvedChapter {
    canonical: ChapterInfo,
    content: String,
    generated: bool,
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

    async fn narrative_world_state(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<(WorldState, Option<PlayerEntity>)> {
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let player = world_state
            .player_entity()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?;
        if player.is_none()
            && self
                .chapter_repo
                .uses_original_player_identity(novel_id, user_id)
                .await
                .map_err(NarrativeError::Unavailable)?
        {
            return Err(NarrativeError::Conflict(
                "Create PlayerEntity before entering an original-player branch".into(),
            ));
        }
        Ok((world_state, player))
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
            return if checkpoint_matches
                && existing.matches_definition(
                    &command.name,
                    &command.background,
                    &command.capabilities,
                    &command.location_id,
                    &command.inventory,
                )
                && existing.matches_rules(&command.rules)
            {
                Ok(existing)
            } else {
                Err(NarrativeError::Conflict(
                    "PlayerEntity already exists with a different definition".into(),
                ))
            };
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
        let stored = self
            .world_state_repo
            .create_player_entity(&candidate)
            .await
            .map_err(NarrativeError::Internal)?;
        if !stored.matches_definition(
            &candidate.name,
            &candidate.background,
            &candidate.capabilities,
            &candidate.location_id,
            &candidate.inventory,
        ) || !stored.matches_rules(&candidate.rules)
        {
            return Err(NarrativeError::Conflict(
                "PlayerEntity was concurrently created with a different definition".into(),
            ));
        }
        Ok(stored)
    }

    #[tracing::instrument(skip(self))]
    pub async fn start_open_world(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<OpenWorldView> {
        self.owned_novel(novel_id, user_id).await?;
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
        if state
            .open_world()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .is_some()
        {
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
        let state = self
            .world_state_repo
            .start_open_world(user_id, novel_id, &context, game_rules.as_ref())
            .await
            .map_err(NarrativeError::Internal)?;
        self.open_world_view(user_id, novel_id, state).await
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_open_world(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<OpenWorldView> {
        self.owned_novel(novel_id, user_id).await?;
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
        let state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        state
            .character_world_context(character_id)
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))
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
        let journal = self
            .world_turn_repo
            .journal(user_id, novel_id, 100)
            .await
            .map_err(NarrativeError::Internal)?;
        Ok(OpenWorldView {
            player,
            session,
            world_state,
            journal,
        })
    }

    /// Fire the journey-memory projection without blocking the turn response:
    /// the committed turn is already durable, and a lost write is healed by the
    /// idempotent replay path (memory id = turn id), so this is best-effort and
    /// must never delay or fail the caller.
    fn fire_journey_memory(
        &self,
        turn_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        checkpoint_chapter: i32,
        event: String,
    ) {
        let agent_memory = Arc::clone(&self.agent_memory);
        let chapter_repo = Arc::clone(&self.chapter_repo);
        tokio::spawn(async move {
            record_world_journey_memory(
                agent_memory.as_ref(),
                chapter_repo.as_ref(),
                turn_id,
                user_id,
                novel_id,
                checkpoint_chapter,
                &event,
            )
            .await;
        });
    }

    #[tracing::instrument(skip(self, action), fields(turn_id = %turn_id))]
    pub async fn submit_world_turn(
        &self,
        turn_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        action: WorldAction,
    ) -> NarrativeResult<WorldTurnResult> {
        let novel = self.owned_novel(novel_id, user_id).await?;
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
        let request =
            serde_json::to_vec(&action).map_err(|error| NarrativeError::Internal(error.into()))?;
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
                    session.turn_number,
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
            expected_turn_number: session.turn_number,
        };
        let (claim, attempt) = match self
            .world_turn_repo
            .begin_turn(&claim)
            .await
            .map_err(NarrativeError::Internal)?
        {
            BeginWorldTurn::Acquired { claim, attempt } => (*claim, attempt),
            BeginWorldTurn::Completed(result) => {
                let result = *result;
                // Replay path: the committed turn may have missed its memory
                // projection on the original attempt; the write is idempotent
                // (memory id = turn id), so firing here heals lost writes.
                // Anchor at the COMMITTED turn's checkpoint: the live session
                // may have advanced since the original commit.
                self.fire_journey_memory(
                    result.turn_id,
                    user_id,
                    novel_id,
                    result.transition.canonical_checkpoint_chapter,
                    result.transition.rendered_narrative.clone(),
                );
                return Ok(result);
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
        if let Err(error) = world_state.validate_world_action(&claim.action, &session.entry_context)
        {
            self.fail_world_turn(&claim, attempt, "validation_error")
                .await;
            return Err(NarrativeError::Validation(error.to_string()));
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
                    .chat_json(NarrativeLlmTask::NarrativeTransition, &prompt),
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
                self.fail_world_turn(&claim, attempt, "commit_error").await;
                return Err(NarrativeError::Internal(error));
            }
            None => {
                self.fail_world_turn(&claim, attempt, "lease_lost").await;
                return Err(NarrativeError::TurnOutcomeUnknown);
            }
        };
        lease.stop();
        // Fresh-commit path: project the committed turn into the permanent
        // memory layer (idempotent by turn id, never fails the turn). The
        // committed transition's checkpoint is the authoritative anchor.
        self.fire_journey_memory(
            committed.turn_id,
            user_id,
            novel_id,
            committed.transition.canonical_checkpoint_chapter,
            committed.transition.rendered_narrative.clone(),
        );
        Ok(committed)
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
        let (world_state, player) = self.narrative_world_state(user_id, novel_id).await?;
        let chapter = self
            .resolve_chapter(user_id, novel_id, chapter_number, &novel_info)
            .await?;
        if let Some(existing_choice) = self
            .choice_repo
            .find_by_novel(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?
            .into_iter()
            .find(|choice| choice.chapter_number == chapter_number)
        {
            let existing_node = self
                .node_repo
                .find_by_id(existing_choice.node_id)
                .await
                .map_err(NarrativeError::Internal)?
                .filter(|node| {
                    node.novel_id == novel_id
                        && node.user_id.is_none_or(|owner_id| owner_id == user_id)
                })
                .ok_or(NarrativeError::NotFound)?;
            return Ok(Some(existing_node));
        }
        let node_owner = chapter.generated.then_some(user_id);
        if let Some(node) = self
            .node_repo
            .find_by_chapter(novel_id, chapter_number, node_owner)
            .await
            .map_err(NarrativeError::Internal)?
        {
            return Ok(Some(node));
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
            .chat_json(NarrativeLlmTask::BranchGeneration, &prompt)
            .await
            .map_err(NarrativeError::Llm)
            .and_then(|json| {
                parse_generated_branch(&json)
                    .map_err(|error| NarrativeError::Llm(anyhow::anyhow!(error)))
            })?;
        if !chapter.content.contains(&generated.anchor_quote) {
            return Err(NarrativeError::Llm(anyhow::anyhow!(
                "generated branch anchor was not present in chapter source"
            )));
        }
        let mut node = NarrativeNode::new(
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
        .with_anchor_quote(generated.anchor_quote);
        if let Some(user_id) = node_owner {
            node = node.for_user(user_id);
        }
        self.node_repo
            .save(&node)
            .await
            .map_err(NarrativeError::Internal)?;
        // Reload after the upsert so concurrent first readers receive the same
        // persisted node id.
        self.node_repo
            .find_by_chapter(novel_id, chapter_number, node_owner)
            .await
            .map_err(NarrativeError::Internal)
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
        let chapter = self
            .resolve_chapter(user_id, novel_id, chapter_number, &novel_info)
            .await?;
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
        let (world_state, _) = self.narrative_world_state(user_id, novel_id).await?;
        let node = self
            .node_repo
            .find_by_id(node_id)
            .await
            .map_err(NarrativeError::Internal)?
            .filter(|node| {
                node.novel_id == novel_id && node.user_id.is_none_or(|owner_id| owner_id == user_id)
            })
            .ok_or(NarrativeError::NotFound)?;
        let choice_text = usize::try_from(requested_choice_index)
            .ok()
            .and_then(|index| node.choices.get(index))
            .map(|choice| choice.text.clone())
            .ok_or_else(|| {
                NarrativeError::Validation("choice_index is outside the node choices".into())
            })?;

        let existing = self
            .choice_repo
            .find_user_choice(user_id, node_id)
            .await
            .map_err(NarrativeError::Internal)?;
        if let Some(existing) = existing.as_ref() {
            require_same_choice(requested_choice_index, existing.choice_index)?;
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
                Some(chapter) => chapter.content,
                None => {
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
                .commit_result(choice_draft(existing, full_content))
                .await;
        }
        if world_state
            .open_world()
            .map_err(|error| NarrativeError::Internal(anyhow::anyhow!(error)))?
            .is_some()
        {
            return Err(NarrativeError::Conflict(
                "Use world actions after entering the open world".into(),
            ));
        }

        let choice_index = requested_choice_index;
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
                .chat_json(NarrativeLlmTask::NarrativeTransition, &prompt)
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
        self.commit_result(ChoiceCommit {
            user_id,
            novel_id,
            node_id,
            chapter_number: node.chapter_number,
            choice_index,
            choice_text,
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
        self.world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)
    }

    async fn commit_result(&self, draft: ChoiceCommit) -> NarrativeResult<ChoiceResult> {
        let requested_choice_index = draft.choice_index;
        let committed = self
            .choice_repo
            .commit_choice(&draft)
            .await
            .map_err(NarrativeError::Internal)?;
        require_same_choice(requested_choice_index, committed.choice.choice_index)?;
        Ok(ChoiceResult {
            chapter_number: committed.choice.chapter_number,
            consequence: committed.choice.consequence.clone(),
            transition: committed.choice.transition,
            chapter_content: committed.player_chapter_content,
            world_state: committed.world_state,
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
        if let Some(player_chapter) = self
            .player_chapter_repo
            .find(user_id, novel_id, chapter_number)
            .await
            .map_err(NarrativeError::Internal)?
        {
            return Ok(ResolvedChapter {
                canonical,
                content: player_chapter.content,
                generated: true,
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
            });
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
                "你是互动小说的玩家时间线主笔。只输出自然的简体中文小说正文。",
                &prompt,
            )
            .await
            .map_err(NarrativeError::Llm)?
            .trim()
            .to_owned();
        validate_player_chapter(&content)?;
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

fn choice_draft(existing: &UserChoiceRecord, rewritten_chapter_content: String) -> ChoiceCommit {
    ChoiceCommit {
        user_id: existing.user_id,
        novel_id: existing.novel_id,
        node_id: existing.node_id,
        chapter_number: existing.chapter_number,
        choice_index: existing.choice_index,
        choice_text: existing.choice_text.clone(),
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
    fn recent_world_context_keeps_the_narrative_ending() {
        assert_eq!(trailing_chars("一二三四", 2), "三四");
    }
}
