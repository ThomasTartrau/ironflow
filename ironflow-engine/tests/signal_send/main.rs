//! Integration tests for signal send steps (`ctx.send_signal::<S>()`).
//!
//! Every test drives a real [`Engine`](ironflow_engine::engine::Engine) over a
//! real [`InMemoryStore`](ironflow_store::memory::InMemoryStore) with real
//! handlers: a workflow waits on a signal, another one sends it. Woken runs
//! resume in-process. `cargo test -p ironflow-engine --test signal_send
//! <word>` selects a group.

mod delivery;
mod fixture;
mod refusal;
mod replay;
