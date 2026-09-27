use crate::domain::{
    entities::canon_story_model::CanonStoryModel,
    ports::series_matcher::{SeriesBookMetadata, SeriesMatchCandidate},
};

pub const SERIES_MATCH_PROMPT_VERSION: &str = "series-match-v2";

pub fn chapter_one_entities(model: &CanonStoryModel) -> Vec<String> {
    let mut names = model
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
                && evidence
                    .provenance
                    .iter()
                    .all(|citation| citation.chapter_number == 1)
                && evidence
                    .provenance
                    .iter()
                    .any(|citation| citation.excerpt.contains(name.as_str()))
        })
        .map(|(name, _)| name.trim().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names.truncate(6);
    names
}

fn normalized(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|character| {
            !character.is_whitespace() && !matches!(character, '·' | '•' | '《' | '》')
        })
        .collect()
}

pub fn title_stem(title: &str) -> Option<String> {
    let title = title.trim().trim_matches(['《', '》']);
    let lower = title.to_lowercase();
    let end = ["与", " and ", "：", ":", "第", "卷", "部"]
        .iter()
        .filter_map(|separator| lower.find(separator))
        .min()?;
    let stem = normalized(&lower[..end]);
    (stem.chars().count() >= 3 && stem.chars().count() <= 80).then_some(stem)
}

pub fn same_family(left: &SeriesBookMetadata, right: &SeriesBookMetadata) -> bool {
    let (Some(left_stem), Some(right_stem)) = (title_stem(&left.title), title_stem(&right.title))
    else {
        return false;
    };
    if left_stem != right_stem {
        return false;
    }
    if left
        .author
        .as_deref()
        .zip(right.author.as_deref())
        .is_some_and(|(left, right)| {
            !left.trim().is_empty()
                && !right.trim().is_empty()
                && normalized(left) != normalized(right)
        })
    {
        return false;
    }
    let author_matches = left
        .author
        .as_deref()
        .zip(right.author.as_deref())
        .is_some_and(|(left, right)| {
            !left.trim().is_empty()
                && !right.trim().is_empty()
                && normalized(left) == normalized(right)
        });
    let mut shared = left
        .world_entities
        .iter()
        .filter(|name| right.world_entities.contains(name))
        .collect::<Vec<_>>();
    shared.sort();
    shared.dedup();
    author_matches || shared.len() >= 2
}

/// Families organize alternatives only. The matcher still decides or abstains.
pub fn group_candidates(candidates: Vec<SeriesMatchCandidate>) -> Vec<SeriesMatchCandidate> {
    // ponytail: at most 32 input books; all-member checks prevent evidence bridges.
    let mut groups: Vec<(SeriesMatchCandidate, Vec<SeriesBookMetadata>)> = Vec::new();
    for mut candidate in candidates {
        if candidate.series_id.is_none() {
            if let Some((group, members)) = groups.iter_mut().find(|(group, members)| {
                group.series_id.is_none()
                    && members
                        .iter()
                        .all(|member| same_family(member, &candidate.book))
            }) {
                members.push(candidate.book.clone());
                group.member_titles.push(candidate.book.title);
                group.member_titles.sort();
                group.member_titles.dedup();
                group
                    .book
                    .world_entities
                    .append(&mut candidate.book.world_entities);
                group.book.world_entities.sort();
                group.book.world_entities.dedup();
                group.book.world_entities.truncate(6);
                continue;
            }
        }
        if candidate.series_id.is_none() {
            if let Some(stem) = title_stem(&candidate.book.title) {
                candidate.name = stem;
            }
        }
        let member = candidate.book.clone();
        groups.push((candidate, vec![member]));
    }
    groups.into_iter().map(|(group, _)| group).collect()
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
    fn book(title: &str, author: Option<&str>, entities: &[&str]) -> SeriesBookMetadata {
        SeriesBookMetadata {
            title: title.into(),
            author: author.map(str::to_owned),
            genre: None,
            world_entities: entities.iter().map(|value| (*value).into()).collect(),
        }
    }
    #[test]
    fn missing_author_needs_two_shared_attested_entities_and_prefix_alone_is_not_enough() {
        let first = book("哈利·波特与魔法石", None, &["霍格沃茨", "伦敦"]);
        assert!(!same_family(
            &first,
            &book("哈利波特与密室", None, &["伦敦"])
        ));
        assert!(same_family(
            &first,
            &book("哈利波特与密室", None, &["伦敦", "霍格沃茨"])
        ));
        assert!(same_family(
            &book("哈利波特与魔法石", Some("J. K. Rowling"), &[]),
            &book("哈利波特与密室", Some("J. K. Rowling"), &[])
        ));
        assert!(!same_family(
            &book("哈利波特", Some("same"), &[]),
            &book("哈利波特", Some("same"), &[])
        ));
        assert!(!same_family(
            &first,
            &book("无关书与密室", None, &["伦敦", "霍格沃茨"])
        ));
        assert!(!same_family(
            &first,
            &book("哈利波特与密室", None, &["伦敦", "伦敦"])
        ));
        assert!(!same_family(
            &book("哈利波特与魔法石", Some("作者甲"), &["伦敦", "霍格沃茨"]),
            &book("哈利波特与密室", Some("作者乙"), &["伦敦", "霍格沃茨"])
        ));
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
        let public = serde_json::to_string(&book("Public title", None, &["hidden clue"])).unwrap();
        assert!(!public.contains("hidden clue"));
    }

    #[test]
    fn only_names_wholly_attested_in_first_chapter_reach_the_projection() {
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
    }

    #[test]
    fn seven_volumes_compete_as_one_advisory_family() {
        let candidates = [
            "魔法石",
            "密室",
            "阿兹卡班",
            "火焰杯",
            "凤凰社",
            "混血王子",
            "死亡圣器",
        ]
        .iter()
        .map(|volume| {
            let title = format!("哈利·波特与{volume}");
            SeriesMatchCandidate {
                series_id: None,
                source_novel_id: uuid::Uuid::new_v4(),
                name: title.clone(),
                book: book(&title, Some("罗琳"), &[]),
                member_titles: vec![title],
            }
        })
        .collect();
        let grouped = group_candidates(candidates);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].member_titles.len(), 7);
        assert!(grouped[0].series_id.is_none());
    }

    #[test]
    fn groups_cannot_hide_author_conflicts_or_bridge_aggregated_clues() {
        let candidate = |title: &str, author, names: &[&str]| SeriesMatchCandidate {
            series_id: None,
            source_novel_id: uuid::Uuid::new_v4(),
            name: title.into(),
            book: book(title, author, names),
            member_titles: vec![title.into()],
        };
        let authors = group_candidates(vec![
            candidate("共同系列：一", None, &["X", "Y"]),
            candidate("共同系列：二", Some("甲"), &["X", "Y"]),
            candidate("共同系列：三", Some("乙"), &["X", "Y"]),
        ]);
        assert_eq!(authors.len(), 2);
        let bridge = group_candidates(vec![
            candidate("共同系列：一", Some("甲"), &["X", "Y"]),
            candidate("共同系列：二", Some("甲"), &["Y", "Z"]),
            candidate("共同系列：三", None, &["X", "Z"]),
        ]);
        assert_eq!(bridge.len(), 2);
        let sparse = group_candidates(vec![
            candidate("共同系列：一", None, &["X", "Y"]),
            candidate("共同系列：二", None, &["Y", "Z"]),
            candidate("共同系列：三", None, &["X", "Z"]),
        ]);
        assert_eq!(sparse.len(), 3);
    }
}
