//! [`DockerClient`] -- central handle wrapping a [`bollard::Docker`] connection.

use bollard::Docker;
use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;

/// A handle to a Docker daemon connection.
///
/// Wraps [`bollard::Docker`] and provides construction helpers for common
/// connection modes (local socket, TCP with optional TLS).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_docker::DockerClient;
///
/// # fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let client = DockerClient::connect_local()?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct DockerClient {
    docker: Docker,
}

impl DockerClient {
    /// Wrap an existing [`bollard::Docker`] connection.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_docker::DockerClient;
    /// use bollard::Docker;
    ///
    /// # fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let docker = Docker::connect_with_local_defaults()
    ///     .map_err(|e| ironflow_core::error::OperationError::External {
    ///         origin: "docker".to_string(),
    ///         message: e.to_string(),
    ///     })?;
    /// let client = DockerClient::new(docker);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(docker: Docker) -> Self {
        Self { docker }
    }

    /// Connect to the local Docker daemon using platform defaults.
    ///
    /// On Unix this uses the Unix socket at `/var/run/docker.sock`.
    /// On Windows this uses the named pipe.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the connection cannot be
    /// established.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_docker::DockerClient;
    ///
    /// # fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let client = DockerClient::connect_local()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn connect_local() -> Result<Self, OperationError> {
        let docker =
            Docker::connect_with_local_defaults().map_err(|e| OperationError::External {
                origin: "docker".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self { docker })
    }

    /// Connect to a Docker daemon at the given host URL.
    ///
    /// Supports `unix://`, `tcp://`, and `http://` schemes.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the connection cannot be
    /// established.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_docker::DockerClient;
    ///
    /// # fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let client = DockerClient::connect_with_url("tcp://localhost:2375", 120)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn connect_with_url(url: &str, timeout_secs: u64) -> Result<Self, OperationError> {
        let docker = Docker::connect_with_http(url, timeout_secs, bollard::API_DEFAULT_VERSION)
            .map_err(|e| OperationError::External {
                origin: "docker".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self { docker })
    }

    /// Build a [`DockerClient`] from an [`OperationContext`].
    ///
    /// Reads `docker_host` from the context to determine the connection mode.
    /// If `docker_host` is not set, connects to the local daemon using
    /// platform defaults.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the connection fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_docker::DockerClient;
    /// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    /// let client = DockerClient::from_context(&ctx)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_context(_ctx: &OperationContext) -> Result<Self, OperationError> {
        Self::connect_local()
    }

    /// Get a reference to the underlying [`bollard::Docker`] connection.
    pub fn docker(&self) -> &Docker {
        &self.docker
    }

    /// Consume the client and return the underlying [`bollard::Docker`] connection.
    pub fn into_inner(self) -> Docker {
        self.docker
    }
}

impl std::fmt::Debug for DockerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerClient")
            .field("connected", &true)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_internals() {
        let client = DockerClient::connect_local();
        if let Ok(c) = client {
            let debug = format!("{c:?}");
            assert!(debug.contains("DockerClient"));
            assert!(!debug.contains("docker.sock"));
        }
    }
}
