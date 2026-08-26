pub mod account_export;
pub mod pg_chat_repo;
pub mod pg_memory_repo;

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
                sqlx::query_as::<
                    _,
                    (
                        Option<uuid::Uuid>,
                        Option<uuid::Uuid>,
                        bool,
                        bool,
                        bool,
                        bool,
                        bool,
                        bool,
                    ),
                >(
                    r#"
                    SELECT
                        (SELECT id FROM public.chat_turns LIMIT 1),
                        (SELECT turn_id FROM public.chat_messages LIMIT 1),
                        EXISTS (
                            SELECT 1
                            FROM pg_catalog.pg_attribute AS attribute
                            LEFT JOIN pg_catalog.pg_attrdef AS default_definition
                              ON default_definition.adrelid = attribute.attrelid
                             AND default_definition.adnum = attribute.attnum
                            WHERE attribute.attrelid =
                                      'public.chat_turns'::pg_catalog.regclass
                              AND attribute.attname =
                                      'persona_source_chapter_high_water'
                              AND attribute.attnum > 0
                              AND NOT attribute.attisdropped
                              AND attribute.atttypid = 'pg_catalog.int4'::pg_catalog.regtype
                              AND attribute.atttypmod = -1
                              AND NOT attribute.attnotnull
                              AND default_definition.oid IS NULL
                        ),
                        EXISTS (
                            SELECT 1
                            FROM pg_catalog.pg_constraint AS constraint_definition
                            WHERE constraint_definition.conrelid =
                                      'public.chat_turns'::pg_catalog.regclass
                              AND constraint_definition.conname =
                                      'chat_turns_persona_source_chapter_high_water_check'
                              AND constraint_definition.contype::pg_catalog.text = 'c'
                              AND constraint_definition.convalidated
                              AND pg_catalog.pg_get_constraintdef(
                                      constraint_definition.oid, FALSE
                                  ) = 'CHECK (((persona_source_chapter_high_water IS NULL) OR ((persona_source_chapter_high_water >= 1) AND (persona_source_chapter_high_water <= chapter_context))))'
                        ),
                        EXISTS (
                            SELECT 1
                            FROM pg_catalog.pg_attribute AS attribute
                            LEFT JOIN pg_catalog.pg_attrdef AS default_definition
                              ON default_definition.adrelid = attribute.attrelid
                             AND default_definition.adnum = attribute.attnum
                            WHERE attribute.attrelid =
                                      'public.character_memories'::pg_catalog.regclass
                              AND attribute.attname =
                                      'persona_source_chapter_high_water'
                              AND attribute.attnum > 0
                              AND NOT attribute.attisdropped
                              AND attribute.atttypid = 'pg_catalog.int4'::pg_catalog.regtype
                              AND attribute.atttypmod = -1
                              AND NOT attribute.attnotnull
                              AND default_definition.oid IS NULL
                        ),
                        EXISTS (
                            SELECT 1
                            FROM pg_catalog.pg_constraint AS constraint_definition
                            WHERE constraint_definition.conrelid =
                                      'public.character_memories'::pg_catalog.regclass
                              AND constraint_definition.conname =
                                      'character_memories_persona_source_chapter_high_water_check'
                              AND constraint_definition.contype::pg_catalog.text = 'c'
                              AND constraint_definition.convalidated
                              AND pg_catalog.pg_get_constraintdef(
                                      constraint_definition.oid, FALSE
                                  ) = 'CHECK (((persona_source_chapter_high_water IS NULL) OR ((((layer)::text = ''mid''::text) OR ((layer)::text = ''long''::text)) AND (chapter_number IS NOT NULL) AND (persona_source_chapter_high_water >= 1) AND (persona_source_chapter_high_water <= chapter_number))))'
                        ),
                        EXISTS (
                            SELECT 1
                            FROM pg_catalog.pg_index AS index_definition
                            JOIN pg_catalog.pg_class AS index_relation
                              ON index_relation.oid = index_definition.indexrelid
                            JOIN pg_catalog.pg_am AS access_method
                              ON access_method.oid = index_relation.relam
                            WHERE index_relation.relnamespace =
                                      'public'::pg_catalog.regnamespace
                              AND index_relation.relname =
                                      'idx_chat_turns_one_in_progress'
                              AND index_definition.indrelid =
                                      'public.chat_turns'::pg_catalog.regclass
                              AND index_definition.indisunique
                              AND index_definition.indisvalid
                              AND index_definition.indisready
                              AND index_definition.indnkeyatts = 3
                              AND index_definition.indnatts = 3
                              AND access_method.amname = 'btree'
                              AND pg_catalog.pg_get_indexdef(
                                      index_definition.indexrelid, 1, true
                                  ) = 'user_id'
                              AND pg_catalog.pg_get_indexdef(
                                      index_definition.indexrelid, 2, true
                                  ) = 'character_id'
                              AND pg_catalog.pg_get_indexdef(
                                      index_definition.indexrelid, 3, true
                                  ) = 'novel_id'
                              AND pg_catalog.pg_get_expr(
                                      index_definition.indpred,
                                      index_definition.indrelid
                                  ) = '((status)::text = ''in_progress''::text)'
                        ),
                        EXISTS (
                            SELECT 1
                            FROM pg_catalog.pg_index AS index_definition
                            JOIN pg_catalog.pg_class AS index_relation
                              ON index_relation.oid = index_definition.indexrelid
                            JOIN pg_catalog.pg_am AS access_method
                              ON access_method.oid = index_relation.relam
                            WHERE index_relation.relnamespace =
                                      'public'::pg_catalog.regnamespace
                              AND index_relation.relname =
                                      'idx_chat_messages_turn_role_unique'
                              AND index_definition.indrelid =
                                      'public.chat_messages'::pg_catalog.regclass
                              AND index_definition.indisunique
                              AND index_definition.indisvalid
                              AND index_definition.indisready
                              AND index_definition.indnkeyatts = 2
                              AND index_definition.indnatts = 2
                              AND access_method.amname = 'btree'
                              AND pg_catalog.pg_get_indexdef(
                                      index_definition.indexrelid, 1, true
                                  ) = 'turn_id'
                              AND pg_catalog.pg_get_indexdef(
                                      index_definition.indexrelid, 2, true
                                  ) = 'role'
                              AND pg_catalog.pg_get_expr(
                                      index_definition.indpred,
                                      index_definition.indrelid
                                  ) = '(turn_id IS NOT NULL)'
                        )
                    "#,
                )
                .fetch_one(&self.pool),
            )
            .await,
            Ok(Ok((_, _, true, true, true, true, true, true)))
        )
    }
}
