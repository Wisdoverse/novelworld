use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    narrative_node::{WorldState, WorldStateError},
    world_session::{
        CanonicalEventState, CanonicalEventStatus, CharacterGoalRef, ScheduledCanonEvent,
        WorldCharacterRef, WorldEntityRef, WorldRuleRef, WorldSessionError,
    },
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDefinition<T> {
    pub definition: T,
    pub source_chapters: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSourceDelta {
    pub model_version: i32,
    pub checkpoint_chapter: i32,
    pub from_source_chapter: i32,
    pub target_chapter: i32,
    pub characters: Vec<SourceDefinition<WorldCharacterRef>>,
    pub locations: Vec<SourceDefinition<WorldEntityRef>>,
    pub factions: Vec<SourceDefinition<WorldEntityRef>>,
    pub hard_rules: Vec<SourceDefinition<WorldRuleRef>>,
    pub threads: Vec<SourceDefinition<WorldEntityRef>>,
    pub scheduled_events: Vec<SourceDefinition<ScheduledCanonEvent>>,
    pub character_goals: Vec<SourceDefinition<CharacterGoalRef>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSourceCommand {
    pub expected_turn_number: i64,
    pub expected_source_chapter: i32,
    pub target_chapter: i32,
}

impl WorldSourceCommand {
    pub fn validate(&self) -> Result<(), WorldSessionError> {
        if self.expected_turn_number < 0
            || self.expected_source_chapter < 1
            || self.expected_source_chapter.checked_add(1) != Some(self.target_chapter)
        {
            return Err(WorldSessionError(
                "source admission must advance exactly one chapter".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSourceOperation {
    pub operation_id: Uuid,
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub command: WorldSourceCommand,
    pub source_context: super::world_session::WorldEntryContext,
}

#[derive(Debug, thiserror::Error)]
pub enum WorldSourceError {
    #[error("world source changed; reload the current world")]
    Changed,
    #[error("a world turn or its memory projection is pending")]
    Busy,
    #[error("new source events would rewrite already advanced chronology")]
    OrderConflict,
    #[error("source operation key has a different scope or command")]
    KeyConflict,
}

fn bounded_sources(chapters: &[i32], upper: i32) -> bool {
    !chapters.is_empty()
        && chapters.len() <= 256
        && chapters.iter().all(|chapter| (1..=upper).contains(chapter))
        && chapters.windows(2).all(|pair| pair[0] < pair[1])
}

fn append<T: Clone>(
    existing: &mut Vec<T>,
    candidates: &[SourceDefinition<T>],
    upper: i32,
    identity: impl Fn(&T) -> String,
) -> Result<(), WorldStateError> {
    if candidates.len() > 256 {
        return Err(WorldStateError::InvalidWorldSession(
            "oversized source delta".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for candidate in candidates {
        let id = identity(&candidate.definition);
        if !seen.insert(id.clone()) {
            return Err(WorldStateError::InvalidWorldSession(
                "duplicate source identity".into(),
            ));
        }
        // Already admitted definitions retain exact authority even if a newer
        // source read returns changed text or unadmitted future citations.
        if existing.iter().any(|old| identity(old) == id) {
            continue;
        }
        if !bounded_sources(&candidate.source_chapters, upper) {
            return Err(WorldStateError::InvalidWorldSession(
                "invalid complete source authority".into(),
            ));
        }
        existing.push(candidate.definition.clone());
    }
    Ok(())
}

impl WorldState {
    pub fn extend_world_source(
        &mut self,
        command: &WorldSourceCommand,
        delta: &WorldSourceDelta,
    ) -> Result<(), anyhow::Error> {
        command.validate()?;
        let mut session = self.open_world()?.ok_or(WorldSourceError::Changed)?;
        let old = session.context();
        if session.turn_number != command.expected_turn_number
            || old.unlocked_through_chapter != command.expected_source_chapter
        {
            return Err(WorldSourceError::Changed.into());
        }
        if delta.model_version != old.model_version
            || delta.checkpoint_chapter != old.checkpoint_chapter
            || delta.from_source_chapter != command.expected_source_chapter
            || delta.target_chapter != command.target_chapter
        {
            return Err(
                WorldStateError::InvalidWorldSession("source delta scope differs".into()).into(),
            );
        }
        self.validate_world_entry_checkpoint(session.entry_context.checkpoint_chapter)?;
        let mut context = old.clone();
        context.unlocked_through_chapter = command.target_chapter;
        append(
            &mut context.characters,
            &delta.characters,
            command.target_chapter,
            |v| v.id.to_string(),
        )?;
        append(
            &mut context.locations,
            &delta.locations,
            command.target_chapter,
            |v| v.id.clone(),
        )?;
        append(
            &mut context.factions,
            &delta.factions,
            command.target_chapter,
            |v| v.id.clone(),
        )?;
        append(
            &mut context.hard_rules,
            &delta.hard_rules,
            command.target_chapter,
            |v| v.id.clone(),
        )?;
        append(
            &mut context.threads,
            &delta.threads,
            command.target_chapter,
            |v| v.id.clone(),
        )?;
        append(
            &mut context.character_goals,
            &delta.character_goals,
            command.target_chapter,
            |v| v.id.clone(),
        )?;
        let last_advanced_sequence = session
            .canonical_events
            .iter()
            .filter(|event| event.advanced_at_world_time.is_some())
            .map(|event| event.event.sequence)
            .max()
            .unwrap_or(0);
        for candidate in &delta.scheduled_events {
            if !context
                .scheduled_events
                .iter()
                .any(|old| old.id == candidate.definition.id)
                && !candidate
                    .definition
                    .source_chapters
                    .iter()
                    .all(|chapter| candidate.source_chapters.contains(chapter))
            {
                return Err(WorldStateError::InvalidWorldSession(
                    "event authority omits own provenance".into(),
                )
                .into());
            }
        }
        for candidate in &delta.character_goals {
            if !old
                .character_goals
                .iter()
                .any(|known| known.id == candidate.definition.id)
                && !candidate
                    .definition
                    .source_chapters
                    .iter()
                    .all(|chapter| candidate.source_chapters.contains(chapter))
            {
                return Err(WorldStateError::InvalidWorldSession(
                    "goal authority omits own provenance".into(),
                )
                .into());
            }
        }
        let old_ids = context
            .scheduled_events
            .iter()
            .map(|event| event.id.clone())
            .collect::<std::collections::HashSet<_>>();
        append(
            &mut context.scheduled_events,
            &delta.scheduled_events,
            command.target_chapter,
            |v| v.id.clone(),
        )?;
        for event in &context.scheduled_events {
            if !old_ids.contains(&event.id) {
                if event.sequence <= last_advanced_sequence {
                    return Err(WorldSourceError::OrderConflict.into());
                }
                session.canonical_events.push(CanonicalEventState {
                    event: event.clone(),
                    status: CanonicalEventStatus::Scheduled,
                    reason: None,
                    advanced_at_world_time: None,
                });
            }
        }
        context.scheduled_events.sort_by_key(|event| event.sequence);
        session
            .canonical_events
            .sort_by_key(|event| event.event.sequence);
        context.validate_source()?;
        session.source_context = Some(context);
        session.schema_version = 2;
        session.validate()?;
        let mut next = self.state.clone();
        let root = next
            .as_object_mut()
            .ok_or(WorldStateError::InvalidObject("root"))?;
        let threads = root
            .entry("threads")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or(WorldStateError::InvalidObject("threads"))?;
        for thread in &session.context().threads {
            threads.entry(thread.id.clone()).or_insert_with(|| {
                serde_json::json!({
                    "status":"open", "description": thread.name, "origin":"canon",
                })
            });
        }
        root.insert("open_world".into(), serde_json::to_value(session)?);
        self.state = next;
        self.updated_at = chrono::Utc::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{
        player_entity::PlayerEntity,
        world_session::{WorldEntryContext, WorldSession},
    };

    fn world() -> WorldState {
        let mut state = WorldState::new(Uuid::new_v4(), Uuid::new_v4());
        let player = PlayerEntity::new(
            state.user_id,
            state.novel_id,
            1,
            "旅人".into(),
            "来自河岸".into(),
            vec!["辨认路径".into()],
            None,
            vec![],
        )
        .unwrap();
        state.state["player_entity"] = serde_json::to_value(player).unwrap();
        let context = WorldEntryContext {
            series_setting: None,
            model_version: 1,
            checkpoint_chapter: 1,
            unlocked_through_chapter: 1,
            characters: vec![],
            locations: vec![],
            factions: vec![],
            hard_rules: vec![],
            dead_character_ids: vec![],
            threads: vec![WorldEntityRef {
                id: "old-thread".into(),
                name: "旧事件".into(),
            }],
            scheduled_events: vec![],
            character_goals: vec![],
        };
        state.start_open_world(&context).unwrap();
        state.state["threads"]["old-thread"]["status"] = "resolved".into();
        state
    }

    fn delta(from: i32) -> WorldSourceDelta {
        WorldSourceDelta {
            model_version: 1,
            checkpoint_chapter: 1,
            from_source_chapter: from,
            target_chapter: from + 1,
            characters: vec![],
            locations: vec![],
            factions: vec![],
            hard_rules: vec![],
            threads: vec![SourceDefinition {
                definition: WorldEntityRef {
                    id: format!("thread-{}", from + 1),
                    name: "后续事件".into(),
                },
                source_chapters: vec![from + 1],
            }],
            scheduled_events: vec![SourceDefinition {
                definition: ScheduledCanonEvent {
                    id: format!("event-{}", from + 1),
                    sequence: from,
                    summary: "后续场景发生变化".into(),
                    character_ids: vec![],
                    location_ids: vec![],
                    faction_ids: vec![],
                    death_character_ids: vec![],
                    source_chapters: vec![from + 1],
                },
                source_chapters: vec![from + 1],
            }],
            character_goals: vec![],
        }
    }

    #[test]
    fn source_admission_preserves_origin_overlay_and_legacy_bytes() {
        let mut state = world();
        let before = state.open_world().unwrap().unwrap();
        let legacy_json = serde_json::to_value(&before).unwrap();
        assert!(legacy_json.get("source_context").is_none());
        let parsed: WorldSession = serde_json::from_value(legacy_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), legacy_json);
        let fingerprint = state.fingerprint();
        let original_player = state.player_entity().unwrap();
        state
            .extend_world_source(
                &WorldSourceCommand {
                    expected_turn_number: 0,
                    expected_source_chapter: 1,
                    target_chapter: 2,
                },
                &delta(1),
            )
            .unwrap();
        let first = state.open_world().unwrap().unwrap();
        assert_eq!(first.entry_context, before.entry_context);
        assert_eq!(first.turn_number, 0);
        assert_eq!(first.world_time, 0);
        assert_eq!(first.context().unlocked_through_chapter, 2);
        assert_eq!(first.canonical_events.len(), 1);
        assert_eq!(state.state["threads"]["old-thread"]["status"], "resolved");
        assert_eq!(state.player_entity().unwrap(), original_player);
        assert_ne!(state.fingerprint(), fingerprint);
        state.start_open_world(&before.entry_context).unwrap();
        assert_eq!(state.open_world().unwrap().unwrap(), first);
        state
            .extend_world_source(
                &WorldSourceCommand {
                    expected_turn_number: 0,
                    expected_source_chapter: 2,
                    target_chapter: 3,
                },
                &delta(2),
            )
            .unwrap();
        let second = state.open_world().unwrap().unwrap();
        assert_eq!(second.entry_context, before.entry_context);
        assert_eq!(second.canonical_events[0], first.canonical_events[0]);
        assert_eq!(state.source_chapter_high_water().unwrap(), Some(3));
    }

    #[test]
    fn mixed_future_authority_wrong_scope_and_stale_source_are_atomic_rejections() {
        let state = world();
        let command = WorldSourceCommand {
            expected_turn_number: 0,
            expected_source_chapter: 1,
            target_chapter: 2,
        };
        for invalid in [0, 1, 2] {
            let mut candidate = state.clone();
            let mut source = delta(1);
            match invalid {
                0 => source.threads[0].source_chapters = vec![2, 10],
                1 => source.model_version = 2,
                _ => source.target_chapter = 3,
            }
            assert!(candidate.extend_world_source(&command, &source).is_err());
            assert_eq!(candidate, state);
        }
        let mut candidate = state.clone();
        let stale = WorldSourceCommand {
            expected_turn_number: 1,
            ..command
        };
        assert!(candidate.extend_world_source(&stale, &delta(1)).is_err());
        assert_eq!(candidate, state);
    }

    #[test]
    fn new_goals_use_source_upper_without_relaxing_origin_and_old_ids_never_overwrite() {
        let mut state = world();
        let mut source = delta(1);
        let actor = Uuid::new_v4();
        source.characters.push(SourceDefinition {
            definition: WorldCharacterRef {
                id: actor,
                name: "新旅伴".into(),
            },
            source_chapters: vec![2],
        });
        source.character_goals.push(SourceDefinition {
            definition: CharacterGoalRef {
                id: "new-goal".into(),
                character_id: actor,
                description: "寻找渡口".into(),
                source_chapters: vec![2],
            },
            source_chapters: vec![2],
        });
        source.threads.push(SourceDefinition {
            definition: WorldEntityRef {
                id: "old-thread".into(),
                name: "不得覆盖".into(),
            },
            source_chapters: vec![1, 10],
        });
        state
            .extend_world_source(
                &WorldSourceCommand {
                    expected_turn_number: 0,
                    expected_source_chapter: 1,
                    target_chapter: 2,
                },
                &source,
            )
            .unwrap();
        let session = state.open_world().unwrap().unwrap();
        assert_eq!(session.context().threads[0].name, "旧事件");
        assert!(session.context().validate().is_err());
        session.context().validate_source().unwrap();
        assert_eq!(
            session.active_character_agents(&super::super::world_session::WorldAction {
                kind: super::super::world_session::WorldActionKind::PursueGoal,
                target_id: Some("new-goal".into()),
                intent: "帮助旅伴".into(),
            }),
            vec![actor]
        );
        assert_eq!(
            state
                .character_world_context(actor)
                .unwrap()
                .unwrap()
                .source_chapter_high_water,
            2
        );
    }
}
