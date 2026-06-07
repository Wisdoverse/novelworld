use serde::{Deserialize, Serialize};

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
pub struct ExtractedCharacter {
    pub name: String,
    pub aliases: Vec<String>,
    pub role: String,
    pub description: String,
    pub personality: String,
    pub background: String,
    pub speaking_style: String,
    pub appearance: String,
    pub first_appearance_chapter: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CharacterRelationship {
    pub from_character: String,
    pub to_character: String,
    pub relationship_type: String,
    pub description: String,
    pub strength: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub characters: Vec<ExtractedCharacter>,
    pub world_summary: String,
    pub genre: String,
    #[serde(default)]
    pub relationships: Vec<CharacterRelationship>,
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
      "strength": 50
    }}
  ],
  "world_summary": "世界观摘要（3-5句话，描述故事背景、时代、地点、核心冲突）",
  "genre": "小说类型（如：奇幻、科幻、言情、武侠等）"
}}

要求：
1. 至少提取5个角色（如有），主角必须包含
2. 外貌描述要详细，包含发型、眼睛、服装风格等，用于 AI 生成头像
3. 说话风格要具体，包含语气词、句式特点
4. world_summary 要包含时代背景和核心世界观设定
5. relationships 要覆盖主要角色之间的关系，strength 为 0-100 的关系密切度
6. 只返回 JSON，不要有其他文字"#,
        title = novel_title,
        text = safe_truncate(sample_text, 8000),
    )
}

pub fn build_chunk_extraction_prompt(novel_title: &str, chunk_text: &str, chunk_index: usize) -> String {
    format!(
        r#"你是一位专业的文学分析师。这是小说《{title}》的第{idx}段文本。请提取其中出现的角色和角色关系。

文本：
---
{text}
---

以 JSON 格式返回：
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
      "first_appearance_chapter": null
    }}
  ],
  "relationships": [
    {{
      "from_character": "角色A",
      "to_character": "角色B",
      "relationship_type": "关系类型",
      "description": "关系描述",
      "strength": 50
    }}
  ]
}}

只返回 JSON。"#,
        title = novel_title,
        idx = chunk_index + 1,
        text = safe_truncate(chunk_text, 6000),
    )
}
