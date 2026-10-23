//! Workflow handlers of the infrastructure team.

mod ping_hosts;

pub use ping_hosts::{PingHosts, PingHostsInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler, registered by both the server and the worker.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(PingHosts)]
}
