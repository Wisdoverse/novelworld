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
    // Extension methods are caller-controlled tokens, so aggregate them rather
    // than creating a metric series for every distinct token.
    let method = req.method().as_str();
    let method = match method {
        "GET" | "HEAD" | "POST" | "PUT" | "DELETE" | "CONNECT" | "OPTIONS" | "TRACE" | "PATCH"
        | "QUERY" => method,
        _ => "OTHER",
    }
    .to_owned();
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
        http::{Method, Request, StatusCode},
        middleware,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    #[tokio::test(flavor = "current_thread")]
    async fn http_methods_and_paths_create_only_bounded_metric_series() {
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

        let methods = [
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::CONNECT,
            Method::OPTIONS,
            Method::TRACE,
            Method::PATCH,
            Method::QUERY,
        ];
        let extensions = (0..50)
            .map(|index| Method::from_bytes(format!("PRIVATE-METHOD-{index}").as_bytes()).unwrap())
            .chain([
                Method::from_bytes(b"get").unwrap(),
                Method::from_bytes(
                    b"PRIVATE-EXTENSION-WITH-CALLER-CONTROLLED-IDENTIFIER-0123456789-ABCDEFGHIJKLMNOPQRSTUVWXYZ",
                )
                .unwrap(),
            ])
            .collect::<Vec<_>>();

        for (index, method) in methods.iter().chain(&extensions).enumerate() {
            let matched = Request::builder()
                .method(method.clone())
                .uri(format!("/api/novels/random-{index}"))
                .body(Body::empty())
                .unwrap();
            let matched_status = if method == Method::GET || method == Method::HEAD {
                StatusCode::NO_CONTENT
            } else {
                StatusCode::METHOD_NOT_ALLOWED
            };
            assert_eq!(
                app.clone().oneshot(matched).await.unwrap().status(),
                matched_status
            );
            let unmatched = Request::builder()
                .method(method.clone())
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
        let duration_series = rendered
            .lines()
            .filter(|line| line.starts_with("http_request_duration_seconds_count{"))
            .collect::<Vec<_>>();
        for series in [&request_series, &duration_series] {
            assert_eq!(series.len(), (methods.len() + 1) * 2, "{rendered}");
            for method in methods
                .iter()
                .map(Method::as_str)
                .chain(std::iter::once("OTHER"))
            {
                for path in ["/api/novels/{*path}", "unmatched"] {
                    let line = series
                        .iter()
                        .find(|line| {
                            line.contains(&format!(r#"method="{method}""#))
                                && line.contains(&format!(r#"path="{path}""#))
                        })
                        .expect("expected method and route metric labels");
                    let count = line
                        .split_whitespace()
                        .last()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap();
                    let expected_count = if method == "OTHER" {
                        extensions.len() as u64
                    } else {
                        1
                    };
                    assert_eq!(count, expected_count, "{line}");
                    if line.starts_with("http_requests_total{") {
                        let status = if path == "unmatched" {
                            "404"
                        } else if matches!(method, "GET" | "HEAD") {
                            "204"
                        } else {
                            "405"
                        };
                        assert!(line.contains(&format!(r#"status="{status}""#)), "{line}");
                    }
                }
            }
        }
        for extension in extensions {
            assert!(!rendered.contains(extension.as_str()), "{rendered}");
        }
        assert!(!rendered.contains("attacker-controlled-"));
        assert!(!rendered.contains("random-"));
    }
}
