//! JWT access and refresh token management.
//!
//! Access tokens are short-lived (default 15 min) and used for API requests.
//! Refresh tokens are long-lived (default 7 days) and used to obtain new access tokens.
//! Token types are enforced — a refresh token cannot be used as an access token.

use chrono::Utc;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AuthError;

const ACCESS_TOKEN_TYPE: &str = "access";
const REFRESH_TOKEN_TYPE: &str = "refresh";

/// JWT configuration.
///
/// # Examples
///
/// ```
/// use ironflow_auth::jwt::JwtConfig;
///
/// let config = JwtConfig {
///     secret: "my-secret-key".to_string(),
///     access_token_ttl_secs: 900,
///     refresh_token_ttl_secs: 604800,
///     cookie_domain: None,
///     cookie_secure: false,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct JwtConfig {
    /// HMAC secret for signing tokens.
    pub secret: String,
    /// Access token time-to-live in seconds (default: 900 = 15 min).
    pub access_token_ttl_secs: i64,
    /// Refresh token time-to-live in seconds (default: 604800 = 7 days).
    pub refresh_token_ttl_secs: i64,
    /// Optional cookie domain (e.g., `.example.com`).
    pub cookie_domain: Option<String>,
    /// Whether to set the `Secure` flag on cookies.
    pub cookie_secure: bool,
}

/// Claims embedded in an access token.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AccessTokenClaims {
    /// Subject (user_id).
    pub sub: Uuid,
    /// Unique token identifier.
    pub jti: String,
    /// Issued at (unix timestamp).
    pub iat: i64,
    /// Expiration (unix timestamp).
    pub exp: i64,
    /// Token type — always "access".
    pub typ: String,
    /// User ID.
    pub user_id: Uuid,
    /// Username.
    pub username: String,
    /// Admin flag.
    pub is_admin: bool,
}

/// Claims embedded in a refresh token.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RefreshTokenClaims {
    /// Subject (user_id).
    pub sub: Uuid,
    /// Unique token identifier.
    pub jti: String,
    /// Issued at (unix timestamp).
    pub iat: i64,
    /// Expiration (unix timestamp).
    pub exp: i64,
    /// Token type — always "refresh".
    pub typ: String,
    /// User ID.
    pub user_id: Uuid,
    /// Username.
    pub username: String,
    /// Admin flag.
    pub is_admin: bool,
}

/// A signed access token (JWT string).
pub struct AccessToken(pub String);

impl AccessToken {
    /// Create an access token for a user.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Jwt`] if JWT encoding fails.
    pub fn for_user(
        user_id: Uuid,
        username: &str,
        is_admin: bool,
        config: &JwtConfig,
    ) -> Result<Self, AuthError> {
        let now = Utc::now().timestamp();
        let claims = AccessTokenClaims {
            sub: user_id,
            jti: Uuid::now_v7().to_string(),
            iat: now,
            exp: now + config.access_token_ttl_secs,
            typ: ACCESS_TOKEN_TYPE.to_string(),
            user_id,
            username: username.to_string(),
            is_admin,
        };
        let token = jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(config.secret.as_bytes()),
        )?;
        Ok(Self(token))
    }

    /// Decode and validate an access token.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Jwt`] if the token is invalid, expired, or not an access token.
    pub fn decode(token: &str, config: &JwtConfig) -> Result<AccessTokenClaims, AuthError> {
        let token_data = jsonwebtoken::decode::<AccessTokenClaims>(
            token,
            &DecodingKey::from_secret(config.secret.as_bytes()),
            &Validation::default(),
        )?;
        if token_data.claims.typ != ACCESS_TOKEN_TYPE {
            return Err(AuthError::InvalidToken);
        }
        Ok(token_data.claims)
    }
}

/// A signed refresh token (JWT string).
pub struct RefreshToken(pub String);

impl RefreshToken {
    /// Create a refresh token for a user.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Jwt`] if JWT encoding fails.
    pub fn for_user(
        user_id: Uuid,
        username: &str,
        is_admin: bool,
        config: &JwtConfig,
    ) -> Result<Self, AuthError> {
        let now = Utc::now().timestamp();
        let claims = RefreshTokenClaims {
            sub: user_id,
            jti: Uuid::now_v7().to_string(),
            iat: now,
            exp: now + config.refresh_token_ttl_secs,
            typ: REFRESH_TOKEN_TYPE.to_string(),
            user_id,
            username: username.to_string(),
            is_admin,
        };
        let token = jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(config.secret.as_bytes()),
        )?;
        Ok(Self(token))
    }

    /// Decode and validate a refresh token.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Jwt`] if the token is invalid, expired, or not a refresh token.
    pub fn decode(token: &str, config: &JwtConfig) -> Result<RefreshTokenClaims, AuthError> {
        let token_data = jsonwebtoken::decode::<RefreshTokenClaims>(
            token,
            &DecodingKey::from_secret(config.secret.as_bytes()),
            &Validation::default(),
        )?;
        if token_data.claims.typ != REFRESH_TOKEN_TYPE {
            return Err(AuthError::InvalidToken);
        }
        Ok(token_data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> JwtConfig {
        JwtConfig {
            secret: "test-secret-key-for-unit-tests".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        }
    }

    #[test]
    fn access_token_round_trip() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "testuser", false, &config).unwrap();
        let claims = AccessToken::decode(&token.0, &config).unwrap();

        assert_eq!(claims.user_id, user_id);
        assert_eq!(claims.username, "testuser");
        assert!(!claims.is_admin);
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.typ, "access");
    }

    #[test]
    fn access_token_admin_flag() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "admin", true, &config).unwrap();
        let claims = AccessToken::decode(&token.0, &config).unwrap();

        assert!(claims.is_admin);
    }

    #[test]
    fn access_token_expiry_from_config() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let before = Utc::now().timestamp();
        let token = AccessToken::for_user(user_id, "user", false, &config).unwrap();
        let claims = AccessToken::decode(&token.0, &config).unwrap();

        assert!(claims.iat >= before);
        assert_eq!(claims.exp - claims.iat, config.access_token_ttl_secs);
    }

    #[test]
    fn decode_invalid_token_fails() {
        let config = test_config();
        let result = AccessToken::decode("not.a.valid.token", &config);
        assert!(result.is_err());
    }

    #[test]
    fn decode_tampered_token_fails() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "user", false, &config).unwrap();
        let tampered = format!("{}x", &token.0[..token.0.len() - 1]);
        assert!(AccessToken::decode(&tampered, &config).is_err());
    }

    #[test]
    fn expired_token_fails() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let claims = AccessTokenClaims {
            sub: user_id,
            jti: Uuid::now_v7().to_string(),
            iat: Utc::now().timestamp() - 7200,
            exp: Utc::now().timestamp() - 3600,
            typ: "access".to_string(),
            user_id,
            username: "expired".to_string(),
            is_admin: false,
        };
        let token_str = jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(config.secret.as_bytes()),
        )
        .unwrap();
        assert!(AccessToken::decode(&token_str, &config).is_err());
    }

    #[test]
    fn refresh_token_round_trip() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let token = RefreshToken::for_user(user_id, "testuser", false, &config).unwrap();
        let claims = RefreshToken::decode(&token.0, &config).unwrap();

        assert_eq!(claims.user_id, user_id);
        assert_eq!(claims.username, "testuser");
        assert_eq!(claims.typ, "refresh");
        assert_eq!(claims.exp - claims.iat, config.refresh_token_ttl_secs);
    }

    #[test]
    fn refresh_token_cannot_be_used_as_access() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let refresh = RefreshToken::for_user(user_id, "user", false, &config).unwrap();
        assert!(AccessToken::decode(&refresh.0, &config).is_err());
    }

    #[test]
    fn access_token_cannot_be_used_as_refresh() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let access = AccessToken::for_user(user_id, "user", false, &config).unwrap();
        assert!(RefreshToken::decode(&access.0, &config).is_err());
    }
}
