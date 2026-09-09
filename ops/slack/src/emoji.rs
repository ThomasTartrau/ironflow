//! Emoji operations.
//!
//! Wraps the Slack [`emoji.*`](https://api.slack.com/methods?filter=emoji) methods.

use slack_morphism::api::SlackApiEmojiListResponse;

use crate::macros::slack_op;

slack_op! {
    /// List custom emoji in a workspace.
    ///
    /// Wraps [`emoji.list`](https://api.slack.com/methods/emoji.list).
    EmojiList => emoji_list() -> SlackApiEmojiListResponse
}
