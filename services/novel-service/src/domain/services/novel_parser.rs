use anyhow::Result;
use regex::Regex;

use crate::domain::entities::chapter::Chapter;
use uuid::Uuid;

/// Novel parsing domain service.
/// Splits raw text into chapters.
pub struct NovelParserService;

impl NovelParserService {
    /// Detect and split chapters automatically.
    pub fn parse_chapters(novel_id: Uuid, raw_text: &str) -> Result<Vec<Chapter>> {
        // Try several chapter-boundary patterns.
        let patterns = [
            r"(?m)^第[零一二三四五六七八九十百千\d]+[章节回部集卷篇][^\n]*$",
            r"(?im)^[ \t\u{3000}]*-?[ \t\u{3000}]*chapter[ \t\u{3000}]+(?:\d+(?:[ \t\u{3000}]+[^\r\n]{1,100})?|[a-z](?:[ \t\u{3000}]*[a-z]){1,24})(?:[ \t\u{3000}]*[-:][ \t\u{3000}]*[^\r\n]{0,100})?[ \t\u{3000}]*-?[ \t\u{3000}]*\r?$",
            r"(?m)^\d+\.[^\n]{0,50}$",
            r"(?m)^【[^】]+】$",
        ];

        for pattern in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                let splits: Vec<_> = re.find_iter(raw_text).collect();
                if splits.len() >= 2 {
                    return Ok(Self::split_by_matches(novel_id, raw_text, &splits));
                }
            }
        }

        // If no chapter structure is detected, split into 3,000-character chunks.
        Ok(Self::split_by_length(novel_id, raw_text, 3000))
    }

    fn split_by_matches(novel_id: Uuid, text: &str, matches: &[regex::Match]) -> Vec<Chapter> {
        let mut chapters = Vec::new();
        for (i, m) in matches.iter().enumerate() {
            let start = m.start();
            let end = if i + 1 < matches.len() {
                matches[i + 1].start()
            } else {
                text.len()
            };
            let title = m.as_str().trim().to_string();
            let content = text[start..end].trim().to_string();
            if !content.is_empty() {
                // Number chapters in retained order. Skipped table-of-contents pages must not
                // leave gaps, or the import will reject the novel as discontinuous.
                let ch = Chapter::new(novel_id, (chapters.len() + 1) as i32, Some(title), content);
                // Skip chapters shorter than 100 characters; they may be table-of-contents entries.
                if ch.word_count() > 100 {
                    chapters.push(ch);
                }
            }
        }
        chapters
    }

    fn split_by_length(novel_id: Uuid, text: &str, chunk_size: usize) -> Vec<Chapter> {
        let chars: Vec<char> = text.chars().collect();
        let mut chapters = Vec::new();
        let mut i = 0;
        let mut chapter_num = 1;
        while i < chars.len() {
            let end = (i + chunk_size).min(chars.len());
            let content: String = chars[i..end].iter().collect();
            chapters.push(Chapter::new(
                novel_id,
                chapter_num,
                Some(format!("第{}章", chapter_num)),
                content,
            ));
            i = end;
            chapter_num += 1;
        }
        chapters
    }
}
