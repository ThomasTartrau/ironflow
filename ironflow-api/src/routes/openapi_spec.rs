//! `GET /api/v1/openapi.json` -- OpenAPI specification.

use axum::response::{IntoResponse, Response};

/// Serve the OpenAPI specification as JSON.
///
/// When the `openapi` feature is disabled, returns 404.
pub async fn openapi_spec() -> Response {
    #[cfg(feature = "openapi")]
    {
        use axum::Json;
        use utoipa::OpenApi;

        use crate::openapi::ApiDoc;

        Json(ApiDoc::openapi()).into_response()
    }

    #[cfg(not(feature = "openapi"))]
    {
        axum::http::StatusCode::NOT_FOUND.into_response()
    }
}
