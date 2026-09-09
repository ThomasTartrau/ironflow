//! Cluster topology and ring status operations.
//!
//! These operations query the state of Tempo's internal hash rings and
//! cluster membership.

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::{Value, json};

use crate::TempoClient;
use crate::error::{check_response, send_request};

/// Retrieve the cluster memberlist.
///
/// Calls `GET /memberlist`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, cluster::GetMemberlist};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetMemberlist::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetMemberlist {
    client: TempoClient,
}

impl GetMemberlist {
    /// Create a new memberlist query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, cluster::GetMemberlist};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetMemberlist::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetMemberlist {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_memberlist"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response = send_request(self.client.get("/memberlist"), "get memberlist").await?;
        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"memberlist": text.as_ref()}))
    }
}

/// Retrieve the distributor hash ring.
///
/// Calls `GET /distributor/ring`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, cluster::GetDistributorRing};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetDistributorRing::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetDistributorRing {
    client: TempoClient,
}

impl GetDistributorRing {
    /// Create a new distributor ring query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, cluster::GetDistributorRing};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetDistributorRing::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetDistributorRing {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_distributor_ring"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            send_request(self.client.get("/distributor/ring"), "get distributor ring").await?;
        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ring": text.as_ref()}))
    }
}

/// Retrieve the live store hash ring.
///
/// Calls `GET /live-store/ring`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, cluster::GetLiveStoreRing};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetLiveStoreRing::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetLiveStoreRing {
    client: TempoClient,
}

impl GetLiveStoreRing {
    /// Create a new live store ring query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, cluster::GetLiveStoreRing};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetLiveStoreRing::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetLiveStoreRing {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_live_store_ring"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            send_request(self.client.get("/live-store/ring"), "get live store ring").await?;
        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ring": text.as_ref()}))
    }
}

/// Retrieve the partition ring.
///
/// Calls `GET /partition-ring`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_tempo::{TempoClient, cluster::GetPartitionRing};
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let tempo = TempoClient::from_context(&ctx).await?;
/// let op = GetPartitionRing::new(tempo);
/// let result = op.execute(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GetPartitionRing {
    client: TempoClient,
}

impl GetPartitionRing {
    /// Create a new partition ring query.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_tempo::{TempoClient, cluster::GetPartitionRing};
    /// use reqwest::Client;
    ///
    /// let tempo = TempoClient::new("http://tempo:3200", Client::new());
    /// let op = GetPartitionRing::new(tempo);
    /// ```
    pub fn new(client: TempoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Operation for GetPartitionRing {
    fn kind(&self) -> &str {
        "tempo"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({"operation": "get_partition_ring"}))
    }

    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the request fails.
    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let response =
            send_request(self.client.get("/partition-ring"), "get partition ring").await?;
        let body = check_response(response).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(json!({"ring": text.as_ref()}))
    }
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;
    use reqwest::Client;

    use super::*;
    use crate::TempoClient;

    #[test]
    fn all_cluster_ops_return_kind_tempo() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());

        assert_eq!(GetMemberlist::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetDistributorRing::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetLiveStoreRing::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetPartitionRing::new(tempo).kind(), "tempo");
    }
}
