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
