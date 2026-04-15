//! `GET /api/v1/health-check` — Liveness probe.

/// Health check handler. Always returns 200 OK.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/health-check",
        tags = ["health"],
        responses(
            (status = 200, description = "Service is healthy")
        )
    )
)]
pub async fn health_check() -> &'static str {
    "OK"
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn returns_ok() {
        let app = Router::new().route("/health-check", get(health_check));

        let request = Request::builder()
            .method("GET")
            .uri("/health-check")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"OK");
    }
}
