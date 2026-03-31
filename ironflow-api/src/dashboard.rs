//! Embedded dashboard served as a Tower fallback service.
//!
//! Enabled by the `dashboard` cargo feature. The built SPA files from
//! `ironflow-api/dashboard/` are compiled into the binary and served
//! with automatic fallback to `index.html` for client-side routing.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;
use tower::Service;

#[derive(Embed)]
#[folder = "$IRONFLOW_DASHBOARD_DIR"]
struct Assets;

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}

fn serve_embedded(path: &str) -> Response {
    let (file, effective_path) = match <Assets as Embed>::get(path) {
        Some(f) => (Some(f), path),
        None => (<Assets as Embed>::get("index.html"), "index.html"),
    };

    match file {
        Some(content) => {
            let mime = mime_for(effective_path);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime)],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Tower [`Service`] that serves the embedded dashboard SPA.
///
/// Tries to match the request path against embedded assets. If no asset
/// matches, falls back to `index.html` so the SPA router can handle it.
///
/// # Examples
///
/// ```no_run
/// use axum::Router;
/// use ironflow_api::dashboard::EmbeddedDashboard;
///
/// let app: Router = Router::new()
///     .fallback_service(EmbeddedDashboard);
/// ```
#[derive(Clone)]
pub struct EmbeddedDashboard;

impl Service<Request<Body>> for EmbeddedDashboard {
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let path = req.uri().path().trim_start_matches('/').to_string();
        Box::pin(async move { Ok(serve_embedded(&path)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_for_html() {
        assert_eq!(mime_for("index.html"), "text/html; charset=utf-8");
    }

    #[test]
    fn mime_for_js() {
        assert_eq!(mime_for("app.js"), "application/javascript");
    }

    #[test]
    fn mime_for_css() {
        assert_eq!(mime_for("style.css"), "text/css");
    }

    #[test]
    fn mime_for_json() {
        assert_eq!(mime_for("data.json"), "application/json");
    }

    #[test]
    fn mime_for_svg() {
        assert_eq!(mime_for("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn mime_for_png() {
        assert_eq!(mime_for("logo.png"), "image/png");
    }

    #[test]
    fn mime_for_ico() {
        assert_eq!(mime_for("favicon.ico"), "image/x-icon");
    }

    #[test]
    fn mime_for_woff2() {
        assert_eq!(mime_for("font.woff2"), "font/woff2");
    }

    #[test]
    fn mime_for_unknown_extension() {
        assert_eq!(mime_for("file.xyz"), "application/octet-stream");
    }

    #[test]
    fn mime_for_no_extension() {
        assert_eq!(mime_for("README"), "application/octet-stream");
    }
}
