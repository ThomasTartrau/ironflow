//! Proxied secrets: a credential other than the Claude one, injected by the
//! proxy into the requests a pod sends under `/r/<host>/` to an allowlisted
//! host. The pod only ever holds the opaque token.

use std::fmt;
use std::str::FromStr;

use reqwest::header::HeaderName;
use serde::{Deserialize, Serialize};

use super::credential::ProxyCredential;
use super::policy::HOP_BY_HOP;
use super::{AuthProxyError, SECRET_URL_SUFFIX};

/// Longest secret name.
const MAX_SECRET_NAME_LEN: usize = 64;

/// Longest DNS name.
const MAX_HOST_LEN: usize = 253;

/// Longest DNS label.
const MAX_LABEL_LEN: usize = 63;

/// Headers a [`SecretInjection::Header`] may never target, on top of the
/// hop-by-hop ones: the proxy owns them.
const FORBIDDEN_INJECTION_HEADERS: [&str; 4] =
    ["host", "content-length", "cookie", "accept-encoding"];

/// How the proxy presents a proxied secret to the upstream host.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::SecretInjection;
///
/// # fn example() -> Result<(), serde_json::Error> {
/// assert_eq!(serde_json::to_string(&SecretInjection::PrivateToken)?, "\"private_token\"");
/// let basic = SecretInjection::Basic { username: "oauth2".to_string() };
/// assert_eq!(serde_json::to_string(&basic)?, r#"{"basic":{"username":"oauth2"}}"#);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretInjection {
    /// `Authorization: Bearer <secret>` (GitHub, most REST APIs).
    Bearer,
    /// `Private-Token: <secret>` (GitLab API).
    PrivateToken,
    /// `x-api-key: <secret>`.
    XApiKey,
    /// `<header>: <secret>`, for any other header name.
    Header(String),
    /// `Authorization: Basic base64(<username>:<secret>)` (git over HTTPS).
    Basic {
        /// User name sent with the secret, such as `oauth2` for GitLab.
        username: String,
    },
}

impl SecretInjection {
    /// Check the injection can be applied: a [`SecretInjection::Header`]
    /// names a valid header the proxy does not own, a
    /// [`SecretInjection::Basic`] user name is non-empty printable ASCII
    /// without `:`.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::InvalidRequest`] otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::SecretInjection;
    ///
    /// assert!(SecretInjection::Header("X-Vault-Token".to_string()).validate().is_ok());
    /// assert!(SecretInjection::Header("Host".to_string()).validate().is_err());
    /// assert!(SecretInjection::Basic { username: "a:b".to_string() }.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), AuthProxyError> {
        match self {
            Self::Bearer | Self::PrivateToken | Self::XApiKey => Ok(()),
            Self::Header(name) => {
                let parsed = HeaderName::from_str(name)
                    .map_err(|e| invalid(format!("injection header {name:?}: {e}")))?;
                let lower = parsed.as_str();
                if HOP_BY_HOP.contains(&lower) || FORBIDDEN_INJECTION_HEADERS.contains(&lower) {
                    return Err(invalid(format!(
                        "injection header {name:?} is managed by the proxy"
                    )));
                }
                Ok(())
            }
            Self::Basic { username } => {
                if username.is_empty()
                    || !username.bytes().all(|b| b.is_ascii_graphic() && b != b':')
                {
                    return Err(invalid(
                        "basic username must be printable ASCII without ':'".to_string(),
                    ));
                }
                Ok(())
            }
        }
    }
}

/// One entry of a secret's host allowlist: an exact host name
/// (`api.github.com`) or a leading wildcard (`*.example.com`, matching any
/// subdomain but not `example.com` itself). Stored lowercased. No scheme,
/// port, path, IP literal or regular expression.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::HostPattern;
///
/// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
/// let exact = HostPattern::parse("API.GitHub.com")?;
/// assert!(exact.matches("api.github.com"));
/// assert!(!exact.matches("api.github.com.attacker.net"));
///
/// let wildcard = HostPattern::parse("*.example.com")?;
/// assert!(wildcard.matches("a.b.example.com"));
/// assert!(!wildcard.matches("example.com"));
/// assert!(HostPattern::parse("10.0.0.1").is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HostPattern(String);

impl HostPattern {
    /// Parse an allowlist entry.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::InvalidRequest`] when `raw` is neither a DNS
    /// name of at least two labels nor `*.` followed by one.
    ///
    /// # Examples
    ///
    /// See [`HostPattern`].
    pub fn parse(raw: &str) -> Result<Self, AuthProxyError> {
        let pattern = raw.trim().to_ascii_lowercase();
        let name = pattern.strip_prefix("*.").unwrap_or(&pattern);
        if !is_dns_name(name) {
            return Err(invalid(format!(
                "host {raw:?} must be a host name or *.domain, without scheme, port or path"
            )));
        }
        Ok(Self(pattern))
    }

    /// The pattern, lowercased.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::HostPattern;
    ///
    /// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// assert_eq!(HostPattern::parse("GitLab.com")?.as_str(), "gitlab.com");
    /// # Ok(())
    /// # }
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the pattern starts with `*.`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::HostPattern;
    ///
    /// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// assert!(HostPattern::parse("*.example.com")?.is_wildcard());
    /// assert!(!HostPattern::parse("example.com")?.is_wildcard());
    /// # Ok(())
    /// # }
    /// ```
    pub fn is_wildcard(&self) -> bool {
        self.0.starts_with("*.")
    }

    /// Whether `host` (any case) is allowed by this pattern. An invalid host
    /// name never matches.
    ///
    /// # Examples
    ///
    /// See [`HostPattern`].
    pub fn matches(&self, host: &str) -> bool {
        if !is_valid_request_host(host) {
            return false;
        }
        let host = host.to_ascii_lowercase();
        match self.0.strip_prefix('*') {
            // `suffix` keeps its leading dot: `evilexample.com` never matches.
            Some(suffix) => host.len() > suffix.len() && host.ends_with(suffix),
            None => host == self.0,
        }
    }
}

impl TryFrom<String> for HostPattern {
    type Error = AuthProxyError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(&raw)
    }
}

impl From<HostPattern> for String {
    fn from(pattern: HostPattern) -> Self {
        pattern.0
    }
}

impl fmt::Display for HostPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `host` (any case) is a DNS name of at least two labels the proxy
/// may relay to: no port, user info, IP literal or wildcard.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::is_valid_request_host;
///
/// assert!(is_valid_request_host("api.github.com"));
/// assert!(!is_valid_request_host("api.github.com:8443"));
/// assert!(!is_valid_request_host("127.0.0.1"));
/// ```
pub fn is_valid_request_host(host: &str) -> bool {
    is_dns_name(&host.to_ascii_lowercase())
}

fn is_dns_name(name: &str) -> bool {
    let labels: Vec<&str> = name.split('.').collect();
    name.len() <= MAX_HOST_LEN
        && labels.len() >= 2
        && labels.iter().all(|label| is_dns_label(label))
        && labels
            .last()
            .is_some_and(|tld| !tld.bytes().all(|b| b.is_ascii_digit()))
}

fn is_dns_label(label: &str) -> bool {
    (1..=MAX_LABEL_LEN).contains(&label.len())
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

fn validate_secret_name(name: &str) -> Result<(), AuthProxyError> {
    if name.is_empty()
        || name.len() > MAX_SECRET_NAME_LEN
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        return Err(invalid(format!(
            "secret name {name:?} must be 1 to {MAX_SECRET_NAME_LEN} characters of [A-Za-z0-9_.-]"
        )));
    }
    Ok(())
}

/// A proxied secret as the proxy holds it: its name (logs and metrics), its
/// value, how it is injected and the hosts it may reach. Its [`Debug`]
/// output never shows the value.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{HostPattern, SecretCredential, SecretInjection};
///
/// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
/// let secret = SecretCredential::new(
///     "GITHUB_TOKEN".to_string(),
///     "ghp_x".to_string(),
///     SecretInjection::Bearer,
///     vec![HostPattern::parse("api.github.com")?],
/// );
/// secret.validate()?;
/// assert!(secret.allows_host("api.github.com"));
/// assert!(!secret.allows_host("github.com"));
/// assert!(!format!("{secret:?}").contains("ghp_x"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretCredential {
    name: String,
    value: String,
    injection: SecretInjection,
    hosts: Vec<HostPattern>,
}

impl SecretCredential {
    /// Wrap a secret value.
    ///
    /// # Examples
    ///
    /// See [`SecretCredential`].
    pub fn new(
        name: String,
        value: String,
        injection: SecretInjection,
        hosts: Vec<HostPattern>,
    ) -> Self {
        Self {
            name,
            value,
            injection,
            hosts,
        }
    }

    /// Name of the secret, for logs and metrics.
    ///
    /// # Examples
    ///
    /// See [`SecretCredential`].
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The raw secret value. Never log it.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::{HostPattern, SecretCredential, SecretInjection};
    ///
    /// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// let hosts = vec![HostPattern::parse("gitlab.com")?];
    /// let secret = SecretCredential::new("GL".to_string(), "v".to_string(), SecretInjection::PrivateToken, hosts);
    /// assert_eq!(secret.expose(), "v");
    /// # Ok(())
    /// # }
    /// ```
    pub fn expose(&self) -> &str {
        &self.value
    }

    /// How the secret is injected.
    ///
    /// # Examples
    ///
    /// See [`SecretCredential::expose`].
    pub fn injection(&self) -> &SecretInjection {
        &self.injection
    }

    /// The host allowlist.
    ///
    /// # Examples
    ///
    /// See [`SecretCredential::expose`].
    pub fn hosts(&self) -> &[HostPattern] {
        &self.hosts
    }

    /// Whether one allowlist entry matches `host`.
    ///
    /// # Examples
    ///
    /// See [`SecretCredential`].
    pub fn allows_host(&self, host: &str) -> bool {
        self.hosts.iter().any(|pattern| pattern.matches(host))
    }

    /// Check the secret can be issued: a name of 1 to 64 `[A-Za-z0-9_.-]`, a
    /// non-empty printable ASCII value, at least one host and a valid
    /// injection.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::InvalidRequest`] otherwise. The message
    /// never carries the value.
    ///
    /// # Examples
    ///
    /// See [`SecretCredential`].
    pub fn validate(&self) -> Result<(), AuthProxyError> {
        validate_secret_name(&self.name)?;
        if self.value.is_empty() {
            return Err(invalid(format!("secret {} is empty", self.name)));
        }
        if !self.value.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(invalid(format!(
                "secret {} holds characters a header cannot carry",
                self.name
            )));
        }
        if self.hosts.is_empty() {
            return Err(invalid(format!(
                "secret {} has an empty host allowlist",
                self.name
            )));
        }
        self.injection.validate()
    }
}

impl fmt::Debug for SecretCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretCredential")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("injection", &self.injection)
            .field("hosts", &self.hosts)
            .finish()
    }
}

/// The credential a grant carries: the Claude credential relayed to the
/// Anthropic API under `/v1/`, or a proxied secret relayed under `/r/`.
///
/// On the wire it is untagged: `{"kind", "value"}` is a Claude credential,
/// `{"name", "value", "injection", "hosts"}` a secret. Its [`Debug`] output
/// never shows the value.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{CredentialKind, GrantCredential, ProxyCredential};
///
/// # fn example() -> Result<(), serde_json::Error> {
/// let credential: GrantCredential =
///     serde_json::from_str(r#"{"kind":"api_key","value":"sk-ant-api03-x"}"#)?;
/// assert!(matches!(credential, GrantCredential::Claude(_)));
/// let from: GrantCredential =
///     ProxyCredential::new(CredentialKind::OauthToken, "t".to_string()).into();
/// assert_eq!(from.expose(), "t");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GrantCredential {
    /// The Claude credential.
    Claude(ProxyCredential),
    /// A proxied secret.
    Secret(SecretCredential),
}

impl GrantCredential {
    /// The raw credential value. Never log it.
    ///
    /// # Examples
    ///
    /// See [`GrantCredential`].
    pub fn expose(&self) -> &str {
        match self {
            Self::Claude(credential) => credential.expose(),
            Self::Secret(secret) => secret.expose(),
        }
    }
}

impl From<ProxyCredential> for GrantCredential {
    fn from(credential: ProxyCredential) -> Self {
        Self::Claude(credential)
    }
}

impl From<SecretCredential> for GrantCredential {
    fn from(secret: SecretCredential) -> Self {
        Self::Secret(secret)
    }
}

/// A secret the workflow author hands to the agent provider or step: the
/// pod gets `<env>` set to an opaque token and `<env>_URL` set to
/// `<proxy>/r`, never `value`. Its [`Debug`] output never shows the value.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{ProxiedSecret, SecretInjection};
///
/// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
/// let secret = ProxiedSecret {
///     name: "GITHUB_TOKEN".to_string(),
///     env: "GITHUB_TOKEN".to_string(),
///     value: "ghp_x".to_string(),
///     injection: SecretInjection::Bearer,
///     hosts: vec!["api.github.com".to_string()],
/// };
/// secret.validate()?;
/// assert_eq!(secret.url_env(), "GITHUB_TOKEN_URL");
/// assert!(!format!("{secret:?}").contains("ghp_x"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct ProxiedSecret {
    /// Name of the secret in proxy logs and metrics.
    pub name: String,
    /// Environment variable receiving the opaque token in the pod.
    pub env: String,
    /// The real secret. Never reaches the pod.
    pub value: String,
    /// How the proxy injects the secret.
    pub injection: SecretInjection,
    /// Host allowlist: exact host names or `*.domain`.
    pub hosts: Vec<String>,
}

impl ProxiedSecret {
    /// Check everything but the value: `env` matches `[A-Za-z_][A-Za-z0-9_]*`,
    /// the name is valid, every host parses as a [`HostPattern`], there is at
    /// least one, and the injection is valid. The value is checked when the
    /// proxy issues the token.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::InvalidRequest`] otherwise. The message
    /// never carries the value.
    ///
    /// # Examples
    ///
    /// See [`ProxiedSecret`].
    pub fn validate(&self) -> Result<(), AuthProxyError> {
        let mut env = self.env.bytes();
        let valid_env = env
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && env.all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if !valid_env {
            return Err(invalid(format!(
                "proxied secret env {:?} must match [A-Za-z_][A-Za-z0-9_]*",
                self.env
            )));
        }
        validate_secret_name(&self.name)?;
        if self.hosts.is_empty() {
            return Err(invalid(format!(
                "secret {} has an empty host allowlist",
                self.name
            )));
        }
        self.parsed_hosts()?;
        self.injection.validate()
    }

    /// The environment variable receiving the relay base URL: `<env>_URL`.
    ///
    /// # Examples
    ///
    /// See [`ProxiedSecret`].
    pub fn url_env(&self) -> String {
        format!("{}{SECRET_URL_SUFFIX}", self.env)
    }

    /// The credential the worker asks the proxy to hold.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::InvalidRequest`] when a host does not parse.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::auth_proxy::{ProxiedSecret, SecretInjection};
    ///
    /// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
    /// let secret = ProxiedSecret {
    ///     name: "GITLAB_TOKEN".to_string(),
    ///     env: "GITLAB_TOKEN".to_string(),
    ///     value: "glpat-x".to_string(),
    ///     injection: SecretInjection::PrivateToken,
    ///     hosts: vec!["GitLab.com".to_string()],
    /// };
    /// let credential = secret.to_credential()?;
    /// assert_eq!(credential.hosts()[0].as_str(), "gitlab.com");
    /// # Ok(())
    /// # }
    /// ```
    pub fn to_credential(&self) -> Result<SecretCredential, AuthProxyError> {
        Ok(SecretCredential::new(
            self.name.clone(),
            self.value.clone(),
            self.injection.clone(),
            self.parsed_hosts()?,
        ))
    }

    fn parsed_hosts(&self) -> Result<Vec<HostPattern>, AuthProxyError> {
        self.hosts
            .iter()
            .map(|host| HostPattern::parse(host))
            .collect()
    }
}

impl fmt::Debug for ProxiedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxiedSecret")
            .field("name", &self.name)
            .field("env", &self.env)
            .field("value", &"<redacted>")
            .field("injection", &self.injection)
            .field("hosts", &self.hosts)
            .finish()
    }
}

fn invalid(message: String) -> AuthProxyError {
    AuthProxyError::InvalidRequest(message)
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::super::CredentialKind;
    use super::*;

    fn pattern(raw: &str) -> HostPattern {
        HostPattern::parse(raw).unwrap()
    }

    fn secret(injection: SecretInjection, hosts: &[&str]) -> SecretCredential {
        SecretCredential::new(
            "GITHUB_TOKEN".to_string(),
            "ghp_test".to_string(),
            injection,
            hosts.iter().map(|h| pattern(h)).collect(),
        )
    }

    fn proxied(env: &str) -> ProxiedSecret {
        ProxiedSecret {
            name: "GITHUB_TOKEN".to_string(),
            env: env.to_string(),
            value: "ghp_test".to_string(),
            injection: SecretInjection::Bearer,
            hosts: vec!["api.github.com".to_string()],
        }
    }

    #[test]
    fn exact_pattern_matches_only_that_host() {
        let exact = pattern("gitlab.com");
        assert!(exact.matches("gitlab.com"));
        assert!(!exact.matches("gitlab.com.attacker.net"));
        assert!(!exact.matches("evilgitlab.com"));
        assert!(!exact.matches("api.gitlab.com"));
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(pattern("api.github.com").matches("API.GitHub.com"));
        assert!(pattern("API.GitHub.com").matches("api.github.com"));
        assert_eq!(pattern("API.GitHub.com").as_str(), "api.github.com");
    }

    #[test]
    fn wildcard_matches_subdomains_not_apex() {
        let wildcard = pattern("*.example.com");
        assert!(wildcard.is_wildcard());
        assert!(wildcard.matches("a.example.com"));
        assert!(wildcard.matches("a.b.example.com"));
        assert!(!wildcard.matches("example.com"));
        assert!(!wildcard.matches("evilexample.com"));
        assert!(!pattern("*.gitlab.com").matches("gitlab.com.attacker.net"));
    }

    #[test]
    fn invalid_host_never_matches() {
        assert!(!pattern("*.example.com").matches("a..example.com"));
        assert!(!pattern("gitlab.com").matches("gitlab.com:443"));
    }

    #[test]
    fn parse_refuses_invalid_patterns() {
        let long_label = format!("{}.com", "a".repeat(64));
        for raw in [
            "",
            "*",
            "*.com",
            "gitlab.com:443",
            "https://gitlab.com",
            "gitlab.com/x",
            "10.0.0.1",
            "a..b",
            "-a.com",
            "a-.com",
            "a*.b.com",
            "*.*.b.com",
            ".*github.com",
            "ex\u{e4}mple.com",
            "user@gitlab.com",
            "com",
            long_label.as_str(),
        ] {
            assert!(HostPattern::parse(raw).is_err(), "{raw:?} must be refused");
        }
        let max_label = format!("{}.com", "a".repeat(63));
        assert!(HostPattern::parse(&max_label).is_ok());
        assert_eq!(pattern("  GitLab.com ").as_str(), "gitlab.com");
    }

    #[test]
    fn request_host_rules() {
        assert!(is_valid_request_host("api.github.com"));
        assert!(is_valid_request_host("API.GITHUB.COM"));
        for host in [
            "",
            "localhost",
            "127.0.0.1",
            "*.github.com",
            "a.com:1",
            "a b.com",
        ] {
            assert!(!is_valid_request_host(host), "{host:?}");
        }
    }

    #[test]
    fn secret_validate_accepts_valid_secret() {
        for injection in [
            SecretInjection::Bearer,
            SecretInjection::PrivateToken,
            SecretInjection::XApiKey,
            SecretInjection::Header("X-Vault-Token".to_string()),
            SecretInjection::Basic {
                username: "oauth2".to_string(),
            },
        ] {
            secret(injection, &["api.github.com"]).validate().unwrap();
        }
    }

    #[test]
    fn secret_validate_refuses_invalid_secrets() {
        assert!(secret(SecretInjection::Bearer, &[]).validate().is_err());

        let mut empty = secret(SecretInjection::Bearer, &["api.github.com"]);
        empty.value = String::new();
        assert!(empty.validate().is_err());

        let mut spaced = secret(SecretInjection::Bearer, &["api.github.com"]);
        spaced.value = "ghp test".to_string();
        let err = spaced.validate().unwrap_err().to_string();
        assert!(!err.contains("ghp test"), "{err}");

        let mut bad_name = secret(SecretInjection::Bearer, &["api.github.com"]);
        bad_name.name = "a b".to_string();
        assert!(bad_name.validate().is_err());
        bad_name.name = "a".repeat(65);
        assert!(bad_name.validate().is_err());

        for injection in [
            SecretInjection::Header("bad header".to_string()),
            SecretInjection::Header("Host".to_string()),
            SecretInjection::Header("transfer-encoding".to_string()),
            SecretInjection::Header("cookie".to_string()),
            SecretInjection::Basic {
                username: "a:b".to_string(),
            },
            SecretInjection::Basic {
                username: String::new(),
            },
        ] {
            assert!(
                secret(injection.clone(), &["api.github.com"])
                    .validate()
                    .is_err(),
                "{injection:?}"
            );
        }
    }

    #[test]
    fn injection_serde_shapes() {
        assert_eq!(to_value(SecretInjection::Bearer).unwrap(), json!("bearer"));
        assert_eq!(
            to_value(SecretInjection::XApiKey).unwrap(),
            json!("x_api_key")
        );
        assert_eq!(
            to_value(SecretInjection::Header("X-Foo".to_string())).unwrap(),
            json!({"header": "X-Foo"})
        );
        let basic: SecretInjection = from_value(json!({"basic": {"username": "oauth2"}})).unwrap();
        assert_eq!(
            basic,
            SecretInjection::Basic {
                username: "oauth2".to_string()
            }
        );
    }

    #[test]
    fn secret_json_round_trip() {
        let original = secret(
            SecretInjection::Basic {
                username: "oauth2".to_string(),
            },
            &["gitlab.com", "*.example.com"],
        );
        let value = to_value(&original).unwrap();
        assert_eq!(value["hosts"], json!(["gitlab.com", "*.example.com"]));
        let back: SecretCredential = from_value(value).unwrap();
        assert_eq!(back.name(), "GITHUB_TOKEN");
        assert_eq!(back.expose(), "ghp_test");
        assert_eq!(back.injection(), original.injection());
        assert_eq!(back.hosts(), original.hosts());
    }

    #[test]
    fn secret_json_with_invalid_host_fails() {
        let result: Result<SecretCredential, _> = from_value(json!({
            "name": "X", "value": "v", "injection": "bearer", "hosts": [".*github.com"]
        }));
        assert!(result.is_err());
    }

    #[test]
    fn grant_credential_is_untagged() {
        let claude: GrantCredential =
            from_value(json!({"kind": "oauth_token", "value": "sk-ant-oat01-x"})).unwrap();
        match claude {
            GrantCredential::Claude(c) => assert_eq!(c.kind(), CredentialKind::OauthToken),
            other => panic!("expected Claude, got {other:?}"),
        }

        let secret: GrantCredential = from_value(json!({
            "name": "GITHUB_TOKEN",
            "value": "ghp_test",
            "injection": "bearer",
            "hosts": ["api.github.com"]
        }))
        .unwrap();
        match secret {
            GrantCredential::Secret(s) => assert_eq!(s.name(), "GITHUB_TOKEN"),
            other => panic!("expected Secret, got {other:?}"),
        }

        let both: Result<GrantCredential, _> = from_value(json!({
            "kind": "oauth_token",
            "value": "x",
            "hosts": ["api.github.com"]
        }));
        assert!(both.is_err());
    }

    #[test]
    fn grant_credential_serializes_without_tag() {
        let value = to_value(GrantCredential::from(secret(
            SecretInjection::Bearer,
            &["api.github.com"],
        )))
        .unwrap();
        assert_eq!(value["name"], "GITHUB_TOKEN");
        assert_eq!(value["injection"], "bearer");
        assert!(value.get("kind").is_none());
    }

    #[test]
    fn debug_never_shows_value() {
        let credential = secret(SecretInjection::Bearer, &["api.github.com"]);
        let grant = GrantCredential::from(credential.clone());
        for debug in [
            format!("{credential:?}"),
            format!("{grant:?}"),
            format!("{:?}", proxied("GITHUB_TOKEN")),
        ] {
            assert!(!debug.contains("ghp_test"), "{debug}");
            assert!(debug.contains("<redacted>"), "{debug}");
        }
    }

    #[test]
    fn proxied_secret_validate_checks_env() {
        proxied("GITHUB_TOKEN").validate().unwrap();
        proxied("_x1").validate().unwrap();
        for env in ["1X", "A-B", "", "A B"] {
            assert!(proxied(env).validate().is_err(), "{env:?}");
        }
    }

    #[test]
    fn proxied_secret_validate_checks_hosts_and_injection() {
        let mut no_hosts = proxied("X");
        no_hosts.hosts.clear();
        assert!(no_hosts.validate().is_err());

        let mut bad_host = proxied("X");
        bad_host.hosts = vec!["https://api.github.com".to_string()];
        assert!(bad_host.validate().is_err());

        let mut bad_injection = proxied("X");
        bad_injection.injection = SecretInjection::Header("Host".to_string());
        assert!(bad_injection.validate().is_err());
    }

    #[test]
    fn proxied_secret_to_credential() {
        let credential = proxied("GITHUB_TOKEN").to_credential().unwrap();
        assert_eq!(credential.name(), "GITHUB_TOKEN");
        assert_eq!(credential.expose(), "ghp_test");
        assert!(credential.allows_host("api.github.com"));
        assert_eq!(proxied("GH").url_env(), "GH_URL");
    }
}
