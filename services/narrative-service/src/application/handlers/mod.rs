use std::sync::Arc;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::domain::entities::narrative_node::{NarrativeNode, WorldState};
use crate::domain::ports::LlmPort;
use crate::domain::repositories::{
    NarrativeNodeRepository, UserChoiceRepository, WorldStateRepository, ChapterReadRepository,
};
use crate::domain::services::narrative_engine::build_consequence_prompt;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChoiceResult {
    pub consequence: String,
    pub world_state: WorldState,
}

pub struct NarrativeCommandHandler {
    pub node_repo: Arc<dyn NarrativeNodeRepository>,
    pub choice_repo: Arc<dyn UserChoiceRepository>,
    pub world_state_repo: Arc<dyn WorldStateRepository>,
    pub chapter_repo: Arc<dyn ChapterReadRepository>,
    pub llm: Arc<dyn LlmPort>,
}

impl NarrativeCommandHandler {
    /// Get the branch node for a given chapter (if it exists)
    #[tracing::instrument(skip(self))]
    pub async fn get_branch_node(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
        _user_id: Uuid,
    ) -> Result<Option<NarrativeNode>> {
        let node = self.node_repo
            .find_by_chapter(novel_id, chapter_number)
            .await?;
        Ok(node)
    }

    /// Submit a reader's choice for a narrative branch node
    #[tracing::instrument(skip(self))]
    pub async fn submit_choice(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        node_id: Uuid,
        choice_index: i32,
    ) -> Result<ChoiceResult> {
        // 1. Find the narrative node
        let node = self.node_repo
            .find_by_id(node_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Narrative node not found: {}", node_id))?;

        // 2. Validate choice_index bounds
        if choice_index < 0 || choice_index >= node.choices.len() as i32 {
            return Err(anyhow::anyhow!(
                "Invalid choice index {}. Must be 0..{}",
                choice_index,
                node.choices.len() - 1
            ));
        }

        // 3. Check if user already made a choice for this node
        if let Some(existing) = self.choice_repo
            .find_user_choice(user_id, node_id)
            .await?
        {
            warn!(
                "User {} already chose index {} for node {}",
                user_id, existing.choice_index, node_id
            );
            let ws = self.world_state_repo
                .get_or_create(user_id, novel_id)
                .await?;
            return Ok(ChoiceResult {
                consequence: existing.consequence.unwrap_or_default(),
                world_state: ws,
            });
        }

        let choice = &node.choices[choice_index as usize];
        let choice_text = choice.text.clone();

        // 4. Get chapter content and novel info
        let chapter_content = self.chapter_repo
            .get_chapter_content(novel_id, node.chapter_number)
            .await?
            .unwrap_or_default();

        let novel_info = self.chapter_repo
            .get_novel_info(novel_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Novel not found: {}", novel_id))?;

        // 5. Get current world state
        let mut world_state = self.world_state_repo
            .get_or_create(user_id, novel_id)
            .await?;

        // 6. Call LLM to generate consequence
        let prompt = build_consequence_prompt(
            &novel_info.title,
            &choice_text,
            &chapter_content,
            &world_state,
            &novel_info.deviation_mode,
        );

        info!(
            "Generating consequence for user {} choice {} on node {}",
            user_id, choice_index, node_id
        );

        let consequence = self.llm
            .chat(
                "You are a narrative engine that generates story consequences based on reader choices.",
                &prompt,
            )
            .await?;

        // 7. Save UserChoice
        self.choice_repo
            .save_choice(
                user_id,
                novel_id,
                node_id,
                node.chapter_number,
                choice_index,
                &choice_text,
                Some(&consequence),
            )
            .await?;

        // 8. Update WorldState
        world_state.record_choice(
            node.chapter_number,
            &choice_text,
            &consequence,
        );
        self.world_state_repo.update(&world_state).await?;

        info!(
            "Choice recorded: user={}, node={}, choice_index={}",
            user_id, node_id, choice_index
        );

        Ok(ChoiceResult {
            consequence,
            world_state,
        })
    }

    /// Get the current world state for a user + novel
    #[tracing::instrument(skip(self))]
    pub async fn get_world_state(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<WorldState> {
        self.world_state_repo
            .get_or_create(user_id, novel_id)
            .await
    }
}
