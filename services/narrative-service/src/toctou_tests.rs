use anyhow::{bail, ensure, Result};
use async_trait::async_trait;
use chrono::Utc;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::application::handlers::{
    CreatePlayerEntityCommand, NarrativeCommandHandler, NarrativeError,
};
use crate::domain::entities::{
    narrative_node::{NarrativeChoice, NarrativeNode, WorldState},
    player_entity::PlayerEntity,
    world_session::{
        WorldAction, WorldActionKind, WorldCharacterRef, WorldEntryContext, WorldTurnTransition,
    },
};
use crate::domain::ports::{AgentMemoryPort, DiceRollerPort, LlmPort, NarrativeLlmTask};
use crate::domain::repositories::{
    BeginWorldTurn, ChapterInfo, ChapterReadRepository, CharacterBrief, ChoiceCommit,
    ChoiceCommitResult, MemoryProjectionStatus, NarrativeNodeRepository, NovelInfo, PlayerChapter,
    PlayerChapterOrigin, PlayerChapterRepository, PlayerEntryContext, UserChoiceRecord,
    UserChoiceRepository, WorldStateRepository, WorldTurnClaim, WorldTurnJournalEntry,
    WorldTurnRepository, WorldTurnResult,
};
use crate::domain::services::narrative_transition::{
    CanonContext, CanonEntityRef, NarrativeTransition, TransitionEvent,
};

const ANCHOR: &str = "城门在暮色中缓缓关闭，守卫举起火把照亮石阶。";

struct ToctouFixture {
    user_id: Uuid,
    other_user_id: Uuid,
    novel_id: Uuid,
    source_chapter: i32,
    available_chapters: Mutex<Vec<i32>>,
    current_chapter: AtomicI32,
    self_identity: AtomicBool,
    block_next_character_list: AtomicBool,
    character_list_entered: Notify,
    character_list_release: Notify,
    world_state: Mutex<WorldState>,
    other_world_state: Mutex<WorldState>,
    nodes: Mutex<Vec<NarrativeNode>>,
    choice: Mutex<Option<UserChoiceRecord>>,
    player_chapter: Mutex<Option<PlayerChapter>>,
    block_next_player_chapter_read: AtomicBool,
    player_chapter_read_entered: Notify,
    player_chapter_read_release: Notify,
    provider_calls: AtomicUsize,
    provider_prompts: Mutex<Vec<String>>,
    provider_entered: Notify,
    provider_release: Notify,
    block_next_journal: AtomicBool,
    journal_calls: AtomicUsize,
    journal_entered: Notify,
    journal_release: Notify,
    journal: Mutex<Vec<WorldTurnJournalEntry>>,
    block_next_node_read: AtomicBool,
    node_read_entered: Notify,
    node_read_release: Notify,
    player_entry_context: Mutex<Option<PlayerEntryContext>>,
    block_next_player_entry_context: AtomicBool,
    player_entry_context_entered: Notify,
    player_entry_context_release: Notify,
    completed_world_turn: Mutex<Option<WorldTurnResult>>,
    memory_projection_status: Mutex<MemoryProjectionStatus>,
    acquire_next_world_turn: AtomicBool,
    world_turn_in_progress: AtomicBool,
    world_turn_stale: AtomicBool,
    begin_turn_timeline_conflict: AtomicBool,
    complete_turn_timeline_conflict: AtomicBool,
    failed_turns: Mutex<Vec<(Uuid, i64, String)>>,
    last_expected_turn_number: AtomicI64,
    block_next_complete_turn: AtomicBool,
    complete_turn_entered: Notify,
    complete_turn_release: Notify,
    begin_turn_calls: AtomicUsize,
    complete_turn_calls: AtomicUsize,
    finish_projection_calls: AtomicUsize,
}

impl ToctouFixture {
    fn new(with_node: bool) -> Self {
        Self::at_chapter(with_node, 2)
    }

    fn at_chapter(with_node: bool, source_chapter: i32) -> Self {
        let user_id = Uuid::new_v4();
        let other_user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let player = PlayerEntity::new(
            user_id,
            novel_id,
            source_chapter,
            "云舟".into(),
            "远行者".into(),
            vec!["观察".into()],
            "city-gate".into(),
            vec![],
        )
        .unwrap();
        let mut world_state = WorldState::new(user_id, novel_id);
        world_state.state["player_entity"] = serde_json::to_value(player).unwrap();
        let other_player = PlayerEntity::new(
            other_user_id,
            novel_id,
            source_chapter,
            "霜璃".into(),
            "只向自己公开身份的旧廷密探".into(),
            vec!["解读暗号".into()],
            "city-gate".into(),
            vec![],
        )
        .unwrap();
        let mut other_world_state = WorldState::new(other_user_id, novel_id);
        other_world_state.state["player_entity"] = serde_json::to_value(other_player).unwrap();
        let node = with_node.then(|| {
            NarrativeNode::new(
                novel_id,
                source_chapter,
                "城门关闭前必须作出决定。".into(),
                vec![
                    NarrativeChoice {
                        index: 0,
                        text: "留在城内追查线索".into(),
                        hint: "风险更高，但接近真相".into(),
                        generated_consequence: None,
                    },
                    NarrativeChoice {
                        index: 1,
                        text: "趁夜离开城门".into(),
                        hint: "暂时避开守卫".into(),
                        generated_consequence: None,
                    },
                ],
            )
            .with_anchor_quote(ANCHOR.into())
            .for_user(user_id)
        });
        Self {
            user_id,
            other_user_id,
            novel_id,
            source_chapter,
            available_chapters: Mutex::new(vec![source_chapter]),
            current_chapter: AtomicI32::new(source_chapter),
            self_identity: AtomicBool::new(true),
            block_next_character_list: AtomicBool::new(false),
            character_list_entered: Notify::new(),
            character_list_release: Notify::new(),
            world_state: Mutex::new(world_state),
            other_world_state: Mutex::new(other_world_state),
            nodes: Mutex::new(node.into_iter().collect()),
            choice: Mutex::new(None),
            player_chapter: Mutex::new(None),
            block_next_player_chapter_read: AtomicBool::new(false),
            player_chapter_read_entered: Notify::new(),
            player_chapter_read_release: Notify::new(),
            provider_calls: AtomicUsize::new(0),
            provider_prompts: Mutex::new(vec![]),
            provider_entered: Notify::new(),
            provider_release: Notify::new(),
            block_next_journal: AtomicBool::new(false),
            journal_calls: AtomicUsize::new(0),
            journal_entered: Notify::new(),
            journal_release: Notify::new(),
            journal: Mutex::new(vec![]),
            block_next_node_read: AtomicBool::new(false),
            node_read_entered: Notify::new(),
            node_read_release: Notify::new(),
            player_entry_context: Mutex::new(None),
            block_next_player_entry_context: AtomicBool::new(false),
            player_entry_context_entered: Notify::new(),
            player_entry_context_release: Notify::new(),
            completed_world_turn: Mutex::new(None),
            memory_projection_status: Mutex::new(MemoryProjectionStatus::Pending),
            acquire_next_world_turn: AtomicBool::new(false),
            world_turn_in_progress: AtomicBool::new(false),
            world_turn_stale: AtomicBool::new(false),
            begin_turn_timeline_conflict: AtomicBool::new(false),
            complete_turn_timeline_conflict: AtomicBool::new(false),
            failed_turns: Mutex::new(vec![]),
            last_expected_turn_number: AtomicI64::new(-1),
            block_next_complete_turn: AtomicBool::new(false),
            complete_turn_entered: Notify::new(),
            complete_turn_release: Notify::new(),
            begin_turn_calls: AtomicUsize::new(0),
            complete_turn_calls: AtomicUsize::new(0),
            finish_projection_calls: AtomicUsize::new(0),
        }
    }

    fn handler(self: &Arc<Self>) -> NarrativeCommandHandler {
        NarrativeCommandHandler {
            node_repo: self.clone(),
            choice_repo: self.clone(),
            world_state_repo: self.clone(),
            player_chapter_repo: self.clone(),
            chapter_repo: self.clone(),
            world_turn_repo: self.clone(),
            llm: self.clone(),
            agent_memory: self.clone(),
            dice_roller: self.clone(),
        }
    }

    fn chapter() -> ChapterInfo {
        ChapterInfo {
            content: format!("夜色笼罩古城。{ANCHOR}城外传来急促的马蹄声。"),
            is_key_node: true,
            key_node_description: Some("城门关闭前的抉择".into()),
        }
    }

    async fn wait_for_provider(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.provider_entered.notified())
            .await
            .expect("provider was not called");
    }

    async fn wait_for_character_list(&self) {
        tokio::time::timeout(
            Duration::from_secs(2),
            self.character_list_entered.notified(),
        )
        .await
        .expect("character list was not requested");
    }

    async fn wait_for_complete_turn(&self) {
        tokio::time::timeout(
            Duration::from_secs(2),
            self.complete_turn_entered.notified(),
        )
        .await
        .expect("world turn was not committed");
    }

    async fn wait_for_journal(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.journal_entered.notified())
            .await
            .expect("journal was not called");
    }

    async fn wait_for_node_read(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.node_read_entered.notified())
            .await
            .expect("node repository was not called");
    }

    async fn wait_for_player_entry_context(&self) {
        tokio::time::timeout(
            Duration::from_secs(2),
            self.player_entry_context_entered.notified(),
        )
        .await
        .expect("player-entry context was not requested");
    }

    async fn block_node_read_if_requested(&self) {
        if self.block_next_node_read.swap(false, Ordering::SeqCst) {
            self.node_read_entered.notify_one();
            self.node_read_release.notified().await;
        }
    }

    fn clear_world_state(&self) {
        *self.world_state.lock().unwrap() = WorldState::new(self.user_id, self.novel_id);
    }

    fn entry_context(
        &self,
        unlocked_through_chapter: i32,
        character_id: Option<Uuid>,
    ) -> WorldEntryContext {
        WorldEntryContext {
            model_version: 1,
            checkpoint_chapter: self.source_chapter,
            unlocked_through_chapter,
            characters: character_id
                .map(|id| {
                    vec![WorldCharacterRef {
                        id,
                        name: "守门人".into(),
                    }]
                })
                .unwrap_or_default(),
            locations: vec![],
            factions: vec![],
            hard_rules: vec![],
            dead_character_ids: vec![],
            threads: vec![],
            scheduled_events: vec![],
            character_goals: vec![],
        }
    }

    fn journal_entry(turn_number: i64) -> WorldTurnJournalEntry {
        let now = Utc::now();
        WorldTurnJournalEntry {
            turn_id: Uuid::new_v4(),
            turn_number,
            memory_projection_status: MemoryProjectionStatus::Saved,
            action: WorldAction {
                kind: WorldActionKind::PursueGoal,
                target_id: None,
                intent: format!("执行第 {turn_number} 回合"),
            },
            resolution: None,
            transition: WorldTurnTransition {
                schema_version: 1,
                prompt_version: "world-turn-v2".into(),
                canon_model_version: 1,
                canonical_checkpoint_chapter: 2,
                rendered_narrative: format!("第 {turn_number} 回合已经提交。"),
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
}

#[async_trait]
impl NarrativeNodeRepository for ToctouFixture {
    async fn save(&self, node: &NarrativeNode) -> Result<()> {
        let mut stored = self.nodes.lock().unwrap();
        if !stored.iter().any(|existing| {
            existing.user_id == node.user_id
                && existing.novel_id == node.novel_id
                && existing.chapter_number == node.chapter_number
        }) {
            stored.push(node.clone());
        }
        Ok(())
    }

    async fn find_by_chapter(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
        user_id: Option<Uuid>,
    ) -> Result<Option<NarrativeNode>> {
        self.block_node_read_if_requested().await;
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .iter()
            .find(|node| {
                node.novel_id == novel_id
                    && node.chapter_number == chapter_number
                    && node.user_id == user_id
            })
            .cloned())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<NarrativeNode>> {
        self.block_node_read_if_requested().await;
        Ok(self
            .nodes
            .lock()
            .unwrap()
            .iter()
            .find(|node| node.id == id)
            .cloned())
    }
}

#[async_trait]
impl UserChoiceRepository for ToctouFixture {
    async fn commit_choice(&self, draft: &ChoiceCommit) -> Result<ChoiceCommitResult> {
        let mut choice = self.choice.lock().unwrap();
        if let Some(existing) = choice.as_ref() {
            return Ok(ChoiceCommitResult {
                choice: existing.clone(),
                world_state: self.world_state.lock().unwrap().clone(),
                player_chapter_content: self
                    .player_chapter
                    .lock()
                    .unwrap()
                    .as_ref()
                    .expect("choice replay has a player chapter")
                    .content
                    .clone(),
            });
        }

        let mut world_state = self.world_state.lock().unwrap();
        ensure!(
            world_state.fingerprint() == draft.expected_world_state_fingerprint,
            "stale world state"
        );
        ensure!(
            world_state.apply_choice_transition(
                draft.node_id,
                draft.chapter_number,
                draft.choice_index,
                &draft.choice_text,
                &draft.transition,
            )?,
            "choice was already committed"
        );
        let record = UserChoiceRecord {
            id: Uuid::new_v4(),
            user_id: draft.user_id,
            novel_id: draft.novel_id,
            node_id: draft.node_id,
            chapter_number: draft.chapter_number,
            choice_index: draft.choice_index,
            choice_text: draft.choice_text.clone(),
            consequence: draft.transition.rendered_narrative.clone(),
            transition: draft.transition.clone(),
            created_at: Utc::now(),
        };
        let player_chapter = PlayerChapter {
            user_id: draft.user_id,
            novel_id: draft.novel_id,
            chapter_number: draft.chapter_number,
            content: draft.rewritten_chapter_content.clone(),
            origin: PlayerChapterOrigin::Choice,
            created_at: record.created_at,
        };
        *self.player_chapter.lock().unwrap() = Some(player_chapter.clone());
        *choice = Some(record.clone());
        Ok(ChoiceCommitResult {
            choice: record,
            world_state: world_state.clone(),
            player_chapter_content: player_chapter.content,
        })
    }

    async fn find_user_choice(
        &self,
        user_id: Uuid,
        node_id: Uuid,
    ) -> Result<Option<UserChoiceRecord>> {
        Ok(self
            .choice
            .lock()
            .unwrap()
            .clone()
            .filter(|choice| choice.user_id == user_id && choice.node_id == node_id))
    }

    async fn find_by_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<Vec<UserChoiceRecord>> {
        Ok(self
            .choice
            .lock()
            .unwrap()
            .clone()
            .filter(|choice| choice.user_id == user_id && choice.novel_id == novel_id)
            .into_iter()
            .collect())
    }
}

#[async_trait]
impl WorldStateRepository for ToctouFixture {
    async fn get_or_create(&self, user_id: Uuid, novel_id: Uuid) -> Result<WorldState> {
        ensure!(novel_id == self.novel_id);
        if user_id == self.user_id {
            Ok(self.world_state.lock().unwrap().clone())
        } else if user_id == self.other_user_id {
            Ok(self.other_world_state.lock().unwrap().clone())
        } else {
            bail!("unknown test user")
        }
    }

    async fn create_player_entity(&self, _player: &PlayerEntity) -> Result<PlayerEntity> {
        bail!("unused")
    }

    async fn start_open_world(
        &self,
        _user_id: Uuid,
        _novel_id: Uuid,
        _context: &WorldEntryContext,
        _game_rules: Option<&crate::domain::entities::game_rules::GameRuleTemplate>,
    ) -> Result<WorldState> {
        bail!("unused")
    }

    async fn update(&self, _state: &WorldState) -> Result<()> {
        bail!("unused")
    }
}

#[async_trait]
impl PlayerChapterRepository for ToctouFixture {
    async fn find(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
    ) -> Result<Option<PlayerChapter>> {
        if self
            .block_next_player_chapter_read
            .swap(false, Ordering::SeqCst)
        {
            self.player_chapter_read_entered.notify_one();
            self.player_chapter_read_release.notified().await;
        }
        Ok(self
            .player_chapter
            .lock()
            .unwrap()
            .clone()
            .filter(|chapter| {
                chapter.user_id == user_id
                    && chapter.novel_id == novel_id
                    && chapter.chapter_number == chapter_number
            }))
    }

    async fn save_if_absent(&self, chapter: &PlayerChapter) -> Result<PlayerChapter> {
        let mut stored = self.player_chapter.lock().unwrap();
        Ok(stored.get_or_insert_with(|| chapter.clone()).clone())
    }

    async fn find_latest_before(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
    ) -> Result<Option<PlayerChapter>> {
        Ok(self
            .player_chapter
            .lock()
            .unwrap()
            .clone()
            .filter(|chapter| {
                chapter.user_id == user_id
                    && chapter.novel_id == novel_id
                    && chapter.chapter_number < chapter_number
            }))
    }
}

#[async_trait]
impl ChapterReadRepository for ToctouFixture {
    async fn get_chapter(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
        user_id: Uuid,
    ) -> Result<Option<ChapterInfo>> {
        Ok((novel_id == self.novel_id
            && matches!(user_id, id if id == self.user_id || id == self.other_user_id)
            && self
                .available_chapters
                .lock()
                .unwrap()
                .contains(&chapter_number))
        .then(Self::chapter))
    }

    async fn get_novel_info(&self, novel_id: Uuid, user_id: Uuid) -> Result<Option<NovelInfo>> {
        Ok((novel_id == self.novel_id
            && matches!(user_id, id if id == self.user_id || id == self.other_user_id))
        .then(|| NovelInfo {
            id: novel_id,
            title: "暮城".into(),
            deviation_mode: if user_id == self.user_id {
                "canon".into()
            } else {
                "creative".into()
            },
            world_summary: None,
        }))
    }

    async fn get_canon_context(
        &self,
        novel_id: Uuid,
        checkpoint_chapter: i32,
        user_id: Uuid,
    ) -> Result<Option<CanonContext>> {
        Ok((novel_id == self.novel_id
            && matches!(user_id, id if id == self.user_id || id == self.other_user_id)
            && checkpoint_chapter == self.source_chapter)
            .then(|| CanonContext {
                model_version: 1,
                checkpoint_chapter: self.source_chapter,
                characters: vec![],
                locations: vec![],
                hard_rules: vec![],
                dead_character_ids: vec![],
                threads: vec![],
            }))
    }

    async fn get_current_chapter(&self, _novel_id: Uuid, _user_id: Uuid) -> Result<i32> {
        Ok(self.current_chapter.load(Ordering::SeqCst))
    }

    async fn list_characters(
        &self,
        _novel_id: Uuid,
        _user_id: Uuid,
    ) -> Result<Vec<CharacterBrief>> {
        if self.block_next_character_list.swap(false, Ordering::SeqCst) {
            self.character_list_entered.notify_one();
            self.character_list_release.notified().await;
        }
        Ok(vec![])
    }

    async fn get_player_entry_context(
        &self,
        _novel_id: Uuid,
        _user_id: Uuid,
        _checkpoint_chapter: Option<i32>,
        _proposed_name: Option<&str>,
    ) -> Result<Option<PlayerEntryContext>> {
        if self
            .block_next_player_entry_context
            .swap(false, Ordering::SeqCst)
        {
            self.player_entry_context_entered.notify_one();
            self.player_entry_context_release.notified().await;
        }
        Ok(self.player_entry_context.lock().unwrap().clone())
    }

    async fn reader_identity_is_self(&self, _novel_id: Uuid, _user_id: Uuid) -> Result<bool> {
        Ok(self.self_identity.load(Ordering::SeqCst))
    }

    async fn get_world_entry_context(
        &self,
        _novel_id: Uuid,
        _checkpoint_chapter: i32,
        _user_id: Uuid,
    ) -> Result<Option<WorldEntryContext>> {
        Ok(None)
    }

    async fn request_game_rule_template(
        &self,
        _novel_id: Uuid,
        _user_id: Uuid,
    ) -> std::result::Result<
        crate::domain::entities::game_rules::GameRuleTemplate,
        crate::domain::repositories::GameRuleTemplateRequestError,
    > {
        Err(
            crate::domain::repositories::GameRuleTemplateRequestError::Unavailable(
                anyhow::anyhow!("game rules are not configured in this test"),
            ),
        )
    }

    async fn get_game_rule_template(
        &self,
        _novel_id: Uuid,
        _canon_model_version: i32,
        _user_id: Uuid,
    ) -> Result<Option<crate::domain::entities::game_rules::GameRuleTemplate>> {
        Ok(None)
    }
}

impl DiceRollerPort for ToctouFixture {
    fn roll_d20(
        &self,
        _user_id: Uuid,
        _novel_id: Uuid,
        _expected_turn_number: i64,
        _request_fingerprint: &[u8; 32],
    ) -> u8 {
        10
    }
}

#[async_trait]
impl LlmPort for ToctouFixture {
    async fn chat_longform(&self, _system: &str, prompt: &str) -> Result<String> {
        self.provider_calls.fetch_add(1, Ordering::SeqCst);
        self.provider_prompts.lock().unwrap().push(prompt.into());
        self.provider_entered.notify_one();
        self.provider_release.notified().await;
        Ok("生成的玩家时间线续章在城门外展开，远处的火光映亮了归途。".into())
    }

    async fn chat_json(&self, task: NarrativeLlmTask, prompt: &str) -> Result<String> {
        self.provider_calls.fetch_add(1, Ordering::SeqCst);
        self.provider_prompts.lock().unwrap().push(prompt.into());
        self.provider_entered.notify_one();
        self.provider_release.notified().await;
        Ok(match task {
            NarrativeLlmTask::BranchGeneration => {
                let private_variant = prompt.contains("霜璃")
                    && prompt.contains("只向自己公开身份的旧廷密探")
                    && prompt.contains("creative");
                serde_json::json!({
                    "anchor_quote": ANCHOR,
                    "description": if private_variant {
                        "密探在城门关闭前辨认出了暗号。"
                    } else {
                        "远行者在城门关闭前必须决定去留。"
                    },
                    "choices": if private_variant {
                        serde_json::json!([
                            {"text": "按暗号潜入内城", "hint": "启用密探身份"},
                            {"text": "销毁暗号离开", "hint": "继续隐藏身份"}
                        ])
                    } else {
                        serde_json::json!([
                            {"text": "留在城内追查", "hint": "接近真相但风险更高"},
                            {"text": "趁夜离开古城", "hint": "避开守卫保存实力"}
                        ])
                    }
                })
                .to_string()
            }
            NarrativeLlmTask::NarrativeTransition => serde_json::json!({
                "schema_version": 1,
                "rendered_narrative": "你留在城内追查线索，守卫的脚步声逐渐逼近。",
                "events": [{
                    "summary": "玩家留在城内继续调查",
                    "actor_character_ids": [],
                    "location_id": null
                }],
                "relationship_changes": [],
                "location_changes": [],
                "thread_changes": []
            })
            .to_string(),
        })
    }
}

#[async_trait]
impl AgentMemoryPort for ToctouFixture {
    async fn save_permanent_memory(
        &self,
        _memory_id: Uuid,
        _character_id: Uuid,
        _user_id: Uuid,
        _novel_id: Uuid,
        _chapter_number: i32,
        _event: &str,
        _importance: i32,
    ) -> Result<()> {
        bail!("unused")
    }
}

#[async_trait]
impl WorldTurnRepository for ToctouFixture {
    async fn begin_turn(&self, claim: &WorldTurnClaim) -> Result<BeginWorldTurn> {
        self.begin_turn_calls.fetch_add(1, Ordering::SeqCst);
        self.last_expected_turn_number
            .store(claim.expected_turn_number, Ordering::SeqCst);
        if self.begin_turn_timeline_conflict.load(Ordering::SeqCst) {
            return Err(
                crate::domain::entities::narrative_node::WorldStateError::TimelineConflict(
                    "durable branch choices do not match the world-state projection".into(),
                )
                .into(),
            );
        }
        if self.world_turn_stale.load(Ordering::SeqCst) {
            return Ok(BeginWorldTurn::Stale);
        }
        if self.world_turn_in_progress.load(Ordering::SeqCst) {
            return Ok(BeginWorldTurn::InProgress {
                retry_after_seconds: 1,
            });
        }
        if self.acquire_next_world_turn.swap(false, Ordering::SeqCst) {
            return Ok(BeginWorldTurn::Acquired {
                claim: Box::new(claim.clone()),
                attempt: 1,
            });
        }
        let result = self
            .completed_world_turn
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("unused"))?;
        ensure!(claim.id == result.turn_id);
        Ok(BeginWorldTurn::Completed {
            result: Box::new(result),
            memory_projection: *self.memory_projection_status.lock().unwrap(),
        })
    }

    async fn renew_turn(&self, _turn_id: Uuid, _attempt: i64) -> Result<bool> {
        bail!("unused")
    }

    async fn complete_turn(
        &self,
        claim: &WorldTurnClaim,
        _attempt: i64,
        _transition: &WorldTurnTransition,
        _context: &WorldEntryContext,
    ) -> Result<WorldTurnResult> {
        self.complete_turn_calls.fetch_add(1, Ordering::SeqCst);
        if self.complete_turn_timeline_conflict.load(Ordering::SeqCst) {
            return Err(
                crate::domain::entities::narrative_node::WorldStateError::TimelineConflict(
                    "durable branch choices do not match the world-state projection".into(),
                )
                .into(),
            );
        }
        let result = self
            .completed_world_turn
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("unused"))?;
        ensure!(claim.id == result.turn_id);
        if self.block_next_complete_turn.swap(false, Ordering::SeqCst) {
            self.complete_turn_entered.notify_one();
            self.complete_turn_release.notified().await;
        }
        Ok(result)
    }

    async fn fail_turn(&self, turn_id: Uuid, attempt: i64, failure_code: &str) -> Result<bool> {
        self.failed_turns
            .lock()
            .unwrap()
            .push((turn_id, attempt, failure_code.into()));
        Ok(true)
    }

    async fn finish_memory_projection(
        &self,
        turn_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        status: MemoryProjectionStatus,
    ) -> Result<bool> {
        self.finish_projection_calls.fetch_add(1, Ordering::SeqCst);
        let result = self.completed_world_turn.lock().unwrap();
        let result = result.as_ref().ok_or_else(|| anyhow::anyhow!("unused"))?;
        ensure!(turn_id == result.turn_id);
        ensure!(user_id == result.world_state.user_id);
        ensure!(novel_id == result.world_state.novel_id);
        *self.memory_projection_status.lock().unwrap() = status;
        Ok(true)
    }

    async fn journal(
        &self,
        _user_id: Uuid,
        _novel_id: Uuid,
        _limit: usize,
    ) -> Result<Vec<WorldTurnJournalEntry>> {
        self.journal_calls.fetch_add(1, Ordering::SeqCst);
        if self.block_next_journal.swap(false, Ordering::SeqCst) {
            self.journal_entered.notify_one();
            self.journal_release.notified().await;
        }
        Ok(self.journal.lock().unwrap().clone())
    }
}

#[tokio::test]
async fn character_identity_rejects_open_world_paths_before_side_effects() {
    let fixture = Arc::new(ToctouFixture::new(false));
    fixture.self_identity.store(false, Ordering::SeqCst);
    fixture
        .block_next_player_entry_context
        .store(true, Ordering::SeqCst);
    let handler = fixture.handler();
    let initial_state = fixture.world_state.lock().unwrap().clone();

    assert!(matches!(
        handler
            .get_player_entry(fixture.user_id, fixture.novel_id, None)
            .await,
        Err(NarrativeError::Conflict(_))
    ));
    assert!(matches!(
        handler
            .create_player_entity(
                fixture.user_id,
                fixture.novel_id,
                CreatePlayerEntityCommand {
                    checkpoint_chapter: Some(fixture.source_chapter),
                    name: "不应创建".into(),
                    background: "角色身份不能创建玩家实体".into(),
                    capabilities: vec!["观察".into()],
                    location_id: "city-gate".into(),
                    inventory: vec![],
                    rules: crate::domain::entities::game_rules::PlayerRuleProfile::narrative(),
                },
            )
            .await,
        Err(NarrativeError::Conflict(_))
    ));
    assert!(matches!(
        handler
            .start_open_world(fixture.user_id, fixture.novel_id)
            .await,
        Err(NarrativeError::Conflict(_))
    ));
    assert!(matches!(
        handler
            .get_open_world(fixture.user_id, fixture.novel_id)
            .await,
        Err(NarrativeError::Conflict(_))
    ));
    assert!(matches!(
        handler
            .submit_world_turn(
                Uuid::new_v4(),
                fixture.user_id,
                fixture.novel_id,
                0,
                WorldAction {
                    kind: WorldActionKind::PursueGoal,
                    target_id: None,
                    intent: "不应执行".into(),
                },
            )
            .await,
        Err(NarrativeError::TurnOutcomeUnknown)
    ));
    assert!(handler
        .get_character_world_context(fixture.user_id, fixture.novel_id, Uuid::new_v4())
        .await
        .unwrap()
        .is_none());

    assert_eq!(*fixture.world_state.lock().unwrap(), initial_state);
    assert!(fixture
        .block_next_player_entry_context
        .load(Ordering::SeqCst));
    assert_eq!(fixture.begin_turn_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.journal_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn character_branch_is_fail_closed_before_provider_or_write() {
    let fixture = Arc::new(ToctouFixture::new(false));
    fixture.clear_world_state();
    {
        let mut state = fixture.world_state.lock().unwrap();
        state.state["relationships"] = serde_json::json!({
            "private-marker": {"score": 99, "last_change": "PRIVATE_RELATIONSHIP_MARKER"}
        });
        state.state["world_events"] = serde_json::json!(["PRIVATE_WORLD_EVENT_MARKER"]);
        state.state["reader_reputation"] =
            serde_json::json!({"private": "PRIVATE_REPUTATION_MARKER"});
    }
    fixture.self_identity.store(false, Ordering::SeqCst);
    let handler = fixture.handler();

    assert!(handler
        .get_branch_node(fixture.novel_id, fixture.source_chapter, fixture.user_id)
        .await
        .unwrap()
        .is_none());
    assert!(fixture.nodes.lock().unwrap().is_empty());
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);

    let node = NarrativeNode::new(
        fixture.novel_id,
        fixture.source_chapter,
        "不得在角色身份创建的新分支".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "不应提交".into(),
            hint: "身份来源未隔离".into(),
            generated_consequence: None,
        }],
    )
    .with_anchor_quote(ANCHOR.into())
    .for_user(fixture.user_id);
    fixture.nodes.lock().unwrap().push(node.clone());
    let error = handler
        .submit_choice(fixture.user_id, fixture.novel_id, node.id, 0)
        .await
        .unwrap_err();
    assert!(matches!(error, NarrativeError::Conflict(_)));
    assert!(fixture.choice.lock().unwrap().is_none());
    assert!(fixture.player_chapter.lock().unwrap().is_none());
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.provider_prompts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn character_identity_never_reuses_an_original_player_continuation() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let private_marker = "PRIVATE_ORIGINAL_PLAYER_CONTINUATION";
    *fixture.player_chapter.lock().unwrap() = Some(PlayerChapter {
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        chapter_number: fixture.source_chapter,
        content: private_marker.into(),
        origin: PlayerChapterOrigin::Continuation,
        created_at: Utc::now(),
    });
    fixture.self_identity.store(false, Ordering::SeqCst);
    let handler = fixture.handler();

    let effective_error = handler
        .get_effective_chapter(fixture.user_id, fixture.novel_id, fixture.source_chapter)
        .await
        .unwrap_err();
    let branch = handler
        .get_branch_node(fixture.novel_id, fixture.source_chapter, fixture.user_id)
        .await
        .unwrap();

    assert!(matches!(effective_error, NarrativeError::Conflict(_)));
    assert!(branch.is_none());
    assert!(!effective_error.to_string().contains(private_marker));
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    assert!(fixture
        .provider_prompts
        .lock()
        .unwrap()
        .iter()
        .all(|prompt| !prompt.contains(private_marker)));
}

#[tokio::test]
async fn cached_original_player_continuation_rechecks_identity_after_the_read() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let private_marker = "PRIVATE_CACHED_CONTINUATION_AFTER_IDENTITY_FLIP";
    *fixture.player_chapter.lock().unwrap() = Some(PlayerChapter {
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        chapter_number: fixture.source_chapter,
        content: private_marker.into(),
        origin: PlayerChapterOrigin::Continuation,
        created_at: Utc::now(),
    });
    fixture
        .block_next_player_chapter_read
        .store(true, Ordering::SeqCst);
    let handler = fixture.handler();
    let request = {
        let fixture = fixture.clone();
        tokio::spawn(async move {
            handler
                .get_effective_chapter(fixture.user_id, fixture.novel_id, fixture.source_chapter)
                .await
        })
    };

    fixture.player_chapter_read_entered.notified().await;
    fixture.self_identity.store(false, Ordering::SeqCst);
    fixture.player_chapter_read_release.notify_one();
    let error = request.await.unwrap().unwrap_err();

    assert!(matches!(error, NarrativeError::Conflict(_)));
    assert!(!error.to_string().contains(private_marker));
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn effective_chapter_hides_a_cached_continuation_above_current_progress() {
    let fixture = Arc::new(ToctouFixture::at_chapter(false, 1));
    fixture.available_chapters.lock().unwrap().push(5);
    let private_marker = "PRIVATE_CHAPTER_FIVE_CONTINUATION";
    *fixture.player_chapter.lock().unwrap() = Some(PlayerChapter {
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        chapter_number: 5,
        content: private_marker.into(),
        origin: PlayerChapterOrigin::Continuation,
        created_at: Utc::now(),
    });

    let chapter = fixture
        .handler()
        .get_effective_chapter(fixture.user_id, fixture.novel_id, 5)
        .await
        .unwrap();

    assert!(!chapter.generated);
    assert!(!chapter.content.contains(private_marker));
    assert_eq!(chapter.content, ToctouFixture::chapter().content);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn future_branch_request_is_rejected_before_provider_or_node_write() {
    let fixture = Arc::new(ToctouFixture::at_chapter(false, 1));

    let error = fixture
        .handler()
        .get_branch_node(fixture.novel_id, 5, fixture.user_id)
        .await
        .unwrap_err();

    assert!(matches!(error, NarrativeError::ReadingProgressBehindWorld));
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.nodes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn continuation_generation_rechecks_progress_before_save_and_response() {
    let fixture = Arc::new(ToctouFixture::at_chapter(true, 1));
    fixture.available_chapters.lock().unwrap().push(2);
    fixture.current_chapter.store(2, Ordering::SeqCst);
    let node = fixture.nodes.lock().unwrap()[0].clone();
    let transition = NarrativeTransition {
        schema_version: 1,
        prompt_version: "narrative-transition-v1".into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: 1,
        rendered_narrative: "玩家在第一章作出了选择。".into(),
        events: vec![],
        relationship_changes: vec![],
        location_changes: vec![],
        thread_changes: vec![],
    };
    let created_at = Utc::now();
    *fixture.choice.lock().unwrap() = Some(UserChoiceRecord {
        id: Uuid::new_v4(),
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        node_id: node.id,
        chapter_number: 1,
        choice_index: 0,
        choice_text: node.choices[0].text.clone(),
        consequence: transition.rendered_narrative.clone(),
        transition,
        created_at,
    });
    *fixture.player_chapter.lock().unwrap() = Some(PlayerChapter {
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        chapter_number: 1,
        content: format!("{ANCHOR}\n\n玩家在第一章作出了选择。"),
        origin: PlayerChapterOrigin::Choice,
        created_at,
    });
    let handler = fixture.handler();
    let request = {
        let fixture = fixture.clone();
        tokio::spawn(async move {
            handler
                .get_effective_chapter(fixture.user_id, fixture.novel_id, 2)
                .await
        })
    };

    fixture.provider_entered.notified().await;
    fixture.current_chapter.store(1, Ordering::SeqCst);
    fixture.provider_release.notify_one();
    let chapter = request.await.unwrap().unwrap();

    assert!(!chapter.generated);
    assert_eq!(chapter.content, ToctouFixture::chapter().content);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    let stored = fixture.player_chapter.lock().unwrap().clone().unwrap();
    assert_eq!(stored.chapter_number, 1);
    assert_eq!(stored.origin, PlayerChapterOrigin::Choice);
}

#[tokio::test]
async fn character_choice_recovery_ignores_hidden_open_world_high_water_and_switching_back_resumes()
{
    let fixture = Arc::new(ToctouFixture::new(true));
    let character_id = Uuid::new_v4();
    let node = fixture.nodes.lock().unwrap()[0].clone();
    let context = fixture.entry_context(5, Some(character_id));
    let original_player_id = fixture
        .world_state
        .lock()
        .unwrap()
        .player_entity()
        .unwrap()
        .unwrap()
        .id;
    {
        let mut state = fixture.world_state.lock().unwrap();
        state.start_open_world(&context).unwrap();
        state
            .record_choice(
                node.id,
                node.chapter_number,
                0,
                &node.choices[0].text,
                "已提交分支后果",
            )
            .unwrap();
        state.state["relationships"] = serde_json::json!({
            character_id.to_string(): {"score": 88, "last_change": "PRIVATE_WORLD_MARKER"}
        });
    }
    let transition = NarrativeTransition {
        schema_version: 1,
        prompt_version: "narrative-transition-v1".into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: fixture.source_chapter,
        rendered_narrative: "已提交分支后果".into(),
        events: vec![],
        relationship_changes: vec![],
        location_changes: vec![],
        thread_changes: vec![],
    };
    let created_at = Utc::now();
    *fixture.choice.lock().unwrap() = Some(UserChoiceRecord {
        id: Uuid::new_v4(),
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        node_id: node.id,
        chapter_number: node.chapter_number,
        choice_index: 0,
        choice_text: node.choices[0].text.clone(),
        consequence: transition.rendered_narrative.clone(),
        transition: transition.clone(),
        created_at,
    });
    *fixture.player_chapter.lock().unwrap() = Some(PlayerChapter {
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        chapter_number: node.chapter_number,
        content: format!("{ANCHOR}\n\n{}", transition.rendered_narrative),
        origin: PlayerChapterOrigin::Choice,
        created_at,
    });

    fixture.self_identity.store(false, Ordering::SeqCst);
    let handler = fixture.handler();
    let choices_only = handler
        .get_world_state(fixture.user_id, fixture.novel_id)
        .await
        .unwrap();
    assert_eq!(
        choices_only.source_chapter_high_water().unwrap(),
        Some(fixture.source_chapter)
    );
    assert!(choices_only.state.get("player_entity").is_none());
    assert!(choices_only.state.get("open_world").is_none());
    assert!(!serde_json::to_string(&choices_only)
        .unwrap()
        .contains("PRIVATE_WORLD_MARKER"));
    assert!(handler
        .get_character_world_context(fixture.user_id, fixture.novel_id, character_id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(fixture.journal_calls.load(Ordering::SeqCst), 0);

    let committed_node = handler
        .get_branch_node(fixture.novel_id, node.chapter_number, fixture.user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(committed_node.id, node.id);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);

    let replay = handler
        .submit_choice(fixture.user_id, fixture.novel_id, node.id, 0)
        .await
        .unwrap();
    assert_eq!(replay.transition, transition);
    assert!(replay.world_state.state.get("open_world").is_none());
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);

    fixture.self_identity.store(true, Ordering::SeqCst);
    fixture.current_chapter.store(5, Ordering::SeqCst);
    let restored_player = handler
        .get_player_entry(fixture.user_id, fixture.novel_id, None)
        .await
        .unwrap()
        .player
        .unwrap();
    assert_eq!(restored_player.id, original_player_id);
    let restored_world = handler
        .get_open_world(fixture.user_id, fixture.novel_id)
        .await
        .unwrap();
    assert_eq!(restored_world.player.id, original_player_id);
    assert_eq!(restored_world.session.entry_context, context);
    assert!(handler
        .get_character_world_context(fixture.user_id, fixture.novel_id, character_id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(fixture.journal_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn branch_nodes_are_private_to_player_world_and_deviation_mode() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let handler = fixture.handler();

    fixture.provider_release.notify_one();
    let first = handler
        .get_branch_node(fixture.novel_id, fixture.source_chapter, fixture.user_id)
        .await
        .unwrap()
        .unwrap();
    fixture.provider_release.notify_one();
    let second = handler
        .get_branch_node(
            fixture.novel_id,
            fixture.source_chapter,
            fixture.other_user_id,
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first.user_id, Some(fixture.user_id));
    assert_eq!(second.user_id, Some(fixture.other_user_id));
    assert_ne!(first.id, second.id);
    assert_ne!(first.description, second.description);
    assert_ne!(first.choices[0].text, second.choices[0].text);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 2);
    let prompts = fixture.provider_prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].contains("云舟"));
    assert!(prompts[0].contains("canon"));
    assert!(!prompts[0].contains("只向自己公开身份的旧廷密探"));
    assert!(prompts[1].contains("霜璃"));
    assert!(prompts[1].contains("只向自己公开身份的旧廷密探"));
    assert!(prompts[1].contains("creative"));
}

#[tokio::test]
async fn uncommitted_legacy_shared_node_cannot_start_a_choice() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let shared = NarrativeNode::new(
        fixture.novel_id,
        fixture.source_chapter,
        "遗留共享节点".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "不应生成后果".into(),
            hint: "不可提交".into(),
            generated_consequence: None,
        }],
    )
    .with_anchor_quote(ANCHOR.into());
    fixture.nodes.lock().unwrap().push(shared.clone());

    let error = fixture
        .handler()
        .submit_choice(fixture.user_id, fixture.novel_id, shared.id, 0)
        .await
        .unwrap_err();

    assert!(matches!(error, NarrativeError::NotFound));
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.choice.lock().unwrap().is_none());
}

#[tokio::test]
async fn committed_legacy_shared_node_replays_exactly_without_provider() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let shared = NarrativeNode::new(
        fixture.novel_id,
        fixture.source_chapter,
        "遗留共享节点".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "留在城内追查线索".into(),
            hint: "旧选项".into(),
            generated_consequence: None,
        }],
    )
    .with_anchor_quote(ANCHOR.into());
    fixture.nodes.lock().unwrap().push(shared.clone());
    let transition = NarrativeTransition {
        schema_version: 1,
        prompt_version: "narrative-transition-v1".into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: fixture.source_chapter,
        rendered_narrative: "已经提交的旧分支后果。".into(),
        events: vec![],
        relationship_changes: vec![],
        location_changes: vec![],
        thread_changes: vec![],
    };
    let created_at = Utc::now();
    *fixture.choice.lock().unwrap() = Some(UserChoiceRecord {
        id: Uuid::new_v4(),
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        node_id: shared.id,
        chapter_number: shared.chapter_number,
        choice_index: 0,
        choice_text: shared.choices[0].text.clone(),
        consequence: transition.rendered_narrative.clone(),
        transition: transition.clone(),
        created_at,
    });
    *fixture.player_chapter.lock().unwrap() = Some(PlayerChapter {
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        chapter_number: shared.chapter_number,
        content: format!("{ANCHOR}\n\n{}", transition.rendered_narrative),
        origin: PlayerChapterOrigin::Choice,
        created_at,
    });

    let replay = fixture
        .handler()
        .submit_choice(fixture.user_id, fixture.novel_id, shared.id, 0)
        .await
        .unwrap();

    assert_eq!(replay.consequence, transition.rendered_narrative);
    assert_eq!(replay.transition, transition);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn choice_commit_rechecks_progress_and_exact_replay_uses_no_provider() {
    let fixture = Arc::new(ToctouFixture::new(true));
    let handler = Arc::new(fixture.handler());
    let node_id = fixture.nodes.lock().unwrap()[0].id;
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let first_handler = handler.clone();
    let first = tokio::spawn(async move {
        first_handler
            .submit_choice(user_id, novel_id, node_id, 0)
            .await
    });

    fixture.wait_for_provider().await;
    fixture.current_chapter.store(1, Ordering::SeqCst);
    fixture.provider_release.notify_one();

    let error = first.await.unwrap().unwrap_err();
    assert!(matches!(error, NarrativeError::ReadingProgressBehindWorld));
    let committed = fixture.choice.lock().unwrap().clone().unwrap();
    let committed_chapter = fixture.player_chapter.lock().unwrap().clone().unwrap();
    assert_eq!(committed.choice_index, 0);
    assert_eq!(committed_chapter.origin, PlayerChapterOrigin::Choice);

    fixture.current_chapter.store(2, Ordering::SeqCst);
    let replay = handler
        .submit_choice(user_id, novel_id, node_id, 0)
        .await
        .unwrap();

    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(replay.consequence, committed.consequence);
    assert_eq!(replay.transition, committed.transition);
    assert_eq!(replay.chapter_content, committed_chapter.content);
    assert_eq!(replay.world_state, *fixture.world_state.lock().unwrap());
}

#[tokio::test]
async fn committed_character_branch_rechecks_its_own_source_chapter() {
    let fixture = Arc::new(ToctouFixture::at_chapter(true, 5));
    fixture.clear_world_state();
    fixture.self_identity.store(false, Ordering::SeqCst);
    let node_id = fixture.nodes.lock().unwrap()[0].id;
    let transition = NarrativeTransition {
        schema_version: 1,
        prompt_version: "narrative-transition-v1".into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: fixture.source_chapter,
        rendered_narrative: "已提交分支。".into(),
        events: vec![],
        relationship_changes: vec![],
        location_changes: vec![],
        thread_changes: vec![],
    };
    *fixture.choice.lock().unwrap() = Some(UserChoiceRecord {
        id: Uuid::new_v4(),
        user_id: fixture.user_id,
        novel_id: fixture.novel_id,
        node_id,
        chapter_number: fixture.source_chapter,
        choice_index: 0,
        choice_text: "留在城内追查线索".into(),
        consequence: transition.rendered_narrative.clone(),
        transition,
        created_at: Utc::now(),
    });
    fixture.block_next_node_read.store(true, Ordering::SeqCst);
    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let source_chapter = fixture.source_chapter;
    let first_handler = handler.clone();
    let in_flight = tokio::spawn(async move {
        first_handler
            .get_branch_node(novel_id, source_chapter, user_id)
            .await
    });

    fixture.wait_for_node_read().await;
    fixture.current_chapter.store(1, Ordering::SeqCst);
    fixture.node_read_release.notify_one();

    assert!(matches!(
        in_flight.await.unwrap(),
        Err(NarrativeError::ReadingProgressBehindWorld)
    ));
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn branch_generation_rechecks_progress_before_returning_options() {
    let fixture = Arc::new(ToctouFixture::at_chapter(false, 5));
    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let source_chapter = fixture.source_chapter;
    let first_handler = handler.clone();
    let first = tokio::spawn(async move {
        first_handler
            .get_branch_node(novel_id, source_chapter, user_id)
            .await
    });

    fixture.wait_for_provider().await;
    fixture.current_chapter.store(1, Ordering::SeqCst);
    fixture.provider_release.notify_one();

    let error = first.await.unwrap().unwrap_err();
    assert!(matches!(error, NarrativeError::ReadingProgressBehindWorld));
    let durable_node = fixture.nodes.lock().unwrap()[0].clone();

    fixture.current_chapter.store(5, Ordering::SeqCst);
    let replay = handler
        .get_branch_node(novel_id, 5, user_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(replay.id, durable_node.id);
    assert_eq!(replay.choices.len(), 2);
}

#[tokio::test]
async fn cached_branch_node_is_hidden_when_open_world_starts_during_the_read() {
    let fixture = Arc::new(ToctouFixture::new(true));
    fixture.block_next_node_read.store(true, Ordering::SeqCst);
    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let source_chapter = fixture.source_chapter;
    let request = tokio::spawn(async move {
        handler
            .get_branch_node(novel_id, source_chapter, user_id)
            .await
    });

    fixture.wait_for_node_read().await;
    let context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: source_chapter,
        unlocked_through_chapter: source_chapter,
        characters: vec![],
        locations: vec![],
        factions: vec![],
        hard_rules: vec![],
        dead_character_ids: vec![],
        threads: vec![],
        scheduled_events: vec![],
        character_goals: vec![],
    };
    fixture
        .world_state
        .lock()
        .unwrap()
        .start_open_world(&context)
        .unwrap();
    fixture.node_read_release.notify_one();

    assert!(request.await.unwrap().unwrap().is_none());
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cached_branch_node_is_hidden_when_player_checkpoint_seals_during_the_read() {
    let fixture = Arc::new(ToctouFixture::new(true));
    fixture.block_next_node_read.store(true, Ordering::SeqCst);
    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let source_chapter = fixture.source_chapter;
    let request = tokio::spawn(async move {
        handler
            .get_branch_node(novel_id, source_chapter, user_id)
            .await
    });

    fixture.wait_for_node_read().await;
    let sealed_player = PlayerEntity::new(
        fixture.user_id,
        fixture.novel_id,
        source_chapter - 1,
        "云舟".into(),
        "远行者".into(),
        vec!["观察".into()],
        "city-gate".into(),
        vec![],
    )
    .unwrap();
    fixture.world_state.lock().unwrap().state["player_entity"] =
        serde_json::to_value(sealed_player).unwrap();
    fixture.node_read_release.notify_one();

    assert!(request.await.unwrap().unwrap().is_none());
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn generated_branch_node_is_hidden_when_a_choice_commits_during_provider_call() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let source_chapter = fixture.source_chapter;
    let request = tokio::spawn(async move {
        handler
            .get_branch_node(novel_id, source_chapter, user_id)
            .await
    });

    fixture.wait_for_provider().await;
    fixture.world_state.lock().unwrap().state["choices"] = serde_json::json!([{
        "node_id": Uuid::new_v4(),
        "chapter": source_chapter,
        "choice_index": 0,
        "choice": "另一标签页已经提交"
    }]);
    fixture.provider_release.notify_one();

    assert!(request.await.unwrap().unwrap().is_none());
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    let durable = fixture.nodes.lock().unwrap();
    assert_eq!(durable.len(), 1);
    assert_eq!(durable[0].user_id, Some(fixture.user_id));
}

#[tokio::test]
async fn player_entry_rechecks_context_checkpoint_before_returning_locations() {
    let fixture = Arc::new(ToctouFixture::at_chapter(false, 5));
    fixture.clear_world_state();
    *fixture.player_entry_context.lock().unwrap() = Some(PlayerEntryContext {
        checkpoint_chapter: 5,
        name_available: true,
        locations: vec![CanonEntityRef {
            id: "city-gate".into(),
            name: "暮城门".into(),
        }],
    });
    fixture
        .block_next_player_entry_context
        .store(true, Ordering::SeqCst);
    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let first_handler = handler.clone();
    let in_flight = tokio::spawn(async move {
        first_handler
            .get_player_entry(user_id, novel_id, None)
            .await
    });

    fixture.wait_for_player_entry_context().await;
    fixture.current_chapter.store(1, Ordering::SeqCst);
    fixture.player_entry_context_release.notify_one();

    let error = in_flight.await.unwrap().unwrap_err();
    assert!(matches!(error, NarrativeError::ReadingProgressBehindWorld));

    fixture.current_chapter.store(5, Ordering::SeqCst);
    let restored = handler
        .get_player_entry(user_id, novel_id, None)
        .await
        .unwrap();
    assert_eq!(restored.checkpoint_chapter, 5);
    assert_eq!(restored.locations[0].id, "city-gate");
}

#[tokio::test]
async fn open_world_view_rechecks_progress_after_loading_the_journal() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let entry_context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 2,
        unlocked_through_chapter: 2,
        characters: vec![],
        locations: vec![],
        factions: vec![],
        hard_rules: vec![],
        dead_character_ids: vec![],
        threads: vec![],
        scheduled_events: vec![],
        character_goals: vec![],
    };
    fixture
        .world_state
        .lock()
        .unwrap()
        .start_open_world(&entry_context)
        .unwrap();
    let durable_state = fixture.world_state.lock().unwrap().clone();
    fixture.block_next_journal.store(true, Ordering::SeqCst);
    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let first_handler = handler.clone();
    let in_flight =
        tokio::spawn(async move { first_handler.get_open_world(user_id, novel_id).await });

    fixture.wait_for_journal().await;
    fixture.current_chapter.store(1, Ordering::SeqCst);
    fixture.journal_release.notify_one();

    let error = in_flight.await.unwrap().unwrap_err();
    assert!(matches!(error, NarrativeError::ReadingProgressBehindWorld));
    assert_eq!(*fixture.world_state.lock().unwrap(), durable_state);

    fixture.current_chapter.store(2, Ordering::SeqCst);
    let restored = handler.get_open_world(user_id, novel_id).await.unwrap();

    assert_eq!(fixture.journal_calls.load(Ordering::SeqCst), 2);
    assert_eq!(restored.world_state, durable_state);
    assert_eq!(restored.session.entry_context, entry_context);
}

#[tokio::test]
async fn open_world_view_does_not_mix_a_future_journal_with_an_older_state_snapshot() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let entry_context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 2,
        unlocked_through_chapter: 2,
        characters: vec![],
        locations: vec![],
        factions: vec![],
        hard_rules: vec![],
        dead_character_ids: vec![],
        threads: vec![],
        scheduled_events: vec![],
        character_goals: vec![],
    };
    {
        let mut state = fixture.world_state.lock().unwrap();
        state.start_open_world(&entry_context).unwrap();
        let mut session = state.open_world().unwrap().unwrap();
        session.turn_number = 1;
        session.world_time = 1;
        state.state["open_world"] = serde_json::to_value(session).unwrap();
    }
    fixture
        .journal
        .lock()
        .unwrap()
        .push(ToctouFixture::journal_entry(1));
    fixture.block_next_journal.store(true, Ordering::SeqCst);

    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let first_handler = handler.clone();
    let in_flight =
        tokio::spawn(async move { first_handler.get_open_world(user_id, novel_id).await });

    fixture.wait_for_journal().await;
    fixture
        .journal
        .lock()
        .unwrap()
        .push(ToctouFixture::journal_entry(2));
    fixture.journal_release.notify_one();

    let view = in_flight.await.unwrap().unwrap();
    assert_eq!(view.session.turn_number, 1);
    assert_eq!(
        view.journal
            .iter()
            .map(|entry| entry.turn_number)
            .collect::<Vec<_>>(),
        vec![1]
    );
}

#[tokio::test]
async fn pending_projection_barrier_stops_new_world_turn_before_provider() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let entry_context = fixture.entry_context(fixture.source_chapter, None);
    fixture
        .world_state
        .lock()
        .unwrap()
        .start_open_world(&entry_context)
        .unwrap();
    fixture.world_turn_in_progress.store(true, Ordering::SeqCst);

    let error = fixture
        .handler()
        .submit_world_turn(
            Uuid::new_v4(),
            fixture.user_id,
            fixture.novel_id,
            0,
            WorldAction {
                kind: WorldActionKind::PursueGoal,
                target_id: None,
                intent: "继续寻找信使".into(),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        NarrativeError::TurnInProgress {
            retry_after_seconds: 1
        }
    ));
    assert_eq!(fixture.begin_turn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.journal_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.complete_turn_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn choice_projection_conflict_before_reservation_is_a_typed_conflict() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let entry_context = fixture.entry_context(fixture.source_chapter, None);
    fixture
        .world_state
        .lock()
        .unwrap()
        .start_open_world(&entry_context)
        .unwrap();
    fixture
        .begin_turn_timeline_conflict
        .store(true, Ordering::SeqCst);

    let error = fixture
        .handler()
        .submit_world_turn(
            Uuid::new_v4(),
            fixture.user_id,
            fixture.novel_id,
            0,
            WorldAction {
                kind: WorldActionKind::PursueGoal,
                target_id: None,
                intent: "不应进入 provider".into(),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        NarrativeError::Conflict(message)
            if message == "durable branch choices do not match the world-state projection"
    ));
    assert_eq!(fixture.begin_turn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.complete_turn_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.failed_turns.lock().unwrap().is_empty());
}

#[tokio::test]
async fn choice_projection_conflict_at_commit_is_fenced_before_typed_conflict() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let entry_context = fixture.entry_context(fixture.source_chapter, None);
    fixture
        .world_state
        .lock()
        .unwrap()
        .start_open_world(&entry_context)
        .unwrap();
    fixture
        .acquire_next_world_turn
        .store(true, Ordering::SeqCst);
    fixture
        .complete_turn_timeline_conflict
        .store(true, Ordering::SeqCst);
    let turn_id = Uuid::new_v4();
    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let in_flight = tokio::spawn(async move {
        handler
            .submit_world_turn(
                turn_id,
                user_id,
                novel_id,
                0,
                WorldAction {
                    kind: WorldActionKind::PursueGoal,
                    target_id: None,
                    intent: "该回合会在提交边界发现旧选择".into(),
                },
            )
            .await
    });
    fixture.wait_for_provider().await;
    fixture.provider_release.notify_one();

    let error = in_flight.await.unwrap().unwrap_err();
    assert!(matches!(
        error,
        NarrativeError::Conflict(message)
            if message == "durable branch choices do not match the world-state projection"
    ));
    assert_eq!(fixture.begin_turn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.complete_turn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        *fixture.failed_turns.lock().unwrap(),
        vec![(turn_id, 1, "commit_error".into())]
    );
}

#[tokio::test]
async fn client_world_revision_is_the_claim_and_stale_requests_stop_before_provider() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let entry_context = fixture.entry_context(fixture.source_chapter, None);
    fixture
        .world_state
        .lock()
        .unwrap()
        .start_open_world(&entry_context)
        .unwrap();
    fixture.world_turn_stale.store(true, Ordering::SeqCst);

    let error = fixture
        .handler()
        .submit_world_turn(
            Uuid::new_v4(),
            fixture.user_id,
            fixture.novel_id,
            7,
            WorldAction {
                kind: WorldActionKind::PursueGoal,
                target_id: None,
                intent: "旧标签页仍停留在先前世界版本".into(),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(error, NarrativeError::Conflict(_)));
    assert_eq!(fixture.last_expected_turn_number.load(Ordering::SeqCst), 7);
    assert_eq!(fixture.begin_turn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.journal_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.complete_turn_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn inconsistent_world_entry_checkpoint_stops_new_turn_before_provider_or_commit() {
    let fixture = Arc::new(ToctouFixture::at_chapter(false, 1));
    let entry_context = fixture.entry_context(1, None);
    {
        let mut state = fixture.world_state.lock().unwrap();
        state.start_open_world(&entry_context).unwrap();
        state.state["choices"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "node_id": Uuid::new_v4(),
                "chapter": 2,
                "choice_index": 0,
                "choice": "来自不一致旧前缀的未来选择",
                "consequence": "不应成为第一章旅程事实",
            }));
    }
    fixture.current_chapter.store(2, Ordering::SeqCst);
    fixture
        .acquire_next_world_turn
        .store(true, Ordering::SeqCst);

    let error = fixture
        .handler()
        .submit_world_turn(
            Uuid::new_v4(),
            fixture.user_id,
            fixture.novel_id,
            0,
            WorldAction {
                kind: WorldActionKind::PursueGoal,
                target_id: None,
                intent: "继续第一章的旅程".into(),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        NarrativeError::Conflict(message)
            if message == "world entry checkpoint precedes a committed branch choice"
    ));
    assert_eq!(fixture.begin_turn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.journal_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.complete_turn_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn post_commit_identity_flip_is_outcome_unknown_and_same_key_replays() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let entry_context = fixture.entry_context(fixture.source_chapter, None);
    fixture
        .world_state
        .lock()
        .unwrap()
        .start_open_world(&entry_context)
        .unwrap();
    let turn_id = Uuid::new_v4();
    let action = WorldAction {
        kind: WorldActionKind::PursueGoal,
        target_id: None,
        intent: "寻找失踪的信使".into(),
    };
    let transition = WorldTurnTransition {
        schema_version: 1,
        prompt_version: "world-turn-v2".into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: fixture.source_chapter,
        rendered_narrative: "你留在城内追查线索，守卫的脚步声逐渐逼近。".into(),
        events: vec![TransitionEvent {
            summary: "玩家继续追查失踪的信使".into(),
            actor_character_ids: vec![],
            location_id: None,
        }],
        relationship_changes: vec![],
        location_changes: vec![],
        thread_changes: vec![],
        player_location_id: None,
        inventory_additions: vec![],
        inventory_removals: vec![],
        knowledge_discoveries: vec![],
        faction_changes: vec![],
        canonical_event_change: None,
    };
    let mut committed_state = fixture.world_state.lock().unwrap().clone();
    committed_state
        .apply_world_turn(turn_id, &action, &transition, &entry_context)
        .unwrap();
    *fixture.completed_world_turn.lock().unwrap() = Some(WorldTurnResult {
        turn_id,
        action: action.clone(),
        resolution: None,
        transition,
        world_state: committed_state,
    });
    fixture
        .acquire_next_world_turn
        .store(true, Ordering::SeqCst);
    fixture
        .block_next_complete_turn
        .store(true, Ordering::SeqCst);

    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let first_handler = handler.clone();
    let first_action = action.clone();
    let first = tokio::spawn(async move {
        first_handler
            .submit_world_turn(turn_id, user_id, novel_id, 0, first_action)
            .await
    });

    fixture.wait_for_provider().await;
    fixture.provider_release.notify_one();
    fixture.wait_for_complete_turn().await;
    fixture.self_identity.store(false, Ordering::SeqCst);
    fixture.complete_turn_release.notify_one();

    assert!(matches!(
        first.await.unwrap(),
        Err(NarrativeError::TurnOutcomeUnknown)
    ));
    assert_eq!(
        *fixture.memory_projection_status.lock().unwrap(),
        MemoryProjectionStatus::Pending
    );
    assert_eq!(fixture.finish_projection_calls.load(Ordering::SeqCst), 0);

    assert!(matches!(
        handler
            .submit_world_turn(turn_id, user_id, novel_id, 0, action.clone())
            .await,
        Err(NarrativeError::TurnOutcomeUnknown)
    ));
    assert_eq!(fixture.begin_turn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);

    fixture.self_identity.store(true, Ordering::SeqCst);
    let replay = handler
        .submit_world_turn(turn_id, user_id, novel_id, 0, action)
        .await
        .unwrap();
    assert_eq!(
        replay.memory_projection_status,
        MemoryProjectionStatus::Skipped
    );
    assert_eq!(fixture.begin_turn_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.complete_turn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pending_projection_stays_pending_if_progress_rewinds_before_acknowledgement() {
    let fixture = Arc::new(ToctouFixture::new(false));
    let entry_context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 2,
        unlocked_through_chapter: 2,
        characters: vec![],
        locations: vec![],
        factions: vec![],
        hard_rules: vec![],
        dead_character_ids: vec![],
        threads: vec![],
        scheduled_events: vec![],
        character_goals: vec![],
    };
    fixture
        .world_state
        .lock()
        .unwrap()
        .start_open_world(&entry_context)
        .unwrap();
    let turn_id = Uuid::new_v4();
    let action = WorldAction {
        kind: WorldActionKind::PursueGoal,
        target_id: None,
        intent: "寻找失踪的信使".into(),
    };
    *fixture.completed_world_turn.lock().unwrap() = Some(WorldTurnResult {
        turn_id,
        action: action.clone(),
        resolution: None,
        transition: WorldTurnTransition {
            schema_version: 1,
            prompt_version: "world-turn-v2".into(),
            canon_model_version: 1,
            canonical_checkpoint_chapter: 2,
            rendered_narrative: "已提交但尚未确认记忆投影。".into(),
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
        world_state: fixture.world_state.lock().unwrap().clone(),
    });
    fixture
        .block_next_character_list
        .store(true, Ordering::SeqCst);

    let handler = Arc::new(fixture.handler());
    let user_id = fixture.user_id;
    let novel_id = fixture.novel_id;
    let first_handler = handler.clone();
    let first_action = action.clone();
    let in_flight = tokio::spawn(async move {
        first_handler
            .submit_world_turn(turn_id, user_id, novel_id, 0, first_action)
            .await
    });

    fixture.wait_for_character_list().await;
    fixture.current_chapter.store(1, Ordering::SeqCst);
    fixture.character_list_release.notify_one();

    let error = in_flight.await.unwrap().unwrap_err();
    assert!(matches!(error, NarrativeError::TurnOutcomeUnknown));
    assert_eq!(
        *fixture.memory_projection_status.lock().unwrap(),
        MemoryProjectionStatus::Pending
    );
    assert_eq!(fixture.finish_projection_calls.load(Ordering::SeqCst), 0);

    fixture.current_chapter.store(2, Ordering::SeqCst);
    let replay = handler
        .submit_world_turn(turn_id, user_id, novel_id, 0, action)
        .await
        .unwrap();

    assert_eq!(
        replay.memory_projection_status,
        MemoryProjectionStatus::Skipped
    );
    assert_eq!(
        *fixture.memory_projection_status.lock().unwrap(),
        MemoryProjectionStatus::Skipped
    );
    assert_eq!(fixture.begin_turn_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.finish_projection_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.complete_turn_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}
