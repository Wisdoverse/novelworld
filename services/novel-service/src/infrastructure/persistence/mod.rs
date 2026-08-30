pub mod account_export;
pub mod canon_story_model_pg_repo;
pub mod chapter_pg_repo;
pub mod chapter_translation_pg_repo;
pub mod character_pg_repo;
pub mod novel_pg_repo;
pub mod pg_progress_repo;
pub mod source_file_deletion_pg_repo;

pub(crate) const SOURCE_UPLOAD_PENDING: &str = "__source_upload_pending__";
pub(crate) const SOURCE_DELETE_CLAIM_PREFIX: &str = "__source_delete_claimed__";

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
                    ),
                    checkpoint_columns AS MATERIALIZED (
                        SELECT novel_id, model_version, prompt_version,
                               chapter_number, chunk_index, is_final,
                               source_content, extraction, created_at, updated_at
                        FROM public.canon_extraction_checkpoints
                        LIMIT 1
                    ),
                    translation_columns AS MATERIALIZED (
                        SELECT chapter_id, source_sha256, profile, status, attempt,
                               lease_expires_at, retry_after_at, translated_content,
                               failure_code, created_at, updated_at, completed_at
                        FROM public.chapter_translations
                        LIMIT 1
                    ),
                    erasure_columns AS MATERIALIZED (
                        SELECT subject_type, subject_id, user_id, erased_at,
                               had_source, source_requeued_at
                        FROM public.erasure_records
                        LIMIT 1
                    )
                    SELECT (SELECT pg_catalog.count(*) >= 0 FROM contract_columns)
                       AND (SELECT pg_catalog.count(*) >= 0 FROM checkpoint_columns)
                       AND (SELECT pg_catalog.count(*) >= 0 FROM translation_columns)
                       AND (SELECT pg_catalog.count(*) >= 0 FROM erasure_columns)
                       AND NOT EXISTS (
                           SELECT 1
                           FROM (
                               VALUES
                                   ('chapter_id', 'uuid', TRUE, NULL::pg_catalog.text),
                                   ('source_sha256', 'bytea', TRUE, NULL::pg_catalog.text),
                                   ('profile', 'character varying(64)', TRUE, NULL::pg_catalog.text),
                                   ('status', 'character varying(16)', TRUE, NULL::pg_catalog.text),
                                   ('attempt', 'bigint', TRUE, '1'),
                                   ('lease_expires_at', 'timestamp with time zone', FALSE, NULL::pg_catalog.text),
                                   ('retry_after_at', 'timestamp with time zone', FALSE, NULL::pg_catalog.text),
                                   ('translated_content', 'text', FALSE, NULL::pg_catalog.text),
                                   ('failure_code', 'character varying(64)', FALSE, NULL::pg_catalog.text),
                                   ('created_at', 'timestamp with time zone', TRUE, 'now()'),
                                   ('updated_at', 'timestamp with time zone', TRUE, 'now()'),
                                   ('completed_at', 'timestamp with time zone', FALSE, NULL::pg_catalog.text)
                           ) AS expected(attname, type_name, is_not_null, default_expression)
                           LEFT JOIN pg_catalog.pg_attribute AS actual
                             ON actual.attrelid =
                                    'public.chapter_translations'::pg_catalog.regclass
                            AND actual.attname = expected.attname
                            AND actual.attnum > 0
                            AND NOT actual.attisdropped
                           LEFT JOIN pg_catalog.pg_attrdef AS actual_default
                             ON actual_default.adrelid = actual.attrelid
                            AND actual_default.adnum = actual.attnum
                           WHERE actual.attname IS NULL
                              OR pg_catalog.format_type(actual.atttypid, actual.atttypmod)
                                     IS DISTINCT FROM expected.type_name
                              OR actual.attnotnull IS DISTINCT FROM expected.is_not_null
                              OR pg_catalog.pg_get_expr(
                                     actual_default.adbin,
                                     actual_default.adrelid
                                 ) IS DISTINCT FROM expected.default_expression
                       )
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_constraint AS translation_pk
                           WHERE translation_pk.conrelid =
                                     'public.chapter_translations'::pg_catalog.regclass
                             AND translation_pk.contype::pg_catalog.text = 'p'
                             AND translation_pk.convalidated
                             AND NOT translation_pk.condeferrable
                             AND NOT translation_pk.condeferred
                             AND translation_pk.conkey = ARRAY[
                                   (SELECT attnum FROM pg_catalog.pg_attribute
                                    WHERE attrelid = translation_pk.conrelid
                                      AND attname = 'chapter_id'),
                                   (SELECT attnum FROM pg_catalog.pg_attribute
                                    WHERE attrelid = translation_pk.conrelid
                                      AND attname = 'source_sha256'),
                                   (SELECT attnum FROM pg_catalog.pg_attribute
                                    WHERE attrelid = translation_pk.conrelid
                                      AND attname = 'profile')
                             ]
                       )
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_constraint AS translation_fk
                           WHERE translation_fk.conrelid =
                                     'public.chapter_translations'::pg_catalog.regclass
                             AND translation_fk.confrelid =
                                     'public.chapters'::pg_catalog.regclass
                             AND translation_fk.contype::pg_catalog.text = 'f'
                             AND translation_fk.confdeltype::pg_catalog.text = 'c'
                             AND translation_fk.convalidated
                             AND translation_fk.conkey = ARRAY[(
                                   SELECT attnum FROM pg_catalog.pg_attribute
                                   WHERE attrelid = translation_fk.conrelid
                                     AND attname = 'chapter_id'
                             )]
                             AND translation_fk.confkey = ARRAY[(
                                   SELECT attnum FROM pg_catalog.pg_attribute
                                   WHERE attrelid = translation_fk.confrelid
                                     AND attname = 'id'
                             )]
                       )
                       AND NOT EXISTS (
                           SELECT 1
                           FROM (
                               VALUES
                                   ('chapter_translations_source_sha256_check',
                                    'CHECK ((octet_length(source_sha256) = 32))'),
                                   ('chapter_translations_profile_check',
                                    'CHECK (((char_length((profile)::text) >= 1) AND (char_length((profile)::text) <= 64)))'),
                                   ('chapter_translations_attempt_check',
                                    'CHECK ((attempt >= 1))'),
                                   ('chapter_translations_state_check',
                                    'CHECK (((((status)::text = ''translating''::text) AND (lease_expires_at IS NOT NULL) AND (retry_after_at IS NULL) AND (translated_content IS NULL) AND (failure_code IS NULL) AND (completed_at IS NULL)) OR (((status)::text = ''ready''::text) AND (lease_expires_at IS NULL) AND (retry_after_at IS NULL) AND (translated_content IS NOT NULL) AND (translated_content <> ''''::text) AND (failure_code IS NULL) AND (completed_at IS NOT NULL)) OR (((status)::text = ''failed''::text) AND (lease_expires_at IS NULL) AND (retry_after_at IS NOT NULL) AND (translated_content IS NULL) AND (failure_code IS NOT NULL) AND ((failure_code)::text <> ''''::text) AND (completed_at IS NULL))))')
                           ) AS expected(conname, definition)
                           LEFT JOIN pg_catalog.pg_constraint AS actual
                             ON actual.conrelid =
                                    'public.chapter_translations'::pg_catalog.regclass
                            AND actual.conname = expected.conname
                           WHERE actual.oid IS NULL
                              OR actual.contype::pg_catalog.text <> 'c'
                              OR NOT actual.convalidated
                              OR pg_catalog.pg_get_constraintdef(actual.oid, FALSE)
                                     <> expected.definition
                       )
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_constraint AS status_check
                           WHERE status_check.conrelid =
                                     'public.chapter_translations'::pg_catalog.regclass
                             AND status_check.conname =
                                     'chapter_translations_status_check'
                             AND status_check.contype::pg_catalog.text = 'c'
                             AND status_check.convalidated
                             AND pg_catalog.pg_get_constraintdef(
                                     status_check.oid,
                                     FALSE
                                 ) IN (
                                   'CHECK (((status)::text = ANY ((ARRAY[''translating''::character varying, ''ready''::character varying, ''failed''::character varying])::text[])))',
                                   'CHECK (((status)::text = ANY (ARRAY[(''translating''::character varying)::text, (''ready''::character varying)::text, (''failed''::character varying)::text])))'
                             )
                       )
                       -- Exactly one lineage token. Without it a restore cannot
                       -- tell a continuation from a sibling of the same
                       -- artifact, so a database that lost the row must not
                       -- serve.
                       AND (
                           SELECT pg_catalog.count(*) = 1
                           FROM public.database_lineage
                           WHERE token IS NOT NULL
                       )
                       -- Erasure records are the deletion-enforcement journal
                       -- replayed by the migration path; a database that lost
                       -- the primary key or either AFTER DELETE row trigger can
                       -- silently stop recording deletions, so readiness fails
                       -- closed on both. Catalog identity again, not deparsed
                       -- text: tgfoid resolves through search_path.
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_constraint AS erasure_pk
                           WHERE erasure_pk.conrelid =
                                     'public.erasure_records'::pg_catalog.regclass
                             AND erasure_pk.contype::pg_catalog.text = 'p'
                             AND erasure_pk.conkey = ARRAY[
                                     (SELECT subject.attnum
                                      FROM pg_catalog.pg_attribute AS subject
                                      WHERE subject.attrelid = erasure_pk.conrelid
                                        AND subject.attname = 'subject_type'),
                                     (SELECT subject.attnum
                                      FROM pg_catalog.pg_attribute AS subject
                                      WHERE subject.attrelid = erasure_pk.conrelid
                                        AND subject.attname = 'subject_id')
                                 ]
                       )
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_constraint AS subject_type_check
                           WHERE subject_type_check.conrelid =
                                     'public.erasure_records'::pg_catalog.regclass
                             AND subject_type_check.conname =
                                     'erasure_records_subject_type_check'
                             AND subject_type_check.contype::pg_catalog.text = 'c'
                             AND pg_catalog.pg_get_constraintdef(subject_type_check.oid) IN (
                                 'CHECK (((subject_type)::text = ANY ((ARRAY[''user''::character varying, ''novel''::character varying])::text[])))',
                                 'CHECK (((subject_type)::text = ANY (ARRAY[(''user''::character varying)::text, (''novel''::character varying)::text])))'
                             )
                       )
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_trigger AS erasure_trigger
                           WHERE erasure_trigger.tgrelid =
                                     'public.users'::pg_catalog.regclass
                             AND erasure_trigger.tgname = 'record_user_erasure'
                             AND NOT erasure_trigger.tgisinternal
                             AND erasure_trigger.tgenabled::pg_catalog.text <> 'D'
                             AND erasure_trigger.tgtype = 9
                             AND erasure_trigger.tgfoid =
                                 'public.record_user_erasure()'::pg_catalog.regprocedure
                       )
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_trigger AS erasure_trigger
                           WHERE erasure_trigger.tgrelid =
                                     'public.novels'::pg_catalog.regclass
                             AND erasure_trigger.tgname = 'record_novel_erasure'
                             AND NOT erasure_trigger.tgisinternal
                             AND erasure_trigger.tgenabled::pg_catalog.text <> 'D'
                             AND erasure_trigger.tgtype = 9
                             AND erasure_trigger.tgfoid =
                                 'public.record_novel_erasure()'::pg_catalog.regprocedure
                       )
                       AND EXISTS (
                           SELECT 1 FROM pg_catalog.pg_constraint
                           WHERE conrelid =
                                     'public.novel_import_jobs'::pg_catalog.regclass
                             AND conname = 'novel_import_jobs_stage_check'
                             AND contype::pg_catalog.text = 'c'
                             -- Two spellings of one constraint: PostgreSQL
                             -- deparses CHECK (stage IN (...)) over a varchar
                             -- column as the first form and re-parses that text
                             -- into the second, which is what restoring a
                             -- pg_dump artifact produces. Only one of them can
                             -- match, so accepting both keeps a restored
                             -- deployment able to reach readiness without
                             -- loosening drift detection.
                             AND pg_catalog.pg_get_constraintdef(oid) IN (
                                 'CHECK (((stage)::text = ANY ((ARRAY[''source''::character varying, ''chapters''::character varying, ''enriched''::character varying, ''completed''::character varying])::text[])))',
                                 'CHECK (((stage)::text = ANY (ARRAY[(''source''::character varying)::text, (''chapters''::character varying)::text, (''enriched''::character varying)::text, (''completed''::character varying)::text])))'
                             )
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
                             AND pg_catalog.pg_get_indexdef(index_definition.indexrelid) IN (
                                 'CREATE INDEX idx_novel_import_jobs_recoverable ON public.novel_import_jobs USING btree (status, lease_expires_at, created_at) WHERE ((status)::text = ANY ((ARRAY[''pending''::character varying, ''in_progress''::character varying])::text[]))',
                                 'CREATE INDEX idx_novel_import_jobs_recoverable ON public.novel_import_jobs USING btree (status, lease_expires_at, created_at) WHERE ((status)::text = ANY (ARRAY[(''pending''::character varying)::text, (''in_progress''::character varying)::text]))'
                             )
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
