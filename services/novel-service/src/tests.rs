#[cfg(test)]
mod tests {
    use crate::domain::services::novel_parser::NovelParserService;
    use crate::domain::entities::novel::Novel;
    use crate::domain::value_objects::{NovelStatus, DeviationMode, CharacterRole};
    use uuid::Uuid;

    #[test]
    fn test_chapter_split_chinese_headers() {
        let text = r#"第一章 少年出山

林枫站在山巅，眺望远方的城市。他知道，属于自己的旅程即将开始。师父说过，山下的世界远比山上复杂。但他不怕。他收拾好行囊，踏上了下山的路。一路上鸟鸣花香，春意盎然。他走了整整一天，终于看到了城门。城门上写着三个大字。

第二章 初入江湖

城里的一切对林枫来说都很新鲜。高楼大厦、车水马龙，与山中的宁静截然不同。他在客栈住了下来，准备明天去武馆报名。客栈掌柜是个和蔼的老人，给了他一碗热汤。林枫喝完汤，感到浑身暖洋洋的。他躺在床上，想着明天的事情，不知不觉就睡着了。

第三章 风云际会

第二天一早，林枫来到了城中最大的武馆。门口排着长队，都是来报名的年轻人。他注意到人群中有一个气质不凡的少女，身穿白色长裙，手持一柄长剑。少女回头看了他一眼，嘴角微微上扬。林枫心想，这江湖果然有趣得很。"#;

        let novel_id = Uuid::new_v4();
        let chapters = NovelParserService::parse_chapters(novel_id, text).unwrap();

        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].chapter_number, 1);
        assert_eq!(chapters[1].chapter_number, 2);
        assert_eq!(chapters[2].chapter_number, 3);
        assert!(chapters[0].title.as_ref().unwrap().contains("少年出山"));
        assert!(chapters[0].content.contains("林枫"));
    }

    #[test]
    fn test_chapter_split_english_headers() {
        let text = r#"Chapter 1 The Beginning

Once upon a time in a land far away, there lived a young hero. He was brave and kind. The village loved him dearly. He spent his days training with the old master who taught him everything he knew about swordsmanship and the ancient arts. Every morning he would wake before dawn and practice his forms.

Chapter 2 The Journey

The hero set out on a grand adventure. He traveled through forests and over mountains. Along the way he met many friends and faced many challenges. Each day brought new surprises and new lessons to learn.

Chapter 3 The Return

After many months the hero finally returned home. He was stronger and wiser. The village celebrated his return with a great feast that lasted three days and three nights."#;

        let novel_id = Uuid::new_v4();
        let chapters = NovelParserService::parse_chapters(novel_id, text).unwrap();

        assert_eq!(chapters.len(), 3);
        assert!(chapters[0].title.as_ref().unwrap().contains("The Beginning"));
    }

    #[test]
    fn test_chapter_split_fallback_by_length() {
        let long_text = "a".repeat(10000);
        let novel_id = Uuid::new_v4();
        let chapters = NovelParserService::parse_chapters(novel_id, &long_text).unwrap();

        assert!(chapters.len() >= 3);
        for ch in &chapters {
            assert!(ch.word_count() <= 3001);
        }
    }

    #[test]
    fn test_novel_status_transitions() {
        let mut novel = Novel::create(Uuid::new_v4(), "Test".into(), None);
        assert!(matches!(novel.status, NovelStatus::Pending));

        novel.start_parsing();
        assert!(matches!(novel.status, NovelStatus::Parsing));

        novel.mark_ready(10, "A fantasy world.".into());
        assert!(matches!(novel.status, NovelStatus::Ready));
        assert_eq!(novel.total_chapters, 10);
        assert!(novel.world_summary.is_some());
    }

    #[test]
    fn test_novel_error_status() {
        let mut novel = Novel::create(Uuid::new_v4(), "Broken".into(), None);
        novel.start_parsing();
        novel.mark_error("Parse failed".into());

        assert!(matches!(novel.status, NovelStatus::Error));
        assert_eq!(novel.parse_error.as_deref(), Some("Parse failed"));
    }

    #[test]
    fn test_deviation_mode() {
        let mut novel = Novel::create(Uuid::new_v4(), "Test".into(), None);
        assert!(matches!(novel.deviation_mode, DeviationMode::Canon));

        novel.set_deviation_mode(DeviationMode::Creative);
        assert!(matches!(novel.deviation_mode, DeviationMode::Creative));
    }

    #[test]
    fn test_character_role_str_roundtrip() {
        for (role, expected) in [
            (CharacterRole::Protagonist, "protagonist"),
            (CharacterRole::Antagonist, "antagonist"),
            (CharacterRole::Supporting, "supporting"),
            (CharacterRole::Minor, "minor"),
        ] {
            assert_eq!(role.to_str(), expected);
            assert_eq!(CharacterRole::from_str(expected), role);
        }
    }

    #[test]
    fn test_chapter_word_count() {
        use crate::domain::entities::chapter::Chapter;

        let ch = Chapter::new(Uuid::new_v4(), 1, Some("Test".into()), "Hello World".into());
        assert_eq!(ch.word_count(), 11);
    }

    #[test]
    fn test_domain_events() {
        let mut novel = Novel::create(Uuid::new_v4(), "Test".into(), None);
        let events = novel.take_events();
        assert_eq!(events.len(), 1);

        novel.mark_ready(5, "summary".into());
        let events = novel.take_events();
        assert_eq!(events.len(), 1);

        let events = novel.take_events();
        assert_eq!(events.len(), 0);
    }
}
