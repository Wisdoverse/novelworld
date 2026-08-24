use crate::retry::RetryPolicy;
use crate::{
    providers::{anthropic, gemini, openai, sse},
    ChatRequest, ChatStreamEvent, LlmClient,
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
fn provider_metrics_count_logical_requests_attempts_usage_and_stream_terminals() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let success = r#"{"choices":[{"message":{"content":"ok"}}],"model":"model","usage":{"prompt_tokens":10,"completion_tokens":3,"prompt_cache_hit_tokens":4}}"#;
    let stream_success = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":2,\"prompt_cache_hit_tokens\":5}}\n\n",
        "data: [DONE]\n\n"
    );
    let stream_drop = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}],\"usage\":null}\n\n",
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
                        .max_tokens(4_096),
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

    let rendered = handle.render();
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
fn anthropic_requires_message_stop_rejects_errors_and_ignores_unknown_events() {
    futures::executor::block_on(async {
        let valid = concat!(
            "event: future_event\ndata: not-json\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let actual: Result<Vec<_>> = decode(
            vec![valid.as_bytes().to_vec()],
            anthropic::parse_stream_frame,
        )
        .await
        .into_iter()
        .collect();
        assert_eq!(
            actual.unwrap(),
            vec![
                ChatStreamEvent::Delta("Hello".into()),
                ChatStreamEvent::Finished
            ]
        );

        let error =
            "event: error\ndata: {\"type\":\"error\",\"error\":{\"message\":\"overloaded\"}}\n\n";
        let results = decode(
            vec![error.as_bytes().to_vec()],
            anthropic::parse_stream_frame,
        )
        .await;
        assert!(results.into_iter().any(|item| item.is_err()));
    });
}

#[test]
fn providers_reject_malformed_known_events() {
    futures::executor::block_on(async {
        type Parser = fn(sse::SseFrame) -> Result<Vec<ChatStreamEvent>>;
        let cases: [(&str, Parser); 3] = [
            ("data: not-json\n\n", openai::parse_stream_frame),
            (
                "event: content_block_delta\ndata: not-json\n\n",
                anthropic::parse_stream_frame,
            ),
            ("data: not-json\n\n", gemini::parse_stream_frame),
        ];

        for (body, parser) in cases {
            let results = decode(vec![body.as_bytes().to_vec()], parser).await;
            assert!(results.into_iter().any(|item| item.is_err()));
        }
    });
}

#[test]
fn gemini_accepts_stop_and_max_tokens_but_rejects_block_or_other_finish() {
    futures::executor::block_on(async {
        for reason in ["STOP", "MAX_TOKENS"] {
            let body = format!(
                "data: {{\"candidates\":[{{\"content\":{{\"parts\":[{{\"text\":\"Hi\"}}]}},\"finishReason\":\"{reason}\"}}]}}\n\n"
            );
            let actual: Result<Vec<_>> =
                decode(vec![body.into_bytes()], gemini::parse_stream_frame)
                    .await
                    .into_iter()
                    .collect();
            assert_eq!(
                actual.unwrap(),
                vec![
                    ChatStreamEvent::Delta("Hi".into()),
                    ChatStreamEvent::Finished
                ]
            );
        }

        for body in [
            "data: {\"promptFeedback\":{\"blockReason\":\"SAFETY\"}}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[]},\"finishReason\":\"SAFETY\"}]}\n\n",
        ] {
            let results = decode(vec![body.as_bytes().to_vec()], gemini::parse_stream_frame).await;
            assert!(results.into_iter().any(|item| item.is_err()));
        }
    });
}
