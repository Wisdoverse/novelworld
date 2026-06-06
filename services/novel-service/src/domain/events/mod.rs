use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum NovelEvent {
    Created {
        novel_id: Uuid,
        user_id: Uuid,
    },
    ParseCompleted {
        novel_id: Uuid,
        total_chapters: i32,
    },
    ParseFailed {
        novel_id: Uuid,
        error: String,
    },
    CharactersExtracted {
        novel_id: Uuid,
        character_count: usize,
    },
    AvatarGenerationRequested {
        character_id: Uuid,
        appearance: String,
    },
}
