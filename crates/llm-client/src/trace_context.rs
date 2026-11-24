//! SPEC 14.1 trace propagation for internal service calls.
//!
//! The request span already carries `trace_id`. This layer keeps a validated
//! copy in span extensions so internal clients can forward it as `X-Trace-Id`
//! without threading a parameter through domain or application code.

use std::fmt;

use tracing::{
    field::{Field, Visit},
    span, Subscriber,
};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer, Registry};

pub const TRACE_ID_HEADER: &str = "X-Trace-Id";

struct TraceId(String);

/// Stores a span's `trace_id` field when it matches the inbound header alphabet.
pub struct TraceContextLayer;

#[derive(Default)]
struct TraceIdVisitor(Option<String>);

impl Visit for TraceIdVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "trace_id" {
            self.0 = Some(value.to_owned());
        }
    }

    // `trace_id = %value` arrives here as a display wrapper.
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "trace_id" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

fn valid_trace_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

impl TraceContextLayer {
    fn store<S>(visitor: TraceIdVisitor, id: &span::Id, ctx: &Context<'_, S>)
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let (Some(value), Some(span)) = (visitor.0, ctx.span(id)) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        if valid_trace_id(&value) {
            extensions.replace(TraceId(value));
        } else {
            extensions.remove::<TraceId>();
        }
    }
}

impl<S> Layer<S> for TraceContextLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let mut visitor = TraceIdVisitor::default();
        attrs.record(&mut visitor);
        Self::store(visitor, id, &ctx);
    }

    fn on_record(&self, id: &span::Id, values: &span::Record<'_>, ctx: Context<'_, S>) {
        let mut visitor = TraceIdVisitor::default();
        values.record(&mut visitor);
        Self::store(visitor, id, &ctx);
    }
}

/// The nearest stored trace ID in the current span scope, if any.
pub fn current_trace_id() -> Option<String> {
    tracing::Span::current()
        .with_subscriber(|(id, dispatch)| {
            let span = dispatch.downcast_ref::<Registry>()?.span(id)?;
            span.scope()
                .find_map(|span| span.extensions().get::<TraceId>().map(|id| id.0.clone()))
        })
        .flatten()
}

pub trait TraceContextExt {
    /// Adds `X-Trace-Id` when the current span scope has a trace ID.
    fn with_trace_context(self) -> Self;
}

impl TraceContextExt for reqwest::RequestBuilder {
    fn with_trace_context(self) -> Self {
        // Only validated IDs are stored, so no second validation is needed.
        match current_trace_id() {
            Some(trace_id) => self.header(TRACE_ID_HEADER, trace_id),
            None => self,
        }
    }
}

/// Test-only HTTP endpoint that reports each request's raw header block.
#[cfg(test)]
pub(crate) mod test_support {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub(crate) struct HeaderCapture {
        pub(crate) origin: String,
        received: tokio::sync::mpsc::UnboundedReceiver<String>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for HeaderCapture {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl HeaderCapture {
        pub(crate) async fn start() -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let (sender, received) = tokio::sync::mpsc::unbounded_channel();
            let task = tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut bytes = Vec::new();
                    let headers = loop {
                        let mut chunk = [0; 4096];
                        let read = socket.read(&mut chunk).await.unwrap();
                        assert!(read > 0 && bytes.len() < 32768);
                        bytes.extend_from_slice(&chunk[..read]);
                        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
                        {
                            let headers = String::from_utf8(bytes[..end].to_vec())
                                .unwrap()
                                .to_ascii_lowercase();
                            let length: usize = headers
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length: "))
                                .map_or(0, |value| value.parse().unwrap());
                            while bytes.len() < end + 4 + length {
                                let read = socket.read(&mut chunk).await.unwrap();
                                assert!(read > 0);
                                bytes.extend_from_slice(&chunk[..read]);
                            }
                            break headers;
                        }
                    };
                    sender.send(headers).unwrap();
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                        .await;
                }
            });
            Self {
                origin,
                received,
                task,
            }
        }

        /// The forwarded trace ID of the next received request.
        pub(crate) async fn next_trace_id(&mut self) -> Option<String> {
            let headers = self.received.recv().await.unwrap();
            headers
                .lines()
                .find_map(|line| line.strip_prefix("x-trace-id: "))
                .map(str::to_owned)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Instrument;
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

    /// Same layer order and filter shape as the service composition roots.
    fn production_stack(filter: &str) -> impl Subscriber + Send + Sync {
        tracing_subscriber::registry()
            .with(EnvFilter::new(filter))
            .with(fmt::layer().json().with_writer(std::io::sink))
            .with(TraceContextLayer)
    }

    const FILTER: &str = "info,reqwest=off,tower_http=off,novelworld_context=info";

    fn request_span(trace_id: &str) -> tracing::Span {
        tracing::info_span!(target: "novelworld_context", "service", service = "test-service", trace_id = %trace_id)
    }

    fn trace_id_inside(filter: &str, trace_id: &str) -> Option<String> {
        tracing::subscriber::with_default(production_stack(filter), || {
            request_span(trace_id).in_scope(current_trace_id)
        })
    }

    #[test]
    fn display_trace_id_on_the_request_span_is_found() {
        assert_eq!(
            trace_id_inside(FILTER, "trace-abc_1.2").as_deref(),
            Some("trace-abc_1.2")
        );
    }

    #[test]
    fn nested_span_without_a_trace_id_finds_the_request_id() {
        let found = tracing::subscriber::with_default(production_stack(FILTER), || {
            let _service =
                tracing::info_span!(target: "novelworld_context", "service", trace_id = "")
                    .entered();
            request_span("outer-request")
                .in_scope(|| tracing::info_span!("handler", user = 7).in_scope(current_trace_id))
        });
        assert_eq!(found.as_deref(), Some("outer-request"));
    }

    #[test]
    fn late_recorded_trace_id_is_found() {
        let found = tracing::subscriber::with_default(production_stack(FILTER), || {
            let span = tracing::info_span!("late", trace_id = tracing::field::Empty);
            span.record("trace_id", "late-id");
            span.in_scope(current_trace_id)
        });
        assert_eq!(found.as_deref(), Some("late-id"));
    }

    #[test]
    fn empty_invalid_or_oversized_trace_ids_store_nothing() {
        for value in ["", "has space", "quote\"", "ümlaut", &"a".repeat(129)] {
            assert_eq!(trace_id_inside(FILTER, value), None, "{value:?}");
        }
        assert!(trace_id_inside(FILTER, &"a".repeat(128)).is_some());
    }

    #[test]
    fn error_only_filter_keeps_the_novelworld_context_request_span() {
        assert_eq!(
            trace_id_inside("error,novelworld_context=info", "kept-id").as_deref(),
            Some("kept-id")
        );
    }

    #[tokio::test]
    async fn spawned_instrumented_task_finds_the_request_id() {
        let _guard = tracing::subscriber::set_default(production_stack(FILTER));
        let found = tokio::spawn(async { current_trace_id() }.instrument(request_span("spawned")))
            .await
            .unwrap();
        assert_eq!(found.as_deref(), Some("spawned"));
        assert_eq!(current_trace_id(), None);
    }

    #[test]
    fn no_subscriber_finds_nothing() {
        tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), || {
            assert_eq!(request_span("ignored").in_scope(current_trace_id), None);
        });
        assert_eq!(current_trace_id(), None);
    }

    #[test]
    fn request_builder_adds_the_header_only_with_a_trace_id() {
        let client = reqwest::Client::new();
        let build = || {
            client
                .get("https://internal.example/ready")
                .with_trace_context()
                .build()
                .unwrap()
        };
        let inside = tracing::subscriber::with_default(production_stack(FILTER), || {
            request_span("builder-id").in_scope(build)
        });
        assert_eq!(inside.headers()[TRACE_ID_HEADER], "builder-id");
        assert!(!build().headers().contains_key(TRACE_ID_HEADER));
    }
}
