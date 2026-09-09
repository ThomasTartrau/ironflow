//! Pin operations.
//!
//! Wraps the Slack [`pins.*`](https://api.slack.com/methods?filter=pins)
//! methods: add, remove, and list.

use slack_morphism::api::{
    SlackApiPinsAddRequest, SlackApiPinsAddResponse, SlackApiPinsListRequest,
    SlackApiPinsListResponse, SlackApiPinsRemoveRequest, SlackApiPinsRemoveResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// Pin a message to a channel.
    ///
    /// Wraps [`pins.add`](https://api.slack.com/methods/pins.add).
    PinsAdd => pins_add(
        SlackApiPinsAddRequest
    ) -> SlackApiPinsAddResponse
}

slack_op! {
    /// Remove a pinned message.
    ///
    /// Wraps [`pins.remove`](https://api.slack.com/methods/pins.remove).
    PinsRemove => pins_remove(
        SlackApiPinsRemoveRequest
    ) -> SlackApiPinsRemoveResponse
}

slack_op! {
    /// List pinned items in a channel.
    ///
    /// Wraps [`pins.list`](https://api.slack.com/methods/pins.list).
    PinsList => pins_list(
        SlackApiPinsListRequest
    ) -> SlackApiPinsListResponse
}
