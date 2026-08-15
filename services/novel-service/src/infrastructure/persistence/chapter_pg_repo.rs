use std::collections::HashSet;

use anyhow::{ensure, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::entities::chapter::Chapter;
use crate::domain::repositories::{ChapterRepository, LoreExcerpt};

const CHUNK_CHARS: usize = 1_200;
const CHUNK_OVERLAP_CHARS: usize = 150;
const MIN_LORE_SCORE: f32 = 0.08;

#[derive(Debug, FromRow)]
struct ChapterRow {
    id: Uuid,
    novel_id: Uuid,
    chapter_number: i32,
    title: Option<String>,
    content: String,
    summary: Option<String>,
    is_key_node: bool,
    key_node_description: Option<String>,
    #[allow(dead_code)]
    word_count: i32,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct LoreExcerptRow {
    chapter_number: i32,
    title: Option<String>,
    content: String,
    score: f32,
}

impl From<LoreExcerptRow> for LoreExcerpt {
    fn from(row: LoreExcerptRow) -> Self {
        Self {
            chapter_number: row.chapter_number,
            title: row.title,
            content: row.content,
            score: row.score,
        }
    }
}

fn split_chapter_content(content: &str) -> Vec<String> {
    let chars: Vec<char> = content.chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < chars.len() {
        let end = (start + CHUNK_CHARS).min(chars.len());
        let chunk: String = chars[start..end].iter().collect();
        let chunk = chunk.trim();
        if !chunk.is_empty() {
            chunks.push(chunk.to_owned());
        }
        if end == chars.len() {
            break;
        }
        start = end - CHUNK_OVERLAP_CHARS;
    }

    chunks
}

fn lore_search_terms(query: &str) -> Vec<String> {
    const MAX_TERMS: usize = 32;
    let mut terms = Vec::new();

    for token in query.split_whitespace() {
        let cleaned: String = token
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect();
        if cleaned.chars().count() < 2 {
            continue;
        }
        let normalized = cleaned.to_lowercase();
        if !terms.contains(&normalized) {
            terms.push(normalized);
        }

        if !cleaned.is_ascii() {
            let chars: Vec<char> = cleaned.chars().collect();
            for pair in chars.windows(2) {
                let term: String = pair.iter().collect();
                if !terms.contains(&term) {
                    terms.push(term);
                }
                if terms.len() == MAX_TERMS {
                    return terms;
                }
            }
        }
        if terms.len() == MAX_TERMS {
            break;
        }
    }

    terms
}

async fn replace_chapter_chunks(
    tx: &mut Transaction<'_, Postgres>,
    chapter_id: Uuid,
    content: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM chapter_chunks WHERE chapter_id = $1")
        .bind(chapter_id)
        .execute(&mut **tx)
        .await?;

    for (chunk_index, content) in split_chapter_content(content).into_iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO chapter_chunks (
                id, chapter_id, chunk_index, content
            ) VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(chapter_id)
        .bind(chunk_index as i32)
        .bind(content)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

impl From<ChapterRow> for Chapter {
    fn from(r: ChapterRow) -> Self {
        Chapter {
            id: r.id,
            novel_id: r.novel_id,
            chapter_number: r.chapter_number,
            title: r.title,
            content: r.content,
            summary: r.summary,
            is_key_node: r.is_key_node,
            key_node_description: r.key_node_description,
            created_at: r.created_at,
        }
    }
}

pub struct ChapterPgRepository {
    pool: PgPool,
}

pub(crate) async fn save_batch_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    chapters: &[Chapter],
) -> Result<()> {
    for chapter in chapters {
        let chapter_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO chapters (
                id, novel_id, chapter_number, title, content,
                summary, is_key_node, key_node_description,
                word_count, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id
            "#,
        )
        .bind(chapter.id)
        .bind(chapter.novel_id)
        .bind(chapter.chapter_number)
        .bind(&chapter.title)
        .bind(&chapter.content)
        .bind(&chapter.summary)
        .bind(chapter.is_key_node)
        .bind(&chapter.key_node_description)
        .bind(chapter.word_count() as i32)
        .bind(chapter.created_at)
        .fetch_one(&mut **tx)
        .await?;

        replace_chapter_chunks(tx, chapter_id, &chapter.content).await?;
    }
    Ok(())
}

impl ChapterPgRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChapterRepository for ChapterPgRepository {
    async fn replace_import_nodes(
        &self,
        novel_id: Uuid,
        attempt: i64,
        nodes: &[(i32, String)],
    ) -> Result<bool> {
        let chapter_numbers = nodes
            .iter()
            .map(|(chapter_number, _)| *chapter_number)
            .collect::<HashSet<_>>();
        ensure!(
            chapter_numbers.len() == nodes.len()
                && nodes.iter().all(|(chapter_number, description)| {
                    *chapter_number >= 1
                        && !description.trim().is_empty()
                        && description.chars().count() <= 1_000
                        && !description.chars().any(char::is_control)
                }),
            "import narrative nodes are invalid"
        );

        let mut transaction = self.pool.begin().await?;
        let fenced = sqlx::query_scalar::<_, bool>(
            "SELECT TRUE FROM novel_import_jobs \
             WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress' \
               AND stage = 'chapters' \
             FOR UPDATE",
        )
        .bind(novel_id)
        .bind(attempt)
        .fetch_optional(&mut *transaction)
        .await?
        .unwrap_or(false);
        if !fenced {
            return Ok(false);
        }

        sqlx::query(
            "UPDATE chapters SET is_key_node = FALSE, key_node_description = NULL \
             WHERE novel_id = $1",
        )
        .bind(novel_id)
        .execute(&mut *transaction)
        .await?;
        for (chapter_number, description) in nodes {
            let result = sqlx::query(
                "UPDATE chapters SET is_key_node = TRUE, key_node_description = $3 \
                 WHERE novel_id = $1 AND chapter_number = $2",
            )
            .bind(novel_id)
            .bind(chapter_number)
            .bind(description)
            .execute(&mut *transaction)
            .await?;
            ensure!(
                result.rows_affected() == 1,
                "import narrative node references a missing chapter"
            );
        }
        transaction.commit().await?;
        Ok(true)
    }

    async fn find_by_novel(&self, novel_id: Uuid) -> Result<Vec<Chapter>> {
        let rows = sqlx::query_as::<_, ChapterRow>(
            "SELECT * FROM chapters WHERE novel_id = $1 ORDER BY chapter_number ASC",
        )
        .bind(novel_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Chapter::from).collect())
    }

    async fn find_by_number(&self, novel_id: Uuid, number: i32) -> Result<Option<Chapter>> {
        let row = sqlx::query_as::<_, ChapterRow>(
            "SELECT * FROM chapters WHERE novel_id = $1 AND chapter_number = $2",
        )
        .bind(novel_id)
        .bind(number)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Chapter::from))
    }

    async fn search_lore(
        &self,
        novel_id: Uuid,
        max_chapter: i32,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LoreExcerpt>> {
        let terms = lore_search_terms(query);
        let rows = sqlx::query_as::<_, LoreExcerptRow>(
            r#"
            WITH ranked AS (
                SELECT
                    chapter.chapter_number,
                    chapter.title,
                    chunk.content,
                    GREATEST(
                        similarity(chunk.content, $3),
                        word_similarity($3, chunk.content),
                        similarity(COALESCE(chapter.title, ''), $3),
                        COALESCE((
                            SELECT COUNT(*)::REAL
                                / LEAST(GREATEST(cardinality($4), 1), 8)::REAL
                            FROM unnest($4::TEXT[]) AS search_term(value)
                            WHERE position(
                                lower(search_term.value) IN lower(COALESCE(chapter.title, '') || ' ' || chunk.content)
                            ) > 0
                        ), 0)
                    )::REAL AS score
                FROM chapter_chunks AS chunk
                JOIN chapters AS chapter ON chapter.id = chunk.chapter_id
                WHERE chapter.novel_id = $1
                  AND chapter.chapter_number BETWEEN 1 AND $2
            )
            SELECT chapter_number, title, content, score
            FROM ranked
            WHERE score >= $5
            ORDER BY score DESC, chapter_number DESC
            LIMIT $6
            "#,
        )
        .bind(novel_id)
        .bind(max_chapter)
        .bind(query)
        .bind(terms)
        .bind(MIN_LORE_SCORE)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(LoreExcerpt::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_chunks_are_unicode_safe_and_overlap() {
        let content = "界".repeat(CHUNK_CHARS + 10);
        let chunks = split_chapter_content(&content);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), CHUNK_CHARS);
        assert_eq!(chunks[1].chars().count(), CHUNK_OVERLAP_CHARS + 10);
        assert_eq!(
            chunks[0]
                .chars()
                .skip(CHUNK_CHARS - CHUNK_OVERLAP_CHARS)
                .collect::<String>(),
            chunks[1]
                .chars()
                .take(CHUNK_OVERLAP_CHARS)
                .collect::<String>()
        );
    }

    #[test]
    fn chinese_lore_queries_keep_entity_bigrams() {
        let terms = lore_search_terms("蛇怪在哪里？");

        assert!(terms.iter().any(|term| term == "蛇怪"));
        assert!(!terms.iter().any(|term| term.contains('？')));
    }
}
