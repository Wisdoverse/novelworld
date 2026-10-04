use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{openai::TruncatedCompletion, sse::SseFrame};
use crate::types::{ChatRequest, ChatResponse, ChatStreamEvent, LlmApiError, Usage};

pub(crate) fn request_body(request: &ChatRequest, stream: bool) -> Result<Value> {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for message in &request.messages {
        match message.role.as_str() {
            "system" | "developer" if messages.is_empty() => system.push(message.content.clone()),
            "user" | "assistant" => messages.push(message.clone()),
            _ => {
                return Err(LlmApiError {
                    status: 400,
                    message:
                        "Claude requires initial system instructions and user/assistant messages"
                            .into(),
                    retry_after: None,
                }
                .into())
            }
        }
    }
    if messages.is_empty() {
        bail!("Claude requires at least one conversation message");
    }
    if request.json_mode {
        // ChatRequest has no schema. Domain consumers still validate the returned JSON.
        system.push(
            "Respond with a non-empty valid JSON object only, without Markdown fences.".into(),
        );
    }
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "max_tokens": request.effective_max_output_tokens().ok_or_else(|| anyhow!("Claude requires an output-token limit"))?,
        "stream": stream,
    });
    if !system.is_empty() {
        body["system"] = Value::String(system.join("\n\n"));
    }
    // Current Claude models own their thinking/sampling defaults. Do not send
    // OpenAI sampling fields or DeepSeek's disabled-thinking control.
    Ok(body)
}

#[derive(Clone, Copy, Deserialize)]
struct NativeUsage {
    input_tokens: u32,
    output_tokens: u32,
    cache_creation_input_tokens: Option<u32>,
    cache_read_input_tokens: Option<u32>,
}

impl NativeUsage {
    fn into_usage(self) -> Result<Usage> {
        let input = self
            .input_tokens
            .checked_add(self.cache_creation_input_tokens.unwrap_or(0))
            .and_then(|input| input.checked_add(self.cache_read_input_tokens.unwrap_or(0)))
            .ok_or_else(|| anyhow!("Claude input usage overflow"))?;
        Usage::new(input, self.output_tokens, self.cache_read_input_tokens)
    }

    fn update(&mut self, delta: &Value) -> Result<bool> {
        for (field, current) in [
            ("input_tokens", &mut self.input_tokens),
            ("output_tokens", &mut self.output_tokens),
        ] {
            if let Some(value) = delta.get(field) {
                let value: u32 = serde_json::from_value(value.clone())
                    .map_err(|_| anyhow!("invalid Claude cumulative token count"))?;
                if value < *current {
                    bail!("Claude cumulative usage decreased");
                }
                *current = value;
            }
        }
        for (field, current) in [
            (
                "cache_creation_input_tokens",
                &mut self.cache_creation_input_tokens,
            ),
            ("cache_read_input_tokens", &mut self.cache_read_input_tokens),
        ] {
            if let Some(value) = delta.get(field) {
                let value: u32 = serde_json::from_value(value.clone())
                    .map_err(|_| anyhow!("invalid Claude cumulative cache count"))?;
                if current.is_some_and(|old| value < old) {
                    bail!("Claude cumulative cache usage decreased");
                }
                *current = Some(value);
            }
        }
        self.into_usage()?;
        Ok(delta.get("output_tokens").is_some())
    }
}

#[derive(Deserialize)]
struct NativeMessage {
    #[serde(rename = "type")]
    kind: String,
    role: String,
    model: String,
    content: Vec<Value>,
    usage: NativeUsage,
    stop_reason: Option<String>,
}

fn message(value: Value) -> Result<NativeMessage> {
    let message: NativeMessage =
        serde_json::from_value(value).map_err(|_| anyhow!("invalid Claude response envelope"))?;
    if message.kind != "message" || message.role != "assistant" || message.model.trim().is_empty() {
        bail!("invalid Claude response envelope");
    }
    message.usage.into_usage()?;
    Ok(message)
}

fn complete_stop(reason: Option<&str>) -> Result<()> {
    match reason {
        Some("end_turn") => Ok(()),
        Some("max_tokens") => Err(TruncatedCompletion.into()),
        _ => Err(anyhow!(
            "Claude response ended without a supported completion reason"
        )),
    }
}

pub(crate) fn response(value: Value) -> Result<ChatResponse> {
    let message = message(value)?;
    let mut content = String::new();
    for block in message.content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => content.push_str(
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("Claude text block is missing text"))?,
            ),
            Some("thinking") if block.get("thinking").is_some_and(Value::is_string) => {}
            Some("redacted_thinking") if block.get("data").is_some_and(Value::is_string) => {}
            _ => bail!("unsupported Claude response content block"),
        }
    }
    complete_stop(message.stop_reason.as_deref())?;
    if content.trim().is_empty() {
        bail!("Claude returned an empty text response");
    }
    Ok(ChatResponse {
        content,
        model: message.model,
        usage: Some(message.usage.into_usage()?),
    })
}

#[derive(Clone, Copy)]
enum BlockKind {
    Text,
    Thinking,
    RedactedThinking,
}

#[derive(Default)]
struct StreamParser {
    usage: Option<NativeUsage>,
    next_index: u64,
    block: Option<(u64, BlockKind)>,
    message_delta: bool,
    output_updated: bool,
    stop_reason: Option<String>,
    has_text: bool,
}

pub(crate) fn stream_parser() -> impl FnMut(SseFrame) -> Result<Vec<ChatStreamEvent>> + Send {
    let mut state = StreamParser::default();
    move |frame| state.parse(frame)
}

impl StreamParser {
    fn parse(&mut self, frame: SseFrame) -> Result<Vec<ChatStreamEvent>> {
        if !matches!(
            frame.event.as_str(),
            "message_start"
                | "content_block_start"
                | "content_block_delta"
                | "content_block_stop"
                | "message_delta"
                | "message_stop"
                | "ping"
                | "error"
        ) {
            return Ok(Vec::new()); // Anthropic's versioning contract permits new event types.
        }
        let value: Value = serde_json::from_str(&frame.data)?;
        if value.get("type").and_then(Value::as_str) != Some(frame.event.as_str()) {
            bail!("Claude SSE event type mismatch");
        }
        if frame.event == "ping" {
            return Ok(Vec::new());
        }
        if frame.event == "error" {
            return Err(LlmApiError {
                status: match value["error"]["type"].as_str() {
                    Some("overloaded_error") => 529,
                    Some("rate_limit_error") => 429,
                    _ => 500,
                },
                message: "Claude stream failed".into(),
                retry_after: None,
            }
            .into());
        }
        if frame.event == "message_start" {
            if self.usage.is_some() {
                bail!("duplicate Claude message_start");
            }
            let message = message(value["message"].clone())?;
            if !message.content.is_empty() || message.stop_reason.is_some() {
                bail!("Claude stream did not start with an empty message");
            }
            self.usage = Some(message.usage);
            return Ok(vec![ChatStreamEvent::ResponseModel(message.model)]);
        }
        if self.usage.is_none() {
            bail!("Claude stream is missing message_start");
        }
        match frame.event.as_str() {
            "content_block_start" => {
                let index = value["index"]
                    .as_u64()
                    .ok_or_else(|| anyhow!("invalid Claude block index"))?;
                if self.block.is_some() || self.message_delta || index != self.next_index {
                    bail!("invalid Claude content block ordering");
                }
                let kind = match value["content_block"]["type"].as_str() {
                    Some("text") => BlockKind::Text,
                    Some("thinking") if value["content_block"]["thinking"].is_string() => {
                        BlockKind::Thinking
                    }
                    Some("redacted_thinking") if value["content_block"]["data"].is_string() => {
                        BlockKind::RedactedThinking
                    }
                    _ => bail!("unsupported Claude stream content block"),
                };
                self.block = Some((index, kind));
                if matches!(kind, BlockKind::Text) {
                    let text = value["content_block"]["text"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Claude text block is missing text"))?;
                    self.has_text |= !text.trim().is_empty();
                    return Ok(vec![ChatStreamEvent::Delta(text.into())]);
                }
            }
            "content_block_delta" => {
                let (index, kind) = self
                    .block
                    .ok_or_else(|| anyhow!("Claude delta has no active block"))?;
                if value["index"].as_u64() != Some(index) {
                    bail!("Claude delta block index mismatch");
                }
                match (kind, value["delta"]["type"].as_str()) {
                    (BlockKind::Text, Some("text_delta")) => {
                        let text = value["delta"]["text"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Claude text delta is missing text"))?;
                        self.has_text |= !text.trim().is_empty();
                        return Ok(vec![ChatStreamEvent::Delta(text.into())]);
                    }
                    (BlockKind::Thinking, Some("thinking_delta"))
                        if value["delta"]["thinking"].is_string() => {}
                    (BlockKind::Thinking, Some("signature_delta"))
                        if value["delta"]["signature"].is_string() => {}
                    _ => bail!("unsupported Claude content block delta"),
                }
            }
            "content_block_stop" => {
                let (index, _) = self
                    .block
                    .take()
                    .ok_or_else(|| anyhow!("Claude stop has no active block"))?;
                if value["index"].as_u64() != Some(index) {
                    bail!("Claude stop block index mismatch");
                }
                self.next_index = self
                    .next_index
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("Claude block index overflow"))?;
            }
            "message_delta" => {
                if self.block.is_some() {
                    bail!("Claude message_delta interrupted a content block");
                }
                if !value.get("delta").is_some_and(Value::is_object) {
                    bail!("Claude message_delta is missing delta");
                }
                self.message_delta = true;
                let usage = value
                    .get("usage")
                    .filter(|value| value.is_object())
                    .ok_or_else(|| anyhow!("Claude message_delta is missing usage"))?;
                self.output_updated |= self.usage.as_mut().unwrap().update(usage)?;
                if let Some(reason) = value["delta"]
                    .get("stop_reason")
                    .filter(|value| !value.is_null())
                {
                    if self.stop_reason.is_some() {
                        bail!("duplicate Claude stop reason");
                    }
                    let reason = reason
                        .as_str()
                        .ok_or_else(|| anyhow!("invalid Claude stop reason"))?;
                    // Keep only bounded known values, never arbitrary streamed content.
                    if !matches!(reason, "end_turn" | "max_tokens") {
                        bail!("unsupported Claude stop reason");
                    }
                    self.stop_reason = Some(reason.into());
                }
            }
            "message_stop" => {
                if self.block.is_some() || !self.message_delta || !self.output_updated {
                    bail!("incomplete Claude stream");
                }
                complete_stop(self.stop_reason.as_deref())?;
                if !self.has_text {
                    bail!("Claude stream returned no text");
                }
                return Ok(vec![
                    ChatStreamEvent::Usage(self.usage.unwrap().into_usage()?),
                    ChatStreamEvent::Finished,
                ]);
            }
            _ => unreachable!(),
        }
        Ok(Vec::new())
    }
}
