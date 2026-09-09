//! View and modal operations.
//!
//! Wraps the Slack [`views.*`](https://api.slack.com/methods?filter=views)
//! methods: open, push, update, and publish.

use slack_morphism::api::{
    SlackApiViewsOpenRequest, SlackApiViewsOpenResponse, SlackApiViewsPublishRequest,
    SlackApiViewsPublishResponse, SlackApiViewsPushRequest, SlackApiViewsPushResponse,
    SlackApiViewsUpdateRequest, SlackApiViewsUpdateResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// Open a modal view.
    ///
    /// Wraps [`views.open`](https://api.slack.com/methods/views.open).
    ViewsOpen => views_open(
        SlackApiViewsOpenRequest
    ) -> SlackApiViewsOpenResponse
}

slack_op! {
    /// Push a new view onto an existing modal stack.
    ///
    /// Wraps [`views.push`](https://api.slack.com/methods/views.push).
    ViewsPush => views_push(
        SlackApiViewsPushRequest
    ) -> SlackApiViewsPushResponse
}

slack_op! {
    /// Update an existing modal view.
    ///
    /// Wraps [`views.update`](https://api.slack.com/methods/views.update).
    ViewsUpdate => views_update(
        SlackApiViewsUpdateRequest
    ) -> SlackApiViewsUpdateResponse
}

slack_op! {
    /// Publish a Home tab view for a user.
    ///
    /// Wraps [`views.publish`](https://api.slack.com/methods/views.publish).
    ViewsPublish => views_publish(
        SlackApiViewsPublishRequest
    ) -> SlackApiViewsPublishResponse
}
