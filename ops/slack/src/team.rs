//! Team operations.
//!
//! Wraps the Slack [`team.*`](https://api.slack.com/methods?filter=team)
//! methods: info and profile.

use slack_morphism::api::{
    SlackApiTeamInfoRequest, SlackApiTeamInfoResponse, SlackApiTeamProfileGetRequest,
    SlackApiTeamProfileGetResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// Get information about a workspace.
    ///
    /// Wraps [`team.info`](https://api.slack.com/methods/team.info).
    TeamInfo => team_info(
        SlackApiTeamInfoRequest
    ) -> SlackApiTeamInfoResponse
}

slack_op! {
    /// Get a team's profile fields.
    ///
    /// Wraps [`team.profile.get`](https://api.slack.com/methods/team.profile.get).
    TeamProfileGet => team_profile_get(
        SlackApiTeamProfileGetRequest
    ) -> SlackApiTeamProfileGetResponse
}
