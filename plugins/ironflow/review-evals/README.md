# Workflow reviewer evals

Measures the `ironflow:workflow-reviewer` agent: does a review request reach the agent, does
the agent find the defect planted in a handler, with the right severity, and does it stay
quiet on handlers that are correct. Run by `claude plugin eval`, kept apart from `evals/` so
the `eval-plugin-skills` CI job and its cost ceiling stay unchanged.

Each case directory holds a `fixture/` crate (`src/lib.rs` and the handler under review)
that `scaffold.sh` copies into the empty workspace through `_shared/seed.sh`, plus a
`case.yaml` with the prompt and the graders. The fixtures compile against
`ironflow-engine` with the `secret-store` feature. Case names are the workflow names, never
the defect, and the fixtures carry no comment pointing at it.

| Tag | Cases | Defect |
|---|---|---|
| `side-effect` | notify-deploy, nightly-report | network call or file write outside a step before `ctx.approval` |
| `step-name` | build-matrix, ping-hosts | step name from `Utc::now()`, literal name inside a loop |
| `injection` | checkout-branch, pr-summary | run input or agent output formatted into `ShellConfig::new` |
| `secret` | publish-crate, sync-mirror | token in a command line, token in `info!` |
| `registration` | cleanup-branches, rotate-logs | handler missing from `handlers()`, duplicate `name()` |
| `stringly` | release-gate, version-check | step output read by key |
| `clean` | deploy-service, tag-release, ping-fleet, triage-issue, audit-report | none, close to a trap |
| `multi` | hotfix | injection, side effect before the gate and token in a command |

Half the prompts ask in plain French (`phrasing:natural`), half use `/ironflow review`
(`phrasing:slash`).

## Graders

All graders are regex or `tool_used`: free and deterministic, no judge.

- `dispatched`: the session called the `workflow-reviewer` agent.
- `located` (weight 2): the user-facing answer cites the planted line (`file.rs:N`, `line N`,
  `l. N`), within the statement's line range.
- `severity`: the trace holds `<severity> <path>:<line>` on the planted line, `blocker` for
  injection and secrets, `blocker` or `bug` elsewhere. It reads the agent's own report,
  because the session rephrases it into headings.
- `no-false-alarm`: no `blocker` or `bug` on any other line. Nits are allowed.
- `concluded` (clean cases): a conclusion line exists, so an empty answer cannot pass as a
  clean review.

`severity`, `no-false-alarm` and `concluded` follow the agent's output contract, so they are
only meaningful on runs where `dispatched` passed. Read recall, severity and false alarms on
those runs, and the dispatch rate separately.

## Run

From `plugins/ironflow`:

```bash
claude plugin eval . --eval-dir review-evals --scaffold --ablation none \
  --model claude-sonnet-5-5 --concurrency 8 --threshold 0 --max-cost-usd 6
claude plugin eval . --eval-dir review-evals --scaffold --ablation none --case hotfix --runs 1
```

`--scaffold` is required: without it the workspace is empty and every case fails.
`--ablation none` because the agent does not exist without the plugin, so a no-plugin arm
measures nothing. A full pass is 18 cases x 3 runs, about $3.40 and 3 minutes on
`claude-sonnet-5-5`. Results land in `review-evals/results/` (ignored by git).
