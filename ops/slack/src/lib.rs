//! Slack integration for Ironflow workflows.
//!
//! This crate provides typed Slack API operations as Ironflow
//! [`Operation`](ironflow_core::operation::Operation) implementations. Each
//! operation wraps a single [`slack_morphism`] API method and returns the
//! typed response.
//!
//! # Architecture
//!
//! - [`SlackClient`] is the central handle, wrapping a bot token and HTTPS connector
//! - Each operation is a standalone struct implementing [`Operation`](ironflow_core::operation::Operation)
//! - All operations return `kind() == "slack"`
//! - Parameters are set at construction time via slack-morphism request types
//!
//! # Quick start
//!
//! ```no_run
//! use ironflow_ops_slack::SlackClient;
//! use ironflow_ops_slack::chat::ChatPostMessage;
//! use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
//! use slack_morphism::api::SlackApiChatPostMessageRequest;
//! use slack_morphism::{SlackChannelId, SlackMessageContent};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ironflow_core::error::OperationError> {
//! let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
//! let slack = SlackClient::from_context(&ctx).await?;
//!
//! let req = SlackApiChatPostMessageRequest::new(
//!     SlackChannelId::new("#general".to_string()),
//!     SlackMessageContent::new().with_text("Hello from Ironflow".to_string()),
//! );
//! let op = ChatPostMessage::new(&slack, req);
//! // op implements Operation -- pass it to ctx.operation("post-msg", &op)
//! # Ok(())
//! # }
//! ```
//!
//! # Tracked operations
//!
//! Every operation implements [`Operation`](ironflow_core::operation::Operation),
//! so it can be passed to `WorkflowContext::operation()` for step lifecycle
//! tracking (step record, status transitions, duration, output persistence).
//!
//! # Modules
//!
//! Operations are organized by Slack API domain:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`apps`] | ConnectionsOpen, ManifestCreate, ManifestExport, ManifestUpdate, ManifestDelete, ManifestValidate |
//! | [`assistant`] | ThreadsSetStatus, ThreadsSetSuggestedPrompts, ThreadsSetTitle |
//! | [`auth`] | AuthTest |
//! | [`bots`] | BotsInfo |
//! | [`chat`] | PostMessage, PostEphemeral, Update, Delete, ScheduleMessage, DeleteScheduledMessage, ScheduledMessagesList, GetPermalink, Unfurl |
//! | [`conversations`] | Create, Archive, Unarchive, Rename, SetTopic, SetPurpose, Info, List, History, Replies, Members, Join, Leave, Invite, Kick, Open, OpenFull, Close |
//! | [`emoji`] | EmojiList |
//! | [`files`] | Info, List, Upload, GetUploadUrlExternal, UploadViaUrl, CompleteUploadExternal, Delete |
//! | [`pins`] | Add, Remove, List |
//! | [`reactions`] | Add, Remove, Get |
//! | [`stars`] | Add, Remove |
//! | [`team`] | Info, ProfileGet |
//! | [`test_api`] | ApiTest |
//! | [`usergroups`] | List, Update, UsersList |
//! | [`users`] | Info, List, LookupByEmail, GetPresence, SetPresence, ProfileGet, ProfileSet, Conversations |
//! | [`views`] | Open, Push, Update, Publish |

pub mod apps;
pub mod assistant;
pub mod auth;
pub mod bots;
pub mod chat;
mod client;
pub mod conversations;
pub mod emoji;
pub(crate) mod error;
pub mod files;
mod macros;
pub mod pins;
pub mod reactions;
pub mod stars;
pub mod team;
pub mod usergroups;
pub mod users;
pub mod views;

/// API test operation.
///
/// Named `test_api` because `test` is a reserved word in Rust module paths.
#[path = "test.rs"]
pub mod test_api;

pub use client::SlackClient;
pub use slack_morphism;
