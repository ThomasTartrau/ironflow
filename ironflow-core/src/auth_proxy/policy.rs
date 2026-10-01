//! What the proxy accepts from a pod and what it forwards each way.

use reqwest::Method;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use super::OAUTH_BETA;
use super::credential::{CredentialKind, ProxyCredential};

const X_API_KEY: &str = "x-api-key";
const ANTHROPIC_BETA: &str = "anthropic-beta";

/// Headers that only make sense on one connection.
const HOP_BY_HOP: [&str; 9] = [
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
/// opaque token, the pod's host, the body length (recomputed), the encoding
/// negotiation (the proxy relays identity bodies) and cookies.
const REQUEST_DROPPED: [&str; 6] = [
    "authorization",
    X_API_KEY,
    "host",
    "content-length",
    "accept-encoding",
    "cookie",
];

fn is_hop_by_hop(name: &HeaderName) -> bool {
    HOP_BY_HOP.contains(&name.as_str())
}

/// The opaque token a pod presents: `Authorization: Bearer <token>` (scheme
/// case-insensitive), else `x-api-key`. `None` when absent or empty.
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
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim())
        .filter(|token| !token.is_empty());
    let api_key = || {
        headers
            .get(X_API_KEY)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|token| !token.is_empty())
    };
    bearer.or_else(api_key).map(str::to_string)
}

/// Whether the proxy relays `path`: it starts with `/v1/`, only holds
/// `[A-Za-z0-9/_.-]` (so no percent-encoding), and has no `..` segment and no
/// `//`.
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
    path.starts_with("/v1/")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
        && !path.contains("//")
        && !path.split('/').any(|segment| segment == "..")
}

/// Whether the proxy relays `method`: GET and POST only.
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
}
