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
            .chat_stream(ChatRequest::new("test/model"))
            .await
            .unwrap();
        stream.collect::<Vec<_>>().await
    });

    server.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(matches!(events.as_slice(), [Ok(ChatStreamEvent::Finished)]));
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
