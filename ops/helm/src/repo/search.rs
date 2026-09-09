//! Search operations: search repo, search hub.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::run_helm_json;

/// Search a chart repository.
///
/// Wraps `helm search repo <keyword> --output json`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::repo::SearchRepo;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = SearchRepo::new(client, "nginx");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct SearchRepo {
    client: HelmClient,
    keyword: String,
}

impl SearchRepo {
    /// Create a new search-repo operation.
    pub fn new(client: HelmClient, keyword: impl Into<String>) -> Self {
        Self {
            client,
            keyword: keyword.into(),
        }
    }

    /// Execute and return the search results as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        run_helm_json(&self.client, &["search", "repo", &self.keyword]).await
    }
}

#[async_trait]
impl Operation for SearchRepo {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run(ctx).await
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "search repo",
            "keyword": self.keyword,
        }))
    }
}

impl TypedOperation for SearchRepo {
    type Output = Value;
}

/// Search the Artifact Hub.
///
/// Wraps `helm search hub <keyword> --output json`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::repo::SearchHub;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = SearchHub::new(client, "nginx");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct SearchHub {
    client: HelmClient,
    keyword: String,
}

impl SearchHub {
    /// Create a new search-hub operation.
    pub fn new(client: HelmClient, keyword: impl Into<String>) -> Self {
        Self {
            client,
            keyword: keyword.into(),
        }
    }

    /// Execute and return the search results as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the command fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        run_helm_json(&self.client, &["search", "hub", &self.keyword]).await
    }
}

#[async_trait]
impl Operation for SearchHub {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        self.run(ctx).await
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "search hub",
            "keyword": self.keyword,
        }))
    }
}

impl TypedOperation for SearchHub {
    type Output = Value;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;

    #[test]
    fn search_repo_kind() {
        assert_eq!(
            SearchRepo::new(HelmClient::default(), "nginx").kind(),
            "helm"
        );
    }

    #[test]
    fn search_hub_kind() {
        assert_eq!(
            SearchHub::new(HelmClient::default(), "nginx").kind(),
            "helm"
        );
    }
}
