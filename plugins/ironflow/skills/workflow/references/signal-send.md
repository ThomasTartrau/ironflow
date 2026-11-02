# Sending a signal from a workflow

`ctx.send_signal(name, &signal, key)` delivers `signal` as a step of the run (kind
`signal_send`) to every run waiting on `S::NAME` and `key` with `ctx.wait_for_signal`.
It needs no API key: no admin key stored as a secret to call `POST /api/v1/signals`.
The handler declares every signal type it sends in `sent_signals`; any other fails the
step with `EngineError::SignalNotAllowed` and sends nothing.

```rust,no_run
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::signal::{SentSignals, Signal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
struct HumanReplied {
    body: String,
}

impl Signal for HumanReplied {
    const NAME: &'static str = "gitlab.human_replied";
}

#[derive(Deserialize)]
struct GitlabNote {
    discussion_id: String,
    body: String,
}

struct RouteNote;

impl WorkflowHandler for RouteNote {
    fn name(&self) -> &str {
        "route-gitlab-note"
    }

    fn sent_signals(&self) -> SentSignals {
        SentSignals::new().allow::<HumanReplied>()
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let note: GitlabNote = ctx.input().await?;
            let reply = HumanReplied { body: note.body };
            ctx.send_signal("wake-agent", &reply, &note.discussion_id).await?;
            Ok(())
        })
    }
}
```

- It returns the `SignalDelivery` (`signal_id`, `duplicate`, `resumed`, `rejected`), also
  stored as the step output. The `signal_received` audit entry names the sending run and
  step (`sender_run_id`, `sender_step_id`).
- The declaration is not a security boundary: whoever can start the workflow chooses the
  key and the payload it sends. Build them from data you trust.
- A completed step is replayed without sending again; a step interrupted by a lost worker
  is deduplicated. A retry of the run is a new attempt and sends again, like a shell step.
- An empty key, or one longer than 255 characters, fails the step with
  `EngineError::InvalidSignal`.
- On a worker, the signal goes through the API with the worker token: upgrade the API
  before the workers.
- While planning, the step is recorded and nothing is sent.
- In tests, `TestEngine` delivers for real into its `InMemoryStore`: check
  `store.list_signals(..)` or the returned delivery.
