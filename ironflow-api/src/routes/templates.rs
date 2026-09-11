//! `GET /api/v1/templates/registry` -- List templates from the configured registry.

use axum::response::IntoResponse;
use ironflow_auth::extractor::Authenticated;
use serde::Serialize;
use tokio::task::spawn_blocking;

use ironflow_engine::VERSION as ENGINE_VERSION;
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
            (status = 502, description = "Failed to fetch registry")
        )
    )
)]
pub async fn list_registry_templates(_auth: Authenticated) -> Result<impl IntoResponse, ApiError> {
    let registry_url = resolve_registry_url(None);

    let index = spawn_blocking(move || fetch_registry_index(&registry_url))
        .await
        .map_err(|e| ApiError::Internal(format!("task join error: {e}")))?
        .map_err(|e| ApiError::BadGateway(format!("registry fetch failed: {e}")))?;

    let templates: Vec<TemplateListEntry> = index.templates.into_iter().map(Into::into).collect();
    let response = TemplateRegistryResponse {
        templates,
        ironflow_version: ENGINE_VERSION.to_string(),
    };
    Ok(ok(response))
}

#[cfg(test)]
mod tests {
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
