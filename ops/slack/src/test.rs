//! API test operation.
//!
//! Wraps the Slack [`api.test`](https://api.slack.com/methods/api.test) method,
//! useful for checking connectivity.

use slack_morphism::api::{SlackApiTestRequest, SlackApiTestResponse};

use crate::macros::slack_op;

slack_op! {
    /// Test the Slack API connection.
    ///
    /// Wraps [`api.test`](https://api.slack.com/methods/api.test).
    ApiTest => api_test(
        SlackApiTestRequest
    ) -> SlackApiTestResponse
}
