use crate::retry::RetryPolicy;
use crate::{
    providers::{anthropic, openai, sse},
    ChatRequest, ChatStreamEvent, EmbeddingRequest, LlmClient,
};
use anyhow::Result;
use bytes::Bytes;
use futures::{stream, StreamExt};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

fn http_response(status: &str, content_type: &str, body: &str, extra: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[test]
fn series_matching_dispatches_once_on_http_and_empty_json_failures() {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for (status, body) in [
        ("503 Service Unavailable", "{}"),
        (
            "200 OK",
            r#"{"choices":[{"message":{"content":""},"finish_reason":"stop"}],"model":"model","usage":{"prompt_tokens":5,"completion_tokens":0}}"#,
        ),
        (
            "200 OK",
            r#"{"choices":[{"message":{"content":"partial"},"finish_reason":"length"}],"model":"model","usage":{"prompt_tokens":5,"completion_tokens":512}}"#,
        ),
        (
            "200 OK",
            r#"{"choices":[{"message":{"content":"{}"},"finish_reason":"stop"}],"model":"model","usage":{"prompt_tokens":5,"completion_tokens":700}}"#,
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let counted = calls.clone();
        let stop = done.clone();
        let response = http_response(status, "application/json", body, "");
        let server = thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut request = [0; 8192];
                        let read = socket.read(&mut request).unwrap();
                        assert!(String::from_utf8_lossy(&request[..read])
                            .starts_with("POST /v1/chat/completions"));
                        counted.fetch_add(1, Ordering::SeqCst);
                        socket.write_all(&response).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("mock accept failed: {error}"),
                }
            }
        });
        let client = LlmClient::new().with_openai_compatible(
            "deepseek",
            "synthetic",
            format!("http://{address}"),
        );
        let mut request =
            crate::production_json_request(crate::LlmOperation::SeriesMatching, "synthetic")
                .runtime_user_id("synthetic");
        request.model = "deepseek/model".into();
        request.thinking = Some(true);
        let failure = runtime.block_on(client.chat(request)).unwrap_err();
        done.store(true, Ordering::SeqCst);
        server.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        if status == "200 OK" {
            let rendered = handle.render();
            assert_eq!(
                metric_value(
                    &rendered,
                    "novelworld_llm_usage_reports_total",
                    &[("operation", "series_matching"), ("status", "present")]
                ),
                1.0
            );
            assert_eq!(
                metric_value(
                    &rendered,
                    "novelworld_llm_billable_tokens_total",
                    &[
                        ("operation", "series_matching"),
                        ("class", "uncached_input")
                    ]
                ),
                5.0
            );
            if body.contains("length") {
                assert!(failure
                    .chain()
                    .any(|cause| cause.is::<crate::TruncatedCompletion>()));
            }
            if body.contains("700") {
                assert_eq!(
                    metric_value(
                        &rendered,
                        "novelworld_llm_billable_tokens_total",
                        &[("operation", "series_matching"), ("class", "output")]
                    ),
                    700.0
                );
            }
        }
        let unsupported = LlmClient::new().with_openai_compatible(
            "openai",
            "synthetic",
            format!("http://{address}"),
        );
        let error = runtime
            .block_on(
                unsupported.chat(
                    ChatRequest::new(crate::LlmOperation::SeriesMatching, "openai/model")
                        .max_tokens(512)
                        .thinking(false),
                ),
            )
            .unwrap_err();
        assert!(error.is::<crate::UnsupportedSeriesProvider>());
    }
}

fn metric_value(rendered: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    rendered
        .lines()
        .filter(|line| line.starts_with(name) && !line.starts_with('#'))
        .find(|line| {
            labels
                .iter()
                .all(|(key, value)| line.contains(&format!(r#"{key}="{value}""#)))
        })
        .and_then(|line| line.rsplit_once(' '))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0.0)
}

async fn decode<P>(chunks: Vec<Vec<u8>>, parse: P) -> Vec<Result<ChatStreamEvent>>
where
    P: FnMut(sse::SseFrame) -> Result<Vec<ChatStreamEvent>> + Send + 'static,
{
    sse::decode_stream(
        stream::iter(
            chunks
                .into_iter()
                .map(|chunk| Ok::<_, std::io::Error>(Bytes::from(chunk))),
        ),
        parse,
    )
    .collect()
    .await
}

fn claude_response() -> serde_json::Value {
    serde_json::json!({
        "type":"message", "role":"assistant", "model":"claude-sonnet-5-5", "stop_reason":"end_turn",
        "content":[{"type":"thinking","thinking":"private-reasoning"},{"type":"text","text":"世界"}],
        "usage":{"input_tokens":3,"output_tokens":5,"cache_creation_input_tokens":4,"cache_read_input_tokens":5}
    })
}

fn claude_frames() -> Vec<(&'static str, serde_json::Value)> {
    use serde_json::json;
    let mut message = claude_response();
    message["content"] = json!([]);
    message["stop_reason"] = serde_json::Value::Null;
    message["usage"]["output_tokens"] = json!(1);
    vec![
        ("future_event", json!({"type":"future_event"})),
        (
            "message_start",
            json!({"type":"message_start","message":message}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"private-reasoning"}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"opaque-signature"}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
        ),
        ("ping", json!({"type":"ping"})),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"世界"}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":1}),
        ),
        (
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":null},"usage":{"output_tokens":3}}),
        ),
        (
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":4,"cache_creation_input_tokens":6,"cache_read_input_tokens":7,"output_tokens":5}}),
        ),
        ("message_stop", json!({"type":"message_stop"})),
    ]
}

fn encode_claude_frames(frames: &[(&str, serde_json::Value)]) -> Vec<u8> {
    frames
        .iter()
        .map(|(event, value)| format!("event: {event}\r\ndata: {value}\r\n\r\n"))
        .collect::<String>()
        .into_bytes()
}

#[test]
fn claude_native_request_and_completion_preserve_prompts_usage_and_truncation() {
    use serde_json::json;
    let request = ChatRequest::new(crate::LlmOperation::CharacterChat, "claude-sonnet-5-5")
        .message("system", "initial instruction")
        .message("developer", "second instruction")
        .message("user", "Hello")
        .message("assistant", "Hi")
        .message("user", "Continue")
        .max_tokens(32)
        .temperature(0.8)
        .thinking(false)
        .json();
    let body = anthropic::request_body(&request, false).unwrap();
    assert!(body["system"]
        .as_str()
        .unwrap()
        .starts_with("initial instruction\n\nsecond instruction\n\n"));
    assert!(body["system"].as_str().unwrap().contains("JSON object"));
    assert_eq!(body["messages"].as_array().unwrap().len(), 3);
    assert_eq!(body["max_tokens"], 32);
    for field in [
        "temperature",
        "thinking",
        "response_format",
        "stream_options",
    ] {
        assert!(body.get(field).is_none());
    }
    assert!(anthropic::request_body(
        &request.clone().message("system", "late instruction"),
        false
    )
    .is_err());
    assert!(
        anthropic::request_body(&request.clone().message("tool", "unsupported tool"), false)
            .is_err()
    );
    let response = anthropic::response(claude_response()).unwrap();
    assert_eq!(response.content, "世界");
    assert_eq!(response.model, "claude-sonnet-5-5");
    assert_eq!(
        response.usage,
        Some(crate::Usage::new(12, 5, Some(5)).unwrap())
    );
    let mut truncated = claude_response();
    truncated["content"] = json!([]);
    truncated["stop_reason"] = json!("max_tokens");
    assert!(anthropic::response(truncated)
        .unwrap_err()
        .is::<crate::TruncatedCompletion>());
    for (field, value) in [
        ("model", json!("")),
        ("usage", serde_json::Value::Null),
        ("usage", json!({"input_tokens":3,"output_tokens":"5"})),
        (
            "usage",
            json!({"input_tokens":u32::MAX,"output_tokens":5,"cache_read_input_tokens":1}),
        ),
        (
            "content",
            json!([{ "type":"tool_use", "id":"unsupported" }]),
        ),
        ("content", json!([{ "type":"text", "text":42 }])),
        ("stop_reason", json!("refusal")),
        ("stop_reason", serde_json::Value::Null),
    ] {
        let mut invalid = claude_response();
        invalid[field] = value;
        assert!(
            anthropic::response(invalid).is_err(),
            "accepted invalid {field}"
        );
    }
    let provider = openai::OpenAIProvider::new(Some("https://api.anthropic.com"));
    let mut private_usage = claude_response();
    private_usage["usage"]["input_tokens"] = json!("private-malformed-usage");
    assert!(!anthropic::response(private_usage)
        .unwrap_err()
        .to_string()
        .contains("private-malformed-usage"));
    assert!(provider
        .embedding_wire_bytes(&EmbeddingRequest {
            model: "embedding-model".into(),
            input: "text".into()
        })
        .is_err());
}

#[tokio::test]
async fn claude_stream_merges_cumulative_usage_once_and_rejects_incomplete_evidence() {
    use serde_json::json;
    let frames = claude_frames();
    let bytes = encode_claude_frames(&frames);
    let events = decode(
        bytes.iter().map(|byte| vec![*byte]).collect(),
        anthropic::stream_parser(),
    )
    .await;
    assert!(events.iter().all(Result::is_ok));
    let events: Vec<_> = events.into_iter().map(Result::unwrap).collect();
    assert_eq!(
        events
            .iter()
            .filter_map(|event| if let ChatStreamEvent::Delta(text) = event {
                Some(text.as_str())
            } else {
                None
            })
            .collect::<String>(),
        "世界"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ChatStreamEvent::Usage(_)))
            .count(),
        1
    );
    assert_eq!(
        &events[events.len() - 2..],
        &[
            ChatStreamEvent::Usage(crate::Usage::new(17, 5, Some(7)).unwrap()),
            ChatStreamEvent::Finished
        ]
    );
    for (index, path, value) in [
        (1, "/message/model", json!("")),
        (1, "/message/usage", serde_json::Value::Null),
        (6, "/content_block/type", json!("tool_use")),
        (8, "/index", json!(9)),
        (10, "/usage/output_tokens", json!(8)),
        (11, "/usage/output_tokens", json!("private-malformed-usage")),
        (11, "/usage/input_tokens", json!(u32::MAX)),
        (11, "/delta/stop_reason", serde_json::Value::Null),
        (11, "/delta/stop_reason", json!("refusal")),
        (11, "/delta/stop_reason", json!("max_tokens")),
    ] {
        let mut invalid = frames.clone();
        *invalid[index].1.pointer_mut(path).unwrap() = value;
        let events = decode(
            vec![encode_claude_frames(&invalid)],
            anthropic::stream_parser(),
        )
        .await;
        assert!(events.iter().any(Result::is_err), "accepted invalid {path}");
        assert!(events
            .iter()
            .filter_map(|event| event.as_ref().err())
            .all(|error| !error.to_string().contains("private-malformed-usage")));
        assert!(!events
            .iter()
            .any(|event| matches!(event, Ok(ChatStreamEvent::Finished))));
        if invalid[11].1["delta"]["stop_reason"] == "max_tokens" {
            assert!(events.iter().any(|event| event
                .as_ref()
                .is_err_and(|error| error.is::<crate::TruncatedCompletion>())));
        }
    }
    for index in [1, 9, 11, 12] {
        let mut invalid = frames.clone();
        invalid.remove(index);
        let events = decode(
            vec![encode_claude_frames(&invalid)],
            anthropic::stream_parser(),
        )
        .await;
        assert!(
            events.iter().any(Result::is_err),
            "accepted missing event {index}"
        );
        assert!(!events
            .iter()
            .any(|event| matches!(event, Ok(ChatStreamEvent::Finished))));
    }
    let mut without_interim = frames.clone();
    without_interim.remove(10);
    let events = decode(
        vec![encode_claude_frames(&without_interim)],
        anthropic::stream_parser(),
    )
    .await;
    assert!(events.iter().all(Result::is_ok));
    assert!(matches!(events.last(), Some(Ok(ChatStreamEvent::Finished))));
    let error = b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"private-provider-error\"}}\n\n";
    let events = decode(vec![error.to_vec()], anthropic::stream_parser()).await;
    let error = events[0].as_ref().unwrap_err();
    assert_eq!(
        error.downcast_ref::<crate::LlmApiError>().unwrap().status,
        529
    );
    assert!(!error.to_string().contains("private-provider-error"));
}

#[tokio::test]
async fn google_and_claude_dispatch_auth_body_and_stream_through_the_shared_client() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    for (provider, origin, model) in [
        (
            "google",
            "https://generativelanguage.googleapis.com/v1beta/openai",
            "gemini-3.8-flash",
        ),
        (
            "anthropic",
            "https://api.anthropic.com",
            "claude-sonnet-5-5",
        ),
    ] {
        for streaming in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let dispatch = format!("http://{}", listener.local_addr().unwrap());
            let mut request = ChatRequest::new(crate::LlmOperation::CharacterChat, model)
                .message("system", "Return JSON only")
                .message("user", "Previous user turn")
                .message("assistant", "Previous character turn")
                .message("user", "Current user turn")
                .max_tokens(32)
                .thinking(false)
                .json();
            request.stream = streaming;
            let expected_wire = openai::OpenAIProvider::new(Some(origin))
                .chat_wire_bytes(&request)
                .unwrap();
            let response = if provider == "anthropic" {
                if streaming {
                    encode_claude_frames(&claude_frames())
                } else {
                    serde_json::to_vec(&claude_response()).unwrap()
                }
            } else if streaming {
                format!("data: {{\"model\":\"{model}\",\"choices\":[{{\"delta\":{{\"content\":\"世界\"}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":12,\"completion_tokens\":5,\"prompt_tokens_details\":{{\"cached_tokens\":5}}}}}}\n\ndata: [DONE]\n\n").into_bytes()
            } else {
                serde_json::to_vec(&serde_json::json!({"model":model,"choices":[{"message":{"content":"世界"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":5}}})).unwrap()
            };
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert!(read > 0);
                    bytes.extend_from_slice(&chunk[..read]);
                    assert!(bytes.len() < 32768);
                    if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&bytes[..end])
                            .unwrap()
                            .to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() < end + 4 + length {
                            continue;
                        }
                        assert_eq!(&bytes[end + 4..end + 4 + length], expected_wire);
                        if provider == "anthropic" {
                            assert!(headers.starts_with("post /v1/messages "));
                            assert!(headers.contains("x-api-key: anthropic-synthetic-key\r\n"));
                            assert!(headers.contains("anthropic-version: 2023-06-01\r\n"));
                            assert!(!headers.contains("authorization:"));
                        } else {
                            assert!(headers.starts_with("post /v1/chat/completions "));
                            assert!(
                                headers.contains("authorization: bearer google-synthetic-key\r\n")
                            );
                            assert!(!headers.contains("x-api-key:"));
                        }
                        break;
                    }
                }
                socket
                    .write_all(&http_response(
                        "200 OK",
                        if streaming {
                            "text/event-stream"
                        } else {
                            "application/json"
                        },
                        std::str::from_utf8(&response).unwrap(),
                        "",
                    ))
                    .await
                    .unwrap();
            });
            let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
            let handle = recorder.handle();
            let _guard = metrics::set_default_local_recorder(&recorder);
            let mut client = LlmClient::new().with_openai_compatible(
                provider,
                format!("{provider}-synthetic-key"),
                origin,
            );
            client.test_dispatch_to(dispatch);
            request.model = format!("{provider}/{model}");
            if streaming {
                let events = client
                    .chat_stream(request)
                    .await
                    .unwrap()
                    .collect::<Vec<_>>()
                    .await;
                assert!(events.iter().all(Result::is_ok));
                assert_eq!(
                    events
                        .iter()
                        .filter_map(|event| if let Ok(ChatStreamEvent::Delta(text)) = event {
                            Some(text.as_str())
                        } else {
                            None
                        })
                        .collect::<String>(),
                    "世界"
                );
                assert!(matches!(events.last(), Some(Ok(ChatStreamEvent::Finished))));
            } else {
                let response = client.chat(request).await.unwrap();
                assert_eq!(response.content, "世界");
                assert_eq!(response.model, model);
                assert_eq!(response.usage.unwrap().input_tokens, 12);
            }
            server.await.unwrap();
            let metrics = handle.render();
            let labels = [("provider", provider), ("operation", "character_chat")];
            assert_eq!(
                metric_value(&metrics, "novelworld_llm_usage_reports_total", &labels),
                1.0
            );
            assert_eq!(
                metric_value(
                    &metrics,
                    "novelworld_llm_billable_tokens_total",
                    &[("provider", provider), ("class", "cached_input")]
                ),
                if provider == "anthropic" && streaming {
                    7.0
                } else {
                    5.0
                }
            );
        }
    }
}

fn test_parser(frame: sse::SseFrame) -> Result<Vec<ChatStreamEvent>> {
    if frame.data == "[DONE]" {
        Ok(vec![ChatStreamEvent::Finished])
    } else {
        Ok(vec![ChatStreamEvent::Delta(frame.data)])
    }
}

#[test]
fn test_should_retry_on_429() {
    assert!(RetryPolicy::should_retry(429, 0));
    assert!(RetryPolicy::should_retry(429, 1));
    assert!(RetryPolicy::should_retry(429, 2));
    assert!(!RetryPolicy::should_retry(429, 3)); // exceeds max
}

#[test]
fn test_should_retry_on_5xx() {
    assert!(RetryPolicy::should_retry(500, 0));
    assert!(RetryPolicy::should_retry(502, 0));
    assert!(RetryPolicy::should_retry(503, 0));
}

#[test]
fn test_should_not_retry_on_4xx() {
    assert!(!RetryPolicy::should_retry(400, 0));
    assert!(!RetryPolicy::should_retry(401, 0));
    assert!(!RetryPolicy::should_retry(403, 0));
    assert!(!RetryPolicy::should_retry(404, 0));
}

#[test]
fn truncated_json_does_not_fallback_or_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 8192];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket
            .write_all(&http_response(
                "200 OK",
                "application/json",
                r#"{"choices":[{"message":{"content":"{\"characters\":["},"finish_reason":"length"}],"model":"test","usage":null}"#,
                "",
            ))
            .unwrap();
    });

    let error = tokio::runtime::Runtime::new().unwrap().block_on(async {
        let client =
            LlmClient::new().with_openai_compatible("test", "key", format!("http://{address}"));
        client
            .chat(
                ChatRequest::new(crate::LlmOperation::CharacterExtraction, "test/model")
                    .message("user", "probe")
                    .max_tokens(4_096)
                    .json(),
            )
            .await
            .unwrap_err()
    });
    assert!(error
        .downcast_ref::<openai::TruncatedCompletion>()
        .is_some());
    server.join().unwrap();
}

#[test]
fn test_retry_delay() {
    let d = RetryPolicy::delay(500, 0, None);
    assert_eq!(d.as_secs(), 1);
    let d = RetryPolicy::delay(500, 1, None);
    assert_eq!(d.as_secs(), 2);
    let d = RetryPolicy::delay(500, 2, None);
    assert_eq!(d.as_secs(), 4);
}

#[test]
fn test_retry_after_header() {
    let d = RetryPolicy::delay(429, 0, Some("30"));
    assert_eq!(d.as_secs(), 30);
    let d = RetryPolicy::delay(503, 0, Some("120"));
    assert_eq!(d.as_secs(), 120);
    let d = RetryPolicy::delay(429, 0, Some("86400"));
    assert_eq!(d.as_secs(), 120);
}

#[test]
fn provider_error_text_is_discarded_and_success_bodies_are_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let oversized = " ".repeat(1024 * 1024 + 1);
    let responses = vec![
        http_response(
            "400 Bad Request",
            "application/json",
            "sentinel-private-provider-text",
            "",
        ),
        http_response("200 OK", "application/json", &oversized, ""),
    ];
    let server = thread::spawn(move || {
        for response in responses {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 8192];
            assert!(socket.read(&mut request).unwrap() > 0);
            socket.write_all(&response).unwrap();
        }
    });

    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let client =
            LlmClient::new().with_openai_compatible("test", "key", format!("http://{address}"));
        let request = || {
            ChatRequest::new(crate::LlmOperation::CharacterExtraction, "test/model")
                .max_tokens(4_096)
        };
        let error = client.chat(request()).await.unwrap_err().to_string();
        assert!(!error.contains("sentinel-private-provider-text"));
        assert!(error.contains("provider request failed"));

        let error = client.chat(request()).await.unwrap_err().to_string();
        assert!(error.contains("provider response exceeds"), "{error}");
    });
    server.join().unwrap();
}

#[test]
fn redirects_are_not_followed_or_observed_as_target_responses() {
    for status in ["307 Temporary Redirect", "308 Permanent Redirect"] {
        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let source_address = source.local_addr().unwrap();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_address = target.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = source.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut bytes = [0; 1024];
                let read = socket.read(&mut bytes).unwrap();
                assert!(read > 0, "request ended before complete HTTP headers");
                request.extend_from_slice(&bytes[..read]);
                assert!(request.len() <= 8192, "request headers exceed test bound");
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            socket
                .write_all(&http_response(
                    status,
                    "application/json",
                    "redirected",
                    &format!("Location: http://{target_address}/target\r\n"),
                ))
                .unwrap();
            request
        });

        let records = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = records.clone();
        let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
            LlmClient::new()
                .with_openai_compatible("test", "synthetic-key", format!("http://{source_address}"))
                .chat(
                    ChatRequest::new(crate::LlmOperation::CharacterExtraction, "test/model")
                        .max_tokens(4_096)
                        .observe_responses(move |evidence| {
                            observed.lock().unwrap().push((
                                evidence.status,
                                evidence.body.to_vec(),
                                evidence.complete,
                            ));
                            Ok(())
                        }),
                )
                .await
        });
        let source_request = server.join().unwrap();
        let error = result.unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<crate::LlmApiError>()
                .expect("redirect must be reported as an HTTP API error")
                .status,
            status[..3].parse::<u16>().unwrap()
        );
        assert_eq!(records.lock().unwrap().len(), 1);
        assert_eq!(
            records.lock().unwrap()[0].0,
            status[..3].parse::<u16>().unwrap()
        );
        assert!(matches!(
            target.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        assert!(String::from_utf8_lossy(&source_request)
            .to_ascii_lowercase()
            .contains("authorization: bearer synthetic-key"));
    }
}

#[test]
fn retry_delay_cannot_outlive_the_total_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 8192];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket
            .write_all(&http_response(
                "429 Too Many Requests",
                "application/json",
                "{}",
                "Retry-After: 1\r\n",
            ))
            .unwrap();
    });

    let started = Instant::now();
    let error = tokio::runtime::Runtime::new().unwrap().block_on(async {
        LlmClient::new()
            .with_openai_compatible("test", "key", format!("http://{address}"))
            .chat(
                ChatRequest::new(crate::LlmOperation::CharacterExtraction, "test/model")
                    .max_tokens(4_096),
            )
            .await
            .unwrap_err()
    });
    assert!(error.to_string().contains("total deadline"));
    assert!(started.elapsed() < Duration::from_secs(1));
    server.join().unwrap();
}

#[test]
fn provider_stream_cannot_outlive_the_total_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 8192];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n",
            )
            .unwrap();
        socket.flush().unwrap();
        thread::sleep(Duration::from_secs(2));
    });

    let started = Instant::now();
    let events = tokio::runtime::Runtime::new().unwrap().block_on(async {
        LlmClient::new()
            .with_openai_compatible("test", "key", format!("http://{address}"))
            .chat_stream(
                ChatRequest::new(crate::LlmOperation::CharacterChat, "test/model")
                    .max_tokens(1_024),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    });
    assert!(events.into_iter().any(|event| event.is_err()));
    assert!(started.elapsed() < Duration::from_secs(1));
    server.join().unwrap();
}

#[test]
fn operation_output_limits_include_hidden_reasoning_and_fail_before_io() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let error = runtime
        .block_on(LlmClient::new().chat(
            ChatRequest::new(crate::LlmOperation::SetupConnection, "missing/model").max_tokens(9),
        ))
        .unwrap_err();
    assert!(error.to_string().contains("allows at most 8"));

    let request = ChatRequest::new(crate::LlmOperation::CharacterChat, "missing/model")
        .max_tokens(1_024)
        .thinking(true);
    assert_eq!(request.effective_max_output_tokens(), Some(5_120));
    assert_eq!(
        crate::LlmOperation::CanonExtraction.max_output_tokens(),
        8_192
    );

    let client = LlmClient::new().with_openai_compatible("configured", "key", "http://127.0.0.1:1");
    let error = runtime
        .block_on(client.chat(
            ChatRequest::new(crate::LlmOperation::SetupConnection, "other/model").max_tokens(8),
        ))
        .unwrap_err();
    assert!(error.to_string().contains("but 'configured' is configured"));
}

#[test]
fn stream_setup_honors_retry_after_and_uses_all_three_retries() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for attempt in 0..4 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 8192];
            let read = socket.read(&mut request).unwrap();
            assert!(
                String::from_utf8_lossy(&request[..read]).starts_with("POST /v1/chat/completions")
            );

            if attempt < 3 {
                socket
                    .write_all(
                        b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .unwrap();
            } else {
                let body = b"data: [DONE]\n\n";
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                socket.write_all(body).unwrap();
            }
        }
    });

    let started = Instant::now();
    let events = tokio::runtime::Runtime::new().unwrap().block_on(async {
        let client =
            LlmClient::new().with_openai_compatible("test", "key", format!("http://{address}"));
        let stream = client
            .chat_stream(
                ChatRequest::new(crate::LlmOperation::CharacterChat, "test/model")
                    .max_tokens(1_024),
            )
            .await
            .unwrap();
        stream.collect::<Vec<_>>().await
    });

    server.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(matches!(events.as_slice(), [Ok(ChatStreamEvent::Finished)]));
}

#[test]
fn embedding_honors_retry_after_and_records_exact_attempts() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 8192];
            let read = socket.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("POST /v1/embeddings"));
            if attempt == 0 {
                socket
                    .write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .unwrap();
            } else {
                let body = r#"{"data":[{"embedding":[0.25,0.75]}],"model":"embedding-model","usage":{"prompt_tokens":1,"total_tokens":2}}"#;
                socket
                    .write_all(&http_response("200 OK", "application/json", body, ""))
                    .unwrap();
            }
        }
    });

    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let response = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            LlmClient::new()
                .with_openai_compatible("test", "key", format!("http://{address}"))
                .embed(EmbeddingRequest {
                    model: "test/embedding-model".into(),
                    input: "remember this".into(),
                })
                .await
                .unwrap()
        });

    server.join().unwrap();
    assert_eq!(response.embedding, vec![0.25, 0.75]);
    let rendered = handle.render();
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_embedding_attempts_total",
            &[
                ("provider", "test"),
                ("model", "embedding-model"),
                ("status", "provider_error"),
            ],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_embedding_attempts_total",
            &[
                ("provider", "test"),
                ("model", "embedding-model"),
                ("status", "success"),
            ],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_embedding_retries_total",
            &[("reason", "provider_error")],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_embedding_requests_total",
            &[("status", "success")],
        ),
        1.0
    );
    assert!(!rendered
        .lines()
        .any(|line| { line.starts_with("novelworld_llm_") && line.contains("embedding") }));
}

#[test]
fn embedding_authentication_is_optional_without_sending_an_empty_bearer() {
    for (api_key, expected_header) in [("", None), ("secret", Some("authorization: bearer secret"))]
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut bytes = [0; 1024];
                let read = socket.read(&mut bytes).unwrap();
                assert!(read > 0, "request ended before complete HTTP headers");
                request.extend_from_slice(&bytes[..read]);
                assert!(request.len() <= 8192, "request headers exceed test bound");
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let body = r#"{"data":[{"embedding":[0.25,0.75]}],"model":"embedding-model"}"#;
            socket
                .write_all(&http_response("200 OK", "application/json", body, ""))
                .unwrap();
            String::from_utf8(request).unwrap().to_ascii_lowercase()
        });

        let response = tokio::runtime::Runtime::new().unwrap().block_on(async {
            LlmClient::new()
                .with_openai_compatible("embedding", api_key, format!("http://{address}"))
                .embed(EmbeddingRequest {
                    model: "embedding/embedding-model".into(),
                    input: "remember this".into(),
                })
                .await
                .unwrap()
        });
        assert_eq!(response.embedding, vec![0.25, 0.75]);
        let request = server.join().unwrap();
        assert_eq!(
            request.contains("authorization:"),
            expected_header.is_some()
        );
        if let Some(expected_header) = expected_header {
            assert!(request.contains(expected_header));
        }
    }
}

#[test]
fn embedding_retry_delay_cannot_outlive_the_total_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 8192];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket
            .write_all(&http_response(
                "503 Service Unavailable",
                "application/json",
                "{}",
                "Retry-After: 1\r\n",
            ))
            .unwrap();
    });

    let started = Instant::now();
    let error = tokio::runtime::Runtime::new().unwrap().block_on(async {
        LlmClient::new()
            .with_openai_compatible("test", "key", format!("http://{address}"))
            .embed(EmbeddingRequest {
                model: "test/embedding-model".into(),
                input: "remember this".into(),
            })
            .await
            .unwrap_err()
    });
    server.join().unwrap();
    assert!(error.to_string().contains("total deadline"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn provider_metrics_count_logical_requests_attempts_usage_and_stream_terminals() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let success = r#"{"choices":[{"message":{"content":"ok"}}],"model":"model","usage":{"prompt_tokens":10,"completion_tokens":3,"prompt_cache_hit_tokens":4}}"#;
    let stream_success = concat!(
        "data: {\"model\":\"model\",\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"model\":\"model\",\"choices\":[],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":2,\"prompt_cache_hit_tokens\":5}}\n\n",
        "data: [DONE]\n\n"
    );
    let stream_drop = concat!(
        "data: {\"model\":\"model\",\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: [DONE]\n\n"
    );
    let missing_terminal =
        "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}],\"usage\":null}\n\n";
    let stream_provider_error =
        "data: {\"error\":{\"message\":\"provider rejected the stream\"}}\n\n";
    let responses = vec![
        http_response(
            "429 Too Many Requests",
            "application/json",
            "{}",
            "Retry-After: 0\r\n",
        ),
        http_response("200 OK", "application/json", success, ""),
        http_response("400 Bad Request", "application/json", "{}", ""),
        http_response(
            "200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":"ok"}}],"model":"model","usage":null}"#,
            "",
        ),
        http_response(
            "200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":""}}],"model":"model","usage":{"prompt_tokens":7,"completion_tokens":1,"prompt_cache_hit_tokens":2}}"#,
            "",
        ),
        http_response(
            "200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":"{}"}}],"model":"model","usage":{"prompt_tokens":5,"completion_tokens":2,"prompt_cache_hit_tokens":1}}"#,
            "",
        ),
        http_response("200 OK", "text/event-stream", stream_success, ""),
        http_response("200 OK", "text/event-stream", stream_drop, ""),
        http_response("200 OK", "text/event-stream", missing_terminal, ""),
        http_response("200 OK", "text/event-stream", stream_provider_error, ""),
    ];
    let server = thread::spawn(move || {
        for response in responses {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 8192];
            assert!(socket.read(&mut request).unwrap() > 0);
            socket.write_all(&response).unwrap();
        }
    });

    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let records = observed.clone();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let client = LlmClient::new().with_openai_compatible(
                "deepseek",
                "key",
                format!("http://{address}"),
            );
            client
                .chat(
                    ChatRequest::new(crate::LlmOperation::CharacterExtraction, "deepseek/model")
                        .max_tokens(4_096)
                        .observe_responses(move |evidence| {
                            records.lock().unwrap().push((
                                evidence.status,
                                evidence.body.to_vec(),
                                evidence.complete,
                            ));
                            Ok(())
                        }),
                )
                .await
                .unwrap();
            assert!(client
                .chat(
                    ChatRequest::new(crate::LlmOperation::NarrativeTransition, "deepseek/model")
                        .max_tokens(4_096),
                )
                .await
                .is_err());
            client
                .chat(
                    ChatRequest::new(crate::LlmOperation::BranchGeneration, "deepseek/model")
                        .max_tokens(4_096),
                )
                .await
                .unwrap();
            client
                .chat(
                    ChatRequest::new(crate::LlmOperation::CanonExtraction, "deepseek/model")
                        .max_tokens(4_096)
                        .json(),
                )
                .await
                .unwrap();

            let events = client
                .chat_stream(
                    ChatRequest::new(crate::LlmOperation::CharacterChat, "deepseek/model")
                        .max_tokens(1_024),
                )
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await;
            assert!(matches!(
                events.as_slice(),
                [Ok(ChatStreamEvent::Delta(text)), Ok(ChatStreamEvent::Finished)] if text == "hello"
            ));

            let mut dropped = client
                .chat_stream(
                    ChatRequest::new(crate::LlmOperation::CharacterChat, "deepseek/model")
                        .max_tokens(1_024),
                )
                .await
                .unwrap();
            assert!(matches!(
                dropped.next().await,
                Some(Ok(ChatStreamEvent::Delta(text))) if text == "partial"
            ));
            drop(dropped);

            let missing = client
                .chat_stream(
                    ChatRequest::new(crate::LlmOperation::CharacterChat, "deepseek/model")
                        .max_tokens(1_024),
                )
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await;
            assert!(missing.into_iter().any(|item| item.is_err()));

            let provider_error = client
                .chat_stream(
                    ChatRequest::new(crate::LlmOperation::CharacterChat, "deepseek/model")
                        .max_tokens(1_024),
                )
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await;
            assert!(provider_error.into_iter().any(|item| item.is_err()));
        });
    server.join().unwrap();
    assert_eq!(
        *observed.lock().unwrap(),
        [
            (429, b"{}".to_vec(), true),
            (200, success.as_bytes().to_vec(), true)
        ]
    );

    let rendered = handle.render();
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_usage_reports_total",
            &[("operation", "canon_extraction"), ("status", "present")]
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_usage_reports_total",
            &[("operation", "canon_extraction"), ("status", "missing")]
        ),
        0.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_requests_total",
            &[("operation", "canon_extraction"), ("status", "success")]
        ),
        1.0
    );
    let usage_key = crate::usage_key_fingerprint("key");
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_requests_started_total",
            &[("operation", "character_extraction")],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_attempts_total",
            &[
                ("operation", "canon_extraction"),
                ("status", "empty_json_mode")
            ],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_retries_total",
            &[
                ("operation", "canon_extraction"),
                ("reason", "json_mode_fallback")
            ],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_tokens_total",
            &[("operation", "canon_extraction"), ("type", "input")],
        ),
        12.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_attempts_total",
            &[
                ("operation", "character_extraction"),
                ("status", "rate_limited")
            ],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_attempts_total",
            &[("operation", "character_extraction"), ("status", "success")],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_retries_total",
            &[("operation", "character_extraction")],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_tokens_total",
            &[
                ("operation", "character_extraction"),
                ("type", "cached_input")
            ],
        ),
        4.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_billable_tokens_total",
            &[
                ("operation", "character_extraction"),
                ("class", "uncached_input"),
                ("usage_key", usage_key.as_str())
            ],
        ),
        6.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_usage_reports_total",
            &[("operation", "branch_generation"), ("status", "missing")],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_requests_total",
            &[("operation", "narrative_transition"), ("status", "error")],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_stream_setup_duration_seconds_count",
            &[("operation", "character_chat"), ("status", "success")],
        ),
        4.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_first_token_duration_seconds_count",
            &[("operation", "character_chat")],
        ),
        3.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_requests_total",
            &[("operation", "character_chat"), ("status", "success")],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_requests_total",
            &[
                ("operation", "character_chat"),
                ("status", "consumer_dropped")
            ],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_requests_total",
            &[("operation", "character_chat"), ("status", "stream_error")],
        ),
        2.0
    );
}

#[test]
fn sse_decoder_is_invariant_at_every_byte_split() {
    futures::executor::block_on(async {
        let transcript = concat!(
            "\u{feff}: keep-alive\r",
            "event: token\r\n",
            "data: \u{4f60}\r",
            "data:\u{597d}\u{1f642}\n",
            "\r",
            "data:[DONE]\r\n",
            "\r\n"
        )
        .as_bytes()
        .to_vec();

        let expected = vec![
            ChatStreamEvent::Delta("\u{4f60}\n\u{597d}\u{1f642}".into()),
            ChatStreamEvent::Finished,
        ];

        for split in 0..=transcript.len() {
            let results = decode(
                vec![transcript[..split].to_vec(), transcript[split..].to_vec()],
                test_parser,
            )
            .await;
            let actual: Result<Vec<_>> = results.into_iter().collect();
            assert_eq!(actual.unwrap(), expected, "byte split {split}");
        }
    });
}

#[test]
fn sse_decoder_fails_closed_on_invalid_utf8_oversize_and_missing_terminal() {
    futures::executor::block_on(async {
        let invalid_utf8 =
            decode(vec![b"data: \xff\n\ndata:[DONE]\n\n".to_vec()], test_parser).await;
        assert!(invalid_utf8.into_iter().any(|item| item.is_err()));

        let oversized = format!("data: {}\n\n", "x".repeat(sse::MAX_FRAME_BYTES + 1));
        let oversized = decode(vec![oversized.into_bytes()], test_parser).await;
        assert!(oversized.into_iter().any(|item| item.is_err()));

        let missing_terminal = decode(vec![b"data: partial\n\n".to_vec()], test_parser).await;
        assert!(matches!(
            missing_terminal.as_slice(),
            [Ok(ChatStreamEvent::Delta(text)), Err(_)] if text == "partial"
        ));
    });
}

#[test]
fn provider_failed_stream_never_finishes_after_partial_text() {
    futures::executor::block_on(async {
        for reason in [
            "sensitive",
            "network_error",
            "model_context_window_exceeded",
            "length",
            "tool_calls",
            "unknown",
        ] {
            let transcript = format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":\"partial\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"{reason}\"}}]}}\n\ndata: [DONE]\n\n"
            );
            let events = decode(vec![transcript.into_bytes()], openai::parse_stream_frame).await;
            assert!(events.iter().any(Result::is_err), "{reason}");
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, Ok(ChatStreamEvent::Finished))),
                "{reason}"
            );
        }
    });
}

#[test]
fn stepfun_stream_reports_only_terminal_cumulative_usage() {
    futures::executor::block_on(async {
        let mut transcript = String::new();
        for (completion_tokens, reason, content) in [
            (1, "", "你好"),
            (2, "", "，"),
            (3, "", "世界"),
            (150, "stop", ""),
        ] {
            let payload = serde_json::json!({
                "model": "step-3.5-flash",
                "choices": [{"delta": {"content": content}, "finish_reason": reason}],
                "usage": {"prompt_tokens": 83, "completion_tokens": completion_tokens, "total_tokens": 83 + completion_tokens}
            });
            transcript.push_str(&format!("data: {payload}\n\n"));
        }
        transcript.push_str("data: [DONE]\n\n");
        let events: Result<Vec<_>> = decode(
            vec![transcript.as_bytes().to_vec()],
            openai::parse_stepfun_stream_frame,
        )
        .await
        .into_iter()
        .collect();
        let events = events.unwrap();
        let usage: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                ChatStreamEvent::Usage(usage) => Some(usage.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(usage, vec![crate::Usage::new(83, 150, None).unwrap()]);
        assert_eq!(events.last(), Some(&ChatStreamEvent::Finished));

        // Other providers still expose duplicate reports to the client's strict guard.
        let ordinary = decode(vec![transcript.into_bytes()], openai::parse_stream_frame).await;
        assert_eq!(
            ordinary
                .iter()
                .filter(|event| matches!(event, Ok(ChatStreamEvent::Usage(_))))
                .count(),
            4
        );
    });
}

#[test]
fn openai_requires_done_and_rejects_content_filter_or_error() {
    futures::executor::block_on(async {
        let transcript = "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":\"\"}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let events = decode(
            vec![transcript.as_bytes().to_vec()],
            openai::parse_stream_frame,
        )
        .await;
        assert!(events.iter().all(Result::is_ok));
        assert!(events
            .iter()
            .any(|event| matches!(event, Ok(ChatStreamEvent::Finished))));
    });
    futures::executor::block_on(async {
        let valid = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"\u{4f60}\u{597d}\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n"
        );
        let actual: Result<Vec<_>> = decode(
            valid.as_bytes().iter().map(|byte| vec![*byte]).collect(),
            openai::parse_stream_frame,
        )
        .await
        .into_iter()
        .collect();
        assert_eq!(
            actual.unwrap(),
            vec![
                ChatStreamEvent::Delta("\u{4f60}\u{597d}".into()),
                ChatStreamEvent::Finished
            ]
        );

        for body in [
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"content_filter\"}]}\n\n",
            "data: {\"error\":{\"message\":\"overloaded\"}}\n\n",
        ] {
            let results = decode(vec![body.as_bytes().to_vec()], openai::parse_stream_frame).await;
            assert!(results.into_iter().any(|item| item.is_err()));
        }
    });
}

#[test]
fn openai_stream_exposes_the_provider_response_model() {
    futures::executor::block_on(async {
        let body = concat!(
            "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n"
        );
        let actual: Result<Vec<_>> =
            decode(vec![body.as_bytes().to_vec()], openai::parse_stream_frame)
                .await
                .into_iter()
                .collect();
        assert_eq!(
            actual.unwrap(),
            vec![
                ChatStreamEvent::ResponseModel("deepseek-v4-flash".into()),
                ChatStreamEvent::Delta("ok".into()),
                ChatStreamEvent::Finished,
            ]
        );
    });
}

#[test]
fn openai_rejects_malformed_known_events() {
    futures::executor::block_on(async {
        let results = decode(
            vec![b"data: not-json\n\n".to_vec()],
            openai::parse_stream_frame,
        )
        .await;
        assert!(results.into_iter().any(|item| item.is_err()));
    });
}

#[test]
fn empty_json_fallback_keeps_earlier_model_and_missing_usage_visible() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for body in [
            r#"{"model":"unregistered-model","choices":[{"message":{"content":""}}]}"#,
            r#"{"model":"registered-model","choices":[{"message":{"content":"{}"}}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
        ] {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 8192];
            assert!(socket.read(&mut request).unwrap() > 0);
            socket
                .write_all(&http_response("200 OK", "application/json", body, ""))
                .unwrap();
        }
    });
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let collected = observed.clone();
    let response = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut request =
                crate::production_json_request(crate::LlmOperation::CanonExtraction, "probe");
            request.model = "registered-model".into();
            let request = request.observe_responses(move |evidence| {
                collected
                    .lock()
                    .unwrap()
                    .push(crate::chat_completion_response_metadata(evidence.body)?.0);
                Ok(())
            });
            LlmClient::new()
                .with_openai_compatible("test", "key", format!("http://{address}"))
                .chat(request)
                .await
                .unwrap()
        });
    server.join().unwrap();
    assert_eq!(response.model, "registered-model");
    let rendered = handle.render();
    assert_eq!(
        *observed.lock().unwrap(),
        ["unregistered-model", "registered-model"]
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_usage_reports_total",
            &[("status", "missing")]
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_usage_reports_total",
            &[("status", "present")]
        ),
        0.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_tokens_total",
            &[("type", "input")]
        ),
        3.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_llm_tokens_total",
            &[("type", "output")]
        ),
        2.0
    );
}

#[tokio::test]
async fn response_evidence_is_bounded_and_retained_before_parse_or_cancellation() {
    for (body, extra_length, stall) in [
        (b"{bad-json".to_vec(), 0, false),
        (vec![b'x'; 1024 * 1024 + 1], 0, false),
        (b"{partial".to_vec(), 100, false),
        (b"{pending".to_vec(), 100, true),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let complete = extra_length == 0 && body.len() <= 1024 * 1024;
        let expected = body[..body.len().min(1024 * 1024)].to_vec();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 8192];
            assert!(socket.read(&mut request).unwrap() > 0);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len() + extra_length
            )
            .unwrap();
            socket.write_all(&body).unwrap();
            if stall {
                thread::sleep(Duration::from_millis(350));
            }
        });
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let records = observed.clone();
        let request = ChatRequest::new(crate::LlmOperation::CanonExtraction, "test-model")
            .max_tokens(20)
            .json()
            .observe_responses(move |evidence| {
                records.lock().unwrap().push((
                    evidence.status,
                    evidence.body.to_vec(),
                    evidence.complete,
                ));
                anyhow::bail!("private body must not appear in the public error");
            });
        let result = LlmClient::new()
            .with_openai_compatible("test", "synthetic-key", format!("http://{address}"))
            .chat(request)
            .await;
        server.join().unwrap();
        let error = result.unwrap_err();
        assert!(!error.to_string().contains("private body"));
        assert!(error.is::<crate::ResponseEvidenceError>(), "{error}");
        assert_eq!(*observed.lock().unwrap(), [(200, expected, complete)]);
    }
}

#[tokio::test]
async fn response_observer_covers_responses_api_and_http_errors() {
    for status in ["200 OK", "429 Too Many Requests"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 8192];
            let size = socket.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..size]).starts_with("POST /v1/responses "));
            socket
                .write_all(&http_response(
                    status,
                    "application/json",
                    "private-envelope",
                    "Retry-After: 0\r\n",
                ))
                .unwrap();
        });
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let records = observed.clone();
        // Force the real DeepSeek route onto loopback; no DNS or provider traffic.
        let http = reqwest::Client::builder()
            .no_proxy()
            .resolve("api.deepseek.com", address)
            .build()
            .unwrap();
        let provider = openai::OpenAIProvider::new(Some(&format!(
            "http://api.deepseek.com:{}",
            address.port()
        )));
        let request = ChatRequest::new(crate::LlmOperation::CanonExtraction, "test-model")
            .max_tokens(20)
            .thinking(true)
            .observe_responses(move |evidence| {
                records.lock().unwrap().push((
                    evidence.status,
                    evidence.body.to_vec(),
                    evidence.complete,
                ));
                anyhow::bail!("synthetic sink failure");
            });
        assert!(provider
            .chat(&http, "synthetic-key", &request)
            .await
            .unwrap_err()
            .is::<crate::ResponseEvidenceError>());
        server.join().unwrap();
        assert_eq!(
            *observed.lock().unwrap(),
            [(
                if status.starts_with("200") { 200 } else { 429 },
                b"private-envelope".to_vec(),
                true
            )]
        );
    }
}

#[tokio::test]
async fn unsupported_stream_evidence_fails_before_io() {
    let request = ChatRequest::new(crate::LlmOperation::CharacterChat, "model")
        .max_tokens(20)
        .observe_responses(|_| panic!("no provider request is permitted"));
    let result = LlmClient::new().chat_stream(request).await;
    assert!(result
        .err()
        .unwrap()
        .to_string()
        .contains("non-streaming chat"));
}
