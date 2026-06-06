use anyhow::Result;
use regex::Regex;

use crate::domain::entities::chapter::Chapter;
use uuid::Uuid;

/// 小说解析领域服务
/// 职责：将原始文本拆分为章节列表
pub struct NovelParserService;

impl NovelParserService {
    /// 自动检测并拆分章节
    pub fn parse_chapters(novel_id: Uuid, raw_text: &str) -> Result<Vec<Chapter>> {
        // 尝试多种章节分隔模式
        let patterns = [
            r"(?m)^第[零一二三四五六七八九十百千\d]+[章节回部集卷篇][^\n]*$",
            r"(?m)^Chapter\s+\d+[^\n]*$",
            r"(?m)^CHAPTER\s+\d+[^\n]*$",
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

        // 无法识别章节结构，按字数切分（每 3000 字一章）
        Ok(Self::split_by_length(novel_id, raw_text, 3000))
    }

    fn split_by_matches(
        novel_id: Uuid,
        text: &str,
        matches: &[regex::Match],
    ) -> Vec<Chapter> {
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
                let ch = Chapter::new(novel_id, (i + 1) as i32, Some(title), content);
                // 章节内容过短（< 100字）可能是目录，跳过
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
