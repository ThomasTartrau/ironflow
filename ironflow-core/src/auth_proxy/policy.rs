//! What the proxy accepts from a pod and what it forwards each way.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::Method;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use super::OAUTH_BETA;
use super::credential::{CredentialKind, ProxyCredential};
use super::secret::{SecretCredential, SecretInjection};

const X_API_KEY: &str = "x-api-key";
const PRIVATE_TOKEN: &str = "private-token";
const ANTHROPIC_BETA: &str = "anthropic-beta";

/// Headers that only make sense on one connection.
pub(super) const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Request headers never forwarded upstream on top of [`HOP_BY_HOP`]: the
/// opaque token (whichever header carries it), the pod's host, the body
/// length (recomputed), the encoding negotiation (the proxy relays identity
/// bodies) and cookies.
const REQUEST_DROPPED: [&str; 7] = [
    "authorization",
    X_API_KEY,
    PRIVATE_TOKEN,
    "host",
    "content-length",
    "accept-encoding",
    "cookie",
];

fn is_hop_by_hop(name: &HeaderName) -> bool {
    HOP_BY_HOP.contains(&name.as_str())
}

/// The opaque token a pod presents, in this order: `Authorization: Bearer
/// <token>` (scheme case-insensitive), `x-api-key`, `Private-Token`, then the
/// password of `Authorization: Basic base64(<user>:<token>)` (what git sends).
/// `None` when absent or empty.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::extract_opaque_token;
/// use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
///
/// let mut headers = HeaderMap::new();
/// headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer ifap_abc"));
/// assert_eq!(extract_opaque_token(&headers).as_deref(), Some("ifap_abc"));
/// assert_eq!(extract_opaque_token(&HeaderMap::new()), None);
/// ```
pub fn extract_opaque_token(headers: &HeaderMap) -> Option<String> {
    let scheme_value = |wanted: &str| {
        headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case(wanted))
            .map(|(_, rest)| rest.trim())
    };
    let plain = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
    };
    let basic = || {
        scheme_value("basic")
            .and_then(|encoded| STANDARD.decode(encoded).ok())
            .and_then(|decoded| String::from_utf8(decoded).ok())
            .and_then(|pair| {
                pair.split_once(':')
                    .map(|(_, password)| password.trim().to_string())
            })
            .filter(|token| !token.is_empty())
    };
    scheme_value("bearer")
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .or_else(|| plain(X_API_KEY))
        .or_else(|| plain(PRIVATE_TOKEN))
        .or_else(basic)
}

/// Whether the proxy relays `path` at all: it starts with `/`, only holds
/// `[A-Za-z0-9/_.-]` (so no percent-encoding), and has no `..` segment and
/// no `//`.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::is_relay_path;
///
/// assert!(is_relay_path("/group/project.git/info/refs"));
/// assert!(!is_relay_path("/a/../b"));
/// assert!(!is_relay_path("/a%2Fb"));
/// ```
pub fn is_relay_path(path: &str) -> bool {
    path.starts_with('/')
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
        && !path.contains("//")
        && !path.split('/').any(|segment| segment == "..")
}

/// Whether the proxy relays `path` to the Anthropic API: it starts with
/// `/v1/` and passes [`is_relay_path`].
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::is_allowed_path;
///
/// assert!(is_allowed_path("/v1/messages"));
/// assert!(!is_allowed_path("/admin/v1/tokens"));
/// assert!(!is_allowed_path("/v1/%2e%2e/x"));
/// ```
pub fn is_allowed_path(path: &str) -> bool {
    path.starts_with("/v1/") && is_relay_path(path)
}

/// Whether the proxy relays `method` to the Anthropic API: GET and POST only.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::is_allowed_method;
/// use reqwest::Method;
///
/// assert!(is_allowed_method(&Method::POST));
/// assert!(!is_allowed_method(&Method::PUT));
/// ```
pub fn is_allowed_method(method: &Method) -> bool {
    method == Method::GET || method == Method::POST
}

/// Whether the proxy relays `method` for a proxied secret: GET, HEAD, POST,
/// PUT, PATCH and DELETE (git push and REST APIs need more than GET/POST).
/// CONNECT, TRACE and OPTIONS are refused.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::is_relay_method;
/// use reqwest::Method;
///
/// assert!(is_relay_method(&Method::PUT));
/// assert!(!is_relay_method(&Method::CONNECT));
/// ```
pub fn is_relay_method(method: &Method) -> bool {
    [
        Method::GET,
        Method::HEAD,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ]
    .contains(method)
}

/// Headers sent upstream: the pod's headers minus the opaque token,
/// hop-by-hop headers, `host`, `content-length`, `accept-encoding` and
/// `cookie`, plus the real credential (`authorization: Bearer` and the OAuth
/// `anthropic-beta` flag for an OAuth token, `x-api-key` for an API key),
/// marked sensitive.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{CredentialKind, ProxyCredential, upstream_headers};
/// use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
///
/// let mut incoming = HeaderMap::new();
/// incoming.insert(AUTHORIZATION, HeaderValue::from_static("Bearer ifap_abc"));
/// let credential = ProxyCredential::new(CredentialKind::ApiKey, "sk-ant-api03-x".to_string());
/// let headers = upstream_headers(&incoming, &credential);
/// assert!(headers.get(AUTHORIZATION).is_none());
/// assert!(headers.get("x-api-key").is_some_and(|v| v.is_sensitive()));
/// ```
pub fn upstream_headers(incoming: &HeaderMap, credential: &ProxyCredential) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in incoming {
        if !is_hop_by_hop(name) && !REQUEST_DROPPED.contains(&name.as_str()) {
            headers.append(name.clone(), value.clone());
        }
    }

    match credential.kind() {
        CredentialKind::OauthToken => {
            if let Some(value) = sensitive_value(&format!("Bearer {}", credential.expose())) {
                headers.insert(AUTHORIZATION, value);
            }
            let beta = with_oauth_beta(&headers);
            headers.insert(ANTHROPIC_BETA, beta);
        }
        CredentialKind::ApiKey => {
            if let Some(value) = sensitive_value(credential.expose()) {
                headers.insert(X_API_KEY, value);
            }
        }
    }
    headers
}

/// Headers sent upstream for a proxied secret: the pod's headers minus the
/// opaque token (`authorization`, `x-api-key`, `private-token`), hop-by-hop
/// headers, `host`, `content-length`, `accept-encoding`, `cookie` and the
/// custom header of a [`SecretInjection::Header`], plus the real secret,
/// marked sensitive:
///
/// | Injection | Header sent |
/// |---|---|
/// | [`SecretInjection::Bearer`] | `authorization: Bearer <secret>` |
/// | [`SecretInjection::PrivateToken`] | `private-token: <secret>` |
/// | [`SecretInjection::XApiKey`] | `x-api-key: <secret>` |
/// | [`SecretInjection::Header`] | `<header>: <secret>` |
/// | [`SecretInjection::Basic`] | `authorization: Basic base64(<username>:<secret>)` |
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{
///     HostPattern, SecretCredential, SecretInjection, secret_upstream_headers,
/// };
/// use reqwest::header::{HeaderMap, HeaderValue};
///
/// # fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
/// let mut incoming = HeaderMap::new();
/// incoming.insert("private-token", HeaderValue::from_static("ifap_abc"));
/// let secret = SecretCredential::new(
///     "GITLAB_TOKEN".to_string(),
///     "glpat-x".to_string(),
///     SecretInjection::PrivateToken,
///     vec![HostPattern::parse("gitlab.com")?],
/// );
/// let headers = secret_upstream_headers(&incoming, &secret);
/// assert!(headers.get("private-token").is_some_and(|v| v == "glpat-x" && v.is_sensitive()));
/// # Ok(())
/// # }
/// ```
pub fn secret_upstream_headers(incoming: &HeaderMap, secret: &SecretCredential) -> HeaderMap {
    let custom = match secret.injection() {
        SecretInjection::Header(name) => HeaderName::from_bytes(name.as_bytes()).ok(),
        _ => None,
    };
    let mut headers = HeaderMap::new();
    for (name, value) in incoming {
        if !is_hop_by_hop(name)
            && !REQUEST_DROPPED.contains(&name.as_str())
            && custom.as_ref() != Some(name)
        {
            headers.append(name.clone(), value.clone());
        }
    }

    let value = secret.expose();
    let (name, raw) = match secret.injection() {
        SecretInjection::Bearer => (AUTHORIZATION, format!("Bearer {value}")),
        SecretInjection::PrivateToken => {
            (HeaderName::from_static(PRIVATE_TOKEN), value.to_string())
        }
        SecretInjection::XApiKey => (HeaderName::from_static(X_API_KEY), value.to_string()),
        SecretInjection::Basic { username } => (
            AUTHORIZATION,
            format!("Basic {}", STANDARD.encode(format!("{username}:{value}"))),
        ),
        SecretInjection::Header(_) => match custom {
            Some(name) => (name, value.to_string()),
            // Refused at issuance: nothing to inject.
            None => return headers,
        },
    };
    if let Some(value) = sensitive_value(&raw) {
        headers.insert(name, value);
    }
    headers
}

/// A header value marked sensitive. `None` only for a value holding
/// characters a header cannot carry, which [`super::AuthProxyRegistry::issue`]
/// refuses.
fn sensitive_value(raw: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(raw).ok().map(|mut value| {
        value.set_sensitive(true);
        value
    })
}

/// The `anthropic-beta` value carrying [`OAUTH_BETA`] once, after the flags
/// the pod already sent.
fn with_oauth_beta(headers: &HeaderMap) -> HeaderValue {
    let existing: Vec<&str> = headers
        .get_all(ANTHROPIC_BETA)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    let mut joined = existing.join(",");
    if !joined.split(',').any(|flag| flag.trim() == OAUTH_BETA) {
        if !joined.is_empty() {
            joined.push(',');
        }
        joined.push_str(OAUTH_BETA);
    }
    HeaderValue::from_str(&joined).unwrap_or_else(|_| HeaderValue::from_static(OAUTH_BETA))
}

/// Headers sent back to the pod: the upstream headers minus hop-by-hop
/// headers and `content-length` (the body is streamed). Rate-limit headers,
/// `content-type` and `request-id` are kept.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::downstream_headers;
/// use reqwest::header::{CONTENT_LENGTH, HeaderMap, HeaderValue};
///
/// let mut upstream = HeaderMap::new();
/// upstream.insert(CONTENT_LENGTH, HeaderValue::from_static("10"));
/// upstream.insert("anthropic-ratelimit-unified-status", HeaderValue::from_static("allowed"));
/// let headers = downstream_headers(&upstream);
/// assert!(headers.get(CONTENT_LENGTH).is_none());
/// assert!(headers.get("anthropic-ratelimit-unified-status").is_some());
/// ```
pub fn downstream_headers(upstream: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in upstream {
        if !is_hop_by_hop(name) && name.as_str() != "content-length" {
            headers.append(name.clone(), value.clone());
        }
    }
    headers
}

/// An error body in the Anthropic API shape, so Claude Code reports it as such.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::error_body;
///
/// let body = error_body("authentication_error", "invalid token");
/// assert_eq!(body["type"], "error");
/// assert_eq!(body["error"]["type"], "authentication_error");
/// ```
pub fn error_body(kind: &str, message: &str) -> Value {
    json!({
        "type": "error",
        "error": { "type": kind, "message": message }
    })
}

#[cfg(test)]
mod tests {
    use super::super::secret::HostPattern;
    use super::*;

    fn oauth() -> ProxyCredential {
        ProxyCredential::new(CredentialKind::OauthToken, "sk-ant-oat01-test".to_string())
    }

    fn api_key() -> ProxyCredential {
        ProxyCredential::new(CredentialKind::ApiKey, "sk-ant-api03-test".to_string())
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn bearer_and_x_api_key_extraction() {
        let bearer = headers(&[("authorization", "Bearer ifap_abc")]);
        assert_eq!(extract_opaque_token(&bearer).as_deref(), Some("ifap_abc"));

        let lower = headers(&[("authorization", "bearer ifap_low")]);
        assert_eq!(extract_opaque_token(&lower).as_deref(), Some("ifap_low"));

        let key = headers(&[("x-api-key", "ifap_key")]);
        assert_eq!(extract_opaque_token(&key).as_deref(), Some("ifap_key"));

        let basic = headers(&[("authorization", "Basic abc"), ("x-api-key", "ifap_key")]);
        assert_eq!(extract_opaque_token(&basic).as_deref(), Some("ifap_key"));
    }

    #[test]
    fn missing_or_empty_auth_is_none() {
        assert_eq!(extract_opaque_token(&HeaderMap::new()), None);
        assert_eq!(
            extract_opaque_token(&headers(&[("authorization", "Bearer ")])),
            None
        );
        assert_eq!(
            extract_opaque_token(&headers(&[("authorization", "Bearer")])),
            None
        );
        assert_eq!(extract_opaque_token(&headers(&[("x-api-key", "")])), None);
        assert_eq!(
            extract_opaque_token(&headers(&[("authorization", "Basic abc")])),
            None
        );
    }

    #[test]
    fn allowed_paths() {
        assert!(is_allowed_path("/v1/messages"));
        assert!(is_allowed_path("/v1/messages/count_tokens"));
        assert!(is_allowed_path("/v1/models/claude-sonnet-4-5"));
    }

    #[test]
    fn refused_paths() {
        for path in [
            "/",
            "",
            "/v1",
            "/admin/v1/tokens",
            "/api/oauth/profile",
            "/v1/../x",
            "/v1/messages/..",
            "/v1/%2e%2e/x",
            "//v1/messages",
            "/v1//messages",
            "/v1/messages?x=1",
            "/v1/mess ages",
        ] {
            assert!(!is_allowed_path(path), "{path} must be refused");
        }
    }

    #[test]
    fn allowed_methods() {
        assert!(is_allowed_method(&Method::GET));
        assert!(is_allowed_method(&Method::POST));
        for method in [
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::CONNECT,
            Method::OPTIONS,
        ] {
            assert!(!is_allowed_method(&method), "{method}");
        }
    }

    #[test]
    fn upstream_headers_oauth_sets_bearer_and_beta() {
        let incoming = headers(&[
            ("authorization", "Bearer ifap_abc"),
            ("content-type", "application/json"),
            ("anthropic-version", "2023-06-01"),
        ]);
        let out = upstream_headers(&incoming, &oauth());
        let auth = out.get(AUTHORIZATION).unwrap();
        assert_eq!(auth, "Bearer sk-ant-oat01-test");
        assert!(auth.is_sensitive());
        assert_eq!(out.get(ANTHROPIC_BETA).unwrap(), OAUTH_BETA);
        assert_eq!(out.get("content-type").unwrap(), "application/json");
        assert_eq!(out.get("anthropic-version").unwrap(), "2023-06-01");
        assert!(out.get(X_API_KEY).is_none());
    }

    #[test]
    fn upstream_headers_appends_existing_beta_once() {
        let incoming = headers(&[("anthropic-beta", "claude-code-20250219")]);
        let out = upstream_headers(&incoming, &oauth());
        assert_eq!(
            out.get(ANTHROPIC_BETA).unwrap(),
            "claude-code-20250219,oauth-2025-04-20"
        );
        assert_eq!(out.get_all(ANTHROPIC_BETA).iter().count(), 1);

        let already = headers(&[("anthropic-beta", "oauth-2025-04-20, other")]);
        let out = upstream_headers(&already, &oauth());
        assert_eq!(out.get(ANTHROPIC_BETA).unwrap(), "oauth-2025-04-20, other");
    }

    #[test]
    fn upstream_headers_api_key_sets_x_api_key_and_drops_authorization() {
        let incoming = headers(&[("authorization", "Bearer ifap_abc")]);
        let out = upstream_headers(&incoming, &api_key());
        assert!(out.get(AUTHORIZATION).is_none());
        let key = out.get(X_API_KEY).unwrap();
        assert_eq!(key, "sk-ant-api03-test");
        assert!(key.is_sensitive());
        assert!(out.get(ANTHROPIC_BETA).is_none());
    }

    #[test]
    fn upstream_headers_drops_opaque_token_and_hop_by_hop() {
        let incoming = headers(&[
            ("authorization", "Bearer ifap_abc"),
            ("x-api-key", "ifap_abc"),
            ("host", "ironflow-auth-proxy"),
            ("connection", "keep-alive"),
            ("keep-alive", "timeout=5"),
            ("proxy-authorization", "Basic x"),
            ("proxy-connection", "keep-alive"),
            ("te", "trailers"),
            ("trailer", "x"),
            ("transfer-encoding", "chunked"),
            ("upgrade", "h2c"),
            ("content-length", "12"),
            ("accept-encoding", "gzip, br"),
            ("cookie", "a=b"),
            ("user-agent", "claude-cli"),
        ]);
        let out = upstream_headers(&incoming, &oauth());
        for (_, value) in &out {
            assert!(!value.as_bytes().windows(4).any(|w| w == b"ifap"));
        }
        for name in [
            "host",
            "connection",
            "keep-alive",
            "proxy-authorization",
            "proxy-connection",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
            "content-length",
            "accept-encoding",
            "cookie",
            "x-api-key",
        ] {
            assert!(out.get(name).is_none(), "{name} must be dropped");
        }
        assert_eq!(out.get("user-agent").unwrap(), "claude-cli");
    }

    #[test]
    fn downstream_headers_keeps_ratelimit_headers() {
        let upstream = headers(&[
            ("content-type", "text/event-stream"),
            ("request-id", "req_1"),
            ("anthropic-ratelimit-unified-status", "allowed"),
            ("anthropic-ratelimit-unified-5h-utilization", "0.2"),
            ("content-length", "42"),
            ("connection", "close"),
            ("transfer-encoding", "chunked"),
        ]);
        let out = downstream_headers(&upstream);
        assert_eq!(out.get("content-type").unwrap(), "text/event-stream");
        assert_eq!(out.get("request-id").unwrap(), "req_1");
        assert_eq!(
            out.get("anthropic-ratelimit-unified-status").unwrap(),
            "allowed"
        );
        assert_eq!(
            out.get("anthropic-ratelimit-unified-5h-utilization")
                .unwrap(),
            "0.2"
        );
        assert!(out.get("content-length").is_none());
        assert!(out.get("connection").is_none());
        assert!(out.get("transfer-encoding").is_none());
    }

    #[test]
    fn error_body_has_anthropic_shape() {
        let body = error_body("permission_error", "nope");
        assert_eq!(
            body,
            json!({"type": "error", "error": {"type": "permission_error", "message": "nope"}})
        );
    }

    fn secret(injection: SecretInjection) -> SecretCredential {
        SecretCredential::new(
            "GITHUB_TOKEN".to_string(),
            "ghp_test".to_string(),
            injection,
            vec![HostPattern::parse("api.github.com").unwrap()],
        )
    }

    /// Every header the pod could carry its opaque token in.
    fn pod_headers() -> HeaderMap {
        headers(&[
            ("authorization", "Bearer ifap_abc"),
            ("x-api-key", "ifap_abc"),
            ("private-token", "ifap_abc"),
            ("x-vault-token", "ifap_abc"),
            ("host", "ironflow-auth-proxy"),
            ("cookie", "a=b"),
            ("connection", "keep-alive"),
            ("accept", "application/json"),
        ])
    }

    fn assert_no_opaque_token(out: &HeaderMap) {
        for (name, value) in out {
            assert!(
                !value.as_bytes().windows(4).any(|w| w == b"ifap"),
                "{name} still carries the opaque token"
            );
        }
        for name in ["host", "cookie", "connection"] {
            assert!(out.get(name).is_none(), "{name} must be dropped");
        }
        assert_eq!(out.get("accept").unwrap(), "application/json");
    }

    #[test]
    fn private_token_is_extracted() {
        let pod = headers(&[("private-token", "ifap_pt")]);
        assert_eq!(extract_opaque_token(&pod).as_deref(), Some("ifap_pt"));
        let empty = headers(&[("private-token", " ")]);
        assert_eq!(extract_opaque_token(&empty), None);
    }

    #[test]
    fn basic_password_is_extracted() {
        let encoded = format!("Basic {}", STANDARD.encode("oauth2:ifap_x"));
        let mut pod = HeaderMap::new();
        pod.insert(AUTHORIZATION, HeaderValue::from_str(&encoded).unwrap());
        assert_eq!(extract_opaque_token(&pod).as_deref(), Some("ifap_x"));

        let encoded = format!("basic {}", STANDARD.encode("oauth2:"));
        let mut empty = HeaderMap::new();
        empty.insert(AUTHORIZATION, HeaderValue::from_str(&encoded).unwrap());
        assert_eq!(extract_opaque_token(&empty), None);

        let encoded = format!("Basic {}", STANDARD.encode("no-colon"));
        let mut no_colon = HeaderMap::new();
        no_colon.insert(AUTHORIZATION, HeaderValue::from_str(&encoded).unwrap());
        assert_eq!(extract_opaque_token(&no_colon), None);
    }

    #[test]
    fn relay_paths() {
        for path in ["/", "/user", "/group/project.git/info/refs", "/v1/messages"] {
            assert!(is_relay_path(path), "{path} must be accepted");
        }
        for path in [
            "", "user", "/..", "/a/../b", "//x", "/a//b", "/a%2Fb", "/a b", "/a?b",
        ] {
            assert!(!is_relay_path(path), "{path} must be refused");
        }
    }

    #[test]
    fn relay_methods() {
        for method in [
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ] {
            assert!(is_relay_method(&method), "{method}");
        }
        for method in [Method::CONNECT, Method::TRACE, Method::OPTIONS] {
            assert!(!is_relay_method(&method), "{method}");
        }
    }

    #[test]
    fn secret_headers_bearer() {
        let out = secret_upstream_headers(&pod_headers(), &secret(SecretInjection::Bearer));
        let auth = out.get(AUTHORIZATION).unwrap();
        assert_eq!(auth, "Bearer ghp_test");
        assert!(auth.is_sensitive());
        assert!(out.get(X_API_KEY).is_none());
        assert!(out.get(PRIVATE_TOKEN).is_none());
        assert!(out.get(ANTHROPIC_BETA).is_none());
        assert_eq!(out.get("x-vault-token").unwrap(), "ifap_abc");
    }

    #[test]
    fn secret_headers_private_token() {
        let mut pod = pod_headers();
        pod.remove("x-vault-token");
        let out = secret_upstream_headers(&pod, &secret(SecretInjection::PrivateToken));
        let value = out.get(PRIVATE_TOKEN).unwrap();
        assert_eq!(value, "ghp_test");
        assert!(value.is_sensitive());
        assert!(out.get(AUTHORIZATION).is_none());
        assert!(out.get(X_API_KEY).is_none());
        assert_no_opaque_token(&out);
    }

    #[test]
    fn secret_headers_x_api_key() {
        let mut pod = pod_headers();
        pod.remove("x-vault-token");
        let out = secret_upstream_headers(&pod, &secret(SecretInjection::XApiKey));
        let value = out.get(X_API_KEY).unwrap();
        assert_eq!(value, "ghp_test");
        assert!(value.is_sensitive());
        assert!(out.get(AUTHORIZATION).is_none());
        assert!(out.get(PRIVATE_TOKEN).is_none());
        assert_no_opaque_token(&out);
    }

    #[test]
    fn secret_headers_custom_header() {
        let injection = SecretInjection::Header("X-Vault-Token".to_string());
        let out = secret_upstream_headers(&pod_headers(), &secret(injection));
        let value = out.get("x-vault-token").unwrap();
        assert_eq!(value, "ghp_test");
        assert!(value.is_sensitive());
        assert_eq!(out.get_all("x-vault-token").iter().count(), 1);
        assert!(out.get(AUTHORIZATION).is_none());
        assert_no_opaque_token(&out);
    }

    #[test]
    fn secret_headers_basic() {
        let mut pod = pod_headers();
        pod.remove("x-vault-token");
        let injection = SecretInjection::Basic {
            username: "oauth2".to_string(),
        };
        let out = secret_upstream_headers(&pod, &secret(injection));
        let auth = out.get(AUTHORIZATION).unwrap();
        assert!(auth.is_sensitive());
        let encoded = auth.to_str().unwrap().strip_prefix("Basic ").unwrap();
        assert_eq!(STANDARD.decode(encoded).unwrap(), b"oauth2:ghp_test");
        assert!(out.get(PRIVATE_TOKEN).is_none());
        assert_no_opaque_token(&out);
    }
}
