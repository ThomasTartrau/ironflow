//! # ironflow-auth
//!
//! Authentication library for the **ironflow** workflow engine.
//!
//! - JWT access + refresh tokens (HS256)
//! - Argon2id password hashing
//! - HttpOnly cookie management
//! - Axum `AuthenticatedUser` extractor
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_auth::jwt::{JwtConfig, AccessToken, RefreshToken};
//! use ironflow_auth::password;
//! use uuid::Uuid;
//!
//! # fn example() -> Result<(), ironflow_auth::error::AuthError> {
//! let config = JwtConfig {
//!     secret: "my-secret".to_string(),
//!     access_token_ttl_secs: 900,
//!     refresh_token_ttl_secs: 604800,
//!     cookie_domain: None,
//!     cookie_secure: false,
//! };
//!
//! let hash = password::hash("hunter2")?;
//! assert!(password::verify("hunter2", &hash)?);
//!
//! let user_id = Uuid::now_v7();
//! let access = AccessToken::for_user(user_id, "alice", false, &config)?;
//! let claims = AccessToken::decode(&access.0, &config)?;
//! assert_eq!(claims.user_id, user_id);
//! # Ok(())
//! # }
//! ```

pub mod cookies;
pub mod error;
pub mod extractor;
pub mod jwt;
pub mod middleware;
pub mod password;
