use anyhow::{Result, anyhow};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use uuid::Uuid;

use crate::domain::repositories::{CharacterInfo, CharacterInfoRepository};

/// HTTP adapter that fetches character info from novel-service,
/// replacing the previous direct-DB query against novel-service's tables.
pub struct NovelServiceClient {
    client: Client,
    base_url: String,
}

/// Intermediate deserialization type for the novel-service character response.
/// Handles both pre-built system_prompt and component fields used to build one.
#[derive(Debug, Deserialize)]
struct CharacterResponse {
    id: Uuid,
    name: String,
    novel_id: Uuid,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    personality: Option<String>,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    speaking_style: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Novel info needed to build a fallback system prompt.
#[derive(Debug, Deserialize)]
struct NovelResponse {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    world_summary: Option<String>,
}

impl NovelServiceClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
        }
    }

    /// Build a system prompt from character + novel fields when no pre-built prompt exists.
    fn build_system_prompt(
        ch: &CharacterResponse,
        novel_title: &str,
        world_summary: &str,
    ) -> String {
        let personality = ch.personality.as_deref().unwrap_or("Unknown");
        let background = ch.background.as_deref().unwrap_or("Unknown");
        let speaking_style = ch.speaking_style.as_deref().unwrap_or("Natural");
        let description = ch.description.as_deref().unwrap_or("");

        format!(
            r#"You are the character "{name}" from "{novel_title}".

## World Background
{world_summary}

## Your Character Info
- **Name**: {name}
- **Description**: {description}
- **Personality**: {personality}
- **Background**: {background}
- **Speaking Style**: {speaking_style}

## Behavioral Rules
1. Always respond in character as "{name}", maintaining consistency
2. Your speech patterns and word choices should match the character
3. You only know events you have experienced in the story (anti-spoiler rule)
4. Remember your conversation history with the reader for relationship continuity
5. Keep responses natural and immersive; do not break the fourth wall
6. If asked about information you should not know, deflect naturally in character"#,
            novel_title = novel_title,
            name = ch.name,
            world_summary = world_summary,
            description = description,
            personality = personality,
            background = background,
            speaking_style = speaking_style,
        )
    }
}

#[async_trait]
impl CharacterInfoRepository for NovelServiceClient {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<CharacterInfo>> {
        // Fetch character from novel-service API
        let url = format!("{}/characters/{}", self.base_url, id);
        let resp = self.client.get(&url).send().await.map_err(|e| {
            anyhow!("Failed to reach novel-service at {}: {}", url, e)
        })?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            return Err(anyhow!(
                "novel-service returned {} for character {}",
                resp.status(),
                id
            ));
        }

        let ch: CharacterResponse = resp.json().await?;

        // If there is no pre-built system_prompt, fetch novel info and build one
        let system_prompt = if let Some(ref prompt) = ch.system_prompt {
            Some(prompt.clone())
        } else {
            // Fetch novel metadata to build the prompt
            let novel_url = format!("{}/novels/{}", self.base_url, ch.novel_id);
            let novel_resp = self.client.get(&novel_url).send().await;

            let (novel_title, world_summary) = match novel_resp {
                Ok(r) if r.status().is_success() => {
                    let novel: NovelResponse = r.json().await.unwrap_or(NovelResponse {
                        title: None,
                        world_summary: None,
                    });
                    (
                        novel.title.unwrap_or_else(|| "Unknown".into()),
                        novel.world_summary.unwrap_or_default(),
                    )
                }
                _ => ("Unknown".into(), String::new()),
            };

            Some(Self::build_system_prompt(&ch, &novel_title, &world_summary))
        };

        Ok(Some(CharacterInfo {
            id: ch.id,
            name: ch.name,
            novel_id: ch.novel_id,
            system_prompt,
        }))
    }
}
