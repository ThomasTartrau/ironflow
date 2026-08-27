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

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::ChannelMsg;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, check_known_hosts_path};
use tokio::time;
use tracing::{debug, warn};

use crate::error::AgentError;
use crate::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture, LogSink};

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
/// SSH handshake. The default is [`RejectAll`](Self::RejectAll), forcing
/// callers to make an explicit security decision.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::claude::SshProvider;
/// use ironflow_core::providers::claude::ssh::HostKeyPolicy;
///
/// // Development only (INSECURE): accept all keys
/// let provider = SshProvider::new("host.example.com", "deploy")
///     .host_key_policy(HostKeyPolicy::AcceptAll)
///     .password("s3cret");
///
/// // Production: verify against a known fingerprint
/// let provider = SshProvider::new("host.example.com", "deploy")
///     .host_key_policy(HostKeyPolicy::Fingerprint(
///         "SHA256:ldyiXa1JQakitNU5tErauu8DvWQ1dZ7aXu+rm7KQuog".to_string(),
///     ))
///     .password("s3cret");
///
/// // Production: verify against a known_hosts file
/// let provider = SshProvider::new("host.example.com", "deploy")
///     .host_key_policy(HostKeyPolicy::KnownHostsFile("/home/deploy/.ssh/known_hosts".into()))
///     .password("s3cret");
/// ```
#[derive(Clone, Default, Debug)]
pub enum HostKeyPolicy {
    /// Accept any server key without verification (**INSECURE**).
    ///
    /// Suitable only for development and testing. A warning is logged
    /// every time a connection is made with this policy.
    AcceptAll,
    /// Reject all server keys unconditionally.
    ///
    /// This is the default policy, forcing callers to make an explicit
    /// security decision before connecting.
    ///
    #[default]
    RejectAll,
    /// Accept only if the server key's SHA-256 fingerprint matches.
    ///
    /// The fingerprint string must be in the standard `SHA256:<base64>`
    /// format (e.g. `"SHA256:ldyiXa1JQakitNU5tErauu8DvWQ1dZ7aXu+rm7KQuog"`).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::ssh::HostKeyPolicy;
    ///
    /// let policy = HostKeyPolicy::Fingerprint(
    ///     "SHA256:ldyiXa1JQakitNU5tErauu8DvWQ1dZ7aXu+rm7KQuog".to_string(),
    /// );
    /// ```
    Fingerprint(String),
    /// Verify the server key against a known_hosts file.
    ///
    /// Uses the OpenSSH `known_hosts` file format. The key must be present
    /// and match; unknown or changed keys are rejected.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::ssh::HostKeyPolicy;
    ///
    /// let policy = HostKeyPolicy::KnownHostsFile("/home/deploy/.ssh/known_hosts".into());
    /// ```
    KnownHostsFile(PathBuf),
}

/// SSH client handler that applies a [`HostKeyPolicy`] during the handshake.
struct SshHandler {
    policy: HostKeyPolicy,
    host: String,
    port: u16,
}

impl russh::client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        match &self.policy {
            HostKeyPolicy::AcceptAll => {
                warn!("accepting SSH server key without verification (HostKeyPolicy::AcceptAll)");
                Ok(true)
            }
            HostKeyPolicy::RejectAll => {
                warn!("rejecting SSH server key (HostKeyPolicy::RejectAll)");
                Ok(false)
            }
            HostKeyPolicy::Fingerprint(expected) => {
                let actual = server_public_key.fingerprint(HashAlg::Sha256).to_string();
                if actual == *expected {
                    debug!(fingerprint = %actual, "SSH server key fingerprint matches");
                    Ok(true)
                } else {
                    warn!(
                        expected = %expected,
                        actual = %actual,
                        "SSH server key fingerprint mismatch"
                    );
                    Ok(false)
                }
            }
            HostKeyPolicy::KnownHostsFile(path) => {
                match check_known_hosts_path(&self.host, self.port, server_public_key, path) {
                    Ok(true) => {
                        debug!(path = %path.display(), "SSH server key verified against known_hosts");
                        Ok(true)
                    }
                    Ok(false) => {
                        warn!(
                            path = %path.display(),
                            host = %self.host,
                            "SSH server key not found in known_hosts"
                        );
                        Ok(false)
                    }
                    Err(e) => {
                        warn!(
                            path = %path.display(),
                            error = %e,
                            "known_hosts verification failed"
                        );
                        Ok(false)
                    }
                }
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

    /// Set the host key verification policy (default: [`HostKeyPolicy::RejectAll`]).
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

impl SshProvider {
    /// Shared implementation for `invoke` and `invoke_with_logs`.
    async fn invoke_inner(
        &self,
        config: &AgentConfig,
        log_sink: Option<Arc<dyn LogSink>>,
    ) -> Result<AgentOutput, AgentError> {
        let forced = common::force_verbose_for_streaming(config, log_sink.is_some());
        let config = forced.as_ref().unwrap_or(config);

        common::validate_prompt_size(config)?;
        let built = common::build_command(config)?;

        let claude_cmd = common::build_shell_command(&self.claude_path, &built.args);
        let mut env_prefix = common::env_unset_shell_prefix();
        if let Some(ref ctx) = config.trace_context {
            env_prefix = format!("export TRACEPARENT='{}'; {env_prefix}", ctx.to_traceparent());
        }
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
            host: self.host.clone(),
            port: self.port,
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

        if let Some(ref prompt) = built.stdin_prompt {
            let cursor = Cursor::new(prompt.as_bytes());
            channel
                .data(cursor)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to write prompt to SSH stdin: {e}"),
                })?;
        }

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
                        common::stream_lines(data, "stdout", log_sink.as_ref());
                        stdout_buf.extend_from_slice(data);
                    }
                    ChannelMsg::ExtendedData { ref data, ext } => {
                        if ext == 1 {
                            common::stream_lines(data, "stderr", log_sink.as_ref());
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
            return common::handle_nonzero_exit(code, &stdout, &stderr, config, duration_ms, "ssh");
        }

        debug!(stdout_len = stdout.len(), "remote claude process completed");

        common::parse_output(&stdout, config, duration_ms)
    }
}

impl AgentProvider for SshProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(self.invoke_inner(config, None))
    }

    fn invoke_with_logs<'a>(
        &'a self,
        config: &'a AgentConfig,
        log_sink: Arc<dyn LogSink>,
    ) -> InvokeFuture<'a> {
        Box::pin(self.invoke_inner(config, Some(log_sink)))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use russh::client::Handler;
    use russh::keys::HashAlg;
    use tempfile::NamedTempFile;

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
        assert!(matches!(provider.host_key_policy, HostKeyPolicy::RejectAll));
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
    fn host_key_policy_default_is_reject_all() {
        let policy = HostKeyPolicy::default();
        assert!(matches!(policy, HostKeyPolicy::RejectAll));
    }

    #[test]
    fn host_key_policy_builder_method() {
        let provider = SshProvider::new("host", "user").host_key_policy(HostKeyPolicy::RejectAll);
        assert!(matches!(provider.host_key_policy, HostKeyPolicy::RejectAll));
    }

    /// A valid Ed25519 public key base64 for testing.
    const TEST_PUBKEY_B64: &str =
        "AAAAC3NzaC1lZDI1NTE5AAAAIBOZMtGiPyW0pMN+JJuYjIGJfqyO5MHBsFkzseVSp60M";

    fn test_public_key() -> russh::keys::PublicKey {
        russh::keys::parse_public_key_base64(TEST_PUBKEY_B64).expect("parse test public key")
    }

    fn make_handler(policy: HostKeyPolicy) -> SshHandler {
        SshHandler {
            policy,
            host: "host.example.com".to_string(),
            port: 22,
        }
    }

    #[tokio::test]
    async fn host_key_policy_accept_all_returns_true() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut handler = make_handler(HostKeyPolicy::AcceptAll);
            let key = test_public_key();
            let result = handler.check_server_key(&key).await;
            assert!(result.unwrap());
        })
        .await
        .expect("test timed out");
    }

    #[tokio::test]
    async fn host_key_policy_reject_all_returns_false() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut handler = make_handler(HostKeyPolicy::RejectAll);
            let key = test_public_key();
            let result = handler.check_server_key(&key).await;
            assert!(!result.unwrap());
        })
        .await
        .expect("test timed out");
    }

    #[tokio::test]
    async fn host_key_policy_fingerprint_match() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let key = test_public_key();
            let expected = key.fingerprint(HashAlg::Sha256).to_string();
            let mut handler = make_handler(HostKeyPolicy::Fingerprint(expected));
            let result = handler.check_server_key(&key).await;
            assert!(result.unwrap());
        })
        .await
        .expect("test timed out");
    }

    #[tokio::test]
    async fn host_key_policy_fingerprint_mismatch() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let key = test_public_key();
            let mut handler = make_handler(HostKeyPolicy::Fingerprint(
                "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            ));
            let result = handler.check_server_key(&key).await;
            assert!(!result.unwrap());
        })
        .await
        .expect("test timed out");
    }

    #[tokio::test]
    async fn host_key_policy_known_hosts_match() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let key = test_public_key();
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, "host.example.com ssh-ed25519 {TEST_PUBKEY_B64}").unwrap();

            let mut handler =
                make_handler(HostKeyPolicy::KnownHostsFile(file.path().to_path_buf()));
            let result = handler.check_server_key(&key).await;
            assert!(result.unwrap());
        })
        .await
        .expect("test timed out");
    }

    #[tokio::test]
    async fn host_key_policy_known_hosts_reject() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let key = test_public_key();
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, "other-host.example.com ssh-ed25519 {TEST_PUBKEY_B64}").unwrap();

            let mut handler =
                make_handler(HostKeyPolicy::KnownHostsFile(file.path().to_path_buf()));
            let result = handler.check_server_key(&key).await;
            assert!(!result.unwrap());
        })
        .await
        .expect("test timed out");
    }

    #[tokio::test]
    async fn host_key_policy_known_hosts_missing_file() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let key = test_public_key();
            let mut handler = make_handler(HostKeyPolicy::KnownHostsFile(PathBuf::from(
                "/nonexistent/path/known_hosts",
            )));
            let result = handler.check_server_key(&key).await;
            assert!(!result.unwrap());
        })
        .await
        .expect("test timed out");
    }
}
