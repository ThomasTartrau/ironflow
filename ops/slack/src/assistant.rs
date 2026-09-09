//! Assistant thread operations.
//!
//! Wraps the Slack [`assistant.threads.*`](https://api.slack.com/methods?filter=assistant)
//! methods: set status, set suggested prompts, and set title.

use slack_morphism::api::{
    SlackApiAssistantThreadSetTitleRequest, SlackApiAssistantThreadSetTitleResponse,
    SlackApiAssistantThreadsSetStatusRequest, SlackApiAssistantThreadsSetStatusResponse,
    SlackApiAssistantThreadsSetSuggestedPromptsRequest,
    SlackApiAssistantThreadsSetSuggestedPromptsResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// Set the status of an assistant thread.
    ///
    /// Wraps [`assistant.threads.setStatus`](https://api.slack.com/methods/assistant.threads.setStatus).
    AssistantThreadsSetStatus => assistant_threads_set_status(
        SlackApiAssistantThreadsSetStatusRequest
    ) -> SlackApiAssistantThreadsSetStatusResponse
}

slack_op! {
    /// Set suggested prompts for an assistant thread.
    ///
    /// Wraps [`assistant.threads.setSuggestedPrompts`](https://api.slack.com/methods/assistant.threads.setSuggestedPrompts).
    AssistantThreadsSetSuggestedPrompts => assistant_threads_set_suggested_prompts(
        SlackApiAssistantThreadsSetSuggestedPromptsRequest
    ) -> SlackApiAssistantThreadsSetSuggestedPromptsResponse
}

slack_op! {
    /// Set the title of an assistant thread.
    ///
    /// Wraps [`assistant.threads.setTitle`](https://api.slack.com/methods/assistant.threads.setTitle).
    AssistantThreadsSetTitle => assistant_threads_set_title(
        SlackApiAssistantThreadSetTitleRequest
    ) -> SlackApiAssistantThreadSetTitleResponse
}
