use anyhow::Result;
use async_trait::async_trait;
use llm_client::TraceContextExt;
use std::time::Duration;
use uuid::Uuid;

use crate::domain::ports::PrivacyCleanupPort;

pub struct AgentPrivacyClient {
    base_url: String,
    internal_service_token: String,
    client: reqwest::Client,
}

impl AgentPrivacyClient {
    pub fn new(base_url: String, internal_service_token: String) -> Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            internal_service_token,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()?,
        })
    }
}

#[async_trait]
impl PrivacyCleanupPort for AgentPrivacyClient {
    async fn clear_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()> {
        let response = self
            .client
            .delete(format!(
                "{}/internal/privacy/users/{user_id}/novels/{novel_id}",
                self.base_url
            ))
            .with_trace_context()
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("agent privacy cleanup returned {}", response.status());
        }
        Ok(())
    }

    async fn allow_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()> {
        let response = self
            .client
            .delete(format!(
                "{}/internal/privacy/tombstones/users/{user_id}/novels/{novel_id}",
                self.base_url
            ))
            .with_trace_context()
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("agent privacy rollback returned {}", response.status());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::HeaderMap, routing::delete, Router};
    use std::sync::{Arc, Mutex};
    use tracing::Instrument;
    use tracing_subscriber::layer::SubscriberExt;

    #[tokio::test]
    async fn privacy_calls_forward_the_trace_id_only_inside_a_request_span() {
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(llm_client::TraceContextLayer),
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        let app = Router::new().route(
            "/internal/privacy/users/{user_id}/novels/{novel_id}",
            delete(move |headers: HeaderMap| async move {
                recorded.lock().unwrap().push(
                    headers
                        .get("x-trace-id")
                        .map(|value| value.to_str().unwrap().to_owned()),
                );
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = AgentPrivacyClient::new(format!("http://{address}"), "token".into()).unwrap();
        client
            .clear_novel(Uuid::nil(), Uuid::nil())
            .instrument(tracing::info_span!("request", trace_id = "novel-trace"))
            .await
            .unwrap();
        client.clear_novel(Uuid::nil(), Uuid::nil()).await.unwrap();
        assert_eq!(*seen.lock().unwrap(), [Some("novel-trace".into()), None]);
        server.abort();
    }
}
