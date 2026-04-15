//! `GET /api/v1/openapi.json` — OpenAPI specification.

use axum::response::IntoResponse;

/// Serve OpenAPI specification as JSON.
///
/// Returns the complete OpenAPI 3.0 specification for the Ironflow REST API.
///
/// # Errors
///
/// - Returns 400 if the `openapi` feature is not enabled
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/openapi.json",
        tags = ["openapi"],
        responses(
            (status = 200, description = "OpenAPI specification in JSON format", content_type = "application/json"),
        )
    )
)]
pub async fn openapi_spec() -> impl IntoResponse {
    #[cfg(feature = "openapi")]
    {
        use axum::Json;
        use utoipa::OpenApi;
        use crate::openapi::ApiDoc;

        Json(ApiDoc::openapi())
    }

    #[cfg(not(feature = "openapi"))]
    {
        crate::error::ApiError::BadRequest(
            "OpenAPI documentation is not enabled".to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    #[tokio::test]
    #[cfg(feature = "openapi")]
    async fn openapi_spec_available() {
        let app = Router::new().route("/", get(openapi_spec));

        let req = Request::builder()
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[cfg(not(feature = "openapi"))]
    async fn openapi_spec_disabled() {
        let app = Router::new().route("/", get(openapi_spec));

        let req = Request::builder()
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
