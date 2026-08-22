use serde::Deserialize;
use thiserror::Error;

use crate::domain::entities::chapter::Chapter;

const MIN_REPAIR_CHARS: usize = 60_000;
const MAX_REPAIR_CHARS: usize = 220_000;
const MAX_BOUNDARIES: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterBoundaryDetection {
    pub boundaries: Vec<ChapterBoundary>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterBoundary {
    pub line: usize,
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct ChapterBoundaryError(String);

pub fn suspicious_chapter_indexes(chapters: &[Chapter]) -> Vec<usize> {
    if chapters.len() < 4 {
        return Vec::new();
    }
    let mut sizes = chapters
        .iter()
        .map(|chapter| chapter.content.chars().count())
        .collect::<Vec<_>>();
    sizes.sort_unstable();
    let median = sizes[sizes.len() / 2];
    let threshold = MIN_REPAIR_CHARS.max(median.saturating_mul(2));
    chapters
        .iter()
        .enumerate()
        .filter_map(|(index, chapter)| {
            (chapter.content.chars().count() > threshold).then_some(index)
        })
        .collect()
}

pub fn expected_boundary_count(chapters: &[Chapter], suspicious_index: usize) -> usize {
    let suspicious = suspicious_chapter_indexes(chapters);
    let typical_sizes = chapters
        .iter()
        .enumerate()
        .filter(|(index, _)| !suspicious.contains(index))
        .map(|(_, chapter)| chapter.content.chars().count())
        .collect::<Vec<_>>();
    let typical_size = typical_sizes
        .iter()
        .copied()
        .sum::<usize>()
        .checked_div(typical_sizes.len())
        .unwrap_or(MIN_REPAIR_CHARS);
    let source_size = chapters[suspicious_index].content.chars().count();
    let expected_parts = source_size
        .saturating_add(typical_size / 2)
        .checked_div(typical_size.max(1))
        .unwrap_or(2)
        .clamp(2, MAX_BOUNDARIES + 1);
    expected_parts - 1
}

pub fn build_prompt(
    chapter: &Chapter,
    expected_boundaries: usize,
) -> Result<String, ChapterBoundaryError> {
    let source_chars = chapter.content.chars().count();
    if source_chars > MAX_REPAIR_CHARS {
        return Err(ChapterBoundaryError(format!(
            "oversized chapter repair source exceeds {MAX_REPAIR_CHARS} characters"
        )));
    }
    let numbered_source = numbered_source_lines(&chapter.content);
    Ok(format!(
        r#"You repair chapter boundaries in damaged plain-text novels.

SOURCE begins at an already known chapter boundary. It may contain later chapters whose heading text was lost during PDF/OCR/TXT conversion.

Return JSON only:
{{"boundaries":[{{"line":123}}]}}

Rules:
- Exclude the first/source chapter; return only the start of each later chapter.
- Return exactly {expected_boundaries} boundaries: the surrounding book's reliable chapters indicate that SOURCE contains {} chapters.
- `line` is the displayed L-number of the first prose line of a later chapter.
- Report only real chapter transitions, not page headers, page numbers, scene changes, letters, quotations, or all-caps dialogue.
- Preserve source order. Return line numbers only; never copy SOURCE text.

SOURCE:
<<<SOURCE_START>>>
{}
<<<SOURCE_END>>>"#,
        expected_boundaries + 1,
        numbered_source,
    ))
}

fn source_line_starts(source: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut offset = 0usize;
    for line in source.split_inclusive('\n') {
        if !line.trim().is_empty() {
            starts.push(offset);
        }
        offset += line.len();
    }
    starts
}

fn numbered_source_lines(source: &str) -> String {
    let mut numbered = String::with_capacity(source.len());
    let mut line_number = 0usize;
    for line in source.split_inclusive('\n') {
        if line.trim().is_empty() {
            continue;
        }
        line_number += 1;
        numbered.push_str(&format!("[L{line_number}] {line}"));
        if !line.ends_with('\n') {
            numbered.push('\n');
        }
    }
    numbered
}

pub fn validate_detection(
    detection: &ChapterBoundaryDetection,
    source: &str,
    expected_boundaries: usize,
) -> Result<(), ChapterBoundaryError> {
    if expected_boundaries == 0
        || expected_boundaries > MAX_BOUNDARIES
        || detection.boundaries.len() != expected_boundaries
    {
        return Err(ChapterBoundaryError(format!(
            "expected exactly {expected_boundaries} missing chapter boundaries"
        )));
    }
    let line_starts = source_line_starts(source);
    let mut starts = vec![0usize];
    for boundary in &detection.boundaries {
        let Some(&position) = boundary
            .line
            .checked_sub(1)
            .and_then(|index| line_starts.get(index))
        else {
            return Err(ChapterBoundaryError(
                "boundary line must identify a displayed non-empty source line".into(),
            ));
        };
        if position <= *starts.last().expect("source start exists") {
            return Err(ChapterBoundaryError(
                "boundary lines must be ordered after the source chapter start".into(),
            ));
        }
        starts.push(position);
    }
    starts.push(source.len());
    if starts
        .windows(2)
        .any(|range| source[range[0]..range[1]].trim().chars().count() <= 100)
    {
        return Err(ChapterBoundaryError(
            "detected chapter content is too short".into(),
        ));
    }
    Ok(())
}

pub fn split_chapter(
    chapter: &Chapter,
    detection: &ChapterBoundaryDetection,
    expected_boundaries: usize,
) -> Result<Vec<Chapter>, ChapterBoundaryError> {
    validate_detection(detection, &chapter.content, expected_boundaries)?;
    let line_starts = source_line_starts(&chapter.content);
    let mut starts = vec![0usize];
    starts.extend(
        detection
            .boundaries
            .iter()
            .map(|boundary| line_starts[boundary.line - 1]),
    );
    starts.push(chapter.content.len());
    Ok(starts
        .windows(2)
        .enumerate()
        .map(|(index, range)| {
            Chapter::new(
                chapter.novel_id,
                0,
                (index == 0).then(|| chapter.title.clone()).flatten(),
                chapter.content[range[0]..range[1]].trim().to_string(),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn chapter(content: String) -> Chapter {
        Chapter::new(Uuid::nil(), 1, Some("Chapter One".into()), content)
    }

    #[test]
    fn length_outliers_trigger_repair_without_book_specific_rules() {
        let chapters = vec![
            chapter("a".repeat(30_000)),
            chapter("b".repeat(31_000)),
            chapter("c".repeat(29_000)),
            chapter("d".repeat(79_000)),
            chapter("e".repeat(199_000)),
        ];
        assert_eq!(suspicious_chapter_indexes(&chapters), vec![3, 4]);
        assert_eq!(expected_boundary_count(&chapters, 3), 2);
        assert_eq!(expected_boundary_count(&chapters, 4), 6);
    }

    #[test]
    fn source_bound_lines_split_in_order() {
        let anchor =
            "A new morning began beside the silent river, and nobody remembered the warning.";
        let source = format!(
            "CHAPTER ONE\n{}\n{anchor}{}",
            "The first journey continued through the night. ".repeat(8),
            " The second journey crossed the distant hills.".repeat(8)
        );
        let detection = ChapterBoundaryDetection {
            boundaries: vec![ChapterBoundary { line: 3 }],
        };
        let split = split_chapter(&chapter(source), &detection, 1).unwrap();
        assert_eq!(split.len(), 2);
        assert!(split[1].content.starts_with(anchor));
        assert!(split[0].title.is_some());
        assert!(split[1].title.is_none());
    }

    #[test]
    fn empty_detection_is_rejected_for_a_suspicious_segment() {
        let source = "A single long chapter. ".repeat(20);
        let detection = ChapterBoundaryDetection {
            boundaries: Vec::new(),
        };
        assert!(split_chapter(&chapter(source), &detection, 1).is_err());
    }

    #[test]
    fn numbered_source_line_is_mapped_back_to_original_content() {
        let source = format!(
            "{}\nOWL POST\n{}",
            "Before the boundary. ".repeat(12),
            "After the boundary. ".repeat(12)
        );
        let detection = ChapterBoundaryDetection {
            boundaries: vec![ChapterBoundary { line: 2 }],
        };
        let split = split_chapter(&chapter(source), &detection, 1).unwrap();
        assert_eq!(split.len(), 2);
        assert!(split[1].content.starts_with("OWL POST"));
    }

    #[test]
    fn nonexistent_and_unordered_lines_are_rejected() {
        let source = format!(
            "{}\n{}",
            "Repeated boundary text is deliberately long enough for validation.".repeat(3),
            "Repeated boundary text is deliberately long enough for validation.".repeat(3)
        );
        let nonexistent = ChapterBoundaryDetection {
            boundaries: vec![ChapterBoundary { line: 99 }],
        };
        assert!(validate_detection(&nonexistent, &source, 1).is_err());

        let unordered = ChapterBoundaryDetection {
            boundaries: vec![ChapterBoundary { line: 2 }, ChapterBoundary { line: 1 }],
        };
        assert!(validate_detection(&unordered, &source, 2).is_err());
    }
}
