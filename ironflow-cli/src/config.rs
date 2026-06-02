//! Configuration loading for the Ironflow CLI.
//!
//! Supports three configuration sources with the following priority
//! (highest wins):
//!
//! 1. CLI arguments (`--url`, `--api-key`)
//! 2. Environment variables (`IRONFLOW_URL`, `IRONFLOW_API_KEY`)
//! 3. TOML file at `~/.ironflow.toml`

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// TOML file representation.
#[derive(Debug, Deserialize)]
struct FileConfig {
    base_url: Option<String>,
    api_key: Option<String>,
}

/// Resolved configuration ready to build an [`ironflow_sdk::IronflowClient`].
#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL of the Ironflow API.
    pub base_url: String,
    /// API key for Bearer authentication.
    pub api_key: String,
}

/// Default config file path: `~/.ironflow.toml`.
pub fn default_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ironflow.toml"))
}

/// Load configuration by merging CLI args, env vars, and the TOML file.
///
/// # Errors
///
/// Returns an error when neither `base_url` nor `api_key` can be resolved
/// from any source.
pub fn load(cli_url: Option<&str>, cli_api_key: Option<&str>) -> Result<Config> {
    let file_config = default_config_path().filter(|p| p.exists()).and_then(|p| {
        let content = fs::read_to_string(&p).ok()?;
        toml::from_str::<FileConfig>(&content).ok()
    });

    let base_url = cli_url
        .map(String::from)
        .or_else(|| std::env::var("IRONFLOW_URL").ok())
        .or_else(|| file_config.as_ref().and_then(|f| f.base_url.clone()));

    let api_key = cli_api_key
        .map(String::from)
        .or_else(|| std::env::var("IRONFLOW_API_KEY").ok())
        .or_else(|| file_config.as_ref().and_then(|f| f.api_key.clone()));

    let base_url = base_url
        .context("missing base_url: set --url, IRONFLOW_URL, or base_url in ~/.ironflow.toml")?;
    let api_key = api_key.context(
        "missing api_key: set --api-key, IRONFLOW_API_KEY, or api_key in ~/.ironflow.toml",
    )?;

    if base_url.is_empty() {
        bail!("base_url cannot be empty");
    }
    if api_key.is_empty() {
        bail!("api_key cannot be empty");
    }

    Ok(Config { base_url, api_key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_args_override_everything() {
        let config = load(Some("https://cli.example.com"), Some("cli-key")).unwrap();
        assert_eq!(config.base_url, "https://cli.example.com");
        assert_eq!(config.api_key, "cli-key");
    }

    #[test]
    fn error_when_empty_base_url() {
        let result = load(Some(""), Some("key"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn error_when_empty_api_key() {
        let result = load(Some("https://example.com"), Some(""));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn toml_parsing_works() {
        let toml_content = r#"
base_url = "https://toml.example.com"
api_key = "toml-key"
"#;
        let parsed: FileConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(parsed.base_url.unwrap(), "https://toml.example.com");
        assert_eq!(parsed.api_key.unwrap(), "toml-key");
    }

    #[test]
    fn toml_partial_config() {
        let toml_content = r#"
base_url = "https://toml.example.com"
"#;
        let parsed: FileConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(parsed.base_url.unwrap(), "https://toml.example.com");
        assert!(parsed.api_key.is_none());
    }

    #[test]
    fn default_config_path_ends_with_ironflow_toml() {
        let path = default_config_path();
        assert!(path.is_some());
        assert!(path.unwrap().ends_with(".ironflow.toml"));
    }
}
