//! Provider Account methods for [`IronflowClient`]. Admin only.
//!
//! No response carries the account credential.

use ironflow_types::ApiResponse;
use serde_json::json;

use crate::client::IronflowClient;
use crate::error::Error;
use crate::types;

/// Percent-encode one path segment (an account UUID or name).
fn segment(raw: &str) -> String {
    raw.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

impl IronflowClient {
    /// List Provider Accounts with their latest usage windows.
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
    /// let accounts = client.list_provider_accounts().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_provider_accounts(
        &self,
    ) -> Result<ApiResponse<Vec<types::ProviderAccountResponse>>, Error> {
        self.send_envelope(self.get("/api/v1/provider-accounts?per_page=100"))
            .await
    }

    /// Add a Provider Account. The token is checked against the provider first.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the token is rejected (422)
    /// or the name is taken (409).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::types::CreateProviderAccountRequest;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let account = client.create_provider_account(&CreateProviderAccountRequest {
    ///     name: "perso".to_string(),
    ///     kind: "claude_subscription".to_string(),
    ///     token: "sk-ant-oat01-...".to_string(),
    ///     alert_threshold: None,
    ///     display_name: None,
    ///     enabled: None,
    ///     expires_at: None,
    ///     max_concurrency: None,
    ///     plan: None,
    ///     priority: None,
    ///     tags: None,
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_provider_account(
        &self,
        request: &types::CreateProviderAccountRequest,
    ) -> Result<ApiResponse<types::ProviderAccountResponse>, Error> {
        self.send_envelope(self.post("/api/v1/provider-accounts").json(request))
            .await
    }

    /// Get a Provider Account by UUID or name.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the account does not exist.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let account = client.get_provider_account("perso").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_provider_account(
        &self,
        account: &str,
    ) -> Result<ApiResponse<types::ProviderAccountResponse>, Error> {
        self.send_envelope(self.get(&format!("/api/v1/provider-accounts/{}", segment(account))))
            .await
    }

    /// Update a Provider Account by UUID or name.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the account does not exist
    /// or a new token is rejected.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::types::UpdateProviderAccountRequest;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let update = UpdateProviderAccountRequest { enabled: Some(false), ..Default::default() };
    /// client.update_provider_account("perso", &update).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn update_provider_account(
        &self,
        account: &str,
        request: &types::UpdateProviderAccountRequest,
    ) -> Result<ApiResponse<types::ProviderAccountResponse>, Error> {
        self.send_envelope(
            self.patch(&format!("/api/v1/provider-accounts/{}", segment(account)))
                .json(request),
        )
        .await
    }

    /// Remove the `max_concurrency` limit of a Provider Account.
    ///
    /// [`update_provider_account`](Self::update_provider_account) leaves an
    /// absent field unchanged, so clearing needs an explicit `null`.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the account does not exist.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// client.clear_provider_account_max_concurrency("perso").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn clear_provider_account_max_concurrency(
        &self,
        account: &str,
    ) -> Result<ApiResponse<types::ProviderAccountResponse>, Error> {
        self.send_envelope(
            self.patch(&format!("/api/v1/provider-accounts/{}", segment(account)))
                .json(&json!({ "max_concurrency": null })),
        )
        .await
    }

    /// Delete a Provider Account and its credential.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the account does not exist.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// client.delete_provider_account("perso").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_provider_account(&self, account: &str) -> Result<(), Error> {
        self.send_no_content(
            self.delete(&format!("/api/v1/provider-accounts/{}", segment(account))),
        )
        .await
    }

    /// Check the stored credential of a Provider Account against the provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the provider is unreachable.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let outcome = client.test_provider_account("perso").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn test_provider_account(
        &self,
        account: &str,
    ) -> Result<ApiResponse<types::ProviderAccountTestResponse>, Error> {
        self.send_envelope(self.post(&format!(
            "/api/v1/provider-accounts/{}/test",
            segment(account)
        )))
        .await
    }

    /// Current windows and 30-day history of a Provider Account.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the account does not exist.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let usage = client.provider_account_usage("perso").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn provider_account_usage(
        &self,
        account: &str,
    ) -> Result<ApiResponse<types::ProviderAccountUsageResponse>, Error> {
        self.send_envelope(self.get(&format!(
            "/api/v1/provider-accounts/{}/usage",
            segment(account)
        )))
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::segment;

    #[test]
    fn segment_keeps_slugs_and_encodes_the_rest() {
        assert_eq!(segment("perso-max"), "perso-max");
        assert_eq!(
            segment("0192f0c1-0000-7000-8000-000000000000"),
            "0192f0c1-0000-7000-8000-000000000000"
        );
        assert_eq!(segment("a/b c"), "a%2Fb%20c");
        assert_eq!(segment("../x"), "..%2Fx");
    }
}
