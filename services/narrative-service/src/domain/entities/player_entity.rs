use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entities::game_rules::PlayerRuleProfile;

const MAX_CAPABILITIES: usize = 16;
const MAX_INVENTORY: usize = 32;
const MAX_RELATIONSHIPS: usize = 256;
const MAX_FACTIONS: usize = 128;
const MAX_KNOWLEDGE: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipState {
    pub score: i32,
    pub last_change: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerEntity {
    pub id: Uuid,
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub canonical_checkpoint_chapter: i32,
    pub name: String,
    pub background: String,
    pub capabilities: Vec<String>,
    pub location_id: String,
    pub inventory: Vec<String>,
    pub relationships: BTreeMap<Uuid, RelationshipState>,
    pub faction_standing: BTreeMap<String, i32>,
    pub discovered_knowledge: Vec<String>,
    #[serde(default, skip_serializing_if = "PlayerRuleProfile::is_narrative")]
    pub rules: PlayerRuleProfile,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid player entity: {0}")]
pub struct PlayerEntityError(String);

impl PlayerEntity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        user_id: Uuid,
        novel_id: Uuid,
        canonical_checkpoint_chapter: i32,
        name: String,
        background: String,
        capabilities: Vec<String>,
        location_id: String,
        inventory: Vec<String>,
    ) -> Result<Self, PlayerEntityError> {
        Self::new_with_rules(
            user_id,
            novel_id,
            canonical_checkpoint_chapter,
            name,
            background,
            capabilities,
            location_id,
            inventory,
            PlayerRuleProfile::narrative(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_rules(
        user_id: Uuid,
        novel_id: Uuid,
        canonical_checkpoint_chapter: i32,
        name: String,
        background: String,
        capabilities: Vec<String>,
        location_id: String,
        inventory: Vec<String>,
        rules: PlayerRuleProfile,
    ) -> Result<Self, PlayerEntityError> {
        let entity = Self {
            id: Uuid::new_v4(),
            user_id,
            novel_id,
            canonical_checkpoint_chapter,
            name,
            background,
            capabilities,
            location_id,
            inventory,
            relationships: BTreeMap::new(),
            faction_standing: BTreeMap::new(),
            discovered_knowledge: Vec::new(),
            rules,
            created_at: Utc::now(),
        };
        entity.validate()?;
        Ok(entity)
    }

    pub fn validate(&self) -> Result<(), PlayerEntityError> {
        if self.id.is_nil() || self.user_id.is_nil() || self.novel_id.is_nil() {
            return invalid("IDs must not be nil");
        }
        if self.canonical_checkpoint_chapter < 1 {
            return invalid("canonical checkpoint chapter must be positive");
        }
        Self::validate_definition(
            &self.name,
            &self.background,
            &self.capabilities,
            &self.location_id,
            &self.inventory,
        )?;
        if self.relationships.len() > MAX_RELATIONSHIPS
            || self.relationships.iter().any(|(id, state)| {
                id.is_nil()
                    || !(0..=100).contains(&state.score)
                    || text("relationship last_change", &state.last_change, 1_000).is_err()
            })
        {
            return invalid("relationships are invalid or exceed their limit");
        }
        if self.faction_standing.len() > MAX_FACTIONS
            || self.faction_standing.iter().any(|(id, score)| {
                token("faction ID", id, 100).is_err() || !(-100..=100).contains(score)
            })
        {
            return invalid("faction standing is invalid or exceeds its limit");
        }
        list(
            "discovered_knowledge",
            &self.discovered_knowledge,
            0,
            MAX_KNOWLEDGE,
            200,
        )?;
        self.rules
            .validate()
            .map_err(|error| PlayerEntityError(error.to_string()))?;
        Ok(())
    }

    pub fn validate_definition(
        name: &str,
        background: &str,
        capabilities: &[String],
        location_id: &str,
        inventory: &[String],
    ) -> Result<(), PlayerEntityError> {
        token("name", name, 100)?;
        text("background", background, 2_000)?;
        list("capabilities", capabilities, 1, MAX_CAPABILITIES, 200)?;
        token("location_id", location_id, 100)?;
        list("inventory", inventory, 0, MAX_INVENTORY, 200)
    }

    pub fn matches_definition(
        &self,
        name: &str,
        background: &str,
        capabilities: &[String],
        location_id: &str,
        inventory: &[String],
    ) -> bool {
        self.name == name
            && self.background == background
            && self.capabilities == capabilities
            && self.location_id == location_id
            && self.inventory == inventory
    }

    pub fn matches_rules(&self, rules: &PlayerRuleProfile) -> bool {
        &self.rules == rules
    }
}

fn list(
    name: &str,
    values: &[String],
    minimum: usize,
    maximum: usize,
    max_chars: usize,
) -> Result<(), PlayerEntityError> {
    let mut unique = HashSet::new();
    if !(minimum..=maximum).contains(&values.len())
        || values
            .iter()
            .any(|value| token(name, value, max_chars).is_err() || !unique.insert(normalize(value)))
    {
        return invalid(format!(
            "{name} must contain {minimum}-{maximum} unique tokens"
        ));
    }
    Ok(())
}

fn text(name: &str, value: &str, max_chars: usize) -> Result<(), PlayerEntityError> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > max_chars
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return invalid(format!(
            "{name} must contain 1-{max_chars} trimmed printable characters"
        ));
    }
    Ok(())
}

fn token(name: &str, value: &str, max_chars: usize) -> Result<(), PlayerEntityError> {
    text(name, value, max_chars)?;
    if value.chars().any(char::is_control) {
        return invalid(format!("{name} must be a single-line token"));
    }
    Ok(())
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn invalid<T>(message: impl Into<String>) -> Result<T, PlayerEntityError> {
    Err(PlayerEntityError(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> PlayerEntity {
        PlayerEntity::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            2,
            "云舟".into(),
            "来自边城的地图学徒。".into(),
            vec!["辨认古地图".into()],
            "north-tower".into(),
            vec!["旧地图".into()],
        )
        .unwrap()
    }

    #[test]
    fn validates_strict_bounded_original_player_state() {
        let entity = valid();
        entity.validate().unwrap();

        let mut duplicate = entity.clone();
        duplicate.capabilities = vec!["识图".into(), "识图".into()];
        assert!(duplicate.validate().is_err());

        let mut bad_relationship = entity.clone();
        bad_relationship.relationships.insert(
            Uuid::new_v4(),
            RelationshipState {
                score: 101,
                last_change: "越界".into(),
            },
        );
        assert!(bad_relationship.validate().is_err());

        let mut unknown = serde_json::to_value(entity).unwrap();
        unknown["source_character_id"] = Uuid::new_v4().to_string().into();
        assert!(serde_json::from_value::<PlayerEntity>(unknown).is_err());
    }

    #[test]
    fn narrative_players_keep_the_previous_serialized_shape() {
        let entity = valid();
        let value = serde_json::to_value(&entity).unwrap();

        assert!(value.get("rules").is_none());
        let restored = serde_json::from_value::<PlayerEntity>(value).unwrap();
        assert!(restored.rules.is_narrative());
    }
}
