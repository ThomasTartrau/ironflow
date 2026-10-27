//! Integration tests for agent supervisors delegating to their team.
//!
//! `ctx.supervise` runs the rounds of a supervisor role: each delegated task
//! becomes a child run of the team's `TeamWorkflow`, and the next round gets
//! the tasks and results in untrusted blocks. The agent provider plays the
//! model from a script; the engine, the store and the child runs are real.
//! `cargo test -p ironflow-engine --test supervisor <word>` selects a group.

mod children;
mod fixture;
mod limits;
mod resume;
mod rounds;
