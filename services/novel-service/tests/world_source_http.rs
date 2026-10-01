//! Opt-in source projection and monotonic progress over disposable PostgreSQL and real routes.
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use novel_service::{
    domain::repositories::ReadingProgressRepository,
    infrastructure::persistence::pg_progress_repo::PgReadingProgressRepository,
    interface::http::router,
};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

#[path = "support/http_state.rs"]
mod http_state;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    user: Option<Uuid>,
    internal: bool,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(user) = user {
        req = req.header("X-User-Id", user.to_string());
    }
    if internal {
        req = req.header("X-Internal-Service-Token", "expected-token");
    }
    let body = match body {
        Some(body) => {
            req = req.header("Content-Type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn evidence(chapters: &[i32]) -> Value {
    json!({"confidence":1.0,"provenance":chapters.iter().map(|chapter| json!({"chapter_number":chapter,"excerpt":"synthetic authority"})).collect::<Vec<_>>()})
}

#[tokio::test]
#[ignore = "requires explicitly disposable TEST_DATABASE_URL; run with --ignored"]
async fn next_source_routes_pin_canon_and_never_rewind_progress_or_expose_future_authority() {
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&std::env::var("TEST_DATABASE_URL").expect("provide disposable PostgreSQL"))
        .await
        .unwrap();
    let owner = Uuid::new_v4();
    let reader = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    for user in [owner, reader, outsider] {
        sqlx::query("INSERT INTO users (id,email,password_hash) VALUES ($1,$2,'synthetic')")
            .bind(user)
            .bind(format!("source-http-{user}@test.invalid"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let novel = Uuid::new_v4();
    sqlx::query("INSERT INTO novels (id,user_id,title,total_chapters,status) VALUES ($1,$2,'Synthetic source',10,'ready'::novel_status)").bind(novel).bind(owner).execute(&pool).await.unwrap();
    for user in [owner, reader] {
        sqlx::query("INSERT INTO user_novels (user_id,novel_id) VALUES ($1,$2)")
            .bind(user)
            .bind(novel)
            .execute(&pool)
            .await
            .unwrap();
    }
    for chapter in (1..=10).filter(|chapter| *chapter != 8) {
        sqlx::query("INSERT INTO chapters (novel_id,chapter_number,content) VALUES ($1,$2,'守门人走进城门。')").bind(novel).bind(chapter).execute(&pool).await.unwrap();
    }
    let character = Uuid::new_v4();
    let unproven = Uuid::new_v4();
    for (id, name) in [(character, "守门人"), (unproven, "未出现者")] {
        sqlx::query("INSERT INTO characters (id,novel_id,name,role,first_appearance_chapter) VALUES ($1,$2,$3,'supporting',1)").bind(id).bind(novel).bind(name).execute(&pool).await.unwrap();
    }
    let content = json!({
        "arcs":[], "locations":[{"id":"gate","name":"城门","description":"synthetic","evidence":evidence(&[1])}],
        "factions":[], "world_rules":[], "relationships":[],
        "events":[
            {"id":"next","sequence":2,"summary":"chapter two scene","caused_by":[],"location_ids":["gate"],"character_ids":[character],"faction_ids":[],"evidence":evidence(&[2])},
            {"id":"mixed","sequence":3,"summary":"whole future text","caused_by":[],"location_ids":[],"character_ids":[character],"faction_ids":[],"evidence":evidence(&[2,10])},
            {"id":"death-gated","sequence":4,"summary":"attached future death","caused_by":[],"location_ids":[],"character_ids":[character],"faction_ids":[],"evidence":evidence(&[2])},
            {"id":"unproven","sequence":5,"summary":"unproven name event","caused_by":[],"location_ids":[],"character_ids":[unproven],"faction_ids":[],"evidence":evidence(&[2])}
        ],
        "deaths":[{"id":"death","character_id":character,"event_id":"death-gated","description":"future death","evidence":evidence(&[2,10])}],
        "character_goals":[{"id":"goal","character_id":character,"description":"future goal","evidence":evidence(&[2,10])}],
        "unresolved_threads":[{"id":"thread","description":"future thread","evidence":evidence(&[2,10])}],
        "ending":{"summary":"ending","character_states":{},"faction_states":{},"location_states":{},"unresolved_thread_ids":[],"evidence":evidence(&[10])}
    });
    for version in [1, 2] {
        let mut version_content = content.clone();
        if version == 2 {
            version_content["events"][0]["summary"] =
                json!("newer model must not replace pinned scene");
        }
        sqlx::query("INSERT INTO canon_story_models (id,novel_id,model_version,schema_version,prompt_version,content) VALUES ($1,$2,$3,1,'canon-extraction-v1',$4)").bind(Uuid::new_v4()).bind(novel).bind(version).bind(version_content).execute(&pool).await.unwrap();
    }
    let app = router(http_state::state_with_pool(pool.clone()));
    let advance = format!("/progress/{novel}/advance");
    assert_eq!(
        request(
            &app,
            "POST",
            &advance,
            None,
            false,
            Some(json!({"current_chapter":2}))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &advance,
            Some(outsider),
            false,
            Some(json!({"current_chapter":2}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &advance,
            Some(reader),
            false,
            Some(json!({"current_chapter":2,"extra":true}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &advance,
            Some(reader),
            false,
            Some(json!({"current_chapter":11}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &advance,
            Some(reader),
            false,
            Some(json!({"current_chapter":8}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let first = request(
        &app,
        "POST",
        &advance,
        Some(reader),
        false,
        Some(json!({"current_chapter":3})),
    )
    .await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(first.1["current_chapter"], 3);
    sqlx::query("UPDATE reading_progress SET reader_identity='自选名字', deviation_mode='remix' WHERE user_id=$1 AND novel_id=$2").bind(reader).bind(novel).execute(&pool).await.unwrap();
    let identity = request(
        &app,
        "GET",
        &format!("/progress/{novel}"),
        Some(reader),
        false,
        None,
    )
    .await
    .1;
    let stale = request(
        &app,
        "POST",
        &advance,
        Some(reader),
        false,
        Some(json!({"current_chapter":2})),
    )
    .await;
    assert_eq!(stale.0, StatusCode::OK);
    assert_eq!(stale.1["current_chapter"], 3);
    for field in [
        "id",
        "reader_identity",
        "reader_identity_type",
        "reader_character_id",
        "deviation_mode",
        "created_at",
    ] {
        assert_eq!(stale.1[field], identity[field]);
    }
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM reading_progress WHERE user_id=$1 AND novel_id=$2 FOR UPDATE")
        .bind(reader)
        .bind(novel)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let blocked = PgReadingProgressRepository::new(pool.clone())
        .advance_chapter(reader, novel, 6)
        .await;
    let error = blocked.unwrap_err();
    assert!(error
        .downcast_ref::<sqlx::Error>()
        .and_then(sqlx::Error::as_database_error)
        .is_some_and(|error| error.code().is_some_and(|code| code == "55P03")));
    assert!(started.elapsed() < std::time::Duration::from_secs(6));
    blocker.rollback().await.unwrap();
    let after_block = request(
        &app,
        "GET",
        &format!("/progress/{novel}"),
        Some(reader),
        false,
        None,
    )
    .await;
    assert_eq!(after_block.1["current_chapter"], 3);
    let (a, b) = tokio::join!(
        request(
            &app,
            "POST",
            &advance,
            Some(reader),
            false,
            Some(json!({"current_chapter":4}))
        ),
        request(
            &app,
            "POST",
            &advance,
            Some(reader),
            false,
            Some(json!({"current_chapter":5}))
        ),
    );
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    let raced = request(
        &app,
        "GET",
        &format!("/progress/{novel}"),
        Some(reader),
        false,
        None,
    )
    .await;
    assert_eq!(raced.1["current_chapter"], 5);
    let url = format!("/internal/novels/{novel}/world-entry/1?source_extension=true&model_version=1&from_source_chapter=1&target_chapter=2");
    assert_eq!(
        request(&app, "GET", &url, Some(reader), false, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let delta = request(&app, "GET", &url, Some(reader), true, None).await;
    assert_eq!(delta.0, StatusCode::OK);
    assert_eq!(delta.1["model_version"], 1);
    assert_eq!(delta.1["scheduled_events"].as_array().unwrap().len(), 1);
    assert_eq!(
        delta.1["scheduled_events"][0]["definition"]["summary"],
        "chapter two scene"
    );
    assert_eq!(
        delta.1["scheduled_events"][0]["source_chapters"],
        json!([1, 2])
    );
    assert_eq!(delta.1["characters"].as_array().unwrap().len(), 1);
    assert!(delta.1["character_goals"].as_array().unwrap().is_empty());
    assert!(delta.1["threads"].as_array().unwrap().is_empty());
    assert!(delta.1.get("series_setting").is_none());
    assert!(delta.1.get("dead_character_ids").is_none());
    assert_eq!(
        request(
            &app,
            "GET",
            &url.replace("model_version=1", "model_version=99"),
            Some(reader),
            true,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &url.replace("&model_version=1", ""),
            Some(reader),
            true,
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // The absolute PUT intentionally retains rewind semantics; the source read then fails closed.
    assert_eq!(
        request(
            &app,
            "PUT",
            &format!("/progress/{novel}"),
            Some(reader),
            false,
            Some(json!({"current_chapter":1}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "GET", &url, Some(reader), true, None).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    request(
        &app,
        "POST",
        &advance,
        Some(reader),
        false,
        Some(json!({"current_chapter":10})),
    )
    .await;
    let late = request(
        &app,
        "GET",
        &url.replace(
            "from_source_chapter=1&target_chapter=2",
            "from_source_chapter=9&target_chapter=10",
        ),
        Some(reader),
        true,
        None,
    )
    .await;
    assert_eq!(late.0, StatusCode::OK);
    let events = late.1["scheduled_events"].as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[1]["definition"]["source_chapters"], json!([2, 10]));
    assert_eq!(
        events[2]["definition"]["death_character_ids"],
        json!([character])
    );
    let legacy = request(
        &app,
        "GET",
        &format!("/internal/novels/{novel}/world-entry/1"),
        Some(reader),
        true,
        None,
    )
    .await;
    assert_eq!(legacy.0, StatusCode::OK);
    assert_eq!(legacy.1["model_version"], 2);
    assert!(legacy.1.get("unlocked_through_chapter").is_some());
    assert!(legacy.1.get("from_source_chapter").is_none());
    // Remove only this fixture's exact identities and source data.
    sqlx::query("DELETE FROM novels WHERE id=$1")
        .bind(novel)
        .execute(&pool)
        .await
        .unwrap();
    for user in [owner, reader, outsider] {
        sqlx::query("DELETE FROM users WHERE id=$1")
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
}
