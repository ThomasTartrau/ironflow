//! File operations.
//!
//! Wraps the Slack [`files.*`](https://api.slack.com/methods?filter=files) methods:
//! upload, info, list, delete, and the external upload flow.

use slack_morphism::api::{
    SlackApiFilesCompleteUploadExternalRequest, SlackApiFilesCompleteUploadExternalResponse,
    SlackApiFilesDeleteRequest, SlackApiFilesDeleteResponse,
    SlackApiFilesGetUploadUrlExternalRequest, SlackApiFilesGetUploadUrlExternalResponse,
    SlackApiFilesInfoRequest, SlackApiFilesInfoResponse, SlackApiFilesListRequest,
    SlackApiFilesListResponse, SlackApiFilesUploadRequest, SlackApiFilesUploadResponse,
    SlackApiFilesUploadViaUrlRequest, SlackApiFilesUploadViaUrlResponse,
};

use crate::macros::slack_op;

slack_op! {
    /// Get info about a file.
    ///
    /// Wraps [`files.info`](https://api.slack.com/methods/files.info).
    FilesInfo => files_info(
        SlackApiFilesInfoRequest
    ) -> SlackApiFilesInfoResponse
}

slack_op! {
    /// List files shared in a workspace.
    ///
    /// Wraps [`files.list`](https://api.slack.com/methods/files.list).
    FilesList => files_list(
        SlackApiFilesListRequest
    ) -> SlackApiFilesListResponse
}

slack_op! {
    /// Upload a file (legacy).
    ///
    /// Wraps [`files.upload`](https://api.slack.com/methods/files.upload).
    ///
    /// **Deprecated by Slack.** Prefer [`FilesGetUploadUrlExternal`] +
    /// [`FilesUploadViaUrl`] + [`FilesCompleteUploadExternal`] instead.
    FilesUpload => files_upload(
        SlackApiFilesUploadRequest
    ) -> SlackApiFilesUploadResponse
}

slack_op! {
    /// Get an external upload URL.
    ///
    /// Wraps [`files.getUploadURLExternal`](https://api.slack.com/methods/files.getUploadURLExternal).
    FilesGetUploadUrlExternal => get_upload_url_external(
        SlackApiFilesGetUploadUrlExternalRequest
    ) -> SlackApiFilesGetUploadUrlExternalResponse
}

slack_op! {
    /// Upload a file via an external URL.
    ///
    /// Wraps the second step of the external upload flow.
    FilesUploadViaUrl => files_upload_via_url(
        SlackApiFilesUploadViaUrlRequest
    ) -> SlackApiFilesUploadViaUrlResponse
}

slack_op! {
    /// Complete an external upload.
    ///
    /// Wraps [`files.completeUploadExternal`](https://api.slack.com/methods/files.completeUploadExternal).
    FilesCompleteUploadExternal => files_complete_upload_external(
        SlackApiFilesCompleteUploadExternalRequest
    ) -> SlackApiFilesCompleteUploadExternalResponse
}

slack_op! {
    /// Delete a file.
    ///
    /// Wraps [`files.delete`](https://api.slack.com/methods/files.delete).
    FilesDelete => files_delete(
        SlackApiFilesDeleteRequest
    ) -> SlackApiFilesDeleteResponse
}
