//! Chat operations.
//!
//! Wraps the Slack [`chat.*`](https://api.slack.com/methods?filter=chat) methods:
//! post, update, delete, schedule, unfurl, and permalink.

use slack_morphism::api::{
    SlackApiChatDeleteRequest, SlackApiChatDeleteResponse,
    SlackApiChatDeleteScheduledMessageRequest, SlackApiChatDeleteScheduledMessageResponse,
    SlackApiChatGetPermalinkRequest, SlackApiChatGetPermalinkResponse,
    SlackApiChatPostEphemeralRequest, SlackApiChatPostEphemeralResponse,
    SlackApiChatPostMessageRequest, SlackApiChatPostMessageResponse,
    SlackApiChatScheduleMessageRequest, SlackApiChatScheduleMessageResponse,
    SlackApiChatScheduledMessagesListRequest, SlackApiChatScheduledMessagesListResponse,
    SlackApiChatUnfurlRequest, SlackApiChatUnfurlResponse, SlackApiChatUpdateRequest,
    SlackApiChatUpdateResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// Post a message to a channel.
    ///
    /// Wraps [`chat.postMessage`](https://api.slack.com/methods/chat.postMessage).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_slack::SlackClient;
    /// use ironflow_ops_slack::chat::ChatPostMessage;
    /// use slack_morphism::api::SlackApiChatPostMessageRequest;
    /// use slack_morphism::{SlackChannelId, SlackMessageContent};
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let slack = SlackClient::new("xoxb-token")?;
    /// let req = SlackApiChatPostMessageRequest::new(
    ///     SlackChannelId::new("#general".to_string()),
    ///     SlackMessageContent::new().with_text("Hello from Ironflow".to_string()),
    /// );
    /// let op = ChatPostMessage::new(&slack, req);
    /// let response = op.run().await?;
    /// # Ok(())
    /// # }
    /// ```
    ChatPostMessage => chat_post_message(
        SlackApiChatPostMessageRequest
    ) -> SlackApiChatPostMessageResponse
}

slack_op! {
    /// Post an ephemeral message visible only to a specific user.
    ///
    /// Wraps [`chat.postEphemeral`](https://api.slack.com/methods/chat.postEphemeral).
    ChatPostEphemeral => chat_post_ephemeral(
        SlackApiChatPostEphemeralRequest
    ) -> SlackApiChatPostEphemeralResponse
}

slack_op! {
    /// Update an existing message.
    ///
    /// Wraps [`chat.update`](https://api.slack.com/methods/chat.update).
    ChatUpdate => chat_update(
        SlackApiChatUpdateRequest
    ) -> SlackApiChatUpdateResponse
}

slack_op! {
    /// Delete a message.
    ///
    /// Wraps [`chat.delete`](https://api.slack.com/methods/chat.delete).
    ChatDelete => chat_delete(
        SlackApiChatDeleteRequest
    ) -> SlackApiChatDeleteResponse
}

slack_op! {
    /// Schedule a message for later delivery.
    ///
    /// Wraps [`chat.scheduleMessage`](https://api.slack.com/methods/chat.scheduleMessage).
    ChatScheduleMessage => chat_schedule_message(
        SlackApiChatScheduleMessageRequest
    ) -> SlackApiChatScheduleMessageResponse
}

slack_op! {
    /// Delete a scheduled message before it is sent.
    ///
    /// Wraps [`chat.deleteScheduledMessage`](https://api.slack.com/methods/chat.deleteScheduledMessage).
    ChatDeleteScheduledMessage => chat_delete_scheduled_message(
        SlackApiChatDeleteScheduledMessageRequest
    ) -> SlackApiChatDeleteScheduledMessageResponse
}

slack_op! {
    /// List scheduled messages.
    ///
    /// Wraps [`chat.scheduledMessages.list`](https://api.slack.com/methods/chat.scheduledMessages.list).
    ChatScheduledMessagesList => chat_scheduled_messages_list(
        SlackApiChatScheduledMessagesListRequest
    ) -> SlackApiChatScheduledMessagesListResponse
}

slack_op! {
    /// Retrieve a permalink URL for a specific message.
    ///
    /// Wraps [`chat.getPermalink`](https://api.slack.com/methods/chat.getPermalink).
    ChatGetPermalink => chat_get_permalink(
        SlackApiChatGetPermalinkRequest
    ) -> SlackApiChatGetPermalinkResponse
}

slack_op! {
    /// Provide custom unfurl behavior for URLs posted in messages.
    ///
    /// Wraps [`chat.unfurl`](https://api.slack.com/methods/chat.unfurl).
    ChatUnfurl => chat_unfurl(
        SlackApiChatUnfurlRequest
    ) -> SlackApiChatUnfurlResponse
}
