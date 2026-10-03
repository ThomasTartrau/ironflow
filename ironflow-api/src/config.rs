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
//! | `JWT_SECRET` | **yes**, except explicit dev | random in explicit dev | JWT signing secret, >= 32 bytes |
//! | `WORKER_TOKEN` | **yes**, except explicit dev | random in explicit dev | Worker-to-API auth token, >= 32 bytes |
//! | `PORT` | no | `3000` | HTTP listen port |
//! | `ALLOWED_ORIGINS` | no | same-origin | Comma-separated CORS origins |
//! | `DASHBOARD_DIR` | no | embedded | Filesystem path to dashboard assets |
//! | `WEBHOOK_URL` | no | - | Outbound webhook URL for notifications |
//! | `IRONFLOW_ENV` | no | unset (strict) | `production`, or `development` for explicit dev mode |
//! | `IRONFLOW_INSECURE_COOKIES` | no | `false` | `1`/`true` drops the `Secure` flag from session cookies (local HTTP-only dev). Ignored in production |
//! | `RATE_LIMIT_AUTH` | no | `10` | Auth rate limit (req/min/IP). `0` = disabled |
//! | `RATE_LIMIT_GENERAL` | no | `60` | General rate limit (req/min/IP). `0` = disabled |
//! | `ARTIFACTS_DIR` | no | - | Filesystem root for artifact blobs. Unset disables artifacts |
//! | `ARTIFACT_MAX_BYTES` | no | `104857600` | Maximum size of a single artifact |
//! | `PURGE_MAX_AGE_DAYS` | no | `90` | Runs older than this are purged |
//! | `PURGE_MAX_RUNS_PER_WORKFLOW` | no | `1000` | Max terminal runs kept per workflow |
//! | `PURGE_DRY_RUN` | no | `false` | Log what would be purged without deleting |
//! | `PURGE_INTERVAL_SECS` | no | `86400` | Seconds between purge ticks (min 60) |
//! | `PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS` | no | `30` | Days of Provider Account usage history kept (min 1) |
//! | `SIGNAL_RETENTION_DAYS` | no | `7` | Days received signals are kept (min 1) |
//! | `ARTIFACT_BACKEND` | no | `local` | Blob storage backend: `local` or `s3` |
//! | `ARTIFACT_S3_BUCKET` | **if s3** | - | S3 bucket name |
//! | `ARTIFACT_S3_REGION` | no | `eu-west-1` | S3 region |
//! | `ARTIFACT_S3_ENDPOINT` | no | - | Custom S3 endpoint (MinIO, R2) |
//! | `ARTIFACT_S3_PREFIX` | no | - | Key prefix within the bucket |
//! | `ARTIFACT_GC_INTERVAL_SECS` | no | `86400` | Seconds between GC ticks (min 60) |
//! | `ARTIFACT_GC_GRACE_DAYS` | no | `7` | Days before an orphan blob is deleted |
//! | `ARTIFACT_GC_DRY_RUN` | no | `false` | Log what would be GC'd without deleting |
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

use ironflow_artifacts::local::DEFAULT_MAX_ARTIFACT_BYTES;
use tracing::warn;

use self::secrets::{ResolvedSecret, SecretMode, resolve_secret};

mod secrets;

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
    /// Filesystem root for artifact blobs.
    ///
    /// `None` leaves artifacts disabled: the artifact routes answer `501` and
    /// a step that declares one fails explicitly. Every other endpoint is
    /// unaffected, so an existing deployment upgrades without changes.
    ///
    /// Read from `ARTIFACTS_DIR`.
    pub artifacts_dir: Option<PathBuf>,
    /// Maximum size of a single artifact, in bytes.
    ///
    /// Read from `ARTIFACT_MAX_BYTES`, defaulting to 100 MiB.
    pub artifact_max_bytes: u64,
    /// Maximum age of a run before it becomes eligible for purging, in days.
    ///
    /// Read from `PURGE_MAX_AGE_DAYS`, defaulting to 90.
    pub purge_max_age_days: u32,
    /// Maximum number of terminal runs to keep per workflow.
    ///
    /// Read from `PURGE_MAX_RUNS_PER_WORKFLOW`, defaulting to 1000.
    pub purge_max_runs_per_workflow: u32,
    /// When `true`, the purger logs what would be deleted but does not delete.
    ///
    /// Read from `PURGE_DRY_RUN`, defaulting to `false`.
    pub purge_dry_run: bool,
    /// Interval between purge ticks, in seconds.
    ///
    /// Read from `PURGE_INTERVAL_SECS`, defaulting to 86400 (once per day).
    pub purge_interval_secs: u64,
    /// Days of Provider Account usage history kept by the purger.
    ///
    /// Read from `PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS`, defaulting to 30.
    pub provider_account_usage_retention_days: u32,
    /// Days received signals are kept by the purger.
    ///
    /// Read from `SIGNAL_RETENTION_DAYS`, defaulting to 7.
    pub signal_retention_days: u32,
    /// Whether the server is running in production mode.
    pub is_production: bool,
    /// Whether session cookies carry the `Secure` flag.
    ///
    /// Always `true` in production. In development it defaults to `true` and
    /// is switched off only by `IRONFLOW_INSECURE_COOKIES=1` (or `true`), for
    /// local setups served over plain HTTP from a non-localhost origin.
    pub cookie_secure: bool,
    /// Rate limit for auth credential routes (sign-in, sign-up) in requests
    /// per minute per IP. `None` disables rate limiting on these routes.
    pub rate_limit_auth: Option<u32>,
    /// Rate limit for general public API routes in requests per minute per IP.
    /// `None` disables rate limiting on these routes.
    pub rate_limit_general: Option<u32>,
    /// Blob storage backend: `local` or `s3`.
    pub artifact_backend: String,
    /// S3 bucket name (required when `artifact_backend` is `s3`).
    pub artifact_s3_bucket: Option<String>,
    /// S3 region, defaults to `eu-west-1`.
    pub artifact_s3_region: String,
    /// Custom S3 endpoint for MinIO, R2, or GCS S3-compat.
    pub artifact_s3_endpoint: Option<String>,
    /// Key prefix within the S3 bucket.
    pub artifact_s3_prefix: Option<String>,
    /// Seconds between GC ticks, defaults to 86400 (once per day).
    pub artifact_gc_interval_secs: u64,
    /// Days before an orphan blob is eligible for deletion, defaults to 7.
    pub artifact_gc_grace_days: u32,
    /// When `true`, the GC logs what would be deleted without deleting.
    pub artifact_gc_dry_run: bool,
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
    /// `JWT_SECRET` and `WORKER_TOKEN` must be set, at least 32 bytes long and
    /// must not carry the prefix `ironflow-dev-` of the development values once
    /// published in the repository. `DATABASE_URL` is also required in
    /// production (`IRONFLOW_ENV=production`).
    ///
    /// Only an explicit `IRONFLOW_ENV=development` relaxes this: a missing
    /// secret is then generated at random for this process (the worker token
    /// is logged so a worker can be started with it), and the length check is
    /// waived. The `ironflow-dev-` prefix and empty values are refused in every
    /// mode. No secret is compiled into the binary.
    ///
    /// Session cookies are `Secure` unless `IRONFLOW_INSECURE_COOKIES` is `1`
    /// or `true` outside production. In production the variable is ignored
    /// with a warning.
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
        let secret_mode = SecretMode::from_ironflow_env(env::var("IRONFLOW_ENV").ok().as_deref());
        let is_production = secret_mode == SecretMode::Production;

        let mut errors = Vec::new();

        let insecure_cookies = env::var("IRONFLOW_INSECURE_COOKIES")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
        if is_production && insecure_cookies {
            warn!("IRONFLOW_INSECURE_COOKIES is ignored in production, cookies stay Secure");
        }
        let cookie_secure = is_production || !insecure_cookies;

        let database_url = env::var("DATABASE_URL").ok();
        if is_production && database_url.is_none() {
            errors.push("DATABASE_URL is required in production".to_string());
        }

        let jwt_secret = match resolve_secret(
            "JWT_SECRET",
            env::var("JWT_SECRET").ok(),
            secret_mode,
        ) {
            Ok(ResolvedSecret::Provided(secret)) => secret,
            Ok(ResolvedSecret::Generated(secret)) => {
                warn!(
                    "JWT_SECRET not set, generated an ephemeral secret: dashboard sessions end with this process"
                );
                secret
            }
            Err(e) => {
                errors.push(e);
                String::new()
            }
        };

        let worker_token = match resolve_secret(
            "WORKER_TOKEN",
            env::var("WORKER_TOKEN").ok(),
            secret_mode,
        ) {
            Ok(ResolvedSecret::Provided(token)) => token,
            // Shown on purpose: a worker is a separate process and cannot
            // reach this process without it. Development only.
            Ok(ResolvedSecret::Generated(token)) => {
                warn!(
                    "WORKER_TOKEN not set, generated an ephemeral token for this process: start workers with WORKER_TOKEN={token}"
                );
                token
            }
            Err(e) => {
                errors.push(e);
                String::new()
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

        let artifacts_dir = env::var("ARTIFACTS_DIR").ok().map(PathBuf::from);
        let artifact_max_bytes = match env::var("ARTIFACT_MAX_BYTES").ok() {
            Some(raw) => raw.parse::<u64>().unwrap_or_else(|_| {
                errors.push(format!(
                    "ARTIFACT_MAX_BYTES must be a valid u64, got: {raw}"
                ));
                DEFAULT_MAX_ARTIFACT_BYTES
            }),
            None => DEFAULT_MAX_ARTIFACT_BYTES,
        };

        let purge_max_age_days = match env::var("PURGE_MAX_AGE_DAYS").ok() {
            Some(raw) => raw.parse::<u32>().unwrap_or_else(|_| {
                errors.push(format!(
                    "PURGE_MAX_AGE_DAYS must be a valid u32, got: {raw}"
                ));
                90
            }),
            None => 90,
        };
        let purge_max_runs_per_workflow = match env::var("PURGE_MAX_RUNS_PER_WORKFLOW").ok() {
            Some(raw) => raw.parse::<u32>().unwrap_or_else(|_| {
                errors.push(format!(
                    "PURGE_MAX_RUNS_PER_WORKFLOW must be a valid u32, got: {raw}"
                ));
                1000
            }),
            None => 1000,
        };
        let purge_dry_run = env::var("PURGE_DRY_RUN")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
        let provider_account_usage_retention_days =
            match env::var("PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS").ok() {
                Some(raw) => match raw.parse::<u32>() {
                    Ok(days) if days >= 1 => days,
                    _ => {
                        errors.push(format!(
                        "PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS must be an integer >= 1, got: {raw}"
                    ));
                        30
                    }
                },
                None => 30,
            };
        let signal_retention_days = match env::var("SIGNAL_RETENTION_DAYS").ok() {
            Some(raw) => match raw.parse::<u32>() {
                Ok(days) if days >= 1 => days,
                _ => {
                    errors.push(format!(
                        "SIGNAL_RETENTION_DAYS must be an integer >= 1, got: {raw}"
                    ));
                    7
                }
            },
            None => 7,
        };
        let purge_interval_secs = match env::var("PURGE_INTERVAL_SECS").ok() {
            Some(raw) => {
                let parsed = raw.parse::<u64>().unwrap_or_else(|_| {
                    errors.push(format!(
                        "PURGE_INTERVAL_SECS must be a valid u64, got: {raw}"
                    ));
                    86400
                });
                if parsed < 60 {
                    errors.push(format!(
                        "PURGE_INTERVAL_SECS must be at least 60, got: {parsed}"
                    ));
                }
                parsed
            }
            None => 86400,
        };

        let artifact_backend = env::var("ARTIFACT_BACKEND")
            .unwrap_or_else(|_| "local".to_string())
            .to_lowercase();
        let artifact_s3_bucket = env::var("ARTIFACT_S3_BUCKET").ok();
        let artifact_s3_region =
            env::var("ARTIFACT_S3_REGION").unwrap_or_else(|_| "eu-west-1".to_string());
        let artifact_s3_endpoint = env::var("ARTIFACT_S3_ENDPOINT").ok();
        let artifact_s3_prefix = env::var("ARTIFACT_S3_PREFIX").ok();

        if artifact_backend == "s3" && artifact_s3_bucket.is_none() {
            errors.push("ARTIFACT_S3_BUCKET is required when ARTIFACT_BACKEND=s3".to_string());
        }
        if artifact_backend != "local" && artifact_backend != "s3" {
            errors.push(format!(
                "ARTIFACT_BACKEND must be 'local' or 's3', got: {artifact_backend}"
            ));
        }

        let artifact_gc_interval_secs = match env::var("ARTIFACT_GC_INTERVAL_SECS").ok() {
            Some(raw) => {
                let parsed = raw.parse::<u64>().unwrap_or_else(|_| {
                    errors.push(format!(
                        "ARTIFACT_GC_INTERVAL_SECS must be a valid u64, got: {raw}"
                    ));
                    86400
                });
                if parsed < 60 {
                    errors.push(format!(
                        "ARTIFACT_GC_INTERVAL_SECS must be at least 60, got: {parsed}"
                    ));
                }
                parsed
            }
            None => 86400,
        };
        let artifact_gc_grace_days = match env::var("ARTIFACT_GC_GRACE_DAYS").ok() {
            Some(raw) => raw.parse::<u32>().unwrap_or_else(|_| {
                errors.push(format!(
                    "ARTIFACT_GC_GRACE_DAYS must be a valid u32, got: {raw}"
                ));
                7
            }),
            None => 7,
        };
        let artifact_gc_dry_run = env::var("ARTIFACT_GC_DRY_RUN")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

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
            cookie_secure,
            rate_limit_auth,
            rate_limit_general,
            artifacts_dir,
            artifact_max_bytes,
            purge_max_age_days,
            purge_max_runs_per_workflow,
            purge_dry_run,
            purge_interval_secs,
            provider_account_usage_retention_days,
            signal_retention_days,
            artifact_backend,
            artifact_s3_bucket,
            artifact_s3_region,
            artifact_s3_endpoint,
            artifact_s3_prefix,
            artifact_gc_interval_secs,
            artifact_gc_grace_days,
            artifact_gc_dry_run,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use ironflow_store::memory::InMemoryStore;

    use crate::purger::{DEFAULT_SIGNAL_RETENTION_DAYS, RunPurger};

    use super::*;

    // Env var mutations are not thread-safe -- serialize all tests that touch them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// # Safety
    ///
    /// Must be called while holding `ENV_LOCK`.
    unsafe fn clear_env() {
        unsafe {
            env::remove_var("IRONFLOW_ENV");
            env::remove_var("IRONFLOW_INSECURE_COOKIES");
            env::remove_var("DATABASE_URL");
            env::remove_var("JWT_SECRET");
            env::remove_var("WORKER_TOKEN");
            env::remove_var("PORT");
            env::remove_var("ALLOWED_ORIGINS");
            env::remove_var("DASHBOARD_DIR");
            env::remove_var("WEBHOOK_URL");
            env::remove_var("RATE_LIMIT_AUTH");
            env::remove_var("RATE_LIMIT_GENERAL");
            env::remove_var("PURGE_MAX_AGE_DAYS");
            env::remove_var("PURGE_MAX_RUNS_PER_WORKFLOW");
            env::remove_var("PURGE_DRY_RUN");
            env::remove_var("PURGE_INTERVAL_SECS");
            env::remove_var("PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS");
            env::remove_var("SIGNAL_RETENTION_DAYS");
            env::remove_var("ARTIFACT_BACKEND");
            env::remove_var("ARTIFACT_S3_BUCKET");
            env::remove_var("ARTIFACT_S3_REGION");
            env::remove_var("ARTIFACT_S3_ENDPOINT");
            env::remove_var("ARTIFACT_S3_PREFIX");
            env::remove_var("ARTIFACT_GC_INTERVAL_SECS");
            env::remove_var("ARTIFACT_GC_GRACE_DAYS");
            env::remove_var("ARTIFACT_GC_DRY_RUN");
        }
    }

    /// Wipe the environment, then opt into explicit development mode, the only
    /// mode that boots without secrets.
    ///
    /// # Safety
    ///
    /// Must be called while holding `ENV_LOCK`.
    unsafe fn setup_dev() {
        unsafe {
            clear_env();
            env::set_var("IRONFLOW_ENV", "development");
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
    fn unset_env_without_secrets_fails() {
        // Non-regression #149: with nothing set, the server booted on secrets
        // published in the repository (internal API open, JWT forgeable).
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe { clear_env() };

        let err = ServerConfig::from_env().unwrap_err();
        for name in ["JWT_SECRET", "WORKER_TOKEN"] {
            assert!(
                err.errors
                    .iter()
                    .any(|e| e.contains(name) && e.contains("required")),
                "{name} must be required, got {:?}",
                err.errors
            );
        }
        // Outside production the in-memory store stays usable.
        assert!(!err.errors.iter().any(|e| e.contains("DATABASE_URL")));
    }

    #[test]
    fn unset_env_rejects_known_default_and_short_secret() {
        // Non-regression #149, acceptance gate 2.
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe {
            clear_env();
            env::set_var("JWT_SECRET", "ironflow-dev-secret");
            env::set_var("WORKER_TOKEN", "x");
        }

        let err = ServerConfig::from_env().unwrap_err();
        let has = |name: &str, needle: &str| {
            err.errors
                .iter()
                .any(|e| e.contains(name) && e.contains(needle))
        };
        assert!(has("JWT_SECRET", "known development default"), "{err}");
        assert!(has("WORKER_TOKEN", "32 bytes"), "{err}");

        unsafe { clear_env() };
    }

    #[test]
    fn explicit_dev_without_secrets_generates_ephemeral_ones() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe { setup_dev() };

        let first = ServerConfig::from_env().expect("explicit dev boots without secrets");
        let second = ServerConfig::from_env().expect("explicit dev boots without secrets");
        assert!(!first.is_production);
        assert!(first.jwt_secret.len() >= 32 && first.worker_token.len() >= 32);
        assert!(!first.worker_token.starts_with("ironflow-dev-"));
        // A constant in disguise would give the same value on every boot.
        assert_ne!(first.jwt_secret, second.jwt_secret);
        assert_ne!(first.worker_token, second.worker_token);
        assert_ne!(first.jwt_secret, first.worker_token);

        unsafe { clear_env() };
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
            setup_dev();
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
        unsafe { setup_dev() };

        let config = ServerConfig::from_env().unwrap();
        assert_eq!(config.rate_limit_auth, Some(10));
        assert_eq!(config.rate_limit_general, Some(60));
    }

    #[test]
    fn custom_rate_limits() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            setup_dev();
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
            setup_dev();
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
            setup_dev();
            env::set_var("RATE_LIMIT_AUTH", "not-a-number");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.errors.iter().any(|e| e.contains("RATE_LIMIT_AUTH")));

        unsafe { env::remove_var("RATE_LIMIT_AUTH") };
    }

    #[test]
    fn default_purge_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { setup_dev() };

        let config = ServerConfig::from_env().unwrap();
        assert_eq!(config.purge_max_age_days, 90);
        assert_eq!(config.purge_max_runs_per_workflow, 1000);
        assert!(!config.purge_dry_run);
        assert_eq!(config.purge_interval_secs, 86400);
        assert_eq!(config.provider_account_usage_retention_days, 30);
        assert_eq!(config.signal_retention_days, 7);
    }

    #[test]
    fn provider_account_usage_retention_days_from_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe { setup_dev() };
        unsafe { env::set_var("PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS", "7") };
        let config = ServerConfig::from_env().unwrap();
        assert_eq!(config.provider_account_usage_retention_days, 7);

        unsafe { env::set_var("PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS", "0") };
        assert!(ServerConfig::from_env().is_err());
        unsafe { clear_env() };
    }

    #[test]
    fn signal_retention_days_from_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe { setup_dev() };
        unsafe { env::set_var("SIGNAL_RETENTION_DAYS", "3") };
        let config = ServerConfig::from_env().unwrap();
        assert_eq!(config.signal_retention_days, 3);

        unsafe { env::set_var("SIGNAL_RETENTION_DAYS", "0") };
        assert!(ServerConfig::from_env().is_err());
        unsafe { env::set_var("SIGNAL_RETENTION_DAYS", "soon") };
        assert!(ServerConfig::from_env().is_err());
        unsafe { clear_env() };
    }

    #[test]
    fn purger_from_config_uses_env_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            setup_dev();
            env::set_var("SIGNAL_RETENTION_DAYS", "1");
            env::set_var("PURGE_MAX_AGE_DAYS", "30");
            env::set_var("PURGE_MAX_RUNS_PER_WORKFLOW", "50");
            env::set_var("PURGE_DRY_RUN", "true");
            env::set_var("PURGE_INTERVAL_SECS", "3600");
            env::set_var("PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS", "5");
        }
        let config = ServerConfig::from_env().unwrap();
        unsafe { clear_env() };

        let purger = RunPurger::from_config(Arc::new(InMemoryStore::new()), &config);
        assert_eq!(purger.signal_retention_days, 1);
        assert_eq!(purger.usage_retention_days, 5);
        assert_eq!(purger.policy.max_age_days, 30);
        assert_eq!(purger.policy.max_runs_per_workflow, 50);
        assert!(purger.policy.dry_run);
        assert_eq!(purger.interval, Duration::from_secs(3600));
    }

    #[test]
    fn purger_from_config_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { setup_dev() };
        let config = ServerConfig::from_env().unwrap();

        let purger = RunPurger::from_config(Arc::new(InMemoryStore::new()), &config);
        assert_eq!(purger.signal_retention_days, DEFAULT_SIGNAL_RETENTION_DAYS);
        assert_eq!(purger.signal_retention_days, 7);
    }

    #[test]
    fn custom_purge_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            setup_dev();
            env::set_var("PURGE_MAX_AGE_DAYS", "30");
            env::set_var("PURGE_MAX_RUNS_PER_WORKFLOW", "500");
            env::set_var("PURGE_DRY_RUN", "true");
            env::set_var("PURGE_INTERVAL_SECS", "3600");
        }

        let config = ServerConfig::from_env().unwrap();
        assert_eq!(config.purge_max_age_days, 30);
        assert_eq!(config.purge_max_runs_per_workflow, 500);
        assert!(config.purge_dry_run);
        assert_eq!(config.purge_interval_secs, 3600);

        unsafe {
            env::remove_var("PURGE_MAX_AGE_DAYS");
            env::remove_var("PURGE_MAX_RUNS_PER_WORKFLOW");
            env::remove_var("PURGE_DRY_RUN");
            env::remove_var("PURGE_INTERVAL_SECS");
        }
    }

    #[test]
    fn invalid_purge_max_age_days_returns_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            setup_dev();
            env::set_var("PURGE_MAX_AGE_DAYS", "not-a-number");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.errors.iter().any(|e| e.contains("PURGE_MAX_AGE_DAYS")));

        unsafe { env::remove_var("PURGE_MAX_AGE_DAYS") };
    }

    // Strong secrets: >= 32 bytes and not carrying the dev prefix.
    const STRONG_JWT: &str = "prod-jwt-secret-0123456789abcdef0123456789";
    const STRONG_WORKER: &str = "prod-worker-token-0123456789abcdef01234567";

    /// Wipe the environment, then set production mode with the given secrets.
    ///
    /// # Safety
    ///
    /// Mutates process-global environment variables; must be called while
    /// holding `ENV_LOCK` so no other test observes a torn environment.
    unsafe fn setup_prod(jwt: &str, worker: &str) {
        unsafe {
            clear_env();
            env::set_var("IRONFLOW_ENV", "production");
            env::set_var("DATABASE_URL", "postgres://x");
            env::set_var("JWT_SECRET", jwt);
            env::set_var("WORKER_TOKEN", worker);
        }
    }

    #[test]
    fn from_env_production_rejects_known_dev_jwt_secret() {
        let _guard = ENV_LOCK.lock().unwrap();
        // "ironflow-dev-jwt-secret" is the exact value the setup template shipped.
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe { setup_prod("ironflow-dev-jwt-secret", STRONG_WORKER) };

        let err = ServerConfig::from_env().unwrap_err();
        assert!(
            err.errors
                .iter()
                .any(|e| e.contains("JWT_SECRET") && e.contains("known development default")),
            "expected a known-default rejection for JWT_SECRET, got {:?}",
            err.errors
        );
        assert!(
            !err.errors.iter().any(|e| e.contains("WORKER_TOKEN")),
            "a strong WORKER_TOKEN must not be flagged, got {:?}",
            err.errors
        );

        unsafe { clear_env() };
    }

    #[test]
    fn from_env_production_rejects_known_dev_worker_token() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe { setup_prod(STRONG_JWT, "ironflow-dev-worker-token") };

        let err = ServerConfig::from_env().unwrap_err();
        assert!(
            err.errors
                .iter()
                .any(|e| e.contains("WORKER_TOKEN") && e.contains("known development default")),
            "expected a known-default rejection for WORKER_TOKEN, got {:?}",
            err.errors
        );
        assert!(!err.errors.iter().any(|e| e.contains("JWT_SECRET")));

        unsafe { clear_env() };
    }

    #[test]
    fn from_env_production_accepts_strong_secrets() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe { setup_prod(STRONG_JWT, STRONG_WORKER) };

        let config = ServerConfig::from_env().expect("strong secrets should be accepted");
        assert!(config.is_production);
        assert_eq!(config.jwt_secret, STRONG_JWT);
        assert_eq!(config.worker_token, STRONG_WORKER);

        unsafe { clear_env() };
    }

    #[test]
    fn from_env_production_cookie_secure_even_if_insecure_requested() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe {
            setup_prod(STRONG_JWT, STRONG_WORKER);
            env::set_var("IRONFLOW_INSECURE_COOKIES", "true");
        }

        let config = ServerConfig::from_env().expect("production config should load");
        assert!(config.cookie_secure);

        unsafe { clear_env() };
    }

    #[test]
    fn from_env_development_cookie_secure_by_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe { setup_dev() };

        let config = ServerConfig::from_env().expect("dev config should load");
        assert!(!config.is_production);
        assert!(config.cookie_secure);

        unsafe { clear_env() };
    }

    #[test]
    fn from_env_development_insecure_cookies_opt_out() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe {
            setup_dev();
            env::set_var("IRONFLOW_INSECURE_COOKIES", "TRUE");
        }

        let config = ServerConfig::from_env().expect("dev config should load");
        assert!(!config.cookie_secure);

        unsafe { clear_env() };
    }

    #[test]
    fn from_env_explicit_dev_accepts_short_secret_but_not_dev_default() {
        // Explicit dev waives the length check only: a dev-prefixed value is
        // still refused, the same as in production.
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe {
            setup_dev();
            env::set_var("JWT_SECRET", "local-jwt");
            env::set_var("WORKER_TOKEN", "tinytoken");
        }
        let config = ServerConfig::from_env().expect("short secrets are allowed in explicit dev");
        assert_eq!(config.jwt_secret, "local-jwt");
        assert_eq!(config.worker_token, "tinytoken");

        // SAFETY: ENV_LOCK still held.
        unsafe { env::set_var("WORKER_TOKEN", "ironflow-dev-worker-token") };
        let err = ServerConfig::from_env().unwrap_err();
        assert!(
            err.to_string().contains("known development default"),
            "{err}"
        );

        unsafe { clear_env() };
    }

    #[test]
    fn from_env_production_lists_all_insecure_secrets() {
        // Non-regression: the exact state a fresh deploy reached by copying the
        // setup template `.env.example` verbatim and flipping IRONFLOW_ENV to
        // production. It used to boot silently; it must now be refused, with
        // both secrets named.
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: env writes serialized by ENV_LOCK, held above.
        unsafe { setup_prod("ironflow-dev-jwt-secret", "ironflow-dev-worker-token") };

        let err = ServerConfig::from_env().unwrap_err();
        assert!(
            err.errors.iter().any(|e| e.contains("JWT_SECRET")),
            "JWT_SECRET must be listed, got {:?}",
            err.errors
        );
        assert!(
            err.errors.iter().any(|e| e.contains("WORKER_TOKEN")),
            "WORKER_TOKEN must be listed, got {:?}",
            err.errors
        );

        unsafe { clear_env() };
    }
}
