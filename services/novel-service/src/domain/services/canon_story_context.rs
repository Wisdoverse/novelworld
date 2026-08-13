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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CanonContextError {
    #[error("checkpoint chapter must be positive")]
    InvalidCheckpoint,
    #[error("canon context exceeds {MAX_CONTEXT_ITEMS} items in {0}")]
    TooLarge(&'static str),
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

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{original_player_name_available, visible_at};
    use crate::domain::entities::canon_story_model::{SourceCitation, SourceEvidence};
    use crate::domain::{entities::character::Character, value_objects::CharacterRole};
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
}
