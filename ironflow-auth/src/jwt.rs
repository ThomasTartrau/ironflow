//! JWT access and refresh token management.
//!
//! Access tokens are short-lived (default 15 min) and used for API requests.
//! Refresh tokens are long-lived (default 7 days) and used to obtain new access tokens.
//! Token types are enforced — a refresh token cannot be used as an access token.

use chrono::Utc;
use hex::encode;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    /// Session generation of the user when the token was issued.
    #[serde(default)]
    pub ver: i64,
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
    /// Session generation of the user when the token was issued.
    #[serde(default)]
    pub ver: i64,
}

/// A signed access token (JWT string).
pub struct AccessToken(pub String);

impl AccessToken {
    /// Create an access token for a user, at session generation 0.
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
        Self::for_user_with_version(user_id, username, is_admin, 0, config)
    }

    /// Create an access token for a user at a given session generation.
    ///
    /// `token_version` is the user's current `token_version`: the token is
    /// rejected once the stored version moves past it.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Jwt`] if JWT encoding fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_auth::jwt::{AccessToken, JwtConfig};
    /// use uuid::Uuid;
    /// # use ironflow_auth::error::AuthError;
    ///
    /// # fn example() -> Result<(), AuthError> {
    /// let config = JwtConfig {
    ///     secret: "my-secret-key".to_string(),
    ///     access_token_ttl_secs: 900,
    ///     refresh_token_ttl_secs: 604800,
    ///     cookie_domain: None,
    ///     cookie_secure: false,
    /// };
    /// let token = AccessToken::for_user_with_version(Uuid::now_v7(), "alice", false, 3, &config)?;
    /// let claims = AccessToken::decode(&token.0, &config)?;
    /// assert_eq!(claims.ver, 3);
    /// # Ok(())
    /// # }
    /// # example().expect("example");
    /// ```
    pub fn for_user_with_version(
        user_id: Uuid,
        username: &str,
        is_admin: bool,
        token_version: i64,
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
            ver: token_version,
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
    /// Create a refresh token for a user, at session generation 0.
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
        Self::for_user_with_version(user_id, username, is_admin, 0, config)
    }

    /// Create a refresh token for a user at a given session generation.
    ///
    /// `token_version` is the user's current `token_version`: the token is
    /// rejected once the stored version moves past it.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Jwt`] if JWT encoding fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_auth::jwt::{JwtConfig, RefreshToken};
    /// use uuid::Uuid;
    /// # use ironflow_auth::error::AuthError;
    ///
    /// # fn example() -> Result<(), AuthError> {
    /// let config = JwtConfig {
    ///     secret: "my-secret-key".to_string(),
    ///     access_token_ttl_secs: 900,
    ///     refresh_token_ttl_secs: 604800,
    ///     cookie_domain: None,
    ///     cookie_secure: false,
    /// };
    /// let token = RefreshToken::for_user_with_version(Uuid::now_v7(), "alice", false, 3, &config)?;
    /// let claims = RefreshToken::decode(&token.0, &config)?;
    /// assert_eq!(claims.ver, 3);
    /// # Ok(())
    /// # }
    /// # example().expect("example");
    /// ```
    pub fn for_user_with_version(
        user_id: Uuid,
        username: &str,
        is_admin: bool,
        token_version: i64,
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
            ver: token_version,
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

/// Hash a raw refresh token for storage.
///
/// Returns the lowercase hex SHA-256 of `raw`. Only this hash is persisted,
/// so a leaked database row cannot be replayed as a token.
///
/// # Examples
///
/// ```
/// use ironflow_auth::jwt::token_hash;
///
/// let hash = token_hash("some.jwt.value");
/// assert_eq!(hash.len(), 64);
/// assert_eq!(hash, token_hash("some.jwt.value"));
/// ```
pub fn token_hash(raw: &str) -> String {
    encode(Sha256::digest(raw.as_bytes()))
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
            ver: 0,
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

    #[test]
    fn decode_with_wrong_secret_fails() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user(user_id, "user", false, &config).unwrap();

        let other_config = JwtConfig {
            secret: "different-secret".to_string(),
            ..test_config()
        };
        assert!(AccessToken::decode(&token.0, &other_config).is_err());
    }

    #[test]
    fn refresh_token_decode_with_wrong_secret_fails() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let token = RefreshToken::for_user(user_id, "user", false, &config).unwrap();

        let other_config = JwtConfig {
            secret: "different-secret".to_string(),
            ..test_config()
        };
        assert!(RefreshToken::decode(&token.0, &other_config).is_err());
    }

    #[test]
    fn access_token_claims_serde_roundtrip() {
        let user_id = Uuid::now_v7();
        let claims = AccessTokenClaims {
            sub: user_id,
            jti: "jti-123".to_string(),
            iat: 1000,
            exp: 2000,
            typ: "access".to_string(),
            user_id,
            username: "alice".to_string(),
            is_admin: true,
            ver: 0,
        };
        let json = serde_json::to_string(&claims).unwrap();
        let decoded: AccessTokenClaims = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.sub, user_id);
        assert_eq!(decoded.jti, "jti-123");
        assert_eq!(decoded.typ, "access");
        assert_eq!(decoded.username, "alice");
        assert!(decoded.is_admin);
    }

    #[test]
    fn refresh_token_claims_serde_roundtrip() {
        let user_id = Uuid::now_v7();
        let claims = RefreshTokenClaims {
            sub: user_id,
            jti: "jti-456".to_string(),
            iat: 1000,
            exp: 2000,
            typ: "refresh".to_string(),
            user_id,
            username: "bob".to_string(),
            is_admin: false,
            ver: 0,
        };
        let json = serde_json::to_string(&claims).unwrap();
        let decoded: RefreshTokenClaims = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.sub, user_id);
        assert_eq!(decoded.typ, "refresh");
        assert_eq!(decoded.username, "bob");
        assert!(!decoded.is_admin);
    }

    #[test]
    fn refresh_token_expiry_from_config() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let before = Utc::now().timestamp();
        let token = RefreshToken::for_user(user_id, "user", false, &config).unwrap();
        let claims = RefreshToken::decode(&token.0, &config).unwrap();

        assert!(claims.iat >= before);
        assert_eq!(claims.exp - claims.iat, config.refresh_token_ttl_secs);
    }

    #[test]
    fn refresh_token_admin_flag() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let token = RefreshToken::for_user(user_id, "admin", true, &config).unwrap();
        let claims = RefreshToken::decode(&token.0, &config).unwrap();

        assert!(claims.is_admin);
    }

    #[test]
    fn access_token_unique_jti_per_token() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let t1 = AccessToken::for_user(user_id, "user", false, &config).unwrap();
        let t2 = AccessToken::for_user(user_id, "user", false, &config).unwrap();

        let c1 = AccessToken::decode(&t1.0, &config).unwrap();
        let c2 = AccessToken::decode(&t2.0, &config).unwrap();
        assert_ne!(c1.jti, c2.jti);
    }

    #[test]
    fn jwt_config_clone() {
        let config = test_config();
        let cloned = config.clone();
        assert_eq!(cloned.secret, config.secret);
        assert_eq!(cloned.access_token_ttl_secs, config.access_token_ttl_secs);
        assert_eq!(cloned.refresh_token_ttl_secs, config.refresh_token_ttl_secs);
        assert_eq!(cloned.cookie_domain, config.cookie_domain);
        assert_eq!(cloned.cookie_secure, config.cookie_secure);
    }

    #[test]
    fn jwt_config_with_cookie_domain() {
        let config = JwtConfig {
            cookie_domain: Some(".example.com".to_string()),
            cookie_secure: true,
            ..test_config()
        };
        assert_eq!(config.cookie_domain.as_deref(), Some(".example.com"));
        assert!(config.cookie_secure);
    }

    #[test]
    fn access_token_carries_version() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let token = AccessToken::for_user_with_version(user_id, "user", true, 7, &config).unwrap();
        let claims = AccessToken::decode(&token.0, &config).unwrap();

        assert_eq!(claims.ver, 7);
        assert_eq!(claims.user_id, user_id);
        assert!(claims.is_admin);

        let legacy = AccessToken::for_user(user_id, "user", false, &config).unwrap();
        assert_eq!(AccessToken::decode(&legacy.0, &config).unwrap().ver, 0);
    }

    #[test]
    fn refresh_token_carries_version() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let token =
            RefreshToken::for_user_with_version(user_id, "user", false, 4, &config).unwrap();
        let claims = RefreshToken::decode(&token.0, &config).unwrap();

        assert_eq!(claims.ver, 4);
        assert_eq!(claims.user_id, user_id);

        let legacy = RefreshToken::for_user(user_id, "user", false, &config).unwrap();
        assert_eq!(RefreshToken::decode(&legacy.0, &config).unwrap().ver, 0);
    }

    /// Claims as encoded before `ver` existed.
    #[derive(Serialize)]
    struct LegacyClaims {
        sub: Uuid,
        jti: String,
        iat: i64,
        exp: i64,
        typ: String,
        user_id: Uuid,
        username: String,
        is_admin: bool,
    }

    #[test]
    fn token_without_ver_claim_decodes_as_zero() {
        let config = test_config();
        let user_id = Uuid::now_v7();
        let now = Utc::now().timestamp();
        let legacy = |typ: &str| {
            jsonwebtoken::encode(
                &Header::default(),
                &LegacyClaims {
                    sub: user_id,
                    jti: Uuid::now_v7().to_string(),
                    iat: now,
                    exp: now + 600,
                    typ: typ.to_string(),
                    user_id,
                    username: "legacy".to_string(),
                    is_admin: false,
                },
                &EncodingKey::from_secret(config.secret.as_bytes()),
            )
            .unwrap()
        };

        let access = AccessToken::decode(&legacy("access"), &config).unwrap();
        assert_eq!(access.ver, 0);
        assert_eq!(access.username, "legacy");

        let refresh = RefreshToken::decode(&legacy("refresh"), &config).unwrap();
        assert_eq!(refresh.ver, 0);
    }

    #[test]
    fn token_hash_is_stable_sha256_hex() {
        let hash = token_hash("a.b.c");

        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert_eq!(hash, token_hash("a.b.c"));
        assert_ne!(hash, token_hash("a.b.d"));
        // SHA-256 of the empty string.
        assert_eq!(
            token_hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
