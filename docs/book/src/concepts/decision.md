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

Without a provider, a decision step fails with `NoDecisionProvider`. In tests, use
`RecordReplayDecisionProvider::replay(dir)` to serve captured JSON fixtures with no
network.

### Through OpenRouter

OpenRouter serves the same System One wire contract on its Decisions endpoint, so
the same provider reaches Jev with only the base URL and key changing. Use the
`openrouter` constructor and select the OpenRouter-namespaced model slug on the
config (`typesafe/jev-latest`, exposed as `OPENROUTER_MODEL`):

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
question `instructions` and `criteria` to be strings, which the builder's `&str`
arguments already satisfy.

## The three question types

| Type | Method | Answer |
|------|--------|--------|
| noul | `.noul(name, instructions)` | probability of "yes" in `[0, 1]` |
| choice | `.choice(name, instructions, &options)` | selected option + per-option probabilities |
| score | `.score(name, instructions, &levels)` | weighted score + per-level probabilities |

```rust,ignore
let out = ctx.decision(
    "triage",
    DecisionConfig::new("Payouts have been failing for 3 days")
        .noul("is_urgent", "Does this convey urgency?")
        .choice("team", "Which team?", &["billing", "technical", "sales"])
        .score("mood", "How frustrated?", &["Calm", "Frustrated", "Very angry"])
        .escalate_below(0.7),
).await?;

let urgent = out.noul("is_urgent")?;      // f64
let team = &out.choice("team")?.choice;   // &str
let mood = out.score("mood")?.score;      // f64
```

`choice` and `score` answers carry a `confidence`. A `noul` answer reports only a
probability `p`; its confidence is derived as `2 * |p - 0.5|` (a coin flip is `0`,
a certain yes/no is `1`).

## Escalation

When `escalate_below(threshold)` is set and any answer's confidence falls below it,
the run suspends in `AwaitingApproval`, exactly like an [approval gate](approval-gates.md).
On resume the decision is **not** re-run: the stored answers are replayed as-is, so
downstream routing stays deterministic across the suspend/resume boundary.

## Cost

The provider reports token usage; the engine imputes the cost to the run's budget like
an agent step. For Jev, only input tokens are billed (output is unmetered).
