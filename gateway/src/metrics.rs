use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
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
    // Route templates are bounded by source code. Never label metrics with the
    // raw URI: unmatched attacker-controlled paths would create one series per
    // request.
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or("unmatched")
        .to_owned();

    gauge!("http_requests_in_flight").increment(1.0);
    let start = Instant::now();

    let response = next.run(req).await;

    let status = response.status().as_u16().to_string();
    let duration = start.elapsed().as_secs_f64();

    counter!("http_requests_total", "method" => method.clone(), "path" => path.clone(), "status" => status)
        .increment(1);
    histogram!("http_request_duration_seconds", "method" => method, "path" => path)
        .record(duration);
    gauge!("http_requests_in_flight").decrement(1.0);

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    #[tokio::test(flavor = "current_thread")]
    async fn dynamic_and_unmatched_paths_create_only_bounded_metric_series() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let app = Router::new()
            .route(
                "/api/novels/{*path}",
                get(|| async { StatusCode::NO_CONTENT }),
            )
            .fallback(|| async { StatusCode::NOT_FOUND })
            .layer(middleware::from_fn(metrics_middleware));

        for index in 0..50 {
            let matched = Request::builder()
                .uri(format!("/api/novels/random-{index}"))
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                app.clone().oneshot(matched).await.unwrap().status(),
                StatusCode::NO_CONTENT
            );
            let unmatched = Request::builder()
                .uri(format!("/attacker-controlled-{index}"))
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                app.clone().oneshot(unmatched).await.unwrap().status(),
                StatusCode::NOT_FOUND
            );
        }

        let rendered = handle.render();
        let request_series = rendered
            .lines()
            .filter(|line| line.starts_with("http_requests_total{"))
            .collect::<Vec<_>>();
        assert_eq!(request_series.len(), 2, "{rendered}");
        assert!(request_series
            .iter()
            .any(|line| line.contains(r#"path="/api/novels/{*path}""#)));
        assert!(request_series
            .iter()
            .any(|line| line.contains(r#"path="unmatched""#)));
        assert!(!rendered.contains("attacker-controlled-"));
        assert!(!rendered.contains("random-"));
    }
}
