//! Internal routes for worker-to-API communication.
//!
//! Protected by a static bearer token (`WORKER_TOKEN`).
//!
//! These routes return raw store entities (not public DTOs) so the worker
//! can deserialize them as `Run`, `Step`, etc. without losing fields like
//! `FsmState<RunStatus>` or `payload`.

pub mod create_run;
pub mod create_step;
pub mod create_step_dependencies;
pub mod get_run;
pub mod get_secret;
pub mod pick_next_run;
pub mod push_logs;
pub mod update_run;
pub mod update_run_status;
pub mod update_step;
