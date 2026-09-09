//! OCI registry operations: login, logout.

use std::fmt;

use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde_json::Value;

use crate::client::HelmClient;
use crate::helpers::{run_helm, run_helm_with_stdin, to_value};
use crate::util::TextOutput;

/// Log in to an OCI registry.
///
/// Wraps `helm registry login <host>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::registry::RegistryLogin;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = RegistryLogin::new(client, "registry.example.com", "user", "pass");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct RegistryLogin {
    client: HelmClient,
    host: String,
    username: String,
    password: String,
}

impl RegistryLogin {
    /// Create a new registry-login operation.
    pub fn new(
        client: HelmClient,
        host: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            client,
            host: host.into(),
            username: username.into(),
            password: password.into(),
        }
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the login fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let extra = vec![
            "--username".to_string(),
            self.username.clone(),
            "--password-stdin".to_string(),
        ];
        let stdout = run_helm_with_stdin(
            &self.client,
            &["registry", "login", &self.host],
            &extra,
            &self.password,
        )
        .await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

impl fmt::Debug for RegistryLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegistryLogin")
            .field("host", &self.host)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[async_trait]
impl Operation for RegistryLogin {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "registry login",
            "host": self.host,
            "username": self.username,
        }))
    }
}

impl TypedOperation for RegistryLogin {
    type Output = TextOutput;
}

/// Log out from an OCI registry.
///
/// Wraps `helm registry logout <host>`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_helm::HelmClient;
/// use ironflow_ops_helm::registry::RegistryLogout;
/// use ironflow_core::operation::Operation;
///
/// let client = HelmClient::default();
/// let op = RegistryLogout::new(client, "registry.example.com");
/// assert_eq!(op.kind(), "helm");
/// ```
pub struct RegistryLogout {
    client: HelmClient,
    host: String,
}

impl RegistryLogout {
    /// Create a new registry-logout operation.
    pub fn new(client: HelmClient, host: impl Into<String>) -> Self {
        Self {
            client,
            host: host.into(),
        }
    }

    /// Execute and return the output.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Shell`] if the logout fails.
    pub async fn run(&self, _ctx: &OperationContext) -> Result<TextOutput, OperationError> {
        let stdout = run_helm(&self.client, &["registry", "logout", &self.host]).await?;
        Ok(TextOutput {
            output: stdout.trim().to_string(),
        })
    }
}

#[async_trait]
impl Operation for RegistryLogout {
    fn kind(&self) -> &str {
        "helm"
    }
    async fn execute(&self, ctx: &OperationContext) -> Result<Value, OperationError> {
        to_value(&self.run(ctx).await?)
    }
    fn input(&self) -> Option<Value> {
        Some(serde_json::json!({
            "command": "registry logout",
            "host": self.host,
        }))
    }
}

impl TypedOperation for RegistryLogout {
    type Output = TextOutput;
}

#[cfg(test)]
mod tests {
    use ironflow_core::operation::Operation;

    use super::*;

    fn client() -> HelmClient {
        HelmClient::default()
    }

    #[test]
    fn login_kind() {
        let op = RegistryLogin::new(client(), "host", "user", "pass");
        assert_eq!(op.kind(), "helm");
    }

    #[test]
    fn login_input_does_not_contain_password() {
        let op = RegistryLogin::new(client(), "host", "user", "secret-pass");
        let input = op.input().unwrap();
        let input_str = input.to_string();
        assert!(
            !input_str.contains("secret-pass"),
            "input() must not contain the password"
        );
        assert!(input_str.contains("user"));
    }

    #[test]
    fn login_debug_does_not_leak_password() {
        let op = RegistryLogin::new(client(), "host", "user", "secret-pass");
        let debug = format!("{op:?}");
        assert!(
            !debug.contains("secret-pass"),
            "Debug must not contain password, got: {debug}"
        );
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn logout_kind() {
        let op = RegistryLogout::new(client(), "host");
        assert_eq!(op.kind(), "helm");
    }
}
