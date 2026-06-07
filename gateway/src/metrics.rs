use axum::{extract::Request, middleware::Next, response::Response};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::PrometheusHandle;
use std::time::Instant;

/// Install the Prometheus metrics recorder and return a handle for rendering.
pub fn init_metrics() -> PrometheusHandle {
    let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
    builder
        .install_recorder()
        .expect("failed to install Prometheus metrics recorder")
}

/// Middleware that tracks request count, duration, and in-flight gauge.
pub async fn metrics_middleware(req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let normalized = normalize_path(&path);

    gauge!("http_requests_in_flight").increment(1.0);
    let start = Instant::now();

    let response = next.run(req).await;

    let status = response.status().as_u16().to_string();
    let duration = start.elapsed().as_secs_f64();

    counter!("http_requests_total", "method" => method.clone(), "path" => normalized.clone(), "status" => status)
        .increment(1);
    histogram!("http_request_duration_seconds", "method" => method, "path" => normalized)
        .record(duration);
    gauge!("http_requests_in_flight").decrement(1.0);

    response
}

/// Collapse UUIDs and numeric path segments to prevent metric cardinality explosion.
fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.len() == 36 && segment.chars().filter(|c| *c == '-').count() == 4 {
                ":id"
            } else if !segment.is_empty() && segment.chars().all(|c| c.is_ascii_digit()) {
                ":num"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_uuid() {
        assert_eq!(
            normalize_path("/api/novels/550e8400-e29b-41d4-a716-446655440000/chapters"),
            "/api/novels/:id/chapters"
        );
    }

    #[test]
    fn test_normalize_path_numeric() {
        assert_eq!(normalize_path("/api/novels/42/chapters"), "/api/novels/:num/chapters");
    }

    #[test]
    fn test_normalize_path_clean() {
        assert_eq!(normalize_path("/api/novels"), "/api/novels");
    }
}
