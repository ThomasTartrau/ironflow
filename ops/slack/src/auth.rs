//! Auth operations.
//!
//! Wraps the Slack [`auth.*`](https://api.slack.com/methods?filter=auth) methods.

use slack_morphism::api::SlackApiAuthTestResponse;

use crate::macros::slack_op;

slack_op! {
    /// Test authentication and get information about the bot token.
    ///
    /// Wraps [`auth.test`](https://api.slack.com/methods/auth.test).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_slack::SlackClient;
    /// use ironflow_ops_slack::auth::AuthTest;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let slack = SlackClient::new("xoxb-token")?;
    /// let op = AuthTest::new(&slack);
    /// let response = op.run().await?;
    /// # Ok(())
    /// # }
    /// ```
    AuthTest => auth_test() -> SlackApiAuthTestResponse
}
