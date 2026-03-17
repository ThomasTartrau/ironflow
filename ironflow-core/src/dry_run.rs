//! Global dry-run mode control.
//!
//! When dry-run mode is active, operations log their intent without executing.
//! Shell commands are not spawned, HTTP requests are not sent, and Agent calls
//! return a placeholder response with zero cost.
//!
//! # Levels of control
//!
//! 1. **Per-operation** - call `.dry_run(true)` on any builder (`Shell`, `Agent`, `Http`).
//! 2. **Global** - call [`set_dry_run(true)`] to enable dry-run for all operations
//!    that don't have an explicit per-operation setting.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//!
//! # async fn example() -> Result<(), OperationError> {
//! // Global dry-run
//! set_dry_run(true);
//! let output = Shell::new("rm -rf /").await?; // not executed
//! assert_eq!(output.stdout(), "");
//!
//! // Per-operation dry-run (overrides global)
//! set_dry_run(false);
//! let output = Shell::new("echo hello").dry_run(true).await?;
//! assert_eq!(output.stdout(), "");
//! # Ok(())
//! # }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};

static DRY_RUN: AtomicBool = AtomicBool::new(false);

/// Enable or disable global dry-run mode.
///
/// When enabled, all operations that do not have an explicit per-operation
/// dry-run setting will use dry-run behavior.
///
/// # Thread safety
///
/// This sets a **process-wide** flag. If you run multiple workflows
/// concurrently, prefer the per-operation `.dry_run(true)` builder method
/// instead, which does not affect other workflows.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::dry_run::set_dry_run;
///
/// set_dry_run(true);  // all operations skip execution
/// set_dry_run(false); // back to normal
/// ```
pub fn set_dry_run(enabled: bool) {
    DRY_RUN.store(enabled, Ordering::Release);
}

/// Check whether global dry-run mode is currently enabled.
pub fn is_dry_run() -> bool {
    DRY_RUN.load(Ordering::Acquire)
}

/// RAII guard that sets global dry-run on creation and restores the previous
/// value on drop. Useful in tests to avoid leaking state.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::dry_run::DryRunGuard;
///
/// {
///     let _guard = DryRunGuard::new(true);
///     // operations here run in dry-run mode
/// }
/// // previous dry-run state restored
/// ```
pub struct DryRunGuard {
    previous: bool,
}

impl DryRunGuard {
    /// Enable or disable global dry-run for the lifetime of this guard.
    pub fn new(enabled: bool) -> Self {
        let previous = is_dry_run();
        set_dry_run(enabled);
        Self { previous }
    }
}

impl Drop for DryRunGuard {
    fn drop(&mut self) {
        set_dry_run(self.previous);
    }
}

/// Resolve the effective dry-run state for an operation.
///
/// If the operation has an explicit per-operation setting, use that.
/// Otherwise, fall back to the global setting.
pub(crate) fn effective_dry_run(per_operation: Option<bool>) -> bool {
    per_operation.unwrap_or_else(is_dry_run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize all dry-run tests since they share a global AtomicBool.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn set_and_get() {
        let _g = LOCK.lock().unwrap();
        set_dry_run(true);
        assert!(is_dry_run());
        set_dry_run(false);
        assert!(!is_dry_run());
    }

    #[test]
    fn effective_uses_per_operation_when_set() {
        let _g = LOCK.lock().unwrap();
        set_dry_run(false);
        assert!(effective_dry_run(Some(true)));

        set_dry_run(true);
        assert!(!effective_dry_run(Some(false)));
        set_dry_run(false);
    }

    #[test]
    fn effective_falls_back_to_global() {
        let _g = LOCK.lock().unwrap();
        set_dry_run(true);
        assert!(effective_dry_run(None));

        set_dry_run(false);
        assert!(!effective_dry_run(None));
    }

    #[test]
    fn guard_restores_previous_value() {
        let _g = LOCK.lock().unwrap();
        set_dry_run(false);
        {
            let _guard = DryRunGuard::new(true);
            assert!(is_dry_run());
        }
        assert!(!is_dry_run());
    }

    #[test]
    fn guard_nested_restores_correctly() {
        let _g = LOCK.lock().unwrap();
        set_dry_run(false);
        {
            let _outer = DryRunGuard::new(true);
            assert!(is_dry_run());
            {
                let _inner = DryRunGuard::new(false);
                assert!(!is_dry_run());
            }
            assert!(is_dry_run());
        }
        assert!(!is_dry_run());
    }
}
