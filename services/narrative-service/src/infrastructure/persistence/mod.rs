pub mod pg_narrative_repo;
pub mod pg_world_state_repo;

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
                    SELECT pg_catalog.count(*) = 7
                    FROM pg_catalog.pg_constraint AS actual
                    JOIN (VALUES
                        ('public.narrative_nodes'::pg_catalog.regclass, 'narrative_nodes_identity_key', 'u', 'UNIQUE (id, novel_id, chapter_number)'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_user_node_key', 'u', 'UNIQUE (user_id, node_id)'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_node_scope_fkey', 'f', 'FOREIGN KEY (node_id, novel_id, chapter_number) REFERENCES narrative_nodes(id, novel_id, chapter_number) ON DELETE CASCADE'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_chapter_check', 'c', 'CHECK ((chapter_number >= 1))'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_index_check', 'c', 'CHECK ((choice_index >= 0))'),
                        ('public.user_choices'::pg_catalog.regclass, 'user_choices_text_check', 'c', 'CHECK ((choice_text <> ''''::text))'),
                        ('public.player_chapters'::pg_catalog.regclass, 'player_chapters_user_id_novel_id_chapter_number_key', 'u', 'UNIQUE (user_id, novel_id, chapter_number)')
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
