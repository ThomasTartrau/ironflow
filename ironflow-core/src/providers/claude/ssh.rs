//! SSH transport for Claude Code CLI.
//!
//! [`SshProvider`] connects to a remote host via SSH and executes the `claude`
//! CLI there. This is useful when the Claude CLI is installed on a remote
//! machine (e.g. a build server or GPU instance) but the workflow runs locally.
//!
//! # Requirements
//!
//! The `claude` binary must be available on the remote host's `$PATH` (or at
//! the custom path configured via [`SshProvider::claude_path`]).
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//! use ironflow_core::providers::claude::SshProvider;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let provider = SshProvider::new("build-server.example.com", "deploy")
//!     .password("s3cret");
//!
//! let result = Agent::new()
//!     .prompt("What is 2 + 2?")
//!     .run(&provider)
//!     .await?;
//!
//! println!("{}", result.text());
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::ChannelMsg;
use russh::keys::PrivateKeyWithHashAlg;
use tokio::time;
use tracing::{debug, error, warn};

use crate::error::AgentError;
use crate::provider::{AgentConfig, AgentProvider, InvokeFuture};

use super::common::{self, DEFAULT_TIMEOUT};

/// SSH authentication method.
#[derive(Clone)]
enum SshAuth {
    /// Password authentication.
    Password(String),
    /// Private key authentication with an optional passphrase.
    PrivateKey {
        key_data: String,
        passphrase: Option<String>,
    },
}

/// Policy for verifying the remote SSH server's host key.
///
/// Controls how [`SshProvider`] handles the server's public key during the
/// SSH handshake. The default is [`AcceptAll`](Self::AcceptAll), which is
/// convenient for development but **insecure** for production.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::claude::SshProvider;
/// use ironflow_core::providers::claude::ssh::HostKeyPolicy;
///
/// let provider = SshProvider::new("host.example.com", "deploy")
///     .host_key_policy(HostKeyPolicy::RejectAll)
///     .password("s3cret");
/// ```
#[derive(Clone, Default, Debug)]
pub enum HostKeyPolicy {
    /// Accept any server key without verification (**INSECURE**).
    ///
    /// Suitable only for development and testing. A warning is logged
    /// every time a connection is made with this policy.
    #[default]
    AcceptAll,
    /// Reject all server keys unconditionally.
    ///
    /// Useful for testing error paths or as a safeguard when no
    /// verification mechanism is configured yet.
    RejectAll,
}

/// SSH client handler that applies a [`HostKeyPolicy`] during the handshake.
struct SshHandler {
    policy: HostKeyPolicy,
}

impl russh::client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        match self.policy {
            HostKeyPolicy::AcceptAll => {
                debug!("accepting SSH server key without verification (HostKeyPolicy::AcceptAll)");
                Ok(true)
            }
            HostKeyPolicy::RejectAll => {
                warn!("rejecting SSH server key (HostKeyPolicy::RejectAll)");
                Ok(false)
            }
        }
    }
}

/// [`AgentProvider`] that executes the `claude` CLI on a remote host via SSH.
///
/// Connects over SSH, builds the CLI command with proper shell escaping,
/// executes it in a non-interactive session (no PTY), and parses the JSON
/// response from stdout.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::claude::SshProvider;
///
/// // Password authentication
/// let provider = SshProvider::new("host.example.com", "user")
///     .password("s3cret")
///     .port(2222);
///
/// // Private key authentication
/// let provider = SshProvider::new("host.example.com", "user")
///     .private_key("-----BEGIN OPENSSH PRIVATE KEY-----\n...");
/// ```
#[derive(Clone)]
pub struct SshProvider {
    host: String,
    port: u16,
    username: String,
    auth: Option<SshAuth>,
    claude_path: String,
    working_dir: Option<String>,
    timeout: Duration,
    host_key_policy: HostKeyPolicy,
}

impl SshProvider {
    /// Create a new SSH provider targeting the given host and username.
    ///
    /// Defaults to port 22, the `claude` binary name, and a 5-minute timeout.
    /// You must call one of [`password`](Self::password) or
    /// [`private_key`](Self::private_key) before using the provider.
    pub fn new(host: &str, username: &str) -> Self {
        Self {
            host: host.to_string(),
            port: 22,
            username: username.to_string(),
            auth: None,
            claude_path: "claude".to_string(),
            working_dir: None,
            timeout: DEFAULT_TIMEOUT,
            host_key_policy: HostKeyPolicy::default(),
        }
    }

    /// Set the SSH port (default: 22).
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Authenticate with a password.
    pub fn password(mut self, password: &str) -> Self {
        self.auth = Some(SshAuth::Password(password.to_string()));
        self
    }

    /// Authenticate with a PEM-encoded private key.
    pub fn private_key(mut self, key_data: &str) -> Self {
        self.auth = Some(SshAuth::PrivateKey {
            key_data: key_data.to_string(),
            passphrase: None,
        });
        self
    }

    /// Authenticate with a PEM-encoded private key protected by a passphrase.
    pub fn private_key_with_passphrase(mut self, key_data: &str, passphrase: &str) -> Self {
        self.auth = Some(SshAuth::PrivateKey {
            key_data: key_data.to_string(),
            passphrase: Some(passphrase.to_string()),
        });
        self
    }

    /// Override the path to the `claude` binary on the remote host (default: `"claude"`).
    pub fn claude_path(mut self, path: &str) -> Self {
        self.claude_path = path.to_string();
        self
    }

    /// Set the working directory on the remote host.
    pub fn working_dir(mut self, dir: &str) -> Self {
        self.working_dir = Some(dir.to_string());
        self
    }

    /// Override the default timeout (default: 5 minutes).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the host key verification policy (default: [`HostKeyPolicy::AcceptAll`]).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::SshProvider;
    /// use ironflow_core::providers::claude::ssh::HostKeyPolicy;
    ///
    /// let provider = SshProvider::new("host.example.com", "user")
    ///     .host_key_policy(HostKeyPolicy::RejectAll)
    ///     .password("s3cret");
    /// ```
    pub fn host_key_policy(mut self, policy: HostKeyPolicy) -> Self {
        self.host_key_policy = policy;
        self
    }

    /// Authenticate the SSH session with the configured method.
    async fn authenticate(
        &self,
        session: &mut russh::client::Handle<SshHandler>,
    ) -> Result<(), AgentError> {
        let auth = self
            .auth
            .as_ref()
            .ok_or_else(|| AgentError::ProcessFailed {
                exit_code: -1,
                stderr:
                    "no SSH authentication method configured - call .password() or .private_key()"
                        .to_string(),
            })?;

        let authenticated = match auth {
            SshAuth::Password(pw) => session
                .authenticate_password(&self.username, pw)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("SSH password auth failed: {e}"),
                })?
                .success(),
            SshAuth::PrivateKey {
                key_data,
                passphrase,
            } => {
                let key = russh::keys::decode_secret_key(key_data, passphrase.as_deref()).map_err(
                    |e| AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: format!("failed to parse SSH private key: {e}"),
                    },
                )?;
                let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), None);
                session
                    .authenticate_publickey(&self.username, key_with_alg)
                    .await
                    .map_err(|e| AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: format!("SSH public key auth failed: {e}"),
                    })?
                    .success()
            }
        };

        if !authenticated {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: "SSH authentication rejected by server".to_string(),
            });
        }

        Ok(())
    }
}

impl AgentProvider for SshProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            common::validate_prompt_size(config)?;
            let args = common::build_args(config)?;

            // Build the remote command with shell escaping.
            // Unset all CLAUDE* and IRONFLOW_ALLOW_BYPASS env vars to prevent
            // sub-agent mode interference on the remote host.
            let claude_cmd = common::build_shell_command(&self.claude_path, &args);
            let env_prefix = common::env_unset_shell_prefix();
            let remote_cmd = match (&self.working_dir, &config.working_dir) {
                (_, Some(dir)) | (Some(dir), None) => {
                    format!(
                        "{env_prefix}cd {} && {}",
                        common::build_shell_command(dir, &[]),
                        claude_cmd
                    )
                }
                (None, None) => format!("{env_prefix}{claude_cmd}"),
            };

            debug!(
                host = %self.host,
                port = self.port,
                username = %self.username,
                model = %config.model,
                "connecting via SSH"
            );

            let start = Instant::now();

            // Connect
            let ssh_config = Arc::new(russh::client::Config::default());
            let handler = SshHandler {
                policy: self.host_key_policy.clone(),
            };
            let mut session = time::timeout(
                Duration::from_secs(30),
                russh::client::connect(ssh_config, (&*self.host, self.port), handler),
            )
            .await
            .map_err(|_| AgentError::Timeout {
                limit: Duration::from_secs(30),
            })?
            .map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("SSH connection failed: {e}"),
            })?;

            // Authenticate
            self.authenticate(&mut session).await?;

            // Open channel and execute
            let mut channel =
                session
                    .channel_open_session()
                    .await
                    .map_err(|e| AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: format!("failed to open SSH session channel: {e}"),
                    })?;

            debug!(
                remote_cmd_len = remote_cmd.len(),
                "executing remote command"
            );

            channel
                .exec(true, remote_cmd.as_bytes())
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to exec remote command: {e}"),
                })?;

            // Close stdin immediately (non-interactive)
            channel.eof().await.map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to send EOF on SSH channel: {e}"),
            })?;

            // Collect stdout/stderr
            let mut stdout_buf = Vec::new();
            let mut stderr_buf = Vec::new();
            let mut exit_code: Option<u32> = None;

            let collect_result = time::timeout(self.timeout, async {
                loop {
                    let msg = channel.wait().await;
                    let Some(msg) = msg else { break };
                    match msg {
                        ChannelMsg::Data { ref data } => {
                            stdout_buf.extend_from_slice(data);
                        }
                        ChannelMsg::ExtendedData { ref data, ext } => {
                            if ext == 1 {
                                stderr_buf.extend_from_slice(data);
                            }
                        }
                        ChannelMsg::ExitStatus { exit_status } => {
                            exit_code = Some(exit_status);
                        }
                        _ => {}
                    }
                }
            })
            .await;

            // Disconnect gracefully (best-effort)
            let _ = session
                .disconnect(russh::Disconnect::ByApplication, "", "")
                .await;

            if collect_result.is_err() {
                warn!(timeout = ?self.timeout, "SSH command timed out");
                return Err(AgentError::Timeout {
                    limit: self.timeout,
                });
            }

            let duration_ms = start.elapsed().as_millis() as u64;
            let code = exit_code.unwrap_or(1) as i32;

            let stdout = String::from_utf8_lossy(&stdout_buf).to_string();
            let stderr = String::from_utf8_lossy(&stderr_buf).to_string();

            if code != 0 {
                let error_detail = if stderr.is_empty() {
                    if stdout.is_empty() {
                        "(no output captured)".to_string()
                    } else {
                        stdout.clone()
                    }
                } else {
                    stderr
                };

                error!(
                    exit_code = code,
                    error_detail_len = error_detail.len(),
                    "remote claude process failed"
                );
                return Err(AgentError::ProcessFailed {
                    exit_code: code,
                    stderr: error_detail,
                });
            }

            debug!(stdout_len = stdout.len(), "remote claude process completed");

            common::parse_output(&stdout, config, duration_ms)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_provider_defaults() {
        let provider = SshProvider::new("host.example.com", "user");
        assert_eq!(provider.host, "host.example.com");
        assert_eq!(provider.port, 22);
        assert_eq!(provider.username, "user");
        assert_eq!(provider.claude_path, "claude");
        assert!(provider.working_dir.is_none());
        assert_eq!(provider.timeout, DEFAULT_TIMEOUT);
        assert!(provider.auth.is_none());
        assert!(matches!(provider.host_key_policy, HostKeyPolicy::AcceptAll));
    }

    #[test]
    fn ssh_provider_builder_chain() {
        let provider = SshProvider::new("host", "user")
            .port(2222)
            .password("pw")
            .claude_path("/usr/local/bin/claude")
            .working_dir("/opt/project")
            .timeout(Duration::from_secs(600));

        assert_eq!(provider.port, 2222);
        assert_eq!(provider.claude_path, "/usr/local/bin/claude");
        assert_eq!(provider.working_dir, Some("/opt/project".to_string()));
        assert_eq!(provider.timeout, Duration::from_secs(600));
        assert!(matches!(provider.auth, Some(SshAuth::Password(_))));
    }

    #[test]
    fn ssh_provider_private_key_auth() {
        let provider = SshProvider::new("host", "user").private_key("-----BEGIN KEY-----");
        assert!(matches!(
            provider.auth,
            Some(SshAuth::PrivateKey {
                passphrase: None,
                ..
            })
        ));
    }

    #[test]
    fn ssh_provider_private_key_with_passphrase() {
        let provider = SshProvider::new("host", "user")
            .private_key_with_passphrase("-----BEGIN KEY-----", "secret");
        assert!(matches!(
            provider.auth,
            Some(SshAuth::PrivateKey {
                passphrase: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn ssh_provider_clone() {
        let provider = SshProvider::new("host", "user").port(2222).password("pw");
        let cloned = provider.clone();
        assert_eq!(cloned.host, "host");
        assert_eq!(cloned.port, 2222);
    }

    #[test]
    fn host_key_policy_default_is_accept_all() {
        let policy = HostKeyPolicy::default();
        assert!(matches!(policy, HostKeyPolicy::AcceptAll));
    }

    #[test]
    fn host_key_policy_builder_method() {
        let provider = SshProvider::new("host", "user").host_key_policy(HostKeyPolicy::RejectAll);
        assert!(matches!(provider.host_key_policy, HostKeyPolicy::RejectAll));
    }

    /// A valid Ed25519 public key in OpenSSH format for testing.
    const TEST_PUBKEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBOZMtGiPyW0pMN+JJuYjIGJfqyO5MHBsFkzseVSp60M test@example";

    fn test_public_key() -> russh::keys::PublicKey {
        russh::keys::PublicKey::from_openssh(TEST_PUBKEY).expect("parse test public key")
    }

    #[tokio::test]
    async fn host_key_policy_accept_all_returns_true() {
        use russh::client::Handler;

        let mut handler = SshHandler {
            policy: HostKeyPolicy::AcceptAll,
        };
        let key = test_public_key();
        let result = handler.check_server_key(&key).await;
        assert_eq!(result.unwrap(), true);
    }

    #[tokio::test]
    async fn host_key_policy_reject_all_returns_false() {
        use russh::client::Handler;

        let mut handler = SshHandler {
            policy: HostKeyPolicy::RejectAll,
        };
        let key = test_public_key();
        let result = handler.check_server_key(&key).await;
        assert_eq!(result.unwrap(), false);
    }
}
