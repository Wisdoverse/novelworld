use agent_service::domain::{entities::memory::ChatMessage, ports::MessageCache};
use agent_service::infrastructure::cache::RedisCache;
use redis::AsyncCommands;
use uuid::Uuid;

fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://localhost:26379".into())
}

#[tokio::test]
async fn test_redis_connection() {
    let client = redis::Client::open(redis_url()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();

    let _: () = conn.set("test_key", "hello").await.unwrap();
    let val: String = conn.get("test_key").await.unwrap();
    assert_eq!(val, "hello");

    let _: () = conn.del("test_key").await.unwrap();
}

#[tokio::test]
async fn test_redis_list_operations() {
    let client = redis::Client::open(redis_url()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();

    let key = format!("test_list_{}", uuid::Uuid::new_v4());

    // LPUSH + LRANGE (simulating chat message cache)
    let _: () = conn.lpush(&key, "msg3").await.unwrap();
    let _: () = conn.lpush(&key, "msg2").await.unwrap();
    let _: () = conn.lpush(&key, "msg1").await.unwrap();

    let msgs: Vec<String> = conn.lrange(&key, 0, -1).await.unwrap();
    assert_eq!(msgs, vec!["msg1", "msg2", "msg3"]);

    // LTRIM (keep only 2)
    let _: () = conn.ltrim(&key, 0, 1).await.unwrap();
    let msgs: Vec<String> = conn.lrange(&key, 0, -1).await.unwrap();
    assert_eq!(msgs.len(), 2);

    let _: () = conn.del(&key).await.unwrap();
}

#[tokio::test]
async fn test_redis_json_roundtrip() {
    let client = redis::Client::open(redis_url()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();

    let key = format!("test_json_{}", uuid::Uuid::new_v4());

    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct ChatMsg {
        role: String,
        content: String,
    }

    let msg = ChatMsg {
        role: "user".into(),
        content: "Hello character".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();

    let _: () = conn.lpush(&key, &json).await.unwrap();

    let raw: Vec<String> = conn.lrange(&key, 0, 0).await.unwrap();
    let parsed: ChatMsg = serde_json::from_str(&raw[0]).unwrap();
    assert_eq!(parsed, msg);

    let _: () = conn.del(&key).await.unwrap();
}

#[tokio::test]
async fn production_cache_filters_unknown_and_future_chapters() {
    let config = deadpool_redis::Config::from_url(redis_url());
    let pool = config
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .unwrap();
    let cache = RedisCache::new(pool);
    let user_id = Uuid::new_v4();
    let character_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();

    for (user_message, character_message) in [
        (
            ChatMessage::new(
                user_id,
                character_id,
                novel_id,
                "user".into(),
                "visible question".into(),
                None,
                Some(1),
            ),
            ChatMessage::new(
                user_id,
                character_id,
                novel_id,
                "character".into(),
                "visible".into(),
                None,
                Some(1),
            ),
        ),
        (
            ChatMessage::new(
                user_id,
                character_id,
                novel_id,
                "user".into(),
                "future question".into(),
                None,
                Some(3),
            ),
            ChatMessage::new(
                user_id,
                character_id,
                novel_id,
                "character".into(),
                "future".into(),
                None,
                Some(3),
            ),
        ),
        (
            ChatMessage::new(
                user_id,
                character_id,
                novel_id,
                "user".into(),
                "unknown question".into(),
                None,
                None,
            ),
            ChatMessage::new(
                user_id,
                character_id,
                novel_id,
                "character".into(),
                "unknown".into(),
                None,
                None,
            ),
        ),
    ] {
        cache
            .push_turn(character_id, user_id, &user_message, &character_message)
            .await
            .unwrap();
    }

    let visible = cache
        .get_recent_messages(character_id, user_id, 1, 10)
        .await
        .unwrap();
    assert_eq!(visible.len(), 2);
    assert_eq!(visible[0].content, "visible question");
    assert_eq!(visible[1].content, "visible");
    cache.clear(character_id, user_id).await.unwrap();
}

#[tokio::test]
async fn privacy_cleanup_is_scoped_to_the_requested_user_and_novel() {
    let config = deadpool_redis::Config::from_url(redis_url());
    let pool = config
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .unwrap();
    let cache = RedisCache::new(pool);
    let user_id = Uuid::new_v4();
    let other_user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let other_novel_id = Uuid::new_v4();
    let character_id = Uuid::new_v4();
    let other_character_id = Uuid::new_v4();
    let other_user_character_id = Uuid::new_v4();

    for (owner, character, novel, content) in [
        (user_id, character_id, novel_id, "delete this novel"),
        (
            user_id,
            other_character_id,
            other_novel_id,
            "keep until user cleanup",
        ),
        (
            other_user_id,
            other_user_character_id,
            novel_id,
            "keep other user",
        ),
    ] {
        let user_message = ChatMessage::new(
            owner,
            character,
            novel,
            "user".into(),
            content.into(),
            None,
            Some(1),
        );
        let character_message = ChatMessage::new(
            owner,
            character,
            novel,
            "character".into(),
            "reply".into(),
            None,
            Some(1),
        );
        assert!(cache
            .push_turn(character, owner, &user_message, &character_message)
            .await
            .unwrap());
    }

    cache.clear_novel(user_id, novel_id).await.unwrap();
    assert!(cache
        .get_recent_messages(character_id, user_id, 1, 10)
        .await
        .unwrap()
        .is_empty());
    let late_user_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "user".into(),
        "late projection".into(),
        None,
        Some(1),
    );
    let late_character_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "character".into(),
        "late reply".into(),
        None,
        Some(1),
    );
    assert!(!cache
        .push_turn(
            character_id,
            user_id,
            &late_user_message,
            &late_character_message,
        )
        .await
        .unwrap());
    assert!(cache
        .get_recent_messages(character_id, user_id, 1, 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        cache
            .get_recent_messages(other_character_id, user_id, 1, 10)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        cache
            .get_recent_messages(other_user_character_id, other_user_id, 1, 10)
            .await
            .unwrap()
            .len(),
        2
    );
    cache.allow_novel(user_id, novel_id).await.unwrap();
    assert!(cache
        .push_turn(
            character_id,
            user_id,
            &late_user_message,
            &late_character_message,
        )
        .await
        .unwrap());
    assert_eq!(
        cache
            .get_recent_messages(character_id, user_id, 1, 10)
            .await
            .unwrap()
            .len(),
        2
    );

    cache.clear_user(user_id).await.unwrap();
    assert!(cache
        .get_recent_messages(other_character_id, user_id, 1, 10)
        .await
        .unwrap()
        .is_empty());
    let late_user_message = ChatMessage::new(
        user_id,
        other_character_id,
        other_novel_id,
        "user".into(),
        "late account projection".into(),
        None,
        Some(1),
    );
    let late_character_message = ChatMessage::new(
        user_id,
        other_character_id,
        other_novel_id,
        "character".into(),
        "late reply".into(),
        None,
        Some(1),
    );
    assert!(!cache
        .push_turn(
            other_character_id,
            user_id,
            &late_user_message,
            &late_character_message,
        )
        .await
        .unwrap());
    assert!(cache
        .get_recent_messages(other_character_id, user_id, 1, 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        cache
            .get_recent_messages(other_user_character_id, other_user_id, 1, 10)
            .await
            .unwrap()
            .len(),
        2
    );
    cache.allow_user(user_id).await.unwrap();
    assert!(cache
        .push_turn(
            other_character_id,
            user_id,
            &late_user_message,
            &late_character_message,
        )
        .await
        .unwrap());
    assert_eq!(
        cache
            .get_recent_messages(other_character_id, user_id, 1, 10)
            .await
            .unwrap()
            .len(),
        2
    );

    cache.clear_user(user_id).await.unwrap();
    cache.clear_user(other_user_id).await.unwrap();
    cache.allow_user(user_id).await.unwrap();
    cache.allow_user(other_user_id).await.unwrap();
    cache.allow_novel(user_id, novel_id).await.unwrap();
}
