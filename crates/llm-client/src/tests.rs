use crate::retry::RetryPolicy;
use crate::{
    providers::{openai, sse},
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

async fn decode(
    chunks: Vec<Vec<u8>>,
    parse: fn(sse::SseFrame) -> Result<Vec<ChatStreamEvent>>,
) -> Vec<Result<ChatStreamEvent>> {
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
                let body = r#"{"data":[{"embedding":[0.25,0.75]}],"model":"embedding-model"}"#;
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
            &[("status", "provider_error")],
        ),
        1.0
    );
    assert_eq!(
        metric_value(
            &rendered,
            "novelworld_embedding_attempts_total",
            &[("status", "success")],
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
fn openai_requires_done_and_rejects_content_filter_or_error() {
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
            socket.read(&mut request).unwrap();
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
