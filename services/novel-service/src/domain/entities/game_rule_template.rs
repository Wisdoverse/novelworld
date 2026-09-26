use std::collections::HashSet;

pub use super::world_series::{SeriesRuleBinding, SeriesRuleContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const GAME_RULE_SCHEMA_VERSION: i32 = 1;
pub const GAME_RULE_PROMPT_VERSION: &str = "novel-game-rules-v1";
pub const BASIC_GAME_RULE_PROMPT_VERSION: &str = "novel-game-rules-v2";
pub const SERIES_GAME_RULE_PROMPT_VERSION: &str = "series-game-rules-v1";
pub const BASIC_ACTION_DESCRIPTION: &str = "在世界规则内处理该类行动的不确定结果";

pub fn supported_prompt_version(version: &str) -> bool {
    supported_novel_prompt_version(version) || version == SERIES_GAME_RULE_PROMPT_VERSION
}

pub fn supported_novel_prompt_version(version: &str) -> bool {
    matches!(
        version,
        GAME_RULE_PROMPT_VERSION | BASIC_GAME_RULE_PROMPT_VERSION
    )
}

// Only these server-owned words may appear in a basic template. The model
// chooses a novel-specific subset and numbers, never public story text.
pub fn basic_attribute(key: &str) -> Option<(&'static str, &'static str)> {
    Some(match key {
        "root" => ("根骨", "身体资质与基础耐受"),
        "agility" => ("身法", "移动、闪避与身体协调"),
        "vigor" => ("力道", "用力、冲撞与持续体能"),
        "insight" => ("悟性", "理解、推理与学习"),
        "fortune" => ("福缘", "处理偶然机会与环境机遇"),
        "strategy" => ("谋略", "分析局势与制定计划"),
        "command" => ("统御", "组织协作与协调行动"),
        "loyalty" => ("义理", "理解承诺、信任与互助"),
        "resolve" => ("心志", "承受压力并保持行动意志"),
        "influence" => ("交涉", "沟通、说服与协商"),
        "knowledge" => ("学识", "运用已掌握知识"),
        "craft" => ("技艺", "运用工具和实践技能"),
        _ => return None,
    })
}
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<SeriesRuleContext>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid game rule template: {0}")]
pub struct GameRuleTemplateError(String);

impl GameRuleTemplate {
    pub fn new_basic(
        novel_id: Uuid,
        canon_model_version: i32,
        attributes: Vec<GameAttribute>,
        action_rules: Vec<GameActionRule>,
    ) -> Result<Self, GameRuleTemplateError> {
        let mut template = Self::new(novel_id, canon_model_version, attributes, action_rules)?;
        template.prompt_version = BASIC_GAME_RULE_PROMPT_VERSION.into();
        template.validate(i32::MAX)?;
        Ok(template)
    }

    pub fn new(
        novel_id: Uuid,
        canon_model_version: i32,
        attributes: Vec<GameAttribute>,
        action_rules: Vec<GameActionRule>,
    ) -> Result<Self, GameRuleTemplateError> {
        let point_budget = attributes
            .iter()
            .try_fold(0_i32, |total, attribute| {
                total.checked_add(attribute.default_score)
            })
            .ok_or_else(|| GameRuleTemplateError("attribute point budget overflows".into()))?;
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
            series: None,
        };
        template.validate(i32::MAX)?;
        Ok(template)
    }

    pub fn validate(&self, maximum_source_chapter: i32) -> Result<(), GameRuleTemplateError> {
        match (&self.series, self.prompt_version.as_str()) {
            (Some(context), SERIES_GAME_RULE_PROMPT_VERSION) => context
                .validate()
                .map_err(|_| GameRuleTemplateError("series context is invalid".into()))?,
            (None, SERIES_GAME_RULE_PROMPT_VERSION) | (Some(_), _) => {
                return invalid("template series context does not match prompt version")
            }
            (None, _) => {}
        }
        if self.novel_id.is_nil() {
            return invalid("novel_id must not be nil");
        }
        if self.canon_model_version < 1
            || self.schema_version != GAME_RULE_SCHEMA_VERSION
            || !supported_prompt_version(&self.prompt_version)
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
            if matches!(
                self.prompt_version.as_str(),
                BASIC_GAME_RULE_PROMPT_VERSION | SERIES_GAME_RULE_PROMPT_VERSION
            ) && basic_attribute(&attribute.key)
                != Some((attribute.label.as_str(), attribute.description.as_str()))
            {
                return invalid("basic attribute text must match the server vocabulary");
            }
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
            if matches!(
                self.prompt_version.as_str(),
                BASIC_GAME_RULE_PROMPT_VERSION | SERIES_GAME_RULE_PROMPT_VERSION
            ) && rule.description != BASIC_ACTION_DESCRIPTION
            {
                return invalid("basic action text must match the server vocabulary");
            }
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
        if matches!(
            self.prompt_version.as_str(),
            BASIC_GAME_RULE_PROMPT_VERSION | SERIES_GAME_RULE_PROMPT_VERSION
        ) {
            // Basic rules expose only trusted vocabulary and bounded numbers.
            // Real citations are provenance, not story-unlock prerequisites.
            return (unlocked_chapter >= 1 && self.validate(i32::MAX).is_ok())
                .then(|| self.clone());
        }
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
