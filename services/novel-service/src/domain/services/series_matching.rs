use crate::domain::entities::canon_story_model::CanonStoryModel;
use std::collections::BTreeSet;

pub const SERIES_MATCH_PROMPT_VERSION: &str = "series-match-v3";
// ponytail: 256-name ceiling; use a bounded overlap scan if real shelves exceed it.
pub const MAX_PRIVATE_NAMES_PER_BOOK: usize = 256;

pub fn chapter_one_entities(model: &CanonStoryModel) -> Vec<String> {
    attested_names(model, true, 6).expect("chapter-one names are capped")
}

/// Private matching evidence. The application never puts these names in a
/// provider projection or response; some are first attested after chapter 1.
pub fn whole_book_entities(model: &CanonStoryModel) -> Option<Vec<String>> {
    attested_names(model, false, MAX_PRIVATE_NAMES_PER_BOOK)
}

fn attested_names(
    model: &CanonStoryModel,
    first_chapter_only: bool,
    limit: usize,
) -> Option<Vec<String>> {
    let names = model
        .content
        .locations
        .iter()
        .map(|item| (&item.name, &item.evidence))
        .chain(
            model
                .content
                .factions
                .iter()
                .map(|item| (&item.name, &item.evidence)),
        )
        .filter(|(name, evidence)| {
            !name.trim().is_empty()
                && name.chars().count() <= 40
                && !name.chars().any(char::is_control)
                && !evidence.provenance.is_empty()
                && (!first_chapter_only
                    || evidence
                        .provenance
                        .iter()
                        .all(|citation| citation.chapter_number == 1))
                && evidence
                    .provenance
                    .iter()
                    .any(|citation| citation.excerpt.contains(name.as_str()))
        })
        .map(|(name, _)| name.trim().to_owned());
    let mut unique = BTreeSet::new();
    for name in names {
        unique.insert(name);
        if unique.len() > limit {
            if !first_chapter_only {
                return None;
            }
            unique.pop_last();
        }
    }
    Some(unique.into_iter().collect())
}

pub fn shared_entity_count(left: &[String], right: &[String]) -> usize {
    left.iter()
        .filter(|name| right.binary_search(name).is_ok())
        .count()
}

pub fn confirmed_group_score(member_scores: &[usize], expected_members: usize) -> usize {
    if member_scores.len() == expected_members {
        member_scores.iter().copied().min().unwrap_or(0)
    } else {
        0
    }
}

/// A local hint needs two exact attested names and one uniquely strongest
/// candidate. Equal evidence is uncertainty, not a tie to break by filename.
pub fn unique_strongest(scores: &[usize]) -> Option<usize> {
    let (index, best) = scores.iter().enumerate().max_by_key(|(_, score)| *score)?;
    (*best >= 2 && scores.iter().filter(|score| *score == best).count() == 1).then_some(index)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelChoice {
    choice: Option<usize>,
    same_world: bool,
    basis: String,
}

pub fn parse_deepseek_choice(response: &str, candidates: usize) -> anyhow::Result<Option<usize>> {
    if response.len() > 4096 {
        anyhow::bail!("series response too large");
    }
    let answer: ModelChoice = serde_json::from_str(response)?;
    match (answer.choice, answer.same_world, answer.basis.as_str()) {
        (Some(index), true, "explicit_series" | "shared_world") if index < candidates => {
            Ok(Some(index))
        }
        (None, false, "insufficient") => Ok(None),
        _ => anyhow::bail!("invalid series decision"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_private_overlap_requires_a_unique_strongest_candidate() {
        let target = vec!["北塔".into(), "月亮会".into(), "雾城".into()];
        let unrelated = vec!["月亮会".into()];
        let related = vec!["北塔".into(), "月亮会".into()];
        let scores = [
            shared_entity_count(&target, &unrelated),
            shared_entity_count(&target, &related),
        ];
        assert_eq!(unique_strongest(&scores), Some(1));
        assert_eq!(unique_strongest(&[2, 2]), None);
        assert_eq!(unique_strongest(&[1, 0]), None);
        assert_eq!(confirmed_group_score(&[5, 1], 2), 1);
        assert_eq!(confirmed_group_score(&[5], 2), 0);
        assert_eq!(confirmed_group_score(&[5, 4], 2), 4);
    }
    #[test]
    fn strict_choice_never_accepts_freeform_rationale_or_out_of_range() {
        assert_eq!(
            parse_deepseek_choice(
                r#"{"choice":0,"same_world":true,"basis":"explicit_series"}"#,
                1
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            parse_deepseek_choice(
                r#"{"choice":null,"same_world":false,"basis":"insufficient"}"#,
                1
            )
            .unwrap(),
            None
        );
        for invalid in [
            r#"{"choice":1,"same_world":true,"basis":"shared_world"}"#,
            r#"{"choice":0,"same_world":false,"basis":"explicit_series"}"#,
            r#"{"choice":0,"same_world":true,"basis":"private prose"}"#,
        ] {
            assert!(parse_deepseek_choice(invalid, 1).is_err());
        }
        let public =
            serde_json::to_string(&crate::domain::ports::series_matcher::SeriesBookMetadata {
                title: "Public title".into(),
                author: None,
                genre: None,
                world_entities: vec!["hidden clue".into()],
            })
            .unwrap();
        assert!(!public.contains("hidden clue"));
    }

    #[test]
    fn whole_book_names_stay_private_and_require_the_name_in_a_source_excerpt() {
        use crate::domain::entities::canon_story_model::{
            CanonEndingSnapshot, CanonLocation, CanonStoryContent, SourceCitation, SourceEvidence,
        };
        let evidence = |chapters: &[i32], excerpt: &str| SourceEvidence {
            confidence: 1.0,
            provenance: chapters
                .iter()
                .map(|chapter| SourceCitation {
                    chapter_number: *chapter,
                    excerpt: excerpt.into(),
                })
                .collect(),
        };
        let mut model = CanonStoryModel {
            id: uuid::Uuid::new_v4(),
            novel_id: uuid::Uuid::new_v4(),
            model_version: 1,
            schema_version: 1,
            prompt_version: "test".into(),
            created_at: chrono::Utc::now(),
            content: CanonStoryContent {
                arcs: vec![],
                events: vec![],
                locations: vec![],
                factions: vec![],
                world_rules: vec![],
                character_goals: vec![],
                relationships: vec![],
                deaths: vec![],
                unresolved_threads: vec![],
                ending: CanonEndingSnapshot {
                    summary: "secret ending".into(),
                    character_states: Default::default(),
                    faction_states: Default::default(),
                    location_states: Default::default(),
                    unresolved_thread_ids: vec![],
                    evidence: evidence(&[9], "ending"),
                },
            },
        };
        for (name, chapters, excerpt) in [
            ("伦敦", vec![1], "第一章伦敦"),
            ("后文秘密", vec![9], "后文秘密"),
            ("混合秘密", vec![1, 9], "混合秘密"),
            ("猜测名称", vec![1], "不包含这个名字"),
        ] {
            model.content.locations.push(CanonLocation {
                id: name.into(),
                name: name.into(),
                description: "secret description".into(),
                evidence: evidence(&chapters, excerpt),
            });
        }
        assert_eq!(chapter_one_entities(&model), vec!["伦敦"]);
        assert_eq!(
            whole_book_entities(&model).unwrap(),
            vec!["伦敦", "后文秘密", "混合秘密"]
        );
        for index in 0..MAX_PRIVATE_NAMES_PER_BOOK {
            let name = format!("后续地点{index}");
            model.content.locations.push(CanonLocation {
                id: name.clone(),
                name: name.clone(),
                description: "source".into(),
                evidence: evidence(&[9], &name),
            });
            if index == 193 {
                assert_eq!(whole_book_entities(&model).unwrap().len(), 197);
            }
        }
        assert!(whole_book_entities(&model).is_none());
    }
}
