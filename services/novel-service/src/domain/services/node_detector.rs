use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct DetectedNode {
    pub chapter_number: i32,
    pub description: String,
    pub choices: Vec<DetectedChoice>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DetectedChoice {
    pub text: String,
    pub hint: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeDetectionResult {
    pub nodes: Vec<DetectedNode>,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid narrative node detection: {0}")]
pub struct NodeDetectionValidationError(String);

fn contains_cjk(value: &str) -> bool {
    value.chars().any(
        |character| matches!(character as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF),
    )
}

pub fn validate_detection(
    result: &NodeDetectionResult,
    chapter_numbers: impl IntoIterator<Item = i32>,
) -> Result<(), NodeDetectionValidationError> {
    let chapter_numbers = chapter_numbers.into_iter().collect::<HashSet<_>>();
    if !(2..=5).contains(&result.nodes.len()) {
        return Err(NodeDetectionValidationError("expected 2-5 nodes".into()));
    }
    for node in &result.nodes {
        if !chapter_numbers.contains(&node.chapter_number) {
            return Err(NodeDetectionValidationError(
                "node references an unknown chapter".into(),
            ));
        }
        if node.description.trim().is_empty()
            || node.description.chars().count() > 1_000
            || !contains_cjk(&node.description)
        {
            return Err(NodeDetectionValidationError(
                "description must be bounded Simplified Chinese text".into(),
            ));
        }
        if !(2..=3).contains(&node.choices.len()) {
            return Err(NodeDetectionValidationError(
                "each node must have 2-3 choices".into(),
            ));
        }
        for choice in &node.choices {
            if choice.text.trim().is_empty()
                || choice.hint.trim().is_empty()
                || choice.text.chars().count() > 300
                || choice.hint.chars().count() > 300
                || !contains_cjk(&choice.text)
                || !contains_cjk(&choice.hint)
            {
                return Err(NodeDetectionValidationError(
                    "choice text and hint must be bounded Simplified Chinese text".into(),
                ));
            }
        }
    }
    Ok(())
}

pub fn build_node_detection_prompt(novel_title: &str, chapters: &[(i32, &str)]) -> String {
    const MAX_CHAPTERS: usize = 40;
    let selected: Vec<_> = if chapters.len() <= MAX_CHAPTERS {
        chapters.iter().collect()
    } else {
        (0..MAX_CHAPTERS)
            .map(|index| &chapters[index * (chapters.len() - 1) / (MAX_CHAPTERS - 1)])
            .collect()
    };
    let summaries: String = selected
        .into_iter()
        .map(|(num, content)| format!("第 {num} 章：{}", safe_truncate(content, 500)))
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        r#"你是一位中文互动小说叙事分析师。用户会创建一个原著中不存在的新玩家角色，亲自进入小说《{title}》的世界。请分析章节摘要，找出 2-5 个玩家可以亲自介入、并影响故事走向的关键时刻。

章节摘要：
---
{summaries}
---

只返回以下结构的 JSON：
{{
  "nodes": [
    {{
      "chapter_number": 3,
      "description": "原著人物正在经历关键危机，刚进入这个世界的玩家也身处现场。",
      "choices": [
        {{ "text": "玩家挺身而出，帮助他们迎战", "hint": "你的出现将改变双方判断……" }},
        {{ "text": "玩家暗中调查，寻找危机根源", "hint": "真相可能藏在战场之外……" }},
        {{ "text": "玩家先去联络援军，再返回现场", "hint": "时间会让局势发生变化……" }}
      ]
    }}
  ]
}}

要求：
1. chapter_number 必须来自上面的真实章节号
2. 每个节点提供 2-3 个真正不同的选择，并附带富有悬念的提示
3. 只选择会改变故事方向的关键时刻，不选择无关紧要的小事
4. description、choices.text、choices.hint 必须全部使用自然的简体中文，即使原文不是中文
5. 每个 choice 必须是玩家角色亲自执行的行动，不能替原著角色作决定、说话或控制其行为
6. 原著角色拥有自己的目标与自主性；玩家只能参与、帮助、阻止、调查、交流或离开
7. 不要照抄示例内容，不要输出 JSON 以外的文字"#,
        title = safe_truncate(novel_title, 500),
        summaries = summaries,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_facing_node_text_must_be_chinese_and_reference_real_chapters() {
        let chinese_node = |chapter_number| DetectedNode {
            chapter_number,
            description: "主角必须作出决定。".into(),
            choices: vec![
                DetectedChoice {
                    text: "继续前进".into(),
                    hint: "前路未卜……".into(),
                },
                DetectedChoice {
                    text: "暂时离开".into(),
                    hint: "退让也有代价……".into(),
                },
            ],
        };
        let valid = NodeDetectionResult {
            nodes: vec![chinese_node(1), chinese_node(2)],
        };
        assert!(validate_detection(&valid, [1, 2]).is_ok());

        let english = NodeDetectionResult {
            nodes: (1..=2)
                .map(|chapter_number| DetectedNode {
                    chapter_number,
                    description: "Choose a path.".into(),
                    choices: vec![
                        DetectedChoice {
                            text: "Continue".into(),
                            hint: "Unknown".into(),
                        },
                        DetectedChoice {
                            text: "Leave".into(),
                            hint: "Danger".into(),
                        },
                    ],
                })
                .collect(),
        };
        assert!(validate_detection(&english, [1, 2]).is_err());
        assert!(validate_detection(&NodeDetectionResult { nodes: vec![] }, [1, 2]).is_err());
    }
}
