//! [`SlackClient`] built from an [`OperationContext`]'s secret store.

use std::fmt;
use std::sync::Arc;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use slack_morphism::hyper_tokio::SlackClientHyperConnector;
use slack_morphism::prelude::{
    SlackApiToken, SlackApiTokenValue, SlackClient as MorphismClient,
    SlackClientHyperHttpsConnector, SlackClientSession,
};

/// A Slack API client that resolves credentials from the workflow's secret store.
///
/// Wraps a [`slack_morphism::SlackClient`] with a bot token. All operation
/// structs in this crate accept a `&SlackClient` to make API calls.
///
/// # Construction
///
/// - [`from_context`](SlackClient::from_context) reads `slack_bot_token` from
///   the secret store.
/// - [`new`](SlackClient::new) accepts an explicit token string.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_slack::SlackClient;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let slack = SlackClient::from_context(&ctx).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct SlackClient {
    inner: Arc<MorphismClient<SlackClientHyperHttpsConnector>>,
    token: SlackApiToken,
}

impl SlackClient {
    /// Build a client from an [`OperationContext`].
    ///
    /// Reads the `slack_bot_token` secret for authentication.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the token is missing, or
    /// [`OperationError::External`] if the HTTP connector cannot be built.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_slack::SlackClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let slack = SlackClient::from_context(&ctx).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let secret =
            ctx.secrets()
                .get("slack_bot_token")
                .await?
                .ok_or_else(|| OperationError::Secret {
                    message: "slack_bot_token secret not found".to_string(),
                })?;
        Self::new(&secret.value)
    }

    /// Build a client with an explicit bot token.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the token is empty, or
    /// [`OperationError::External`] if the HTTP connector cannot be built.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_slack::SlackClient;
    ///
    /// # fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let slack = SlackClient::new("xoxb-xxxx")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(bot_token: &str) -> Result<Self, OperationError> {
        let trimmed = bot_token.trim();
        if trimmed.is_empty() {
            return Err(OperationError::Secret {
                message: "slack bot token must not be empty".to_string(),
            });
        }

        let connector = SlackClientHyperConnector::new().map_err(|e| OperationError::External {
            origin: "slack".to_string(),
            message: format!("failed to build HTTPS connector: {e}"),
        })?;

        let client = MorphismClient::new(connector);
        let token = SlackApiToken::new(SlackApiTokenValue::new(trimmed.to_string()));

        Ok(Self {
            inner: Arc::new(client),
            token,
        })
    }

    /// Open a session for making API calls.
    ///
    /// The session borrows from this client and its token, so it cannot
    /// outlive the `SlackClient`.
    pub fn session(&self) -> SlackClientSession<'_, SlackClientHyperHttpsConnector> {
        self.inner.open_session(&self.token)
    }
}

impl fmt::Debug for SlackClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlackClient")
            .field("client", &"[SlackClient]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    use super::*;

    fn install_crypto_provider() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    #[tokio::test]
    async fn from_context_fails_when_token_missing() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = SlackClient::from_context(&ctx).await.unwrap_err();
        assert!(err.to_string().contains("slack_bot_token"));
    }

    #[test]
    fn new_rejects_empty_token() {
        let err = SlackClient::new("").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn new_rejects_whitespace_token() {
        let err = SlackClient::new("   ").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn debug_does_not_leak_token() {
        install_crypto_provider();
        let client = SlackClient::new("xoxb-super-secret-token").unwrap();
        let debug = format!("{client:?}");
        assert!(
            !debug.contains("xoxb-super-secret-token"),
            "token leaked in Debug output: {debug}"
        );
    }
}
