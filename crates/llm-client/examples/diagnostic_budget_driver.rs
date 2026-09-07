use anyhow::Result;
use futures::StreamExt;
use serde::Deserialize;

use llm_client::{
    diagnostic_budget::{self as budget, BudgetControlError, BudgetEvidenceError},
    ChatRequest, ChatStreamEvent, EmbeddingRequest, LlmClient, LlmOperation, RuntimeLlmClient,
};

const SYNTHETIC_KEY: &str = "test-only";

#[derive(Deserialize)]
struct Profile {
    model: String,
    origin: String,
}

enum Client {
    Direct(LlmClient),
    Configured(RuntimeLlmClient),
}

impl Client {
    async fn chat(&self, request: ChatRequest) -> Result<llm_client::ChatResponse> {
        match self {
            Self::Direct(client) => client.chat(request).await,
            Self::Configured(client) => client.chat(request).await,
        }
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<llm_client::ChatStream> {
        match self {
            Self::Direct(client) => client.chat_stream(request).await,
            Self::Configured(client) => client.chat_stream(request).await,
        }
    }

    async fn embed(&self, request: EmbeddingRequest) -> Result<llm_client::EmbeddingResponse> {
        match self {
            Self::Direct(client) => client.embed(request).await,
            Self::Configured(_) => panic!("embedding requires direct client"),
        }
    }
}

fn assert_expected_error(error: anyhow::Error, expected: &str) {
    match expected {
        "control" => assert!(error.downcast_ref::<BudgetControlError>().is_some()),
        "evidence" => assert!(error.downcast_ref::<BudgetEvidenceError>().is_some()),
        _ => panic!("unexpected isolated driver expectation"),
    }
}

fn request(profile: &Profile) -> ChatRequest {
    ChatRequest::new(LlmOperation::SetupConnection, profile.model.clone())
        .message("user", "synthetic test")
        .max_tokens(8)
        .thinking(false)
}

#[tokio::main]
async fn main() -> Result<()> {
    assert_eq!(
        std::env::var("NOVELWORLD_TEST_DIAGNOSTIC_ISOLATED")
            .ok()
            .as_deref(),
        Some("1")
    );
    assert_eq!(
        std::env::var("HTTPS_PROXY").ok().as_deref(),
        Some("http://mock:3128")
    );
    assert_eq!(
        std::env::var("NO_PROXY").ok().as_deref(),
        Some("owner,mock,postgres,127.0.0.1,localhost")
    );

    let client_mode = std::env::var("NOVELWORLD_TEST_BUDGET_CLIENT")?;
    let budget_mode = std::env::var("NOVELWORLD_TEST_BUDGET_MODE")?;
    let expected =
        std::env::var("NOVELWORLD_TEST_BUDGET_EXPECT").unwrap_or_else(|_| "success".into());
    let profile: Profile = serde_json::from_str(budget::PROFILE_JSON)?;
    let client = match client_mode.as_str() {
        "direct" => Client::Direct(LlmClient::new().with_openai_compatible(
            "deepseek",
            SYNTHETIC_KEY,
            profile.origin.clone(),
        )),
        "static" => Client::Configured(RuntimeLlmClient::static_config(
            profile.origin.clone(),
            profile.model.clone(),
            SYNTHETIC_KEY.into(),
            false,
        )),
        "runtime" => Client::Configured(RuntimeLlmClient::from_env()?),
        _ => panic!("unknown isolated client mode"),
    };

    if budget_mode == "embed" {
        assert_eq!(client_mode, "direct");
        assert_expected_error(
            client
                .embed(EmbeddingRequest {
                    model: "embedding".into(),
                    input: "synthetic".into(),
                })
                .await
                .unwrap_err(),
            "control",
        );
        return Ok(());
    }

    let mut request = request(&profile);
    if budget_mode == "sync"
        && std::env::var("NOVELWORLD_TEST_BUDGET_JSON").as_deref() == Ok("true")
    {
        request.json_mode = true;
    }
    match budget_mode.as_str() {
        "sync" => match client.chat(request).await {
            Ok(response) => {
                assert_eq!(expected, "success");
                assert_eq!(response.content, "OK");
            }
            Err(error) => assert_expected_error(error, &expected),
        },
        "stream" => {
            let mut stream = match client.chat_stream(request).await {
                Ok(stream) => stream,
                Err(error) => {
                    assert_expected_error(error, &expected);
                    return Ok(());
                }
            };
            let mut finished = 0;
            let mut content = String::new();
            let mut stream_error = None;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(ChatStreamEvent::Finished) => finished += 1,
                    Ok(ChatStreamEvent::Delta(delta)) => content.push_str(&delta),
                    Ok(_) => {}
                    Err(error) => {
                        stream_error = Some(error);
                        break;
                    }
                }
            }
            if expected == "success" {
                assert_eq!(finished, 1);
                assert_eq!(content, "OK");
                assert!(stream_error.is_none());
            } else {
                assert_eq!(finished, 0);
                assert_expected_error(stream_error.unwrap(), &expected);
            }
        }
        "drop" => {
            let mut stream = client.chat_stream(request).await?;
            while let Some(item) = stream.next().await {
                if matches!(item, Ok(ChatStreamEvent::Delta(_))) {
                    drop(stream);
                    return Ok(());
                }
            }
            panic!("isolated stream produced no delta");
        }
        _ => panic!("unknown isolated budget mode"),
    }
    Ok(())
}
