//! Reaction operations.
//!
//! Wraps the Slack [`reactions.*`](https://api.slack.com/methods?filter=reactions)
//! methods: add, remove, and get.

use slack_morphism::api::{
    SlackApiReactionsAddRequest, SlackApiReactionsAddResponse, SlackApiReactionsGetRequest,
    SlackApiReactionsGetResponse, SlackApiReactionsRemoveRequest, SlackApiReactionsRemoveResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// Add a reaction to a message.
    ///
    /// Wraps [`reactions.add`](https://api.slack.com/methods/reactions.add).
    ReactionsAdd => reactions_add(
        SlackApiReactionsAddRequest
    ) -> SlackApiReactionsAddResponse
}

slack_op! {
    /// Remove a reaction from a message.
    ///
    /// Wraps [`reactions.remove`](https://api.slack.com/methods/reactions.remove).
    ReactionsRemove => reactions_remove(
        SlackApiReactionsRemoveRequest
    ) -> SlackApiReactionsRemoveResponse
}

slack_op! {
    /// Get reactions for a message.
    ///
    /// Wraps [`reactions.get`](https://api.slack.com/methods/reactions.get).
    ReactionsGet => reactions_get(
        SlackApiReactionsGetRequest
    ) -> SlackApiReactionsGetResponse
}
