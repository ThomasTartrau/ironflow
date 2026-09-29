//! `GET /api/v1/templates/registry` -- List templates from the configured registry.

use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use serde::Serialize;
use tokio::task::spawn_blocking;
use tracing::warn;

use ironflow_engine::VERSION as ENGINE_VERSION;
use ironflow_templates::error::TemplateError;
use ironflow_templates::registry::{RegistryEntry, fetch_registry_index, resolve_registry_url};

use crate::error::ApiError;
use crate::response::ok;

/// A template entry returned by the API.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TemplateListEntry {
    /// Template name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Category for UI grouping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Git repository URL.
    pub repo: String,
    /// Minimum Ironflow version required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_ironflow_version: Option<String>,
    /// Template authors.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// Latest published version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
}

/// Response for the template registry listing.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TemplateRegistryResponse {
    /// Available templates.
    pub templates: Vec<TemplateListEntry>,
    /// Current Ironflow engine version.
    pub ironflow_version: String,
}

impl From<RegistryEntry> for TemplateListEntry {
    fn from(entry: RegistryEntry) -> Self {
        Self {
            name: entry.name,
            description: entry.description,
            category: entry.category,
            repo: entry.repo,
            min_ironflow_version: entry.min_ironflow_version.map(|v| v.to_string()),
            authors: entry.authors,
            latest_version: entry.latest_version.map(|v| v.to_string()),
        }
    }
}

/// List available templates from the configured registry.
///
/// Fetches the remote registry index and returns the template catalogue.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/templates/registry",
        tags = ["templates"],
        responses(
            (status = 200, description = "Template catalogue", body = TemplateRegistryResponse),
            (status = 404, description = "No registry at the configured URL (`REGISTRY_NOT_FOUND`)"),
            (status = 502, description = "Registry host unreachable (`REGISTRY_UNREACHABLE`)")
        )
    )
)]
pub async fn list_registry_templates(_auth: Authenticated) -> Result<impl IntoResponse, ApiError> {
    let response = fetch_registry_listing(resolve_registry_url(None)).await?;
    Ok(ok(response))
}

/// Fetch the registry index at `registry_url` and build the API response.
///
/// The error message never carries the URL nor the git output, which may hold
/// credentials: both are logged instead.
async fn fetch_registry_listing(
    registry_url: String,
) -> Result<TemplateRegistryResponse, ApiError> {
    let index = spawn_blocking(move || fetch_registry_index(&registry_url))
        .await
        .map_err(|e| ApiError::Internal(format!("task join error: {e}")))?
        .map_err(|e| {
            warn!(error = %e, "template registry fetch failed");
            match e {
                TemplateError::RegistryNotFound { .. } => ApiError::RegistryNotFound,
                TemplateError::RegistryUnreachable { .. } => ApiError::RegistryUnreachable,
                other => ApiError::BadGateway(format!("registry fetch failed: {other}")),
            }
        })?;

    let templates: Vec<TemplateListEntry> = index.templates.into_iter().map(Into::into).collect();
    Ok(TemplateRegistryResponse {
        templates,
        ironflow_version: ENGINE_VERSION.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use axum::response::Response;
    use semver::Version;

    use super::*;

    #[test]
    fn template_list_entry_from_registry_entry() {
        let entry = RegistryEntry {
            name: "hello".to_string(),
            description: "Hello world".to_string(),
            category: Some("getting-started".to_string()),
            repo: "https://example.com/templates".to_string(),
            min_ironflow_version: Some(Version::new(0, 5, 0)),
            authors: vec!["Alice".to_string()],
            latest_version: Some(Version::new(1, 2, 0)),
        };

        let api_entry = TemplateListEntry::from(entry);
        assert_eq!(api_entry.name, "hello");
        assert_eq!(api_entry.category, Some("getting-started".to_string()));
        assert_eq!(api_entry.min_ironflow_version, Some("0.5.0".to_string()));
        assert_eq!(api_entry.authors, vec!["Alice"]);
        assert_eq!(api_entry.latest_version, Some("1.2.0".to_string()));
    }

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    async fn listing_error(url: String) -> Result<ApiError, String> {
        match fetch_registry_listing(url).await {
            Ok(_) => Err("registry listing unexpectedly succeeded".to_string()),
            Err(err) => Ok(err),
        }
    }

    async fn error_body(
        response: Response,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    #[tokio::test]
    async fn missing_registry_answers_404_registry_not_found() -> TestResult {
        let tmp = tempfile::TempDir::new()?;
        let url = format!("file://{}", tmp.path().join("absent").display());

        let response = listing_error(url).await?.into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            error_body(response).await?["error"]["code"],
            "REGISTRY_NOT_FOUND"
        );
        Ok(())
    }

    #[tokio::test]
    async fn unreachable_registry_answers_502_registry_unreachable() -> TestResult {
        // Port 1 is never listening on the loopback interface.
        let url = "http://127.0.0.1:1/ironflow/registry".to_string();

        let response = listing_error(url).await?.into_response();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            error_body(response).await?["error"]["code"],
            "REGISTRY_UNREACHABLE"
        );
        Ok(())
    }

    #[tokio::test]
    async fn registry_error_body_does_not_echo_the_url() -> TestResult {
        let url = "http://user:s3cret@127.0.0.1:1/registry".to_string();

        let response = listing_error(url).await?.into_response();

        let body = error_body(response).await?.to_string();
        assert!(!body.contains("s3cret"), "leaked: {body}");
        Ok(())
    }

    #[test]
    fn template_list_entry_serializes_without_optional_fields() {
        let entry = TemplateListEntry {
            name: "hello".to_string(),
            description: "Hello".to_string(),
            category: None,
            repo: "https://example.com".to_string(),
            min_ironflow_version: None,
            authors: vec![],
            latest_version: None,
        };

        let json = serde_json::to_value(&entry).expect("test serialization");
        assert!(json.get("category").is_none());
        assert!(json.get("min_ironflow_version").is_none());
        assert!(json.get("authors").is_none());
        assert!(json.get("latest_version").is_none());
    }
}
