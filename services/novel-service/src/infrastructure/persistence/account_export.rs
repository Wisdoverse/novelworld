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

// One read-only statement gives this service one PostgreSQL statement snapshot
// while retaining backpressure instead of collecting rows in application memory.
const EXPORT_SQL: &str = r#"
WITH shelf_novels AS (
    SELECT id, user_id, title, author, cover_url, description, world_summary, genre,
           total_chapters, status, parse_error, deviation_mode, created_at, updated_at
    FROM novels AS n
    WHERE EXISTS (
        SELECT 1 FROM user_novels AS shelf
        WHERE shelf.user_id = $1 AND shelf.novel_id = n.id
    )
), export_records AS (
    SELECT 10 AS section_order, n.id::text AS sort_group, 0::bigint AS sort_number,
           n.id::text AS sort_id, 'novel'::text AS kind,
           jsonb_build_object(
               'id', n.id, 'user_id', $1, 'title', n.title, 'author', n.author,
               'cover_url', n.cover_url, 'description', n.description,
               'world_summary', n.world_summary, 'genre', n.genre,
               'total_chapters', n.total_chapters, 'status', n.status::text,
               'parse_error', n.parse_error,
               'deviation_mode', n.deviation_mode::text,
               'created_at', n.created_at, 'updated_at', n.updated_at
           ) AS data
    FROM shelf_novels n

    UNION ALL
    SELECT 20, c.novel_id::text, c.chapter_number::bigint, c.id::text, 'chapter',
           jsonb_build_object(
               'id', c.id, 'novel_id', c.novel_id, 'chapter_number', c.chapter_number,
               'title', c.title, 'content', c.content, 'summary', c.summary,
               'is_key_node', c.is_key_node,
               'key_node_description', c.key_node_description,
               'word_count', c.word_count, 'created_at', c.created_at
           )
    FROM chapters c
    JOIN shelf_novels n ON n.id = c.novel_id

    UNION ALL
    SELECT 30, c.novel_id::text, 0::bigint, c.id::text, 'character',
           jsonb_build_object(
               'id', c.id, 'novel_id', c.novel_id, 'name', c.name,
               'aliases', c.aliases, 'role', c.role::text,
               'description', c.description, 'personality', c.personality,
               'background', c.background, 'speaking_style', c.speaking_style,
               'appearance', c.appearance, 'avatar_url', c.avatar_url,
               'avatar_status', c.avatar_status::text,
               'first_appearance_chapter', c.first_appearance_chapter,
               'traits', c.traits, 'created_at', c.created_at, 'updated_at', c.updated_at
           )
    FROM characters c
    JOIN shelf_novels n ON n.id = c.novel_id

    UNION ALL
    SELECT 40, r.novel_id::text, 0::bigint, r.id::text, 'character_relationship',
           jsonb_build_object(
               'id', r.id, 'novel_id', r.novel_id,
               'from_character_id', r.from_character_id,
               'to_character_id', r.to_character_id,
               'relationship_type', r.relationship_type,
               'description', r.description, 'strength', r.strength,
               'created_at', r.created_at
           )
    FROM character_relationships r
    JOIN shelf_novels n ON n.id = r.novel_id

    UNION ALL
    SELECT 50, m.novel_id::text, m.model_version::bigint, m.id::text,
           'canon_story_model',
           jsonb_build_object(
               'id', m.id, 'novel_id', m.novel_id,
               'model_version', m.model_version, 'schema_version', m.schema_version,
               'prompt_version', m.prompt_version, 'content', m.content,
               'created_at', m.created_at,
               'source', CASE WHEN jsonb_path_exists(
                   m.content, '$.** ? (exists(@.confidence) && @.confidence < 1.0)'
               ) OR NOT jsonb_path_exists(
                   m.content, '$.** ? (exists(@.confidence))'
               ) THEN 'uncertain' ELSE 'canon' END
           )
    FROM canon_story_models m
    JOIN shelf_novels n ON n.id = m.novel_id

    UNION ALL
    SELECT 60, p.novel_id::text, 0::bigint, p.id::text, 'reading_progress',
           jsonb_build_object(
               'id', p.id, 'user_id', p.user_id, 'novel_id', p.novel_id,
               'current_chapter', p.current_chapter,
               'reader_identity', p.reader_identity,
               'reader_identity_type', p.reader_identity_type::text,
               'reader_character_id', p.reader_character_id,
               'deviation_mode', p.deviation_mode::text,
               'last_read_at', p.last_read_at, 'created_at', p.created_at
           )
    FROM reading_progress p
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
    fn export_is_explicit_and_omits_nonportable_columns() {
        assert!(!EXPORT_SQL.to_ascii_lowercase().contains("select *"));
        for excluded in ["original_file_key", "chapter_chunks"] {
            assert!(!EXPORT_SQL.contains(excluded));
        }
    }
}
