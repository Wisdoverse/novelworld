use crate::application::handlers::{
    journey_memory_id, record_world_journey_memory, resolve_protagonist,
};
use crate::domain::entities::game_rules::GameRuleTemplate;
use crate::domain::entities::narrative_node::{NarrativeChoice, NarrativeNode, WorldState};
use crate::domain::entities::player_entity::PlayerEntity;
use crate::domain::entities::world_session::{
    WorldAction, WorldActionKind, WorldEntryContext, WorldSession, WorldTurnTransition,
};
use crate::domain::ports::AgentMemoryPort;
use crate::domain::repositories::{
    ChapterInfo, ChapterReadRepository, CharacterBrief, NovelInfo, PlayerEntryContext,
    WorldTurnResult,
};
use crate::domain::services::narrative_transition::{
    CanonContext, NarrativeTransition, RelationshipChange, ThreadChange, ThreadStatus,
    TransitionEvent, TRANSITION_PROMPT_VERSION, TRANSITION_SCHEMA_VERSION,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedMemoryCall {
    memory_id: Uuid,
    character_id: Uuid,
    user_id: Uuid,
    novel_id: Uuid,
    chapter_number: i32,
    event: String,
    importance: i32,
}

struct RecordingAgentMemory {
    calls: Mutex<Vec<RecordedMemoryCall>>,
    failures_remaining: AtomicUsize,
}

#[async_trait]
impl AgentMemoryPort for RecordingAgentMemory {
    async fn save_permanent_memory(
        &self,
        memory_id: Uuid,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        event: &str,
        importance: i32,
    ) -> Result<()> {
        if self.failures_remaining.fetch_sub(1, Ordering::SeqCst) > 0 {
            return Err(anyhow!("agent unavailable"));
        }
        self.calls.lock().unwrap().push(RecordedMemoryCall {
            memory_id,
            character_id,
            user_id,
            novel_id,
            chapter_number,
            event: event.to_string(),
            importance,
        });
        Ok(())
    }
}

struct FixedChapterRepo {
    characters: Vec<CharacterBrief>,
}

fn journey_result(
    user_id: Uuid,
    novel_id: Uuid,
    witness_character_id: Option<Uuid>,
    source_chapter_high_water: i32,
) -> WorldTurnResult {
    let mut world_state = WorldState::new(user_id, novel_id);
    let entry_context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 1,
        unlocked_through_chapter: source_chapter_high_water,
        characters: vec![],
        locations: vec![],
        factions: vec![],
        hard_rules: vec![],
        dead_character_ids: vec![],
        threads: vec![],
        scheduled_events: vec![],
        character_goals: vec![],
    };
    world_state.state["open_world"] =
        serde_json::to_value(WorldSession::from_context(&entry_context).unwrap()).unwrap();
    WorldTurnResult {
        turn_id: Uuid::new_v4(),
        action: WorldAction {
            kind: WorldActionKind::Converse,
            target_id: witness_character_id.map(|id| id.to_string()),
            intent: "主角踏上了北境之路".into(),
        },
        resolution: None,
        transition: WorldTurnTransition {
            schema_version: 1,
            prompt_version: "world-turn-v2".into(),
            canon_model_version: 1,
            canonical_checkpoint_chapter: 1,
            rendered_narrative: "生成叙事".into(),
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
        world_state,
    }
}

#[async_trait]
impl ChapterReadRepository for FixedChapterRepo {
    async fn get_chapter(
        &self,
        _novel_id: Uuid,
        _chapter_number: i32,
        _user_id: Uuid,
    ) -> Result<Option<ChapterInfo>> {
        Ok(None)
    }

    async fn get_novel_info(&self, _novel_id: Uuid, _user_id: Uuid) -> Result<Option<NovelInfo>> {
        Ok(None)
    }

    async fn get_canon_context(
        &self,
        _novel_id: Uuid,
        _checkpoint_chapter: i32,
        _user_id: Uuid,
    ) -> Result<Option<CanonContext>> {
        Ok(None)
    }

    async fn get_current_chapter(&self, _novel_id: Uuid, _user_id: Uuid) -> Result<i32> {
        Ok(i32::MAX)
    }

    async fn list_characters(
        &self,
        _novel_id: Uuid,
        _user_id: Uuid,
    ) -> Result<Vec<CharacterBrief>> {
        Ok(self.characters.clone())
    }

    async fn get_player_entry_context(
        &self,
        _novel_id: Uuid,
        _user_id: Uuid,
        _checkpoint_chapter: Option<i32>,
        _proposed_name: Option<&str>,
    ) -> Result<Option<PlayerEntryContext>> {
        Ok(None)
    }

    async fn reader_identity_is_self(&self, _novel_id: Uuid, _user_id: Uuid) -> Result<bool> {
        Ok(false)
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
        GameRuleTemplate,
        crate::domain::repositories::GameRuleTemplateRequestError,
    > {
        Err(
            crate::domain::repositories::GameRuleTemplateRequestError::Unavailable(anyhow!(
                "game rules are not configured in this test"
            )),
        )
    }

    async fn get_game_rule_template(
        &self,
        _novel_id: Uuid,
        _canon_model_version: i32,
        _user_id: Uuid,
    ) -> Result<Option<GameRuleTemplate>> {
        Ok(None)
    }
}

#[test]
fn protagonist_resolution_is_deterministic_and_fallbacks_to_none() {
    let lead = Uuid::new_v4();
    let second = Uuid::new_v4();
    let supporting = Uuid::new_v4();
    let characters = vec![
        CharacterBrief {
            id: supporting,
            role: "supporting".into(),
            first_appearance_chapter: Some(1),
        },
        CharacterBrief {
            id: second,
            role: "protagonist".into(),
            first_appearance_chapter: Some(5),
        },
        CharacterBrief {
            id: lead,
            role: "protagonist".into(),
            first_appearance_chapter: Some(1),
        },
    ];
    // Earliest first appearance wins: lead (chapter 1) beats second (chapter 5).
    assert_eq!(resolve_protagonist(&characters), Some(lead));
    assert_eq!(resolve_protagonist(&characters[..1]), None);
    assert_eq!(resolve_protagonist(&[]), None);
    let only_supporting = vec![CharacterBrief {
        id: supporting,
        role: "supporting".into(),
        first_appearance_chapter: None,
    }];
    assert_eq!(resolve_protagonist(&only_supporting), None);
}

#[test]
fn protagonist_tie_breaks_on_stable_id() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let (lower_id, higher_id) = (a.min(b), a.max(b));
    let tied = vec![
        CharacterBrief {
            id: higher_id,
            role: "protagonist".into(),
            first_appearance_chapter: Some(2),
        },
        CharacterBrief {
            id: lower_id,
            role: "protagonist".into(),
            first_appearance_chapter: Some(2),
        },
    ];
    // Same earliest chapter: the smaller id wins, regardless of list order.
    assert_eq!(resolve_protagonist(&tied), Some(lower_id));
    let reversed = vec![tied[1].clone(), tied[0].clone()];
    assert_eq!(resolve_protagonist(&reversed), Some(lower_id));
}

#[test]
fn protagonist_with_known_first_appearance_precedes_unknown() {
    let known = CharacterBrief {
        id: Uuid::new_v4(),
        role: "protagonist".into(),
        first_appearance_chapter: Some(7),
    };
    let unknown = CharacterBrief {
        id: Uuid::new_v4(),
        role: "protagonist".into(),
        first_appearance_chapter: None,
    };

    assert_eq!(
        resolve_protagonist(&[unknown.clone(), known.clone()]),
        Some(known.id)
    );
    assert_eq!(
        resolve_protagonist(&[known.clone(), unknown.clone()]),
        Some(known.id)
    );
    assert_eq!(
        resolve_protagonist(std::slice::from_ref(&unknown)),
        Some(unknown.id)
    );
}

#[test]
fn journey_memory_uses_a_stable_private_uuid_namespace() {
    let turn_id = Uuid::parse_str("00000000-0000-0000-0000-000000000042").unwrap();
    let memory_id = journey_memory_id(turn_id);
    assert_eq!(
        memory_id,
        Uuid::parse_str("edb805be-a358-580f-bc45-e2d98473ac11").unwrap()
    );
    assert_eq!(memory_id.get_version_num(), 5);
    assert_ne!(memory_id, turn_id);
}

#[test]
fn open_world_start_is_first_writer_wins_and_later_calls_resume() {
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let mut state = WorldState::new(user_id, novel_id);
    let player = PlayerEntity::new(
        user_id,
        novel_id,
        1,
        "云舟".into(),
        "远行者".into(),
        vec!["观察".into()],
        "gate".into(),
        vec![],
    )
    .unwrap();
    state.state["player_entity"] = serde_json::to_value(player).unwrap();
    let context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 1,
        unlocked_through_chapter: 1,
        characters: vec![],
        locations: vec![],
        factions: vec![],
        hard_rules: vec![],
        dead_character_ids: vec![],
        threads: vec![],
        scheduled_events: vec![],
        character_goals: vec![],
    };
    let started = state.start_open_world(&context).unwrap();
    let committed_state = state.state.clone();
    let mut fresh_candidate = context;
    fresh_candidate.model_version = 2;

    let resumed = state.start_open_world(&fresh_candidate).unwrap();

    assert_eq!(resumed, started);
    assert_eq!(resumed.entry_context.model_version, 1);
    assert_eq!(state.state, committed_state);
}

#[tokio::test]
async fn journey_memory_records_on_the_protagonist_with_source_high_water() {
    let agent = RecordingAgentMemory {
        calls: Mutex::new(vec![]),
        failures_remaining: AtomicUsize::new(0),
    };
    let protagonist = Uuid::new_v4();
    let repo = FixedChapterRepo {
        characters: vec![CharacterBrief {
            id: protagonist,
            role: "protagonist".into(),
            first_appearance_chapter: Some(1),
        }],
    };
    let (turn_id, user_id, novel_id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let result = journey_result(user_id, novel_id, Some(protagonist), 7);
    let projected = record_world_journey_memory(&agent, &repo, turn_id, user_id, novel_id, &result)
        .await
        .unwrap();
    assert!(projected);
    let calls = agent.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.memory_id, turn_id);
    assert_eq!(call.character_id, protagonist);
    assert_eq!(call.user_id, user_id);
    assert_eq!(call.novel_id, novel_id);
    assert_eq!(call.chapter_number, 7);
    assert_eq!(call.importance, 7);
    let fact: serde_json::Value = serde_json::from_str(&call.event).unwrap();
    assert_eq!(fact["reader_action"]["kind"], "converse");
    assert_eq!(fact["reader_action"]["target_id"], protagonist.to_string());
    assert!(fact["reader_action"].get("intent").is_none());
    assert!(!call.event.contains("北境"));
}

#[tokio::test]
async fn journey_memory_retries_boundedly_and_never_panics_on_failure() {
    let agent = RecordingAgentMemory {
        calls: Mutex::new(vec![]),
        failures_remaining: AtomicUsize::new(2),
    };
    let protagonist = Uuid::new_v4();
    let repo = FixedChapterRepo {
        characters: vec![CharacterBrief {
            id: protagonist,
            role: "protagonist".into(),
            first_appearance_chapter: None,
        }],
    };
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let result = journey_result(user_id, novel_id, Some(protagonist), 3);
    let projected =
        record_world_journey_memory(&agent, &repo, Uuid::new_v4(), user_id, novel_id, &result)
            .await
            .unwrap();
    assert!(projected);
    // 2 failures then success within the bounded retry budget (0..=2).
    assert_eq!(agent.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn journey_memory_skips_without_a_protagonist() {
    let agent = RecordingAgentMemory {
        calls: Mutex::new(vec![]),
        failures_remaining: AtomicUsize::new(0),
    };
    let repo = FixedChapterRepo {
        characters: vec![CharacterBrief {
            id: Uuid::new_v4(),
            role: "supporting".into(),
            first_appearance_chapter: Some(1),
        }],
    };
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let result = journey_result(user_id, novel_id, Some(Uuid::new_v4()), 3);
    let projected =
        record_world_journey_memory(&agent, &repo, Uuid::new_v4(), user_id, novel_id, &result)
            .await
            .unwrap();
    assert!(!projected);
    assert!(agent.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn journey_memory_skips_when_the_protagonist_did_not_witness_the_turn() {
    let agent = RecordingAgentMemory {
        calls: Mutex::new(vec![]),
        failures_remaining: AtomicUsize::new(0),
    };
    let protagonist = Uuid::new_v4();
    let repo = FixedChapterRepo {
        characters: vec![CharacterBrief {
            id: protagonist,
            role: "protagonist".into(),
            first_appearance_chapter: Some(1),
        }],
    };
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let result = journey_result(user_id, novel_id, Some(Uuid::new_v4()), 3);

    let projected =
        record_world_journey_memory(&agent, &repo, Uuid::new_v4(), user_id, novel_id, &result)
            .await
            .unwrap();

    assert!(!projected);
    assert!(agent.calls.lock().unwrap().is_empty());
}

#[test]
fn test_choice_index_bounds() {
    let node = NarrativeNode::new(
        Uuid::new_v4(),
        3,
        "A critical moment".into(),
        vec![
            NarrativeChoice {
                index: 0,
                text: "Fight".into(),
                hint: "Danger".into(),
                generated_consequence: None,
            },
            NarrativeChoice {
                index: 1,
                text: "Flee".into(),
                hint: "Safety".into(),
                generated_consequence: None,
            },
            NarrativeChoice {
                index: 2,
                text: "Talk".into(),
                hint: "Wisdom".into(),
                generated_consequence: None,
            },
        ],
    );

    assert!(!node.choices.is_empty());
    assert!(node.choices.get(2).is_some());
    assert!(node.choices.get(3).is_none());
    assert!(node.choices.get(99).is_none());
}

#[test]
fn test_world_state_record_choice() {
    let mut ws = WorldState::new(Uuid::new_v4(), Uuid::new_v4());
    let node_id = Uuid::new_v4();

    assert!(ws
        .record_choice(
            node_id,
            3,
            0,
            "Fight the dragon",
            "The hero took a deep breath...",
        )
        .unwrap());
    assert!(!ws
        .record_choice(
            node_id,
            3,
            0,
            "Fight the dragon",
            "The hero took a deep breath...",
        )
        .unwrap());

    let choices = ws.state["choices"].as_array().unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0]["node_id"], node_id.to_string());
    assert_eq!(choices[0]["chapter"].as_i64().unwrap(), 3);
    assert_eq!(choices[0]["choice"].as_str().unwrap(), "Fight the dragon");
}

#[test]
fn test_world_state_relationships() {
    let mut ws = WorldState::new(Uuid::new_v4(), Uuid::new_v4());

    assert_eq!(ws.get_relationship_score("Alice"), 50);

    ws.update_relationship("Alice", 20, "Saved her life")
        .unwrap();
    assert_eq!(ws.get_relationship_score("Alice"), 70);

    ws.update_relationship("Alice", -30, "Betrayal").unwrap();
    assert_eq!(ws.get_relationship_score("Alice"), 40);
}

#[test]
fn test_world_state_relationship_clamping() {
    let mut ws = WorldState::new(Uuid::new_v4(), Uuid::new_v4());

    ws.update_relationship("Bob", 100, "Best friends").unwrap();
    assert_eq!(ws.get_relationship_score("Bob"), 100);

    ws.update_relationship("Bob", 50, "Even more").unwrap();
    assert_eq!(ws.get_relationship_score("Bob"), 100); // clamped

    ws.update_relationship("Enemy", -200, "Mortal enemies")
        .unwrap();
    assert_eq!(ws.get_relationship_score("Enemy"), 0); // clamped
}

#[test]
fn test_multiple_choices_accumulate() {
    let mut ws = WorldState::new(Uuid::new_v4(), Uuid::new_v4());

    ws.record_choice(Uuid::new_v4(), 1, 0, "Choice A", "Result A")
        .unwrap();
    ws.record_choice(Uuid::new_v4(), 2, 0, "Choice B", "Result B")
        .unwrap();
    ws.record_choice(Uuid::new_v4(), 3, 0, "Choice C", "Result C")
        .unwrap();

    let choices = ws.state["choices"].as_array().unwrap();
    assert_eq!(choices.len(), 3);
}

#[test]
fn legacy_choice_is_enriched_in_place() {
    let mut ws = WorldState::new(Uuid::new_v4(), Uuid::new_v4());
    ws.state["choices"] = serde_json::json!([{
        "chapter": 2,
        "choice": "Wait",
        "consequence": ""
    }]);
    let node_id = Uuid::new_v4();

    assert!(ws
        .record_choice(node_id, 2, 1, "Wait", "Dawn arrives")
        .unwrap());
    let choices = ws.state["choices"].as_array().unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0]["node_id"], node_id.to_string());
    assert_eq!(choices[0]["consequence"], "Dawn arrives");
}

#[test]
fn malformed_player_entity_fails_without_mutating_world_state() {
    let mut state = WorldState::new(Uuid::new_v4(), Uuid::new_v4());
    state.state["player_entity"] = serde_json::Value::Null;
    let before = state.state.clone();

    assert!(state
        .record_choice(Uuid::new_v4(), 1, 0, "前进", "继续")
        .is_err());
    assert_eq!(state.state, before);
}

#[test]
fn structured_transition_mutates_world_state_exactly_once() {
    let mut state = WorldState::new(Uuid::new_v4(), Uuid::new_v4());
    let node_id = Uuid::new_v4();
    let character_id = Uuid::new_v4();
    let transition = NarrativeTransition {
        schema_version: TRANSITION_SCHEMA_VERSION,
        prompt_version: TRANSITION_PROMPT_VERSION.into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: 2,
        rendered_narrative: "你救下阿宁，城门后的线索仍待追查。".into(),
        events: vec![TransitionEvent {
            summary: "玩家救下阿宁".into(),
            actor_character_ids: vec![character_id],
            location_id: None,
        }],
        relationship_changes: vec![RelationshipChange {
            character_id,
            delta: 10,
            reason: "救命之恩".into(),
        }],
        location_changes: vec![],
        thread_changes: vec![ThreadChange {
            thread_id: "thread-1".into(),
            status: ThreadStatus::Open,
            description: "继续寻找线索".into(),
        }],
    };

    assert!(state
        .apply_choice_transition(node_id, 2, 0, "救下阿宁", &transition)
        .unwrap());
    assert!(!state
        .apply_choice_transition(node_id, 2, 0, "救下阿宁", &transition)
        .unwrap());
    assert_eq!(state.state["choices"].as_array().unwrap().len(), 1);
    assert_eq!(state.state["world_events"].as_array().unwrap().len(), 1);
    assert_eq!(
        state.state["relationships"][character_id.to_string()]["score"],
        60
    );
    assert_eq!(state.state["threads"]["thread-1"]["status"], "open");
}
