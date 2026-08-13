use futures::TryStreamExt;
use sqlx::{prelude::FromRow, PgPool};
use uuid::Uuid;

use crate::domain::ports::{AccountExportPort, AccountExportRecord, AccountExportStream};

#[derive(FromRow)]
struct ExportRow {
    kind: String,
    data: serde_json::Value,
}

pub struct PgAccountExport {
    pool: PgPool,
}

impl PgAccountExport {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl AccountExportPort for PgAccountExport {
    fn export_user(&self, user_id: Uuid) -> AccountExportStream {
        let pool = self.pool.clone();
        Box::pin(async_stream::try_stream! {
            let mut rows = sqlx::query_as::<_, ExportRow>(EXPORT_SQL)
                .bind(user_id)
                .fetch(&pool);
            while let Some(row) = rows.try_next().await? {
                yield AccountExportRecord { kind: row.kind, data: row.data };
            }
        })
    }
}

const EXPORT_SQL: &str = r#"
WITH relevant_nodes AS (
    SELECT n.id, n.user_id, n.novel_id, n.chapter_number, n.description,
           n.anchor_quote, n.choices, n.created_at
    FROM narrative_nodes n
    WHERE n.user_id = $1
       OR (
           n.user_id IS NULL
           AND EXISTS (
               SELECT 1
               FROM user_choices c
               WHERE c.user_id = $1 AND c.node_id = n.id
           )
       )
), export_records AS (
    SELECT 10 AS section_order, n.novel_id::text AS sort_group,
           n.chapter_number::bigint AS sort_number, n.id::text AS sort_id,
           'narrative_node'::text AS kind,
           jsonb_build_object(
               'id', n.id, 'user_id', n.user_id, 'novel_id', n.novel_id,
               'chapter_number', n.chapter_number, 'description', n.description,
               'anchor_quote', n.anchor_quote, 'choices', n.choices,
               'created_at', n.created_at
           ) AS data
    FROM relevant_nodes n

    UNION ALL
    SELECT 20, c.novel_id::text, c.chapter_number::bigint, c.id::text, 'user_choice',
           jsonb_build_object(
               'id', c.id, 'user_id', c.user_id, 'novel_id', c.novel_id,
               'node_id', c.node_id, 'chapter_number', c.chapter_number,
               'choice_index', c.choice_index, 'choice_text', c.choice_text,
               'consequence', c.consequence, 'transition', c.transition,
               'created_at', c.created_at
           )
    FROM user_choices c
    WHERE c.user_id = $1

    UNION ALL
    SELECT 30, w.novel_id::text, 0::bigint, w.id::text, 'world_state',
           jsonb_build_object(
               'id', w.id, 'user_id', w.user_id, 'novel_id', w.novel_id,
               'state', w.state, 'updated_at', w.updated_at
           )
    FROM world_states w
    WHERE w.user_id = $1

    UNION ALL
    SELECT 40, p.novel_id::text, p.chapter_number::bigint, p.id::text, 'player_chapter',
           jsonb_build_object(
               'id', p.id, 'user_id', p.user_id, 'novel_id', p.novel_id,
               'chapter_number', p.chapter_number, 'content', p.content,
               'origin', p.origin, 'created_at', p.created_at, 'updated_at', p.updated_at
           )
    FROM player_chapters p
    WHERE p.user_id = $1
)
SELECT kind, data
FROM export_records
ORDER BY section_order, sort_group, sort_number, sort_id
"#;

#[cfg(test)]
mod tests {
    use super::EXPORT_SQL;

    #[test]
    fn export_is_explicit_and_scopes_shared_nodes_through_the_users_choices() {
        assert!(!EXPORT_SQL.to_ascii_lowercase().contains("select *"));
        assert!(EXPORT_SQL.contains("n.user_id IS NULL"));
        assert!(EXPORT_SQL.contains("c.user_id = $1 AND c.node_id = n.id"));
    }
}
