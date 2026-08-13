use crate::domain::entities::narrative_node::{NarrativeChoice, NarrativeNode, WorldState};
use crate::domain::services::narrative_transition::{
    NarrativeTransition, RelationshipChange, ThreadChange, ThreadStatus, TransitionEvent,
    TRANSITION_PROMPT_VERSION, TRANSITION_SCHEMA_VERSION,
};
use uuid::Uuid;

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

    ws.update_relationship("Alice", 20, "Saved her life");
    assert_eq!(ws.get_relationship_score("Alice"), 70);

    ws.update_relationship("Alice", -30, "Betrayal");
    assert_eq!(ws.get_relationship_score("Alice"), 40);
}

#[test]
fn test_world_state_relationship_clamping() {
    let mut ws = WorldState::new(Uuid::new_v4(), Uuid::new_v4());

    ws.update_relationship("Bob", 100, "Best friends");
    assert_eq!(ws.get_relationship_score("Bob"), 100);

    ws.update_relationship("Bob", 50, "Even more");
    assert_eq!(ws.get_relationship_score("Bob"), 100); // clamped

    ws.update_relationship("Enemy", -200, "Mortal enemies");
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
