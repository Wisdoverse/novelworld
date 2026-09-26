use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entities::{
    player_entity::PlayerEntity,
    world_session::{WorldAction, WorldActionKind, WorldSession},
};

pub const GAME_RULE_SCHEMA_VERSION: i32 = 1;
pub const GAME_RULE_PROMPT_VERSION: &str = "novel-game-rules-v1";
pub const BASIC_GAME_RULE_PROMPT_VERSION: &str = "novel-game-rules-v2";
pub const SERIES_GAME_RULE_PROMPT_VERSION: &str = "series-game-rules-v1";
pub const BASIC_ACTION_DESCRIPTION: &str = "在世界规则内处理该类行动的不确定结果";
pub const ACTION_ADJUDICATION_CONTEXT_LIMIT: usize = 8 * 1024;

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
    pub kind: WorldActionKind,
    pub attribute_key: String,
    pub difficulty_class: i32,
    pub description: String,
    pub source_chapters: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesRuleBinding {
    pub series_id: Uuid,
    pub revision: i32,
}

impl SeriesRuleBinding {
    pub fn validate(&self) -> Result<(), GameRulesError> {
        if self.series_id.is_nil() || self.revision != 1 {
            return invalid("series binding identity or revision is invalid");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesSetting {
    pub binding: SeriesRuleBinding,
    pub name: String,
    pub background: String,
}

impl SeriesSetting {
    pub fn validate(&self) -> Result<(), GameRulesError> {
        self.binding.validate()?;
        text(&self.name, 80)?;
        text(&self.background, 2_000)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesRuleContext {
    pub binding: SeriesRuleBinding,
    pub target_novel_id: Uuid,
    pub name: String,
    pub background: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameRuleTemplate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<SeriesRuleContext>,
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

impl GameRuleTemplate {
    pub fn validate(&self) -> Result<(), GameRulesError> {
        if self.novel_id.is_nil()
            || self.canon_model_version < 1
            || self.schema_version != GAME_RULE_SCHEMA_VERSION
            || !supported_prompt_version(&self.prompt_version)
            || self.minimum_score != 8
            || self.maximum_score != 15
            || !(3..=6).contains(&self.attributes.len())
        {
            return invalid("template identity, versions, or bounds are invalid");
        }
        validate_series_binding(
            &self.prompt_version,
            self.series.as_ref().map(|series| &series.binding),
        )?;
        if let Some(series) = &self.series {
            if series.target_novel_id.is_nil() {
                return invalid("series target novel is invalid");
            }
            text(&series.name, 80)?;
            text(&series.background, 2_000)?;
        }
        let mut keys = HashSet::new();
        for attribute in &self.attributes {
            key(&attribute.key)?;
            text(&attribute.label, 40)?;
            text(&attribute.description, 300)?;
            source_chapters(&attribute.source_chapters)?;
            if is_basic_prompt_version(&self.prompt_version)
                && basic_attribute(&attribute.key)
                    != Some((attribute.label.as_str(), attribute.description.as_str()))
            {
                return invalid("basic template attribute is not in the trusted vocabulary");
            }
            if !keys.insert(attribute.key.as_str())
                || !(self.minimum_score..=self.maximum_score).contains(&attribute.default_score)
            {
                return invalid("template attributes are invalid");
            }
        }
        if self.point_budget
            != self
                .attributes
                .iter()
                .map(|attribute| attribute.default_score)
                .sum::<i32>()
        {
            return invalid("point budget does not match default scores");
        }
        let all_kinds = [
            WorldActionKind::Travel,
            WorldActionKind::Investigate,
            WorldActionKind::Converse,
            WorldActionKind::Ally,
            WorldActionKind::Oppose,
            WorldActionKind::AdvanceThread,
            WorldActionKind::ResolveThread,
            WorldActionKind::PursueGoal,
        ];
        let mut kinds = HashSet::new();
        if self.action_rules.len() != all_kinds.len() {
            return invalid("action rules are incomplete");
        }
        for rule in &self.action_rules {
            text(&rule.description, 300)?;
            source_chapters(&rule.source_chapters)?;
            if is_basic_prompt_version(&self.prompt_version)
                && rule.description != BASIC_ACTION_DESCRIPTION
            {
                return invalid("basic template action description is invalid");
            }
            if !kinds.insert(rule.kind)
                || !keys.contains(rule.attribute_key.as_str())
                || !(5..=30).contains(&rule.difficulty_class)
            {
                return invalid("action rule is invalid");
            }
        }
        if all_kinds.iter().any(|kind| !kinds.contains(kind)) {
            return invalid("action rules are incomplete");
        }
        Ok(())
    }

    pub fn applies_to_novel(&self, novel_id: Uuid) -> bool {
        self.series
            .as_ref()
            .map_or(self.novel_id == novel_id, |series| {
                series.target_novel_id == novel_id
            })
    }

    pub fn series_setting(&self) -> Option<SeriesSetting> {
        self.series.as_ref().map(|series| SeriesSetting {
            binding: series.binding.clone(),
            name: series.name.clone(),
            background: series.background.clone(),
        })
    }

    pub fn binding(&self) -> Option<&SeriesRuleBinding> {
        self.series.as_ref().map(|series| &series.binding)
    }

    pub fn rule_for(&self, kind: WorldActionKind) -> Option<&GameActionRule> {
        self.action_rules.iter().find(|rule| rule.kind == kind)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionMode {
    #[default]
    Narrative,
    Advanced,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRuleProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_binding: Option<SeriesRuleBinding>,
    #[serde(default)]
    pub mode: ResolutionMode,
    pub canon_model_version: Option<i32>,
    pub template_schema_version: Option<i32>,
    pub template_prompt_version: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, i32>,
}

impl PlayerRuleProfile {
    pub fn narrative() -> Self {
        Self::default()
    }

    pub fn is_narrative(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self) -> Result<(), GameRulesError> {
        match self.mode {
            ResolutionMode::Narrative => {
                if self.canon_model_version.is_some()
                    || self.template_schema_version.is_some()
                    || self.template_prompt_version.is_some()
                    || !self.attributes.is_empty()
                    || self.series_binding.is_some()
                {
                    return invalid("narrative mode cannot bind an advanced template");
                }
            }
            ResolutionMode::Advanced => {
                validate_series_binding(
                    self.template_prompt_version.as_deref().unwrap_or_default(),
                    self.series_binding.as_ref(),
                )?;
                if self.canon_model_version.is_none_or(|version| version < 1)
                    || self.template_schema_version != Some(GAME_RULE_SCHEMA_VERSION)
                    || !self
                        .template_prompt_version
                        .as_deref()
                        .is_some_and(supported_prompt_version)
                    || !(3..=6).contains(&self.attributes.len())
                    || self.attributes.iter().any(|(name, score)| {
                        key(name).is_err()
                            || !(8..=15).contains(score)
                            || (self
                                .template_prompt_version
                                .as_deref()
                                .is_some_and(is_basic_prompt_version)
                                && basic_attribute(name).is_none())
                    })
                {
                    return invalid("advanced mode template binding or attributes are invalid");
                }
            }
        }
        Ok(())
    }

    pub fn validate_against(&self, template: &GameRuleTemplate) -> Result<(), GameRulesError> {
        self.validate()?;
        template.validate()?;
        if self.mode != ResolutionMode::Advanced
            || self.canon_model_version != Some(template.canon_model_version)
            || self.template_schema_version != Some(template.schema_version)
            || self.template_prompt_version.as_deref() != Some(template.prompt_version.as_str())
            || self.attributes.len() != template.attributes.len()
            || self.series_binding.as_ref() != template.binding()
        {
            return invalid("player profile does not bind the requested template");
        }
        let mut total = 0;
        for attribute in &template.attributes {
            let score = self
                .attributes
                .get(&attribute.key)
                .ok_or_else(|| GameRulesError("player attribute set is incomplete".into()))?;
            if !(template.minimum_score..=template.maximum_score).contains(score) {
                return invalid("player attribute score is outside template bounds");
            }
            total += score;
        }
        if total != template.point_budget {
            return invalid("player attributes must spend the exact point budget");
        }
        Ok(())
    }
}

/// A bounded model classification, never executable rules or a supplied roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjudicationDecision {
    Pending,
    TemplateFallback,
    Impossible,
    AutomaticSuccess,
    EasyCheck,
    StandardCheck,
    HardCheck,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionAdjudication {
    pub schema_version: i32,
    pub template_difficulty_class: i32,
    pub decision: AdjudicationDecision,
}

/// Authorized display facts only; no routing IDs, history, raw novel or dice.
#[derive(Debug, Clone, Serialize)]
pub struct ActionAdjudicationContext {
    pub kind: WorldActionKind,
    pub intent: String,
    pub target: Option<String>,
    pub location: String,
    pub background: String,
    pub capabilities: Vec<String>,
    pub inventory: Vec<String>,
    pub hard_rules: Vec<String>,
    pub attribute_label: String,
    pub attribute_description: String,
    pub attribute_score: i32,
    pub template_difficulty_class: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionCheck {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_binding: Option<SeriesRuleBinding>,
    pub schema_version: i32,
    pub canon_model_version: i32,
    pub template_prompt_version: String,
    pub attribute_key: String,
    pub attribute_label: String,
    pub score: i32,
    pub modifier: i32,
    pub roll: i32,
    pub difficulty_class: i32,
    pub total: i32,
    pub succeeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<ActionAdjudication>,
}

impl ActionCheck {
    pub fn validate(&self) -> Result<(), GameRulesError> {
        if self.schema_version != GAME_RULE_SCHEMA_VERSION
            || self.canon_model_version < 1
            || !supported_prompt_version(&self.template_prompt_version)
            || !(8..=15).contains(&self.score)
            || self.modifier != (self.score - 10).div_euclid(2)
            || !(1..=20).contains(&self.roll)
            || !(5..=30).contains(&self.difficulty_class)
            || self.total != self.roll + self.modifier
        {
            return invalid("action check arithmetic or versions are invalid");
        }
        validate_series_binding(&self.template_prompt_version, self.series_binding.as_ref())?;
        let expected_success = match &self.adjudication {
            Some(adjudication) => {
                if adjudication.schema_version != 1
                    || !(5..=30).contains(&adjudication.template_difficulty_class)
                {
                    return invalid("action adjudication version or base DC is invalid");
                }
                let base = adjudication.template_difficulty_class;
                let expected_dc = match adjudication.decision {
                    AdjudicationDecision::EasyCheck => (base - 5).max(5),
                    AdjudicationDecision::HardCheck => (base + 5).min(30),
                    _ => base,
                };
                if self.difficulty_class != expected_dc {
                    return invalid("action adjudication DC mapping is invalid");
                }
                match adjudication.decision {
                    AdjudicationDecision::Impossible => false,
                    AdjudicationDecision::AutomaticSuccess => true,
                    _ => self.total >= self.difficulty_class,
                }
            }
            None => self.total >= self.difficulty_class,
        };
        if self.succeeded != expected_success {
            return invalid("action check result conflicts with its decision");
        }
        key(&self.attribute_key)?;
        text(&self.attribute_label, 40)?;
        if is_basic_prompt_version(&self.template_prompt_version)
            && basic_attribute(&self.attribute_key).map(|(label, _)| label)
                != Some(self.attribute_label.as_str())
        {
            return invalid("basic action check attribute is invalid");
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        template: &GameRuleTemplate,
        kind: WorldActionKind,
    ) -> Result<(), GameRulesError> {
        self.validate_resolved()?;
        template.validate()?;
        let rule = template
            .rule_for(kind)
            .ok_or_else(|| GameRulesError("check action rule is unavailable".into()))?;
        let attribute = template
            .attributes
            .iter()
            .find(|attribute| attribute.key == rule.attribute_key)
            .ok_or_else(|| GameRulesError("check attribute is unavailable".into()))?;
        let base_dc = self
            .adjudication
            .as_ref()
            .map_or(self.difficulty_class, |value| {
                value.template_difficulty_class
            });
        if self.canon_model_version != template.canon_model_version
            || self.template_prompt_version != template.prompt_version
            || self.series_binding.as_ref() != template.binding()
            || self.attribute_key != attribute.key
            || self.attribute_label != attribute.label
            || base_dc != rule.difficulty_class
        {
            return invalid("action check does not bind its frozen template");
        }
        Ok(())
    }

    pub fn validate_resolved(&self) -> Result<(), GameRulesError> {
        self.validate()?;
        if self.adjudication_pending() {
            return invalid("action adjudication has not been frozen");
        }
        Ok(())
    }

    pub fn adjudication_pending(&self) -> bool {
        self.adjudication
            .as_ref()
            .is_some_and(|value| value.decision == AdjudicationDecision::Pending)
    }

    pub fn with_pending_adjudication(mut self) -> Self {
        self.adjudication = Some(ActionAdjudication {
            schema_version: 1,
            template_difficulty_class: self.difficulty_class,
            decision: AdjudicationDecision::Pending,
        });
        self
    }

    pub fn settle_adjudication(
        &self,
        decision: AdjudicationDecision,
    ) -> Result<Self, GameRulesError> {
        self.validate()?;
        if !self.adjudication_pending() || decision == AdjudicationDecision::Pending {
            return invalid("only a pending adjudication may be settled");
        }
        let mut check = self.clone();
        let adjudication = check
            .adjudication
            .as_mut()
            .expect("pending metadata exists");
        adjudication.decision = decision;
        check.difficulty_class = match decision {
            AdjudicationDecision::EasyCheck => (adjudication.template_difficulty_class - 5).max(5),
            AdjudicationDecision::HardCheck => (adjudication.template_difficulty_class + 5).min(30),
            _ => adjudication.template_difficulty_class,
        };
        check.succeeded = match decision {
            AdjudicationDecision::Impossible => false,
            AdjudicationDecision::AutomaticSuccess => true,
            _ => check.total >= check.difficulty_class,
        };
        check.validate_resolved()?;
        Ok(check)
    }
}

pub fn resolve_action_check(
    template: &GameRuleTemplate,
    profile: &PlayerRuleProfile,
    kind: WorldActionKind,
    roll: u8,
) -> Result<ActionCheck, GameRulesError> {
    profile.validate_against(template)?;
    if !(1..=20).contains(&roll) {
        return invalid("D20 roll must be between 1 and 20");
    }
    let rule = template
        .rule_for(kind)
        .ok_or_else(|| GameRulesError("action kind has no rule".into()))?;
    let attribute = template
        .attributes
        .iter()
        .find(|attribute| attribute.key == rule.attribute_key)
        .ok_or_else(|| GameRulesError("action rule attribute is missing".into()))?;
    let score = *profile
        .attributes
        .get(&attribute.key)
        .ok_or_else(|| GameRulesError("player attribute is missing".into()))?;
    let modifier = (score - 10).div_euclid(2);
    let total = i32::from(roll) + modifier;
    let check = ActionCheck {
        series_binding: template.binding().cloned(),
        schema_version: template.schema_version,
        canon_model_version: template.canon_model_version,
        template_prompt_version: template.prompt_version.clone(),
        attribute_key: attribute.key.clone(),
        attribute_label: attribute.label.clone(),
        score,
        modifier,
        roll: i32::from(roll),
        difficulty_class: rule.difficulty_class,
        total,
        succeeded: total >= rule.difficulty_class,
        adjudication: None,
    };
    check.validate()?;
    Ok(check)
}

pub fn build_action_adjudication_context(
    player: &PlayerEntity,
    session: &WorldSession,
    action: &WorldAction,
    check: &ActionCheck,
) -> Option<ActionAdjudicationContext> {
    let context = &session.entry_context;
    let target = match action.target_id.as_deref() {
        None if action.kind == WorldActionKind::PursueGoal => None,
        Some(target) => Some(match action.kind {
            WorldActionKind::Converse | WorldActionKind::Ally | WorldActionKind::Oppose => {
                let id = Uuid::parse_str(target).ok()?;
                context
                    .characters
                    .iter()
                    .find(|item| item.id == id)?
                    .name
                    .clone()
            }
            WorldActionKind::PursueGoal => context
                .character_goals
                .iter()
                .find(|item| item.id == target)?
                .description
                .clone(),
            WorldActionKind::Travel => context
                .locations
                .iter()
                .find(|item| item.id == target)?
                .name
                .clone(),
            WorldActionKind::AdvanceThread | WorldActionKind::ResolveThread => context
                .threads
                .iter()
                .find(|item| item.id == target)?
                .name
                .clone(),
            WorldActionKind::Investigate => {
                let mut matches = context
                    .locations
                    .iter()
                    .chain(&context.threads)
                    .filter(|item| item.id == target);
                let item = matches.next()?;
                if matches.next().is_some() {
                    return None;
                }
                item.name.clone()
            }
        }),
        None => return None,
    };
    let attribute = session
        .game_rules
        .as_ref()?
        .attributes
        .iter()
        .find(|attribute| attribute.key == check.attribute_key)?;
    let context = ActionAdjudicationContext {
        kind: action.kind,
        intent: action.intent.clone(),
        target,
        location: context
            .locations
            .iter()
            .find(|item| item.id == player.location_id)?
            .name
            .clone(),
        background: player.background.clone(),
        capabilities: player.capabilities.clone(),
        inventory: player.inventory.clone(),
        hard_rules: context
            .hard_rules
            .iter()
            .map(|item| item.description.clone())
            .collect(),
        attribute_label: check.attribute_label.clone(),
        attribute_description: attribute.description.clone(),
        attribute_score: check.score,
        template_difficulty_class: check.adjudication.as_ref()?.template_difficulty_class,
    };
    (serde_json::to_vec(&context).ok()?.len() <= ACTION_ADJUDICATION_CONTEXT_LIMIT)
        .then_some(context)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid game rules: {0}")]
pub struct GameRulesError(String);

fn source_chapters(chapters: &[i32]) -> Result<(), GameRulesError> {
    if chapters.is_empty()
        || chapters.len() > 16
        || chapters.iter().any(|chapter| *chapter < 1)
        || chapters.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return invalid("source chapters are invalid");
    }
    Ok(())
}

fn key(value: &str) -> Result<(), GameRulesError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return invalid("attribute key is invalid");
    }
    Ok(())
}

fn text(value: &str, maximum: usize) -> Result<(), GameRulesError> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > maximum
        || value
            .chars()
            .any(|character| character.is_control() && character != '\n')
    {
        return invalid("game rule text is invalid");
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, GameRulesError> {
    Err(GameRulesError(message.into()))
}

fn supported_prompt_version(version: &str) -> bool {
    version == GAME_RULE_PROMPT_VERSION || is_basic_prompt_version(version)
}

fn is_basic_prompt_version(version: &str) -> bool {
    version == BASIC_GAME_RULE_PROMPT_VERSION || version == SERIES_GAME_RULE_PROMPT_VERSION
}

fn validate_series_binding(
    version: &str,
    binding: Option<&SeriesRuleBinding>,
) -> Result<(), GameRulesError> {
    match (version == SERIES_GAME_RULE_PROMPT_VERSION, binding) {
        (true, Some(binding)) => binding.validate(),
        (false, None) => Ok(()),
        _ => invalid("series prompt and binding must be used together"),
    }
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
        let kinds = [
            WorldActionKind::Travel,
            WorldActionKind::Investigate,
            WorldActionKind::Converse,
            WorldActionKind::Ally,
            WorldActionKind::Oppose,
            WorldActionKind::AdvanceThread,
            WorldActionKind::ResolveThread,
            WorldActionKind::PursueGoal,
        ];
        GameRuleTemplate {
            series: None,
            novel_id: Uuid::new_v4(),
            canon_model_version: 1,
            schema_version: GAME_RULE_SCHEMA_VERSION,
            prompt_version: GAME_RULE_PROMPT_VERSION.into(),
            minimum_score: 8,
            maximum_score: 15,
            point_budget: 30,
            action_rules: kinds
                .into_iter()
                .enumerate()
                .map(|(index, kind)| GameActionRule {
                    kind,
                    attribute_key: attributes[index % attributes.len()].key.clone(),
                    difficulty_class: 11,
                    description: "Resolve uncertain action".into(),
                    source_chapters: vec![1],
                })
                .collect(),
            attributes,
        }
    }

    fn profile() -> PlayerRuleProfile {
        PlayerRuleProfile {
            series_binding: None,
            mode: ResolutionMode::Advanced,
            canon_model_version: Some(1),
            template_schema_version: Some(GAME_RULE_SCHEMA_VERSION),
            template_prompt_version: Some(GAME_RULE_PROMPT_VERSION.into()),
            attributes: BTreeMap::from([
                ("vigor".into(), 12),
                ("insight".into(), 10),
                ("influence".into(), 8),
            ]),
        }
    }

    fn basic_template() -> GameRuleTemplate {
        let mut template = template();
        template.prompt_version = BASIC_GAME_RULE_PROMPT_VERSION.into();
        for attribute in &mut template.attributes {
            let (label, description) = basic_attribute(&attribute.key).unwrap();
            attribute.label = label.into();
            attribute.description = description.into();
            attribute.source_chapters = vec![2];
        }
        for rule in &mut template.action_rules {
            rule.description = BASIC_ACTION_DESCRIPTION.into();
            rule.source_chapters = vec![2];
        }
        template
    }

    #[test]
    fn basic_templates_allow_full_world_rule_provenance_but_only_trusted_text() {
        let template = basic_template();
        assert!(template.validate().is_ok());

        let mut forged = template.clone();
        forged.attributes[0].description = "自由生成的能力说明".into();
        assert!(forged.validate().is_err());

        let mut forged = template.clone();
        forged.action_rules[0].description = "自由生成的动作说明".into();
        assert!(forged.validate().is_err());

        let mut unsupported = template;
        unsupported.prompt_version = "novel-game-rules-v3".into();
        assert!(unsupported.validate().is_err());
    }

    #[test]
    fn basic_profile_and_checks_validate_exact_vocabulary_while_v1_remains_valid() {
        let template = basic_template();
        let profile = PlayerRuleProfile {
            series_binding: None,
            mode: ResolutionMode::Advanced,
            canon_model_version: Some(template.canon_model_version),
            template_schema_version: Some(template.schema_version),
            template_prompt_version: Some(BASIC_GAME_RULE_PROMPT_VERSION.into()),
            attributes: BTreeMap::from([
                ("vigor".into(), 12),
                ("insight".into(), 10),
                ("influence".into(), 8),
            ]),
        };
        assert!(profile.validate_against(&template).is_ok());
        let mut forged_profile = profile.clone();
        forged_profile.attributes.insert("untrusted".into(), 8);
        forged_profile.attributes.remove("vigor");
        assert!(forged_profile.validate().is_err());
        let check = resolve_action_check(&template, &profile, WorldActionKind::Travel, 10).unwrap();
        assert!(check.validate_resolved().is_ok());
        let replayed =
            serde_json::from_value::<ActionCheck>(serde_json::to_value(&check).unwrap()).unwrap();
        assert_eq!(replayed, check);
        assert!(replayed.validate_resolved().is_ok());

        let mut forged = check;
        forged.attribute_label = "伪造标签".into();
        assert!(forged.validate().is_err());
    }

    fn series_template(target_novel_id: Uuid) -> GameRuleTemplate {
        let mut template = basic_template();
        template.canon_model_version = 7;
        template.prompt_version = SERIES_GAME_RULE_PROMPT_VERSION.into();
        template.series = Some(SeriesRuleContext {
            binding: SeriesRuleBinding {
                series_id: Uuid::new_v4(),
                revision: 1,
            },
            target_novel_id,
            name: "暮城系列".into(),
            background: "共同设定：古城居民在城门附近生活。".into(),
        });
        template.attributes[0].source_chapters = vec![99];
        template
    }

    fn series_profile(template: &GameRuleTemplate) -> PlayerRuleProfile {
        PlayerRuleProfile {
            series_binding: template.binding().cloned(),
            mode: ResolutionMode::Advanced,
            canon_model_version: Some(template.canon_model_version),
            template_schema_version: Some(template.schema_version),
            template_prompt_version: Some(template.prompt_version.clone()),
            attributes: template
                .attributes
                .iter()
                .map(|attribute| (attribute.key.clone(), attribute.default_score))
                .collect(),
        }
    }

    #[test]
    fn series_binding_is_required_exact_and_keeps_source_provenance() {
        let target = Uuid::new_v4();
        let template = series_template(target);
        let profile = series_profile(&template);
        profile.validate_against(&template).unwrap();
        assert_ne!(template.novel_id, target);
        assert!(template.applies_to_novel(target));
        assert!(!template.applies_to_novel(template.novel_id));
        assert_eq!(template.attributes[0].source_chapters, vec![99]);
        let check = resolve_action_check(&template, &profile, WorldActionKind::Travel, 20).unwrap();
        assert_eq!(check.canon_model_version, 7);
        assert_eq!(check.series_binding.as_ref(), template.binding());
        check
            .validate_against(&template, WorldActionKind::Travel)
            .unwrap();
        let mut forged = profile;
        forged.series_binding.as_mut().unwrap().series_id = Uuid::new_v4();
        assert!(forged.validate_against(&template).is_err());
        let mut forged = check;
        forged.series_binding.as_mut().unwrap().series_id = Uuid::new_v4();
        assert!(forged
            .validate_against(&template, WorldActionKind::Travel)
            .is_err());
        let mut forged = template.clone();
        forged.attributes[0].description = "后文章节秘密".into();
        assert!(forged.validate().is_err());
        let mut missing = template;
        missing.series = None;
        assert!(missing.validate().is_err());
        let mut mislabeled = basic_template();
        mislabeled.series = forged.series;
        assert!(mislabeled.validate().is_err());
        assert!(serde_json::to_value(basic_template())
            .unwrap()
            .get("series")
            .is_none());
        assert!(serde_json::to_value(PlayerRuleProfile::narrative())
            .unwrap()
            .get("series_binding")
            .is_none());
    }

    #[test]
    fn series_session_freezes_source_rules_with_independent_target_canon() {
        use crate::domain::entities::narrative_node::WorldState;
        use crate::domain::entities::world_session::{
            build_world_turn_prompt_with_check, parse_world_turn_transition_with_check,
            WorldEntityRef, WorldEntryContext,
        };
        let target = Uuid::new_v4();
        let template = series_template(target);
        let profile = series_profile(&template);
        let context = WorldEntryContext {
            series_setting: template.series_setting(),
            model_version: 2,
            checkpoint_chapter: 1,
            unlocked_through_chapter: 1,
            characters: vec![],
            locations: vec![WorldEntityRef {
                id: "gate".into(),
                name: "城门".into(),
            }],
            factions: vec![],
            hard_rules: vec![],
            dead_character_ids: vec![],
            threads: vec![],
            scheduled_events: vec![],
            character_goals: vec![],
        };
        let player = PlayerEntity::new_with_rules(
            Uuid::new_v4(),
            target,
            1,
            "云舟".into(),
            "远行者".into(),
            vec!["观察".into()],
            "gate".into(),
            vec![],
            profile,
        )
        .unwrap();
        let mut state = WorldState::new(player.user_id, target);
        state.state["player_entity"] = serde_json::to_value(&player).unwrap();
        let session = state
            .start_open_world_with_rules(&context, Some(&template))
            .unwrap();
        assert_eq!(session.entry_context.model_version, 2);
        assert_eq!(session.game_rules.as_ref().unwrap().canon_model_version, 7);
        let mut changed_context = context.clone();
        changed_context.series_setting.as_mut().unwrap().background = "新背景不覆盖旧世界".into();
        let frozen = state
            .start_open_world_with_rules(&changed_context, Some(&template))
            .unwrap();
        assert_eq!(frozen, session);
        let restored =
            serde_json::from_value::<WorldSession>(serde_json::to_value(&frozen).unwrap()).unwrap();
        restored.validate().unwrap();
        let action = WorldAction {
            kind: WorldActionKind::PursueGoal,
            target_id: None,
            intent: "继续探索".into(),
        };
        let check = resolve_action_check(&template, &player.rules, action.kind, 20).unwrap();
        let raw = serde_json::json!({"schema_version":1,"rendered_narrative":"旅人继续在城门附近探索。","events":[{"summary":"旅人观察城门。","actor_character_ids":[],"location_id":"gate"}],"relationship_changes":[],"location_changes":[],"thread_changes":[],"player_location_id":null,"inventory_additions":[],"inventory_removals":[],"knowledge_discoveries":[],"faction_changes":[],"canonical_event_change":null}).to_string();
        let transition = parse_world_turn_transition_with_check(
            &raw,
            &action,
            &context,
            &restored,
            Some(&check),
        )
        .unwrap();
        assert_eq!(transition.canon_model_version, 2);
        let mut wrong_target = transition.clone();
        wrong_target.canon_model_version = 7;
        assert!(wrong_target
            .validate_against_with_check(&action, &context, &restored, Some(&check))
            .is_err());
        let mut wrong_source = check.clone();
        wrong_source.canon_model_version = 2;
        assert!(transition
            .validate_against_with_check(&action, &context, &restored, Some(&wrong_source))
            .is_err());
        let prompt = build_world_turn_prompt_with_check(
            "暮城",
            &player,
            &action,
            &restored,
            &state.state,
            &[],
            Some(&check),
        )
        .unwrap();
        assert!(prompt.contains("共同设定"));
        assert!(prompt.contains("hard_rules take precedence"));
        let mut future_context = context.clone();
        future_context
            .hard_rules
            .push(crate::domain::entities::world_session::WorldRuleRef {
                id: "future".into(),
                description: "隐蔽信息".into(),
            });
        assert!(transition
            .validate_against_with_check(&action, &future_context, &restored, Some(&check))
            .is_err());
        let mut another = template;
        another.series.as_mut().unwrap().target_novel_id = Uuid::new_v4();
        let mut fresh_state = WorldState::new(player.user_id, target);
        fresh_state.state["player_entity"] = serde_json::to_value(&player).unwrap();
        assert!(fresh_state
            .start_open_world_with_rules(&context, Some(&another))
            .is_err());
    }

    #[test]
    fn resolves_a_bounded_d20_check() {
        let template = template();
        let profile = profile();
        let check = resolve_action_check(&template, &profile, WorldActionKind::Travel, 10).unwrap();
        assert_eq!(check.modifier, 1);
        assert_eq!(check.total, 11);
        assert!(check.succeeded);
    }

    #[test]
    fn adjudication_maps_only_bounded_code_owned_results_without_rerolling() {
        let pending = resolve_action_check(&template(), &profile(), WorldActionKind::Travel, 10)
            .unwrap()
            .with_pending_adjudication();
        assert!(pending.validate().is_ok());
        assert!(pending.validate_resolved().is_err());
        for (decision, dc, success) in [
            (AdjudicationDecision::TemplateFallback, 11, true),
            (AdjudicationDecision::Impossible, 11, false),
            (AdjudicationDecision::AutomaticSuccess, 11, true),
            (AdjudicationDecision::EasyCheck, 6, true),
            (AdjudicationDecision::StandardCheck, 11, true),
            (AdjudicationDecision::HardCheck, 16, false),
        ] {
            let settled = pending.settle_adjudication(decision).unwrap();
            assert_eq!((settled.difficulty_class, settled.succeeded), (dc, success));
            assert_eq!(
                (settled.roll, settled.score, settled.modifier, settled.total),
                (pending.roll, pending.score, pending.modifier, pending.total)
            );
            assert!(settled.validate_resolved().is_ok());
            assert!(settled
                .settle_adjudication(AdjudicationDecision::Impossible)
                .is_err());
        }
        let failing = resolve_action_check(&template(), &profile(), WorldActionKind::Travel, 1)
            .unwrap()
            .with_pending_adjudication();
        assert!(
            failing
                .settle_adjudication(AdjudicationDecision::AutomaticSuccess)
                .unwrap()
                .succeeded
        );
        assert!(pending
            .settle_adjudication(AdjudicationDecision::Pending)
            .is_err());
        for (base, decision, dc) in [
            (5, AdjudicationDecision::EasyCheck, 5),
            (30, AdjudicationDecision::HardCheck, 30),
        ] {
            let mut template = template();
            template.action_rules[0].difficulty_class = base;
            let settled = resolve_action_check(&template, &profile(), WorldActionKind::Travel, 20)
                .unwrap()
                .with_pending_adjudication()
                .settle_adjudication(decision)
                .unwrap();
            assert_eq!(settled.difficulty_class, dc);
        }
    }

    #[test]
    fn legacy_checks_keep_their_wire_shape_and_forged_semantic_results_fail() {
        let legacy =
            resolve_action_check(&template(), &profile(), WorldActionKind::Travel, 10).unwrap();
        let wire = serde_json::to_value(&legacy).unwrap();
        assert!(wire.get("adjudication").is_none());
        assert_eq!(serde_json::from_value::<ActionCheck>(wire).unwrap(), legacy);
        let settled = legacy
            .with_pending_adjudication()
            .settle_adjudication(AdjudicationDecision::HardCheck)
            .unwrap();
        let mut forged = settled.clone();
        forged.succeeded = true;
        assert!(forged.validate_resolved().is_err());
        forged = settled.clone();
        forged.difficulty_class = 5;
        assert!(forged.validate_resolved().is_err());
        forged = settled.clone();
        forged.adjudication.as_mut().unwrap().schema_version = 2;
        assert!(forged.validate_resolved().is_err());
        forged = settled;
        forged.roll = 0;
        assert!(forged.validate_resolved().is_err());
    }

    #[test]
    fn rejects_invalid_player_allocations_and_template_versions() {
        let template = template();
        let mut missing = profile();
        missing.attributes.remove("influence");
        assert!(missing.validate_against(&template).is_err());

        let mut extra = profile();
        extra.attributes.insert("luck".into(), 8);
        assert!(extra.validate_against(&template).is_err());

        for invalid_score in [7, 16] {
            let mut outside_bounds = profile();
            outside_bounds
                .attributes
                .insert("vigor".into(), invalid_score);
            assert!(outside_bounds.validate_against(&template).is_err());
        }

        let mut wrong_total = profile();
        wrong_total.attributes.insert("insight".into(), 9);
        assert!(wrong_total.validate_against(&template).is_err());

        let mut wrong_version = profile();
        wrong_version.canon_model_version = Some(2);
        assert!(wrong_version.validate_against(&template).is_err());
    }
}
