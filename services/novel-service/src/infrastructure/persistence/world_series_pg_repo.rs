use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

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
    source_template: serde_json::Value,
    created_at: DateTime<Utc>,
}
impl SeriesRow {
    fn decode(self) -> Result<WorldSeries> {
        let series = WorldSeries {
            id: self.id,
            name: self.name,
            background: self.background,
            revision: self.revision,
            source_template: serde_json::from_value(self.source_template)
                .context("invalid persisted series source")?,
            created_at: self.created_at,
        };
        series.validate()?;
        Ok(series)
    }
}

#[async_trait]
impl WorldSeriesRepository for PgWorldSeriesRepository {
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
                .bind(user_id).bind(series.source_template.novel_id).fetch_optional(&mut *transaction).await?;
            if source.is_none() { return Ok(CreateWorldSeriesResult::SourceUnavailable); }
            let associated = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM user_novel_world_series WHERE user_id = $1 AND novel_id = $2)")
                .bind(user_id).bind(series.source_template.novel_id).fetch_one(&mut *transaction).await?;
            if associated { return Ok(CreateWorldSeriesResult::SourceAlreadyAssociated); }
            let inserted = sqlx::query(
                r#"INSERT INTO user_world_series
                       (id, user_id, name, background, revision, source_template, created_at)
                   SELECT $1, $2, $3, $4, 1, t.content, $5
                   FROM user_novels AS shelf
                   JOIN novels AS n ON n.id = shelf.novel_id
                   JOIN novel_game_rule_templates AS t ON t.novel_id = n.id
                   WHERE shelf.user_id = $2 AND shelf.novel_id = $6
                     AND n.status = 'ready'::novel_status AND n.total_chapters > 0
                     AND t.canon_model_version = $7 AND t.prompt_version = 'novel-game-rules-v2'
                     AND t.status = 'ready' AND t.content = $8
                     AND t.canon_model_version = (
                         SELECT MAX(m.model_version) FROM canon_story_models AS m WHERE m.novel_id = n.id
                     )"#)
                .bind(series.id).bind(user_id).bind(&series.name).bind(&series.background)
                .bind(series.created_at).bind(series.source_template.novel_id)
                .bind(series.source_template.canon_model_version).bind(serde_json::to_value(&series.source_template)?)
                .execute(&mut *transaction).await?;
            if inserted.rows_affected() != 1 { return Ok(CreateWorldSeriesResult::SourceUnavailable); }
            sqlx::query("INSERT INTO user_novel_world_series (user_id, novel_id, series_id) VALUES ($1, $2, $3)")
                .bind(user_id).bind(series.source_template.novel_id).bind(series.id).execute(&mut *transaction).await?;
            transaction.commit().await?;
            Ok(CreateWorldSeriesResult::Created)
        }).await.context("series creation deadline exceeded")?
    }

    async fn list(&self, user_id: Uuid) -> Result<Vec<WorldSeries>> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            sqlx::query_as::<_, SeriesRow>(
                "SELECT id, name, background, revision, source_template, created_at FROM user_world_series \
                 WHERE user_id = $1 ORDER BY created_at, id")
                .bind(user_id).fetch_all(&self.pool).await?.into_iter().map(SeriesRow::decode).collect()
        }).await.context("series list deadline exceeded")?
    }

    async fn find(&self, user_id: Uuid, series_id: Uuid) -> Result<Option<WorldSeries>> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            sqlx::query_as::<_, SeriesRow>(
                "SELECT id, name, background, revision, source_template, created_at FROM user_world_series \
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
                r#"SELECT s.id, s.name, s.background, s.revision, s.source_template, s.created_at
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
