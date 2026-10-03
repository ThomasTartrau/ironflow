# Ironflow plugin for Claude Code

Skills and an agent for developers building on [Ironflow](https://gitlab.com/ThomasTartrau/ironflow),
the workflow engine where workflows are plain async Rust.

## Install

```bash
# 1. Register the Ironflow repository as a plugin marketplace
claude plugin marketplace add https://gitlab.com/ThomasTartrau/ironflow.git

# 2. Install the plugin
claude plugin install ironflow@ironflow
```

## Use

One entry point, `/ironflow`, with a verb:

| Command | What it does |
|---|---|
| `/ironflow` | Looks at the current project and proposes the next verb |
| `/ironflow setup` | Scaffolds a workspace: `workflows/` lib, `server/` and `worker/` binaries, `.env.example`, `scripts/dev.sh`, one `hello` workflow and its end-to-end test |
| `/ironflow workflow <name>` | Writes a `WorkflowHandler` with a typed input schema, registers it in `handlers()`, then offers a review |
| `/ironflow operation <name>` | Writes a custom `Operation` (any API call tracked as a step) |
| `/ironflow test <workflow>` | Writes an end-to-end test: real engine, in-memory store, record/replay provider |
| `/ironflow review [file]` | Runs the workflow reviewer agent on a handler |
| `/ironflow upgrade [version]` | Bumps every ironflow crate (latest by default), reads the changelog in between, maps each change to your code, then proposes or applies the migrations (`--apply`). `--from <git-rev>` analyses a bump already made |
| `/ironflow audit` | Checks existing code against the current idioms (typed API, deprecated calls, wiring) without bumping |

The sub-skills are hidden from the `/` menu; the hub loads the right one.

## What the reviewer checks

- Side effects outside of steps before an approval gate. After approval the handler is
  replayed from the top: steps come back from cache, everything between them runs again.
- Step names that are not stable or not unique. The replay cache is keyed by name.
- Secrets leaking into a command line, a log, or a step output.
- Handler missing from `handlers()`, or a name that is not kebab-case.

## Layout

```text
plugins/ironflow/
  .claude-plugin/plugin.json
  skills/
    ironflow/      hub (user-invocable)
    setup/         scaffold script, project template in assets/, options in references/
    workflow/      handler recipe, step catalogue, approval replay pitfalls
    operation/     Operation trait recipe, HTTP JSON and GitLab issue examples
    test/          end-to-end test recipe
    upgrade/       upgrade and audit procedure, migration catalogue in references/,
                   changes.sh (changelog between two Cargo.lock), scan.sh (catalogue grep)
  agents/
    workflow-reviewer.md
```

## Keeping the plugin honest

The project template is scaffolded and compiled against the workspace by
`scripts/check-plugin-template.sh`. Every Rust snippet in the skills is compiled as a
doctest by `examples/plugin-tests`. Both run in CI, so a change to the Ironflow API that
breaks a skill breaks the pipeline.

The upgrade catalogue (`skills/upgrade/references/`) is written by hand: a change that
breaks, deprecates or changes the behavior of code an Ironflow user writes adds an entry
there, with a `detect:` pattern when a line of code can reveal it. An entry is a `## <id>`
heading followed by `- kind:` (`breaking`, `deprecated`, `behavior`, `idiom` or `adopt`),
`- since: <crate> <version> (#<issue>)`, any number of ``- detect: `<grep -E pattern>` ``
lines, an optional `- compiler:` line quoting the error it fixes, then a `diff` block from
the old form to the new one. Check a new pattern against the workspace:
`plugins/ironflow/skills/upgrade/scripts/scan.sh .` must not report code that is already
up to date.
