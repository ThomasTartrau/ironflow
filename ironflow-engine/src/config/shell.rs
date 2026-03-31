//! [`ShellConfig`] — serializable configuration for a shell step.

use serde::{Deserialize, Serialize};

/// Serializable configuration for a shell step.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::ShellConfig;
///
/// let config = ShellConfig::new("cargo build --release")
///     .timeout_secs(300)
///     .dir("/app");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    /// The shell command to execute.
    pub command: String,
    /// Timeout in seconds (default: 300).
    pub timeout_secs: Option<u64>,
    /// Working directory.
    pub dir: Option<String>,
    /// Environment variables to set.
    pub env: Vec<(String, String)>,
    /// If true, start with a clean environment.
    pub clean_env: bool,
}

impl ShellConfig {
    /// Create a new shell config with the given command.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::ShellConfig;
    ///
    /// let config = ShellConfig::new("echo hello");
    /// assert_eq!(config.command, "echo hello");
    /// ```
    pub fn new(command: &str) -> Self {
        Self {
            command: command.to_string(),
            timeout_secs: None,
            dir: None,
            env: Vec::new(),
            clean_env: false,
        }
    }

    /// Set the timeout in seconds.
    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Set the working directory.
    pub fn dir(mut self, dir: &str) -> Self {
        self.dir = Some(dir.to_string());
        self
    }

    /// Add an environment variable.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    /// Start with a clean environment (no inherited vars).
    pub fn clean_env(mut self) -> Self {
        self.clean_env = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder() {
        let config = ShellConfig::new("cargo test")
            .timeout_secs(60)
            .dir("/app")
            .env("RUST_LOG", "debug")
            .clean_env();

        assert_eq!(config.command, "cargo test");
        assert_eq!(config.timeout_secs, Some(60));
        assert_eq!(config.dir, Some("/app".to_string()));
        assert_eq!(
            config.env,
            vec![("RUST_LOG".to_string(), "debug".to_string())]
        );
        assert!(config.clean_env);
    }
}
