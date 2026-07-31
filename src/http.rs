use axum::{
    Router,
    extract::State,
    http::{StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
    routing::get,
};
use prometheus::{Encoder, Registry, TextEncoder};
use tracing::error;

pub fn router(registry: Option<Registry>) -> Router {
    match registry {
        Some(registry) => Router::new()
            .route("/metrics", get(metrics))
            .route("/healthz", get(health))
            .with_state(registry),
        None => Router::new().route("/healthz", get(health)),
    }
}

async fn health() -> &'static str {
    "ok\n"
}

async fn metrics(State(registry): State<Registry>) -> Response {
    let encoder = TextEncoder::new();
    let mut body = Vec::new();
    match encoder.encode(&registry.gather(), &mut body) {
        Ok(()) => ([(CONTENT_TYPE, encoder.format_type())], body).into_response(),
        Err(encode_error) => {
            error!(error = %encode_error, "failed to encode Prometheus metrics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "metrics encoding failed\n",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt as _;
    use prometheus::Registry;
    use tower::ServiceExt as _;

    use super::router;

    #[tokio::test]
    async fn serves_health_endpoint() {
        let response = router(None)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), 200);
        assert_eq!(
            response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes(),
            "ok\n"
        );
    }

    #[tokio::test]
    async fn serves_prometheus_content_type() {
        let response = router(Some(Registry::new()))
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-type"],
            "text/plain; version=0.0.4"
        );
    }

    #[tokio::test]
    async fn omits_metrics_route_without_prometheus_exporter() {
        let response = router(None)
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), 404);
    }
}
