pub mod openai;
pub(crate) mod sse;

use crate::types::{ChatRequest, HttpResponseEvidence, LlmApiError, ResponseEvidenceError};
use anyhow::Result;
use futures::StreamExt;
use reqwest::header::RETRY_AFTER;
use serde::de::DeserializeOwned;

const MAX_JSON_RESPONSE_BYTES: usize = 1024 * 1024;

pub(crate) async fn response_error(response: reqwest::Response) -> anyhow::Error {
    response_error_with_evidence(response, None).await
}

pub(crate) async fn response_error_with_evidence(
    response: reqwest::Response,
    request: Option<&ChatRequest>,
) -> anyhow::Error {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if request.is_some_and(|request| request.response_observer.is_some()) {
        if let Err(error) = response_body(response, request).await {
            return error;
        }
    }
    LlmApiError {
        status,
        message: "provider request failed".into(),
        retry_after,
    }
    .into()
}

pub(crate) async fn json_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    json_response_with_evidence(response, None).await
}

pub(crate) async fn json_response_with_evidence<T: DeserializeOwned>(
    response: reqwest::Response,
    request: Option<&ChatRequest>,
) -> Result<T> {
    Ok(serde_json::from_slice(
        &response_body(response, request).await?,
    )?)
}

struct ResponseCapture<'a> {
    request: Option<&'a ChatRequest>,
    status: u16,
    body: Vec<u8>,
    observed: bool,
}

impl ResponseCapture<'_> {
    fn observe(&mut self, complete: bool) -> Result<()> {
        self.observed = true;
        if let Some(request) = self.request {
            request.observe_response(HttpResponseEvidence {
                status: self.status,
                body: &self.body,
                complete,
            })?;
        }
        Ok(())
    }
}

impl Drop for ResponseCapture<'_> {
    fn drop(&mut self) {
        if !self.observed {
            // The total deadline may cancel a body read before it returns an error.
            // The observer still retains the prefix and can invalidate the run.
            let _ = self.observe(false);
        }
    }
}

async fn response_body(
    response: reqwest::Response,
    request: Option<&ChatRequest>,
) -> Result<Vec<u8>> {
    let mut capture = ResponseCapture {
        request: request.filter(|request| request.response_observer.is_some()),
        status: response.status().as_u16(),
        body: Vec::new(),
        observed: false,
    };
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                capture.observe(false)?;
                if capture.request.is_some() {
                    return Err(ResponseEvidenceError.into());
                }
                return Err(error.into());
            }
        };
        if capture.body.len().saturating_add(chunk.len()) > MAX_JSON_RESPONSE_BYTES {
            let remaining = MAX_JSON_RESPONSE_BYTES - capture.body.len();
            capture.body.extend_from_slice(&chunk[..remaining]);
            capture.observe(false)?;
            return Err(LlmApiError {
                status: 413,
                message: format!("provider response exceeds {MAX_JSON_RESPONSE_BYTES} bytes"),
                retry_after: None,
            }
            .into());
        }
        capture.body.extend_from_slice(&chunk);
    }
    capture.observe(true)?;
    Ok(std::mem::take(&mut capture.body))
}
