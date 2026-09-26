use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entities::{
    player_entity::PlayerEntity,
    world_session::{WorldAction, WorldActionKind, WorldSession},
};

pub const GAME_RULE_SCHEMA_VERSION: i32 = 1;
pub const GAME_RULE_PROMPT_VERSION: &str = "novel-game-rules-v1";
pub const ACTION_ADJUDICATION_CONTEXT_LIMIT: usize = 8 * 1024;

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

impl GameRuleTemplate {
    pub fn validate(&self) -> Result<(), GameRulesError> {
        if self.novel_id.is_nil()
            || self.canon_model_version < 1
            || self.schema_version != GAME_RULE_SCHEMA_VERSION
            || self.prompt_version != GAME_RULE_PROMPT_VERSION
            || self.minimum_score != 8
            || self.maximum_score != 15
            || !(3..=6).contains(&self.attributes.len())
        {
            return invalid("template identity, versions, or bounds are invalid");
        }
        let mut keys = HashSet::new();
        for attribute in &self.attributes {
            key(&attribute.key)?;
            text(&attribute.label, 40)?;
            text(&attribute.description, 300)?;
            source_chapters(&attribute.source_chapters)?;
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
                {
                    return invalid("narrative mode cannot bind an advanced template");
                }
            }
            ResolutionMode::Advanced => {
                if self.canon_model_version.is_none_or(|version| version < 1)
                    || self.template_schema_version != Some(GAME_RULE_SCHEMA_VERSION)
                    || self.template_prompt_version.as_deref() != Some(GAME_RULE_PROMPT_VERSION)
                    || !(3..=6).contains(&self.attributes.len())
                    || self
                        .attributes
                        .iter()
                        .any(|(name, score)| key(name).is_err() || !(8..=15).contains(score))
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
            || self.template_prompt_version != GAME_RULE_PROMPT_VERSION
            || !(8..=15).contains(&self.score)
            || self.modifier != (self.score - 10).div_euclid(2)
            || !(1..=20).contains(&self.roll)
            || !(5..=30).contains(&self.difficulty_class)
            || self.total != self.roll + self.modifier
        {
            return invalid("action check arithmetic or versions are invalid");
        }
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
        text(&self.attribute_label, 40)
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
