//! HttpOnly cookie builders for auth and refresh tokens.
//!
//! All cookies are built with [`cookie::Cookie`] for correctness,
//! then serialized to `Set-Cookie` header strings.

use axum::http::HeaderMap;
use axum_extra::extract::CookieJar;
use cookie::time::Duration;
use cookie::{Cookie, SameSite};

use crate::jwt::JwtConfig;

/// Cookie name for the access token.
pub const AUTH_COOKIE_NAME: &str = "ironflow_token";
/// Cookie name for the refresh token.
pub const REFRESH_COOKIE_NAME: &str = "ironflow_refresh";

/// Build a base cookie with common security attributes.
fn base_cookie<'a>(
    name: &'a str,
    value: &'a str,
    max_age_secs: i64,
    config: &JwtConfig,
) -> Cookie<'a> {
    let mut cookie = Cookie::build((name, value))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(Duration::seconds(max_age_secs))
        .secure(config.cookie_secure)
        .build();

    if let Some(ref domain) = config.cookie_domain {
        cookie.set_domain(domain.clone());
    }

    cookie
}

/// Build a `Set-Cookie` header value for the access token.
pub fn build_auth_cookie(token: &str, config: &JwtConfig) -> String {
    base_cookie(
        AUTH_COOKIE_NAME,
        token,
        config.access_token_ttl_secs,
        config,
    )
    .to_string()
}

/// Build a `Set-Cookie` header value for the refresh token.
pub fn build_refresh_cookie(token: &str, config: &JwtConfig) -> String {
    base_cookie(
        REFRESH_COOKIE_NAME,
        token,
        config.refresh_token_ttl_secs,
        config,
    )
    .to_string()
}

/// Build a `Set-Cookie` header value that clears the access token.
pub fn clear_auth_cookie(config: &JwtConfig) -> String {
    base_cookie(AUTH_COOKIE_NAME, "", 0, config).to_string()
}

/// Build a `Set-Cookie` header value that clears the refresh token.
pub fn clear_refresh_cookie(config: &JwtConfig) -> String {
    base_cookie(REFRESH_COOKIE_NAME, "", 0, config).to_string()
}

/// Extract the refresh token from request headers (cookie jar).
pub fn extract_refresh_token(headers: &HeaderMap) -> Option<String> {
    let jar = CookieJar::from_headers(headers);
    jar.get(REFRESH_COOKIE_NAME).map(|c| c.value().to_string())
}

#[cfg(test)]
mod tests {
    use axum::http::header::COOKIE;

    use super::*;

    fn test_config() -> JwtConfig {
        JwtConfig {
            secret: "test".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        }
    }

    #[test]
    fn auth_cookie_attributes() {
        let config = test_config();
        let cookie = build_auth_cookie("my-token", &config);
        assert!(cookie.contains("ironflow_token=my-token"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age=900"));
    }

    #[test]
    fn clear_auth_cookie_expires() {
        let config = test_config();
        let cookie = clear_auth_cookie(&config);
        assert!(cookie.contains("ironflow_token="));
        assert!(cookie.contains("Max-Age=0"));
    }

    #[test]
    fn refresh_cookie_attributes() {
        let config = test_config();
        let cookie = build_refresh_cookie("refresh-tok", &config);
        assert!(cookie.contains("ironflow_refresh=refresh-tok"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Max-Age=604800"));
    }

    #[test]
    fn clear_refresh_cookie_expires() {
        let config = test_config();
        let cookie = clear_refresh_cookie(&config);
        assert!(cookie.contains("ironflow_refresh="));
        assert!(cookie.contains("Max-Age=0"));
    }

    #[test]
    fn secure_flag_when_enabled() {
        let mut config = test_config();
        config.cookie_secure = true;
        let cookie = build_auth_cookie("tok", &config);
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn secure_flag_on_clear_when_enabled() {
        let mut config = test_config();
        config.cookie_secure = true;
        let cookie = clear_auth_cookie(&config);
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn domain_when_set() {
        let mut config = test_config();
        config.cookie_domain = Some("example.com".to_string());
        let cookie = build_auth_cookie("tok", &config);
        assert!(cookie.contains("Domain=example.com"));
    }

    #[test]
    fn extract_refresh_token_from_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            format!("{REFRESH_COOKIE_NAME}=my-refresh-token")
                .parse()
                .unwrap(),
        );
        let extracted = extract_refresh_token(&headers);
        assert_eq!(extracted, Some("my-refresh-token".to_string()));
    }

    #[test]
    fn extract_refresh_token_missing() {
        let headers = HeaderMap::new();
        assert_eq!(extract_refresh_token(&headers), None);
    }
}
