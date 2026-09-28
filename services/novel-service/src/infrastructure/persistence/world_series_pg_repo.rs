use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::domain::ports::series_matcher::{BeginSeriesMatch, SeriesMatchMethod, SeriesSuggestion};
use crate::domain::{
    entities::world_series::WorldSeries,
    repositories::{CreateWorldSeriesResult, WorldSeriesRepository},
};

pub struct PgWorldSeriesRepository {
    pool: PgPool,
}
impl PgWorldSeriesRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct SeriesRow {
    id: Uuid,
    name: String,
    background: String,
    revision: i32,
    source_novel_id: Uuid,
    source_template: Option<serde_json::Value>,
    created_at: DateTime<Utc>,
}
impl SeriesRow {
    fn decode(self) -> Result<WorldSeries> {
        let series = WorldSeries {
            id: self.id,
            name: self.name,
            background: self.background,
            revision: self.revision,
            source_novel_id: self.source_novel_id,
            source_template: self
                .source_template
                .map(serde_json::from_value)
                .transpose()
                .context("invalid persisted series source")?,
            created_at: self.created_at,
        };
        series.validate()?;
        Ok(series)
    }
}

#[async_trait]
impl WorldSeriesRepository for PgWorldSeriesRepository {
    async fn begin_match(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        method: SeriesMatchMethod,
        evidence_key: &str,
        check_only: bool,
    ) -> Result<BeginSeriesMatch> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            if !check_only {
                let token = Uuid::new_v4();
                let inserted = sqlx::query(
                r#"INSERT INTO series_match_decisions (user_id, novel_id, method, evidence_key, claim_token)
                   SELECT $1, $2, $3, $4, $5 FROM user_novels AS shelf JOIN novels AS n ON n.id = shelf.novel_id
                   WHERE shelf.user_id = $1 AND shelf.novel_id = $2
                     AND n.status = 'ready'::novel_status AND n.total_chapters > 0
                   ON CONFLICT (user_id, novel_id, method, evidence_key) DO NOTHING"#)
                .bind(user_id).bind(novel_id).bind(method.as_str()).bind(evidence_key).bind(token)
                .execute(&self.pool).await?;
                if inserted.rows_affected() == 1 { return Ok(BeginSeriesMatch::Acquired { token }); }
            }
            let row = sqlx::query_as::<_, (Option<serde_json::Value>, bool)>(
                r#"SELECT d.result, d.claimed_at < NOW() - INTERVAL '60 seconds'
                   FROM series_match_decisions AS d
                   JOIN user_novels AS shelf ON shelf.user_id = d.user_id AND shelf.novel_id = d.novel_id
                   JOIN novels AS n ON n.id = shelf.novel_id
                   WHERE d.user_id = $1 AND d.novel_id = $2 AND d.method = $3 AND d.evidence_key = $4
                     AND n.status = 'ready'::novel_status AND n.total_chapters > 0"#)
                .bind(user_id).bind(novel_id).bind(method.as_str()).bind(evidence_key).fetch_optional(&self.pool).await?;
            match row {
                Some((Some(result), _)) => Ok(BeginSeriesMatch::Cached(serde_json::from_value(result)?)),
                Some((None, false)) => Ok(BeginSeriesMatch::InProgress),
                Some((None, true)) | None => Ok(BeginSeriesMatch::UnknownOutcome),
            }
        }).await.context("series match claim deadline exceeded")?
    }

    async fn complete_match(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        method: SeriesMatchMethod,
        evidence_key: &str,
        token: Uuid,
        result: &SeriesSuggestion,
    ) -> Result<bool> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            Ok(sqlx::query(
                r#"UPDATE series_match_decisions AS d SET result = $6, completed_at = NOW()
                   WHERE user_id = $1 AND novel_id = $2 AND method = $3 AND evidence_key = $4
                     AND claim_token = $5 AND result IS NULL
                     AND EXISTS (SELECT 1 FROM user_novels AS shelf JOIN novels AS n ON n.id = shelf.novel_id
                       WHERE shelf.user_id = d.user_id AND shelf.novel_id = d.novel_id
                         AND n.status = 'ready'::novel_status AND n.total_chapters > 0)"#)
                .bind(user_id).bind(novel_id).bind(method.as_str()).bind(evidence_key).bind(token)
                .bind(serde_json::to_value(result)?).execute(&self.pool).await?.rows_affected() == 1)
        }).await.context("series match completion deadline exceeded")?
    }
    async fn create(&self, user_id: Uuid, series: &WorldSeries) -> Result<CreateWorldSeriesResult> {
        series.validate()?;
        // Lock the authorized source shelf, freeze exact rules and associate
        // the source atomically. Never claim generation or read source prose.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut transaction = self.pool.begin().await?;
            let source = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT shelf.novel_id FROM user_novels AS shelf JOIN novels AS n ON n.id = shelf.novel_id
                   WHERE shelf.user_id = $1 AND shelf.novel_id = $2
                     AND n.status = 'ready'::novel_status AND n.total_chapters > 0
                   FOR UPDATE OF shelf, n"#)
                .bind(user_id).bind(series.source_novel_id).fetch_optional(&mut *transaction).await?;
            if source.is_none() { return Ok(CreateWorldSeriesResult::SourceUnavailable); }
            let associated = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM user_novel_world_series WHERE user_id = $1 AND novel_id = $2)")
                .bind(user_id).bind(series.source_novel_id).fetch_one(&mut *transaction).await?;
            if associated { return Ok(CreateWorldSeriesResult::SourceAlreadyAssociated); }
            let inserted = sqlx::query(
                r#"INSERT INTO user_world_series
                       (id, user_id, name, background, revision, source_novel_id, source_template, created_at)
                   SELECT $1, $2, $3, $4, 1, $6, $8, $5
                   FROM user_novels AS shelf
                   JOIN novels AS n ON n.id = shelf.novel_id
                   WHERE shelf.user_id = $2 AND shelf.novel_id = $6
                     AND n.status = 'ready'::novel_status AND n.total_chapters > 0
                     AND EXISTS (SELECT 1 FROM canon_story_models AS m WHERE m.novel_id = n.id)
                     AND ($8::jsonb IS NULL OR $7 = (
                         SELECT MAX(m.model_version) FROM canon_story_models AS m WHERE m.novel_id = n.id
                     ))
                     AND ($8::jsonb IS NULL OR EXISTS (
                         SELECT 1 FROM novel_game_rule_templates AS t
                         WHERE t.novel_id = n.id AND t.canon_model_version = $7
                           AND t.prompt_version = 'novel-game-rules-v2'
                           AND t.status = 'ready' AND t.content = $8
                     ))"#)
                .bind(series.id).bind(user_id).bind(&series.name).bind(&series.background)
                .bind(series.created_at).bind(series.source_novel_id)
                .bind(series.source_template.as_ref().map(|template| template.canon_model_version).unwrap_or(0).max(1))
                .bind(series.source_template.as_ref().map(serde_json::to_value).transpose()?)
                .execute(&mut *transaction).await?;
            if inserted.rows_affected() != 1 { return Ok(CreateWorldSeriesResult::SourceUnavailable); }
            sqlx::query("INSERT INTO user_novel_world_series (user_id, novel_id, series_id) VALUES ($1, $2, $3)")
                .bind(user_id).bind(series.source_novel_id).bind(series.id).execute(&mut *transaction).await?;
            transaction.commit().await?;
            Ok(CreateWorldSeriesResult::Created)
        }).await.context("series creation deadline exceeded")?
    }

    async fn bind_ready_source(
        &self,
        user_id: Uuid,
        series_id: Uuid,
    ) -> Result<Option<WorldSeries>> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut transaction = self.pool.begin().await?;
            let row = sqlx::query_as::<_, SeriesRow>(
                "SELECT id, name, background, revision, source_novel_id, source_template, created_at \
                 FROM user_world_series WHERE user_id = $1 AND id = $2")
                .bind(user_id).bind(series_id).fetch_optional(&mut *transaction).await?;
            let Some(series) = row.map(SeriesRow::decode).transpose()? else {
                return Ok(None);
            };
            if series.source_template.is_some() {
                return Ok(Some(series));
            }
                let authorized = sqlx::query_scalar::<_, Uuid>(
                    r#"SELECT shelf.novel_id FROM user_novels AS shelf
                       JOIN novels AS n ON n.id = shelf.novel_id
                       JOIN user_novel_world_series AS member
                         ON member.user_id = shelf.user_id AND member.novel_id = shelf.novel_id
                       WHERE shelf.user_id = $1 AND shelf.novel_id = $2
                         AND member.series_id = $3
                         AND n.status = 'ready'::novel_status AND n.total_chapters > 0
                       FOR UPDATE OF shelf, n, member"#)
                    .bind(user_id).bind(series.source_novel_id).bind(series.id)
                    .fetch_optional(&mut *transaction).await?.is_some();
                if !authorized {
                    return Ok(Some(series));
                }
                let row = sqlx::query_as::<_, SeriesRow>(
                    "SELECT id, name, background, revision, source_novel_id, source_template, created_at \
                     FROM user_world_series WHERE user_id = $1 AND id = $2 FOR UPDATE")
                    .bind(user_id).bind(series_id).fetch_optional(&mut *transaction).await?;
                let Some(mut series) = row.map(SeriesRow::decode).transpose()? else {
                    return Ok(None);
                };
                if series.source_template.is_none() {
                    sqlx::query(
                        r#"UPDATE user_world_series AS s SET source_template = t.content
                           FROM novel_game_rule_templates AS t
                           WHERE s.user_id = $1 AND s.id = $2 AND s.source_template IS NULL
                             AND t.novel_id = s.source_novel_id
                             AND t.prompt_version = 'novel-game-rules-v2' AND t.status = 'ready'
                             AND t.canon_model_version = (
                                 SELECT MAX(m.model_version) FROM canon_story_models AS m
                                 WHERE m.novel_id = s.source_novel_id)"#)
                        .bind(user_id).bind(series_id).execute(&mut *transaction).await?;
                    series = sqlx::query_as::<_, SeriesRow>(
                        "SELECT id, name, background, revision, source_novel_id, source_template, created_at \
                         FROM user_world_series WHERE user_id = $1 AND id = $2")
                        .bind(user_id).bind(series_id).fetch_one(&mut *transaction).await?
                        .decode()?;
                }
            transaction.commit().await?;
            Ok(Some(series))
        }).await.context("series rule bind deadline exceeded")?
    }

    async fn list(&self, user_id: Uuid) -> Result<Vec<WorldSeries>> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            sqlx::query_as::<_, SeriesRow>(
                "SELECT id, name, background, revision, source_novel_id, source_template, created_at FROM user_world_series \
                 WHERE user_id = $1 ORDER BY created_at, id")
                .bind(user_id).fetch_all(&self.pool).await?.into_iter().map(SeriesRow::decode).collect()
        }).await.context("series list deadline exceeded")?
    }

    async fn find(&self, user_id: Uuid, series_id: Uuid) -> Result<Option<WorldSeries>> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            sqlx::query_as::<_, SeriesRow>(
                "SELECT id, name, background, revision, source_novel_id, source_template, created_at FROM user_world_series \
                 WHERE user_id = $1 AND id = $2")
                .bind(user_id).bind(series_id).fetch_optional(&self.pool).await?.map(SeriesRow::decode).transpose()
        }).await.context("series lookup deadline exceeded")?
    }

    async fn memberships(&self, user_id: Uuid) -> Result<Vec<(Uuid, Uuid)>> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            sqlx::query_as::<_, (Uuid, Uuid)>(
                "SELECT novel_id, series_id FROM user_novel_world_series WHERE user_id = $1 ORDER BY novel_id",
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
        })
        .await
        .context("series membership list deadline exceeded")?
    }

    async fn find_for_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<Option<WorldSeries>> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            sqlx::query_as::<_, SeriesRow>(
                r#"SELECT s.id, s.name, s.background, s.revision, s.source_novel_id, s.source_template, s.created_at
                   FROM user_novel_world_series AS member
                   JOIN user_world_series AS s ON s.id = member.series_id AND s.user_id = member.user_id
                   JOIN user_novels AS shelf ON shelf.user_id = member.user_id AND shelf.novel_id = member.novel_id
                   JOIN novels AS n ON n.id = shelf.novel_id
                   WHERE member.user_id = $1 AND member.novel_id = $2
                     AND n.status = 'ready'::novel_status AND n.total_chapters > 0"#)
                .bind(user_id).bind(novel_id).fetch_optional(&self.pool).await?.map(SeriesRow::decode).transpose()
        }).await.context("novel series lookup deadline exceeded")?
    }

    async fn associate(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        series_id: Option<Uuid>,
    ) -> Result<bool> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut transaction = self.pool.begin().await?;
            // Serialize against shelf removal and readiness changes. Every
            // write is confined to this authenticated user's shelf selection.
            let authorized = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT shelf.novel_id FROM user_novels AS shelf JOIN novels AS n ON n.id = shelf.novel_id
                   WHERE shelf.user_id = $1 AND shelf.novel_id = $2
                     AND n.status = 'ready'::novel_status AND n.total_chapters > 0
                   FOR UPDATE OF shelf, n"#)
                .bind(user_id).bind(novel_id).fetch_optional(&mut *transaction).await?.is_some();
            if !authorized { return Ok(false); }
            if let Some(id) = series_id {
                let written = sqlx::query(
                    r#"INSERT INTO user_novel_world_series (user_id, novel_id, series_id)
                       SELECT $1, $2, s.id FROM user_world_series AS s WHERE s.user_id = $1 AND s.id = $3
                       ON CONFLICT (user_id, novel_id) DO UPDATE SET series_id = EXCLUDED.series_id"#)
                    .bind(user_id).bind(novel_id).bind(id).execute(&mut *transaction).await?;
                if written.rows_affected() != 1 { return Ok(false); }
            } else {
                sqlx::query("DELETE FROM user_novel_world_series WHERE user_id = $1 AND novel_id = $2")
                    .bind(user_id).bind(novel_id).execute(&mut *transaction).await?;
            }
            transaction.commit().await?;
            Ok(true)
        }).await.context("series association deadline exceeded")?
    }
}
