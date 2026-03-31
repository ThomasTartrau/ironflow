//! Authentication error types.

use thiserror::Error;

/// Errors that can occur during authentication operations.
///
/// # Examples
///
/// ```
/// use ironflow_auth::error::AuthError;
///
/// let err = AuthError::MissingToken;
/// assert_eq!(err.to_string(), "no authentication token provided");
/// ```
#[derive(Debug, Error)]
pub enum AuthError {
    /// No token was provided in the request.
    #[error("no authentication token provided")]
    MissingToken,

    /// The provided token is invalid or expired.
    #[error("invalid or expired token")]
    InvalidToken,

    /// JWT encoding/decoding failed.
    #[error("jwt error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    /// Password hashing failed.
    #[error("password hashing error")]
    PasswordHash,

    /// Password verification failed (wrong password).
    #[error("invalid credentials")]
    InvalidCredentials,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        assert_eq!(
            AuthError::MissingToken.to_string(),
            "no authentication token provided"
        );
        assert_eq!(
            AuthError::InvalidToken.to_string(),
            "invalid or expired token"
        );
        assert_eq!(
            AuthError::InvalidCredentials.to_string(),
            "invalid credentials"
        );
    }
}
