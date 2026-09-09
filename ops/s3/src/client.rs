//! [`S3Client`] built from an [`OperationContext`]'s secret store.

use std::fmt;

use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Builder;
use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;

/// An AWS S3 client that resolves credentials from the workflow's secret store.
///
/// Wraps [`aws_sdk_s3::Client`] and provides convenience constructors for
/// building the client from an [`OperationContext`] or explicit parameters.
///
/// # Construction
///
/// - [`from_context`](S3Client::from_context) reads `aws_access_key_id`,
///   `aws_secret_access_key`, and optionally `aws_region` and `aws_endpoint_url`
///   from the secret store.
/// - [`new`](S3Client::new) wraps an existing [`aws_sdk_s3::Client`] directly.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_s3::S3Client;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let s3 = S3Client::from_context(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct S3Client {
    inner: Client,
}

impl S3Client {
    /// Build a client from an [`OperationContext`].
    ///
    /// Reads the following secrets from the workflow's secret store:
    /// - `aws_access_key_id` (required)
    /// - `aws_secret_access_key` (required)
    /// - `aws_region` (optional, defaults to `eu-west-1`)
    /// - `aws_endpoint_url` (optional, for MinIO/LocalStack)
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if a required secret is missing.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_s3::S3Client;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let s3 = S3Client::from_context(&ctx).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let access_key_id = ctx
            .secrets()
            .get("aws_access_key_id")
            .await?
            .ok_or_else(|| OperationError::Secret {
                message: "aws_access_key_id secret not found".to_string(),
            })?;

        let secret_access_key = ctx
            .secrets()
            .get("aws_secret_access_key")
            .await?
            .ok_or_else(|| OperationError::Secret {
                message: "aws_secret_access_key secret not found".to_string(),
            })?;

        let region = match ctx.secrets().get("aws_region").await? {
            Some(s) => s.value.clone(),
            None => "eu-west-1".to_string(),
        };

        let endpoint_url = match ctx.secrets().get("aws_endpoint_url").await? {
            Some(s) => Some(s.value.clone()),
            None => None,
        };

        Self::from_credentials(
            &access_key_id.value,
            &secret_access_key.value,
            &region,
            endpoint_url.as_deref(),
        )
    }

    /// Build a client with explicit credentials.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the access key ID or secret access
    /// key is empty.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_s3::S3Client;
    ///
    /// # fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let s3 = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_credentials(
        access_key_id: &str,
        secret_access_key: &str,
        region: &str,
        endpoint_url: Option<&str>,
    ) -> Result<Self, OperationError> {
        if access_key_id.trim().is_empty() {
            return Err(OperationError::Secret {
                message: "aws_access_key_id must not be empty".to_string(),
            });
        }
        if secret_access_key.trim().is_empty() {
            return Err(OperationError::Secret {
                message: "aws_secret_access_key must not be empty".to_string(),
            });
        }

        let creds = Credentials::new(
            access_key_id,
            secret_access_key,
            None,
            None,
            "ironflow-ops-s3",
        );

        let mut config = Builder::new()
            .region(Region::new(region.to_string()))
            .credentials_provider(creds)
            .force_path_style(true);

        if let Some(url) = endpoint_url {
            config = config.endpoint_url(url);
        }

        let client = Client::from_conf(config.build());
        Ok(Self { inner: client })
    }

    /// Wrap an existing [`aws_sdk_s3::Client`] directly.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_s3::S3Client;
    /// use aws_sdk_s3::Client;
    ///
    /// # async fn example() {
    /// let config = aws_sdk_s3::Config::builder().build();
    /// let raw = Client::from_conf(config);
    /// let s3 = S3Client::new(raw);
    /// # }
    /// ```
    pub fn new(client: Client) -> Self {
        Self { inner: client }
    }

    /// The underlying [`aws_sdk_s3::Client`].
    ///
    /// Use this for SDK calls not covered by the operation structs in this
    /// crate, such as streaming operations.
    pub fn client(&self) -> &Client {
        &self.inner
    }
}

impl Clone for S3Client {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl fmt::Debug for S3Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Client")
            .field("client", &"[aws_sdk_s3::Client]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    use super::*;

    #[tokio::test]
    async fn from_context_fails_when_access_key_missing() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = S3Client::from_context(&ctx).await.unwrap_err();
        assert!(
            err.to_string().contains("aws_access_key_id"),
            "expected aws_access_key_id error, got: {err}"
        );
    }

    #[test]
    fn from_credentials_rejects_empty_access_key() {
        let err = S3Client::from_credentials("", "secret", "eu-west-1", None).unwrap_err();
        assert!(err.to_string().contains("aws_access_key_id"));
    }

    #[test]
    fn from_credentials_rejects_empty_secret_key() {
        let err = S3Client::from_credentials("AKID", "", "eu-west-1", None).unwrap_err();
        assert!(err.to_string().contains("aws_secret_access_key"));
    }

    #[test]
    fn from_credentials_builds_with_valid_params() {
        let client = S3Client::from_credentials("AKID", "SECRET", "eu-west-1", None);
        assert!(client.is_ok());
    }

    #[test]
    fn from_credentials_accepts_custom_endpoint() {
        let client = S3Client::from_credentials(
            "AKID",
            "SECRET",
            "us-east-1",
            Some("http://localhost:9000"),
        );
        assert!(client.is_ok());
    }

    #[test]
    fn debug_does_not_leak_credentials() {
        let client = S3Client::from_credentials(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "eu-west-1",
            None,
        )
        .unwrap();
        let debug = format!("{client:?}");
        assert!(!debug.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!debug.contains("wJalrXUtnFEMI"));
        assert!(!debug.contains("EXAMPLEKEY"));
        assert!(debug.contains("S3Client"));
    }
}
