use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const GAME_RULE_SCHEMA_VERSION: i32 = 1;
pub const GAME_RULE_PROMPT_VERSION: &str = "novel-game-rules-v1";
pub const MIN_ATTRIBUTE_SCORE: i32 = 8;
pub const MAX_ATTRIBUTE_SCORE: i32 = 15;
const MIN_ATTRIBUTES: usize = 3;
const MAX_ATTRIBUTES: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameActionKind {
    Travel,
    Investigate,
    Converse,
    Ally,
    Oppose,
    AdvanceThread,
    ResolveThread,
    PursueGoal,
}

impl GameActionKind {
    pub const ALL: [Self; 8] = [
        Self::Travel,
        Self::Investigate,
        Self::Converse,
        Self::Ally,
        Self::Oppose,
        Self::AdvanceThread,
        Self::ResolveThread,
        Self::PursueGoal,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameAttribute {
    pub key: String,
    pub label: String,
    pub description: String,
    pub default_score: i32,
    pub source_chapters: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameActionRule {
    pub kind: GameActionKind,
    pub attribute_key: String,
    pub difficulty_class: i32,
    pub description: String,
    pub source_chapters: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameRuleTemplate {
    pub novel_id: Uuid,
    pub canon_model_version: i32,
    pub schema_version: i32,
    pub prompt_version: String,
    pub minimum_score: i32,
    pub maximum_score: i32,
    pub point_budget: i32,
    pub attributes: Vec<GameAttribute>,
    pub action_rules: Vec<GameActionRule>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid game rule template: {0}")]
pub struct GameRuleTemplateError(String);

impl GameRuleTemplate {
    pub fn new(
        novel_id: Uuid,
        canon_model_version: i32,
        attributes: Vec<GameAttribute>,
        action_rules: Vec<GameActionRule>,
    ) -> Result<Self, GameRuleTemplateError> {
        let point_budget = attributes
            .iter()
            .map(|attribute| attribute.default_score)
            .sum();
        let template = Self {
            novel_id,
            canon_model_version,
            schema_version: GAME_RULE_SCHEMA_VERSION,
            prompt_version: GAME_RULE_PROMPT_VERSION.into(),
            minimum_score: MIN_ATTRIBUTE_SCORE,
            maximum_score: MAX_ATTRIBUTE_SCORE,
            point_budget,
            attributes,
            action_rules,
        };
        template.validate(i32::MAX)?;
        Ok(template)
    }

    pub fn validate(&self, maximum_source_chapter: i32) -> Result<(), GameRuleTemplateError> {
        if self.novel_id.is_nil() {
            return invalid("novel_id must not be nil");
        }
        if self.canon_model_version < 1
            || self.schema_version != GAME_RULE_SCHEMA_VERSION
            || self.prompt_version != GAME_RULE_PROMPT_VERSION
        {
            return invalid("template version metadata is invalid");
        }
        if self.minimum_score != MIN_ATTRIBUTE_SCORE || self.maximum_score != MAX_ATTRIBUTE_SCORE {
            return invalid("attribute score bounds are invalid");
        }
        if !(MIN_ATTRIBUTES..=MAX_ATTRIBUTES).contains(&self.attributes.len()) {
            return invalid(format!(
                "attributes must contain {MIN_ATTRIBUTES}-{MAX_ATTRIBUTES} items"
            ));
        }

        let mut attribute_keys = HashSet::new();
        for attribute in &self.attributes {
            key("attribute key", &attribute.key)?;
            text("attribute label", &attribute.label, 40)?;
            text("attribute description", &attribute.description, 300)?;
            if !attribute_keys.insert(attribute.key.as_str()) {
                return invalid("attribute keys must be unique");
            }
            if !(self.minimum_score..=self.maximum_score).contains(&attribute.default_score) {
                return invalid("attribute default score is outside template bounds");
            }
            source_chapters(&attribute.source_chapters, maximum_source_chapter)?;
        }
        let expected_budget: i32 = self
            .attributes
            .iter()
            .map(|attribute| attribute.default_score)
            .sum();
        if self.point_budget != expected_budget {
            return invalid("point_budget must equal the sum of default scores");
        }

        if self.action_rules.len() != GameActionKind::ALL.len() {
            return invalid("action_rules must cover every supported action kind");
        }
        let mut action_kinds = HashSet::new();
        for rule in &self.action_rules {
            if !action_kinds.insert(rule.kind) {
                return invalid("action rule kinds must be unique");
            }
            if !attribute_keys.contains(rule.attribute_key.as_str()) {
                return invalid("action rule references an unknown attribute");
            }
            if !(5..=30).contains(&rule.difficulty_class) {
                return invalid("action difficulty class must be between 5 and 30");
            }
            text("action rule description", &rule.description, 300)?;
            source_chapters(&rule.source_chapters, maximum_source_chapter)?;
        }
        if GameActionKind::ALL
            .iter()
            .any(|kind| !action_kinds.contains(kind))
        {
            return invalid("action_rules are incomplete");
        }
        Ok(())
    }

    pub fn visible_at(&self, unlocked_chapter: i32) -> Option<Self> {
        // A template version is immutable. Returning a progress-filtered shape
        // under the same version would make an existing player sheet invalid
        // as soon as another attribute became visible. Expose the exact shared
        // template only when every citation is already unlocked.
        (unlocked_chapter >= 1 && self.validate(unlocked_chapter).is_ok()).then(|| self.clone())
    }
}

fn source_chapters(
    chapters: &[i32],
    maximum_source_chapter: i32,
) -> Result<(), GameRuleTemplateError> {
    if chapters.is_empty()
        || chapters.len() > 16
        || chapters
            .iter()
            .any(|chapter| *chapter < 1 || *chapter > maximum_source_chapter)
        || chapters.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return invalid("source_chapters must be sorted, unique, and within the novel");
    }
    Ok(())
}

fn key(name: &str, value: &str) -> Result<(), GameRuleTemplateError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return invalid(format!("{name} must be a lowercase ASCII key"));
    }
    Ok(())
}

fn text(name: &str, value: &str, maximum: usize) -> Result<(), GameRuleTemplateError> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > maximum
        || value
            .chars()
            .any(|character| character.is_control() && character != '\n')
    {
        return invalid(format!("{name} is empty, untrimmed, or too long"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, GameRuleTemplateError> {
    Err(GameRuleTemplateError(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> GameRuleTemplate {
        let attributes = ["vigor", "insight", "influence"]
            .into_iter()
            .map(|key| GameAttribute {
                key: key.into(),
                label: key.into(),
                description: format!("{key} description"),
                default_score: 10,
                source_chapters: vec![1],
            })
            .collect::<Vec<_>>();
        let action_rules = GameActionKind::ALL
            .into_iter()
            .enumerate()
            .map(|(index, kind)| GameActionRule {
                kind,
                attribute_key: attributes[index % attributes.len()].key.clone(),
                difficulty_class: 10 + i32::try_from(index % 3).unwrap(),
                description: "Resolve an uncertain action".into(),
                source_chapters: vec![1],
            })
            .collect();
        GameRuleTemplate::new(Uuid::new_v4(), 1, attributes, action_rules).unwrap()
    }

    #[test]
    fn validates_bounded_complete_templates() {
        let template = template();
        template.validate(2).unwrap();
        assert_eq!(template.point_budget, 30);

        let mut incomplete = template.clone();
        incomplete.action_rules.pop();
        assert!(incomplete.validate(2).is_err());

        let mut unknown = template;
        unknown.action_rules[0].attribute_key = "missing".into();
        assert!(unknown.validate(2).is_err());
    }

    #[test]
    fn hides_incomplete_rules_instead_of_leaking_future_systems() {
        let mut template = template();
        template.attributes[0].source_chapters = vec![2];
        assert!(template.visible_at(1).is_none());
        assert!(template.visible_at(2).is_some());
    }
}
