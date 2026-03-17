//! Typed error for the ironflow runtime.

use thiserror::Error;
use tokio_cron_scheduler::JobSchedulerError;

/// Error returned by [`Runtime::serve`](crate::runtime::Runtime::serve).
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Failed to bind the TCP listener to the requested address.
    #[error("failed to bind address: {0}")]
    Bind(std::io::Error),
    /// A cron expression is invalid or the scheduler failed to start.
    #[error("cron scheduler error: {0}")]
    Cron(#[from] JobSchedulerError),
    /// The Axum server encountered a fatal I/O error.
    #[error("http server error: {0}")]
    Serve(std::io::Error),
    /// The cron scheduler failed to shut down cleanly.
    #[error("scheduler shutdown error: {0}")]
    Shutdown(JobSchedulerError),
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
}
