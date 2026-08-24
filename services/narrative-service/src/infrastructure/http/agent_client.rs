use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;
use uuid::Uuid;

use crate::domain::ports::AgentMemoryPort;

const AGENT_SERVICE_TIMEOUT: Duration = Duration::from_secs(2);

/// HTTP adapter that writes permanent memories on agent-service.
/// Only the journey-memory producer endpoint is exposed.
pub struct AgentServiceClient {
    client: Client,
    base_url: String,
    internal_service_token: String,
}

impl AgentServiceClient {
    pub fn new(base_url: String, internal_service_token: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(AGENT_SERVICE_TIMEOUT)
                .build()
                .expect("valid agent-service HTTP client configuration"),
            base_url,
            internal_service_token,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_memory(
        &self,
        url: &str,
        memory_id: Uuid,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        event: &str,
        importance: i32,
    ) -> Result<reqwest::Response> {
        let payload = serde_json::json!({
            "memory_id": memory_id,
            "character_id": character_id,
            "user_id": user_id,
            "novel_id": novel_id,
            "chapter_number": chapter_number,
            "event": event,
            "importance": importance,
        });
        self.client
            .post(url)
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .json(&payload)
            .send()
            .await
            .map_err(|error| anyhow!("Failed to reach agent-service at {}: {}", url, error))
    }
}

#[derive(serde::Deserialize)]
struct SaveMemoryAcknowledgement {
    saved: bool,
}

#[async_trait]
impl AgentMemoryPort for AgentServiceClient {
    async fn save_permanent_memory(
        &self,
        memory_id: Uuid,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        event: &str,
        importance: i32,
    ) -> Result<()> {
        let url = format!("{}/internal/memories", self.base_url);
        let resp = self
            .send_memory(
                &url,
                memory_id,
                character_id,
                user_id,
                novel_id,
                chapter_number,
                event,
                importance,
            )
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "agent-service returned {} for permanent memory write",
                resp.status()
            ));
        }
        let acknowledgement = resp
            .json::<SaveMemoryAcknowledgement>()
            .await
            .map_err(|error| {
                anyhow!("agent-service returned an invalid memory acknowledgement: {error}")
            })?;
        if !acknowledgement.saved {
            return Err(anyhow!(
                "agent-service did not acknowledge the permanent fact"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::handlers::journey_memory_id;
    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    async fn client_for(app: Router) -> AgentServiceClient {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        AgentServiceClient::new(format!("http://{address}"), "test-token".into())
    }

    #[tokio::test]
    async fn sends_the_stable_permanent_memory_contract_once() {
        let source_turn_id = Uuid::parse_str("10000000-0000-4000-8000-000000000001").unwrap();
        let character_id = Uuid::parse_str("20000000-0000-4000-8000-000000000002").unwrap();
        let user_id = Uuid::parse_str("30000000-0000-4000-8000-000000000003").unwrap();
        let novel_id = Uuid::parse_str("40000000-0000-4000-8000-000000000004").unwrap();
        let memory_id = journey_memory_id(source_turn_id);
        let event = serde_json::json!({
            "schema_version": 2,
            "source": "committed_world_turn",
            "authority": "explicit_character_witness_facts",
            "source_turn_id": source_turn_id,
            "witness_character_id": character_id,
            "turn_number": 1,
            "world_time": 1,
            "canonical_checkpoint_chapter": 1,
            "change_counts": {"events": 0, "relationships": 0, "reader_action": 1},
            "committed_changes": {},
            "reader_action": {
                "kind": "converse",
                "target_id": character_id,
            },
        })
        .to_string();
        let expected_payload = Arc::new(serde_json::json!({
            "memory_id": memory_id,
            "character_id": character_id,
            "user_id": user_id,
            "novel_id": novel_id,
            "chapter_number": 1,
            "event": event,
            "importance": 7,
        }));
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/internal/memories",
                post(
                    |State((calls, expected_payload)): State<(
                        Arc<AtomicUsize>,
                        Arc<serde_json::Value>,
                    )>,
                     headers: HeaderMap,
                     Json(payload): Json<serde_json::Value>| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        assert_eq!(
                            headers
                                .get("x-internal-service-token")
                                .and_then(|value| value.to_str().ok()),
                            Some("test-token")
                        );
                        assert_eq!(payload, *expected_payload);
                        (StatusCode::OK, Json(serde_json::json!({"saved": true}))).into_response()
                    },
                ),
            )
            .with_state((calls.clone(), expected_payload));
        let client = client_for(app).await;

        client
            .save_permanent_memory(memory_id, character_id, user_id, novel_id, 1, &event, 7)
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn saved_false_is_not_a_permanent_fact_acknowledgement() {
        let app = Router::new().route(
            "/internal/memories",
            post(|| async {
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "saved": false,
                        "reason": "embedding dimension policy"
                    })),
                )
            }),
        );
        let client = client_for(app).await;

        assert!(client
            .save_permanent_memory(
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
                1,
                "structured fact",
                8,
            )
            .await
            .is_err());
    }
}
