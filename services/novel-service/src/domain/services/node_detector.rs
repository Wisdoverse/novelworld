use serde::{Deserialize, Serialize};

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

pub fn build_node_detection_prompt(novel_title: &str, chapters: &[(i32, &str)]) -> String {
    let summaries: String = chapters.iter()
        .map(|(num, content)| format!("Chapter {}: {}", num, &content[..content.len().min(500)]))
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        r#"You are a narrative analyst for the novel "{title}".

Analyze these chapter summaries and identify 2-5 key narrative moments where a reader could make a meaningful choice that would change the story direction.

Chapter summaries:
---
{summaries}
---

Return JSON:
{{
  "nodes": [
    {{
      "chapter_number": 3,
      "description": "A tense moment where the protagonist must decide...",
      "choices": [
        {{ "text": "Fight the dragon head-on", "hint": "Courage has its price..." }},
        {{ "text": "Negotiate with the dragon", "hint": "Words can be sharper than swords..." }},
        {{ "text": "Flee and regroup", "hint": "Sometimes retreat is the wisest strategy..." }}
      ]
    }}
  ]
}}

Requirements:
1. Each node must reference a real chapter number from the summaries
2. 2-3 choices per node, each with a mysterious hint
3. Choices should represent genuinely different paths (brave/cautious/creative)
4. Only pick truly pivotal story moments, not minor decisions"#,
        title = novel_title,
        summaries = summaries,
    )
}
