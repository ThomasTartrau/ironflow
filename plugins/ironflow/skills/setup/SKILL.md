---
name: setup
description: Scaffold a new Ironflow project - a Cargo workspace with a workflows lib, an API server binary, a worker binary, a hello workflow and its end-to-end test. Loaded by the ironflow hub for the setup verb.
user-invocable: false
allowed-tools: Bash(cargo:*), Bash(${CLAUDE_SKILL_DIR}/scripts/scaffold.sh:*)
---

# Ironflow setup

Creates a runnable Ironflow platform in an empty directory. Everything the script copies lives in `${CLAUDE_SKILL_DIR}/assets/` and is compiled in Ironflow's CI, so read the assets rather than re-deriving them.

## 1. Check the environment

```bash
rustc --version            # 1.94 or newer
cargo --version
claude --version 2>/dev/null || echo "claude CLI not found"
psql --version 2>/dev/null || echo "psql not found"
```

Rust below 1.94: stop and say so, Ironflow does not compile on older toolchains.

## 2. Ask two questions, then stop asking

Ask both in one AskUserQuestion call:

1. **Store**: in-memory (runs are lost on restart, no database needed) or Postgres (needs `DATABASE_URL`, migrations run automatically at boot).
2. **Agent provider for the worker**: Claude Code CLI (`claude` on PATH), Anthropic API (`ANTHROPIC_API_KEY`), OpenAI (`OPENAI_API_KEY`), or "no agent steps for now" (keep the Claude Code provider, it is only exercised when a workflow runs an agent step).

Also confirm the target directory (default: current directory if empty, otherwise ask for a name).

## 3. Scaffold

```bash
"${CLAUDE_SKILL_DIR}/scripts/scaffold.sh" <target-dir>
```

The script refuses a non-empty directory. It copies the template and runs `cargo add` for every ironflow crate, so versions come from crates.io at scaffold time. Then apply the answers from step 2 by following `${CLAUDE_SKILL_DIR}/references/options.md` (Postgres block, provider block). Skip it entirely for in-memory + Claude Code.

## 4. Verify

```bash
cd <target-dir>
cargo build --workspace
cargo test --workspace
```

Both must pass before you report anything. The test suite runs the `hello` workflow through a real engine.

## 5. Proof of life

Tell the user how to see a run complete, in this order:

```bash
scripts/dev.sh             # starts the API server, then one worker
```

Then in the dashboard at `http://localhost:3000`: create the first account (it becomes admin), open **API keys**, create a key with the `workflows_read`, `runs_read` and `runs_write` scopes, and run:

```bash
cargo install ironflow-cli
export IRONFLOW_URL=http://localhost:3000 IRONFLOW_API_KEY=irfl_...
ironflow-cli workflow list
ironflow-cli run create hello --payload '{"name":"Ada"}'
ironflow-cli logs <run-id>
```

A `completed` run with one `greet` step is the proof of life.

## 6. Hand over

Report the generated layout in five lines, then point to the next verbs: `/ironflow workflow <name>` to add a handler, `/ironflow operation <name>` for an integration, `/ironflow test <name>` for a test.

## Layout the script produces

```text
<target>/
  Cargo.toml            workspace: workflows, server, worker
  .env.example          every variable, commented; copied to .env
  scripts/dev.sh        server then worker, Ctrl+C stops both
  workflows/            lib: handlers() + hello.rs + tests/hello.rs
  server/               API server, dashboard embedded, reaper, SSE
  worker/               polls the server, executes handlers
```

## Failure modes

- `cargo add` fails with a network error: retry once, then tell the user the crate resolution failed and which command to rerun.
- `cargo build` fails inside an ironflow crate: the template and the published crates drifted. Report the error verbatim with the crate version from `Cargo.lock`, do not patch the ironflow crate.
- Port 3000 busy: set `PORT` in `.env`; `scripts/dev.sh` honours it.
