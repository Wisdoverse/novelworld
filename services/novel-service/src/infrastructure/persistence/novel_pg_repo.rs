use anyhow::{ensure, Result};
use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::entities::{
    chapter::{chapters_are_importable, Chapter},
    novel::Novel,
};
use crate::domain::repositories::{
    ImportClaim, NovelRepository, RecoverableImport, IMPORT_BUDGET_EXHAUSTED_MESSAGE,
    MAX_IMPORT_ATTEMPTS,
};
use crate::domain::value_objects::{DeviationMode, ImportStage, NovelStatus};
use crate::infrastructure::persistence::chapter_pg_repo::save_batch_in_transaction;
use crate::infrastructure::persistence::SOURCE_UPLOAD_PENDING;

pub struct NovelPgRepository {
    pool: PgPool,
}

impl NovelPgRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// `import-provider-budget-v1`: a claimable job at the attempt ceiling is
    /// terminally failed with `budget_exhausted`; recovery and retry must
    /// never reclaim it.
    async fn mark_import_budget_exhausted(&self, novel_id: Uuid, user_id: Uuid) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        let job = sqlx::query(
            r#"
            UPDATE novel_import_jobs AS job
            SET status = 'failed', failure_code = 'budget_exhausted',
                lease_expires_at = NULL, updated_at = NOW()
            FROM novels AS owned
            WHERE job.novel_id = owned.id
              AND job.novel_id = $1
              AND owned.user_id = $2
              AND job.attempt >= $3
              AND job.status <> 'completed'
              AND (job.status <> 'failed' OR job.failure_code IS DISTINCT FROM 'budget_exhausted')
            "#,
        )
        .bind(novel_id)
        .bind(user_id)
        .bind(MAX_IMPORT_ATTEMPTS)
        .execute(&mut *transaction)
        .await?;
        if job.rows_affected() == 1 {
            sqlx::query(
                "UPDATE novels \
                 SET status = 'error'::novel_status, parse_error = $2, updated_at = NOW() \
                 WHERE id = $1 AND status <> 'ready'::novel_status",
            )
            .bind(novel_id)
            .bind(IMPORT_BUDGET_EXHAUSTED_MESSAGE)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

async fn insert_novel(tx: &mut Transaction<'_, Postgres>, novel: &Novel) -> Result<()> {
    if let Some(key) = &novel.file_key {
        // Consume and lock the upload reservation before touching `novels`.
        // Cleanup claims the same row with SKIP LOCKED, so a slow INSERT/commit
        // cannot race an external object deletion. If cleanup won first, this
        // transaction fails closed and never publishes a dangling file key.
        let reservation = sqlx::query(
            "DELETE FROM source_file_deletions WHERE object_key = $1 AND last_error = $2",
        )
        .bind(key)
        .bind(SOURCE_UPLOAD_PENDING)
        .execute(&mut **tx)
        .await?;
        ensure!(
            reservation.rows_affected() == 1,
            "retained source upload lost its database reservation"
        );
    }
    sqlx::query(
        r#"INSERT INTO novels (
            id, user_id, title, author, cover_url, description,
            world_summary, genre, original_file_key, total_chapters,
            status, parse_error, deviation_mode, created_at, updated_at
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11::novel_status,$12,$13::deviation_mode,$14,$15)"#,
    )
    .bind(novel.id)
    .bind(novel.user_id)
    .bind(&novel.title)
    .bind(&novel.author)
    .bind(&novel.cover_url)
    .bind(&novel.description)
    .bind(&novel.world_summary)
    .bind(&novel.genre)
    .bind(&novel.file_key)
    .bind(novel.total_chapters)
    .bind(novel.status.to_str())
    .bind(&novel.parse_error)
    .bind(novel.deviation_mode.to_str())
    .bind(novel.created_at)
    .bind(novel.updated_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_initial_shelf(tx: &mut Transaction<'_, Postgres>, novel: &Novel) -> Result<()> {
    sqlx::query("INSERT INTO user_novels (user_id, novel_id, added_at) VALUES ($1, $2, $3)")
        .bind(novel.user_id)
        .bind(novel.id)
        .bind(novel.created_at)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO reading_progress \
         (id, user_id, novel_id, current_chapter, reader_identity_type, deviation_mode, last_read_at, created_at) \
         VALUES ($1, $2, $3, 1, 'self', $4::deviation_mode, $5, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(novel.user_id)
    .bind(novel.id)
    .bind(novel.deviation_mode.to_str())
    .bind(novel.created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_pending_import(
    tx: &mut Transaction<'_, Postgres>,
    novel: &Novel,
    chapters: &[Chapter],
) -> Result<()> {
    let stage = if chapters.is_empty() {
        ensure!(
            novel.file_key.is_some(),
            "source-stage import requires a retained source key"
        );
        "source"
    } else {
        ensure!(
            chapters.iter().all(|chapter| chapter.novel_id == novel.id),
            "durable import chapters belong to another novel"
        );
        ensure!(
            chapters_are_importable(chapters),
            "durable import chapters must be contiguous and non-empty"
        );
        "chapters"
    };

    insert_novel(tx, novel).await?;
    insert_initial_shelf(tx, novel).await?;
    if !chapters.is_empty() {
        save_batch_in_transaction(tx, chapters).await?;
    }
    sqlx::query(
        "INSERT INTO novel_import_jobs (novel_id, stage, status) \
         VALUES ($1, $2, 'pending')",
    )
    .bind(novel.id)
    .bind(stage)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn lock_import(
    tx: &mut Transaction<'_, Postgres>,
    novel_id: Uuid,
    attempt: i64,
) -> Result<Option<ImportStage>> {
    let stage = sqlx::query_scalar::<_, String>(
        "SELECT stage FROM novel_import_jobs \
         WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress' \
         FOR UPDATE",
    )
    .bind(novel_id)
    .bind(attempt)
    .fetch_optional(&mut **tx)
    .await?;
    stage
        .map(|value| {
            ImportStage::from_str(&value)
                .ok_or_else(|| anyhow::anyhow!("invalid persisted import stage"))
        })
        .transpose()
}

#[async_trait]
impl NovelRepository for NovelPgRepository {
    async fn create_import(&self, novel: &Novel, chapters: &[Chapter]) -> Result<()> {
        ensure!(!chapters.is_empty(), "durable import requires chapters");
        let mut transaction = self.pool.begin().await?;
        insert_pending_import(&mut transaction, novel, chapters).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn create_import_batch(&self, imports: &[(Novel, Vec<Chapter>)]) -> Result<()> {
        ensure!(!imports.is_empty(), "import batch cannot be empty");
        let mut transaction = self.pool.begin().await?;
        for (novel, chapters) in imports {
            insert_pending_import(&mut transaction, novel, chapters).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn create_source_import(&self, novel: &Novel) -> Result<()> {
        ensure!(
            novel.file_key.is_some(),
            "source-stage import requires a retained source key"
        );
        let mut transaction = self.pool.begin().await?;
        insert_pending_import(&mut transaction, novel, &[]).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn claim_import(&self, novel_id: Uuid, user_id: Uuid) -> Result<Option<ImportClaim>> {
        let row = sqlx::query_as::<_, ImportClaimRow>(
            r#"
            WITH claimed AS (
                UPDATE novel_import_jobs AS job
                SET status = 'in_progress', attempt = attempt + 1,
                    lease_expires_at = NOW() + INTERVAL '2 minutes',
                    failure_code = NULL, updated_at = NOW()
                WHERE job.novel_id = $1
                  AND job.stage IN ('source', 'chapters', 'enriched')
                  AND job.attempt < $3
                  AND EXISTS (
                      SELECT 1 FROM novels AS owned
                      WHERE owned.id = job.novel_id AND owned.user_id = $2
                  )
                  AND (
                      job.status IN ('pending', 'failed')
                      OR (job.status = 'in_progress' AND job.lease_expires_at <= NOW())
                  )
                RETURNING job.stage, job.attempt
            )
            UPDATE novels AS novel
            SET status = 'parsing', parse_error = NULL, updated_at = NOW()
            FROM claimed
            WHERE novel.id = $1 AND novel.user_id = $2
            RETURNING novel.id AS novel_id, novel.user_id, claimed.stage, claimed.attempt
            "#,
        )
        .bind(novel_id)
        .bind(user_id)
        .bind(MAX_IMPORT_ATTEMPTS)
        .fetch_optional(&self.pool)
        .await?;
        if row.is_none() {
            self.mark_import_budget_exhausted(novel_id, user_id).await?;
        }
        row.map(ImportClaimRow::into_domain).transpose()
    }

    async fn recoverable_imports(&self, limit: i64) -> Result<Vec<RecoverableImport>> {
        ensure!(
            (1..=100).contains(&limit),
            "recoverable import limit is invalid"
        );
        let rows = sqlx::query_as::<_, RecoverableImportRow>(
            r#"
            SELECT job.novel_id, novel.user_id
            FROM novel_import_jobs AS job
            JOIN novels AS novel ON novel.id = job.novel_id
            WHERE job.status = 'pending'
               OR (job.status = 'in_progress' AND job.lease_expires_at <= NOW())
            ORDER BY COALESCE(job.lease_expires_at, job.created_at), job.novel_id
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| RecoverableImport {
                novel_id: row.novel_id,
                user_id: row.user_id,
            })
            .collect())
    }

    async fn renew_import(&self, novel_id: Uuid, attempt: i64) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE novel_import_jobs
            SET lease_expires_at = NOW() + INTERVAL '2 minutes', updated_at = NOW()
            WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress'
            "#,
        )
        .bind(novel_id)
        .bind(attempt)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn replace_import_chapters(
        &self,
        novel_id: Uuid,
        attempt: i64,
        chapters: &[Chapter],
    ) -> Result<bool> {
        ensure!(!chapters.is_empty(), "import replacement requires chapters");
        ensure!(
            chapters.iter().all(|chapter| chapter.novel_id == novel_id),
            "replacement chapters belong to another novel"
        );
        ensure!(
            chapters_are_importable(chapters),
            "replacement chapters must be contiguous and non-empty"
        );
        let mut transaction = self.pool.begin().await?;
        let Some(stage) = lock_import(&mut transaction, novel_id, attempt).await? else {
            return Ok(false);
        };
        if !matches!(stage, ImportStage::Source | ImportStage::Chapters) {
            return Ok(false);
        }
        // Source replay and pre-enrichment boundary repair both replace the
        // complete aggregate. Derived chunks cascade.
        sqlx::query("DELETE FROM chapters WHERE novel_id = $1")
            .bind(novel_id)
            .execute(&mut *transaction)
            .await?;
        save_batch_in_transaction(&mut transaction, chapters).await?;
        if stage == ImportStage::Source {
            let job = sqlx::query(
                "UPDATE novel_import_jobs SET stage = 'chapters', updated_at = NOW() \
                 WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress' AND stage = 'source'",
            )
            .bind(novel_id)
            .bind(attempt)
            .execute(&mut *transaction)
            .await?;
            ensure!(
                job.rows_affected() == 1,
                "replayed import stage advance failed"
            );
        }
        transaction.commit().await?;
        Ok(true)
    }

    async fn record_import_enrichment(
        &self,
        novel_id: Uuid,
        attempt: i64,
        total_chapters: i32,
        world_summary: &str,
        genre: &str,
    ) -> Result<bool> {
        ensure!(total_chapters > 0, "enriched import requires chapters");
        ensure!(!world_summary.trim().is_empty(), "world summary is empty");
        ensure!(
            !genre.trim().is_empty() && genre.chars().count() <= 100,
            "genre is invalid"
        );
        let mut transaction = self.pool.begin().await?;
        let Some(stage) = lock_import(&mut transaction, novel_id, attempt).await? else {
            return Ok(false);
        };
        ensure!(
            matches!(stage, ImportStage::Chapters | ImportStage::Enriched),
            "import enrichment is invalid for the current stage"
        );
        let chapter_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM chapters WHERE novel_id = $1")
                .bind(novel_id)
                .fetch_one(&mut *transaction)
                .await?;
        ensure!(
            chapter_count == i64::from(total_chapters),
            "enriched import chapter count does not match persisted chapters"
        );
        let novel = sqlx::query(
            r#"
            UPDATE novels
            SET total_chapters = $2, world_summary = $3, genre = $4,
                updated_at = NOW()
            WHERE id = $1 AND status = 'parsing'::novel_status
            "#,
        )
        .bind(novel_id)
        .bind(total_chapters)
        .bind(world_summary)
        .bind(genre)
        .execute(&mut *transaction)
        .await?;
        ensure!(
            novel.rows_affected() == 1,
            "enriched import novel is not parsing"
        );
        let job = sqlx::query(
            r#"
            UPDATE novel_import_jobs
            SET stage = 'enriched', lease_expires_at = NOW() + INTERVAL '2 minutes',
                updated_at = NOW()
            WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress'
            "#,
        )
        .bind(novel_id)
        .bind(attempt)
        .execute(&mut *transaction)
        .await?;
        ensure!(
            job.rows_affected() == 1,
            "enriched import job update failed"
        );
        transaction.commit().await?;
        Ok(true)
    }

    async fn complete_import(&self, novel_id: Uuid, attempt: i64) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let Some(stage) = lock_import(&mut transaction, novel_id, attempt).await? else {
            return Ok(false);
        };
        ensure!(
            stage == ImportStage::Enriched,
            "import completion is invalid for the current stage"
        );
        // Blank means "carries no character outside Unicode White_Space". The
        // explicit BTRIM() set mirrors exactly what Rust str::trim() strips, so
        // it agrees with the domain predicate chapters_are_importable. Unlike
        // POSIX [:space:] it is locale-independent: under LC_CTYPE=C the regex
        // class matches no non-ASCII character, so NBSP-only text would pass
        // here while the domain rejects it.
        let complete = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT novel.total_chapters > 0
               AND COALESCE(BTRIM(novel.world_summary, U&' \0009\000A\000B\000C\000D\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000') <> '', FALSE)
               AND COALESCE(BTRIM(novel.genre, U&' \0009\000A\000B\000C\000D\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000') <> '', FALSE)
               AND novel.total_chapters = (
                   SELECT COUNT(*)::INTEGER FROM chapters WHERE novel_id = novel.id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM chapters AS c
                   WHERE c.novel_id = novel.id
                     AND (c.chapter_number < 1
                          OR c.chapter_number > novel.total_chapters
                          OR BTRIM(c.content, U&' \0009\000A\000B\000C\000D\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000') = '')
               )
               AND EXISTS (
                   SELECT 1 FROM characters WHERE novel_id = novel.id
               )
               AND EXISTS (
                   SELECT 1 FROM canon_story_models WHERE novel_id = novel.id
               )
            FROM novels AS novel
            WHERE novel.id = $1 AND novel.status = 'parsing'::novel_status
            "#,
        )
        .bind(novel_id)
        .fetch_optional(&mut *transaction)
        .await?
        .unwrap_or(false);
        ensure!(complete, "completed import is missing authoritative data");
        let novel = sqlx::query(
            "UPDATE novels SET status = 'ready'::novel_status, parse_error = NULL, \
             updated_at = NOW() WHERE id = $1",
        )
        .bind(novel_id)
        .execute(&mut *transaction)
        .await?;
        ensure!(
            novel.rows_affected() == 1,
            "completed import novel update failed"
        );
        let job = sqlx::query(
            r#"
            UPDATE novel_import_jobs
            SET stage = 'completed', status = 'completed', lease_expires_at = NULL,
                failure_code = NULL, updated_at = NOW()
            WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress'
            "#,
        )
        .bind(novel_id)
        .bind(attempt)
        .execute(&mut *transaction)
        .await?;
        ensure!(
            job.rows_affected() == 1,
            "completed import job update failed"
        );
        transaction.commit().await?;
        Ok(true)
    }

    async fn fail_import(
        &self,
        novel_id: Uuid,
        attempt: i64,
        failure_code: &str,
        public_error: &str,
    ) -> Result<bool> {
        ensure!(
            !failure_code.is_empty()
                && failure_code.len() <= 64
                && !failure_code.chars().any(char::is_control),
            "import failure code is invalid"
        );
        ensure!(
            !public_error.trim().is_empty() && public_error.chars().count() <= 500,
            "public import error is invalid"
        );
        let result = sqlx::query(
            r#"
            WITH failed AS (
                UPDATE novel_import_jobs
                SET status = 'failed', lease_expires_at = NULL,
                    failure_code = $3, updated_at = NOW()
                WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress'
                RETURNING novel_id
            )
            UPDATE novels AS novel
            SET status = 'error'::novel_status, parse_error = $4, updated_at = NOW()
            FROM failed
            WHERE novel.id = failed.novel_id
            "#,
        )
        .bind(novel_id)
        .bind(attempt)
        .bind(failure_code)
        .bind(public_error)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Novel>> {
        let row = sqlx::query_as::<_, NovelRow>(
            "SELECT id, user_id, title, author, cover_url, description, world_summary, genre, original_file_key, total_chapters, status::text, parse_error, deviation_mode::text, created_at, updated_at FROM novels WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }

    async fn find_by_user(&self, user_id: Uuid) -> Result<Vec<Novel>> {
        let rows = sqlx::query_as::<_, NovelRow>(
            "SELECT n.id, n.user_id, n.title, n.author, n.cover_url, n.description, n.world_summary, n.genre, \
                    n.original_file_key, n.total_chapters, n.status::text, n.parse_error, \
                    COALESCE(p.deviation_mode, n.deviation_mode)::text AS deviation_mode, n.created_at, n.updated_at \
             FROM user_novels AS shelf \
             JOIN novels AS n ON n.id = shelf.novel_id \
             LEFT JOIN reading_progress AS p ON p.user_id = shelf.user_id AND p.novel_id = shelf.novel_id \
             WHERE shelf.user_id = $1 ORDER BY n.updated_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn find_for_user(&self, user_id: Uuid, novel_id: Uuid) -> Result<Option<Novel>> {
        let row = sqlx::query_as::<_, NovelRow>(
            "SELECT n.id, n.user_id, n.title, n.author, n.cover_url, n.description, n.world_summary, n.genre, \
                    n.original_file_key, n.total_chapters, n.status::text, n.parse_error, \
                    COALESCE(p.deviation_mode, n.deviation_mode)::text AS deviation_mode, n.created_at, n.updated_at \
             FROM user_novels AS shelf \
             JOIN novels AS n ON n.id = shelf.novel_id \
             LEFT JOIN reading_progress AS p ON p.user_id = shelf.user_id AND p.novel_id = shelf.novel_id \
             WHERE shelf.user_id = $1 AND shelf.novel_id = $2",
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn find_available_to_user(&self, user_id: Uuid) -> Result<Vec<Novel>> {
        let rows = sqlx::query_as::<_, NovelRow>(
            "SELECT n.id, n.user_id, n.title, n.author, n.cover_url, n.description, n.world_summary, n.genre, \
                    n.original_file_key, n.total_chapters, n.status::text, n.parse_error, n.deviation_mode::text, \
                    n.created_at, n.updated_at \
             FROM novels AS n \
             WHERE n.status = 'ready'::novel_status \
               AND NOT EXISTS (SELECT 1 FROM user_novels AS shelf WHERE shelf.user_id = $1 AND shelf.novel_id = n.id) \
             ORDER BY n.updated_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn attach_to_user(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        deviation_mode: DeviationMode,
    ) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let ready = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM novels WHERE id = $1 AND status = 'ready'::novel_status)",
        )
        .bind(novel_id)
        .fetch_one(&mut *transaction)
        .await?;
        if !ready {
            transaction.rollback().await?;
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2) \
             ON CONFLICT (user_id, novel_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(novel_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO reading_progress \
             (id, user_id, novel_id, current_chapter, reader_identity_type, deviation_mode) \
             VALUES ($1, $2, $3, 1, 'self', $4::deviation_mode) \
             ON CONFLICT (user_id, novel_id) DO UPDATE \
             SET deviation_mode = EXCLUDED.deviation_mode",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(novel_id)
        .bind(deviation_mode.to_str())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    async fn detach_from_user(&self, user_id: Uuid, novel_id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM user_novels WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }
}

#[derive(sqlx::FromRow)]
struct NovelRow {
    id: Uuid,
    user_id: Uuid,
    title: String,
    author: Option<String>,
    cover_url: Option<String>,
    description: Option<String>,
    world_summary: Option<String>,
    genre: Option<String>,
    original_file_key: Option<String>,
    total_chapters: i32,
    status: String,
    parse_error: Option<String>,
    deviation_mode: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct ImportClaimRow {
    novel_id: Uuid,
    user_id: Uuid,
    stage: String,
    attempt: i64,
}

impl ImportClaimRow {
    fn into_domain(self) -> Result<ImportClaim> {
        Ok(ImportClaim {
            novel_id: self.novel_id,
            user_id: self.user_id,
            stage: ImportStage::from_str(&self.stage)
                .ok_or_else(|| anyhow::anyhow!("invalid persisted import stage"))?,
            attempt: self.attempt,
        })
    }
}

#[derive(sqlx::FromRow)]
struct RecoverableImportRow {
    novel_id: Uuid,
    user_id: Uuid,
}

impl From<NovelRow> for Novel {
    fn from(r: NovelRow) -> Self {
        Novel {
            id: r.id,
            user_id: r.user_id,
            title: r.title,
            author: r.author,
            cover_url: r.cover_url,
            description: r.description,
            world_summary: r.world_summary,
            genre: r.genre,
            file_key: r.original_file_key,
            total_chapters: r.total_chapters,
            status: NovelStatus::from_str(&r.status),
            parse_error: r.parse_error,
            deviation_mode: DeviationMode::from_str(&r.deviation_mode),
            created_at: r.created_at,
            updated_at: r.updated_at,
            domain_events: vec![],
        }
    }
}
