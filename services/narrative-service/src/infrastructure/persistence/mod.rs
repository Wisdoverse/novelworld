pub mod account_export;
pub mod pg_narrative_repo;
pub mod pg_world_state_repo;
pub mod pg_world_turn_repo;

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
                    SELECT pg_catalog.count(*) = 18
                       AND (
                           SELECT pg_catalog.count(*) = 2
                           FROM pg_catalog.pg_index AS index_definition
                           JOIN (VALUES
                               ('idx_world_turns_one_in_progress', 'CREATE UNIQUE INDEX idx_world_turns_one_in_progress ON public.world_turns USING btree (user_id, novel_id) WHERE ((status)::text = ''in_progress''::text)'),
                               ('idx_world_turns_journal', 'CREATE INDEX idx_world_turns_journal ON public.world_turns USING btree (user_id, novel_id, completed_at DESC) WHERE ((status)::text = ''completed''::text)')
                           ) AS expected_index(name, definition)
                             ON pg_catalog.pg_get_indexdef(index_definition.indexrelid) = expected_index.definition
                           JOIN pg_catalog.pg_class AS index_relation
                             ON index_relation.oid = index_definition.indexrelid
                            AND index_relation.relname = expected_index.name
                           WHERE index_definition.indrelid = 'public.world_turns'::pg_catalog.regclass
                       )
                       -- Catalog columns only: pg_get_constraintdef() deparses
                       -- the parent table name relative to search_path, so a
                       -- cascading key onto a decoy schema's clone reads
                       -- identically while the real one fails to match.
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_constraint AS node_scope_fk
                           WHERE node_scope_fk.conrelid =
                                     'public.user_choices'::pg_catalog.regclass
                             AND node_scope_fk.confrelid =
                                     'public.narrative_nodes'::pg_catalog.regclass
                             AND node_scope_fk.conname = 'user_choices_node_scope_fkey'
                             AND node_scope_fk.contype::pg_catalog.text = 'f'
                             AND node_scope_fk.confdeltype::pg_catalog.text = 'c'
                             AND node_scope_fk.convalidated
                             AND node_scope_fk.conkey = ARRAY[(
                                     SELECT child.attnum
                                     FROM pg_catalog.pg_attribute AS child
                                     WHERE child.attrelid = node_scope_fk.conrelid
                                       AND child.attname = 'node_id'
                                 ), (
                                     SELECT child.attnum
                                     FROM pg_catalog.pg_attribute AS child
                                     WHERE child.attrelid = node_scope_fk.conrelid
                                       AND child.attname = 'novel_id'
                                 ), (
                                     SELECT child.attnum
                                     FROM pg_catalog.pg_attribute AS child
                                     WHERE child.attrelid = node_scope_fk.conrelid
                                       AND child.attname = 'chapter_number'
                                 )]
                             AND node_scope_fk.confkey = ARRAY[(
                                     SELECT parent.attnum
                                     FROM pg_catalog.pg_attribute AS parent
                                     WHERE parent.attrelid = node_scope_fk.confrelid
                                       AND parent.attname = 'id'
                                 ), (
                                     SELECT parent.attnum
                                     FROM pg_catalog.pg_attribute AS parent
                                     WHERE parent.attrelid = node_scope_fk.confrelid
                                       AND parent.attname = 'novel_id'
                                 ), (
                                     SELECT parent.attnum
                                     FROM pg_catalog.pg_attribute AS parent
                                     WHERE parent.attrelid = node_scope_fk.confrelid
                                       AND parent.attname = 'chapter_number'
                                 )]
                       )
                       AND EXISTS (
                           SELECT 1
                           FROM pg_catalog.pg_constraint AS world_state_fk
                           WHERE world_state_fk.conrelid =
                                     'public.world_turns'::pg_catalog.regclass
                             AND world_state_fk.confrelid =
                                     'public.world_states'::pg_catalog.regclass
                             AND world_state_fk.conname = 'world_turns_world_state_fkey'
                             AND world_state_fk.contype::pg_catalog.text = 'f'
                             AND world_state_fk.confdeltype::pg_catalog.text = 'c'
                             AND world_state_fk.convalidated
                             AND world_state_fk.conkey = ARRAY[(
                                     SELECT child.attnum
                                     FROM pg_catalog.pg_attribute AS child
                                     WHERE child.attrelid = world_state_fk.conrelid
                                       AND child.attname = 'user_id'
                                 ), (
                                     SELECT child.attnum
                                     FROM pg_catalog.pg_attribute AS child
                                     WHERE child.attrelid = world_state_fk.conrelid
                                       AND child.attname = 'novel_id'
                                 )]
                             AND world_state_fk.confkey = ARRAY[(
                                     SELECT parent.attnum
                                     FROM pg_catalog.pg_attribute AS parent
                                     WHERE parent.attrelid = world_state_fk.confrelid
                                       AND parent.attname = 'user_id'
                                 ), (
                                     SELECT parent.attnum
                                     FROM pg_catalog.pg_attribute AS parent
                                     WHERE parent.attrelid = world_state_fk.confrelid
                                       AND parent.attname = 'novel_id'
                                 )]
                       )
                    FROM pg_catalog.pg_constraint AS actual
                    JOIN (VALUES
                        ('public.narrative_nodes'::pg_catalog.regclass, 'narrative_nodes_identity_key', 'u', 'UNIQUE (id, novel_id, chapter_number)'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_user_node_key', 'u', 'UNIQUE (user_id, node_id)'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_chapter_check', 'c', 'CHECK ((chapter_number >= 1))'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_index_check', 'c', 'CHECK ((choice_index >= 0))'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_text_check', 'c', 'CHECK ((choice_text <> ''''::text))'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_consequence_check', 'c', 'CHECK ((consequence <> ''''::text))'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_transition_check', 'c', 'CHECK (((jsonb_typeof(transition) = ''object''::text) AND (transition @> ''{"schema_version": 1}''::jsonb) AND (jsonb_typeof((transition -> ''prompt_version''::text)) = ''string''::text) AND (jsonb_typeof((transition -> ''canon_model_version''::text)) = ''number''::text) AND (jsonb_typeof((transition -> ''canonical_checkpoint_chapter''::text)) = ''number''::text) AND (jsonb_typeof((transition -> ''rendered_narrative''::text)) = ''string''::text) AND (jsonb_typeof((transition -> ''events''::text)) = ''array''::text) AND (jsonb_typeof((transition -> ''relationship_changes''::text)) = ''array''::text) AND (jsonb_typeof((transition -> ''location_changes''::text)) = ''array''::text) AND (jsonb_typeof((transition -> ''thread_changes''::text)) = ''array''::text)))'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_transition_projection_check', 'c', 'CHECK (((transition ->> ''rendered_narrative''::text) = consequence))'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_consequence_not_null', 'n', 'NOT NULL consequence'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_transition_not_null', 'n', 'NOT NULL transition'),
                        ('public.player_chapters'::pg_catalog.regclass, 'player_chapters_user_id_novel_id_chapter_number_key', 'u', 'UNIQUE (user_id, novel_id, chapter_number)'),
                        ('public.world_turns'::pg_catalog.regclass, 'world_turns_pkey', 'p', 'PRIMARY KEY (id)'),
                        ('public.world_turns'::pg_catalog.regclass, 'world_turns_request_fingerprint_check', 'c', 'CHECK ((octet_length(request_fingerprint) = 32))'),
                        ('public.world_turns'::pg_catalog.regclass, 'world_turns_action_check', 'c', 'CHECK ((jsonb_typeof(action) = ''object''::text))'),
                        ('public.world_turns'::pg_catalog.regclass, 'world_turns_expected_turn_check', 'c', 'CHECK ((expected_turn_number >= 0))'),
                        ('public.world_turns'::pg_catalog.regclass, 'world_turns_status_check', 'c', 'CHECK (((status)::text = ANY ((ARRAY[''in_progress''::character varying, ''completed''::character varying, ''failed''::character varying])::text[])))'),
                        ('public.world_turns'::pg_catalog.regclass, 'world_turns_attempt_check', 'c', 'CHECK ((attempt >= 1))'),
                        ('public.world_turns'::pg_catalog.regclass, 'world_turns_state_check', 'c', 'CHECK (((((status)::text = ''in_progress''::text) AND (lease_expires_at IS NOT NULL) AND (transition IS NULL) AND (result IS NULL) AND (failure_code IS NULL) AND (completed_at IS NULL)) OR (((status)::text = ''completed''::text) AND (lease_expires_at IS NULL) AND (jsonb_typeof(transition) = ''object''::text) AND (jsonb_typeof(result) = ''object''::text) AND (failure_code IS NULL) AND (completed_at IS NOT NULL)) OR (((status)::text = ''failed''::text) AND (lease_expires_at IS NULL) AND (transition IS NULL) AND (result IS NULL) AND (failure_code IS NOT NULL) AND ((failure_code)::text <> ''''::text) AND (completed_at IS NULL))))')
                    ) AS expected(relation_id, name, kind, definition)
                      ON actual.conrelid = expected.relation_id
                     AND actual.conname = expected.name
                     AND actual.contype::pg_catalog.text = expected.kind
                     AND pg_catalog.pg_get_constraintdef(actual.oid) = expected.definition
                    "#,
                )
                .fetch_one(&self.pool),
            )
            .await,
            Ok(Ok(true))
        )
    }
}
