#[cfg(test)]
mod tests {
    use crate::domain::entities::memory::{Memory, MemoryLayer, ChatMessage};
    use uuid::Uuid;

    #[test]
    fn test_memory_layer_creation() {
        let user_id = Uuid::new_v4();
        let char_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();

        let short = Memory::new_short(char_id, user_id, novel_id, "Hello".into(), Some(1));
        assert!(matches!(short.layer, MemoryLayer::Short));
        assert_eq!(short.importance, 5);

        let perm = Memory::new_permanent(char_id, user_id, novel_id, "Critical event".into(), 10);
        assert!(matches!(perm.layer, MemoryLayer::Permanent));
        assert_eq!(perm.importance, 10);
    }

    #[test]
    fn test_chat_message_creation() {
        let msg = ChatMessage::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "user".into(),
            "Hello character".into(),
            Some("Reader".into()),
            Some(3),
        );
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "Hello character");
        assert_eq!(msg.chapter_context, Some(3));
        assert!(msg.reader_identity.is_some());
    }

    #[test]
    fn test_memory_ordering_permanent_first() {
        let user_id = Uuid::new_v4();
        let char_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();

        let mut memories = vec![
            Memory::new_short(char_id, user_id, novel_id, "short1".into(), Some(1)),
            Memory::new_permanent(char_id, user_id, novel_id, "perm1".into(), 10),
            Memory::new_short(char_id, user_id, novel_id, "short2".into(), Some(2)),
        ];

        memories.sort_by(|a, b| {
            let layer_order = |l: &MemoryLayer| match l {
                MemoryLayer::Permanent => 0,
                MemoryLayer::Long => 1,
                MemoryLayer::Mid => 2,
                MemoryLayer::Short => 3,
            };
            layer_order(&a.layer).cmp(&layer_order(&b.layer))
        });

        assert!(matches!(memories[0].layer, MemoryLayer::Permanent));
        assert!(matches!(memories[1].layer, MemoryLayer::Short));
    }

    #[test]
    fn test_anti_spoiler_chapter_filter() {
        let user_id = Uuid::new_v4();
        let char_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();

        let memories = vec![
            Memory::new_short(char_id, user_id, novel_id, "chapter 1 event".into(), Some(1)),
            Memory::new_short(char_id, user_id, novel_id, "chapter 3 event".into(), Some(3)),
            Memory::new_short(char_id, user_id, novel_id, "chapter 5 event".into(), Some(5)),
            Memory::new_short(char_id, user_id, novel_id, "chapter 10 event".into(), Some(10)),
        ];

        let current_chapter = 5;
        let filtered: Vec<_> = memories
            .into_iter()
            .filter(|m| m.chapter_number.map_or(true, |ch| ch <= current_chapter))
            .collect();

        assert_eq!(filtered.len(), 3);
        assert!(filtered.iter().all(|m| m.chapter_number.map_or(true, |ch| ch <= 5)));
    }
}
