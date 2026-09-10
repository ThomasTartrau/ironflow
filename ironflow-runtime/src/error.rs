//! Typed error for the ironflow runtime.

use thiserror::Error;

/// Error returned by [`Runtime::serve`](crate::runtime::Runtime::serve).
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Failed to bind the TCP listener to the requested address.
    #[error("failed to bind address: {0}")]
    Bind(std::io::Error),
    /// The Axum server encountered a fatal I/O error.
    #[error("http server error: {0}")]
    Serve(std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_error_display() {
        let err = RuntimeError::Bind(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "port taken",
        ));
        assert_eq!(err.to_string(), "failed to bind address: port taken");
    }

    #[test]
    fn serve_error_display() {
        let err = RuntimeError::Serve(std::io::Error::other("fatal"));
        assert_eq!(err.to_string(), "http server error: fatal");
    }

    #[test]
    fn runtime_error_implements_std_error() {
        let err = RuntimeError::Bind(std::io::Error::other("x"));
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn bind_error_debug() {
        let err = RuntimeError::Bind(std::io::Error::other("test"));
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Bind"));
    }

    #[test]
    fn serve_error_debug() {
        let err = RuntimeError::Serve(std::io::Error::other("test"));
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Serve"));
    }

    #[test]
    fn error_variants_are_error() {
        use std::error::Error;

        let err_bind: Box<dyn Error> = Box::new(RuntimeError::Bind(std::io::Error::other("x")));
        let err_serve: Box<dyn Error> = Box::new(RuntimeError::Serve(std::io::Error::other("x")));

        assert!(!err_bind.to_string().is_empty());
        assert!(!err_serve.to_string().is_empty());
    }
}
