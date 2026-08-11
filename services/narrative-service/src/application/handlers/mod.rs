use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::domain::entities::narrative_node::{NarrativeNode, WorldState};
use crate::domain::ports::LlmPort;
use crate::domain::repositories::{
    ChapterReadRepository, ChoiceCommit, NarrativeNodeRepository, NovelInfo, UserChoiceRecord,
    UserChoiceRepository, WorldStateRepository,
};
use crate::domain::services::narrative_engine::build_consequence_prompt;

const MAX_NARRATIVE_PROMPT_BYTES: usize = 32 * 1024;
const MAX_NARRATIVE_PROMPT_CHARS: usize = 16_000;
const MAX_CONSEQUENCE_BYTES: usize = 32 * 1024;
const MAX_CONSEQUENCE_CHARS: usize = 8_000;

#[derive(Debug, thiserror::Error)]
pub enum NarrativeError {
    #[error("Resource not found")]
    NotFound,
    #[error("{0}")]
    Validation(String),
    #[error("Novel service is unavailable")]
    Unavailable(#[source] anyhow::Error),
    #[error("Consequence generation failed")]
    Llm(#[source] anyhow::Error),
    #[error("Narrative operation failed")]
    Internal(#[source] anyhow::Error),
}

pub type NarrativeResult<T> = std::result::Result<T, NarrativeError>;

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
    async fn owned_novel(&self, novel_id: Uuid, user_id: Uuid) -> NarrativeResult<NovelInfo> {
        self.chapter_repo
            .get_novel_info(novel_id, user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_branch_node(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
        user_id: Uuid,
    ) -> NarrativeResult<Option<NarrativeNode>> {
        if chapter_number < 1 {
            return Err(NarrativeError::Validation(
                "chapter must be at least 1".into(),
            ));
        }
        self.owned_novel(novel_id, user_id).await?;
        self.node_repo
            .find_by_chapter(novel_id, chapter_number)
            .await
            .map_err(NarrativeError::Internal)
    }

    #[tracing::instrument(skip(self))]
    pub async fn submit_choice(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        node_id: Uuid,
        requested_choice_index: i32,
    ) -> NarrativeResult<ChoiceResult> {
        let novel_info = self.owned_novel(novel_id, user_id).await?;
        let node = self
            .node_repo
            .find_by_id(node_id)
            .await
            .map_err(NarrativeError::Internal)?
            .filter(|node| node.novel_id == novel_id)
            .ok_or(NarrativeError::NotFound)?;

        let existing = self
            .choice_repo
            .find_user_choice(user_id, node_id)
            .await
            .map_err(NarrativeError::Internal)?;
        if let Some(existing) = existing.as_ref() {
            if let Some(consequence) = existing
                .consequence
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                warn!(
                    user_id = %user_id,
                    node_id = %node_id,
                    choice_index = existing.choice_index,
                    "repairing or replaying an existing narrative choice"
                );
                return self
                    .commit_result(choice_draft(existing, consequence.to_owned()))
                    .await;
            }
        }

        let (choice_index, choice_text) = match existing.as_ref() {
            Some(existing) => (existing.choice_index, existing.choice_text.clone()),
            None => {
                let choice = usize::try_from(requested_choice_index)
                    .ok()
                    .and_then(|index| node.choices.get(index))
                    .ok_or_else(|| {
                        NarrativeError::Validation(
                            "choice_index is outside the node choices".into(),
                        )
                    })?;
                (requested_choice_index, choice.text.clone())
            }
        };

        if choice_index < 0 {
            return Err(NarrativeError::Internal(anyhow::anyhow!(
                "persisted choice index is invalid"
            )));
        }
        let chapter_content = self
            .chapter_repo
            .get_chapter_content(novel_id, node.chapter_number, user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)?;
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let prompt = build_consequence_prompt(
            &novel_info.title,
            &choice_text,
            &chapter_content,
            &world_state,
            &novel_info.deviation_mode,
        );
        if prompt.len() > MAX_NARRATIVE_PROMPT_BYTES
            || prompt.chars().count() > MAX_NARRATIVE_PROMPT_CHARS
        {
            return Err(NarrativeError::Internal(anyhow::anyhow!(
                "narrative prompt exceeded its budget"
            )));
        }

        info!(
            user_id = %user_id,
            node_id = %node_id,
            choice_index,
            "generating narrative consequence"
        );
        let consequence = self
            .llm
            .chat(
                "You are a narrative engine that generates story consequences based on reader choices.",
                &prompt,
            )
            .await
            .map_err(NarrativeError::Llm)?;
        let consequence = consequence.trim().to_owned();
        if consequence.is_empty()
            || consequence.len() > MAX_CONSEQUENCE_BYTES
            || consequence.chars().count() > MAX_CONSEQUENCE_CHARS
        {
            return Err(NarrativeError::Llm(anyhow::anyhow!(
                "language model consequence was empty or exceeded the limit"
            )));
        }

        self.commit_result(ChoiceCommit {
            user_id,
            novel_id,
            node_id,
            chapter_number: node.chapter_number,
            choice_index,
            choice_text,
            consequence,
        })
        .await
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_world_state(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> NarrativeResult<WorldState> {
        self.owned_novel(novel_id, user_id).await?;
        self.world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)
    }

    async fn commit_result(&self, draft: ChoiceCommit) -> NarrativeResult<ChoiceResult> {
        let committed = self
            .choice_repo
            .commit_choice(&draft)
            .await
            .map_err(NarrativeError::Internal)?;
        Ok(ChoiceResult {
            consequence: committed.choice.consequence.unwrap_or_default(),
            world_state: committed.world_state,
        })
    }
}

fn choice_draft(existing: &UserChoiceRecord, consequence: String) -> ChoiceCommit {
    ChoiceCommit {
        user_id: existing.user_id,
        novel_id: existing.novel_id,
        node_id: existing.node_id,
        chapter_number: existing.chapter_number,
        choice_index: existing.choice_index,
        choice_text: existing.choice_text.clone(),
        consequence,
    }
}
