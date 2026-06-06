use async_trait::async_trait;
use anyhow::{Result, anyhow};
use reqwest::Client;
use uuid::Uuid;
use crate::domain::repositories::{ChapterReadRepository, NovelInfo};

pub struct NovelServiceClient {
    client: Client,
    base_url: String,
}

impl NovelServiceClient {
    pub fn new(base_url: String) -> Self {
        Self { client: Client::new(), base_url }
    }
}

#[derive(serde::Deserialize)]
struct ChapterResponse {
    content: String,
}

#[derive(serde::Deserialize)]
struct NovelResponse {
    id: Uuid,
    title: String,
    deviation_mode: String,
    world_summary: Option<String>,
}

#[async_trait]
impl ChapterReadRepository for NovelServiceClient {
    async fn get_chapter_content(&self, novel_id: Uuid, chapter_number: i32) -> Result<Option<String>> {
        let url = format!("{}/novels/{}/chapters/{}", self.base_url, novel_id, chapter_number);
        let resp = self.client.get(&url).send().await?;
        if resp.status().as_u16() == 404 { return Ok(None); }
        if !resp.status().is_success() {
            return Err(anyhow!("Novel service returned {}", resp.status()));
        }
        let ch: ChapterResponse = resp.json().await?;
        Ok(Some(ch.content))
    }

    async fn get_novel_info(&self, novel_id: Uuid) -> Result<Option<NovelInfo>> {
        let url = format!("{}/novels/{}", self.base_url, novel_id);
        let resp = self.client.get(&url).send().await?;
        if resp.status().as_u16() == 404 { return Ok(None); }
        if !resp.status().is_success() {
            return Err(anyhow!("Novel service returned {}", resp.status()));
        }
        let n: NovelResponse = resp.json().await?;
        Ok(Some(NovelInfo {
            id: n.id,
            title: n.title,
            deviation_mode: n.deviation_mode,
            world_summary: n.world_summary,
        }))
    }
}
