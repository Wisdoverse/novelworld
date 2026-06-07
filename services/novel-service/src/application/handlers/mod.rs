use std::sync::Arc;
use anyhow::Result;
use tracing::{info, error};
use uuid::Uuid;
use tokio::sync::Semaphore;

use crate::application::commands::ImportNovelCommand;
use crate::domain::entities::{novel::Novel, character::Character};
use crate::domain::ports::{LlmPort, ImagePort};
use crate::domain::repositories::{NovelRepository, ChapterRepository, CharacterRepository};
use crate::domain::services::{
    novel_parser::NovelParserService,
    character_extractor::{build_extraction_prompt, ExtractionResult},
};
use crate::domain::services::node_detector;

pub struct NovelCommandHandler {
    pub novel_repo: Arc<dyn NovelRepository>,
    pub chapter_repo: Arc<dyn ChapterRepository>,
    pub character_repo: Arc<dyn CharacterRepository>,
    pub llm: Arc<dyn LlmPort>,
    pub image_client: Arc<dyn ImagePort>,
}

impl NovelCommandHandler {
    /// 处理小说导入命令（异步解析流程）
    #[tracing::instrument(skip(self))]
    pub async fn handle_import(&self, cmd: ImportNovelCommand) -> Result<Uuid> {
        info!("Importing novel: {}", cmd.title);

        // 1. 创建 Novel 聚合根
        let mut novel = Novel::create(cmd.user_id, cmd.title.clone(), cmd.author.clone());
        if let Some(mode) = cmd.deviation_mode {
            novel.set_deviation_mode(mode);
        }
        self.novel_repo.save(&novel).await?;

        let novel_id = novel.id;

        // 2. 获取原始文本
        let raw_text = match cmd.raw_content {
            Some(text) => text,
            None => {
                // TODO: 从 S3 下载文件并解析 PDF/TXT
                return Err(anyhow::anyhow!("File upload parsing not yet implemented"));
            }
        };

        // 3. 异步执行解析（不阻塞响应）
        let novel_repo = self.novel_repo.clone();
        let chapter_repo = self.chapter_repo.clone();
        let character_repo = self.character_repo.clone();
        let llm = self.llm.clone();
        let image_client = self.image_client.clone();
        let title = cmd.title.clone();

        tokio::spawn(async move {
            if let Err(e) = Self::parse_novel_async(
                novel_id, &title, &raw_text,
                novel_repo, chapter_repo, character_repo, llm, image_client,
            ).await {
                error!("Novel parsing failed for {}: {}", novel_id, e);
            }
        });

        Ok(novel_id)
    }

    #[tracing::instrument(skip_all, fields(novel_id = %novel_id))]
    async fn parse_novel_async(
        novel_id: Uuid,
        title: &str,
        raw_text: &str,
        novel_repo: Arc<dyn NovelRepository>,
        chapter_repo: Arc<dyn ChapterRepository>,
        character_repo: Arc<dyn CharacterRepository>,
        llm: Arc<dyn LlmPort>,
        image_client: Arc<dyn ImagePort>,
    ) -> Result<()> {
        // 更新状态为解析中
        let mut novel = novel_repo.find_by_id(novel_id).await?
            .ok_or_else(|| anyhow::anyhow!("Novel not found"))?;
        novel.start_parsing();
        novel_repo.update(&novel).await?;

        // 拆分章节
        info!("Parsing chapters for novel {}", novel_id);
        let chapters = NovelParserService::parse_chapters(novel_id, raw_text)?;
        let total_chapters = chapters.len() as i32;
        chapter_repo.save_batch(&chapters).await?;

        // 提取角色和世界观（使用前3章作为样本）
        info!("Extracting characters for novel {}", novel_id);
        let sample_text: String = chapters.iter()
            .take(3)
            .map(|c| c.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        let prompt = build_extraction_prompt(title, &sample_text);
        let extraction_json = llm.chat_json(&prompt).await?;
        let extraction: ExtractionResult = serde_json::from_str(&extraction_json)?;

        // 保存角色
        let characters: Vec<Character> = extraction.characters.iter()
            .map(|ec| Character::from_extraction(novel_id, ec, &extraction.world_summary, title))
            .collect();

        character_repo.save_batch(&characters).await?;

        // Save character relationship graph
        if !extraction.relationships.is_empty() {
            let char_name_to_id: std::collections::HashMap<String, Uuid> = characters.iter()
                .map(|c| (c.name.to_lowercase(), c.id))
                .collect();

            for rel in &extraction.relationships {
                let from_id = char_name_to_id.get(&rel.from_character.to_lowercase());
                let to_id = char_name_to_id.get(&rel.to_character.to_lowercase());
                if let (Some(&from), Some(&to)) = (from_id, to_id) {
                    character_repo.save_relationship(
                        novel_id, from, to,
                        &rel.relationship_type,
                        Some(rel.description.as_str()),
                        rel.strength,
                    ).await.ok();
                }
            }
            info!("Saved {} character relationships for novel {}", extraction.relationships.len(), novel_id);
        }

        // Detect narrative branch nodes
        info!("Detecting narrative nodes for novel {}", novel_id);
        let chapter_summaries: Vec<(i32, &str)> = chapters.iter()
            .map(|c| (c.chapter_number, c.content.as_str()))
            .collect();
        let node_prompt = node_detector::build_node_detection_prompt(title, &chapter_summaries);
        if let Ok(node_json) = llm.chat_json(&node_prompt).await {
            if let Ok(detection) = serde_json::from_str::<node_detector::NodeDetectionResult>(&node_json) {
                for node in &detection.nodes {
                    if let Some(ch) = chapters.iter().find(|c| c.chapter_number == node.chapter_number) {
                        // Mark chapter as key node
                        let mut updated_ch = ch.clone();
                        updated_ch.mark_as_key_node(node.description.clone());
                        chapter_repo.update(&updated_ch).await.ok();
                    }
                }
                info!("Detected {} narrative nodes for novel {}", detection.nodes.len(), novel_id);
            }
        }

        // 标记小说为 ready
        novel.mark_ready(total_chapters, extraction.world_summary.clone());
        novel_repo.update(&novel).await?;

        // Concurrent avatar generation (max 3 parallel)
        let semaphore = Arc::new(Semaphore::new(3));
        for character in &characters {
            if let Some(appearance) = &character.appearance {
                let char_id = character.id;
                let appearance = appearance.clone();
                let char_repo = character_repo.clone();
                let img_client = image_client.clone();
                let sem = semaphore.clone();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    if let Err(e) = Self::generate_avatar(char_id, &appearance, char_repo, img_client).await {
                        error!("Avatar generation failed for {}: {}", char_id, e);
                    }
                });
            }
        }

        info!("Novel {} parsed successfully: {} chapters, {} characters",
            novel_id, total_chapters, characters.len());
        Ok(())
    }

    async fn generate_avatar(
        character_id: Uuid,
        appearance: &str,
        character_repo: Arc<dyn CharacterRepository>,
        image_client: Arc<dyn ImagePort>,
    ) -> Result<()> {
        let prompt = format!(
            "Portrait of a fictional character. {appearance}. \
            Anime/illustration style, high quality, detailed face, \
            dramatic lighting, cosmic background with stars.",
            appearance = appearance
        );
        let url = image_client.generate(&prompt).await?;
        let mut character = character_repo.find_by_id(character_id).await?
            .ok_or_else(|| anyhow::anyhow!("Character not found"))?;
        character.set_avatar(url);
        character_repo.update(&character).await?;
        Ok(())
    }
}
