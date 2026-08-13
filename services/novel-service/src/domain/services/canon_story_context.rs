use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entities::{
    canon_story_model::{CanonStoryModel, SourceEvidence},
    character::Character,
};

const MAX_CONTEXT_ITEMS: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonContext {
    pub model_version: i32,
    pub checkpoint_chapter: i32,
    pub characters: Vec<CanonCharacterRef>,
    pub locations: Vec<CanonEntityRef>,
    pub hard_rules: Vec<CanonRuleRef>,
    pub dead_character_ids: Vec<Uuid>,
    pub threads: Vec<CanonEntityRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonCharacterRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonEntityRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonRuleRef {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonEventRef {
    pub id: String,
    pub sequence: i32,
    pub summary: String,
    pub character_ids: Vec<Uuid>,
    pub location_ids: Vec<String>,
    pub faction_ids: Vec<String>,
    pub death_character_ids: Vec<Uuid>,
    pub source_chapters: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonCharacterGoalRef {
    pub id: String,
    pub character_id: Uuid,
    pub description: String,
    pub source_chapters: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldEntryContext {
    pub model_version: i32,
    pub checkpoint_chapter: i32,
    pub unlocked_through_chapter: i32,
    pub characters: Vec<CanonCharacterRef>,
    pub locations: Vec<CanonEntityRef>,
    pub factions: Vec<CanonEntityRef>,
    pub hard_rules: Vec<CanonRuleRef>,
    pub dead_character_ids: Vec<Uuid>,
    pub threads: Vec<CanonEntityRef>,
    pub scheduled_events: Vec<CanonEventRef>,
    pub character_goals: Vec<CanonCharacterGoalRef>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CanonContextError {
    #[error("checkpoint chapter must be positive")]
    InvalidCheckpoint,
    #[error("canon context exceeds {MAX_CONTEXT_ITEMS} items in {0}")]
    TooLarge(&'static str),
    #[error("world checkpoint must be within the unlocked chapter range")]
    InvalidWorldRange,
}

pub fn build_canon_context(
    model: &CanonStoryModel,
    characters: &[Character],
    checkpoint_chapter: i32,
) -> Result<CanonContext, CanonContextError> {
    if checkpoint_chapter < 1 {
        return Err(CanonContextError::InvalidCheckpoint);
    }
    let characters = characters
        .iter()
        .filter(|character| {
            character
                .first_appearance_chapter
                .is_some_and(|chapter| chapter <= checkpoint_chapter)
        })
        .map(|character| CanonCharacterRef {
            id: character.id,
            name: character.name.clone(),
        })
        .collect::<Vec<_>>();
    let visible_character_ids = characters
        .iter()
        .map(|character| character.id)
        .collect::<HashSet<_>>();
    let locations = model
        .content
        .locations
        .iter()
        .filter(|location| visible_at(&location.evidence, checkpoint_chapter))
        .map(|location| CanonEntityRef {
            id: location.id.clone(),
            name: location.name.clone(),
        })
        .collect::<Vec<_>>();
    let hard_rules = model
        .content
        .world_rules
        .iter()
        .filter(|rule| rule.hard && visible_at(&rule.evidence, checkpoint_chapter))
        .map(|rule| CanonRuleRef {
            id: rule.id.clone(),
            description: rule.description.clone(),
        })
        .collect::<Vec<_>>();
    let mut dead = HashSet::new();
    let dead_character_ids = model
        .content
        .deaths
        .iter()
        .filter(|death| {
            visible_character_ids.contains(&death.character_id)
                && visible_at(&death.evidence, checkpoint_chapter)
                && dead.insert(death.character_id)
        })
        .map(|death| death.character_id)
        .collect::<Vec<_>>();
    let threads = model
        .content
        .unresolved_threads
        .iter()
        .filter(|thread| visible_at(&thread.evidence, checkpoint_chapter))
        .map(|thread| CanonEntityRef {
            id: thread.id.clone(),
            name: thread.description.clone(),
        })
        .collect::<Vec<_>>();

    for (name, count) in [
        ("characters", characters.len()),
        ("locations", locations.len()),
        ("hard_rules", hard_rules.len()),
        ("dead_character_ids", dead_character_ids.len()),
        ("threads", threads.len()),
    ] {
        if count > MAX_CONTEXT_ITEMS {
            return Err(CanonContextError::TooLarge(name));
        }
    }

    Ok(CanonContext {
        model_version: model.model_version,
        checkpoint_chapter,
        characters,
        locations,
        hard_rules,
        dead_character_ids,
        threads,
    })
}

pub fn build_world_entry_context(
    model: &CanonStoryModel,
    characters: &[Character],
    checkpoint_chapter: i32,
    unlocked_through_chapter: i32,
) -> Result<WorldEntryContext, CanonContextError> {
    if checkpoint_chapter < 1
        || unlocked_through_chapter < 1
        || checkpoint_chapter > unlocked_through_chapter
    {
        return Err(CanonContextError::InvalidWorldRange);
    }
    let checkpoint = build_canon_context(model, characters, checkpoint_chapter)?;
    let unlocked = build_canon_context(model, characters, unlocked_through_chapter)?;

    let factions = model
        .content
        .factions
        .iter()
        .filter(|faction| visible_at(&faction.evidence, unlocked_through_chapter))
        .map(|faction| CanonEntityRef {
            id: faction.id.clone(),
            name: faction.name.clone(),
        })
        .collect::<Vec<_>>();
    let mut scheduled_events = model
        .content
        .events
        .iter()
        .filter_map(|event| {
            let source_chapters = source_chapters(&event.evidence);
            let first = source_chapters.first().copied()?;
            (first > checkpoint_chapter && first <= unlocked_through_chapter).then(|| {
                let death_character_ids = model
                    .content
                    .deaths
                    .iter()
                    .filter(|death| {
                        death.event_id == event.id
                            && visible_at(&death.evidence, unlocked_through_chapter)
                    })
                    .map(|death| death.character_id)
                    .collect();
                CanonEventRef {
                    id: event.id.clone(),
                    sequence: event.sequence,
                    summary: event.summary.clone(),
                    character_ids: event.character_ids.clone(),
                    location_ids: event.location_ids.clone(),
                    faction_ids: event.faction_ids.clone(),
                    death_character_ids,
                    source_chapters,
                }
            })
        })
        .collect::<Vec<_>>();
    scheduled_events.sort_by_key(|event| event.sequence);
    let character_goals = model
        .content
        .character_goals
        .iter()
        .filter(|goal| visible_at(&goal.evidence, checkpoint_chapter))
        .map(|goal| CanonCharacterGoalRef {
            id: goal.id.clone(),
            character_id: goal.character_id,
            description: goal.description.clone(),
            source_chapters: source_chapters(&goal.evidence),
        })
        .collect::<Vec<_>>();

    for (name, count) in [
        ("factions", factions.len()),
        ("scheduled_events", scheduled_events.len()),
        ("character_goals", character_goals.len()),
    ] {
        if count > MAX_CONTEXT_ITEMS {
            return Err(CanonContextError::TooLarge(name));
        }
    }

    Ok(WorldEntryContext {
        model_version: model.model_version,
        checkpoint_chapter,
        unlocked_through_chapter,
        characters: unlocked.characters,
        locations: unlocked.locations,
        factions,
        hard_rules: unlocked.hard_rules,
        dead_character_ids: checkpoint.dead_character_ids,
        threads: checkpoint.threads,
        scheduled_events,
        character_goals,
    })
}

pub fn original_player_name_available(name: &str, characters: &[Character]) -> bool {
    let proposed = normalize(name);
    !characters.iter().any(|character| {
        normalize(&character.name) == proposed
            || character
                .aliases
                .iter()
                .any(|alias| normalize(alias) == proposed)
    })
}

fn visible_at(evidence: &SourceEvidence, checkpoint_chapter: i32) -> bool {
    evidence
        .provenance
        .iter()
        .any(|citation| citation.chapter_number <= checkpoint_chapter)
}

fn source_chapters(evidence: &SourceEvidence) -> Vec<i32> {
    let mut chapters = evidence
        .provenance
        .iter()
        .map(|citation| citation.chapter_number)
        .collect::<Vec<_>>();
    chapters.sort_unstable();
    chapters.dedup();
    chapters
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{build_world_entry_context, original_player_name_available, visible_at};
    use crate::domain::entities::canon_story_model::{
        CanonDeath, CanonEndingSnapshot, CanonEvent, CanonFaction, CanonLocation,
        CanonStoryContent, CanonStoryModel, CharacterGoal, SourceCitation, SourceEvidence,
        StoryArc, UnresolvedThread, WorldRule, CANON_STORY_SCHEMA_VERSION,
    };
    use crate::domain::{entities::character::Character, value_objects::CharacterRole};
    use chrono::Utc;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    #[test]
    fn evidence_is_not_visible_before_its_first_source_chapter() {
        let evidence = SourceEvidence {
            provenance: vec![SourceCitation {
                chapter_number: 3,
                excerpt: "第三章证据".into(),
            }],
            confidence: 1.0,
        };

        assert!(!visible_at(&evidence, 2));
        assert!(visible_at(&evidence, 3));
    }

    #[test]
    fn original_player_name_checks_all_canonical_names_and_aliases() {
        let mut character =
            Character::new(Uuid::new_v4(), "林岚".into(), CharacterRole::Protagonist);
        character.aliases = vec!["守门人".into()];

        assert!(!original_player_name_available(
            " 林岚 ",
            &[character.clone()]
        ));
        assert!(!original_player_name_available(
            "守门人",
            &[character.clone()]
        ));
        assert!(original_player_name_available("云舟", &[character]));
    }

    #[test]
    fn world_entry_schedules_only_unlocked_events_after_the_checkpoint() {
        let character_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let evidence = |chapter_number| SourceEvidence {
            provenance: vec![SourceCitation {
                chapter_number,
                excerpt: format!("第{chapter_number}章证据"),
            }],
            confidence: 1.0,
        };
        let model = CanonStoryModel {
            id: Uuid::new_v4(),
            novel_id,
            model_version: 3,
            schema_version: CANON_STORY_SCHEMA_VERSION,
            prompt_version: "canon-extraction-v1".into(),
            content: CanonStoryContent {
                arcs: vec![StoryArc {
                    id: "arc".into(),
                    title: "主线".into(),
                    summary: "主线".into(),
                    event_ids: vec!["past".into(), "next".into(), "locked".into()],
                    evidence: evidence(1),
                }],
                events: vec![
                    CanonEvent {
                        id: "past".into(),
                        sequence: 1,
                        summary: "已经发生".into(),
                        caused_by: vec![],
                        location_ids: vec!["gate".into()],
                        character_ids: vec![character_id],
                        faction_ids: vec!["guard".into()],
                        evidence: evidence(1),
                    },
                    CanonEvent {
                        id: "next".into(),
                        sequence: 2,
                        summary: "即将发生".into(),
                        caused_by: vec!["past".into()],
                        location_ids: vec!["gate".into()],
                        character_ids: vec![character_id],
                        faction_ids: vec!["guard".into()],
                        evidence: evidence(2),
                    },
                    CanonEvent {
                        id: "locked".into(),
                        sequence: 3,
                        summary: "尚未解锁".into(),
                        caused_by: vec!["next".into()],
                        location_ids: vec!["gate".into()],
                        character_ids: vec![character_id],
                        faction_ids: vec!["guard".into()],
                        evidence: evidence(3),
                    },
                ],
                locations: vec![CanonLocation {
                    id: "gate".into(),
                    name: "城门".into(),
                    description: "北门".into(),
                    evidence: evidence(1),
                }],
                factions: vec![CanonFaction {
                    id: "guard".into(),
                    name: "守军".into(),
                    description: "守卫城门".into(),
                    evidence: evidence(1),
                }],
                world_rules: vec![WorldRule {
                    id: "rule".into(),
                    description: "死者不会自然复生".into(),
                    hard: true,
                    evidence: evidence(1),
                }],
                character_goals: vec![CharacterGoal {
                    id: "goal".into(),
                    character_id,
                    description: "守住城门".into(),
                    evidence: evidence(1),
                }],
                relationships: vec![],
                deaths: vec![CanonDeath {
                    id: "death".into(),
                    character_id,
                    event_id: "next".into(),
                    description: "守门人战死".into(),
                    evidence: evidence(2),
                }],
                unresolved_threads: vec![UnresolvedThread {
                    id: "thread".into(),
                    description: "找出内应".into(),
                    evidence: evidence(1),
                }],
                ending: CanonEndingSnapshot {
                    summary: "故事结束".into(),
                    character_states: BTreeMap::from([(character_id, "存活".into())]),
                    faction_states: BTreeMap::from([("guard".into(), "仍在守城".into())]),
                    location_states: BTreeMap::from([("gate".into(), "完好".into())]),
                    unresolved_thread_ids: vec!["thread".into()],
                    evidence: evidence(3),
                },
            },
            created_at: Utc::now(),
        };
        let mut character = Character::new(novel_id, "守门人".into(), CharacterRole::Supporting);
        character.id = character_id;
        character.first_appearance_chapter = Some(1);

        let context = build_world_entry_context(&model, &[character], 1, 2).unwrap();

        assert_eq!(context.checkpoint_chapter, 1);
        assert_eq!(context.unlocked_through_chapter, 2);
        assert_eq!(context.scheduled_events.len(), 1);
        assert_eq!(context.scheduled_events[0].id, "next");
        assert_eq!(
            context.scheduled_events[0].death_character_ids,
            vec![character_id]
        );
        assert_eq!(context.scheduled_events[0].source_chapters, vec![2]);
        assert_eq!(context.character_goals[0].id, "goal");
        assert_eq!(context.factions[0].id, "guard");
        assert!(build_world_entry_context(&model, &[], 3, 2).is_err());
    }
}
