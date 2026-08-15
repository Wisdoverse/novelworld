pub mod account_export;
pub mod canon_story_model_pg_repo;
pub mod chapter_pg_repo;
pub mod character_pg_repo;
pub mod novel_pg_repo;
pub mod pg_progress_repo;
pub mod source_file_deletion_pg_repo;

use crate::domain::ports::ReadinessProbe;
use async_trait::async_trait;
use sqlx::PgPool;
use std::time::Duration;

pub struct PgReadinessProbe {
    pool: PgPool,
}

impl PgReadinessProbe {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReadinessProbe for PgReadinessProbe {
    async fn is_ready(&self) -> bool {
        matches!(
            tokio::time::timeout(
                Duration::from_secs(2),
                sqlx::query_scalar::<_, bool>(
                    r#"
                    WITH contract_columns AS MATERIALIZED (
                        SELECT novel_id, stage, status, attempt, lease_expires_at,
                               failure_code, created_at, updated_at
                        FROM public.novel_import_jobs
                        LIMIT 1
                    )
                    SELECT (SELECT pg_catalog.count(*) >= 0 FROM contract_columns)
                       AND EXISTS (
                           SELECT 1 FROM pg_catalog.pg_constraint
                           WHERE conrelid =
                                     'public.novel_import_jobs'::pg_catalog.regclass
                             AND conname = 'novel_import_jobs_stage_check'
                             AND contype::pg_catalog.text = 'c'
                             AND pg_catalog.pg_get_constraintdef(oid) =
                                 'CHECK (((stage)::text = ANY ((ARRAY[''source''::character varying, ''chapters''::character varying, ''enriched''::character varying, ''completed''::character varying])::text[])))'
                       )
                       AND EXISTS (
                           SELECT 1 FROM pg_catalog.pg_constraint
                           WHERE conrelid =
                                     'public.novel_import_jobs'::pg_catalog.regclass
                             AND conname = 'novel_import_jobs_attempt_check'
                             AND contype::pg_catalog.text = 'c'
                             AND pg_catalog.pg_get_constraintdef(oid) =
                                 'CHECK ((attempt >= 0))'
                       )
                       AND EXISTS (
                           SELECT 1 FROM pg_catalog.pg_constraint
                           WHERE conrelid =
                                     'public.novel_import_jobs'::pg_catalog.regclass
                             AND conname = 'novel_import_jobs_state_check'
                             AND contype::pg_catalog.text = 'c'
                             AND pg_catalog.pg_get_constraintdef(oid) =
                                 'CHECK (((((status)::text = ''pending''::text) AND (attempt = 0) AND (lease_expires_at IS NULL) AND (failure_code IS NULL) AND ((stage)::text <> ''completed''::text)) OR (((status)::text = ''in_progress''::text) AND (attempt >= 1) AND (lease_expires_at IS NOT NULL) AND (failure_code IS NULL) AND ((stage)::text <> ''completed''::text)) OR (((status)::text = ''failed''::text) AND (lease_expires_at IS NULL) AND (failure_code IS NOT NULL) AND ((stage)::text <> ''completed''::text)) OR (((status)::text = ''completed''::text) AND (lease_expires_at IS NULL) AND (failure_code IS NULL) AND ((stage)::text = ''completed''::text))))'
                       )
                       -- Catalog columns only: pg_get_constraintdef() deparses
                       -- the parent table name relative to search_path, so a
                       -- cascading key onto a decoy schema's novels table reads
                       -- identically while the real one fails to match.
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_constraint AS import_fk
                           WHERE import_fk.conrelid =
                                     'public.novel_import_jobs'::pg_catalog.regclass
                             AND import_fk.confrelid =
                                     'public.novels'::pg_catalog.regclass
                             AND import_fk.contype::pg_catalog.text = 'f'
                             AND import_fk.confdeltype::pg_catalog.text = 'c'
                             AND import_fk.convalidated
                             AND import_fk.conkey = ARRAY[(
                                     SELECT child.attnum
                                     FROM pg_catalog.pg_attribute AS child
                                     WHERE child.attrelid = import_fk.conrelid
                                       AND child.attname = 'novel_id'
                                 )]
                             AND import_fk.confkey = ARRAY[(
                                     SELECT parent.attnum
                                     FROM pg_catalog.pg_attribute AS parent
                                     WHERE parent.attrelid = import_fk.confrelid
                                       AND parent.attname = 'id'
                                 )]
                       )
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_index AS index_definition
                           WHERE index_definition.indexrelid =
                                     'public.idx_novel_import_jobs_recoverable'::pg_catalog.regclass
                             AND index_definition.indrelid =
                                     'public.novel_import_jobs'::pg_catalog.regclass
                             AND index_definition.indisvalid
                             AND index_definition.indisready
                             AND pg_catalog.pg_get_indexdef(index_definition.indexrelid) =
                                 'CREATE INDEX idx_novel_import_jobs_recoverable ON public.novel_import_jobs USING btree (status, lease_expires_at, created_at) WHERE ((status)::text = ANY ((ARRAY[''pending''::character varying, ''in_progress''::character varying])::text[]))'
                       )
                    "#,
                )
                .fetch_one(&self.pool),
            )
            .await,
            Ok(Ok(true))
        )
    }
}
