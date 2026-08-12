use agent_service::domain::{
    entities::memory::{ChatMessage, Memory, MemoryLayer},
    repositories::{BeginChatTurn, ChatRepository, ChatTurnClaim, MemoryRepository},
};
use agent_service::infrastructure::persistence::{
    pg_chat_repo::PgChatRepository, pg_memory_repo::PgMemoryRepository,
};
use narrative_service::domain::{
    entities::narrative_node::{NarrativeChoice, NarrativeNode},
    repositories::{ChoiceCommit, NarrativeNodeRepository, UserChoiceRepository},
};
use narrative_service::infrastructure::persistence::pg_narrative_repo::{
    PgNarrativeNodeRepository, PgUserChoiceRepository,
};
use novel_service::domain::repositories::ReadingProgressRepository;
use novel_service::infrastructure::persistence::pg_progress_repo::PgReadingProgressRepository;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

fn db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://test:test@localhost:25432/novelworld_test".into())
}

async fn wait_for_blocked_query(pool: &PgPool, needle: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (\
                    SELECT 1 FROM pg_stat_activity \
                    WHERE datname = current_database() \
                      AND wait_event_type = 'Lock' \
                      AND POSITION($1 IN query) > 0\
                )",
            )
            .bind(needle)
            .fetch_one(pool)
            .await
            .unwrap();
            if blocked {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("repository query did not reach the expected row lock");
}

#[tokio::test]
async fn reading_progress_creation_and_identity_transition_are_atomic() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let future_character_id = Uuid::new_v4();
    let visible_character_id = Uuid::new_v4();
    let unknown_character_id = Uuid::new_v4();
    let invalid_character_id = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("progress-contract-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, deviation_mode) VALUES ($1, $2, $3, 5, 'creative')",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind("Progress contract")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO characters (id, novel_id, name, first_appearance_chapter) VALUES ($1, $2, 'Future', 5), ($3, $2, 'Unknown', NULL), ($4, $2, 'Invalid', 0), ($5, $2, 'Visible', 1)",
    )
    .bind(future_character_id)
    .bind(novel_id)
    .bind(unknown_character_id)
    .bind(invalid_character_id)
    .bind(visible_character_id)
    .execute(&pool)
    .await
    .unwrap();

    let repository = PgReadingProgressRepository::new(pool.clone());
    let (left, right) = tokio::join!(
        repository.get_or_create(user_id, novel_id, "creative"),
        repository.get_or_create(user_id, novel_id, "creative")
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.id, right.id);
    assert_eq!(left.deviation_mode, "creative");
    assert_eq!(
        repository
            .get_or_create(user_id, novel_id, "remix")
            .await
            .unwrap()
            .deviation_mode,
        "creative"
    );
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM reading_progress WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 1);

    repository
        .update_chapter(user_id, novel_id, 5)
        .await
        .unwrap();
    repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Future"),
            Some(future_character_id),
        )
        .await
        .unwrap();
    repository
        .update_chapter(user_id, novel_id, 1)
        .await
        .unwrap();
    let rewound = repository
        .get_or_create(user_id, novel_id, "creative")
        .await
        .unwrap();
    assert_eq!(rewound.current_chapter, 1);
    assert_eq!(rewound.reader_identity_type, "self");
    assert!(rewound.reader_identity.is_none());
    assert!(rewound.reader_character_id.is_none());
    assert!(repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Unknown"),
            Some(unknown_character_id),
        )
        .await
        .is_err());

    repository
        .update_chapter(user_id, novel_id, 5)
        .await
        .unwrap();
    repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Visible"),
            Some(visible_character_id),
        )
        .await
        .unwrap();

    let mut identity_first = pool.begin().await.unwrap();
    sqlx::query(
        "UPDATE reading_progress \
         SET reader_identity_type = 'character', reader_identity = 'Future', reader_character_id = $3 \
         WHERE user_id = $1 AND novel_id = $2 AND current_chapter = 5",
    )
    .bind(user_id)
    .bind(novel_id)
    .bind(future_character_id)
    .execute(&mut *identity_first)
    .await
    .unwrap();
    let rewind_repository = PgReadingProgressRepository::new(pool.clone());
    let rewind =
        tokio::spawn(async move { rewind_repository.update_chapter(user_id, novel_id, 1).await });
    wait_for_blocked_query(&pool, "locked_progress AS MATERIALIZED").await;
    identity_first.commit().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), rewind)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let identity_first_result = repository
        .get_or_create(user_id, novel_id, "creative")
        .await
        .unwrap();
    assert_eq!(identity_first_result.current_chapter, 1);
    assert_eq!(identity_first_result.reader_identity_type, "self");
    assert!(identity_first_result.reader_character_id.is_none());

    repository
        .update_chapter(user_id, novel_id, 5)
        .await
        .unwrap();
    repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Visible"),
            Some(visible_character_id),
        )
        .await
        .unwrap();
    let mut rewind_first = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT id FROM reading_progress \
         WHERE user_id = $1 AND novel_id = $2 FOR UPDATE",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&mut *rewind_first)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE reading_progress SET current_chapter = 1 \
         WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&mut *rewind_first)
    .await
    .unwrap();
    let identity_repository = PgReadingProgressRepository::new(pool.clone());
    let set_future = tokio::spawn(async move {
        identity_repository
            .set_identity(
                user_id,
                novel_id,
                "character",
                Some("Future"),
                Some(future_character_id),
            )
            .await
    });
    wait_for_blocked_query(&pool, "SET reader_identity_type = $3::identity_type").await;
    rewind_first.commit().await.unwrap();
    let set_result = tokio::time::timeout(std::time::Duration::from_secs(2), set_future)
        .await
        .unwrap()
        .unwrap();
    assert!(set_result.is_err());
    let rewind_first_result = repository
        .get_or_create(user_id, novel_id, "creative")
        .await
        .unwrap();
    assert_eq!(rewind_first_result.current_chapter, 1);
    assert_eq!(
        rewind_first_result.reader_identity.as_deref(),
        Some("Visible")
    );
    assert_eq!(
        rewind_first_result.reader_character_id,
        Some(visible_character_id)
    );
    assert!(repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Invalid"),
            Some(invalid_character_id),
        )
        .await
        .is_err());

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn production_repositories_match_fresh_schema() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();

    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let chapter_id = Uuid::new_v4();
    let character_id = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("repository-contract-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO novels (id, user_id, title) VALUES ($1, $2, $3)")
        .bind(novel_id)
        .bind(user_id)
        .bind("Repository contract")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO chapters (id, novel_id, chapter_number, content) VALUES ($1, $2, 1, $3)",
    )
    .bind(chapter_id)
    .bind(novel_id)
    .bind("A chapter long enough for persistence testing.")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO characters (id, novel_id, name) VALUES ($1, $2, $3)")
        .bind(character_id)
        .bind(novel_id)
        .bind("Contract Character")
        .execute(&pool)
        .await
        .unwrap();

    let memory_repo = PgMemoryRepository::new(pool.clone());
    let mut memory = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "The reader kept a promise.".into(),
        8,
        1,
    );
    let embedding = vec![0.1_f32; 1536];
    memory.embedding = Some(embedding.clone());
    memory_repo.save(&memory).await.unwrap();
    let mut future_memory = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "A future revelation.".into(),
        9,
        3,
    );
    future_memory.embedding = Some(embedding.clone());
    memory_repo.save(&future_memory).await.unwrap();
    let mut unscoped_memory = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "Unknown provenance.".into(),
        10,
        1,
    );
    unscoped_memory.chapter_number = None;
    assert!(memory_repo.save(&unscoped_memory).await.is_err());
    sqlx::query(
        "INSERT INTO character_memories (id, character_id, user_id, novel_id, layer, content, importance, chapter_number) VALUES ($1, $2, $3, $4, 'permanent', $5, 10, NULL)",
    )
    .bind(unscoped_memory.id)
    .bind(character_id)
    .bind(user_id)
    .bind(novel_id)
    .bind("Legacy memory without provenance")
    .execute(&pool)
    .await
    .unwrap();
    let memories = memory_repo
        .find_by_layer(
            character_id,
            user_id,
            novel_id,
            MemoryLayer::Permanent,
            1,
            10,
            0,
        )
        .await
        .unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].novel_id, novel_id);
    assert_eq!(
        memory_repo
            .search_similar(character_id, user_id, novel_id, &embedding, 1, 5)
            .await
            .unwrap()
            .len(),
        1
    );

    let chat_repo = PgChatRepository::new(pool.clone());
    let claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        user_id,
        character_id,
        novel_id,
        request_fingerprint: vec![1; 32],
        chapter_context: 1,
        reader_identity: Some("Reader".into()),
        reader_identity_type: "self".into(),
        reader_character_id: None,
        deviation_mode: "canon".into(),
    };
    let attempt = match chat_repo.begin_turn(&claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected turn reservation: {result:?}"),
    };
    let user_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "user".into(),
        "Hello".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(claim.id);
    let character_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "character".into(),
        "Welcome".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(claim.id);
    chat_repo
        .complete_turn(&claim, attempt, &user_message, &character_message)
        .await
        .unwrap();

    let future_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![2; 32],
        chapter_context: 3,
        ..claim.clone()
    };
    let future_attempt = match chat_repo.begin_turn(&future_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected future turn reservation: {result:?}"),
    };
    let future_user_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "user".into(),
        "Ask about the future".into(),
        Some("Reader".into()),
        Some(3),
    )
    .with_turn_id(future_claim.id);
    let future_character_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "character".into(),
        "A future spoiler".into(),
        Some("Reader".into()),
        Some(3),
    )
    .with_turn_id(future_claim.id);
    chat_repo
        .complete_turn(
            &future_claim,
            future_attempt,
            &future_user_message,
            &future_character_message,
        )
        .await
        .unwrap();
    let messages = chat_repo
        .find_recent(character_id, user_id, novel_id, 1, 10)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].reader_identity.as_deref(), Some("Reader"));
    assert_eq!(messages[1].chapter_context, Some(1));
    assert_eq!(
        chat_repo
            .find_by_character_user(character_id, user_id, novel_id, 1, 10, 0)
            .await
            .unwrap()
            .len(),
        2
    );
    let advanced_context = ChatTurnClaim {
        chapter_context: 2,
        deviation_mode: "creative".into(),
        ..claim.clone()
    };
    assert!(matches!(
        chat_repo.begin_turn(&advanced_context).await.unwrap(),
        BeginChatTurn::Completed {
            claim: persisted,
            response
        } if persisted == claim && response == "Welcome"
    ));
    assert!(matches!(
        chat_repo
            .begin_turn(&ChatTurnClaim {
                request_fingerprint: vec![9; 32],
                ..claim.clone()
            })
            .await
            .unwrap(),
        BeginChatTurn::Conflict
    ));

    let reclaim_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![3; 32],
        ..claim.clone()
    };
    let (first, second) = tokio::join!(
        chat_repo.begin_turn(&reclaim_claim),
        chat_repo.begin_turn(&reclaim_claim)
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert!(matches!(
        (&first, &second),
        (
            BeginChatTurn::Acquired { attempt: 1, .. },
            BeginChatTurn::InProgress { .. }
        ) | (
            BeginChatTurn::InProgress { .. },
            BeginChatTurn::Acquired { attempt: 1, .. }
        )
    ));
    sqlx::query(
        "UPDATE chat_turns SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE id = $1",
    )
    .bind(reclaim_claim.id)
    .execute(&pool)
    .await
    .unwrap();
    let reclaimed_attempt = match chat_repo.begin_turn(&reclaim_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected reclaimed turn: {result:?}"),
    };
    assert_eq!(reclaimed_attempt, 2);
    let reclaimed_user = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "user".into(),
        "Retry me".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(reclaim_claim.id);
    let reclaimed_character = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "character".into(),
        "Recovered once".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(reclaim_claim.id);
    assert!(chat_repo
        .complete_turn(&reclaim_claim, 1, &reclaimed_user, &reclaimed_character,)
        .await
        .is_err());
    chat_repo
        .complete_turn(
            &reclaim_claim,
            reclaimed_attempt,
            &reclaimed_user,
            &reclaimed_character,
        )
        .await
        .unwrap();
    let reclaimed_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages WHERE turn_id = $1")
            .bind(reclaim_claim.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reclaimed_count, 2);

    let active_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![5; 32],
        ..claim.clone()
    };
    assert!(matches!(
        chat_repo.begin_turn(&active_claim).await.unwrap(),
        BeginChatTurn::Acquired { attempt: 1, .. }
    ));
    let next_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![6; 32],
        ..claim.clone()
    };
    assert!(matches!(
        chat_repo.begin_turn(&next_claim).await.unwrap(),
        BeginChatTurn::InProgress {
            retry_after_seconds: 1..=120
        }
    ));
    sqlx::query(
        "UPDATE chat_turns SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE id = $1",
    )
    .bind(active_claim.id)
    .execute(&pool)
    .await
    .unwrap();
    let next_attempt = match chat_repo.begin_turn(&next_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected next turn reservation: {result:?}"),
    };
    assert_eq!(next_attempt, 1);
    assert!(matches!(
        chat_repo.begin_turn(&active_claim).await.unwrap(),
        BeginChatTurn::Conflict
    ));
    assert!(chat_repo
        .fail_turn(next_claim.id, next_attempt, "test_cleanup")
        .await
        .unwrap());

    let failed_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![7; 32],
        ..claim.clone()
    };
    let failed_attempt = match chat_repo.begin_turn(&failed_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected failed turn reservation: {result:?}"),
    };
    assert!(chat_repo
        .fail_turn(failed_claim.id, failed_attempt, "llm_error")
        .await
        .unwrap());
    let expired_other_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![8; 32],
        ..claim.clone()
    };
    assert!(matches!(
        chat_repo.begin_turn(&expired_other_claim).await.unwrap(),
        BeginChatTurn::Acquired { attempt: 1, .. }
    ));
    sqlx::query(
        "UPDATE chat_turns SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE id = $1",
    )
    .bind(expired_other_claim.id)
    .execute(&pool)
    .await
    .unwrap();
    let recovered_failed_attempt = match chat_repo.begin_turn(&failed_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("failed key was not recovered: {result:?}"),
    };
    assert_eq!(recovered_failed_attempt, 2);
    let (expired_status, expired_failure): (String, Option<String>) =
        sqlx::query_as("SELECT status, failure_code FROM chat_turns WHERE id = $1")
            .bind(expired_other_claim.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(expired_status, "failed");
    assert_eq!(expired_failure.as_deref(), Some("superseded"));
    assert!(chat_repo
        .fail_turn(failed_claim.id, recovered_failed_attempt, "test_cleanup")
        .await
        .unwrap());

    let rollback_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![4; 32],
        ..claim.clone()
    };
    let rollback_attempt = match chat_repo.begin_turn(&rollback_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected rollback turn reservation: {result:?}"),
    };
    let rollback_user = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "user".into(),
        "Atomic?".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(rollback_claim.id);
    let mut rollback_character = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "character".into(),
        "Yes".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(rollback_claim.id);
    rollback_character.id = rollback_user.id;
    assert!(chat_repo
        .complete_turn(
            &rollback_claim,
            rollback_attempt,
            &rollback_user,
            &rollback_character,
        )
        .await
        .is_err());
    let (rollback_status, rollback_messages): (String, i64) = sqlx::query_as(
        "SELECT status, (SELECT COUNT(*) FROM chat_messages WHERE turn_id = $1) FROM chat_turns WHERE id = $1",
    )
    .bind(rollback_claim.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rollback_status, "in_progress");
    assert_eq!(rollback_messages, 0);

    let node_repo = PgNarrativeNodeRepository::new(pool.clone());
    let node = NarrativeNode::new(
        novel_id,
        1,
        "A decisive moment".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "Stay".into(),
            hint: "Stand your ground".into(),
            generated_consequence: None,
        }],
    );
    node_repo.save(&node).await.unwrap();
    node_repo.save(&node).await.unwrap();
    assert_eq!(
        node_repo
            .find_by_chapter(novel_id, 1, None)
            .await
            .unwrap()
            .unwrap()
            .id,
        node.id
    );

    let choice_repo = PgUserChoiceRepository::new(pool.clone());
    let draft = ChoiceCommit {
        user_id,
        novel_id,
        node_id: node.id,
        chapter_number: 1,
        choice_index: 0,
        choice_text: "Stay".into(),
        consequence: "The character stays.".into(),
        rewritten_chapter_content: "Canonical opening. The character stays.".into(),
    };
    let (left, right) = tokio::join!(
        choice_repo.commit_choice(&draft),
        choice_repo.commit_choice(&draft)
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.choice.id, right.choice.id);
    assert_eq!(
        left.player_chapter_content,
        "Canonical opening. The character stays."
    );
    assert_eq!(left.player_chapter_content, right.player_chapter_content);
    let persisted_player_chapter: (String, String) = sqlx::query_as(
        "SELECT content, origin FROM player_chapters \
         WHERE user_id = $1 AND novel_id = $2 AND chapter_number = 1",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        persisted_player_chapter,
        (
            "Canonical opening. The character stays.".into(),
            "choice".into()
        )
    );
    assert_eq!(
        left.world_state.state["choices"].as_array().unwrap().len(),
        1
    );
    let persisted_choices: serde_json::Value = sqlx::query_scalar(
        "SELECT state -> 'choices' FROM world_states WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted_choices.as_array().unwrap().len(), 1);
    assert_eq!(
        choice_repo
            .find_user_choice(user_id, node.id)
            .await
            .unwrap()
            .unwrap()
            .choice_index,
        0
    );

    let mut more_drafts = Vec::new();
    for chapter_number in 2..=4 {
        let next = NarrativeNode::new(
            novel_id,
            chapter_number,
            format!("Decision {chapter_number}"),
            vec![NarrativeChoice {
                index: 0,
                text: format!("Choice {chapter_number}"),
                hint: "Continue".into(),
                generated_consequence: None,
            }],
        );
        node_repo.save(&next).await.unwrap();
        more_drafts.push(ChoiceCommit {
            user_id,
            novel_id,
            node_id: next.id,
            chapter_number,
            choice_index: 0,
            choice_text: format!("Choice {chapter_number}"),
            consequence: format!("Result {chapter_number}"),
            rewritten_chapter_content: format!("Rewritten chapter {chapter_number}"),
        });
    }
    let (second, third) = tokio::join!(
        choice_repo.commit_choice(&more_drafts[0]),
        choice_repo.commit_choice(&more_drafts[1])
    );
    second.unwrap();
    third.unwrap();
    let valid_state: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(valid_state["choices"].as_array().unwrap().len(), 3);

    sqlx::query(
        "UPDATE world_states SET state = '{\"choices\":{}}'::jsonb WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(choice_repo.commit_choice(&more_drafts[2]).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_choices WHERE user_id = $1 AND node_id = $2",
        )
        .bind(user_id)
        .bind(more_drafts[2].node_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    sqlx::query("UPDATE world_states SET state = $3 WHERE user_id = $1 AND novel_id = $2")
        .bind(user_id)
        .bind(novel_id)
        .bind(valid_state)
        .execute(&pool)
        .await
        .unwrap();
    choice_repo.commit_choice(&more_drafts[2]).await.unwrap();

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}
