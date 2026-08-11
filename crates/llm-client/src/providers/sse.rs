use crate::types::{ChatStream, ChatStreamEvent};
use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut};
use futures::{stream, Stream, StreamExt};
use std::{collections::VecDeque, fmt::Display, pin::Pin};

pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseFrame {
    pub(crate) event: String,
    pub(crate) data: String,
}

#[derive(Default)]
struct SseDecoder {
    buffer: BytesMut,
    event: String,
    data: Vec<String>,
    frame_bytes: usize,
    first_line: bool,
}

impl SseDecoder {
    fn new() -> Self {
        Self {
            first_line: true,
            ..Self::default()
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseFrame>> {
        self.buffer.extend_from_slice(chunk);
        self.drain_lines(false)
    }

    fn finish(&mut self) -> Result<Vec<SseFrame>> {
        let frames = self.drain_lines(true)?;
        if !self.buffer.is_empty() {
            std::str::from_utf8(&self.buffer)
                .map_err(|_| anyhow!("invalid UTF-8 in SSE stream"))?;
            return Err(anyhow!("SSE stream ended in the middle of a line"));
        }
        if !self.event.is_empty() || !self.data.is_empty() {
            return Err(anyhow!("SSE stream ended in the middle of an event"));
        }
        Ok(frames)
    }

    fn drain_lines(&mut self, eof: bool) -> Result<Vec<SseFrame>> {
        let mut frames = Vec::new();

        while let Some(index) = self
            .buffer
            .iter()
            .position(|byte| matches!(*byte, b'\r' | b'\n'))
        {
            if self.buffer[index] == b'\r' && index + 1 == self.buffer.len() && !eof {
                break;
            }

            let delimiter_len =
                if self.buffer[index] == b'\r' && self.buffer.get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
            let line = self.buffer.split_to(index + delimiter_len);
            self.frame_bytes = self
                .frame_bytes
                .checked_add(line.len())
                .ok_or_else(|| anyhow!("SSE frame exceeds {MAX_FRAME_BYTES} bytes"))?;
            if self.frame_bytes > MAX_FRAME_BYTES {
                return Err(anyhow!("SSE frame exceeds {MAX_FRAME_BYTES} bytes"));
            }

            let line = std::str::from_utf8(&line[..index])
                .map_err(|_| anyhow!("invalid UTF-8 in SSE stream"))?;
            let line = if self.first_line {
                self.first_line = false;
                line.strip_prefix('\u{feff}').unwrap_or(line)
            } else {
                line
            };

            if line.is_empty() {
                if !self.data.is_empty() {
                    frames.push(SseFrame {
                        event: if self.event.is_empty() {
                            "message".into()
                        } else {
                            std::mem::take(&mut self.event)
                        },
                        data: std::mem::take(&mut self.data).join("\n"),
                    });
                } else {
                    self.event.clear();
                }
                self.frame_bytes = 0;
                continue;
            }

            if line.starts_with(':') {
                continue;
            }

            let (field, mut value) = line.split_once(':').unwrap_or((line, ""));
            if let Some(stripped) = value.strip_prefix(' ') {
                value = stripped;
            }
            match field {
                "event" => self.event = value.to_string(),
                "data" => self.data.push(value.to_string()),
                _ => {}
            }
        }

        if self.frame_bytes.saturating_add(self.buffer.len()) > MAX_FRAME_BYTES {
            return Err(anyhow!("SSE frame exceeds {MAX_FRAME_BYTES} bytes"));
        }

        Ok(frames)
    }
}

struct DecodeState<S> {
    upstream: Pin<Box<S>>,
    decoder: SseDecoder,
    pending: VecDeque<Result<ChatStreamEvent>>,
    terminal: bool,
    failed: bool,
    eof: bool,
}

impl<S> DecodeState<S> {
    fn queue_frames(
        &mut self,
        frames: Vec<SseFrame>,
        parse: fn(SseFrame) -> Result<Vec<ChatStreamEvent>>,
    ) -> Result<()> {
        for frame in frames {
            if self.terminal {
                break;
            }
            for event in parse(frame)? {
                if self.terminal {
                    break;
                }
                if event == ChatStreamEvent::Finished {
                    self.terminal = true;
                }
                self.pending.push_back(Ok(event));
            }
        }
        Ok(())
    }

    fn fail(&mut self, error: anyhow::Error) {
        self.failed = true;
        self.pending.push_back(Err(error));
    }
}

pub(crate) fn decode_stream<S, E>(
    upstream: S,
    parse: fn(SseFrame) -> Result<Vec<ChatStreamEvent>>,
) -> ChatStream
where
    S: Stream<Item = std::result::Result<Bytes, E>> + Send + 'static,
    E: Display + Send + 'static,
{
    let state = DecodeState {
        upstream: Box::pin(upstream),
        decoder: SseDecoder::new(),
        pending: VecDeque::new(),
        terminal: false,
        failed: false,
        eof: false,
    };

    Box::pin(stream::unfold(state, move |mut state| async move {
        loop {
            if let Some(item) = state.pending.pop_front() {
                return Some((item, state));
            }
            if state.terminal || state.failed {
                return None;
            }
            if state.eof {
                state.fail(anyhow!("LLM SSE stream ended without a terminal event"));
                continue;
            }

            match state.upstream.next().await {
                Some(Ok(chunk)) => match state.decoder.push(&chunk) {
                    Ok(frames) => {
                        if let Err(error) = state.queue_frames(frames, parse) {
                            state.fail(error);
                        }
                    }
                    Err(error) => state.fail(error),
                },
                Some(Err(error)) => {
                    state.fail(anyhow!("LLM stream transport error: {error}"));
                }
                None => {
                    state.eof = true;
                    match state.decoder.finish() {
                        Ok(frames) => {
                            if let Err(error) = state.queue_frames(frames, parse) {
                                state.fail(error);
                            }
                        }
                        Err(error) => state.fail(error),
                    }
                }
            }
        }
    }))
}
