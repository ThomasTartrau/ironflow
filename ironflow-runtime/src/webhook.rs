//! Webhook authentication strategies for incoming HTTP requests.
//!
//! This module provides [`WebhookAuth`], an enum that supports multiple
//! authentication schemes commonly used by webhook providers such as GitHub
//! and GitLab. It can also be used with custom header-based or HMAC-based
//! authentication flows.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tracing::warn;

type HmacSha256 = Hmac<Sha256>;

/// Authentication strategy for a webhook endpoint.
///
/// Each variant describes how to verify that an incoming request is legitimate.
/// Use the constructor methods ([`WebhookAuth::none`], [`WebhookAuth::header`],
/// [`WebhookAuth::github`], [`WebhookAuth::gitlab`]) rather than building
/// variants manually, as they normalise header names and apply the correct
/// defaults.
///
/// # Examples
///
/// ```no_run
/// use ironflow_runtime::webhook::WebhookAuth;
///
/// // No authentication
/// let auth = WebhookAuth::none();
///
/// // GitHub HMAC-SHA256 authentication
/// let auth = WebhookAuth::github("my-webhook-secret");
///
/// // GitLab static-token authentication
/// let auth = WebhookAuth::gitlab("my-gitlab-token");
///
/// // Custom header authentication
/// let auth = WebhookAuth::header("x-api-key", "secret-value");
/// ```
#[derive(Clone)]
pub enum WebhookAuth {
    /// No authentication - every request is accepted.
    None,
    /// Static header comparison.
    ///
    /// The request must contain a header whose value exactly matches
    /// `expected`. The header `name` is stored in lower-case.
    Header {
        /// Lower-cased header name.
        name: String,
        /// Expected header value.
        expected: String,
    },
    /// HMAC-SHA256 signature verification.
    ///
    /// The request must contain a header whose value is a hex-encoded
    /// HMAC-SHA256 digest prefixed with `sha256=`. The digest is computed
    /// over the raw request body using `secret` as the HMAC key.
    HmacSha256 {
        /// Header name that carries the signature (e.g. `x-hub-signature-256`).
        header: String,
        /// Shared secret used to compute the HMAC.
        secret: String,
    },
}

impl std::fmt::Debug for WebhookAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "WebhookAuth::None"),
            Self::Header { name, .. } => f
                .debug_struct("WebhookAuth::Header")
                .field("name", name)
                .field("expected", &"[REDACTED]")
                .finish(),
            Self::HmacSha256 { header, .. } => f
                .debug_struct("WebhookAuth::HmacSha256")
                .field("header", header)
                .field("secret", &"[REDACTED]")
                .finish(),
        }
    }
}

impl WebhookAuth {
    /// Creates an authentication strategy that accepts every request.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::webhook::WebhookAuth;
    ///
    /// let auth = WebhookAuth::none();
    /// ```
    pub fn none() -> Self {
        Self::None
    }

    /// Creates a static-header authentication strategy.
    ///
    /// The `name` is automatically lower-cased so that look-ups are
    /// case-insensitive (HTTP headers are case-insensitive by spec).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::webhook::WebhookAuth;
    ///
    /// let auth = WebhookAuth::header("x-api-key", "super-secret");
    /// ```
    pub fn header(name: &str, expected: &str) -> Self {
        if expected.is_empty() {
            warn!(
                "WebhookAuth::header created with an empty expected value - any request with an empty header will be accepted"
            );
        }
        Self::Header {
            name: name.to_lowercase(),
            expected: expected.to_string(),
        }
    }

    /// Preset for **GitLab** webhooks.
    ///
    /// GitLab sends the secret token in the `X-Gitlab-Token` header as a
    /// plain-text value. This is a convenience wrapper around
    /// [`WebhookAuth::header`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::webhook::WebhookAuth;
    ///
    /// let auth = WebhookAuth::gitlab("my-gitlab-secret");
    /// ```
    pub fn gitlab(secret: &str) -> Self {
        if secret.is_empty() {
            warn!(
                "WebhookAuth::gitlab created with an empty token - any request with an empty X-Gitlab-Token header will be accepted"
            );
        }
        Self::Header {
            name: "x-gitlab-token".to_string(),
            expected: secret.to_string(),
        }
    }

    /// Preset for **GitHub** webhooks.
    ///
    /// GitHub signs the payload body with HMAC-SHA256 and sends the result
    /// in the `X-Hub-Signature-256` header, prefixed with `sha256=`.
    /// This constructor configures the correct header name and stores the
    /// shared secret.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::webhook::WebhookAuth;
    ///
    /// let auth = WebhookAuth::github("my-github-webhook-secret");
    /// ```
    pub fn github(secret: &str) -> Self {
        if secret.is_empty() {
            warn!(
                "WebhookAuth::github created with an empty secret - HMAC verification will be trivially bypassable"
            );
        }
        Self::HmacSha256 {
            header: "x-hub-signature-256".to_string(),
            secret: secret.to_string(),
        }
    }

    /// Verifies an incoming request against this authentication strategy.
    ///
    /// Returns `true` if the request is authentic, `false` otherwise.
    ///
    /// # Behaviour per variant
    ///
    /// | Variant | Verification |
    /// |---|---|
    /// | [`None`](WebhookAuth::None) | Always returns `true`. |
    /// | [`Header`](WebhookAuth::Header) | Checks that the named header equals the expected value. |
    /// | [`HmacSha256`](WebhookAuth::HmacSha256) | Strips the `sha256=` prefix, hex-decodes the signature, computes HMAC-SHA256 over `body`, and compares in constant time. |
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use axum::http::HeaderMap;
    /// use ironflow_runtime::webhook::WebhookAuth;
    ///
    /// let auth = WebhookAuth::none();
    /// let headers = HeaderMap::new();
    /// assert!(auth.verify(&headers, b"any body"));
    /// ```
    pub fn verify(&self, headers: &axum::http::HeaderMap, body: &[u8]) -> bool {
        match self {
            Self::None => true,
            Self::Header { name, expected } => headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.as_bytes().ct_eq(expected.as_bytes()).into()),
            Self::HmacSha256 { header, secret } => {
                let Some(signature) = headers.get(header).and_then(|v| v.to_str().ok()) else {
                    return false;
                };
                let Some(signature) = signature.strip_prefix("sha256=") else {
                    return false;
                };
                let Ok(sig_bytes) = hex::decode(signature) else {
                    return false;
                };
                let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
                    return false;
                };
                mac.update(body);
                mac.verify_slice(&sig_bytes).is_ok()
            }
        }
    }
}

/// Compute an HMAC-SHA256 signature string (prefixed with `sha256=`).
///
/// Providers that stamp each delivery with a unique identifier, and the header
/// carrying it. The prefix disambiguates identifiers across providers, since
/// idempotency keys are global.
const DELIVERY_ID_HEADERS: &[(&str, &str)] = &[
    ("x-github-delivery", "github"),
    ("x-gitlab-event-uuid", "gitlab"),
];

/// Maximum length of a derived idempotency key, in bytes.
///
/// Mirrors the `Idempotency-Key` limit enforced by `POST /api/v1/runs`. Kept
/// local so the runtime does not depend on the storage crate.
///
/// # Examples
///
/// ```
/// use ironflow_runtime::webhook::MAX_IDEMPOTENCY_KEY_LEN;
///
/// assert_eq!(MAX_IDEMPOTENCY_KEY_LEN, 255);
/// ```
pub const MAX_IDEMPOTENCY_KEY_LEN: usize = 255;

/// Extract a provider delivery identifier from request headers.
///
/// Returns a prefixed key such as `github:8f4e2a10-...` suitable for use as an
/// `Idempotency-Key`. Returns `None` when no known provider header is present,
/// when its value is not printable ASCII, or when it exceeds
/// [`MAX_IDEMPOTENCY_KEY_LEN`] once prefixed.
///
/// # Examples
///
/// ```
/// use axum::http::HeaderMap;
/// use ironflow_runtime::webhook::extract_delivery_id;
///
/// let mut headers = HeaderMap::new();
/// headers.insert("x-github-delivery", "abc-123".parse().unwrap());
/// assert_eq!(extract_delivery_id(&headers), Some("github:abc-123".to_string()));
///
/// assert_eq!(extract_delivery_id(&HeaderMap::new()), None);
/// ```
pub fn extract_delivery_id(headers: &axum::http::HeaderMap) -> Option<String> {
    for (header, provider) in DELIVERY_ID_HEADERS {
        let Some(raw) = headers.get(*header).and_then(|v| v.to_str().ok()) else {
            continue;
        };
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_graphic()) {
            warn!(header = %header, "delivery id is not printable ASCII, ignoring");
            continue;
        }

        let key = format!("{provider}:{raw}");
        if key.len() > MAX_IDEMPOTENCY_KEY_LEN {
            warn!(header = %header, "delivery id is too long for an idempotency key, ignoring");
            continue;
        }
        return Some(key);
    }
    None
}

/// Shared helper for tests and integration test suites.
#[cfg(test)]
pub(crate) fn compute_test_hmac(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC key rejected");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    // ── WebhookAuth::None ──────────────────────────────────────────

    #[test]
    fn none_always_returns_true() {
        let auth = WebhookAuth::none();
        let headers = HeaderMap::new();
        assert!(auth.verify(&headers, b"anything"));
    }

    #[test]
    fn none_empty_body_returns_true() {
        let auth = WebhookAuth::none();
        let headers = HeaderMap::new();
        assert!(auth.verify(&headers, b""));
    }

    #[test]
    fn none_empty_headers_returns_true() {
        let auth = WebhookAuth::none();
        let headers = HeaderMap::new();
        assert!(auth.verify(&headers, b"payload"));
    }

    // ── WebhookAuth::Header ────────────────────────────────────────

    #[test]
    fn header_correct_value_returns_true() {
        let auth = WebhookAuth::header("x-api-key", "secret123");
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "secret123".parse().unwrap());
        assert!(auth.verify(&headers, b""));
    }

    #[test]
    fn header_wrong_value_returns_false() {
        let auth = WebhookAuth::header("x-api-key", "secret123");
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "wrong".parse().unwrap());
        assert!(!auth.verify(&headers, b""));
    }

    #[test]
    fn header_missing_returns_false() {
        let auth = WebhookAuth::header("x-api-key", "secret123");
        let headers = HeaderMap::new();
        assert!(!auth.verify(&headers, b""));
    }

    #[test]
    fn header_name_lookup_is_case_insensitive() {
        let auth = WebhookAuth::header("X-Api-Key", "secret123");
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "secret123".parse().unwrap());
        assert!(auth.verify(&headers, b""));
    }

    #[test]
    fn header_value_comparison_is_case_sensitive() {
        let auth = WebhookAuth::header("x-api-key", "Secret123");
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "secret123".parse().unwrap());
        assert!(!auth.verify(&headers, b""));
    }

    #[test]
    fn header_empty_expected_with_empty_value_returns_true() {
        let auth = WebhookAuth::header("x-api-key", "");
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "".parse().unwrap());
        assert!(auth.verify(&headers, b""));
    }

    // ── WebhookAuth::gitlab ────────────────────────────────────────

    #[test]
    fn gitlab_correct_token_returns_true() {
        let auth = WebhookAuth::gitlab("gl-token-abc");
        let mut headers = HeaderMap::new();
        headers.insert("x-gitlab-token", "gl-token-abc".parse().unwrap());
        assert!(auth.verify(&headers, b""));
    }

    #[test]
    fn gitlab_wrong_token_returns_false() {
        let auth = WebhookAuth::gitlab("gl-token-abc");
        let mut headers = HeaderMap::new();
        headers.insert("x-gitlab-token", "wrong".parse().unwrap());
        assert!(!auth.verify(&headers, b""));
    }

    #[test]
    fn gitlab_missing_header_returns_false() {
        let auth = WebhookAuth::gitlab("gl-token-abc");
        let headers = HeaderMap::new();
        assert!(!auth.verify(&headers, b""));
    }

    // ── WebhookAuth::github ────────────────────────────────────────

    #[test]
    fn github_valid_hmac_returns_true() {
        let secret = "gh-secret";
        let body = b"payload body";
        let auth = WebhookAuth::github(secret);
        let sig = compute_test_hmac(secret.as_bytes(), body);
        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", sig.parse().unwrap());
        assert!(auth.verify(&headers, body));
    }

    #[test]
    fn github_invalid_signature_returns_false() {
        let auth = WebhookAuth::github("gh-secret");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-hub-signature-256",
            "sha256=0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );
        assert!(!auth.verify(&headers, b"payload"));
    }

    #[test]
    fn github_missing_header_returns_false() {
        let auth = WebhookAuth::github("gh-secret");
        let headers = HeaderMap::new();
        assert!(!auth.verify(&headers, b"payload"));
    }

    // ── WebhookAuth::HmacSha256 (direct) ──────────────────────────

    #[test]
    fn hmac_valid_signature_verifies() {
        let secret = "my-secret";
        let body = b"request body";
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: secret.to_string(),
        };
        let sig = compute_test_hmac(secret.as_bytes(), body);
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", sig.parse().unwrap());
        assert!(auth.verify(&headers, body));
    }

    #[test]
    fn hmac_tampered_signature_returns_false() {
        let secret = "my-secret";
        let body = b"request body";
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: secret.to_string(),
        };
        let mut sig = compute_test_hmac(secret.as_bytes(), body);
        // Tamper with the last character
        sig.pop();
        sig.push('0');
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", sig.parse().unwrap());
        assert!(!auth.verify(&headers, body));
    }

    #[test]
    fn hmac_missing_header_returns_false() {
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: "my-secret".to_string(),
        };
        let headers = HeaderMap::new();
        assert!(!auth.verify(&headers, b"body"));
    }

    #[test]
    fn hmac_no_sha256_prefix_returns_false() {
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: "my-secret".to_string(),
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-signature",
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
                .parse()
                .unwrap(),
        );
        assert!(!auth.verify(&headers, b"body"));
    }

    #[test]
    fn hmac_invalid_hex_returns_false() {
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: "my-secret".to_string(),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", "sha256=not-valid-hex!".parse().unwrap());
        assert!(!auth.verify(&headers, b"body"));
    }

    #[test]
    fn hmac_wrong_secret_returns_false() {
        let body = b"request body";
        let sig = compute_test_hmac(b"correct-secret", body);
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: "wrong-secret".to_string(),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", sig.parse().unwrap());
        assert!(!auth.verify(&headers, body));
    }

    #[test]
    fn hmac_empty_body_verifies() {
        let secret = "my-secret";
        let body = b"";
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: secret.to_string(),
        };
        let sig = compute_test_hmac(secret.as_bytes(), body);
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", sig.parse().unwrap());
        assert!(auth.verify(&headers, body));
    }

    #[test]
    fn hmac_body_tampered_returns_false() {
        let secret = "my-secret";
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: secret.to_string(),
        };
        let sig = compute_test_hmac(secret.as_bytes(), b"original body");
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", sig.parse().unwrap());
        assert!(!auth.verify(&headers, b"tampered body"));
    }

    #[test]
    fn hmac_empty_secret_still_works() {
        let secret = "";
        let body = b"some body";
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: secret.to_string(),
        };
        let sig = compute_test_hmac(secret.as_bytes(), body);
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", sig.parse().unwrap());
        assert!(auth.verify(&headers, body));
    }

    #[test]
    fn debug_redacts_header_secret() {
        let auth = WebhookAuth::header("x-api-key", "super-secret");
        let debug = format!("{:?}", auth);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn debug_redacts_hmac_secret() {
        let auth = WebhookAuth::github("my-secret-key");
        let debug = format!("{:?}", auth);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("my-secret-key"));
    }

    #[test]
    fn debug_none_format() {
        let auth = WebhookAuth::none();
        let debug = format!("{:?}", auth);
        assert_eq!(debug, "WebhookAuth::None");
    }

    #[test]
    fn hmac_rfc4231_test_vector() {
        // RFC 4231 Test Case 2 (adapted: Key=0x0b*20, Data="Hi There")
        let key_bytes = hex::decode("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b").unwrap();
        let body = b"Hi There";
        let expected_mac = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";

        // Compute with raw key bytes to verify the test vector
        let mut mac = HmacSha256::new_from_slice(&key_bytes).unwrap();
        mac.update(body);
        let computed = hex::encode(mac.finalize().into_bytes());
        assert_eq!(computed, expected_mac);

        // Now verify through WebhookAuth - but note that WebhookAuth uses
        // secret.as_bytes() (UTF-8), not raw hex bytes. So we construct an
        // HmacSha256 variant with the raw key by converting key_bytes to a
        // String via unsafe (the bytes are not valid UTF-8, so we use
        // Latin-1 mapping instead).
        // Since the verify method uses `secret.as_bytes()`, and we need the
        // key to be exactly `key_bytes`, we can only test this if those bytes
        // round-trip through String::as_bytes(). They do if we build a String
        // from those bytes using Latin-1. However Rust strings are UTF-8 and
        // 0x0b is valid UTF-8 (it's a control char), so this works.
        let secret_str = String::from_utf8(key_bytes).unwrap();
        let auth = WebhookAuth::HmacSha256 {
            header: "x-signature".to_string(),
            secret: secret_str,
        };
        let sig = format!("sha256={}", expected_mac);
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", sig.parse().unwrap());
        assert!(auth.verify(&headers, body));
    }

    // ── extract_delivery_id ────────────────────────────────────────

    #[test]
    fn github_delivery_id_is_prefixed() {
        let mut headers = HeaderMap::new();
        headers.insert("x-github-delivery", "abc-123".parse().unwrap());
        assert_eq!(
            extract_delivery_id(&headers),
            Some("github:abc-123".to_string())
        );
    }

    #[test]
    fn gitlab_event_uuid_is_prefixed() {
        let mut headers = HeaderMap::new();
        headers.insert("x-gitlab-event-uuid", "def-456".parse().unwrap());
        assert_eq!(
            extract_delivery_id(&headers),
            Some("gitlab:def-456".to_string())
        );
    }

    #[test]
    fn no_provider_header_yields_none() {
        let headers = HeaderMap::new();
        assert_eq!(extract_delivery_id(&headers), None);
    }

    #[test]
    fn unrelated_headers_yield_none() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", "abc".parse().unwrap());
        assert_eq!(extract_delivery_id(&headers), None);
    }

    #[test]
    fn github_wins_over_gitlab_when_both_are_present() {
        let mut headers = HeaderMap::new();
        headers.insert("x-github-delivery", "gh".parse().unwrap());
        headers.insert("x-gitlab-event-uuid", "gl".parse().unwrap());
        assert_eq!(extract_delivery_id(&headers), Some("github:gh".to_string()));
    }

    #[test]
    fn empty_delivery_id_is_ignored() {
        let mut headers = HeaderMap::new();
        headers.insert("x-github-delivery", "".parse().unwrap());
        assert_eq!(extract_delivery_id(&headers), None);
    }

    #[test]
    fn delivery_id_with_a_space_is_ignored() {
        let mut headers = HeaderMap::new();
        headers.insert("x-github-delivery", "abc 123".parse().unwrap());
        assert_eq!(extract_delivery_id(&headers), None);
    }

    #[test]
    fn overlong_delivery_id_is_ignored() {
        let mut headers = HeaderMap::new();
        let raw = "a".repeat(MAX_IDEMPOTENCY_KEY_LEN);
        headers.insert("x-github-delivery", raw.parse().unwrap());
        assert_eq!(extract_delivery_id(&headers), None);
    }

    #[test]
    fn delivery_id_at_the_length_limit_is_accepted() {
        let mut headers = HeaderMap::new();
        // "github:" is 7 bytes.
        let raw = "a".repeat(MAX_IDEMPOTENCY_KEY_LEN - 7);
        headers.insert("x-github-delivery", raw.parse().unwrap());
        let key = extract_delivery_id(&headers).expect("key accepted");
        assert_eq!(key.len(), MAX_IDEMPOTENCY_KEY_LEN);
    }

    #[test]
    fn gitlab_is_used_when_github_header_is_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert("x-github-delivery", "".parse().unwrap());
        headers.insert("x-gitlab-event-uuid", "def".parse().unwrap());
        assert_eq!(
            extract_delivery_id(&headers),
            Some("gitlab:def".to_string())
        );
    }
}
