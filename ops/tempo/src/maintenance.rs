//! Partition and live store downscale operations.
//!
//! These operations manage graceful scaling of Tempo's partition and
//! live store components, allowing safe downscale preparation and cancellation.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::TempoClient;
use crate::error::{check_response, parse_json_body, send_request};

/// Prepare a partition for downscaling.
///
/// Calls `POST /live-store/prepare-partition-downscale`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, maintenance::PreparePartitionDownscale};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = PreparePartitionDownscale::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct PreparePartitionDownscale {
    client: TempoClient,
}

impl PreparePartitionDownscale {
    /// Create a new partition downscale preparation operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, maintenance::PreparePartitionDownscale};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = PreparePartitionDownscale::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for PreparePartitionDownscale {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "prepare_partition_downscale"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(
            self.client.post("/live-store/prepare-partition-downscale"),
            "prepare partition downscale",
        )
        .await?;
        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        parse_json_body(&body)
    }
}

/// Cancel a partition downscale.
///
/// Calls `DELETE /live-store/prepare-partition-downscale`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, maintenance::CancelPartitionDownscale};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = CancelPartitionDownscale::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CancelPartitionDownscale {
    client: TempoClient,
}

impl CancelPartitionDownscale {
    /// Create a new partition downscale cancellation operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, maintenance::CancelPartitionDownscale};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = CancelPartitionDownscale::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for CancelPartitionDownscale {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "cancel_partition_downscale"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(
            self.client
                .delete("/live-store/prepare-partition-downscale"),
            "cancel partition downscale",
        )
        .await?;
        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        parse_json_body(&body)
    }
}

/// Get the status of a partition downscale.
///
/// Calls `GET /live-store/prepare-partition-downscale`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, maintenance::GetPartitionDownscaleStatus};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetPartitionDownscaleStatus::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetPartitionDownscaleStatus {
    client: TempoClient,
}

impl GetPartitionDownscaleStatus {
    /// Create a new partition downscale status query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, maintenance::GetPartitionDownscaleStatus};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetPartitionDownscaleStatus::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetPartitionDownscaleStatus {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_partition_downscale_status"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(
            self.client.get("/live-store/prepare-partition-downscale"),
            "get partition downscale status",
        )
        .await?;
        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "not_in_progress"}));
        }
        parse_json_body(&body)
    }
}

/// Prepare the live store for downscaling.
///
/// Calls `POST /live-store/prepare-downscale`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, maintenance::PrepareLiveStoreDownscale};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = PrepareLiveStoreDownscale::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct PrepareLiveStoreDownscale {
    client: TempoClient,
}

impl PrepareLiveStoreDownscale {
    /// Create a new live store downscale preparation operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, maintenance::PrepareLiveStoreDownscale};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = PrepareLiveStoreDownscale::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for PrepareLiveStoreDownscale {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "prepare_live_store_downscale"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(
            self.client.post("/live-store/prepare-downscale"),
            "prepare live store downscale",
        )
        .await?;
        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        parse_json_body(&body)
    }
}

/// Cancel a live store downscale.
///
/// Calls `DELETE /live-store/prepare-downscale`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, maintenance::CancelLiveStoreDownscale};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = CancelLiveStoreDownscale::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CancelLiveStoreDownscale {
    client: TempoClient,
}

impl CancelLiveStoreDownscale {
    /// Create a new live store downscale cancellation operation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, maintenance::CancelLiveStoreDownscale};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = CancelLiveStoreDownscale::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for CancelLiveStoreDownscale {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "cancel_live_store_downscale"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(
            self.client.delete("/live-store/prepare-downscale"),
            "cancel live store downscale",
        )
        .await?;
        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "success"}));
        }
        parse_json_body(&body)
    }
}

/// Get the status of a live store downscale.
///
/// Calls `GET /live-store/prepare-downscale`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, maintenance::GetLiveStoreDownscaleStatus};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetLiveStoreDownscaleStatus::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetLiveStoreDownscaleStatus {
    client: TempoClient,
}

impl GetLiveStoreDownscaleStatus {
    /// Create a new live store downscale status query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, maintenance::GetLiveStoreDownscaleStatus};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetLiveStoreDownscaleStatus::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetLiveStoreDownscaleStatus {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_live_store_downscale_status"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(
            self.client.get("/live-store/prepare-downscale"),
            "get live store downscale status",
        )
        .await?;
        let body = check_response(response).await?;
        if body.is_empty() {
            return Ok(json!({"status": "not_in_progress"}));
        }
        parse_json_body(&body)
    }
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;
    use reqwest::Client;

    use super::*;
    use crate::TempoClient;

    #[test]
    fn all_maintenance_ops_return_kind_tempo() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());

        assert_eq!(
            PreparePartitionDownscale::new(tempo.clone()).kind(),
            "tempo"
        );
        assert_eq!(CancelPartitionDownscale::new(tempo.clone()).kind(), "tempo");
        assert_eq!(
            GetPartitionDownscaleStatus::new(tempo.clone()).kind(),
            "tempo"
        );
        assert_eq!(
            PrepareLiveStoreDownscale::new(tempo.clone()).kind(),
            "tempo"
        );
        assert_eq!(CancelLiveStoreDownscale::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetLiveStoreDownscaleStatus::new(tempo).kind(), "tempo");
    }
}
