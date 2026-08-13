use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::domain::entities::narrative_node::{NarrativeChoice, NarrativeNode, WorldState};
use crate::domain::ports::LlmPort;
use crate::domain::repositories::{
    ChapterInfo, ChapterReadRepository, ChoiceCommit, NarrativeNodeRepository, NovelInfo,
    PlayerChapter, PlayerChapterOrigin, PlayerChapterRepository, UserChoiceRecord,
    UserChoiceRepository, WorldStateRepository,
};
use crate::domain::services::narrative_engine::{
    build_branch_prompt, build_player_chapter_prompt, is_chinese_narrative, parse_generated_branch,
};
use crate::domain::services::narrative_transition::{
    build_transition_prompt, parse_transition, NarrativeTransition,
};

const MAX_NARRATIVE_PROMPT_BYTES: usize = 32 * 1024;
const MAX_NARRATIVE_PROMPT_CHARS: usize = 16_000;
const MAX_CONSEQUENCE_BYTES: usize = 32 * 1024;
const MAX_CONSEQUENCE_CHARS: usize = 8_000;
const MAX_TRANSITION_BYTES: usize = 128 * 1024;
const MAX_PLAYER_CHAPTER_BYTES: usize = 64 * 1024;
const MAX_PLAYER_CHAPTER_CHARS: usize = 20_000;

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
    pub chapter_number: i32,
    pub consequence: String,
    pub transition: NarrativeTransition,
    pub chapter_content: String,
    pub world_state: WorldState,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EffectiveChapter {
    pub chapter_number: i32,
    pub content: String,
    pub generated: bool,
}

struct ResolvedChapter {
    canonical: ChapterInfo,
    content: String,
    generated: bool,
}

pub struct NarrativeCommandHandler {
    pub node_repo: Arc<dyn NarrativeNodeRepository>,
    pub choice_repo: Arc<dyn UserChoiceRepository>,
    pub world_state_repo: Arc<dyn WorldStateRepository>,
    pub player_chapter_repo: Arc<dyn PlayerChapterRepository>,
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
        let novel_info = self.owned_novel(novel_id, user_id).await?;
        let chapter = self
            .resolve_chapter(user_id, novel_id, chapter_number, &novel_info)
            .await?;
        if let Some(existing_choice) = self
            .choice_repo
            .find_by_novel(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?
            .into_iter()
            .find(|choice| choice.chapter_number == chapter_number)
        {
            let existing_node = self
                .node_repo
                .find_by_id(existing_choice.node_id)
                .await
                .map_err(NarrativeError::Internal)?
                .filter(|node| {
                    node.novel_id == novel_id
                        && node.user_id.is_none_or(|owner_id| owner_id == user_id)
                })
                .ok_or(NarrativeError::NotFound)?;
            return Ok(Some(existing_node));
        }
        let node_owner = chapter.generated.then_some(user_id);
        if let Some(node) = self
            .node_repo
            .find_by_chapter(novel_id, chapter_number, node_owner)
            .await
            .map_err(NarrativeError::Internal)?
        {
            return Ok(Some(node));
        }
        if !chapter.canonical.is_key_node {
            return Ok(None);
        }
        let key_node_description = chapter
            .canonical
            .key_node_description
            .as_deref()
            .filter(|description| !description.trim().is_empty())
            .ok_or_else(|| {
                NarrativeError::Internal(anyhow::anyhow!(
                    "key chapter is missing its node description"
                ))
            })?;
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let prompt = build_branch_prompt(
            &novel_info.title,
            &chapter.content,
            key_node_description,
            &world_state,
            &novel_info.deviation_mode,
            "读者",
        );
        if prompt.len() > MAX_NARRATIVE_PROMPT_BYTES
            || prompt.chars().count() > MAX_NARRATIVE_PROMPT_CHARS
        {
            return Err(NarrativeError::Internal(anyhow::anyhow!(
                "branch prompt exceeded its budget"
            )));
        }
        let generated = self
            .llm
            .chat_json(&prompt)
            .await
            .map_err(NarrativeError::Llm)
            .and_then(|json| {
                parse_generated_branch(&json)
                    .map_err(|error| NarrativeError::Llm(anyhow::anyhow!(error)))
            })?;
        if !chapter.content.contains(&generated.anchor_quote) {
            return Err(NarrativeError::Llm(anyhow::anyhow!(
                "generated branch anchor was not present in chapter source"
            )));
        }
        let mut node = NarrativeNode::new(
            novel_id,
            chapter_number,
            generated.description,
            generated
                .choices
                .into_iter()
                .enumerate()
                .map(|(index, choice)| NarrativeChoice {
                    index: index as i32,
                    text: choice.text,
                    hint: choice.hint,
                    generated_consequence: None,
                })
                .collect(),
        )
        .with_anchor_quote(generated.anchor_quote);
        if let Some(user_id) = node_owner {
            node = node.for_user(user_id);
        }
        self.node_repo
            .save(&node)
            .await
            .map_err(NarrativeError::Internal)?;
        // Reload after the upsert so concurrent first readers receive the same
        // persisted node id.
        self.node_repo
            .find_by_chapter(novel_id, chapter_number, node_owner)
            .await
            .map_err(NarrativeError::Internal)
    }

    #[tracing::instrument(skip(self))]
    pub async fn get_effective_chapter(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
    ) -> NarrativeResult<EffectiveChapter> {
        if chapter_number < 1 {
            return Err(NarrativeError::Validation(
                "chapter must be at least 1".into(),
            ));
        }
        let novel_info = self.owned_novel(novel_id, user_id).await?;
        let chapter = self
            .resolve_chapter(user_id, novel_id, chapter_number, &novel_info)
            .await?;
        Ok(EffectiveChapter {
            chapter_number,
            content: chapter.content,
            generated: chapter.generated,
        })
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
            .filter(|node| {
                node.novel_id == novel_id && node.user_id.is_none_or(|owner_id| owner_id == user_id)
            })
            .ok_or(NarrativeError::NotFound)?;

        let existing = self
            .choice_repo
            .find_user_choice(user_id, node_id)
            .await
            .map_err(NarrativeError::Internal)?;
        if let Some(existing) = existing.as_ref() {
            warn!(
                user_id = %user_id,
                node_id = %node_id,
                choice_index = existing.choice_index,
                "replaying an existing narrative choice"
            );
            let full_content = match self
                .player_chapter_repo
                .find(user_id, novel_id, node.chapter_number)
                .await
                .map_err(NarrativeError::Internal)?
            {
                Some(chapter) => chapter.content,
                None => {
                    let chapter = self
                        .chapter_repo
                        .get_chapter(novel_id, node.chapter_number, user_id)
                        .await
                        .map_err(NarrativeError::Unavailable)?
                        .ok_or(NarrativeError::NotFound)?;
                    rewrite_after_anchor(
                        &chapter.content,
                        node.anchor_quote.as_deref(),
                        &existing.consequence,
                    )?
                }
            };
            return self
                .commit_result(choice_draft(existing, full_content))
                .await;
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
        let chapter = self
            .resolve_chapter(user_id, novel_id, node.chapter_number, &novel_info)
            .await?;
        let chapter_prefix =
            chapter_prefix_through_anchor(&chapter.content, node.anchor_quote.as_deref())?;
        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        let canon_context = self
            .chapter_repo
            .get_canon_context(novel_id, node.chapter_number, user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or_else(|| {
                NarrativeError::Internal(anyhow::anyhow!(
                    "ready novel has no canonical story context"
                ))
            })?;
        if canon_context.checkpoint_chapter != node.chapter_number {
            return Err(NarrativeError::Validation(
                "chapter is ahead of the reader's progress".into(),
            ));
        }
        let prompt = build_transition_prompt(
            &novel_info.title,
            &choice_text,
            chapter_prefix,
            &world_state,
            &novel_info.deviation_mode,
            &canon_context,
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
        let raw_transition = self
            .llm
            .chat_json(&prompt)
            .await
            .map_err(NarrativeError::Llm)?;
        if raw_transition.len() > MAX_TRANSITION_BYTES {
            return Err(NarrativeError::Llm(anyhow::anyhow!(
                "language model transition exceeded the limit"
            )));
        }
        let transition = parse_transition(&raw_transition, &canon_context)
            .map_err(|error| NarrativeError::Llm(anyhow::anyhow!(error)))?;
        let consequence = transition.rendered_narrative.clone();
        if consequence.is_empty()
            || consequence.len() > MAX_CONSEQUENCE_BYTES
            || consequence.chars().count() > MAX_CONSEQUENCE_CHARS
            || !is_chinese_narrative(&consequence)
        {
            return Err(NarrativeError::Llm(anyhow::anyhow!(
                "language model consequence was empty or exceeded the limit"
            )));
        }

        let rewritten_chapter_content = format!(
            "{}\n\n{}",
            chapter_prefix.trim_end(),
            consequence.trim_start()
        );
        self.commit_result(ChoiceCommit {
            user_id,
            novel_id,
            node_id,
            chapter_number: node.chapter_number,
            choice_index,
            choice_text,
            transition,
            rewritten_chapter_content,
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
            chapter_number: committed.choice.chapter_number,
            consequence: committed.choice.consequence.clone(),
            transition: committed.choice.transition,
            chapter_content: committed.player_chapter_content,
            world_state: committed.world_state,
        })
    }

    async fn resolve_chapter(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        novel_info: &NovelInfo,
    ) -> NarrativeResult<ResolvedChapter> {
        let canonical = self
            .chapter_repo
            .get_chapter(novel_id, chapter_number, user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)?;
        if let Some(player_chapter) = self
            .player_chapter_repo
            .find(user_id, novel_id, chapter_number)
            .await
            .map_err(NarrativeError::Internal)?
        {
            return Ok(ResolvedChapter {
                canonical,
                content: player_chapter.content,
                generated: true,
            });
        }

        let committed_choices = self
            .choice_repo
            .find_by_novel(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?
            .into_iter()
            .filter(|choice| {
                choice.chapter_number <= chapter_number && !choice.consequence.trim().is_empty()
            })
            .collect::<Vec<_>>();
        if committed_choices.is_empty() {
            return Ok(ResolvedChapter {
                content: canonical.content.clone(),
                canonical,
                generated: false,
            });
        }

        let mut previous = self
            .player_chapter_repo
            .find_latest_before(user_id, novel_id, chapter_number)
            .await
            .map_err(NarrativeError::Internal)?;
        if previous.is_none() {
            let first_choice = committed_choices
                .iter()
                .min_by_key(|choice| choice.chapter_number)
                .expect("committed choices are non-empty");
            previous = Some(self.reconstruct_choice_chapter(first_choice).await?);
        }

        let mut previous = previous.expect("divergence chapter was reconstructed");
        if previous.chapter_number == chapter_number {
            return Ok(ResolvedChapter {
                canonical,
                content: previous.content,
                generated: true,
            });
        }

        let world_state = self
            .world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .map_err(NarrativeError::Internal)?;
        for next_chapter_number in (previous.chapter_number + 1)..=chapter_number {
            if let Some(existing) = self
                .player_chapter_repo
                .find(user_id, novel_id, next_chapter_number)
                .await
                .map_err(NarrativeError::Internal)?
            {
                previous = existing;
                continue;
            }
            if let Some(choice) = committed_choices
                .iter()
                .find(|choice| choice.chapter_number == next_chapter_number)
            {
                previous = self.reconstruct_choice_chapter(choice).await?;
                continue;
            }
            let source = self
                .chapter_repo
                .get_chapter(novel_id, next_chapter_number, user_id)
                .await
                .map_err(NarrativeError::Unavailable)?
                .ok_or(NarrativeError::NotFound)?;
            previous = self
                .generate_player_chapter(
                    user_id,
                    novel_id,
                    next_chapter_number,
                    &previous.content,
                    &source.content,
                    novel_info,
                    &world_state,
                )
                .await?;
        }
        Ok(ResolvedChapter {
            canonical,
            content: previous.content,
            generated: true,
        })
    }

    async fn reconstruct_choice_chapter(
        &self,
        choice: &UserChoiceRecord,
    ) -> NarrativeResult<PlayerChapter> {
        let source = self
            .chapter_repo
            .get_chapter(choice.novel_id, choice.chapter_number, choice.user_id)
            .await
            .map_err(NarrativeError::Unavailable)?
            .ok_or(NarrativeError::NotFound)?;
        let node = self
            .node_repo
            .find_by_id(choice.node_id)
            .await
            .map_err(NarrativeError::Internal)?
            .filter(|node| {
                node.novel_id == choice.novel_id
                    && node
                        .user_id
                        .is_none_or(|owner_id| owner_id == choice.user_id)
            })
            .ok_or(NarrativeError::NotFound)?;
        let content = rewrite_after_anchor(
            &source.content,
            node.anchor_quote.as_deref(),
            &choice.consequence,
        )?;
        self.player_chapter_repo
            .save_if_absent(&PlayerChapter {
                user_id: choice.user_id,
                novel_id: choice.novel_id,
                chapter_number: choice.chapter_number,
                content,
                origin: PlayerChapterOrigin::Choice,
                created_at: choice.created_at,
            })
            .await
            .map_err(NarrativeError::Internal)
    }

    #[allow(clippy::too_many_arguments)]
    async fn generate_player_chapter(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        previous_content: &str,
        canonical_content: &str,
        novel_info: &NovelInfo,
        world_state: &WorldState,
    ) -> NarrativeResult<PlayerChapter> {
        let prompt = build_player_chapter_prompt(
            &novel_info.title,
            chapter_number,
            previous_content,
            canonical_content,
            novel_info.world_summary.as_deref(),
            world_state,
            &novel_info.deviation_mode,
        );
        if prompt.len() > MAX_NARRATIVE_PROMPT_BYTES
            || prompt.chars().count() > MAX_NARRATIVE_PROMPT_CHARS
        {
            return Err(NarrativeError::Internal(anyhow::anyhow!(
                "player chapter prompt exceeded its budget"
            )));
        }
        info!(
            user_id = %user_id,
            novel_id = %novel_id,
            chapter_number,
            "generating complete player timeline chapter"
        );
        let content = self
            .llm
            .chat_longform(
                "你是互动小说的玩家时间线主笔。只输出自然的简体中文小说正文。",
                &prompt,
            )
            .await
            .map_err(NarrativeError::Llm)?
            .trim()
            .to_owned();
        validate_player_chapter(&content)?;
        self.player_chapter_repo
            .save_if_absent(&PlayerChapter {
                user_id,
                novel_id,
                chapter_number,
                content,
                origin: PlayerChapterOrigin::Continuation,
                created_at: Utc::now(),
            })
            .await
            .map_err(NarrativeError::Internal)
    }
}

fn choice_draft(existing: &UserChoiceRecord, rewritten_chapter_content: String) -> ChoiceCommit {
    ChoiceCommit {
        user_id: existing.user_id,
        novel_id: existing.novel_id,
        node_id: existing.node_id,
        chapter_number: existing.chapter_number,
        choice_index: existing.choice_index,
        choice_text: existing.choice_text.clone(),
        transition: existing.transition.clone(),
        rewritten_chapter_content,
    }
}

fn chapter_prefix_through_anchor<'a>(
    content: &'a str,
    anchor_quote: Option<&str>,
) -> NarrativeResult<&'a str> {
    let anchor = anchor_quote
        .filter(|value| !value.is_empty())
        .ok_or_else(|| NarrativeError::Validation("narrative node has no source anchor".into()))?;
    let start = content.find(anchor).ok_or_else(|| {
        NarrativeError::Validation("narrative anchor is not present in this timeline".into())
    })?;
    Ok(&content[..start + anchor.len()])
}

fn rewrite_after_anchor(
    content: &str,
    anchor_quote: Option<&str>,
    consequence: &str,
) -> NarrativeResult<String> {
    let prefix = chapter_prefix_through_anchor(content, anchor_quote)?;
    Ok(format!(
        "{}\n\n{}",
        prefix.trim_end(),
        consequence.trim_start()
    ))
}

fn validate_player_chapter(content: &str) -> NarrativeResult<()> {
    if content.is_empty()
        || content.len() > MAX_PLAYER_CHAPTER_BYTES
        || content.chars().count() > MAX_PLAYER_CHAPTER_CHARS
        || !is_chinese_narrative(content)
    {
        return Err(NarrativeError::Llm(anyhow::anyhow!(
            "generated player chapter was empty, non-Chinese, or exceeded the limit"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod timeline_tests {
    use super::*;

    #[test]
    fn choice_replaces_everything_after_the_anchor() {
        let rewritten = rewrite_after_anchor(
            "原著开端。关键时刻。原著结局不再成立。",
            Some("关键时刻。"),
            "你介入以后，新的因果开始运转。",
        )
        .unwrap();

        assert_eq!(
            rewritten,
            "原著开端。关键时刻。\n\n你介入以后，新的因果开始运转。"
        );
        assert!(!rewritten.contains("原著结局不再成立"));
    }

    #[test]
    fn missing_anchor_fails_closed() {
        assert!(rewrite_after_anchor("原著正文", Some("不存在"), "新正文").is_err());
    }

    #[test]
    fn generated_player_chapter_must_be_bounded_chinese() {
        assert!(validate_player_chapter("你进入了新的时间线。").is_ok());
        assert!(validate_player_chapter("English only").is_err());
    }
}
