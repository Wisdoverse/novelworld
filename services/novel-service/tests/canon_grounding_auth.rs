use axum::{
    body::Body,
    http::{header::CACHE_CONTROL, HeaderValue, Request, StatusCode},
};
use novel_service::interface::http::{router, AppState};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

#[path = "support/http_state.rs"]
mod http_state;

fn auth_test_state() -> AppState {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(10))
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    http_state::state_with_pool(pool)
}

#[tokio::test]
async fn source_relationship_route_rejects_missing_principal_before_database_work() {
    let response = router(auth_test_state())
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/novels/{}/relationships/source-v1",
                    Uuid::new_v4()
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()[CACHE_CONTROL],
        HeaderValue::from_static("private, no-store")
    );
}

#[tokio::test]
async fn canon_grounding_route_rejects_missing_and_wrong_internal_tokens() {
    let app = router(auth_test_state());
    let uri = format!(
        "/internal/novels/{}/characters/{}/grounding-v1/3",
        Uuid::new_v4(),
        Uuid::new_v4()
    );
    for token in [None, Some("wrong-token")] {
        let mut request = Request::builder()
            .uri(&uri)
            .header("X-User-Id", Uuid::new_v4().to_string());
        if let Some(token) = token {
            request = request.header("X-Internal-Service-Token", token);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers().get(CACHE_CONTROL),
            Some(&HeaderValue::from_static("private, no-store"))
        );
    }
}

#[tokio::test]
async fn series_routes_reject_missing_principal_before_database_or_provider_work() {
    let app = router(auth_test_state());
    let novel = Uuid::new_v4();
    let create = serde_json::json!({"name":"同一世界", "background":"用户确认的共同背景", "source_novel_id":novel});
    for (method, uri, body) in [
        ("GET", "/novels/world-series".into(), None),
        (
            "POST",
            format!("/novels/{novel}/world-series/suggestion/deepseek"),
            None,
        ),
        (
            "POST",
            "/novels/world-series".into(),
            Some(create.to_string()),
        ),
        ("GET", format!("/novels/{novel}/world-series"), None),
        (
            "GET",
            format!("/novels/world-series/{novel}/background-draft"),
            None,
        ),
        (
            "GET",
            format!("/novels/{novel}/world-series/background-draft"),
            None,
        ),
        (
            "PUT",
            format!("/novels/{novel}/world-series"),
            Some(r#"{"series_id":null}"#.into()),
        ),
        (
            "POST",
            format!("/novels/{novel}/world-series/suggestion"),
            None,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.unwrap_or_default()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()[CACHE_CONTROL],
            HeaderValue::from_static("private, no-store")
        );
    }
}

#[tokio::test]
async fn direct_series_generation_is_rejected_without_a_database_claim() {
    let app = router(auth_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/novels/{}/game-rules?prompt_version=series-game-rules-v1",
                    Uuid::new_v4()
                ))
                .header("X-User-Id", Uuid::new_v4().to_string())
                .header("X-Internal-Service-Token", "expected-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["code"], "unsupported_game_rule_version");
}
