use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::{redirect::Policy, Client, Url};
use serde_json::{json, Map, Value};
use std::time::Duration;
use tokio::sync::Semaphore;

use crate::domain::entities::world_session::WorldActionKind;
use crate::domain::ports::ActionSuggestionPort;

const RESPONSE_LIMIT: usize = 16 * 1024;
const MIN_SUGGESTION_PROBABILITY: f64 = 0.6;
const OPTIONS: [(WorldActionKind, &str, &str); 7] = [
    (WorldActionKind::Travel, "A", "前往一个具体地点"),
    (
        WorldActionKind::Investigate,
        "B",
        "调查已有线索、地点或事件",
    ),
    (WorldActionKind::Converse, "C", "与一个角色交谈"),
    (WorldActionKind::Ally, "D", "争取与一个角色结盟"),
    (WorldActionKind::Oppose, "E", "反对一个角色"),
    (WorldActionKind::AdvanceThread, "F", "推进已经存在的事件线"),
    (
        WorldActionKind::PursueGoal,
        "G",
        "追求玩家自己设定的长期目标",
    ),
];

pub struct LayaActionSuggester {
    client: Client,
    endpoint: Url,
    api_key: String,
    admission: Semaphore,
}

impl LayaActionSuggester {
    pub fn new(api_url: &str, api_key: String) -> Result<Self> {
        let mut endpoint = Url::parse(api_url).context("Invalid LAYA_API_URL")?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.path() != "/"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || api_key.is_empty()
        {
            bail!("Invalid Laya action suggestion configuration");
        }
        endpoint.set_path("/v1/systemone");
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_millis(300))
                .timeout(Duration::from_secs(2))
                .redirect(Policy::none())
                .build()?,
            endpoint,
            api_key,
            admission: Semaphore::new(4),
        })
    }
}

#[async_trait]
impl ActionSuggestionPort for LayaActionSuggester {
    async fn suggest(
        &self,
        intent: &str,
        available: &[WorldActionKind],
    ) -> Result<Option<WorldActionKind>> {
        let _permit = self
            .admission
            .try_acquire()
            .context("Laya suggestion is busy")?;
        let criteria: Map<String, Value> = OPTIONS
            .iter()
            .filter(|(kind, _, _)| available.contains(kind))
            .map(|(_, key, description)| ((*key).into(), (*description).into()))
            .collect();
        if criteria.len() < 2 {
            return Ok(None);
        }
        let payload = json!({
            "model": "multilingual",
            "state": {"intent": intent},
            "questions": {"action": {
                "type": "choice",
                "instructions": "只判断玩家在这句话中明确想做的主要动作；不要推测后续目标。",
                "criteria": criteria,
            }},
        });
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
                bail!("Laya suggestion response exceeded the size limit");
            }
            body.extend_from_slice(&chunk);
        }
        parse_suggestion(&body, available)
    }
}

fn parse_suggestion(body: &[u8], available: &[WorldActionKind]) -> Result<Option<WorldActionKind>> {
    let value: Value = serde_json::from_slice(body)?;
    let answer = value
        .pointer("/answers/action")
        .context("Missing Laya answer")?;
    let choice = answer["choice"].as_str().context("Missing Laya choice")?;
    let probability = answer["probabilities"][choice]
        .as_f64()
        .context("Missing Laya choice probability")?;
    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
        bail!("Invalid Laya choice probability");
    }
    let kind = OPTIONS
        .iter()
        .find(|(_, key, _)| *key == choice)
        .map(|(kind, _, _)| *kind)
        .filter(|kind| available.contains(kind))
        .context("Laya returned an unavailable action")?;
    Ok((probability >= MIN_SUGGESTION_PROBABILITY).then_some(kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::HeaderMap, response::Redirect, routing::post, Json, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn accepts_only_bounded_available_decisions() {
        let available = [WorldActionKind::Travel, WorldActionKind::Converse];
        let answer = br#"{"answers":{"action":{"choice":"C","probabilities":{"C":0.8}}}}"#;
        assert_eq!(
            parse_suggestion(answer, &available).unwrap(),
            Some(WorldActionKind::Converse)
        );
        let unavailable = br#"{"answers":{"action":{"choice":"G","probabilities":{"G":0.99}}}}"#;
        assert!(parse_suggestion(unavailable, &available).is_err());
        let uncertain = br#"{"answers":{"action":{"choice":"C","probabilities":{"C":0.5}}}}"#;
        assert_eq!(parse_suggestion(uncertain, &available).unwrap(), None);
        assert!(parse_suggestion(b"{}", &available).is_err());
    }

    #[tokio::test]
    async fn sends_only_intent_and_allowed_choices_to_the_protected_endpoint() {
        let app = Router::new().route(
            "/v1/systemone",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers["authorization"], "Bearer local-test-key");
                assert_eq!(body["model"], "multilingual");
                assert_eq!(body["state"], json!({"intent": "查看脚印"}));
                assert!(body["questions"]["action"]["criteria"].get("B").is_some());
                assert!(body["questions"]["action"]["criteria"].get("G").is_none());
                Json(json!({"answers": {"action": {
                    "choice": "B", "probabilities": {"B": 0.8}
                }}}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client =
            LayaActionSuggester::new(&format!("http://{address}"), "local-test-key".into())
                .unwrap();
        let result = client
            .suggest(
                "查看脚印",
                &[WorldActionKind::Travel, WorldActionKind::Investigate],
            )
            .await
            .unwrap();
        server.abort();
        assert_eq!(result, Some(WorldActionKind::Investigate));
    }

    #[tokio::test]
    async fn refuses_redirects_before_forwarding_private_intent() {
        let visits = Arc::new(AtomicUsize::new(0));
        let trap_visits = visits.clone();
        let app = Router::new()
            .route(
                "/v1/systemone",
                post(|| async { Redirect::temporary("/trap") }),
            )
            .route(
                "/trap",
                post(move || {
                    let visits = trap_visits.clone();
                    async move {
                        visits.fetch_add(1, Ordering::SeqCst);
                        "redirected"
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client =
            LayaActionSuggester::new(&format!("http://{address}"), "test-key".into()).unwrap();

        assert!(client
            .suggest(
                "私人意图",
                &[WorldActionKind::Travel, WorldActionKind::Investigate]
            )
            .await
            .is_err());
        assert_eq!(visits.load(Ordering::SeqCst), 0);
        server.abort();
    }
}
