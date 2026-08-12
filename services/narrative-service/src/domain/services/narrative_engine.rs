use serde::{Deserialize, Serialize};

use crate::domain::entities::narrative_node::WorldState;

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

const MAX_TITLE_BYTES: usize = 2_000;
const MAX_CHAPTER_BYTES: usize = 4_000;
const MAX_CHOICE_BYTES: usize = 2_000;
const MAX_STATE_SECTION_BYTES: usize = 8_000;
const MAX_IDENTITY_BYTES: usize = 800;
const MAX_MODE_BYTES: usize = 32;

/// LLM 返回的分支生成结果
#[derive(Debug, Serialize, Deserialize)]
pub struct GeneratedBranch {
    pub anchor_quote: String,
    pub description: String,
    pub choices: Vec<ChoiceOption>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub text: String,
    pub hint: String,
}

fn contains_cjk(value: &str) -> bool {
    value.chars().any(
        |character| matches!(character as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF),
    )
}

pub fn parse_generated_branch(raw: &str) -> Result<GeneratedBranch, String> {
    let start = raw
        .find('{')
        .ok_or_else(|| "branch response did not contain a JSON object".to_string())?;
    let end = raw
        .rfind('}')
        .ok_or_else(|| "branch response did not contain a complete JSON object".to_string())?;
    let branch = serde_json::from_str::<GeneratedBranch>(&raw[start..=end])
        .map_err(|error| format!("branch response was invalid JSON: {error}"))?;
    validate_generated_branch(&branch)?;
    Ok(branch)
}

pub fn validate_generated_branch(branch: &GeneratedBranch) -> Result<(), String> {
    if !(10..=300).contains(&branch.anchor_quote.chars().count())
        || !contains_cjk(&branch.anchor_quote)
    {
        return Err("branch anchor must be a bounded Chinese source quote".into());
    }
    if branch.description.trim().is_empty()
        || branch.description.chars().count() > 1_000
        || !contains_cjk(&branch.description)
    {
        return Err("branch description must be bounded Chinese text".into());
    }
    if !(2..=3).contains(&branch.choices.len()) {
        return Err("branch must contain 2-3 choices".into());
    }
    for choice in &branch.choices {
        if choice.text.trim().is_empty()
            || choice.hint.trim().is_empty()
            || choice.text.chars().count() > 300
            || choice.hint.chars().count() > 300
            || !contains_cjk(&choice.text)
            || !contains_cjk(&choice.hint)
        {
            return Err("choice text and hint must be bounded Chinese text".into());
        }
    }
    Ok(())
}

pub fn is_chinese_narrative(value: &str) -> bool {
    contains_cjk(value)
}

/// 构建分支节点生成提示词
pub fn build_branch_prompt(
    novel_title: &str,
    chapter_content: &str,
    key_node_description: &str,
    world_state: &WorldState,
    deviation_mode: &str,
    reader_identity: &str,
) -> String {
    let choices_history =
        serde_json::to_string_pretty(&world_state.state["choices"]).unwrap_or_default();
    let relationships =
        serde_json::to_string_pretty(&world_state.state["relationships"]).unwrap_or_default();

    format!(
        r#"你是《{title}》的叙事引擎。用户以原著中不存在的新玩家角色进入小说世界。请根据当前章节内容，在关键时刻为玩家生成可亲自执行的行动。

## 章节内容（节选）
{chapter}

## 已识别的关键时刻
{key_node_description}

## 玩家信息
- 玩家身份：{identity}
- 故事偏离度：{mode}（canon=忠实原著, creative=创意扩展, remix=自由改写）

## 玩家历史行动
{choices}

## 角色关系状态
{relationships}

## 任务
请识别章节中最关键的介入时刻，为玩家生成3个不同的行动。

返回 JSON 格式：
{{
  "anchor_quote": "从章节内容中原样复制的一段连续文字，选项会紧接在它后面出现",
  "description": "当前情境描述（1-2句话，营造紧迫感）",
  "choices": [
    {{
      "text": "选项A的完整描述（15-30字）",
      "hint": "选择后的简短预告（不剧透，制造悬念，10字以内）"
    }},
    {{
      "text": "选项B的完整描述",
      "hint": "选择后的简短预告"
    }},
    {{
      "text": "选项C的完整描述",
      "hint": "选择后的简短预告"
    }}
  ]
}}

要求：
1. 选项要有明显差异（勇敢/谨慎/智慧，或不同情感倾向）
2. 根据故事偏离度决定选项的创意程度
3. 考虑玩家与各角色的关系分数
4. hint 要制造悬念，不直接说结果
5. 所有面向读者的文字必须使用自然的简体中文
6. 每个选项必须是玩家角色自己的行动，绝不能替原著角色作决定或控制其行为
7. 原著角色保有自己的目标、知识和自主性，只会对玩家行动作出反应
8. anchor_quote 必须从上面的章节内容中逐字、连续复制，长度 20-120 字，不得改写；它应在决策发生前结束
9. 只返回 JSON，不要添加 Markdown 代码围栏或解释"#,
        title = safe_truncate(novel_title, MAX_TITLE_BYTES),
        chapter = safe_truncate(chapter_content, MAX_CHAPTER_BYTES),
        key_node_description = safe_truncate(key_node_description, MAX_CHOICE_BYTES),
        identity = safe_truncate(reader_identity, MAX_IDENTITY_BYTES),
        mode = safe_truncate(deviation_mode, MAX_MODE_BYTES),
        choices = safe_truncate(&choices_history, MAX_STATE_SECTION_BYTES),
        relationships = safe_truncate(&relationships, MAX_STATE_SECTION_BYTES),
    )
}

/// 构建选择后果生成提示词
pub fn build_consequence_prompt(
    novel_title: &str,
    choice_text: &str,
    chapter_content: &str,
    world_state: &WorldState,
    deviation_mode: &str,
) -> String {
    let state = serde_json::to_string_pretty(&world_state.state).unwrap_or_default();
    format!(
        r#"你是《{title}》的叙事引擎。用户作为原著中不存在的新玩家角色，在关键时刻采取了行动。请生成行动后的故事发展。

## 当前章节背景
{chapter}

## 玩家的行动
{choice}

## 故事偏离度：{mode}

## 当前世界状态（包含玩家此前造成的变化）
{state}

请生成300-500字的后续剧情，要求：
1. 自然衔接原著内容
2. 体现玩家行动的影响
3. 保持角色性格一致
4. 根据偏离度决定与原著的差异程度
5. 结尾留有悬念，引导读者继续阅读
6. 全文必须使用自然的简体中文，并以第二人称描述玩家
7. 原著角色必须自主行动，不能表现得像被玩家直接控制"#,
        title = safe_truncate(novel_title, MAX_TITLE_BYTES),
        chapter = safe_truncate(chapter_content, MAX_CHAPTER_BYTES),
        choice = safe_truncate(choice_text, MAX_CHOICE_BYTES),
        mode = safe_truncate(deviation_mode, MAX_MODE_BYTES),
        state = safe_truncate(&state, MAX_STATE_SECTION_BYTES),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_branch_requires_bounded_chinese_reader_text() {
        let valid = r#"```json
        {"anchor_quote":"老人留下警告之后，化作一阵清风而去。","description":"盟誓之前，三人必须作出决定。","choices":[
          {"text":"上前见证三人的盟誓","hint":"你的出现将被众人记住……"},
          {"text":"暗中调查附近的异常","hint":"真相也许藏在桃园之外……"}
        ]}
        ```"#;
        assert!(parse_generated_branch(valid).is_ok());

        let english = r#"{"anchor_quote":"source quote","description":"Choose now","choices":[
          {"text":"Fight","hint":"Danger"},
          {"text":"Wait","hint":"Patience"}
        ]}"#;
        assert!(parse_generated_branch(english).is_err());
    }
}
