//! [`KubeClient`] built from an [`OperationContext`]'s secret store or direct configuration.

use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use kube::Client;
use kube::Config;
use kube::config::{KubeConfigOptions, Kubeconfig};

use crate::operation::KubeOp;
use crate::verb::Verb;

/// A Kubernetes client that resolves credentials from the workflow's secret store.
///
/// Wraps [`kube::Client`] and provides convenience methods to construct
/// [`KubeOp`] instances for tracked workflow steps.
///
/// # Construction
///
/// Three construction paths are available:
///
/// - [`from_context`](KubeClient::from_context) reads a `kubeconfig` secret
///   from the workflow's secret store; falls back to in-cluster detection if
///   the secret is absent.
/// - [`from_config`](KubeClient::from_config) accepts an explicit [`kube::Config`].
/// - [`new_in_cluster`](KubeClient::new_in_cluster) forces in-cluster auto-detection.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::KubeClient;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let kube = KubeClient::from_context(&ctx).await?;
/// # Ok(())
/// # }
/// ```
pub struct KubeClient {
    inner: Client,
}

impl KubeClient {
    /// Build a client from an [`OperationContext`].
    ///
    /// Reads the `kubeconfig` secret from the workflow's secret store. If
    /// the secret is not found, falls back to in-cluster configuration via
    /// [`Config::incluster`].
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the secret store itself fails,
    /// or [`OperationError::Http`] if neither the kubeconfig nor in-cluster
    /// configuration can produce a valid client.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::KubeClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let kube = KubeClient::from_context(&ctx).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let secret = ctx.secrets().get("kubeconfig").await?;

        match secret {
            Some(s) => {
                let kubeconfig =
                    Kubeconfig::from_yaml(&s.value).map_err(|e| OperationError::Http {
                        status: None,
                        message: format!("invalid kubeconfig: {e}"),
                    })?;

                let config =
                    Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
                        .await
                        .map_err(|e| OperationError::Http {
                            status: None,
                            message: format!("kubeconfig error: {e}"),
                        })?;

                Self::from_config(config).await
            }
            None => Self::new_in_cluster().await,
        }
    }

    /// Build a client from an explicit [`kube::Config`].
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the client cannot be built.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::KubeClient;
    /// use kube::Config;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let config = Config::infer().await.expect("kubeconfig");
    /// let kube = KubeClient::from_config(config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_config(config: Config) -> Result<Self, OperationError> {
        let inner = Client::try_from(config).map_err(|e| OperationError::Http {
            status: None,
            message: format!("kube client error: {e}"),
        })?;
        Ok(Self { inner })
    }

    /// Build a client using in-cluster auto-detection.
    ///
    /// Reads the service account token and CA bundle from the standard
    /// mount paths (`/var/run/secrets/kubernetes.io/serviceaccount/`).
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if in-cluster configuration is not
    /// available (e.g. running outside a pod).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::KubeClient;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let kube = KubeClient::new_in_cluster().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new_in_cluster() -> Result<Self, OperationError> {
        let config = Config::incluster().map_err(|e| OperationError::Http {
            status: None,
            message: format!("in-cluster config error: {e}"),
        })?;
        Self::from_config(config).await
    }

    /// The underlying [`kube::Client`].
    ///
    /// Use this for direct API calls that are not covered by [`KubeOp`],
    /// such as streaming operations (watch, exec, attach, port-forward).
    pub fn client(&self) -> &Client {
        &self.inner
    }

    /// Construct a [`kube::Api`] handle for resource `R` in the given namespace.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::KubeClient;
    /// use k8s_openapi::api::core::v1::Pod;
    /// use kube::Config;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let config = Config::infer().await.expect("kubeconfig");
    /// let kube = KubeClient::from_config(config).await?;
    /// let pods = kube.namespaced::<Pod>("default");
    /// # Ok(())
    /// # }
    /// ```
    pub fn namespaced<R>(&self, namespace: &str) -> kube::Api<R>
    where
        R: kube::Resource<Scope = k8s_openapi::NamespaceResourceScope>,
        <R as kube::Resource>::DynamicType: Default,
    {
        kube::Api::namespaced(self.inner.clone(), namespace)
    }

    /// Construct a [`kube::Api`] handle for a cluster-scoped resource `R`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::KubeClient;
    /// use k8s_openapi::api::core::v1::Namespace;
    /// use kube::Config;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let config = Config::infer().await.expect("kubeconfig");
    /// let kube = KubeClient::from_config(config).await?;
    /// let namespaces = kube.all::<Namespace>();
    /// # Ok(())
    /// # }
    /// ```
    pub fn all<R>(&self) -> kube::Api<R>
    where
        R: kube::Resource,
        <R as kube::Resource>::DynamicType: Default,
    {
        kube::Api::all(self.inner.clone())
    }

    /// Wrap a verb as a tracked [`KubeOp`] for the given [`kube::Api`] handle.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::{KubeClient, verb};
    /// use k8s_openapi::api::core::v1::Pod;
    /// use kube::Config;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let config = Config::infer().await.expect("kubeconfig");
    /// let kube = KubeClient::from_config(config).await?;
    /// let pods = kube.namespaced::<Pod>("default");
    /// let op = kube.op(pods, verb::List::default());
    /// # Ok(())
    /// # }
    /// ```
    pub fn op<R, V>(&self, api: kube::Api<R>, verb: V) -> KubeOp<R, V>
    where
        R: kube::Resource,
        V: Verb,
    {
        KubeOp::new(api, verb)
    }
}

impl fmt::Debug for KubeClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KubeClient")
            .field("client", &"[kube::Client]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    use super::*;

    #[tokio::test]
    async fn from_context_fails_when_not_in_cluster() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = KubeClient::from_context(&ctx).await.unwrap_err();
        assert!(
            err.to_string().contains("in-cluster"),
            "expected in-cluster error, got: {err}"
        );
    }

    #[tokio::test]
    async fn debug_does_not_leak_kubeconfig() {
        use std::convert::Infallible;

        use http::Response;
        use hyper::body::Bytes;
        use tower::service_fn;

        let svc = service_fn(|_req: http::Request<kube::client::Body>| async {
            Ok::<_, Infallible>(Response::new(kube::client::Body::from(Bytes::from_static(
                b"{}",
            ))))
        });
        let client = Client::new(svc, "default");
        let debug = format!("{:?}", KubeClient { inner: client });
        assert!(!debug.contains("token"));
        assert!(!debug.contains("Bearer"));
        assert!(debug.contains("KubeClient"));
    }
}
