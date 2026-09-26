use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::game_rule_template::{GameRuleTemplate, BASIC_GAME_RULE_PROMPT_VERSION};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesRuleBinding {
    pub series_id: Uuid,
    pub revision: i32,
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
pub struct SeriesSetting {
    pub binding: SeriesRuleBinding,
    pub name: String,
    pub background: String,
}

impl SeriesSetting {
    pub fn validate(&self) -> Result<(), WorldSeriesError> {
        if self.binding.series_id.is_nil() || self.binding.revision != 1 {
            return Err(WorldSeriesError(
                "series identity or revision is invalid".into(),
            ));
        }
        validate_text(&self.name, 80)?;
        validate_text(&self.background, 2_000)
    }
}

impl SeriesRuleContext {
    pub fn validate(&self) -> Result<(), WorldSeriesError> {
        if self.binding.series_id.is_nil()
            || self.binding.revision != 1
            || self.target_novel_id.is_nil()
        {
            return Err(WorldSeriesError(
                "series identity or revision is invalid".into(),
            ));
        }
        validate_text(&self.name, 80)?;
        validate_text(&self.background, 2_000)
    }
}

/// User-provided setting and an exact, immutable source-book basic ruleset.
/// Source chapters always remain chapters of source_template.novel_id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSeries {
    pub id: Uuid,
    pub name: String,
    pub background: String,
    pub revision: i32,
    pub source_template: GameRuleTemplate,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid world series: {0}")]
pub struct WorldSeriesError(pub String);

impl WorldSeries {
    pub fn setting(&self) -> SeriesSetting {
        SeriesSetting {
            binding: SeriesRuleBinding {
                series_id: self.id,
                revision: self.revision,
            },
            name: self.name.clone(),
            background: self.background.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), WorldSeriesError> {
        if self.id.is_nil() || self.revision != 1 {
            return Err(WorldSeriesError(
                "series identity or revision is invalid".into(),
            ));
        }
        validate_text(&self.name, 80)?;
        validate_text(&self.background, 2_000)?;
        if self.source_template.prompt_version != BASIC_GAME_RULE_PROMPT_VERSION
            || self.source_template.series.is_some()
        {
            return Err(WorldSeriesError(
                "series requires an original basic template".into(),
            ));
        }
        self.source_template
            .validate(i32::MAX)
            .map_err(|_| WorldSeriesError("source template is invalid".into()))
    }

    pub fn rules_for(&self, target_novel_id: Uuid) -> Result<GameRuleTemplate, WorldSeriesError> {
        self.validate()?;
        let mut template = self.source_template.clone();
        template.prompt_version = super::game_rule_template::SERIES_GAME_RULE_PROMPT_VERSION.into();
        template.series = Some(SeriesRuleContext {
            binding: SeriesRuleBinding {
                series_id: self.id,
                revision: self.revision,
            },
            target_novel_id,
            name: self.name.clone(),
            background: self.background.clone(),
        });
        template
            .validate(i32::MAX)
            .map_err(|_| WorldSeriesError("series rule context is invalid".into()))?;
        Ok(template)
    }
}

pub fn validate_text(value: &str, maximum: usize) -> Result<(), WorldSeriesError> {
    if value.is_empty()
        || value.trim() != value
        || value.chars().count() > maximum
        || value.chars().any(|ch| ch.is_control() && ch != '\n')
    {
        return Err(WorldSeriesError(
            "series text is empty, untrimmed, or too long".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::game_rule_template::{
        basic_attribute, GameActionKind, GameActionRule, GameAttribute, BASIC_ACTION_DESCRIPTION,
        SERIES_GAME_RULE_PROMPT_VERSION,
    };

    fn series() -> WorldSeries {
        let attributes = ["root", "agility", "resolve"]
            .into_iter()
            .map(|key| {
                let (label, description) = basic_attribute(key).unwrap();
                GameAttribute {
                    key: key.into(),
                    label: label.into(),
                    description: description.into(),
                    default_score: 10,
                    source_chapters: vec![55],
                }
            })
            .collect();
        let action_rules = GameActionKind::ALL
            .into_iter()
            .map(|kind| GameActionRule {
                kind,
                attribute_key: "root".into(),
                difficulty_class: 12,
                description: BASIC_ACTION_DESCRIPTION.into(),
                source_chapters: vec![55],
            })
            .collect();
        WorldSeries {
            id: Uuid::new_v4(),
            name: "江湖系列".into(),
            background: "用户确认的共同设定".into(),
            revision: 1,
            source_template: GameRuleTemplate::new_basic(
                Uuid::new_v4(),
                7,
                attributes,
                action_rules,
            )
            .unwrap(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn shared_rules_keep_real_source_and_safe_text_at_different_book_progress() {
        let series = series();
        let source_bytes = serde_json::to_vec(&series.source_template).unwrap();
        let target = Uuid::new_v4();
        let shared = series.rules_for(target).unwrap();
        assert_eq!(shared.novel_id, series.source_template.novel_id);
        assert_eq!(shared.canon_model_version, 7);
        assert_eq!(shared.prompt_version, SERIES_GAME_RULE_PROMPT_VERSION);
        assert_eq!(shared.attributes, series.source_template.attributes);
        assert_eq!(shared.action_rules, series.source_template.action_rules);
        assert_eq!(shared.series.as_ref().unwrap().target_novel_id, target);
        assert!(shared.visible_at(0).is_none());
        assert!(shared.visible_at(1).is_some());
        assert_eq!(
            serde_json::to_vec(&series.source_template).unwrap(),
            source_bytes
        );
        assert!(!serde_json::to_value(&series.source_template)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("series"));
        let restored: GameRuleTemplate = serde_json::from_slice(&source_bytes).unwrap();
        assert_eq!(serde_json::to_vec(&restored).unwrap(), source_bytes);
    }

    #[test]
    fn forged_series_context_and_free_story_labels_are_rejected() {
        let series = series();
        let mut shared = series.rules_for(Uuid::new_v4()).unwrap();
        shared.attributes[0].label = "后文秘密".into();
        assert!(shared.validate(i32::MAX).is_err());
        let mut shared = series.rules_for(Uuid::new_v4()).unwrap();
        shared.series = None;
        assert!(shared.validate(i32::MAX).is_err());
        let mut shared = series.rules_for(Uuid::new_v4()).unwrap();
        shared.series.as_mut().unwrap().binding.revision = 2;
        assert!(shared.validate(i32::MAX).is_err());
        let mut shared = series.rules_for(Uuid::new_v4()).unwrap();
        shared.prompt_version = BASIC_GAME_RULE_PROMPT_VERSION.into();
        assert!(shared.validate(i32::MAX).is_err());
        assert!(series.rules_for(Uuid::nil()).is_err());
    }

    #[test]
    fn bounded_user_setting_does_not_accept_wrapped_source_or_invalid_text() {
        let mut definition = series();
        definition.background = "界".repeat(2_001);
        assert!(definition.validate().is_err());
        definition.background = "界".repeat(2_000);
        assert!(definition.validate().is_ok());
        definition.name = " name ".into();
        assert!(definition.validate().is_err());
        definition.name = "name".into();
        definition.source_template = series().rules_for(Uuid::new_v4()).unwrap();
        assert!(definition.validate().is_err());
    }

    #[test]
    fn series_setting_contains_only_confirmed_background_and_versioned_binding() {
        let series = series();
        let setting = series.setting();
        setting.validate().unwrap();
        let value = serde_json::to_value(&setting).unwrap();
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, ["background", "binding", "name"]);
        assert_eq!(setting.binding.series_id, series.id);
        let mut forged = setting;
        forged.binding.revision = 2;
        assert!(forged.validate().is_err());
        forged.binding.revision = 1;
        forged.background = "hidden\u{0}instruction".into();
        assert!(forged.validate().is_err());
    }
}
