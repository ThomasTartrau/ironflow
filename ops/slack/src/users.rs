//! User operations.
//!
//! Wraps the Slack [`users.*`](https://api.slack.com/methods?filter=users) methods:
//! info, list, lookup, presence, profile, and conversations.

use slack_morphism::api::{
    SlackApiUsersConversationsRequest, SlackApiUsersConversationsResponse,
    SlackApiUsersGetPresenceRequest, SlackApiUsersGetPresenceResponse, SlackApiUsersInfoRequest,
    SlackApiUsersInfoResponse, SlackApiUsersListRequest, SlackApiUsersListResponse,
    SlackApiUsersLookupByEmailRequest, SlackApiUsersLookupByEmailResponse,
    SlackApiUsersProfileGetRequest, SlackApiUsersProfileGetResponse,
    SlackApiUsersProfileSetRequest, SlackApiUsersProfileSetResponse,
    SlackApiUsersSetPresenceRequest, SlackApiUsersSetPresenceResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// Get information about a user.
    ///
    /// Wraps [`users.info`](https://api.slack.com/methods/users.info).
    UsersInfo => users_info(
        SlackApiUsersInfoRequest
    ) -> SlackApiUsersInfoResponse
}

slack_op! {
    /// List all users in a workspace.
    ///
    /// Wraps [`users.list`](https://api.slack.com/methods/users.list).
    UsersList => users_list(
        SlackApiUsersListRequest
    ) -> SlackApiUsersListResponse
}

slack_op! {
    /// Find a user by email address.
    ///
    /// Wraps [`users.lookupByEmail`](https://api.slack.com/methods/users.lookupByEmail).
    UsersLookupByEmail => users_lookup_by_email(
        SlackApiUsersLookupByEmailRequest
    ) -> SlackApiUsersLookupByEmailResponse
}

slack_op! {
    /// Get user presence information.
    ///
    /// Wraps [`users.getPresence`](https://api.slack.com/methods/users.getPresence).
    UsersGetPresence => users_get_presence(
        SlackApiUsersGetPresenceRequest
    ) -> SlackApiUsersGetPresenceResponse
}

slack_op! {
    /// Set user presence.
    ///
    /// Wraps [`users.setPresence`](https://api.slack.com/methods/users.setPresence).
    UsersSetPresence => users_set_presence(
        SlackApiUsersSetPresenceRequest
    ) -> SlackApiUsersSetPresenceResponse
}

slack_op! {
    /// Get a user's profile.
    ///
    /// Wraps [`users.profile.get`](https://api.slack.com/methods/users.profile.get).
    UsersProfileGet => users_profile_get(
        SlackApiUsersProfileGetRequest
    ) -> SlackApiUsersProfileGetResponse
}

slack_op! {
    /// Set a user's profile fields.
    ///
    /// Wraps [`users.profile.set`](https://api.slack.com/methods/users.profile.set).
    UsersProfileSet => users_profile_set(
        SlackApiUsersProfileSetRequest
    ) -> SlackApiUsersProfileSetResponse
}

slack_op! {
    /// List conversations a user is a member of.
    ///
    /// Wraps [`users.conversations`](https://api.slack.com/methods/users.conversations).
    UsersConversations => users_conversations(
        SlackApiUsersConversationsRequest
    ) -> SlackApiUsersConversationsResponse
}
