//! Agent role methods for [`IronflowClient`]: the cost each named role
//! consumed over its period, and its cap.

use ironflow_types::ApiResponse;
use serde::Serialize;

use crate::client::IronflowClient;
use crate::error::Error;
use crate::provider_account_methods::segment;
use crate::types;

/// Page of `GET /api/v1/agent-roles`.
#[derive(Debug, Serialize)]
struct PageQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_page: Option<u32>,
}

/// Path segment of a role name. Dots are encoded too, so a name such as `..`
/// can never be resolved as a relative path by the URL parser.
fn role_segment(name: &str) -> String {
    segment(name).replace('.', "%2E")
}

impl IronflowClient {
    /// List the agent roles, ordered by name, with their cost and cap.
    ///
    /// `page` is 1-based (default 1); `per_page` defaults to 50, max 100.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the API key lacks the
    /// `stats_read` scope, or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let roles = client.list_agent_roles(None, None).await?;
    /// for role in &roles.data {
    ///     println!("{}: {} spent", role.name, role.spent_usd);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_agent_roles(
        &self,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<ApiResponse<Vec<types::AgentRoleResponse>>, Error> {
        let query = PageQuery { page, per_page };
        self.send_envelope(self.get("/api/v1/agent-roles").query(&query))
            .await
    }

    /// Get the cost spent, reserved and left of one agent role.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the name is invalid, no
    /// step of the role ever ran, or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let role = client.get_agent_role("reviewer").await?;
    /// println!("{:?} left", role.data.remaining_usd);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_agent_role(
        &self,
        name: &str,
    ) -> Result<ApiResponse<types::AgentRoleResponse>, Error> {
        self.send_envelope(self.get(&format!("/api/v1/agent-roles/{}", role_segment(name))))
            .await
    }

    /// Set or clear (`operator_cap_usd: None`) the operator cap of an agent
    /// role. Admin only. The cap enforced is the lower of the code and
    /// operator caps: an operator cap can only lower it.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the caller is not an
    /// admin, the cap is negative, the role does not exist, or the response
    /// cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::types::UpdateAgentRoleCostCapRequest;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let request = UpdateAgentRoleCostCapRequest {
    ///     operator_cap_usd: Some(5.0),
    /// };
    /// let role = client.set_agent_role_cost_cap("reviewer", &request).await?;
    /// println!("{:?} cap", role.data.cap_usd);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_agent_role_cost_cap(
        &self,
        name: &str,
        request: &types::UpdateAgentRoleCostCapRequest,
    ) -> Result<ApiResponse<types::AgentRoleResponse>, Error> {
        self.send_envelope(
            self.put(&format!(
                "/api/v1/agent-roles/{}/cost-cap",
                role_segment(name)
            ))
            .json(request),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::role_segment;

    #[test]
    fn role_segment_encodes_dots_and_separators() {
        assert_eq!(role_segment("code-reviewer"), "code-reviewer");
        assert_eq!(role_segment("reviewer.v2"), "reviewer%2Ev2");
        assert_eq!(role_segment(".."), "%2E%2E");
        assert_eq!(role_segment("a/b"), "a%2Fb");
    }
}
