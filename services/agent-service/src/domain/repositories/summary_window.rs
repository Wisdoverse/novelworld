use anyhow::{ensure, Result};
use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::entities::memory::{ChatMessage, Memory};

pub const SUMMARY_WINDOW_TURNS: i64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryWindow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub character_id: Uuid,
    pub novel_id: Uuid,
    pub sequence: i64,
    pub memory_id: Uuid,
    pub attempt: i64,
}

#[derive(Debug, Clone)]
pub struct SummarySource {
    pub sequence: i64,
    pub turn_id: Uuid,
    pub chapter: i32,
    pub reader_identity: Option<String>,
    pub message: ChatMessage,
}

impl SummaryWindow {
    pub fn memory(&self, text: String, sources: &[SummarySource]) -> Result<Memory> {
        ensure!(valid_summary_output(&text), "invalid summary output");
        let (chapter, persona) = self.validate_sources(sources)?;
        Ok(Memory {
            id: self.memory_id,
            user_id: self.user_id,
            novel_id: self.novel_id,
            character_id: self.character_id,
            layer: crate::domain::entities::memory::MemoryLayer::Mid,
            content: text,
            importance: 6,
            chapter_number: Some(chapter),
            persona_source_chapter_high_water: Some(persona),
            embedding: None,
            created_at: chrono::Utc::now(),
        })
    }

    /// Sources arrive in immutable sequence order, user then character.
    /// Repository selection also requires completed self claims in this scope.
    pub fn validate_sources(&self, sources: &[SummarySource]) -> Result<(i32, i32)> {
        ensure!(self.sequence >= SUMMARY_WINDOW_TURNS && self.sequence % SUMMARY_WINDOW_TURNS == 0);
        ensure!(sources.len() == (SUMMARY_WINDOW_TURNS * 2) as usize);
        let mut turns = std::collections::HashSet::new();
        let mut messages = std::collections::HashSet::new();
        let mut chapter = 0;
        let mut persona = 0;
        for (index, pair) in sources.as_chunks::<2>().0.iter().enumerate() {
            let expected = self.sequence - SUMMARY_WINDOW_TURNS + 1 + index as i64;
            ensure!(turns.insert(pair[0].turn_id));
            ensure!(pair[0].turn_id == pair[1].turn_id);
            ensure!(pair[0].chapter == pair[1].chapter);
            ensure!(pair[0].reader_identity == pair[1].reader_identity);
            for (source, role) in pair.iter().zip(["user", "character"]) {
                let message = &source.message;
                ensure!(source.sequence == expected && !source.turn_id.is_nil());
                ensure!(message.turn_id == Some(source.turn_id) && message.role == role);
                ensure!(!message.id.is_nil() && messages.insert(message.id));
                ensure!(
                    message.user_id == self.user_id
                        && message.novel_id == self.novel_id
                        && message.character_id == self.character_id
                );
                ensure!(source.chapter >= 1 && message.chapter_context == Some(source.chapter));
                ensure!(message.reader_identity == source.reader_identity);
                let marker = message
                    .persona_source_chapter_high_water
                    .filter(|marker| (1..=source.chapter).contains(marker))
                    .ok_or_else(|| anyhow::anyhow!("unproven summary source"))?;
                chapter = chapter.max(source.chapter);
                persona = persona.max(marker);
            }
        }
        ensure!(sources
            .last()
            .is_some_and(|source| source.turn_id == self.id));
        Ok((chapter, persona))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SummaryOutcome {
    SourceInvalid,
    OutputInvalid,
    EligibilityChanged,
    DispatchUnknown,
}

impl SummaryOutcome {
    pub fn to_str(self) -> &'static str {
        match self {
            Self::SourceInvalid => "source_invalid",
            Self::OutputInvalid => "output_invalid",
            Self::EligibilityChanged => "eligibility_changed",
            Self::DispatchUnknown => "dispatch_unknown",
        }
    }
}

#[async_trait]
pub trait SummaryWindowRepository: Send + Sync {
    async fn due_windows(&self) -> Result<Vec<SummaryWindow>>;
    async fn claim_window(&self, window: &SummaryWindow) -> Result<Option<SummaryWindow>>;
    async fn defer_window(&self, window: &SummaryWindow, claimed: bool) -> Result<bool>;
    async fn summary_sources(&self, window: &SummaryWindow) -> Result<Vec<SummarySource>>;
    async fn start_summary_dispatch(&self, window: &SummaryWindow) -> Result<bool>;
    async fn finish_summary(&self, window: &SummaryWindow, memory: &Memory) -> Result<bool>;
    async fn fail_summary(&self, window: &SummaryWindow, outcome: SummaryOutcome) -> Result<bool>;
}

pub fn valid_summary_output(text: &str) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= crate::domain::services::memory_manager::MAX_MEMORY_BLOCK_CHARS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (SummaryWindow, Vec<SummarySource>) {
        let window = SummaryWindow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            character_id: Uuid::new_v4(),
            novel_id: Uuid::new_v4(),
            sequence: 20,
            memory_id: Uuid::new_v4(),
            attempt: 1,
        };
        let mut sources = Vec::new();
        for sequence in 11..=20 {
            let turn = if sequence == 20 {
                window.id
            } else {
                Uuid::new_v4()
            };
            for role in ["user", "character"] {
                let chapter = if sequence == 11 { 3 } else { 2 };
                let mut message = ChatMessage::new(
                    window.user_id,
                    window.character_id,
                    window.novel_id,
                    role.into(),
                    format!("source-{sequence}-{role}"),
                    Some("Reader".into()),
                    Some(chapter),
                )
                .with_turn_id(turn);
                message.persona_source_chapter_high_water =
                    Some(if sequence == 11 { 3 } else { 1 });
                sources.push(SummarySource {
                    sequence,
                    turn_id: turn,
                    chapter,
                    reader_identity: Some("Reader".into()),
                    message,
                });
            }
        }
        (window, sources)
    }

    #[test]
    fn exact_window_preserves_max_chapter_and_rejects_source_or_output_drift() {
        let (window, sources) = fixture();
        let memory = window.memory("proven summary".into(), &sources).unwrap();
        assert_eq!(memory.id, window.memory_id);
        assert_eq!(memory.chapter_number, Some(3));
        assert_eq!(memory.persona_source_chapter_high_water, Some(3));
        assert!(window.validate_sources(&sources[..18]).is_err());
        for mutation in 0..9 {
            let mut invalid = sources.clone();
            match mutation {
                0 => invalid[0].sequence = 12,
                1 => invalid[0].message.user_id = Uuid::new_v4(),
                2 => invalid[0].message.character_id = Uuid::new_v4(),
                3 => invalid[0].message.novel_id = Uuid::new_v4(),
                4 => invalid[0].message.role = "system".into(),
                5 => invalid[0].message.chapter_context = Some(1),
                6 => invalid[0].message.persona_source_chapter_high_water = None,
                7 => invalid[0].message.persona_source_chapter_high_water = Some(4),
                _ => invalid[0].message.turn_id = Some(Uuid::new_v4()),
            }
            assert!(
                window.validate_sources(&invalid).is_err(),
                "mutation {mutation}"
            );
        }
        assert!(valid_summary_output(&"字".repeat(4000)));
        assert!(!valid_summary_output(&"字".repeat(4001)));
        assert!(!valid_summary_output(" \n\t"));
        assert!(window.memory("".into(), &sources).is_err());
    }
}
