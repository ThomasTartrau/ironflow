//! Auth-related methods for [`IronflowClient`].

use crate::client::IronflowClient;
use crate::error::Error;

impl IronflowClient {
    /// Change the current user's password.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the old password is incorrect.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// client.change_password("old_pass", "new_pass").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn change_password(
        &self,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), Error> {
        let body = serde_json::json!({
            "old_password": old_password,
            "new_password": new_password,
        });
        self.send_no_content(self.patch("/api/v1/auth/password").json(&body))
            .await
    }
}
