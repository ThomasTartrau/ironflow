//! Error conversion helpers for mapping AWS SDK errors to [`OperationError`].

use std::fmt::Display;

use aws_sdk_s3::error::SdkError;
use ironflow_core::error::OperationError;

/// Convert any AWS SDK error into an [`OperationError::Http`].
pub(crate) fn sdk_err<E: Display, R>(err: SdkError<E, R>) -> OperationError {
    OperationError::Http {
        status: None,
        message: format!("S3 error: {err}"),
    }
}
