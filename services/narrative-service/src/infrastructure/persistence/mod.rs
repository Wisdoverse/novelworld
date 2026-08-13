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
                    SELECT pg_catalog.count(*) = 20
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
                    FROM pg_catalog.pg_constraint AS actual
                    JOIN (VALUES
                        ('public.narrative_nodes'::pg_catalog.regclass, 'narrative_nodes_identity_key', 'u', 'UNIQUE (id, novel_id, chapter_number)'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_user_node_key', 'u', 'UNIQUE (user_id, node_id)'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_node_scope_fkey', 'f', 'FOREIGN KEY (node_id, novel_id, chapter_number) REFERENCES narrative_nodes(id, novel_id, chapter_number) ON DELETE CASCADE'),
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
                        ('public.world_turns'::pg_catalog.regclass, 'world_turns_world_state_fkey', 'f', 'FOREIGN KEY (user_id, novel_id) REFERENCES world_states(user_id, novel_id) ON DELETE CASCADE'),
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
