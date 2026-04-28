//! Typed error for the ironflow runtime.

use thiserror::Error;
use tokio_cron_scheduler::JobSchedulerError;

/// Error returned by [`Runtime::serve`](crate::runtime::Runtime::serve) and
/// [`Runtime::run_crons`](crate::runtime::Runtime::run_crons).
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

    #[test]
    fn cron_error_display() {
        // Create a cron error through the From implementation
        // by providing a JobSchedulerError (ParseSchedule variant for cron parse failures)
        let err = RuntimeError::Cron(JobSchedulerError::ParseSchedule);
        let msg = err.to_string();
        assert!(msg.contains("cron scheduler error"));
    }

    #[test]
    fn cron_error_from_impl() {
        // Test From implementation: JobSchedulerError -> RuntimeError::Cron
        let cron_err = JobSchedulerError::ParseSchedule;
        let err: RuntimeError = cron_err.into();
        let msg = err.to_string();
        assert!(msg.contains("cron scheduler error"));
    }

    #[test]
    fn shutdown_error_display() {
        // Test Shutdown variant with a JobSchedulerError
        let err = RuntimeError::Shutdown(JobSchedulerError::ParseSchedule);
        let msg = err.to_string();
        assert!(msg.contains("scheduler shutdown error"));
    }

    #[test]
    fn cron_error_debug() {
        let err = RuntimeError::Cron(JobSchedulerError::ParseSchedule);
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Cron"));
    }

    #[test]
    fn shutdown_error_debug() {
        let err = RuntimeError::Shutdown(JobSchedulerError::ParseSchedule);
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Shutdown"));
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
