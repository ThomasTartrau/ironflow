//! App operations.
//!
//! Wraps the Slack [`apps.*`](https://api.slack.com/methods?filter=apps) methods:
//! connections, manifest management.

use slack_morphism::api::{
    SlackApiAppsConnectionOpenRequest, SlackApiAppsConnectionOpenResponse,
    SlackApiAppsManifestCreateRequest, SlackApiAppsManifestCreateResponse,
    SlackApiAppsManifestDeleteRequest, SlackApiAppsManifestExportRequest,
    SlackApiAppsManifestExportResponse, SlackApiAppsManifestUpdateRequest,
    SlackApiAppsManifestUpdateResponse, SlackApiAppsManifestValidateRequest,
};

use crate::macros::slack_op;

slack_op! {
    /// Open a WebSocket connection for Socket Mode.
    ///
    /// Wraps [`apps.connections.open`](https://api.slack.com/methods/apps.connections.open).
    AppsConnectionsOpen => apps_connections_open(
        SlackApiAppsConnectionOpenRequest
    ) -> SlackApiAppsConnectionOpenResponse
}

slack_op! {
    /// Create an app from a manifest.
    ///
    /// Wraps [`apps.manifest.create`](https://api.slack.com/methods/apps.manifest.create).
    AppsManifestCreate => apps_manifest_create(
        SlackApiAppsManifestCreateRequest
    ) -> SlackApiAppsManifestCreateResponse
}

slack_op! {
    /// Export an app manifest.
    ///
    /// Wraps [`apps.manifest.export`](https://api.slack.com/methods/apps.manifest.export).
    AppsManifestExport => apps_manifest_export(
        SlackApiAppsManifestExportRequest
    ) -> SlackApiAppsManifestExportResponse
}

slack_op! {
    /// Update an app manifest.
    ///
    /// Wraps [`apps.manifest.update`](https://api.slack.com/methods/apps.manifest.update).
    AppsManifestUpdate => apps_manifest_update(
        SlackApiAppsManifestUpdateRequest
    ) -> SlackApiAppsManifestUpdateResponse
}

slack_op! {
    /// Delete an app manifest.
    ///
    /// Wraps [`apps.manifest.delete`](https://api.slack.com/methods/apps.manifest.delete).
    AppsManifestDelete => apps_manifest_delete(
        SlackApiAppsManifestDeleteRequest
    ) -> ()
}

slack_op! {
    /// Validate an app manifest.
    ///
    /// Wraps [`apps.manifest.validate`](https://api.slack.com/methods/apps.manifest.validate).
    AppsManifestValidate => apps_manifest_validate(
        SlackApiAppsManifestValidateRequest
    ) -> ()
}
