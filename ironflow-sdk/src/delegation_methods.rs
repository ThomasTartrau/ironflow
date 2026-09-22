//! Approval-delegation methods for [`IronflowClient`].

use uuid::Uuid;

use ironflow_types::ApiResponse;

use crate::client::IronflowClient;
use crate::error::Error;
use crate::types;

impl IronflowClient {
    /// List the active approval delegations visible to the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let delegations = client.list_approval_delegations().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_approval_delegations(
        &self,
    ) -> Result<ApiResponse<Vec<types::ApprovalDelegationResponse>>, Error> {
        self.send_envelope(self.get("/api/v1/approval-delegations"))
            .await
    }

    /// Delegate the caller's approval power to another user.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use chrono::{TimeDelta, Utc};
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::types::CreateApprovalDelegationRequest;
    /// use uuid::Uuid;
    ///
    /// # async fn example(bob: Uuid) -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let delegation = client.create_approval_delegation(&CreateApprovalDelegationRequest {
    ///     to_user_id: bob,
    ///     valid_from: None,
    ///     valid_until: Utc::now() + TimeDelta::days(7),
    ///     workflow_filter: Some("deploy-*".to_string()),
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_approval_delegation(
        &self,
        request: &types::CreateApprovalDelegationRequest,
    ) -> Result<ApiResponse<types::ApprovalDelegationResponse>, Error> {
        self.send_envelope(self.post("/api/v1/approval-delegations").json(request))
            .await
    }

    /// Revoke an approval delegation by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// client.delete_approval_delegation(Uuid::nil()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_approval_delegation(&self, id: Uuid) -> Result<(), Error> {
        self.send_no_content(self.delete(&format!("/api/v1/approval-delegations/{id}")))
            .await
    }
}
