//! Hand-written [`Endpoint`](gitlab::api::Endpoint) implementations for GitLab API v4
//! routes that are missing from the [`gitlab`] crate: merge request discussion replies and
//! versions, issue and commit discussions, single note reads, releases, file uploads, job traces and the
//! merge base.

pub mod discussions;
pub mod jobs;
pub mod merge_requests;
pub mod notes;
pub mod releases;
pub mod repository;
pub mod uploads;
