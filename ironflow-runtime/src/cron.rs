//! Internal representation of a scheduled cron job.
//!
//! This module is `pub(crate)` and not part of the public API. Cron jobs are
//! registered through [`Runtime::cron`](crate::runtime::Runtime::cron) and
//! executed by the [`tokio_cron_scheduler`] scheduler that
//! [`Runtime::serve`](crate::runtime::Runtime::serve) or
//! [`Runtime::run_crons`](crate::runtime::Runtime::run_crons) starts.

use std::future::Future;
use std::pin::Pin;

/// A cron job definition holding its schedule expression, display name, and
/// async handler function.
pub(crate) struct CronJob {
    /// A cron expression (6-field format) such as `"0 */5 * * * *"`.
    pub schedule: String,
    /// A human-readable name used in log messages.
    pub name: String,
    /// The async function to execute on each tick.
    pub handler: Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>,
}
