use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures::TryStreamExt;
use reqwest::Client;

use crate::AppState;

pub struct ServiceProxy {
    pub novel_service_url: String,
    pub agent_service_url: String,
    pub narrative_service_url: String,
    pub user_service_url: String,
    pub client: Client,
}

impl ServiceProxy {
    async fn forward(
        &self,
        target_base: &str,
        original_path: &str,
        request: Request,
    ) -> Response {
        let method = request.method().clone();
        let headers = request.headers().clone();
        let is_sse = original_path.contains("/stream");

        let body = match axum::body::to_bytes(request.into_body(), 20 * 1024 * 1024).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("Failed to read request body: {}", e);
                return (StatusCode::BAD_REQUEST, format!("Failed to read request body: {}", e)).into_response();
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

                if is_sse {
                    let byte_stream = resp.bytes_stream()
                        .map_err(|e| std::io::Error::other(e));
                    let body = Body::from_stream(byte_stream);

                    let mut response = Response::builder()
                        .status(status.as_u16())
                        .header("Content-Type", "text/event-stream")
                        .header("Cache-Control", "no-cache")
                        .header("X-Accel-Buffering", "no")
                        .header("Connection", "keep-alive");

                    for (key, value) in &resp_headers {
                        let k = key.as_str();
                        if k != "content-length" && k != "content-type" && k != "transfer-encoding" {
                            response = response.header(key, value);
                        }
                    }

                    response.body(body)
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
                } else {
                    let resp_body = match resp.bytes().await {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::error!("Failed to read response from {}: {}", target_url, e);
                            return (StatusCode::BAD_GATEWAY, format!("Service response error: {}", e)).into_response();
                        }
                    };
                    let mut response = Response::builder().status(status.as_u16());
                    for (key, value) in &resp_headers {
                        response = response.header(key, value);
                    }
                    response.body(Body::from(resp_body))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
                }
            }
            Err(e) => {
                tracing::error!("Proxy error to {}: {}", target_url, e);
                (StatusCode::BAD_GATEWAY, format!("Service unavailable: {}", e))
                    .into_response()
            }
        }
    }
}

pub async fn forward_to_novel(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let path_and_query = request.uri().path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let stripped = path_and_query.strip_prefix("/api").unwrap_or(&path_and_query);
    state.proxy.forward(&state.proxy.novel_service_url, stripped, request).await
}

pub async fn forward_to_agent(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let path_and_query = request.uri().path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let stripped = path_and_query.strip_prefix("/api").unwrap_or(&path_and_query);
    state.proxy.forward(&state.proxy.agent_service_url, stripped, request).await
}

pub async fn forward_to_narrative(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let path_and_query = request.uri().path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let stripped = path_and_query.strip_prefix("/api").unwrap_or(&path_and_query);
    state.proxy.forward(&state.proxy.narrative_service_url, stripped, request).await
}

pub async fn forward_to_user(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let path_and_query = request.uri().path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let stripped = path_and_query.strip_prefix("/api").unwrap_or(&path_and_query);
    state.proxy.forward(&state.proxy.user_service_url, stripped, request).await
}
