use axum::{
    body::Body,
    extract::{Request, State},
    http::{
        header::{CONTENT_TYPE, RETRY_AFTER, WWW_AUTHENTICATE},
        HeaderMap, StatusCode,
    },
    response::{IntoResponse, Response},
};
use futures::TryStreamExt;
use reqwest::Client;

use crate::AppState;

const MAX_PROXY_BODY_BYTES: usize = 21 * 1024 * 1024;
const MAX_PUBLIC_ERROR_CHARS: usize = 512;

pub fn api_error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({
            "error": {"code": code, "message": message}
        })),
    )
        .into_response()
}

fn error_code(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => "validation_error",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::CONFLICT => "conflict",
        StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "unsupported_media_type",
        StatusCode::UPGRADE_REQUIRED => "client_upgrade_required",
        StatusCode::TOO_MANY_REQUESTS => "rate_limited",
        StatusCode::BAD_GATEWAY => "bad_gateway",
        StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT => "service_unavailable",
        _ if status.is_server_error() => "internal_error",
        _ => "request_error",
    }
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

    match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => "Request validation failed",
        StatusCode::UNAUTHORIZED => "Authentication is required",
        StatusCode::FORBIDDEN => "Access is forbidden",
        StatusCode::NOT_FOUND => "Resource not found",
        StatusCode::CONFLICT => "Request conflicts with existing state",
        StatusCode::PAYLOAD_TOO_LARGE => "Request body is too large",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "Unsupported media type",
        StatusCode::UPGRADE_REQUIRED => "Client upgrade required",
        StatusCode::TOO_MANY_REQUESTS => "Rate limit exceeded",
        StatusCode::BAD_GATEWAY => "Upstream service returned an invalid response",
        StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT => {
            "Service is temporarily unavailable"
        }
        _ => "Request failed",
    }
    .into()
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
    async fn forward(&self, target_base: &str, original_path: &str, request: Request) -> Response {
        let method = request.method().clone();
        let headers = request.headers().clone();
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

        let target_url = format!("{}{}", target_base, original_path);

        let mut req_builder = self.client.request(method, &target_url);

        for (key, value) in &headers {
            if key == "host" {
                continue;
            }
            req_builder = req_builder.header(key, value);
        }

        match req_builder.body(body).send().await {
            Ok(resp) => {
                let status = resp.status();
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
    use super::{has_stable_error_envelope, is_sse_response, normalized_error_response};
    use axum::body::to_bytes;
    use axum::http::{header::CONTENT_TYPE, HeaderMap, HeaderValue, StatusCode};

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
}
