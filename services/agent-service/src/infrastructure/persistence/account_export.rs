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
WITH export_records AS (
    SELECT 10 AS section_order, created_at AS sort_time, id::text AS sort_id,
           'chat_message'::text AS kind,
           jsonb_build_object(
               'id', id, 'user_id', user_id, 'character_id', character_id,
               'novel_id', novel_id, 'role', role, 'content', content,
               'reader_identity', reader_identity,
               'chapter_context', chapter_context, 'created_at', created_at
           ) AS data
    FROM chat_messages
    WHERE user_id = $1

    UNION ALL
    SELECT 20, created_at, id::text, 'character_memory',
           jsonb_build_object(
               'id', id, 'user_id', user_id, 'character_id', character_id,
               'novel_id', novel_id, 'layer', layer::text, 'content', content,
               'importance', importance, 'chapter_number', chapter_number,
               'created_at', created_at
           )
    FROM character_memories
    WHERE user_id = $1
)
SELECT kind, data
FROM export_records
ORDER BY section_order, sort_time, sort_id
"#;

#[cfg(test)]
mod tests {
    use super::EXPORT_SQL;

    #[test]
    fn export_is_explicit_and_omits_operational_and_secret_material() {
        assert!(!EXPORT_SQL.to_ascii_lowercase().contains("select *"));
        for excluded in [
            "chat_turns",
            "request_fingerprint",
            "lease_expires_at",
            "failure_code",
            "embedding",
            "access_count",
            "last_accessed",
            "expires_at",
        ] {
            assert!(!EXPORT_SQL.contains(excluded));
        }
    }
}
