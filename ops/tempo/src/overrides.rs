//! Runtime override operations.
//!
//! These operations manage Tempo's per-tenant runtime overrides,
//! allowing dynamic configuration without restarting the service.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::TempoClient;
use crate::error::{check_response, parse_json_body, send_request};

/// Retrieve the current runtime overrides.
///
/// Calls `GET /api/overrides`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, overrides::GetOverrides};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetOverrides::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetOverrides {
    client: TempoClient,
}

impl GetOverrides {
    /// Create a new overrides query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, overrides::GetOverrides};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetOverrides::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetOverrides {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_overrides"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(self.client.get("/api/overrides"), "get overrides").await?;
        let body = check_response(response).await?;
        parse_json_body(&body)
    }
}

/// Create or replace runtime overrides.
///
/// Calls `POST /api/overrides` with the given JSON body.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, overrides::CreateOverrides};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let overrides = json!({"ingestion_rate_limit_bytes": 15_000_000});
/// let op = CreateOverrides::new(tempo, overrides);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CreateOverrides {
    client: TempoClient,
    body: Value,
}

impl CreateOverrides {
    /// Create a new override creation operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, overrides::CreateOverrides};
    /// use serde_json::json;
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let overrides = json!({"ingestion_rate_limit_bytes": 15_000_000});
    /// let op = CreateOverrides::new(tempo, overrides);
    /// ```
    pub fn new(client: TempoClient, body: Value) -> Self {
        Self { client, body }
    }
}

#[async_trait]
impl Operation for CreateOverrides {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "create_overrides",
            "body": self.body,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(
            self.client.post("/api/overrides").json(&self.body),
            "create overrides",
        )
        .await?;
        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        parse_json_body(&body)
    }
}

/// Update runtime overrides.
///
/// Calls `PATCH /api/overrides` with the given JSON body.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, overrides::UpdateOverrides};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use serde_json::json;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let overrides = json!({"ingestion_rate_limit_bytes": 20_000_000});
/// let op = UpdateOverrides::new(tempo, overrides);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct UpdateOverrides {
    client: TempoClient,
    body: Value,
}

impl UpdateOverrides {
    /// Create a new override update operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, overrides::UpdateOverrides};
    /// use serde_json::json;
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let overrides = json!({"ingestion_rate_limit_bytes": 20_000_000});
    /// let op = UpdateOverrides::new(tempo, overrides);
    /// ```
    pub fn new(client: TempoClient, body: Value) -> Self {
        Self { client, body }
    }
}

#[async_trait]
impl Operation for UpdateOverrides {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "operation": "update_overrides",
            "body": self.body,
        }))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(
            self.client.patch("/api/overrides").json(&self.body),
            "update overrides",
        )
        .await?;
        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        parse_json_body(&body)
    }
}

/// Delete runtime overrides.
///
/// Calls `DELETE /api/overrides`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, overrides::DeleteOverrides};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = DeleteOverrides::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DeleteOverrides {
    client: TempoClient,
}

impl DeleteOverrides {
    /// Create a new override deletion operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, overrides::DeleteOverrides};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = DeleteOverrides::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for DeleteOverrides {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "delete_overrides"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            send_request(self.client.delete("/api/overrides"), "delete overrides").await?;
        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        parse_json_body(&body)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
    use reqwest::Client;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::TempoClient;

    #[test]
    fn all_override_ops_return_kind_tempo() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());

        assert_eq!(GetOverrides::new(tempo.clone()).kind(), "tempo");
        assert_eq!(
            CreateOverrides::new(tempo.clone(), json!({})).kind(),
            "tempo"
        );
        assert_eq!(
            UpdateOverrides::new(tempo.clone(), json!({})).kind(),
            "tempo"
        );
        assert_eq!(DeleteOverrides::new(tempo).kind(), "tempo");
    }

    #[tokio::test]
    async fn overrides_crud_lifecycle() {
        let server = MockServer::start().await;
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));

        Mock::given(method("GET"))
            .and(path("/api/overrides"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"overrides":{}}"#))
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());

        let result = GetOverrides::new(tempo.clone())
            .execute(&ctx)
            .await
            .unwrap();
        assert_eq!(result["overrides"], json!({}));

        let override_body = json!({"ingestion_rate_limit_bytes": 15_000_000});
        Mock::given(method("POST"))
            .and(path("/api/overrides"))
            .and(body_json(&override_body))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"status":"created"}"#))
            .mount(&server)
            .await;

        let result = CreateOverrides::new(tempo.clone(), override_body)
            .execute(&ctx)
            .await
            .unwrap();
        assert_eq!(result["status"], "created");

        Mock::given(method("DELETE"))
            .and(path("/api/overrides"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let result = DeleteOverrides::new(tempo).execute(&ctx).await.unwrap();
        assert_eq!(result["status"], "success");
    }

    #[tokio::test]
    async fn update_overrides_uses_patch_method() {
        let server = MockServer::start().await;
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));

        let update_body = json!({"ingestion_rate_limit_bytes": 20_000_000});
        Mock::given(method("PATCH"))
            .and(path("/api/overrides"))
            .and(body_json(&update_body))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"status":"updated"}"#))
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let result = UpdateOverrides::new(tempo, update_body)
            .execute(&ctx)
            .await
            .unwrap();
        assert_eq!(result["status"], "updated");
    }
}
