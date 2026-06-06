use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::domain::entities::narrative_node::{NarrativeNode, NarrativeChoice, WorldState};

/// LLM 返回的分支生成结果
#[derive(Debug, Serialize, Deserialize)]
pub struct GeneratedBranch {
    pub description: String,
    pub choices: Vec<ChoiceOption>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub text: String,
    pub hint: String,
}

/// 构建分支节点生成提示词
pub fn build_branch_prompt(
    novel_title: &str,
    chapter_content: &str,
    world_state: &WorldState,
    deviation_mode: &str,
    reader_identity: &str,
) -> String {
    let choices_history = serde_json::to_string_pretty(&world_state.state["choices"])
        .unwrap_or_default();
    let relationships = serde_json::to_string_pretty(&world_state.state["relationships"])
        .unwrap_or_default();

    format!(
        r#"你是《{title}》的叙事引擎。请根据当前章节内容，在关键时刻为读者生成分支选择。

## 章节内容（节选）
{chapter}

## 读者信息
- 读者身份：{identity}
- 故事偏离度：{mode}（canon=忠实原著, creative=创意扩展, remix=自由改写）

## 读者历史选择
{choices}

## 角色关系状态
{relationships}

## 任务
请识别章节中最关键的一个决策时刻，为读者生成3个不同的选择。

返回 JSON 格式：
{{
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
3. 考虑读者与各角色的关系分数
4. hint 要制造悬念，不直接说结果"#,
        title = novel_title,
        chapter = &chapter_content[..chapter_content.len().min(2000)],
        identity = reader_identity,
        mode = deviation_mode,
        choices = choices_history,
        relationships = relationships,
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
    format!(
        r#"你是《{title}》的叙事引擎。读者在关键时刻做出了选择，请生成选择后的故事发展。

## 当前章节背景
{chapter}

## 读者的选择
{choice}

## 故事偏离度：{mode}

## 当前世界状态
{state}

请生成300-500字的后续剧情，要求：
1. 自然衔接原著内容
2. 体现读者选择的影响
3. 保持角色性格一致
4. 根据偏离度决定与原著的差异程度
5. 结尾留有悬念，引导读者继续阅读"#,
        title = novel_title,
        chapter = &chapter_content[..chapter_content.len().min(1500)],
        choice = choice_text,
        mode = deviation_mode,
        state = serde_json::to_string_pretty(&world_state.state).unwrap_or_default(),
    )
}
