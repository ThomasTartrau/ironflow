# Decisions

A **decision** is a typed machine verdict: classify, route, score, or answer yes/no,
with a *calibrated confidence* instead of free text. It is the shape of TypeSafe AI's
System One model ([Jev](https://typesafe.ai)): for a structured verdict it is far
faster and cheaper than routing a prompt through a conversational LLM.

Decisions use a dedicated abstraction rather than the [agent](steps.md) one: there is
no single prompt, no tools, and no streaming. A run wires a `DecisionProvider`
independently of its agent provider.

## Wiring a provider

```rust,ignore
use ironflow_core::providers::http::TypeSafeProvider; // feature "provider-typesafe"
use std::sync::Arc;

let engine = Engine::new(store, agent_provider)
    .with_decision_provider(Arc::new(TypeSafeProvider::new(api_key)));
```

When the workflow runs in a [worker](engine-worker.md), wire the provider on the
`WorkerBuilder` instead; every run the worker executes gets it:

```rust,ignore
let worker = WorkerBuilder::new(&api_url, &worker_token)
    .provider(agent_provider)
    .decision_provider(Arc::new(TypeSafeProvider::new(api_key)))
    .build()?;
```

Without a provider, a decision step fails with `NoDecisionProvider`. In tests, use
`RecordReplayDecisionProvider::replay(dir)` to serve captured JSON fixtures with no
network.

### Through OpenRouter

OpenRouter serves the same System One wire contract on its Decisions endpoint, so
the same provider reaches Jev with only the base URL and key changing. Use the
`openrouter` constructor and select the OpenRouter model slug on the config
(`typesafe/jev-1.13`, exposed as `OPENROUTER_MODEL`). OpenRouter requires this
concrete versioned slug; the `jev-latest` alias returns `400 "Model does not
exist"`:

```rust,ignore
use ironflow_core::providers::http::typesafe::OPENROUTER_MODEL;
use ironflow_core::providers::http::TypeSafeProvider;
use ironflow_engine::config::DecisionConfig;
use std::sync::Arc;

let engine = Engine::new(store, agent_provider)
    .with_decision_provider(Arc::new(TypeSafeProvider::openrouter(openrouter_key)));

let config = DecisionConfig::new(state).model(OPENROUTER_MODEL);
```

OpenRouter's `decisions` route is on an `alpha` path that may move; override it with
`TypeSafeProvider::with_endpoint(url)` if it relocates. OpenRouter also requires
question `instructions` and `criteria` to be strings, which the derive's string
literals already satisfy.

## The three question types

The questions are the fields of a struct deriving `DecisionAnswers`; the options of a
choice are the unit variants of an enum deriving `DecisionChoice`. `ctx.decision`
returns the struct itself.

| Type | Field attribute | Field type | Answer |
|------|-----------------|------------|--------|
| noul | `#[noul("..")]`, optionally `if_true = ".."`, `if_false = ".."` | `f64` | probability of "yes" in `[0, 1]` |
| choice | `#[choice("..")]` | an enum deriving `DecisionChoice` | the option picked |
| score | `#[score("..", levels = ["..", ".."])]` | `f64` | probability-weighted level index |

```rust,ignore
use ironflow_engine::config::DecisionConfig;
use ironflow_engine::decision::{DecisionAnswers, DecisionChoice};

#[derive(DecisionChoice)]
enum Team {
    #[choice(description = "Payments, invoices, refunds")]
    Billing,
    Technical,
    Sales,
}

#[derive(DecisionAnswers)]
struct Triage {
    #[noul("Does this convey urgency?")]
    is_urgent: f64,
    #[choice("Which team?")]
    team: Team,
    #[score("How frustrated?", levels = ["Calm", "Frustrated", "Very angry"])]
    mood: f64,
}

let triage = ctx.decision(
    "triage",
    DecisionConfig::new("Payouts have been failing for 3 days")
        .answers::<Triage>()
        .escalate_below(0.7),
).await?;

match triage.team {
    Team::Billing => { /* .. */ }
    Team::Technical | Team::Sales => { /* .. */ }
}
```

A question is named after its field. An option is labelled with its variant name in
`snake_case`; `#[choice(rename = "..")]` changes the label and
`#[choice(description = "..")]` tells the model what the option means (doc comments are
never sent). A field without a question, a score without levels or a field of the wrong
type does not compile. An option the provider returns that is not a variant fails the
step with `DecisionError::UnknownChoice`.

The options are fixed at compile time. `choice` and `score` answers carry a
`confidence`; a `noul` answer reports only a probability `p`, and its confidence is
derived as `2 * |p - 0.5|` (a coin flip is `0`, a certain yes/no is `1`). Confidence
drives escalation, below; it is not part of the typed answer.

## Escalation

When `escalate_below(threshold)` is set and any answer's confidence falls below it,
the run suspends in `AwaitingApproval`, exactly like an [approval gate](approval-gates.md).
On resume the decision is **not** re-run: the stored answers are replayed as-is, so
downstream routing stays deterministic across the suspend/resume boundary.

## Cost

The provider reports token usage; the engine imputes the cost to the run's budget like
an agent step. For Jev, only input tokens are billed (output is unmetered).
