//! Bot operations.
//!
//! Wraps the Slack [`bots.*`](https://api.slack.com/methods?filter=bots) methods.

use slack_morphism::api::{SlackApiBotsInfoRequest, SlackApiBotsInfoResponse};

use crate::macros::slack_op;

slack_op! {
    /// Get information about a bot user.
    ///
    /// Wraps [`bots.info`](https://api.slack.com/methods/bots.info).
    BotsInfo => bots_info(
        SlackApiBotsInfoRequest
    ) -> SlackApiBotsInfoResponse
}
