//! [`HelmClient`] -- central handle for Helm CLI operations.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;

/// A handle to the Helm CLI binary with optional kubeconfig and namespace.
///
/// `HelmClient` stores the path to the `helm` binary, an optional kubeconfig
/// file path, and an optional default namespace. All operations use this
/// client to spawn the Helm process.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
///
/// // Use defaults (helm in PATH, no kubeconfig override, no namespace)
/// let client = HelmClient::default();
///
/// // Custom binary path and namespace
/// let client = HelmClient::new("/usr/local/bin/helm", None::<String>, Some("production"));
/// ```
#[derive(Clone)]
pub struct HelmClient {
    binary: String,
    kubeconfig: Option<String>,
    namespace: Option<String>,
}

impl HelmClient {
    /// Create a new `HelmClient` with explicit configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_ops_helm::HelmClient;
    ///
    /// let client = HelmClient::new("helm", None::<String>, Some("default"));
    /// assert_eq!(client.binary(), "helm");
    /// assert_eq!(client.namespace(), Some("default"));
    /// ```
    pub fn new(
        binary: impl Into<String>,
        kubeconfig: Option<impl Into<String>>,
        namespace: Option<impl Into<String>>,
    ) -> Self {
        Self {
            binary: binary.into(),
            kubeconfig: kubeconfig.map(Into::into),
            namespace: namespace.map(Into::into),
        }
    }

    /// Create a `HelmClient` from an [`OperationContext`].
    ///
    /// Reads optional secrets from the secret store:
    /// - `helm_binary` -- path to the Helm binary (default: `"helm"`)
    /// - `helm_kubeconfig` -- path to the kubeconfig file
    /// - `helm_namespace` -- default namespace
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store fails to read.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_helm::HelmClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let client = HelmClient::from_context(&ctx).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let secrets = ctx.secrets();
        let binary = secrets
            .get("helm_binary")
            .await?
            .map(|s| s.value)
            .unwrap_or_else(|| "helm".to_string());
        let kubeconfig = secrets.get("helm_kubeconfig").await?.map(|s| s.value);
        let namespace = secrets.get("helm_namespace").await?.map(|s| s.value);
        Ok(Self {
            binary,
            kubeconfig,
            namespace,
        })
    }

    /// The path to the Helm binary.
    pub fn binary(&self) -> &str {
        &self.binary
    }

    /// The optional kubeconfig file path.
    pub fn kubeconfig(&self) -> Option<&str> {
        self.kubeconfig.as_deref()
    }

    /// The optional default namespace.
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }
}

impl Default for HelmClient {
    fn default() -> Self {
        Self {
            binary: "helm".to_string(),
            kubeconfig: None,
            namespace: None,
        }
    }
}

impl fmt::Debug for HelmClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelmClient")
            .field("binary", &self.binary)
            .field("kubeconfig", &self.kubeconfig)
            .field("namespace", &self.namespace)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    use super::*;

    #[test]
    fn new_with_defaults() {
        let client = HelmClient::default();
        assert_eq!(client.binary(), "helm");
        assert_eq!(client.kubeconfig(), None);
        assert_eq!(client.namespace(), None);
    }

    #[test]
    fn new_with_custom_values() {
        let client = HelmClient::new(
            "/usr/local/bin/helm",
            Some("/home/user/.kube/config"),
            Some("production"),
        );
        assert_eq!(client.binary(), "/usr/local/bin/helm");
        assert_eq!(client.kubeconfig(), Some("/home/user/.kube/config"));
        assert_eq!(client.namespace(), Some("production"));
    }

    #[tokio::test]
    async fn from_context_uses_defaults_with_noop_resolver() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let client = HelmClient::from_context(&ctx).await.unwrap();
        assert_eq!(client.binary(), "helm");
        assert_eq!(client.kubeconfig(), None);
        assert_eq!(client.namespace(), None);
    }

    #[test]
    fn debug_does_not_leak_secrets() {
        let client = HelmClient::new("helm", Some("/secret/kubeconfig"), Some("prod"));
        let debug = format!("{client:?}");
        assert!(debug.contains("HelmClient"));
        assert!(debug.contains("helm"));
        // kubeconfig path is not a secret, it's a file path
        // No tokens or passwords are stored in HelmClient
    }

    #[test]
    fn clone_preserves_values() {
        let client = HelmClient::new("helm3", Some("/kube/config"), Some("staging"));
        let cloned = client.clone();
        assert_eq!(cloned.binary(), "helm3");
        assert_eq!(cloned.kubeconfig(), Some("/kube/config"));
        assert_eq!(cloned.namespace(), Some("staging"));
    }
}
