//! User group operations.
//!
//! Wraps the Slack [`usergroups.*`](https://api.slack.com/methods?filter=usergroups)
//! methods: list, update, and users list.

use slack_morphism::api::{
    SlackApiUserGroupsListRequest, SlackApiUserGroupsListResponse, SlackApiUserGroupsUpdateRequest,
    SlackApiUserGroupsUpdateResponse, SlackApiUserGroupsUsersListRequest,
    SlackApiUserGroupsUsersListResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// List user groups in a workspace.
    ///
    /// Wraps [`usergroups.list`](https://api.slack.com/methods/usergroups.list).
    UserGroupsList => usergroups_list(
        SlackApiUserGroupsListRequest
    ) -> SlackApiUserGroupsListResponse
}

slack_op! {
    /// Update a user group.
    ///
    /// Wraps [`usergroups.update`](https://api.slack.com/methods/usergroups.update).
    UserGroupsUpdate => usergroups_update(
        SlackApiUserGroupsUpdateRequest
    ) -> SlackApiUserGroupsUpdateResponse
}

slack_op! {
    /// List users in a user group.
    ///
    /// Wraps [`usergroups.users.list`](https://api.slack.com/methods/usergroups.users.list).
    UserGroupsUsersList => usergroups_users_list(
        SlackApiUserGroupsUsersListRequest
    ) -> SlackApiUserGroupsUsersListResponse
}
