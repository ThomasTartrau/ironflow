//! Server configuration with startup validation.
//!
//! Loads configuration from environment variables and validates that all
//! required values are present **at startup**, not at first use.
//!
//! # Environment Variables
//!
//! | Variable | Required | Default | Description |
//! |----------|----------|---------|-------------|
//! | `DATABASE_URL` | **prod** | - | PostgreSQL connection string |
//! | `JWT_SECRET` | **prod** | dev default | JWT signing secret |
//! | `WORKER_TOKEN` | **prod** | dev default | Worker-to-API auth token |
//! | `PORT` | no | `3000` | HTTP listen port |
//! | `ALLOWED_ORIGINS` | no | same-origin | Comma-separated CORS origins |
//! | `DASHBOARD_DIR` | no | embedded | Filesystem path to dashboard assets |
//! | `WEBHOOK_URL` | no | - | Outbound webhook URL for notifications |
//! | `IRONFLOW_ENV` | no | `development` | `production` or `development` |
//! | `RATE_LIMIT_AUTH` | no | `10` | Auth rate limit (req/min/IP). `0` = disabled |
//! | `RATE_LIMIT_GENERAL` | no | `60` | General rate limit (req/min/IP). `0` = disabled |
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_api::config::ServerConfig;
//!
//! # fn example() -> Result<(), ironflow_api::config::ConfigError> {
//! let config = ServerConfig::from_env()?;
//! println!("Listening on port {}", config.port);
//! # Ok(())
//! # }
//! ```

use std::env;
use std::fmt;
use std::path::PathBuf;

use tracing::warn;

/// Server configuration loaded from environment variables.
///
/// Use [`ServerConfig::from_env`] to load and validate at startup.
///
/// # Examples
///
/// ```no_run
/// use ironflow_api::config::ServerConfig;
///
/// # fn example() -> Result<(), ironflow_api::config::ConfigError> {
/// let config = ServerConfig::from_env()?;
/// assert!(config.port > 0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// PostgreSQL connection string. Required in production.
    pub database_url: Option<String>,
    /// JWT signing secret.
    pub jwt_secret: String,
    /// Worker-to-API authentication token.
    pub worker_token: String,
    /// HTTP listen port.
    pub port: u16,
    /// Comma-separated list of allowed CORS origins.
    pub allowed_origins: Option<String>,
    /// Filesystem path to dashboard assets (overrides embedded).
    pub dashboard_dir: Option<PathBuf>,
    /// Outbound webhook URL for event notifications.
    pub webhook_url: Option<String>,
    /// Whether the server is running in production mode.
    pub is_production: bool,
    /// Rate limit for auth credential routes (sign-in, sign-up) in requests
    /// per minute per IP. `None` disables rate limiting on these routes.
    pub rate_limit_auth: Option<u32>,
    /// Rate limit for general public API routes in requests per minute per IP.
    /// `None` disables rate limiting on these routes.
    pub rate_limit_general: Option<u32>,
}

/// Configuration validation error.
///
/// Collects all missing/invalid values so the operator sees every problem
/// in a single error message, not one at a time.
///
/// # Examples
///
/// ```
/// use ironflow_api::config::ConfigError;
///
/// let err = ConfigError::new(vec!["JWT_SECRET is required in production".to_string()]);
/// assert!(err.to_string().contains("JWT_SECRET"));
/// ```
#[derive(Debug, Clone)]
pub struct ConfigError {
    /// Individual validation failure messages.
    pub errors: Vec<String>,
}

impl ConfigError {
    /// Create a new `ConfigError` from a list of validation messages.
    pub fn new(errors: Vec<String>) -> Self {
        Self { errors }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "configuration errors:")?;
        for error in &self.errors {
            writeln!(f, "  - {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

const DEV_JWT_SECRET: &str = "ironflow-dev-secret";
const DEV_WORKER_TOKEN: &str = "ironflow-dev-worker-token";

/// Parse an optional u32 env var. Returns `Some(default)` if unset,
/// `Some(value)` if set to a positive number, `None` if set to `0`
/// (meaning disabled). Pushes to `errors` if the value is not a valid u32.
fn parse_optional_u32(name: &str, default: u32, errors: &mut Vec<String>) -> Option<u32> {
    match env::var(name).ok() {
        Some(raw) => match raw.parse::<u32>() {
            Ok(0) => None,
            Ok(v) => Some(v),
            Err(_) => {
                errors.push(format!(
                    "{name} must be a valid u32 (0 to disable), got: {raw}"
                ));
                Some(default)
            }
        },
        None => Some(default),
    }
}

impl ServerConfig {
    /// Load configuration from environment variables and validate.
    ///
    /// In production mode (`IRONFLOW_ENV=production`), `JWT_SECRET` and
    /// `WORKER_TOKEN` must be explicitly set (dev defaults are rejected).
    /// `DATABASE_URL` is required in production.
    ///
    /// In development mode, insecure defaults are used with a warning.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] with all validation failures collected,
    /// so the operator can fix everything in one pass.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_api::config::ServerConfig;
    ///
    /// # fn example() -> Result<(), ironflow_api::config::ConfigError> {
    /// let config = ServerConfig::from_env()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env() -> Result<Self, ConfigError> {
        let is_production = env::var("IRONFLOW_ENV")
            .map(|v| v.eq_ignore_ascii_case("production"))
            .unwrap_or(false);

        let mut errors = Vec::new();

        let database_url = env::var("DATABASE_URL").ok();
        if is_production && database_url.is_none() {
            errors.push("DATABASE_URL is required in production".to_string());
        }

        let jwt_secret_env = env::var("JWT_SECRET").ok();
        let jwt_secret = match jwt_secret_env {
            Some(val) => val,
            None if is_production => {
                errors.push("JWT_SECRET is required in production".to_string());
                String::new()
            }
            None => {
                warn!("JWT_SECRET not set, using insecure dev default -- do NOT use in production");
                DEV_JWT_SECRET.to_string()
            }
        };

        let worker_token_env = env::var("WORKER_TOKEN").ok();
        let worker_token = match worker_token_env {
            Some(val) => val,
            None if is_production => {
                errors.push("WORKER_TOKEN is required in production".to_string());
                String::new()
            }
            None => {
                warn!(
                    "WORKER_TOKEN not set, using insecure dev default -- do NOT use in production"
                );
                DEV_WORKER_TOKEN.to_string()
            }
        };

        let port = match env::var("PORT").ok() {
            Some(raw) => raw.parse::<u16>().unwrap_or_else(|_| {
                errors.push(format!("PORT must be a valid u16, got: {raw}"));
                0
            }),
            None => 3000,
        };

        let allowed_origins = env::var("ALLOWED_ORIGINS").ok();
        let dashboard_dir = env::var("DASHBOARD_DIR").ok().map(PathBuf::from);
        let webhook_url = env::var("WEBHOOK_URL").ok();

        let rate_limit_auth = parse_optional_u32("RATE_LIMIT_AUTH", 10, &mut errors);
        let rate_limit_general = parse_optional_u32("RATE_LIMIT_GENERAL", 60, &mut errors);

        if !errors.is_empty() {
            return Err(ConfigError::new(errors));
        }

        Ok(Self {
            database_url,
            jwt_secret,
            worker_token,
            port,
            allowed_origins,
            dashboard_dir,
            webhook_url,
            is_production,
            rate_limit_auth,
            rate_limit_general,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    // Env var mutations are not thread-safe -- serialize all tests that touch them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// # Safety
    ///
    /// Must be called while holding `ENV_LOCK`.
    unsafe fn clear_env() {
        unsafe {
            env::remove_var("IRONFLOW_ENV");
            env::remove_var("DATABASE_URL");
            env::remove_var("JWT_SECRET");
            env::remove_var("WORKER_TOKEN");
            env::remove_var("PORT");
            env::remove_var("ALLOWED_ORIGINS");
            env::remove_var("DASHBOARD_DIR");
            env::remove_var("WEBHOOK_URL");
            env::remove_var("RATE_LIMIT_AUTH");
            env::remove_var("RATE_LIMIT_GENERAL");
        }
    }

    #[test]
    fn config_error_display_lists_all_errors() {
        let err = ConfigError::new(vec![
            "JWT_SECRET is required".to_string(),
            "DATABASE_URL is required".to_string(),
        ]);
        let msg = err.to_string();
        assert!(msg.contains("JWT_SECRET"));
        assert!(msg.contains("DATABASE_URL"));
        assert!(msg.contains("configuration errors:"));
    }

    #[test]
    fn config_error_is_std_error() {
        let err = ConfigError::new(vec!["test".to_string()]);
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn default_dev_config_succeeds() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { clear_env() };

        let config = ServerConfig::from_env().expect("dev config should succeed");
        assert!(!config.is_production);
        assert_eq!(config.port, 3000);
        assert_eq!(config.jwt_secret, DEV_JWT_SECRET);
        assert_eq!(config.worker_token, DEV_WORKER_TOKEN);
    }

    #[test]
    fn production_without_secrets_fails() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            env::set_var("IRONFLOW_ENV", "production");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.errors.len() >= 3);
        assert!(err.errors.iter().any(|e| e.contains("DATABASE_URL")));
        assert!(err.errors.iter().any(|e| e.contains("JWT_SECRET")));
        assert!(err.errors.iter().any(|e| e.contains("WORKER_TOKEN")));

        unsafe { env::remove_var("IRONFLOW_ENV") };
    }

    #[test]
    fn invalid_port_returns_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            env::set_var("PORT", "not-a-number");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.errors.iter().any(|e| e.contains("PORT")));

        unsafe { env::remove_var("PORT") };
    }

    #[test]
    fn default_rate_limits() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { clear_env() };

        let config = ServerConfig::from_env().unwrap();
        assert_eq!(config.rate_limit_auth, Some(10));
        assert_eq!(config.rate_limit_general, Some(60));
    }

    #[test]
    fn custom_rate_limits() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            env::set_var("RATE_LIMIT_AUTH", "20");
            env::set_var("RATE_LIMIT_GENERAL", "120");
        }

        let config = ServerConfig::from_env().unwrap();
        assert_eq!(config.rate_limit_auth, Some(20));
        assert_eq!(config.rate_limit_general, Some(120));

        unsafe {
            env::remove_var("RATE_LIMIT_AUTH");
            env::remove_var("RATE_LIMIT_GENERAL");
        }
    }

    #[test]
    fn zero_rate_limit_disables() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            env::set_var("RATE_LIMIT_AUTH", "0");
            env::set_var("RATE_LIMIT_GENERAL", "0");
        }

        let config = ServerConfig::from_env().unwrap();
        assert!(config.rate_limit_auth.is_none());
        assert!(config.rate_limit_general.is_none());

        unsafe {
            env::remove_var("RATE_LIMIT_AUTH");
            env::remove_var("RATE_LIMIT_GENERAL");
        }
    }

    #[test]
    fn invalid_rate_limit_returns_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            env::set_var("RATE_LIMIT_AUTH", "not-a-number");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.errors.iter().any(|e| e.contains("RATE_LIMIT_AUTH")));

        unsafe { env::remove_var("RATE_LIMIT_AUTH") };
    }
}
