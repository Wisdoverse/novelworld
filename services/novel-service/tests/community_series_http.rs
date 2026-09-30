//! Opt-in test over disposable PostgreSQL and actual Axum routes.
use axum::{
    body::{to_bytes, Body},
    http::{header::CACHE_CONTROL, Request, StatusCode},
    Router,
};
use novel_service::interface::http::router;
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
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(user) = user {
        request = request.header("X-User-Id", user.to_string());
    }
    let body = if let Some(body) = body {
        request = request.header("Content-Type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()[CACHE_CONTROL], "private, no-store");
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
#[ignore = "requires explicitly disposable TEST_DATABASE_URL; run with --ignored"]
async fn community_routes_wire_owner_consent_and_live_consensus_without_providers() {
    let url = std::env::var("TEST_DATABASE_URL").expect("provide disposable PostgreSQL");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    let users = (0..5).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
    let recipient = users[0];
    for user in &users {
        sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'synthetic')")
            .bind(user)
            .bind(format!("community-http-{user}@test.invalid"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let source = Uuid::new_v4();
    let target = Uuid::new_v4();
    for (novel, title) in [(source, "Recipient source"), (target, "Recipient target")] {
        sqlx::query("INSERT INTO novels (id, user_id, title, total_chapters, status) VALUES ($1, $2, $3, 1, 'ready'::novel_status)")
            .bind(novel).bind(recipient).bind(title).execute(&pool).await.unwrap();
        for user in users.iter().take(4) {
            sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
                .bind(user)
                .bind(novel)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    let mut series = Vec::new();
    for (index, user) in users.iter().take(4).enumerate() {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO user_world_series (id, user_id, name, revision, source_novel_id, created_at) VALUES ($1, $2, $3, 1, $4, NOW())")
            .bind(id).bind(user).bind(if index == 0 { "MY SERIES" } else { "PRIVATE DONOR" }).bind(source).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO user_novel_world_series (user_id, novel_id, series_id) VALUES ($1, $2, $3)")
            .bind(user).bind(source).bind(id).execute(&pool).await.unwrap();
        if index > 0 {
            sqlx::query("INSERT INTO user_novel_world_series (user_id, novel_id, series_id) VALUES ($1, $2, $3)")
                .bind(user).bind(target).bind(id).execute(&pool).await.unwrap();
        }
        series.push(id);
    }
    let app = router(http_state::state_with_pool(pool.clone()));
    let own = format!("/novels/world-series/{}/contribution", series[0]);
    let community = format!("/novels/{target}/world-series/community-suggestion");
    for (method, path, body) in [
        ("GET", own.as_str(), None),
        (
            "PUT",
            own.as_str(),
            Some(serde_json::json!({"enabled":true})),
        ),
        ("POST", community.as_str(), None),
    ] {
        assert_eq!(
            request(&app, method, path, None, body).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(&app, "GET", &own, Some(users[4]), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            &own,
            Some(users[4]),
            Some(serde_json::json!({"enabled":true}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "POST", &community, Some(users[4]), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "GET", &own, Some(recipient), None).await.1,
        serde_json::json!({"enabled":false})
    );
    for enabled in [true, true, false] {
        let (status, body) = request(
            &app,
            "PUT",
            &own,
            Some(recipient),
            Some(serde_json::json!({"enabled":enabled})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"enabled":enabled}));
    }
    assert_eq!(
        request(&app, "GET", &own, Some(recipient), None).await.1,
        serde_json::json!({"enabled":false})
    );
    assert_eq!(
        request(&app, "POST", &community, Some(recipient), None)
            .await
            .1["status"],
        "uncertain"
    );
    for (user, series_id) in users.iter().zip(&series).skip(1) {
        let path = format!("/novels/world-series/{series_id}/contribution");
        assert_eq!(
            request(
                &app,
                "PUT",
                &path,
                Some(*user),
                Some(serde_json::json!({"enabled":true}))
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let (status, suggestion) = request(&app, "POST", &community, Some(recipient), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(suggestion["status"], "suggested");
    assert_eq!(suggestion["method"], "community");
    assert_eq!(suggestion["reason"], "community_consensus");
    assert_eq!(suggestion["cached"], false);
    assert_eq!(suggestion["suggestion"]["series_id"], series[0].to_string());
    assert_eq!(suggestion["suggestion"]["name"], "MY SERIES");
    let serialized = suggestion.to_string();
    assert!(!serialized.contains("PRIVATE DONOR"));
    for donor in users.iter().skip(1) {
        assert!(!serialized.contains(&donor.to_string()));
    }
    let path = format!("/novels/world-series/{}/contribution", series[3]);
    request(
        &app,
        "PUT",
        &path,
        Some(users[3]),
        Some(serde_json::json!({"enabled":false})),
    )
    .await;
    assert_eq!(
        request(&app, "POST", &community, Some(recipient), None)
            .await
            .1["status"],
        "uncertain"
    );
    for user in users {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
}
