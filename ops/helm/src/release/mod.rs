//! Release operations: install, upgrade, uninstall, rollback, list, status,
//! history, get, test.

mod inspect;
mod install;
mod lifecycle;
mod query;
mod upgrade;

pub use inspect::{Get, GetSubcommand, Test};
pub use install::Install;
pub use lifecycle::{Rollback, Uninstall};
pub use query::{History, HistoryEntry, List, ReleaseEntry, Status};
pub use upgrade::Upgrade;
