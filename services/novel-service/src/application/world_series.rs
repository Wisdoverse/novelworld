use crate::domain::{
    entities::{
        game_rule_template::{GameRuleTemplate, BASIC_GAME_RULE_PROMPT_VERSION},
        world_series::{validate_text, WorldSeries},
    },
    ports::series_matcher::{
        SeriesBookMetadata, SeriesMatchCandidate, SeriesMatcherPort, MAX_SERIES_MATCH_CANDIDATES,
    },
    repositories::{
        CanonStoryModelRepository, CreateWorldSeriesResult, NovelRepository, WorldSeriesRepository,
    },
    value_objects::NovelStatus,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWorldSeries {
    pub name: String,
    pub background: String,
    pub source_novel_id: Uuid,
    pub canon_model_version: Option<i32>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorldSeriesApplicationError {
    #[error("invalid series input")]
    InvalidInput,
    #[error("novel or series not found")]
    NotFound,
    #[error("ready source rules are unavailable")]
    SourceUnavailable,
    #[error("source novel is already associated with a series")]
    SourceAlreadyAssociated,
    #[error("novel is not ready")]
    NovelNotReady,
    #[error("series repository is unavailable")]
    Repository(#[source] anyhow::Error),
}

pub struct WorldSeriesHandler {
    pub series_repo: Arc<dyn WorldSeriesRepository>,
    pub novel_repo: Arc<dyn NovelRepository>,
    pub canon_repo: Arc<dyn CanonStoryModelRepository>,
    pub matcher: Option<Arc<dyn SeriesMatcherPort>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesSuggestionStatus {
    Suggested,
    Unconfigured,
    Uncertain,
    Unavailable,
}

#[derive(Debug, Serialize)]
pub struct SeriesSuggestion {
    pub status: SeriesSuggestionStatus,
    pub suggestion: Option<SeriesMatchCandidate>,
}

impl WorldSeriesHandler {
    pub async fn create(
        &self,
        user_id: Uuid,
        command: CreateWorldSeries,
    ) -> Result<WorldSeries, WorldSeriesApplicationError> {
        validate_text(&command.name, 80).map_err(|_| WorldSeriesApplicationError::InvalidInput)?;
        validate_text(&command.background, 2_000)
            .map_err(|_| WorldSeriesApplicationError::InvalidInput)?;
        if command.source_novel_id.is_nil() || command.canon_model_version.is_some_and(|v| v < 1) {
            return Err(WorldSeriesApplicationError::InvalidInput);
        }
        self.ready_novel(user_id, command.source_novel_id).await?;
        let model = bounded_read(self.canon_repo.find_latest(command.source_novel_id))
            .await?
            .ok_or(WorldSeriesApplicationError::SourceUnavailable)?;
        if command
            .canon_model_version
            .is_some_and(|v| v != model.model_version)
        {
            return Err(WorldSeriesApplicationError::SourceUnavailable);
        }
        let source_template = bounded_read(self.canon_repo.find_game_rule_template(
            command.source_novel_id,
            model.model_version,
            BASIC_GAME_RULE_PROMPT_VERSION,
        ))
        .await?
        .ok_or(WorldSeriesApplicationError::SourceUnavailable)?;
        if source_template.novel_id != command.source_novel_id
            || source_template.canon_model_version != model.model_version
        {
            return Err(WorldSeriesApplicationError::SourceUnavailable);
        }
        let series = WorldSeries {
            id: Uuid::new_v4(),
            name: command.name,
            background: command.background,
            revision: 1,
            source_template,
            created_at: Utc::now(),
        };
        series
            .validate()
            .map_err(|_| WorldSeriesApplicationError::SourceUnavailable)?;
        match self
            .series_repo
            .create(user_id, &series)
            .await
            .map_err(WorldSeriesApplicationError::Repository)?
        {
            CreateWorldSeriesResult::Created => {}
            CreateWorldSeriesResult::SourceUnavailable => {
                return Err(WorldSeriesApplicationError::SourceUnavailable)
            }
            CreateWorldSeriesResult::SourceAlreadyAssociated => {
                return Err(WorldSeriesApplicationError::SourceAlreadyAssociated)
            }
        }
        Ok(series)
    }

    pub async fn list(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<WorldSeries>, WorldSeriesApplicationError> {
        self.series_repo
            .list(user_id)
            .await
            .map_err(WorldSeriesApplicationError::Repository)
    }

    async fn ready_novel(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<crate::domain::entities::novel::Novel, WorldSeriesApplicationError> {
        let novel = bounded_read(self.novel_repo.find_for_user(user_id, novel_id))
            .await?
            .ok_or(WorldSeriesApplicationError::NotFound)?;
        if novel.status != NovelStatus::Ready || novel.total_chapters < 1 {
            return Err(WorldSeriesApplicationError::NovelNotReady);
        }
        Ok(novel)
    }

    pub async fn get_for_novel(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<Option<WorldSeries>, WorldSeriesApplicationError> {
        self.ready_novel(user_id, novel_id).await?;
        self.series_repo
            .find_for_novel(user_id, novel_id)
            .await
            .map_err(WorldSeriesApplicationError::Repository)
    }

    pub async fn associate(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        series_id: Option<Uuid>,
    ) -> Result<Option<WorldSeries>, WorldSeriesApplicationError> {
        if series_id.is_some_and(|id| id.is_nil()) {
            return Err(WorldSeriesApplicationError::InvalidInput);
        }
        self.ready_novel(user_id, novel_id).await?;
        if !self
            .series_repo
            .associate(user_id, novel_id, series_id)
            .await
            .map_err(WorldSeriesApplicationError::Repository)?
        {
            return Err(WorldSeriesApplicationError::NotFound);
        }
        // Return the committed selection, not a concurrent later selection.
        match series_id {
            Some(id) => self
                .series_repo
                .find(user_id, id)
                .await
                .map_err(WorldSeriesApplicationError::Repository),
            None => Ok(None),
        }
    }

    pub async fn frozen_rules(
        &self,
        user_id: Uuid,
        target_novel_id: Uuid,
        series_id: Uuid,
        revision: i32,
        source_canon: i32,
        require_current: bool,
    ) -> Result<GameRuleTemplate, WorldSeriesApplicationError> {
        self.ready_novel(user_id, target_novel_id).await?;
        if revision != 1 || source_canon < 1 || series_id.is_nil() {
            return Err(WorldSeriesApplicationError::InvalidInput);
        }
        let series = self
            .series_repo
            .find(user_id, series_id)
            .await
            .map_err(WorldSeriesApplicationError::Repository)?
            .ok_or(WorldSeriesApplicationError::NotFound)?;
        if series.revision != revision || series.source_template.canon_model_version != source_canon
        {
            return Err(WorldSeriesApplicationError::NotFound);
        }
        if require_current
            && self
                .series_repo
                .find_for_novel(user_id, target_novel_id)
                .await
                .map_err(WorldSeriesApplicationError::Repository)?
                .is_none_or(|current| current.id != series_id || current.revision != revision)
        {
            return Err(WorldSeriesApplicationError::NotFound);
        }
        series
            .rules_for(target_novel_id)
            .map_err(|_| WorldSeriesApplicationError::SourceUnavailable)
    }

    pub async fn suggest(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<SeriesSuggestion, WorldSeriesApplicationError> {
        let target = self.ready_novel(user_id, novel_id).await?;
        let Some(matcher) = &self.matcher else {
            return Ok(SeriesSuggestion {
                status: SeriesSuggestionStatus::Unconfigured,
                suggestion: None,
            });
        };
        let books = bounded_read(self.novel_repo.find_by_user(user_id)).await?;
        let books = books
            .into_iter()
            .filter(|book| {
                book.id != novel_id && book.status == NovelStatus::Ready && book.total_chapters > 0
            })
            .collect::<Vec<_>>();
        let series = self
            .series_repo
            .list(user_id)
            .await
            .map_err(WorldSeriesApplicationError::Repository)?;
        let memberships = self
            .series_repo
            .memberships(user_id)
            .await
            .map_err(WorldSeriesApplicationError::Repository)?;
        let mut candidates = Vec::new();
        for definition in series {
            if let Some(book) = books
                .iter()
                .filter(|book| {
                    memberships
                        .iter()
                        .any(|(novel, series)| *novel == book.id && *series == definition.id)
                })
                .min_by_key(|book| {
                    (
                        target.author.is_none() || target.author != book.author,
                        book.id != definition.source_template.novel_id,
                        book.title.clone(),
                        book.id,
                    )
                })
            {
                candidates.push(SeriesMatchCandidate {
                    series_id: Some(definition.id),
                    source_novel_id: book.id,
                    name: definition.name,
                    book: metadata(book),
                });
            }
        }
        for book in &books {
            if !memberships.iter().any(|(novel, _)| *novel == book.id) {
                candidates.push(SeriesMatchCandidate {
                    series_id: None,
                    source_novel_id: book.id,
                    name: book.title.clone(),
                    book: metadata(book),
                });
            }
        }
        // Deterministic bounded candidates; shared authors rank higher without
        // excluding collaborations, adaptations, or differently credited books.
        candidates.sort_by_key(|candidate| {
            (
                target.author.is_none() || target.author != candidate.book.author,
                candidate.series_id.is_none(),
                candidate.name.clone(),
                candidate.source_novel_id,
                candidate.series_id,
            )
        });
        candidates.truncate(MAX_SERIES_MATCH_CANDIDATES);
        if candidates.is_empty() {
            return Ok(SeriesSuggestion {
                status: SeriesSuggestionStatus::Uncertain,
                suggestion: None,
            });
        }
        let (status, candidate) = match matcher.suggest(&metadata(&target), &candidates).await {
            Ok(Some(index)) if index < candidates.len() => (
                SeriesSuggestionStatus::Suggested,
                Some(candidates.remove(index)),
            ),
            Ok(_) => (SeriesSuggestionStatus::Uncertain, None),
            Err(_) => (SeriesSuggestionStatus::Unavailable, None),
        };
        // Recommendations never establish association or authorize a later
        // write. Recheck the target after external I/O; confirmation validates
        // candidate ownership and readiness again through create/associate.
        self.ready_novel(user_id, novel_id).await?;
        if let Some(selected) = &candidate {
            // Do not publish a stale source-book recommendation after shelf
            // removal while the classifier was running.
            match self.ready_novel(user_id, selected.source_novel_id).await {
                Ok(_) => {}
                Err(
                    WorldSeriesApplicationError::NotFound
                    | WorldSeriesApplicationError::NovelNotReady,
                ) => {
                    return Ok(SeriesSuggestion {
                        status: SeriesSuggestionStatus::Uncertain,
                        suggestion: None,
                    })
                }
                Err(error) => return Err(error),
            }
        }
        Ok(SeriesSuggestion {
            status,
            suggestion: candidate,
        })
    }
}

// These existing read ports do not impose an adapter deadline. Bound only the
// new Series callers; cancellation cannot hide a side effect because they read.
async fn bounded_read<T>(
    read: impl Future<Output = anyhow::Result<T>>,
) -> Result<T, WorldSeriesApplicationError> {
    tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .map_err(|_| {
            WorldSeriesApplicationError::Repository(anyhow::anyhow!("series source read timed out"))
        })?
        .map_err(WorldSeriesApplicationError::Repository)
}

fn metadata(book: &crate::domain::entities::novel::Novel) -> SeriesBookMetadata {
    SeriesBookMetadata {
        title: book.title.clone(),
        author: book.author.clone(),
        genre: book.genre.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::CreateWorldSeries;

    #[test]
    fn source_version_is_optional_but_source_and_user_confirmation_are_required() {
        let input = serde_json::json!({"name":"系列", "background":"用户确认的设定", "source_novel_id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"});
        let command: CreateWorldSeries = serde_json::from_value(input.clone()).unwrap();
        assert!(command.canon_model_version.is_none());
        let mut forged = input;
        forged["source_template"] = serde_json::json!({"attributes":[]});
        assert!(serde_json::from_value::<CreateWorldSeries>(forged).is_err());
    }
}
