//! Repository operations: add, remove, update, list, index, search.

mod manage;
mod search;

pub use manage::{RepoAdd, RepoEntry, RepoIndex, RepoList, RepoRemove, RepoUpdate};
pub use search::{SearchHub, SearchRepo};
