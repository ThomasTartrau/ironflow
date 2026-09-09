//! Error conversion from [`slack_morphism`] errors to [`OperationError`].

use ironflow_core::error::OperationError;
use slack_morphism::errors::SlackClientError;

/// Convert a [`SlackClientError`] into an [`OperationError::External`].
pub(crate) fn from_slack_error(err: SlackClientError) -> OperationError {
    OperationError::External {
        origin: "slack".to_string(),
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use slack_morphism::errors::{SlackClientApiError, SlackClientError};

    use super::*;

    #[test]
    fn api_error_maps_to_external() {
        let slack_err =
            SlackClientError::ApiError(SlackClientApiError::new("channel_not_found".to_string()));
        let op_err = from_slack_error(slack_err);
        let msg = op_err.to_string();
        assert!(msg.contains("slack"), "expected 'slack' in: {msg}");
        assert!(
            msg.contains("channel_not_found"),
            "expected error code in: {msg}"
        );
    }

    #[test]
    fn rate_limit_error_maps_to_external() {
        let slack_err =
            SlackClientError::RateLimitError(slack_morphism::errors::SlackRateLimitError::new());
        let op_err = from_slack_error(slack_err);
        assert!(op_err.to_string().contains("slack"));
    }
}
