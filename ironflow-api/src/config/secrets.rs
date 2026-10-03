//! Policy for the server's shared secrets, `JWT_SECRET` and `WORKER_TOKEN`.
//!
//! No usable secret is compiled into the binary. A secret is either provided
//! and valid, or generated at random for this process in explicit development
//! mode, or the server refuses to boot.

use hex::encode;
use rand::{Rng, rng};

/// Prefix shared by every development secret ever published in the
/// repository (`ironflow-dev-secret`, `ironflow-dev-worker-token`, the setup
/// template's `ironflow-dev-jwt-secret`). Refused in every mode.
pub(super) const DEV_SECRET_PREFIX: &str = "ironflow-dev-";

/// Minimum accepted length, in bytes, outside explicit development mode. A
/// shorter value is trivially brute-forced against an HS256 signature.
pub(super) const MIN_SECRET_BYTES: usize = 32;

/// Random bytes behind a generated secret, hex-encoded to twice as many chars.
const GENERATED_SECRET_BYTES: usize = 32;

/// How strictly secrets are checked, derived from `IRONFLOW_ENV`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SecretMode {
    /// `IRONFLOW_ENV=production`.
    Production,
    /// `IRONFLOW_ENV` unset or set to anything but `production` or
    /// `development`: secrets required, as in production.
    Strict,
    /// `IRONFLOW_ENV=development`, set explicitly: a missing secret is
    /// generated and the length check is waived.
    Development,
}

impl SecretMode {
    /// Read the mode from the raw `IRONFLOW_ENV` value, case-insensitively.
    pub(super) fn from_ironflow_env(value: Option<&str>) -> Self {
        match value {
            Some(v) if v.eq_ignore_ascii_case("production") => Self::Production,
            Some(v) if v.eq_ignore_ascii_case("development") => Self::Development,
            _ => Self::Strict,
        }
    }
}

/// A secret accepted for this process.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ResolvedSecret {
    /// Read from the environment and valid for the mode.
    Provided(String),
    /// Absent in explicit development mode: random, lost when the process
    /// stops.
    Generated(String),
}

/// Validate the secret `name` read from the environment, or generate one.
///
/// Returns the operator-facing message on rejection, so the caller can collect
/// every configuration error before refusing to boot.
pub(super) fn resolve_secret(
    name: &str,
    value: Option<String>,
    mode: SecretMode,
) -> Result<ResolvedSecret, String> {
    let Some(value) = value else {
        return match mode {
            SecretMode::Development => Ok(ResolvedSecret::Generated(generate_secret())),
            SecretMode::Production => Err(format!("{name} is required in production")),
            SecretMode::Strict => Err(format!(
                "{name} is required: generate one with `openssl rand -hex 32`, \
                 or set IRONFLOW_ENV=development to use an ephemeral one"
            )),
        };
    };

    // An empty secret would let `Authorization: Bearer ` through.
    if value.is_empty() {
        return Err(format!("{name} must not be empty"));
    }
    if value.starts_with(DEV_SECRET_PREFIX) {
        return Err(format!(
            "{name} must not use a known development default (`{DEV_SECRET_PREFIX}` prefix)"
        ));
    }
    if mode != SecretMode::Development && value.len() < MIN_SECRET_BYTES {
        return Err(format!(
            "{name} must be at least {MIN_SECRET_BYTES} bytes, got {}",
            value.len()
        ));
    }
    Ok(ResolvedSecret::Provided(value))
}

fn generate_secret() -> String {
    let mut bytes = [0u8; GENERATED_SECRET_BYTES];
    rng().fill(&mut bytes);
    encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_MODES: [SecretMode; 3] = [
        SecretMode::Production,
        SecretMode::Strict,
        SecretMode::Development,
    ];
    const STRONG: &str = "prod-jwt-secret-0123456789abcdef0123456789";

    fn resolve(value: Option<&str>, mode: SecretMode) -> Result<ResolvedSecret, String> {
        resolve_secret("JWT_SECRET", value.map(str::to_string), mode)
    }

    #[test]
    fn mode_from_ironflow_env() {
        assert_eq!(
            SecretMode::from_ironflow_env(Some("production")),
            SecretMode::Production
        );
        assert_eq!(
            SecretMode::from_ironflow_env(Some("PRODUCTION")),
            SecretMode::Production
        );
        assert_eq!(
            SecretMode::from_ironflow_env(Some("Development")),
            SecretMode::Development
        );
        // Only the exact word relaxes the checks: a typo or a stage name must
        // not silently open the development path.
        for raw in [None, Some(""), Some("dev"), Some("staging")] {
            assert_eq!(
                SecretMode::from_ironflow_env(raw),
                SecretMode::Strict,
                "{raw:?}"
            );
        }
    }

    #[test]
    fn missing_secret_is_required_outside_development() {
        let prod = resolve(None, SecretMode::Production).unwrap_err();
        assert!(
            prod.contains("JWT_SECRET is required in production"),
            "{prod}"
        );

        let strict = resolve(None, SecretMode::Strict).unwrap_err();
        assert!(strict.contains("JWT_SECRET is required"), "{strict}");
        assert!(strict.contains("IRONFLOW_ENV=development"), "{strict}");
    }

    #[test]
    fn missing_secret_is_generated_in_development() {
        let Ok(ResolvedSecret::Generated(first)) = resolve(None, SecretMode::Development) else {
            panic!("a missing secret must be generated in development");
        };
        let Ok(ResolvedSecret::Generated(second)) = resolve(None, SecretMode::Development) else {
            panic!("a missing secret must be generated in development");
        };

        assert_eq!(first.len(), GENERATED_SECRET_BYTES * 2);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(first.len() >= MIN_SECRET_BYTES);
        assert!(!first.starts_with(DEV_SECRET_PREFIX));
        assert_ne!(first, second);
    }

    #[test]
    fn known_dev_defaults_are_rejected_in_every_mode() {
        for mode in ALL_MODES {
            for value in [
                "ironflow-dev-secret",
                "ironflow-dev-worker-token",
                "ironflow-dev-jwt-secret",
                // Long enough to pass the length check: the prefix alone refuses it.
                "ironflow-dev-0123456789abcdef0123456789abcdef",
            ] {
                let err = resolve(Some(value), mode).unwrap_err();
                assert!(
                    err.contains("JWT_SECRET") && err.contains("known development default"),
                    "{mode:?} {value}: {err}"
                );
            }
        }
    }

    #[test]
    fn empty_secret_is_rejected_in_every_mode() {
        for mode in ALL_MODES {
            let err = resolve(Some(""), mode).unwrap_err();
            assert!(err.contains("must not be empty"), "{mode:?}: {err}");
        }
    }

    #[test]
    fn short_secret_is_rejected_outside_development() {
        for mode in [SecretMode::Production, SecretMode::Strict] {
            let err = resolve(Some("shortsecret"), mode).unwrap_err();
            assert!(err.contains("32 bytes, got 11"), "{mode:?}: {err}");
        }
        assert_eq!(
            resolve(Some("shortsecret"), SecretMode::Development),
            Ok(ResolvedSecret::Provided("shortsecret".to_string()))
        );
    }

    #[test]
    fn length_boundary() {
        // Exactly 32 bytes is accepted, 31 is rejected: guards `<` against `<=`.
        let exactly_32 = "0123456789abcdef0123456789abcdef";
        let just_under = &exactly_32[..31];
        for mode in [SecretMode::Production, SecretMode::Strict] {
            assert_eq!(
                resolve(Some(exactly_32), mode),
                Ok(ResolvedSecret::Provided(exactly_32.to_string()))
            );
            assert!(resolve(Some(just_under), mode).is_err(), "{mode:?}");
        }
    }

    #[test]
    fn strong_secret_is_provided_unchanged_in_every_mode() {
        for mode in ALL_MODES {
            assert_eq!(
                resolve(Some(STRONG), mode),
                Ok(ResolvedSecret::Provided(STRONG.to_string()))
            );
        }
    }

    #[test]
    fn non_ascii_secret_length_is_counted_in_bytes() {
        // 16 two-byte chars = 32 bytes: accepted on byte length, not char count.
        let accented = "é".repeat(16);
        assert_eq!(
            resolve(Some(&accented), SecretMode::Strict),
            Ok(ResolvedSecret::Provided(accented.clone()))
        );
    }
}
