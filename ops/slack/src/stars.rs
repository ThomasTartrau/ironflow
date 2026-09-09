//! Star operations.
//!
//! Wraps the Slack [`stars.*`](https://api.slack.com/methods?filter=stars)
//! methods: add and remove.

use slack_morphism::api::{
    SlackApiStarsAddRequest, SlackApiStarsAddResponse, SlackApiStarsRemoveRequest,
    SlackApiStarsRemoveResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// Star a message or file.
    ///
    /// Wraps [`stars.add`](https://api.slack.com/methods/stars.add).
    StarsAdd => stars_add(
        SlackApiStarsAddRequest
    ) -> SlackApiStarsAddResponse
}

slack_op! {
    /// Remove a star from a message or file.
    ///
    /// Wraps [`stars.remove`](https://api.slack.com/methods/stars.remove).
    StarsRemove => stars_remove(
        SlackApiStarsRemoveRequest
    ) -> SlackApiStarsRemoveResponse
}
