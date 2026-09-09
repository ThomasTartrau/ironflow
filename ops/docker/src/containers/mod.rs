//! Container operations.
//!
//! Operations are grouped by concern:
//!
//! | Module | Operations |
//! |--------|-----------|
//! | [`create`] | Create |
//! | [`lifecycle`] | Start, Stop, Restart |
//! | [`manage`] | Kill, Remove, Inspect, List |
//! | [`exec`] | Logs, Exec, Wait |
//! | [`state`] | Pause, Unpause, Rename, Top |
//! | [`cleanup`] | Stats, Changes, Prune |

pub mod cleanup;
pub mod create;
pub mod exec;
pub mod lifecycle;
pub mod manage;
pub mod state;

pub use cleanup::*;
pub use create::*;
pub use exec::*;
pub use lifecycle::*;
pub use manage::*;
pub use state::*;

use bollard::Docker;

/// Wrapper enabling `impl Into<DockerRef>` on operation constructors.
///
/// This allows passing either a [`DockerClient`](crate::DockerClient) or a
/// `&DockerClient` to any operation constructor without explicit conversion.
pub struct DockerRef(pub(crate) Docker);

impl From<crate::DockerClient> for DockerRef {
    fn from(client: crate::DockerClient) -> Self {
        Self(client.into_inner())
    }
}

impl From<&crate::DockerClient> for DockerRef {
    fn from(client: &crate::DockerClient) -> Self {
        Self(client.docker().clone())
    }
}
