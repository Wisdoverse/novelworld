use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::domain::entities::chapter::Chapter;

const SUMMARY_SAMPLE_BYTES: usize = 8_000;
const SCAN_CHUNK_BYTES: usize = 24_000;
const SCAN_OVERLAP_BYTES: usize = 256;
/// SPEC 5.4: the extractor returns at most 50 characters per novel to bound
/// provider cost.
const MAX_EXTRACTED_CHARACTERS: usize = 50;
/// SPEC 5.5: the stored world summary must not exceed 2000 characters.
const MAX_WORLD_SUMMARY_CHARS: usize = 2_000;

/// Truncate a string to at most `max_bytes` bytes without splitting a UTF-8
/// codepoint.  Always returns a valid `&str`.
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedCharacter {
    pub name: String,
    pub aliases: Vec<String>,
    pub role: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub personality: String,
    #[serde(default)]
    pub background: String,
    #[serde(default)]
    pub speaking_style: String,
    #[serde(default)]
    pub appearance: String,
    pub first_appearance_chapter: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterRelationship {
    pub from_character: String,
    pub to_character: String,
    pub relationship_type: String,
    pub description: String,
    pub strength: i32,
    /// Chapter where the relationship first becomes evident in the source
    /// (mirrors ExtractedCharacter::first_appearance_chapter; absent when
    /// the provider omitted it — the extraction-quality gate enforces the
    /// citation). Not persisted at import; gate-only provenance.
    pub first_appearance_chapter: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub characters: Vec<ExtractedCharacter>,
    pub world_summary: String,
    pub genre: String,
    #[serde(default)]
    pub relationships: Vec<CharacterRelationship>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkExtractionResult {
    #[serde(default)]
    pub characters: Vec<ExtractedCharacter>,
    #[serde(default)]
    pub relationships: Vec<CharacterRelationship>,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid character extraction: {0}")]
pub struct ExtractionValidationError(String);

/// OpenAI-compatible providers sometimes wrap an otherwise valid JSON object
/// in a Markdown fence when native JSON mode is unavailable. Keep transport
/// quirks out of the application handler while still requiring one JSON object.
pub fn json_object_payload(response: &str) -> &str {
    let trimmed = response.trim();
    let Some(start) = trimmed.find('{') else {
        return trimmed;
    };
    let Some(relative_end) = trimmed[start..].rfind('}') else {
        return trimmed;
    };
    &trimmed[start..=start + relative_end]
}

pub fn build_representative_sample(chapters: &[Chapter]) -> String {
    if chapters.is_empty() {
        return String::new();
    }

    let indexes = if !needs_chunk_scan(chapters) || chapters.len() <= 3 {
        (0..chapters.len()).collect::<Vec<_>>()
    } else {
        vec![0, chapters.len() / 2, chapters.len() - 1]
    };
    let per_chapter = SUMMARY_SAMPLE_BYTES / indexes.len();

    indexes
        .into_iter()
        .map(|index| {
            let chapter = &chapters[index];
            let header = format!("Chapter {}:\n", chapter.chapter_number);
            let content = safe_truncate(
                &chapter.content,
                per_chapter.saturating_sub(header.len() + 2),
            );
            format!("{header}{content}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn needs_chunk_scan(chapters: &[Chapter]) -> bool {
    chapters
        .iter()
        .map(|chapter| chapter.content.len() + 32)
        .sum::<usize>()
        > SUMMARY_SAMPLE_BYTES
}

pub fn build_scan_plan(chapters: &[Chapter]) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for chapter in chapters {
        let header = format!("Chapter {}:\n", chapter.chapter_number);
        let part_bytes = SCAN_CHUNK_BYTES.saturating_sub(header.len());
        for part in split_at_utf8_boundaries(&chapter.content, part_bytes) {
            if !current.is_empty()
                && current.len() + header.len() + part.len() + 2 > SCAN_CHUNK_BYTES
            {
                chunks.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(&header);
            current.push_str(part);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

fn split_at_utf8_boundaries(text: &str, max_bytes: usize) -> Vec<&str> {
    if text.is_empty() {
        return vec![text];
    }

    let mut parts = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        parts.push(&text[start..end]);
        if end == text.len() {
            break;
        }
        let overlap = SCAN_OVERLAP_BYTES.min(max_bytes / 4);
        start = end.saturating_sub(overlap);
        while !text.is_char_boundary(start) {
            start += 1;
        }
    }
    parts
}

pub fn build_extraction_prompt(novel_title: &str, sample_text: &str) -> String {
    format!(
        r#"你是一位专业的文学分析师。请分析以下小说《{title}》的文本，提取所有重要角色信息、世界观摘要，以及角色之间的关系图谱。

小说文本（节选）：
---
{text}
---

请以 JSON 格式返回，结构如下：
{{
  "characters": [
    {{
      "name": "角色全名",
      "aliases": ["别名1", "别名2"],
      "role": "protagonist|antagonist|supporting|minor",
      "description": "角色简介（2-3句话）",
      "personality": "性格特征（列举3-5个关键词并说明）",
      "background": "背景故事（2-4句话）",
      "speaking_style": "说话风格描述（语气、用词习惯、口头禅等）",
      "appearance": "外貌描述（用于生成头像，尽量详细）",
      "first_appearance_chapter": 1
    }}
  ],
  "relationships": [
    {{
      "from_character": "角色A的名字",
      "to_character": "角色B的名字",
      "relationship_type": "关系类型（如：师徒、恋人、敌对、朋友、亲属、同盟）",
      "description": "关系描述（1句话说明）",
      "strength": 50,
      "first_appearance_chapter": 1
    }}
  ],
  "world_summary": "世界观摘要（不超过2000字，覆盖：时代与地理社会背景、主要势力或团体、核心冲突、独特世界规则（如魔法体系、科技设定，如有））",
  "genre": "小说类型（如：奇幻、科幻、言情、武侠等）"
}}

要求：
1. 提取 5–12 个重要角色（原文不足 5 个则全部提取），主角必须包含
2. 外貌描述要详细，包含发型、眼睛、服装风格等，用于 AI 生成头像
3. 说话风格要具体，包含语气词、句式特点
4. world_summary 必须覆盖时代/地理/社会背景、主要势力或团体、核心冲突，以及独特世界规则（如魔法体系、科技设定，如有），总长不超过 2000 字
5. relationships 要覆盖主要角色之间的关系，strength 为 0-100 的关系密切度
6. 文本中的 `Chapter N` 是真实章节号，first_appearance_chapter 必须填写角色或关系在所给文本中首次明确出现的 N（关系不能早于其双方角色的首次出现章节）
7. aliases 只收原文明确使用的姓名或称谓，不要收“他/她/那人”等代词
8. 只返回 JSON，不要有其他文字"#,
        title = safe_truncate(novel_title, 500),
        text = safe_truncate(sample_text, 8000),
    )
}

pub fn build_chunk_extraction_prompt(
    novel_title: &str,
    chunk_text: &str,
    chunk_index: usize,
) -> String {
    format!(
        r#"你是一位专业的文学分析师。请从小说文本中提取角色和角色关系，并以 JSON 格式返回：
{{
  "characters": [
    {{
      "name": "角色全名",
      "aliases": [],
      "role": "protagonist|antagonist|supporting|minor",
      "description": "简短描述",
      "personality": "性格",
      "background": "背景",
      "speaking_style": "说话风格",
      "appearance": "外貌",
      "first_appearance_chapter": 12
    }}
  ],
  "relationships": [
    {{
      "from_character": "角色A",
      "to_character": "角色B",
      "relationship_type": "关系类型",
      "description": "关系描述",
      "strength": 50,
      "first_appearance_chapter": 1
    }}
  ]
}}

要求：
1. 文本中的 `Chapter N` 是真实章节号，first_appearance_chapter 填该角色或关系在本段首次明确出现的最小 N（关系不能早于其双方角色的首次出现章节）
2. aliases 只收原文明确使用的姓名或称谓，不要收“他/她/那人”等代词
3. role 只能是 protagonist、antagonist、supporting、minor
4. 最多返回本段最重要的 12 个角色，各描述字段保持在 1 句话内
5. 只返回 JSON，不要有其他文字。

小说：{title}
扫描段落：{idx}
文本：
---
{text}
---"#,
        title = safe_truncate(novel_title, 500),
        idx = chunk_index + 1,
        text = safe_truncate(chunk_text, SCAN_CHUNK_BYTES),
    )
}

pub fn validate_extraction(result: &ExtractionResult) -> Result<(), ExtractionValidationError> {
    validate_nonempty(
        "world_summary",
        &result.world_summary,
        Some(MAX_WORLD_SUMMARY_CHARS),
    )?;
    validate_identifier("genre", &result.genre, 100)?;
    validate_parts(&result.characters, &result.relationships)
}

pub fn validate_chunk_extraction(
    result: &ChunkExtractionResult,
) -> Result<(), ExtractionValidationError> {
    validate_parts(&result.characters, &result.relationships)
}

fn validate_parts(
    characters: &[ExtractedCharacter],
    relationships: &[CharacterRelationship],
) -> Result<(), ExtractionValidationError> {
    for character in characters {
        validate_name("character name", &character.name)?;
        for alias in &character.aliases {
            validate_name("character alias", alias)?;
        }
        if !matches!(
            character.role.trim(),
            "protagonist" | "antagonist" | "supporting" | "minor"
        ) {
            return invalid(format!(
                "{} has unsupported role {}",
                character.name, character.role
            ));
        }
        for (field, value) in [
            ("description", &character.description),
            ("personality", &character.personality),
            ("background", &character.background),
            ("speaking_style", &character.speaking_style),
            ("appearance", &character.appearance),
        ] {
            if value.chars().count() > 2_000 {
                return invalid(format!(
                    "{} {field} exceeds its maximum length",
                    character.name
                ));
            }
        }
        if character
            .first_appearance_chapter
            .is_some_and(|chapter| chapter < 1)
        {
            return invalid(format!(
                "{} has invalid first_appearance_chapter",
                character.name
            ));
        }
    }

    for relationship in relationships {
        validate_name("relationship source", &relationship.from_character)?;
        validate_name("relationship target", &relationship.to_character)?;
        validate_identifier("relationship_type", &relationship.relationship_type, 50)?;
        validate_nonempty("relationship description", &relationship.description, None)?;
        if !(0..=100).contains(&relationship.strength) {
            return invalid("relationship strength must be between 0 and 100");
        }
        if relationship
            .first_appearance_chapter
            .is_some_and(|chapter| chapter < 1)
        {
            return invalid(format!(
                "relationship {} has invalid first_appearance_chapter",
                relationship.from_character
            ));
        }
    }
    Ok(())
}

fn validate_name(field: &str, value: &str) -> Result<(), ExtractionValidationError> {
    validate_identifier(field, value, 200)
}

fn validate_identifier(
    field: &str,
    value: &str,
    max_chars: usize,
) -> Result<(), ExtractionValidationError> {
    validate_nonempty(field, value, Some(max_chars))?;
    if value.chars().any(char::is_control) {
        return invalid(format!("{field} must contain no control characters"));
    }
    Ok(())
}

fn validate_nonempty(
    field: &str,
    value: &str,
    max_chars: Option<usize>,
) -> Result<(), ExtractionValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return invalid(format!("{field} must be non-empty"));
    }
    if max_chars.is_some_and(|max| value.chars().count() > max) {
        return invalid(format!("{field} exceeds its maximum length"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ExtractionValidationError> {
    Err(ExtractionValidationError(message.into()))
}

pub fn merge_extractions(
    mut base: ExtractionResult,
    chunks: Vec<ChunkExtractionResult>,
) -> ExtractionResult {
    struct Candidate {
        character: ExtractedCharacter,
        mentions: usize,
    }

    let mut merged: Vec<Candidate> = Vec::new();
    let mut relationships = std::mem::take(&mut base.relationships);
    let batches = std::iter::once(std::mem::take(&mut base.characters)).chain(
        chunks.into_iter().map(|chunk| {
            relationships.extend(chunk.relationships);
            chunk.characters
        }),
    );

    for batch in batches {
        for mut character in batch {
            normalize_character(&mut character);
            if let Some(existing) = merged
                .iter_mut()
                .find(|candidate| same_character(&candidate.character, &character))
            {
                merge_character(&mut existing.character, character);
                existing.mentions += 1;
            } else {
                merged.push(Candidate {
                    character,
                    mentions: 1,
                });
            }
        }
    }

    merged.sort_by(|left, right| {
        right
            .mentions
            .cmp(&left.mentions)
            .then_with(|| role_rank(&right.character.role).cmp(&role_rank(&left.character.role)))
            .then_with(|| {
                left.character
                    .first_appearance_chapter
                    .cmp(&right.character.first_appearance_chapter)
            })
            .then_with(|| left.character.name.cmp(&right.character.name))
    });
    // SPEC 5.4 cost bound: keep the 50 most prominent characters. The sort
    // ranks by merge mentions (a proxy for cross-chapter presence), then
    // role, first appearance, and name, so the cap is deterministic. Canon
    // enrichment is bounded to these characters; a provider that names an
    // uncapped character in canon output fails the existing fail-closed
    // validation with a bounded retryable error.
    merged.truncate(MAX_EXTRACTED_CHARACTERS);
    base.characters = merged
        .into_iter()
        .map(|candidate| candidate.character)
        .collect();

    let mut seen = HashSet::new();
    base.relationships = relationships
        .into_iter()
        .filter_map(|mut relationship| {
            relationship.from_character =
                canonical_name(&relationship.from_character, &base.characters)?;
            relationship.to_character =
                canonical_name(&relationship.to_character, &base.characters)?;
            relationship.relationship_type = relationship.relationship_type.trim().to_owned();
            relationship.description = relationship.description.trim().to_owned();
            if relationship.from_character == relationship.to_character {
                return None;
            }
            let key = (
                relationship.from_character.to_lowercase(),
                relationship.to_character.to_lowercase(),
                relationship.relationship_type.to_lowercase(),
            );
            seen.insert(key).then_some(relationship)
        })
        .collect();
    base.world_summary = base.world_summary.trim().to_owned();
    base.genre = base.genre.trim().to_owned();
    base
}

fn normalize_character(character: &mut ExtractedCharacter) {
    character.name = character.name.trim().to_owned();
    character.role = character.role.trim().to_lowercase();
    character.description = character.description.trim().to_owned();
    character.personality = character.personality.trim().to_owned();
    character.background = character.background.trim().to_owned();
    character.speaking_style = character.speaking_style.trim().to_owned();
    character.appearance = character.appearance.trim().to_owned();

    let mut seen = HashSet::new();
    character.aliases = character
        .aliases
        .drain(..)
        .map(|alias| alias.trim().to_owned())
        .filter(|alias| {
            !alias.is_empty() && alias != &character.name && seen.insert(alias.to_lowercase())
        })
        .collect();
}

fn merge_character(existing: &mut ExtractedCharacter, incoming: ExtractedCharacter) {
    for alias in std::iter::once(incoming.name.clone()).chain(incoming.aliases) {
        if alias != existing.name
            && !existing
                .aliases
                .iter()
                .any(|current| current.eq_ignore_ascii_case(&alias))
        {
            existing.aliases.push(alias);
        }
    }
    if role_rank(&incoming.role) > role_rank(&existing.role) {
        existing.role = incoming.role;
    }
    fill_missing(&mut existing.description, incoming.description);
    fill_missing(&mut existing.personality, incoming.personality);
    fill_missing(&mut existing.background, incoming.background);
    fill_missing(&mut existing.speaking_style, incoming.speaking_style);
    fill_missing(&mut existing.appearance, incoming.appearance);
    existing.first_appearance_chapter = match (
        existing.first_appearance_chapter,
        incoming.first_appearance_chapter,
    ) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    };
}

fn fill_missing(existing: &mut String, incoming: String) {
    if existing.is_empty() && !incoming.is_empty() {
        *existing = incoming;
    }
}

fn role_rank(role: &str) -> u8 {
    match role {
        "protagonist" => 4,
        "antagonist" => 3,
        "supporting" => 2,
        _ => 1,
    }
}

fn same_character(left: &ExtractedCharacter, right: &ExtractedCharacter) -> bool {
    left.name.eq_ignore_ascii_case(&right.name)
        || left
            .aliases
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(&right.name))
        || right
            .aliases
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(&left.name))
}

fn canonical_name(name: &str, characters: &[ExtractedCharacter]) -> Option<String> {
    let key = name.trim().to_lowercase();
    if let Some(character) = characters
        .iter()
        .find(|character| character.name.to_lowercase() == key)
    {
        return Some(character.name.clone());
    }
    let mut matches = characters.iter().filter(|character| {
        character
            .aliases
            .iter()
            .any(|alias| alias.trim().to_lowercase() == key)
    });
    let character = matches.next()?;
    matches.next().is_none().then(|| character.name.clone())
}

pub fn find_first_appearance(
    character: &ExtractedCharacter,
    all_characters: &[ExtractedCharacter],
    chapters: &[Chapter],
) -> Option<i32> {
    let known_names = all_characters
        .iter()
        .flat_map(|candidate| std::iter::once(&candidate.name).chain(&candidate.aliases))
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .collect::<HashSet<_>>();
    first_chapter_containing(&character.name, &known_names, chapters).or_else(|| {
        character
            .aliases
            .iter()
            .filter(|alias| alias.trim().chars().count() >= 2)
            .filter_map(|alias| first_chapter_containing(alias, &known_names, chapters))
            .min()
    })
}

fn first_chapter_containing(
    name: &str,
    known_names: &HashSet<&str>,
    chapters: &[Chapter],
) -> Option<i32> {
    chapters
        .iter()
        .find(|chapter| text_contains_name(&chapter.content, name.trim(), known_names))
        .map(|chapter| chapter.chapter_number)
}

fn text_contains_name(text: &str, name: &str, known_names: &HashSet<&str>) -> bool {
    if name.is_empty() {
        return false;
    }
    if !name.is_ascii() {
        return text.match_indices(name).any(|(start, _)| {
            let end = start + name.len();
            !known_names
                .iter()
                .filter(|known| known.len() > name.len())
                .any(|known| {
                    text.match_indices(known).any(|(known_start, _)| {
                        known_start <= start && known_start + known.len() >= end
                    })
                })
        });
    }

    let text = text.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    text.match_indices(&name).any(|(start, _)| {
        let end = start + name.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        !before.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            && !after.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn character(name: &str, aliases: &[&str], role: &str) -> ExtractedCharacter {
        ExtractedCharacter {
            name: name.into(),
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
            role: role.into(),
            description: "description".into(),
            personality: "personality".into(),
            background: "background".into(),
            speaking_style: "speaking style".into(),
            appearance: "appearance".into(),
            first_appearance_chapter: Some(2),
        }
    }

    fn chapter(number: i32, content: &str) -> Chapter {
        Chapter::new(Uuid::new_v4(), number, None, content.into())
    }

    #[test]
    fn scan_plan_covers_every_chunk_and_keeps_the_endpoints() {
        let chapters = (1..=30)
            .map(|number| chapter(number, &format!("marker-{number} {}", "x".repeat(24_000))))
            .collect::<Vec<_>>();

        let plan = build_scan_plan(&chapters);

        assert!(plan.len() > 24);
        assert!(plan.first().unwrap().contains("Chapter 1:"));
        assert!(plan.last().unwrap().contains("Chapter 30:"));
        assert!(plan.iter().all(|chunk| chunk.len() <= SCAN_CHUNK_BYTES));
    }

    #[test]
    fn merge_uses_aliases_without_replacing_an_earlier_profile() {
        let mut brief = character("姑娘", &["沈知微"], "supporting");
        brief.description = "short".into();
        let mut detailed = character("沈知微", &["姑娘"], "protagonist");
        detailed.description = "a much richer description".into();
        let result = merge_extractions(
            ExtractionResult {
                characters: vec![brief],
                world_summary: "world".into(),
                genre: "fantasy".into(),
                relationships: vec![],
            },
            vec![ChunkExtractionResult {
                characters: vec![detailed],
                relationships: vec![],
            }],
        );

        assert_eq!(result.characters.len(), 1);
        assert_eq!(result.characters[0].role, "protagonist");
        assert_eq!(result.characters[0].description, "short");
    }

    #[test]
    fn shared_titles_do_not_merge_distinct_characters_or_resolve_ambiguously() {
        let he_jin = character("何进", &["大将军"], "supporting");
        let cao_shuang = character("曹爽", &["大将军"], "supporting");
        let result = merge_extractions(
            ExtractionResult {
                characters: vec![he_jin, cao_shuang],
                world_summary: "world".into(),
                genre: "history".into(),
                relationships: vec![CharacterRelationship {
                    from_character: "大将军".into(),
                    to_character: "何进".into(),
                    relationship_type: "同盟".into(),
                    description: "ambiguous title".into(),
                    strength: 50,
                    first_appearance_chapter: Some(1),
                }],
            },
            vec![],
        );

        assert_eq!(result.characters.len(), 2);
        assert!(result.relationships.is_empty());
    }

    #[test]
    fn merge_fills_a_missing_optional_profile_field() {
        let mut first = character("沈知微", &[], "protagonist");
        first.speaking_style.clear();
        let later = character("沈知微", &[], "protagonist");
        let result = merge_extractions(
            ExtractionResult {
                characters: vec![first],
                world_summary: "world".into(),
                genre: "fantasy".into(),
                relationships: vec![],
            },
            vec![ChunkExtractionResult {
                characters: vec![later],
                relationships: vec![],
            }],
        );

        assert_eq!(result.characters[0].speaking_style, "speaking style");
    }

    #[test]
    fn missing_optional_profile_fields_do_not_discard_a_scan() {
        let extraction: ChunkExtractionResult = serde_json::from_str(
            r#"{"characters":[{"name":"何进","aliases":[],"role":"supporting","first_appearance_chapter":1}],"relationships":[]}"#,
        )
        .unwrap();

        assert!(validate_chunk_extraction(&extraction).is_ok());
        assert!(extraction.characters[0].speaking_style.is_empty());
    }

    #[test]
    fn merge_does_not_drop_late_characters_to_meet_an_asset_budget() {
        let characters = (1..=35)
            .map(|number| character(&format!("Character {number}"), &[], "supporting"))
            .collect();
        let result = merge_extractions(
            ExtractionResult {
                characters,
                world_summary: "world".into(),
                genre: "fantasy".into(),
                relationships: vec![],
            },
            vec![],
        );

        assert_eq!(result.characters.len(), 35);
    }

    #[test]
    fn merge_caps_characters_at_the_spec_cost_bound() {
        let characters = (1..=55)
            .map(|number| character(&format!("Character {number}"), &[], "supporting"))
            .collect();
        let result = merge_extractions(
            ExtractionResult {
                characters,
                world_summary: "world".into(),
                genre: "fantasy".into(),
                relationships: vec![],
            },
            vec![],
        );

        assert_eq!(result.characters.len(), MAX_EXTRACTED_CHARACTERS);
    }

    #[test]
    fn merge_cap_keeps_the_most_prominent_characters() {
        let mut characters = (1..=51)
            .map(|number| character(&format!("Character {number}"), &[], "supporting"))
            .collect::<Vec<_>>();
        characters.push(character("Hero", &[], "protagonist"));
        let result = merge_extractions(
            ExtractionResult {
                characters,
                world_summary: "world".into(),
                genre: "fantasy".into(),
                relationships: vec![],
            },
            vec![],
        );

        assert_eq!(result.characters.len(), MAX_EXTRACTED_CHARACTERS);
        assert!(
            result.characters.iter().any(|kept| kept.name == "Hero"),
            "the protagonist must survive the prominence cap"
        );
    }

    #[test]
    fn validate_rejects_world_summaries_over_2000_characters() {
        let base = ExtractionResult {
            characters: vec![],
            world_summary: "界".repeat(MAX_WORLD_SUMMARY_CHARS),
            genre: "fantasy".into(),
            relationships: vec![],
        };
        assert!(validate_extraction(&base).is_ok());

        let oversized = ExtractionResult {
            characters: vec![],
            world_summary: "界".repeat(MAX_WORLD_SUMMARY_CHARS + 1),
            genre: "fantasy".into(),
            relationships: vec![],
        };
        assert!(validate_extraction(&oversized).is_err());
    }

    #[test]
    fn extraction_prompt_requests_all_world_summary_dimensions() {
        let prompt = build_extraction_prompt("北塔旧事", "第一章 文本。");
        for dimension in [
            "时代与地理社会背景",
            "主要势力或团体",
            "核心冲突",
            "独特世界规则",
            "2000",
        ] {
            assert!(
                prompt.contains(dimension),
                "extraction prompt must request {dimension}"
            );
        }
    }

    #[test]
    fn first_appearance_is_proven_from_source_text() {
        let alice = character("Alice", &["the woman"], "protagonist");
        let chapters = vec![
            chapter(1, "The woman waited outside."),
            chapter(2, "Alice entered the room."),
        ];

        assert_eq!(
            find_first_appearance(&alice, std::slice::from_ref(&alice), &chapters),
            Some(2)
        );
    }

    #[test]
    fn first_appearance_ignores_names_embedded_in_a_longer_alias() {
        let zhang_yi = character("张翼", &[], "supporting");
        let zhang_fei = character("张飞", &["张翼德"], "protagonist");
        let characters = vec![zhang_yi.clone(), zhang_fei];
        let chapters = vec![
            chapter(2, "张翼德怒鞭督邮。"),
            chapter(100, "张翼领兵出战。"),
        ];

        assert_eq!(
            find_first_appearance(&zhang_yi, &characters, &chapters),
            Some(100)
        );
    }

    #[test]
    fn validate_rejects_relationship_first_appearance_below_one() {
        let base = ExtractionResult {
            characters: vec![character("Alice", &[], "protagonist")],
            world_summary: "world".into(),
            genre: "fantasy".into(),
            relationships: vec![CharacterRelationship {
                from_character: "Alice".into(),
                to_character: "Bob".into(),
                relationship_type: "师徒".into(),
                description: "旧日师徒".into(),
                strength: 50,
                first_appearance_chapter: None,
            }],
        };
        assert!(
            validate_extraction(&base).is_ok(),
            "absent citation is allowed"
        );

        let invalid = ExtractionResult {
            relationships: vec![CharacterRelationship {
                from_character: "Alice".into(),
                to_character: "Bob".into(),
                relationship_type: "师徒".into(),
                description: "旧日师徒".into(),
                strength: 50,
                first_appearance_chapter: Some(0),
            }],
            ..base.clone()
        };
        assert!(validate_extraction(&invalid).is_err());
    }

    #[test]
    fn scan_overlap_does_not_split_a_character_name_out_of_every_chunk() {
        let parts = split_at_utf8_boundaries("123456789Alice", 10);

        assert!(parts.iter().any(|part| part.contains("Alice")));
    }

    #[test]
    fn validation_rejects_model_values_that_break_storage_contracts() {
        let mut invalid = character("Alice", &[], "hero");
        invalid.first_appearance_chapter = Some(0);
        let result = ChunkExtractionResult {
            characters: vec![invalid],
            relationships: vec![],
        };

        assert!(validate_chunk_extraction(&result).is_err());
    }

    #[test]
    fn json_payload_accepts_provider_markdown_fences() {
        assert_eq!(
            json_object_payload("```json\n{\"characters\": []}\n```"),
            "{\"characters\": []}"
        );
        assert_eq!(json_object_payload("not json"), "not json");
    }
}
