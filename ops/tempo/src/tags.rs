//! Tag discovery operations.
//!
//! These operations query the Tempo tag and tag value endpoints, useful for
//! building dynamic queries and understanding the shape of ingested trace data.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::TempoClient;
use crate::error::{check_response, parse_json_body, send_request, validate_path_segment};

/// Retrieve the list of known tag names.
///
/// Calls `GET /api/search/tags`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, tags::GetSearchTags};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetSearchTags::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetSearchTags {
    client: TempoClient,
}

impl GetSearchTags {
    /// Create a new tag listing operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, tags::GetSearchTags};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetSearchTags::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetSearchTags {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_search_tags"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(self.client.get("/api/search/tags"), "get search tags").await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

/// Retrieve the list of known tag names (v2 endpoint with scoped tags).
///
/// Calls `GET /api/v2/search/tags`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, tags::GetSearchTagsV2};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetSearchTagsV2::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetSearchTagsV2 {
    client: TempoClient,
}

impl GetSearchTagsV2 {
    /// Create a new v2 tag listing operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, tags::GetSearchTagsV2};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetSearchTagsV2::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetSearchTagsV2 {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_search_tags_v2"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            send_request(self.client.get("/api/v2/search/tags"), "get search tags v2").await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

/// Retrieve the values for a specific tag.
///
/// Calls `GET /api/search/tag/{tag_name}/values`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, tags::GetSearchTagValues};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetSearchTagValues::new(tempo, "service.name");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetSearchTagValues {
    client: TempoClient,
    tag_name: String,
}

impl GetSearchTagValues {
    /// Create a new tag values query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, tags::GetSearchTagValues};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetSearchTagValues::new(tempo, "service.name");
    /// ```
    pub fn new(client: TempoClient, tag_name: &str) -> Self {
        Self {
            client,
            tag_name: tag_name.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for GetSearchTagValues {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_search_tag_values",
            "tag_name": self.tag_name,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the tag name is invalid, or
    /// [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.tag_name, "tag_name")?;

        let response = send_request(
            self.client
                .get(&format!("/api/search/tag/{}/values", self.tag_name)),
            "get search tag values",
        )
        .await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

/// Retrieve the values for a specific tag (v2 endpoint).
///
/// Calls `GET /api/v2/search/tag/{tag_name}/values`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, tags::GetSearchTagValuesV2};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetSearchTagValuesV2::new(tempo, "service.name");
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetSearchTagValuesV2 {
    client: TempoClient,
    tag_name: String,
}

impl GetSearchTagValuesV2 {
    /// Create a new v2 tag values query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, tags::GetSearchTagValuesV2};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetSearchTagValuesV2::new(tempo, "service.name");
    /// ```
    pub fn new(client: TempoClient, tag_name: &str) -> Self {
        Self {
            client,
            tag_name: tag_name.to_owned(),
        }
    }
}

#[async_trait]
impl Operation for GetSearchTagValuesV2 {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "get_search_tag_values_v2",
            "tag_name": self.tag_name,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the tag name is invalid, or
    /// [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        validate_path_segment(&self.tag_name, "tag_name")?;

        let response = send_request(
            self.client
                .get(&format!("/api/v2/search/tag/{}/values", self.tag_name)),
            "get search tag values v2",
        )
        .await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
    use reqwest::Client;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::TempoClient;

    #[test]
    fn all_tag_ops_return_kind_tempo() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());

        assert_eq!(GetSearchTags::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetSearchTagsV2::new(tempo.clone()).kind(), "tempo");
        assert_eq!(
            GetSearchTagValues::new(tempo.clone(), "tag").kind(),
            "tempo"
        );
        assert_eq!(GetSearchTagValuesV2::new(tempo, "tag").kind(), "tempo");
    }

    #[tokio::test]
    async fn get_search_tag_values_hits_correct_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/search/tag/service.name/values"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"tagValues":["svc-a","svc-b"]}"#),
            )
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let result = GetSearchTagValues::new(tempo, "service.name")
            .execute(&ctx)
            .await
            .unwrap();
        assert_eq!(result["tagValues"][0], "svc-a");
    }

    #[tokio::test]
    async fn get_search_tag_values_v2_hits_correct_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/search/tag/http.method/values"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"tagValues":["GET","POST"]}"#),
            )
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let result = GetSearchTagValuesV2::new(tempo, "http.method")
            .execute(&ctx)
            .await
            .unwrap();
        assert_eq!(result["tagValues"][1], "POST");
    }

    #[tokio::test]
    async fn get_search_tag_values_rejects_invalid_tag_name() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = GetSearchTagValues::new(tempo, "../admin")
            .execute(&ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(err, OperationError::External { .. }),
            "expected External error for invalid tag name, got: {err}"
        );
    }

    #[tokio::test]
    async fn get_search_tag_values_v2_rejects_invalid_tag_name() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = GetSearchTagValuesV2::new(tempo, "a/b")
            .execute(&ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(err, OperationError::External { .. }),
            "expected External error for invalid tag name, got: {err}"
        );
    }
}
