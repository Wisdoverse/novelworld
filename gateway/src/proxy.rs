use axum::{
    body::Body,
    extract::{Request, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, RETRY_AFTER, WWW_AUTHENTICATE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use reqwest::Client;
use std::{io, sync::Arc, time::Duration};
use tokio::{sync::OwnedSemaphorePermit, time::Instant};
use uuid::Uuid;

use crate::AppState;

const MAX_PROXY_BODY_BYTES: usize = 21 * 1024 * 1024;
const MAX_PUBLIC_ERROR_CHARS: usize = 512;
const ACCOUNT_EXPORT_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const ACCOUNT_EXPORT_CONTENT_TYPE: &str = "application/x-ndjson";

pub fn api_error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let mut response = (
        status,
        axum::Json(serde_json::json!({
            "error": {"code": code, "message": message}
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The exhaustive mapping from upstream HTTP status to the stable public error
/// envelope. Every upstream client/server error the gateway normalizes is pinned
/// here, and the test module holds an independent copy of this contract, so a
/// row added, removed, or altered without updating the expectation fails the
/// build. Direct gateway-generated responses call api_error_response with an
/// explicit code and are not constrained to this table.
const NORMALIZED_ERROR_RESPONSES: &[(StatusCode, &str, &str)] = &[
    (
        StatusCode::BAD_REQUEST,
        "validation_error",
        "Request validation failed",
    ),
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_error",
        "Request validation failed",
    ),
    (
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "Authentication is required",
    ),
    (StatusCode::FORBIDDEN, "forbidden", "Access is forbidden"),
    (StatusCode::NOT_FOUND, "not_found", "Resource not found"),
    (
        StatusCode::CONFLICT,
        "conflict",
        "Request conflicts with existing state",
    ),
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        "Request body is too large",
    ),
    (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
        "Unsupported media type",
    ),
    (
        StatusCode::UPGRADE_REQUIRED,
        "client_upgrade_required",
        "Client upgrade required",
    ),
    (
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "Rate limit exceeded",
    ),
    (
        StatusCode::BAD_GATEWAY,
        "bad_gateway",
        "Upstream service returned an invalid response",
    ),
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "service_unavailable",
        "Service is temporarily unavailable",
    ),
    (
        StatusCode::GATEWAY_TIMEOUT,
        "service_unavailable",
        "Service is temporarily unavailable",
    ),
];

fn error_code(status: StatusCode) -> &'static str {
    NORMALIZED_ERROR_RESPONSES
        .iter()
        .find(|(mapped, _, _)| *mapped == status)
        .map(|(_, code, _)| *code)
        .unwrap_or(if status.is_server_error() {
            "internal_error"
        } else {
            "request_error"
        })
}

fn fallback_error_message(status: StatusCode) -> &'static str {
    NORMALIZED_ERROR_RESPONSES
        .iter()
        .find(|(mapped, _, _)| *mapped == status)
        .map(|(_, _, message)| *message)
        .unwrap_or("Request failed")
}

fn public_error_message(status: StatusCode, body: &[u8]) -> String {
    if status.is_client_error() {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
            let message = value
                .get("error")
                .and_then(|error| error.as_str().or_else(|| error.get("message")?.as_str()))
                .or_else(|| value.get("message").and_then(serde_json::Value::as_str));
            if let Some(message) = message {
                return message.chars().take(MAX_PUBLIC_ERROR_CHARS).collect();
            }
        }
    }

    fallback_error_message(status).into()
}

fn has_stable_error_envelope(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            let error = value.get("error")?;
            Some(error.get("code")?.is_string() && error.get("message")?.is_string())
        })
        .unwrap_or(false)
}

fn normalized_error_response(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> Response {
    if has_stable_error_envelope(body) {
        let mut response = Response::builder().status(status);
        for (key, value) in headers {
            response = response.header(key, value);
        }
        return response
            .body(Body::from(body.to_vec()))
            .unwrap_or_else(|_| {
                api_error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Response construction failed",
                )
            });
    }

    let mut response = api_error_response(
        status,
        error_code(status),
        &public_error_message(status, body),
    );
    for header in [RETRY_AFTER, WWW_AUTHENTICATE] {
        if let Some(value) = headers.get(&header) {
            response.headers_mut().insert(header, value.clone());
        }
    }
    response
}

pub struct ServiceProxy {
    pub novel_service_url: String,
    pub agent_service_url: String,
    pub narrative_service_url: String,
    pub user_service_url: String,
    pub client: Client,
    pub internal_service_token: Arc<str>,
}

fn json_line(value: serde_json::Value) -> io::Result<Bytes> {
    let mut line = serde_json::to_vec(&value).map_err(io::Error::other)?;
    line.push(b'\n');
    Ok(Bytes::from(line))
}

fn is_account_export_response(response: &reqwest::Response) -> bool {
    response.status() == StatusCode::OK
        && response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| {
                value
                    .trim()
                    .eq_ignore_ascii_case(ACCOUNT_EXPORT_CONTENT_TYPE)
            })
}

fn is_internal_service_path(path: &str) -> bool {
    path == "/internal" || path.starts_with("/internal/")
}

fn is_sse_response(path: &str, status: StatusCode, headers: &HeaderMap) -> bool {
    path.contains("/stream")
        && status.is_success()
        && headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
}

impl ServiceProxy {
    async fn account_export(&self, user_id: Uuid, permit: OwnedSemaphorePermit) -> Response {
        let deadline = Instant::now() + ACCOUNT_EXPORT_TIMEOUT;
        let user_url = format!(
            "{}/internal/privacy/users/{user_id}/export",
            self.user_service_url
        );
        let user_response = match tokio::time::timeout_at(
            deadline,
            self.client
                .get(&user_url)
                .header(
                    "X-Internal-Service-Token",
                    self.internal_service_token.as_ref(),
                )
                .send(),
        )
        .await
        {
            Ok(Ok(response)) if is_account_export_response(&response) => response,
            Ok(Ok(response)) if response.status() == StatusCode::NOT_FOUND => {
                return api_error_response(StatusCode::NOT_FOUND, "not_found", "Account not found")
            }
            Ok(Ok(response)) => {
                tracing::error!(status = %response.status(), "user export preflight failed");
                return api_error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service_unavailable",
                    "Account export is temporarily unavailable",
                );
            }
            Ok(Err(error)) => {
                tracing::error!(?error, "user export preflight failed");
                return api_error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service_unavailable",
                    "Account export is temporarily unavailable",
                );
            }
            Err(_) => {
                return api_error_response(
                    StatusCode::GATEWAY_TIMEOUT,
                    "service_unavailable",
                    "Account export timed out",
                )
            }
        };

        let exported_at = chrono::Utc::now();
        let client = self.client.clone();
        let internal_service_token = self.internal_service_token.clone();
        let fragments = [
            ("user", user_url),
            (
                "novel",
                format!(
                    "{}/internal/privacy/users/{user_id}/export",
                    self.novel_service_url
                ),
            ),
            (
                "agent",
                format!(
                    "{}/internal/privacy/users/{user_id}/export",
                    self.agent_service_url
                ),
            ),
            (
                "narrative",
                format!(
                    "{}/internal/privacy/users/{user_id}/export",
                    self.narrative_service_url
                ),
            ),
        ];

        let stream: std::pin::Pin<Box<dyn futures::Stream<Item = io::Result<Bytes>> + Send>> =
            Box::pin(
                async_stream::try_stream! {
                let _permit = permit;
                let mut initial_user = Some(user_response);
                yield json_line(serde_json::json!({
                    "type": "manifest",
                    "schema": "account-export-v1",
                    "user_id": user_id,
                    "created_at": exported_at,
                    "snapshot": "service-local",
                    "services": ["user", "novel", "agent", "narrative"],
                }))?;

                for (service, url) in fragments {
                    let response = if let Some(response) = initial_user.take() {
                        response
                    } else {
                        tokio::time::timeout_at(
                            deadline,
                            client
                                .get(&url)
                                .header(
                                    "X-Internal-Service-Token",
                                    internal_service_token.as_ref(),
                                )
                                .send(),
                        )
                        .await
                        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "account export timed out"))?
                        .map_err(io::Error::other)?
                    };
                    if !is_account_export_response(&response) {
                        tracing::error!(service, status = %response.status(), "account export fragment failed");
                        Err(io::Error::other(format!("{service} export fragment failed")))?;
                    }

                    yield json_line(serde_json::json!({
                        "type": "service_start",
                        "service": service,
                    }))?;
                    let mut body = response.bytes_stream();
                    loop {
                        let next = tokio::time::timeout_at(deadline, body.next())
                            .await
                            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "account export timed out"))?;
                        match next {
                            Some(Ok(chunk)) => yield chunk,
                            Some(Err(error)) => Err(io::Error::other(error))?,
                            None => break,
                        }
                    }
                    yield json_line(serde_json::json!({
                        "type": "service_complete",
                        "service": service,
                    }))?;
                }

                    yield json_line(serde_json::json!({
                        "type": "complete",
                        "schema": "account-export-v1",
                        "services": ["user", "novel", "agent", "narrative"],
                    }))?;
                }
                .take_until(tokio::time::sleep_until(deadline)),
            );

        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, ACCOUNT_EXPORT_CONTENT_TYPE)
            .header(CACHE_CONTROL, "no-store")
            .header("X-Accel-Buffering", "no")
            .header(
                CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"novelworld-account-{user_id}-{}.ndjson\"",
                    exported_at.format("%Y-%m-%d")
                ),
            )
            .body(Body::from_stream(stream))
            .unwrap_or_else(|_| {
                api_error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Response construction failed",
                )
            })
    }

    async fn forward(&self, target_base: &str, original_path: &str, request: Request) -> Response {
        let method = request.method().clone();
        let headers = request.headers().clone();
        let target_url = match reqwest::Url::parse(&format!("{target_base}{original_path}")) {
            Ok(url) if !is_internal_service_path(url.path()) => url,
            Ok(_) => {
                return api_error_response(StatusCode::NOT_FOUND, "not_found", "Route not found")
            }
            Err(error) => {
                tracing::warn!(?error, "rejected invalid proxy target");
                return api_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "Invalid request path",
                );
            }
        };
        let body = match axum::body::to_bytes(request.into_body(), MAX_PROXY_BODY_BYTES).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("Failed to read request body: {}", e);
                return api_error_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "payload_too_large",
                    "Request body is too large",
                );
            }
        };

        let mut req_builder = self.client.request(method, target_url.clone());

        for (key, value) in &headers {
            if key == "host" {
                continue;
            }
            req_builder = req_builder.header(key, value);
        }

        match req_builder.body(body).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_server_error() {
                    tracing::warn!(status = %status, upstream = target_base, "upstream returned a server error");
                }
                let resp_headers = resp.headers().clone();

                if is_sse_response(original_path, status, &resp_headers) {
                    let byte_stream = resp.bytes_stream().map_err(std::io::Error::other);
                    let body = Body::from_stream(byte_stream);

                    let mut response = Response::builder()
                        .status(status.as_u16())
                        .header("Content-Type", "text/event-stream")
                        .header("Cache-Control", "no-cache")
                        .header("X-Accel-Buffering", "no")
                        .header("Connection", "keep-alive");

                    for (key, value) in &resp_headers {
                        let k = key.as_str();
                        if k != "content-length" && k != "content-type" && k != "transfer-encoding"
                        {
                            response = response.header(key, value);
                        }
                    }

                    response.body(body).unwrap_or_else(|_| {
                        api_error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "internal_error",
                            "Response construction failed",
                        )
                    })
                } else {
                    let resp_body = match resp.bytes().await {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::error!("Failed to read response from {}: {}", target_url, e);
                            return api_error_response(
                                StatusCode::BAD_GATEWAY,
                                "bad_gateway",
                                "Upstream service returned an invalid response",
                            );
                        }
                    };
                    if status.is_client_error() || status.is_server_error() {
                        return normalized_error_response(status, &resp_headers, &resp_body);
                    }
                    let mut response = Response::builder().status(status.as_u16());
                    for (key, value) in &resp_headers {
                        response = response.header(key, value);
                    }
                    response.body(Body::from(resp_body)).unwrap_or_else(|_| {
                        api_error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "internal_error",
                            "Response construction failed",
                        )
                    })
                }
            }
            Err(e) => {
                tracing::error!("Proxy error to {}: {}", target_url, e);
                api_error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service_unavailable",
                    "Service is temporarily unavailable",
                )
            }
        }
    }
}

pub async fn export_account(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user_id = match headers
        .get("X-User-Id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
    {
        Some(user_id) => user_id,
        None => {
            return api_error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing or invalid user identity",
            )
        }
    };
    let permit = match state.account_export_permits.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            let mut response = api_error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many account exports are running",
            );
            response
                .headers_mut()
                .insert(RETRY_AFTER, "30".parse().expect("static header is valid"));
            return response;
        }
    };

    state.proxy.account_export(user_id, permit).await
}

pub async fn forward_to_novel(State(state): State<AppState>, request: Request) -> Response {
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let stripped = path_and_query
        .strip_prefix("/api")
        .unwrap_or(&path_and_query);
    state
        .proxy
        .forward(&state.proxy.novel_service_url, stripped, request)
        .await
}

pub async fn forward_to_agent(State(state): State<AppState>, request: Request) -> Response {
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let stripped = path_and_query
        .strip_prefix("/api")
        .unwrap_or(&path_and_query);
    state
        .proxy
        .forward(&state.proxy.agent_service_url, stripped, request)
        .await
}

pub async fn forward_to_narrative(State(state): State<AppState>, request: Request) -> Response {
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let stripped = path_and_query
        .strip_prefix("/api")
        .unwrap_or(&path_and_query);
    state
        .proxy
        .forward(&state.proxy.narrative_service_url, stripped, request)
        .await
}

pub async fn forward_to_user(State(state): State<AppState>, request: Request) -> Response {
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let stripped = path_and_query
        .strip_prefix("/api")
        .unwrap_or(&path_and_query);
    state
        .proxy
        .forward(&state.proxy.user_service_url, stripped, request)
        .await
}

#[cfg(test)]
mod tests {
    use super::{
        has_stable_error_envelope, is_internal_service_path, is_sse_response,
        normalized_error_response, ServiceProxy, ACCOUNT_EXPORT_CONTENT_TYPE,
        MAX_PUBLIC_ERROR_CHARS, NORMALIZED_ERROR_RESPONSES,
    };
    use axum::{
        body::{to_bytes, Body},
        extract::{Path, State},
        http::{
            header::{CACHE_CONTROL, CONTENT_TYPE, RETRY_AFTER, WWW_AUTHENTICATE},
            HeaderMap, HeaderValue, StatusCode,
        },
        response::{IntoResponse, Response},
        routing::get,
        Router,
    };
    use bytes::Bytes;
    use futures::{StreamExt, TryStreamExt};
    use std::{io, sync::Arc, time::Duration};
    use tokio::sync::Semaphore;
    use uuid::Uuid;

    const TEST_INTERNAL_TOKEN: &str = "account-export-test-token-32-bytes";

    #[derive(Clone, Default)]
    struct ExportServerState {
        fail_service: Option<&'static str>,
        hang_service: Option<&'static str>,
    }

    async fn export_fragment(
        State(state): State<ExportServerState>,
        Path((service, _user_id)): Path<(String, Uuid)>,
        headers: HeaderMap,
    ) -> Response {
        if headers
            .get("X-Internal-Service-Token")
            .and_then(|value| value.to_str().ok())
            != Some(TEST_INTERNAL_TOKEN)
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        if state.fail_service == Some(service.as_str()) {
            return Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(CONTENT_TYPE, ACCOUNT_EXPORT_CONTENT_TYPE)
                .body(Body::empty())
                .unwrap();
        }

        let mut line = serde_json::to_vec(&serde_json::json!({
            "type": "record",
            "service": service,
            "kind": "fixture",
            "data": {"attacker_text": "line one\nline two"},
        }))
        .unwrap();
        line.push(b'\n');
        let body = if state.hang_service == Some(service.as_str()) {
            Body::from_stream(
                futures::stream::once(async move { Ok::<_, io::Error>(Bytes::from(line)) })
                    .chain(futures::stream::pending()),
            )
        } else {
            Body::from(line)
        };
        Response::builder()
            .header(CONTENT_TYPE, ACCOUNT_EXPORT_CONTENT_TYPE)
            .body(body)
            .unwrap()
    }

    async fn export_server(state: ExportServerState) -> String {
        let app = Router::new()
            .route(
                "/{service}/internal/privacy/users/{user_id}/export",
                get(export_fragment),
            )
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    fn export_proxy(base_url: &str) -> ServiceProxy {
        ServiceProxy {
            user_service_url: format!("{base_url}/user"),
            novel_service_url: format!("{base_url}/novel"),
            agent_service_url: format!("{base_url}/agent"),
            narrative_service_url: format!("{base_url}/narrative"),
            client: reqwest::Client::new(),
            internal_service_token: Arc::from(TEST_INTERNAL_TOKEN),
        }
    }

    #[test]
    fn only_successful_event_streams_use_sse_passthrough() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );

        assert!(is_sse_response(
            "/chat/character/stream",
            StatusCode::OK,
            &headers
        ));
        assert!(!is_sse_response(
            "/chat/character/stream",
            StatusCode::UPGRADE_REQUIRED,
            &headers
        ));

        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        assert!(!is_sse_response(
            "/chat/character/stream",
            StatusCode::OK,
            &headers
        ));
    }

    #[test]
    fn normalized_internal_service_paths_are_not_public_proxy_targets() {
        for path in [
            "/users/../internal/privacy/users/id/export",
            "/users/%2e%2e/internal/privacy/users/id/export",
        ] {
            let url = reqwest::Url::parse(&format!("http://service{path}")).unwrap();
            assert!(is_internal_service_path(url.path()));
        }
        assert!(!is_internal_service_path("/users/internal/preferences"));
    }

    #[tokio::test]
    async fn nonconforming_errors_are_normalized_without_leaking_server_details() {
        let response = normalized_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &HeaderMap::new(),
            br#"{"error":"database password leaked"}"#,
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(has_stable_error_envelope(&body));
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "internal_error");
        assert!(!body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("password"));

        let response = normalized_error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &HeaderMap::new(),
            br#"{"error":"title is required"}"#,
        );
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "validation_error");
        assert_eq!(body["error"]["message"], "title is required");
    }
    /// The independently pinned normalization contract. NORMALIZED_ERROR_RESPONSES
    /// must equal this list exactly (same rows, same order), so adding, removing,
    /// or altering a mapping without updating the contract fails the build.
    const EXPECTED_NORMALIZED_ERROR_RESPONSES: &[(StatusCode, &str, &str)] = &[
        (
            StatusCode::BAD_REQUEST,
            "validation_error",
            "Request validation failed",
        ),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            "Request validation failed",
        ),
        (
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Authentication is required",
        ),
        (StatusCode::FORBIDDEN, "forbidden", "Access is forbidden"),
        (StatusCode::NOT_FOUND, "not_found", "Resource not found"),
        (
            StatusCode::CONFLICT,
            "conflict",
            "Request conflicts with existing state",
        ),
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "Request body is too large",
        ),
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Unsupported media type",
        ),
        (
            StatusCode::UPGRADE_REQUIRED,
            "client_upgrade_required",
            "Client upgrade required",
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Rate limit exceeded",
        ),
        (
            StatusCode::BAD_GATEWAY,
            "bad_gateway",
            "Upstream service returned an invalid response",
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            "Service is temporarily unavailable",
        ),
        (
            StatusCode::GATEWAY_TIMEOUT,
            "service_unavailable",
            "Service is temporarily unavailable",
        ),
    ];

    #[tokio::test]
    async fn normalization_table_matches_the_pinned_contract_exactly() {
        assert_eq!(
            NORMALIZED_ERROR_RESPONSES,
            EXPECTED_NORMALIZED_ERROR_RESPONSES
        );
        for &(status, code, message) in EXPECTED_NORMALIZED_ERROR_RESPONSES {
            let response =
                normalized_error_response(status, &HeaderMap::new(), b"not a json envelope");
            assert_eq!(response.status(), status, "status for {code}");
            assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(has_stable_error_envelope(&body), "envelope for {code}");
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["error"]["code"], code, "code for {status}");
            assert_eq!(body["error"]["message"], message, "message for {status}");
        }
    }

    #[tokio::test]
    async fn unmapped_statuses_fall_back_to_request_or_internal_error() {
        let response = normalized_error_response(
            StatusCode::IM_A_TEAPOT,
            &HeaderMap::new(),
            b"<html>not json</html>",
        );
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "request_error");
        assert_eq!(body["error"]["message"], "Request failed");

        let response = normalized_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &HeaderMap::new(),
            br#"{"error":"database password leaked"}"#,
        );
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "internal_error");
        assert_eq!(body["error"]["message"], "Request failed");
    }

    #[tokio::test]
    async fn stable_envelopes_pass_through_with_error_headers_intact() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("30"));
        headers.insert(
            WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"novelworld\""),
        );
        let body = br#"{"error":{"code":"unauthorized","message":"Invalid token"}}"#;
        let response = normalized_error_response(StatusCode::UNAUTHORIZED, &headers, body);
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers().get(RETRY_AFTER).unwrap(), "30");
        assert_eq!(
            response.headers().get(WWW_AUTHENTICATE).unwrap(),
            "Bearer realm=\"novelworld\""
        );
        let out_body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(out_body.as_ref(), body);

        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("1"));
        let response = normalized_error_response(StatusCode::TOO_MANY_REQUESTS, &headers, b"boom");
        assert_eq!(response.headers().get(RETRY_AFTER).unwrap(), "1");
    }

    #[tokio::test]
    async fn extracted_client_error_messages_are_truncated_to_the_public_limit() {
        let long_message = "x".repeat(MAX_PUBLIC_ERROR_CHARS + 100);
        let body = serde_json::to_vec(&serde_json::json!({
            "error": {"message": long_message.clone()}
        }))
        .unwrap();
        let response =
            normalized_error_response(StatusCode::UNPROCESSABLE_ENTITY, &HeaderMap::new(), &body);
        let out_body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let out_body: serde_json::Value = serde_json::from_slice(&out_body).unwrap();
        assert_eq!(out_body["error"]["code"], "validation_error");
        let message = out_body["error"]["message"].as_str().unwrap();
        assert_eq!(message.chars().count(), MAX_PUBLIC_ERROR_CHARS);
        assert!(long_message.starts_with(message));
    }

    #[tokio::test]
    async fn account_export_has_fixed_order_safe_json_and_terminal_completion() {
        let proxy = export_proxy(&export_server(ExportServerState::default()).await);
        let permit = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
        let response = proxy.account_export(Uuid::new_v4(), permit).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            ACCOUNT_EXPORT_CONTENT_TYPE
        );
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(!response.headers().contains_key("content-length"));
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let records: Vec<serde_json::Value> = body
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        let events: Vec<String> = records
            .iter()
            .map(|record| match record["type"].as_str().unwrap() {
                "service_start" | "service_complete" | "record" => format!(
                    "{}:{}",
                    record["type"].as_str().unwrap(),
                    record["service"].as_str().unwrap()
                ),
                event => event.into(),
            })
            .collect();
        assert_eq!(
            events,
            [
                "manifest",
                "service_start:user",
                "record:user",
                "service_complete:user",
                "service_start:novel",
                "record:novel",
                "service_complete:novel",
                "service_start:agent",
                "record:agent",
                "service_complete:agent",
                "service_start:narrative",
                "record:narrative",
                "service_complete:narrative",
                "complete",
            ]
        );
        assert_eq!(records[2]["data"]["attacker_text"], "line one\nline two");
    }

    #[tokio::test]
    async fn failed_fragment_omits_terminal_completion() {
        let proxy = export_proxy(
            &export_server(ExportServerState {
                fail_service: Some("narrative"),
                hang_service: None,
            })
            .await,
        );
        let permit = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
        let response = proxy.account_export(Uuid::new_v4(), permit).await;
        let mut stream = response.into_body().into_data_stream();
        let mut received = Vec::new();
        let mut failed = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => received.extend_from_slice(&chunk),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }

        let received = String::from_utf8(received).unwrap();
        assert!(failed);
        let records: Vec<serde_json::Value> = received
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(records.iter().any(|record| {
            record["type"] == "service_complete" && record["service"] == "agent"
        }));
        assert!(!records.iter().any(|record| record["type"] == "complete"));
    }

    #[tokio::test]
    async fn account_export_streams_before_fragment_end_and_releases_disconnect_permit() {
        let proxy = export_proxy(
            &export_server(ExportServerState {
                fail_service: None,
                hang_service: Some("novel"),
            })
            .await,
        );
        let permits = Arc::new(Semaphore::new(1));
        let permit = permits.clone().try_acquire_owned().unwrap();
        let response = proxy.account_export(Uuid::new_v4(), permit).await;
        let mut stream = response.into_body().into_data_stream();
        let mut received = Vec::new();

        tokio::time::timeout(Duration::from_secs(2), async {
            while !String::from_utf8_lossy(&received).contains(r#""service":"novel""#) {
                received.extend_from_slice(&stream.try_next().await.unwrap().unwrap());
            }
        })
        .await
        .expect("streaming export must expose a novel fragment before it completes");
        drop(stream);
        assert_eq!(permits.available_permits(), 1);
    }
}
