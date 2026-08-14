use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 章节实体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: Uuid,
    pub novel_id: Uuid,
    pub chapter_number: i32,
    pub title: Option<String>,
    pub content: String,
    pub summary: Option<String>,
    /// 是否为关键分支节点（借鉴 CharMem 防剧透 RAG）
    pub is_key_node: bool,
    pub key_node_description: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Chapter {
    pub fn new(
        novel_id: Uuid,
        chapter_number: i32,
        title: Option<String>,
        content: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            novel_id,
            chapter_number,
            title,
            content,
            summary: None,
            is_key_node: false,
            key_node_description: None,
            created_at: Utc::now(),
        }
    }

    pub fn set_summary(&mut self, summary: String) {
        self.summary = Some(summary);
    }

    pub fn mark_as_key_node(&mut self, description: String) {
        self.is_key_node = true;
        self.key_node_description = Some(description);
    }

    /// 字数统计
    pub fn word_count(&self) -> usize {
        self.content.chars().count()
    }
}

/// A durable import may only accept or resume chapters that are numbered
/// 1..N in order and carry readable content. Gapped or blank chapters cannot
/// be published: enrichment derives `total_chapters` from the row count, so a
/// hole would silently truncate or misnumber the novel.
pub fn chapters_are_importable(chapters: &[Chapter]) -> bool {
    !chapters.is_empty()
        && chapters.iter().enumerate().all(|(index, chapter)| {
            usize::try_from(chapter.chapter_number).ok() == Some(index + 1)
                && !chapter.content.trim().is_empty()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(number: i32, content: &str) -> Chapter {
        Chapter::new(Uuid::nil(), number, None, content.into())
    }

    #[test]
    fn only_contiguous_readable_chapters_are_importable() {
        assert!(chapters_are_importable(&[
            chapter(1, "first"),
            chapter(2, "second")
        ]));
        assert!(!chapters_are_importable(&[]));
        assert!(!chapters_are_importable(&[
            chapter(1, "first"),
            chapter(3, "third")
        ]));
        assert!(!chapters_are_importable(&[
            chapter(2, "second"),
            chapter(1, "first")
        ]));
        assert!(!chapters_are_importable(&[
            chapter(1, "first"),
            chapter(2, " \n ")
        ]));
        assert!(!chapters_are_importable(&[chapter(0, "zero")]));
    }
}
