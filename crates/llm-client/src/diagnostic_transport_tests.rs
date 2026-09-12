use futures::StreamExt;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use uuid::Uuid;

use crate::{
    diagnostic_budget::{
        self as budget, Binding, BudgetClient, BudgetControlError, BudgetEvidenceError,
    },
    providers::openai::OpenAIProvider,
    ChatRequest, ChatStreamEvent, EmbeddingRequest, LlmClient, LlmOperation,
};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn missing_control_url_is_tested_in_an_isolated_process() {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "diagnostic_transport_tests::missing_control_url_child",
            "--ignored",
        ])
        .env_clear()
        .env("LLM_DIAGNOSTIC_BUDGET_ID", Uuid::new_v4().to_string())
        .env("INTERNAL_SERVICE_TOKEN", TOKEN)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("isolated configuration test deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[tokio::test]
#[ignore = "executed by missing_control_url_is_tested_in_an_isolated_process with an isolated environment"]
async fn missing_control_url_child() {
    assert!(std::env::var_os("USER_SERVICE_URL").is_none());
    let control = LocalHttp::start(control_reply).await;
    let provider = LocalHttp::start(|_, _| json_reply(completion("ok", true))).await;
    let mut client = client(&control, &provider);
    client.budget = BudgetClient::from_environment();
    assert!(client.budget.is_err());
    assert!(client
        .chat(request())
        .await
        .unwrap_err()
        .is::<BudgetControlError>());
    assert_eq!(control.count(), 0);
    assert_eq!(provider.count(), 0);
}

struct LocalHttp {
    origin: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for LocalHttp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl LocalHttp {
    async fn start(
        reply: impl Fn(&str, &Value) -> Option<(u16, &'static str, Vec<u8>)> + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let (path, body) = tokio::time::timeout(Duration::from_secs(2), async {
                    loop {
                        let mut chunk = [0; 4096];
                        let read = socket.read(&mut chunk).await.unwrap();
                        assert!(read > 0);
                        bytes.extend_from_slice(&chunk[..read]);
                        assert!(bytes.len() <= 32768);
                        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
                        {
                            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                            let length: usize = headers
                                .lines()
                                .find_map(|line| {
                                    line.to_ascii_lowercase()
                                        .strip_prefix("content-length: ")
                                        .map(str::to_owned)
                                })
                                .unwrap()
                                .parse()
                                .unwrap();
                            if bytes.len() < end + 4 + length {
                                continue;
                            }
                            let path = headers.split_whitespace().nth(1).unwrap().to_owned();
                            break (
                                path,
                                serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                                    .unwrap(),
                            );
                        }
                    }
                })
                .await
                .unwrap();
                recorded.lock().unwrap().push(body.clone());
                if let Some((status, content_type, body)) = reply(&path, &body) {
                    // Test-only sentinel: keep an accepted request unanswered past its logical deadline.
                    if status == 0 {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                    let headers = format!("HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nRetry-After: 0\r\n\r\n", body.len());
                    let _ = socket.write_all(headers.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                }
            }
        });
        Self {
            origin,
            requests,
            task,
        }
    }

    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

fn json_reply(value: Value) -> Option<(u16, &'static str, Vec<u8>)> {
    Some((200, "application/json", serde_json::to_vec(&value).unwrap()))
}

fn control_reply(path: &str, request: &Value) -> Option<(u16, &'static str, Vec<u8>)> {
    if path.ends_with("/reserve") {
        json_reply(
            json!({"binding": request["binding"], "attempt_id": request["attempt_id"], "ordinal":1,
            "reservation":{"attempts":1,"tokens":1048584,"cost_micro_cny":4194400}}),
        )
    } else {
        assert!(path.ends_with("/settle"));
        json_reply(request.clone())
    }
}

fn request() -> ChatRequest {
    ChatRequest::new(
        LlmOperation::SetupConnection,
        budget::profile().model.clone(),
    )
    .message("user", "synthetic test")
    .max_tokens(8)
    .thinking(false)
}

fn client(control: &LocalHttp, provider: &LocalHttp) -> LlmClient {
    let budget =
        BudgetClient::new(Binding::new(Uuid::new_v4()), &control.origin, TOKEN.into()).unwrap();
    LlmClient::diagnostic_test_client(Arc::new(budget), provider.origin.clone())
}

fn completion(content: &str, usage: bool) -> Value {
    json!({"model":budget::profile().model, "choices":[{"message":{"content":content}, "finish_reason":"stop"}],
        "usage": if usage { json!({"prompt_tokens":10,"completion_tokens":2}) } else { Value::Null }})
}

#[test]
fn actual_wire_caps_and_fixed_identity_fail_closed() {
    let provider = OpenAIProvider::new(Some("https://api.deepseek.com"));
    let validate = |request: &ChatRequest| {
        budget::validate_dispatch(
            "deepseek",
            provider.base_url(),
            request,
            &provider.chat_wire_bytes(request).unwrap(),
        )
    };
    let mut boundary = request();
    let overhead =
        provider.chat_wire_bytes(&boundary).unwrap().len() - boundary.messages[0].content.len();
    boundary.messages[0].content = "x".repeat(262144 - overhead);
    assert!(validate(&boundary).is_ok());
    boundary.messages[0].content.push('x');
    assert!(validate(&boundary).is_err());
    boundary.messages[0].content = "\u{0001}".repeat(50000);
    assert!(
        validate(&boundary).is_err(),
        "JSON escaping counts actual bytes"
    );
    boundary = request();
    boundary.messages = vec![boundary.messages[0].clone(); 64];
    assert!(validate(&boundary).is_ok());
    boundary.messages.push(boundary.messages[0].clone());
    assert!(validate(&boundary).is_err());
    for thinking in [None, Some(true)] {
        let mut invalid = request();
        invalid.thinking = thinking;
        assert!(validate(&invalid).is_err());
    }
    for role in ["tool", "developer", "function"] {
        let mut invalid = request();
        invalid.messages[0].role = role.into();
        assert!(validate(&invalid).is_err());
    }
    for origin in [
        "http://api.deepseek.com",
        "https://api.deepseek.com/v1",
        "https://user:secret@api.deepseek.com",
        "https://api.deepseek.com?query",
        "https://api.deepseek.com#fragment",
        "https://@api.deepseek.com",
        "https://api.deep\nseek.com",
    ] {
        assert!(budget::validate_dispatch("deepseek", origin, &request(), b"{}").is_err());
    }
}

#[tokio::test]
async fn empty_json_without_explicit_completion_never_settles_or_falls_back() {
    let control = LocalHttp::start(control_reply).await;
    let provider = LocalHttp::start(|_, _| {
        let mut response = completion("", true);
        response["choices"][0]
            .as_object_mut()
            .unwrap()
            .remove("finish_reason");
        json_reply(response)
    })
    .await;
    let mut request = request();
    request.json_mode = true;
    assert!(client(&control, &provider)
        .chat(request)
        .await
        .unwrap_err()
        .is::<BudgetEvidenceError>());
    assert_eq!(control.count(), 1);
    assert_eq!(provider.count(), 1);
}

#[tokio::test]
async fn control_provider_and_settlement_deadlines_are_typed_and_never_retry() {
    for stage in ["reserve", "provider", "settle"] {
        for streaming in [false, true] {
            let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
            let handle = recorder.handle();
            let _guard = metrics::set_default_local_recorder(&recorder);
            let control = LocalHttp::start(move |path, body| {
                if path.ends_with(stage) {
                    Some((0, "application/json", vec![]))
                } else {
                    control_reply(path, body)
                }
            })
            .await;
            let provider = LocalHttp::start(move |_, _| {
                if stage == "provider" { Some((0, "application/json", vec![])) }
                else if streaming {
                    let frames = format!("data: {}\n\ndata: [DONE]\n\n", json!({"model":budget::profile().model, "choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2}}));
                    Some((200, "text/event-stream", frames.into_bytes()))
                } else { json_reply(completion("ok", true)) }
            }).await;
            let client = client(&control, &provider);
            let error = if streaming {
                match client.chat_stream(request()).await {
                    Ok(mut stream) => stream.next().await.unwrap().unwrap_err(),
                    Err(error) => error,
                }
            } else {
                client.chat(request()).await.unwrap_err()
            };
            if stage == "reserve" {
                assert!(error.is::<BudgetControlError>());
            } else {
                assert!(error.is::<BudgetEvidenceError>());
            }
            assert_eq!(provider.count(), usize::from(stage != "reserve"));
            assert_eq!(control.count(), if stage == "settle" { 2 } else { 1 });
            let attempts: f64 = handle
                .render()
                .lines()
                .filter(|line| line.starts_with("novelworld_llm_attempts_total{"))
                .map(|line| line.rsplit_once(' ').unwrap().1.parse::<f64>().unwrap())
                .sum();
            assert_eq!(
                attempts,
                if stage == "reserve" { 0.0 } else { 1.0 },
                "every dispatched attempt, including timeout, is counted once"
            );
        }
    }
}

#[tokio::test]
async fn malformed_or_mismatched_reserve_responses_never_dispatch() {
    for mode in ["binding", "amount", "duplicate", "oversize", "unknown"] {
        let control = LocalHttp::start(move |path, request| {
            let (_, _, bytes) = control_reply(path, request).unwrap();
            if mode == "oversize" {
                return Some((200, "application/json", vec![b' '; 4097]));
            }
            if mode == "duplicate" {
                return Some((
                    200,
                    "application/json",
                    String::from_utf8(bytes)
                        .unwrap()
                        .replace("\"ordinal\":1", "\"ordinal\":1,\"ordinal\":1")
                        .into_bytes(),
                ));
            }
            let mut body: Value = serde_json::from_slice(&bytes).unwrap();
            match mode {
                "binding" => body["binding"]["profile_sha256"] = json!("b".repeat(64)),
                "amount" => body["reservation"]["tokens"] = json!(1),
                "unknown" => body["unregistered_grant"] = json!(true),
                _ => unreachable!(),
            }
            json_reply(body)
        })
        .await;
        let provider = LocalHttp::start(|_, _| json_reply(completion("ok", true))).await;
        assert!(client(&control, &provider)
            .chat(request())
            .await
            .unwrap_err()
            .is::<BudgetControlError>());
        assert_eq!(control.count(), 1);
        assert_eq!(provider.count(), 0);
    }
}

#[tokio::test]
async fn denied_or_lost_reservation_never_dispatches_or_retries() {
    for lost_ack in [false, true] {
        let control = LocalHttp::start(move |_, _| {
            if lost_ack {
                None
            } else {
                Some((429, "application/json", b"{}".to_vec()))
            }
        })
        .await;
        let provider = LocalHttp::start(|_, _| json_reply(completion("ok", true))).await;
        let client = client(&control, &provider);
        assert!(client
            .chat(request())
            .await
            .unwrap_err()
            .is::<BudgetControlError>());
        assert_eq!(control.count(), 1);
        assert_eq!(provider.count(), 0);
        assert!(client
            .chat_stream(request())
            .await
            .err()
            .unwrap()
            .is::<BudgetControlError>());
        assert_eq!(control.count(), 2);
        assert_eq!(provider.count(), 0);
        assert!(client
            .embed(EmbeddingRequest {
                model: "embedding".into(),
                input: "synthetic".into()
            })
            .await
            .unwrap_err()
            .is::<BudgetControlError>());
        assert_eq!(control.count(), 2, "unpriced embedding does not reserve");
        assert_eq!(provider.count(), 0);
    }
}

#[tokio::test]
async fn lost_settlement_ack_retains_provider_attempt_and_usage_metrics() {
    for empty_json in [false, true] {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let control = LocalHttp::start(|path, body| {
            if path.ends_with("/settle") {
                None
            } else {
                control_reply(path, body)
            }
        })
        .await;
        let provider = LocalHttp::start(move |_, _| {
            json_reply(completion(if empty_json { "" } else { "ok" }, true))
        })
        .await;
        let mut request = request();
        request.json_mode = empty_json;
        assert!(client(&control, &provider)
            .chat(request)
            .await
            .unwrap_err()
            .is::<BudgetEvidenceError>());
        let rendered = handle.render();
        let values = |name: &str, label: &str| -> Vec<f64> {
            rendered
                .lines()
                .filter(|line| line.starts_with(&format!("{name}{{")) && line.contains(label))
                .map(|line| line.rsplit_once(' ').unwrap().1.parse().unwrap())
                .collect()
        };
        assert_eq!(
            values("novelworld_llm_attempts_total", "mode=\"sync\""),
            vec![1.0]
        );
        assert_eq!(
            values("novelworld_llm_requests_total", "status=\"evidence_error\""),
            vec![1.0]
        );
        assert_eq!(
            values("novelworld_llm_tokens_total", "type=\"input\""),
            vec![10.0]
        );
        assert_eq!(
            values("novelworld_llm_tokens_total", "type=\"output\""),
            vec![2.0]
        );
        assert_eq!(provider.count(), 1);
        assert_eq!(control.count(), 2);
    }
}

#[tokio::test]
async fn malformed_success_usage_never_retries_settles_or_falls_back() {
    for usage in [
        json!({"prompt_tokens":10}),
        json!({"prompt_tokens":"10","completion_tokens":2}),
        json!({"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":8,"prompt_cache_miss_tokens":8}),
    ] {
        for json_mode in [false, true] {
            let control = LocalHttp::start(control_reply).await;
            let usage = usage.clone();
            let provider = LocalHttp::start(move |_, _| {
                let mut response = completion(if json_mode { "" } else { "ok" }, true);
                response["usage"] = usage.clone();
                json_reply(response)
            })
            .await;
            let mut req = request();
            req.json_mode = json_mode;
            let error = client(&control, &provider).chat(req).await.unwrap_err();
            assert!(error.is::<BudgetEvidenceError>());
            assert_eq!(provider.count(), 1);
            assert_eq!(control.count(), 1);
        }
    }
}

#[tokio::test]
async fn ordinary_malformed_usage_retains_existing_retry_behavior() {
    let control = LocalHttp::start(control_reply).await;
    let calls = Arc::new(Mutex::new(0));
    let provider = LocalHttp::start(move |_, _| {
        let mut calls = calls.lock().unwrap();
        *calls += 1;
        let mut response = completion("ok", true);
        if *calls == 1 {
            response["usage"] = json!({"prompt_tokens":10});
        }
        json_reply(response)
    })
    .await;
    let mut ordinary = client(&control, &provider);
    ordinary.budget = Ok(None);
    let result = ordinary
        .chat_with_deadline(
            request(),
            Some(tokio::time::Instant::now() + Duration::from_secs(5)),
        )
        .await;
    assert_eq!(result.unwrap().content, "ok");
    assert_eq!(provider.count(), 2);
    assert_eq!(control.count(), 0);
}

#[tokio::test]
async fn successful_sync_requires_matching_usage_and_settlement_ack() {
    for (with_usage, settle_ack) in [(true, true), (false, true), (true, false)] {
        let control = LocalHttp::start(move |path, body| {
            if !settle_ack && path.ends_with("/settle") {
                None
            } else {
                control_reply(path, body)
            }
        })
        .await;
        let provider = LocalHttp::start(move |_, _| json_reply(completion("ok", with_usage))).await;
        let result = client(&control, &provider).chat(request()).await;
        if with_usage && settle_ack {
            assert_eq!(result.unwrap().content, "ok");
        } else {
            assert!(result.unwrap_err().is::<BudgetEvidenceError>());
        }
        assert_eq!(provider.count(), 1);
        assert_eq!(control.count(), if with_usage { 2 } else { 1 });
    }
}

#[tokio::test]
async fn retry_and_json_fallback_require_fresh_reservations() {
    for fallback in [false, true] {
        let control = LocalHttp::start(control_reply).await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let provider = LocalHttp::start(move |_, _| {
            if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                if fallback {
                    json_reply(completion("", true))
                } else {
                    Some((429, "application/json", b"{}".to_vec()))
                }
            } else {
                json_reply(completion("ok", true))
            }
        })
        .await;
        let mut request = request();
        request.json_mode = fallback;
        assert_eq!(
            client(&control, &provider)
                .chat(request)
                .await
                .unwrap()
                .content,
            "ok"
        );
        assert_eq!(provider.count(), 2);
        let records = control.requests.lock().unwrap();
        let ids: Vec<_> = records
            .iter()
            .filter(|request| request.get("operation").is_some())
            .map(|request| request["attempt_id"].clone())
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert_eq!(records.len(), if fallback { 4 } else { 3 });
    }
}

#[tokio::test]
async fn invalid_stream_evidence_stops_once_without_settlement_or_finished() {
    let model = &budget::profile().model;
    let usage = json!({"prompt_tokens":10,"completion_tokens":2});
    for (frames, done) in [
        (
            vec![
                json!({"model":model,"choices":[],"usage":{"prompt_tokens":"10","completion_tokens":2}}),
            ],
            true,
        ),
        (
            vec![
                json!({"model":model,"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":8,"prompt_cache_miss_tokens":8}}),
            ],
            true,
        ),
        (
            vec![
                json!({"model":model,"choices":[]}),
                json!({"model":"different","choices":[]}),
            ],
            true,
        ),
        (
            vec![
                json!({"model":model,"choices":[],"usage":usage}),
                json!({"model":model,"choices":[],"usage":usage}),
            ],
            true,
        ),
        (vec![json!({"model":model,"choices":[]})], false),
    ] {
        let control = LocalHttp::start(control_reply).await;
        let provider = LocalHttp::start(move |_, _| {
            let mut raw = frames
                .iter()
                .map(|frame| format!("data: {frame}\n\n"))
                .collect::<String>();
            if done {
                raw.push_str("data: [DONE]\n\n");
            }
            Some((200, "text/event-stream", raw.into_bytes()))
        })
        .await;
        let events = client(&control, &provider)
            .chat_stream(request())
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert_eq!(events.len(), 1);
        assert!(events[0].as_ref().unwrap_err().is::<BudgetEvidenceError>());
        assert_eq!(provider.count(), 1);
        assert_eq!(control.count(), 1);
    }
}

#[tokio::test]
async fn stream_requires_settlement_before_finished_and_never_refunds_a_drop() {
    for (usage, acknowledge, drop_stream) in [
        (true, true, false),
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        let control = LocalHttp::start(move |path, body| {
            if !acknowledge && path.ends_with("/settle") {
                None
            } else {
                control_reply(path, body)
            }
        })
        .await;
        let provider = LocalHttp::start(move |_, _| {
            let mut frames = format!("data: {}\n\n", json!({"model":budget::profile().model,"choices":[{"delta":{"content":"partial"},"finish_reason":null}]}));
            if usage { frames.push_str(&format!("data: {}\n\n", json!({"model":budget::profile().model,"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2}}))); }
            frames.push_str("data: [DONE]\n\n");
            Some((200, "text/event-stream", frames.into_bytes()))
        }).await;
        let client = client(&control, &provider);
        let mut stream = client.chat_stream(request()).await.unwrap();
        assert!(matches!(
            stream.next().await,
            Some(Ok(ChatStreamEvent::Delta(_)))
        ));
        if drop_stream {
            drop(stream);
            assert_eq!(control.count(), 1);
        } else {
            let terminal = stream.next().await.unwrap();
            if usage && acknowledge {
                assert_eq!(terminal.unwrap(), ChatStreamEvent::Finished);
                assert_eq!(
                    control.count(),
                    2,
                    "settlement acknowledged before Finished"
                );
            } else {
                assert!(terminal.unwrap_err().is::<BudgetEvidenceError>());
            }
            assert!(stream.next().await.is_none());
        }
        assert_eq!(provider.count(), 1);
    }
}
