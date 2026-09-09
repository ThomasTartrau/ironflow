//! Conversation operations.
//!
//! Wraps the Slack [`conversations.*`](https://api.slack.com/methods?filter=conversations)
//! methods: create, archive, rename, history, members, invite, and more.

use slack_morphism::api::{
    SlackApiConversationsArchiveRequest, SlackApiConversationsArchiveResponse,
    SlackApiConversationsCloseRequest, SlackApiConversationsCloseResponse,
    SlackApiConversationsCreateRequest, SlackApiConversationsCreateResponse,
    SlackApiConversationsHistoryRequest, SlackApiConversationsHistoryResponse,
    SlackApiConversationsInfoRequest, SlackApiConversationsInfoResponse,
    SlackApiConversationsInviteRequest, SlackApiConversationsInviteResponse,
    SlackApiConversationsJoinRequest, SlackApiConversationsJoinResponse,
    SlackApiConversationsKickRequest, SlackApiConversationsKickResponse,
    SlackApiConversationsLeaveRequest, SlackApiConversationsLeaveResponse,
    SlackApiConversationsListRequest, SlackApiConversationsListResponse,
    SlackApiConversationsMembersRequest, SlackApiConversationsMembersResponse,
    SlackApiConversationsOpenRequest, SlackApiConversationsOpenResponse,
    SlackApiConversationsRenameRequest, SlackApiConversationsRenameResponse,
    SlackApiConversationsRepliesRequest, SlackApiConversationsRepliesResponse,
    SlackApiConversationsSetPurposeRequest, SlackApiConversationsSetPurposeResponse,
    SlackApiConversationsSetTopicRequest, SlackApiConversationsSetTopicResponse,
    SlackApiConversationsUnarchiveRequest, SlackApiConversationsUnarchiveResponse,
};
use slack_morphism::{SlackBasicChannelInfo, SlackChannelInfo};

use crate::macros::slack_op;

slack_op! {
    /// Create a new channel.
    ///
    /// Wraps [`conversations.create`](https://api.slack.com/methods/conversations.create).
    ConversationsCreate => conversations_create(
        SlackApiConversationsCreateRequest
    ) -> SlackApiConversationsCreateResponse
}

slack_op! {
    /// Archive a channel.
    ///
    /// Wraps [`conversations.archive`](https://api.slack.com/methods/conversations.archive).
    ConversationsArchive => conversations_archive(
        SlackApiConversationsArchiveRequest
    ) -> SlackApiConversationsArchiveResponse
}

slack_op! {
    /// Unarchive a channel.
    ///
    /// Wraps [`conversations.unarchive`](https://api.slack.com/methods/conversations.unarchive).
    ConversationsUnarchive => conversations_unarchive(
        SlackApiConversationsUnarchiveRequest
    ) -> SlackApiConversationsUnarchiveResponse
}

slack_op! {
    /// Rename a channel.
    ///
    /// Wraps [`conversations.rename`](https://api.slack.com/methods/conversations.rename).
    ConversationsRename => conversations_rename(
        SlackApiConversationsRenameRequest
    ) -> SlackApiConversationsRenameResponse
}

slack_op! {
    /// Set the topic of a channel.
    ///
    /// Wraps [`conversations.setTopic`](https://api.slack.com/methods/conversations.setTopic).
    ConversationsSetTopic => conversations_set_topic(
        SlackApiConversationsSetTopicRequest
    ) -> SlackApiConversationsSetTopicResponse
}

slack_op! {
    /// Set the purpose of a channel.
    ///
    /// Wraps [`conversations.setPurpose`](https://api.slack.com/methods/conversations.setPurpose).
    ConversationsSetPurpose => conversations_set_purpose(
        SlackApiConversationsSetPurposeRequest
    ) -> SlackApiConversationsSetPurposeResponse
}

slack_op! {
    /// Get information about a channel.
    ///
    /// Wraps [`conversations.info`](https://api.slack.com/methods/conversations.info).
    ConversationsInfo => conversations_info(
        SlackApiConversationsInfoRequest
    ) -> SlackApiConversationsInfoResponse
}

slack_op! {
    /// List all channels in a workspace.
    ///
    /// Wraps [`conversations.list`](https://api.slack.com/methods/conversations.list).
    ConversationsList => conversations_list(
        SlackApiConversationsListRequest
    ) -> SlackApiConversationsListResponse
}

slack_op! {
    /// Fetch the message history of a channel.
    ///
    /// Wraps [`conversations.history`](https://api.slack.com/methods/conversations.history).
    ConversationsHistory => conversations_history(
        SlackApiConversationsHistoryRequest
    ) -> SlackApiConversationsHistoryResponse
}

slack_op! {
    /// Fetch replies to a message thread.
    ///
    /// Wraps [`conversations.replies`](https://api.slack.com/methods/conversations.replies).
    ConversationsReplies => conversations_replies(
        SlackApiConversationsRepliesRequest
    ) -> SlackApiConversationsRepliesResponse
}

slack_op! {
    /// List members of a channel.
    ///
    /// Wraps [`conversations.members`](https://api.slack.com/methods/conversations.members).
    ConversationsMembers => conversations_members(
        SlackApiConversationsMembersRequest
    ) -> SlackApiConversationsMembersResponse
}

slack_op! {
    /// Join a channel.
    ///
    /// Wraps [`conversations.join`](https://api.slack.com/methods/conversations.join).
    ConversationsJoin => conversations_join(
        SlackApiConversationsJoinRequest
    ) -> SlackApiConversationsJoinResponse
}

slack_op! {
    /// Leave a channel.
    ///
    /// Wraps [`conversations.leave`](https://api.slack.com/methods/conversations.leave).
    ConversationsLeave => conversations_leave(
        SlackApiConversationsLeaveRequest
    ) -> SlackApiConversationsLeaveResponse
}

slack_op! {
    /// Invite users to a channel.
    ///
    /// Wraps [`conversations.invite`](https://api.slack.com/methods/conversations.invite).
    ConversationsInvite => conversations_invite(
        SlackApiConversationsInviteRequest
    ) -> SlackApiConversationsInviteResponse
}

slack_op! {
    /// Remove a user from a channel.
    ///
    /// Wraps [`conversations.kick`](https://api.slack.com/methods/conversations.kick).
    ConversationsKick => conversations_kick(
        SlackApiConversationsKickRequest
    ) -> SlackApiConversationsKickResponse
}

slack_op! {
    /// Open or resume a direct message or multi-party DM (basic channel info).
    ///
    /// Wraps [`conversations.open`](https://api.slack.com/methods/conversations.open).
    ///
    /// Returns [`SlackBasicChannelInfo`] in the response. For full channel info,
    /// use [`ConversationsOpenFull`].
    ConversationsOpen => conversations_open(
        SlackApiConversationsOpenRequest
    ) -> SlackApiConversationsOpenResponse<SlackBasicChannelInfo>
}

slack_op! {
    /// Open or resume a direct message or multi-party DM (full channel info).
    ///
    /// Wraps [`conversations.open`](https://api.slack.com/methods/conversations.open)
    /// with full channel details in the response.
    ConversationsOpenFull => conversations_open_full(
        SlackApiConversationsOpenRequest
    ) -> SlackApiConversationsOpenResponse<SlackChannelInfo>
}

slack_op! {
    /// Close a direct message or multi-party DM.
    ///
    /// Wraps [`conversations.close`](https://api.slack.com/methods/conversations.close).
    ConversationsClose => conversations_close(
        SlackApiConversationsCloseRequest
    ) -> SlackApiConversationsCloseResponse
}
