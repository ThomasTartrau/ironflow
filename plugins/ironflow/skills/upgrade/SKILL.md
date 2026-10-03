---
name: upgrade
description: Upgrade the Ironflow crates of a project and migrate its code, or audit existing code against the current Ironflow idioms. Reads the changelog between the installed and the target release, maps every change to the project's code, then proposes or applies the migrations. Loaded by the ironflow hub for the upgrade and audit verbs.
user-invocable: false
allowed-tools: Bash(cargo:*), Bash(git status:*), Bash(git diff:*), Bash(git log:*), Bash(git show:*), Bash(${CLAUDE_SKILL_DIR}/scripts/changes.sh:*), Bash(${CLAUDE_SKILL_DIR}/scripts/scan.sh:*)
---

# Ironflow upgrade

Moves a project to a newer Ironflow release and brings its code up to the idioms of that
release. Arguments: `$ARGUMENTS`.

| Arguments | Mode |
|---|---|
| none | `upgrade` to the latest release |
| `2.43.0` or `ironflow-core@4.8.0` | `upgrade` to that release (a bare version is `ironflow-engine`) |
| `--from <git-rev>` | the bump is already done: analyse from the `Cargo.lock` of that revision |
| `audit` | no bump: check the code against the current idioms, whatever its history |
| `--apply` | apply without asking, except `adopt` entries and anything ambiguous |

Semver does not protect an Ironflow project: author-API breaks ship in minor releases
(`ironflow-engine` 2.39.0 retyped the whole author API). The compiler, the changelog and
the catalogue in `${CLAUDE_SKILL_DIR}/references/` are the source of truth.

## 1. Preflight

```bash
git status --porcelain                                  # must be empty, or ask
grep -rn --include=Cargo.toml '^ironflow-' . | grep -v '/target/'
cargo check --workspace --all-targets 2>&1 | tail -3    # baseline
```

- Uncommitted changes: ask before going on. The migration must be one reviewable diff.
- An ironflow dependency declared with `path =` or `git =`: stop. It follows its checkout,
  not crates.io.
- Baseline already broken: note the errors. They are not the upgrade's to fix.

## 2. Bump (upgrade only)

Target versions: `cargo search <crate> --limit 5`, keep the line `<crate> = "<version>"`.
Move every ironflow crate together, never one alone.

- Each package: `cargo add -p <package> <crate>@<version>` (`--dev` for a dev-dependency).
  It keeps the features. A `workspace = true` dependency: edit `[workspace.dependencies]`.
- Then `cargo update <crate> <crate> ...` with every ironflow crate, nothing else.
- Target older than latest: require `=<version>` for the named crate, run `cargo update`
  on the ironflow crates (cargo picks the others compatible with it), then write back the
  versions `Cargo.lock` resolved, without `=`. No downgrade.
- `awk '/^name = "ironflow-/ {print $3}' Cargo.lock | sort | uniq -d` must print nothing.
  Two versions of one crate give errors like `expected ironflow_core::X, found
  ironflow_core::X`: raise the requirement that holds the old one.

## 3. What changed

```bash
"${CLAUDE_SKILL_DIR}/scripts/changes.sh"              # HEAD:Cargo.lock vs Cargo.lock
"${CLAUDE_SKILL_DIR}/scripts/changes.sh" <git-rev>    # --from: lock of that revision
```

It prints the versions before and after, then every changelog entry in between,
deduplicated across crates. Skip it in `audit` mode. Find the revision before a past bump
with `git log --oneline -- Cargo.lock`.

Keep the entries that touch the project: handler code, server or worker wiring,
environment, runtime behavior. Most have a catalogue entry (same issue number). For an
author-facing entry without one, read the issue (its Solution and Specs sections):

```bash
curl -s https://gitlab.com/api/v4/projects/ThomasTartrau%2Fironflow/issues/<n> | jq -r '.title, .description'
```

## 4. Where it hits the code

```bash
cargo check --workspace --all-targets --message-format short 2>&1 | grep -E '^[^ ]+: (error|warning)'
"${CLAUDE_SKILL_DIR}/scripts/scan.sh" .
```

- Every compiler error and deprecation warning maps to a catalogue entry (`compiler:`
  line) or to a changelog entry. Neither: read the item in the new crate source,
  `~/.cargo/registry/src/*/<crate>-<version>/src/`. Its rustdoc names the replacement.
  Never guess an API.
- Every `scan.sh` hit is a candidate. Read the code around it before keeping it: some
  patterns (`json::<`, `ServerConfig::from_env`) also match correct code.
- Upgrade mode: catalogue entries whose `since` falls in the bumped range apply even
  without a hit when they are `behavior` (environment, defaults, model aliases).
- Wiring: compare the project's server and worker `main.rs` with
  `${CLAUDE_SKILL_DIR}/../setup/assets/{server,worker}/src/main.rs`. Report only a
  missing piece that changes behavior: a background task, a required variable.
- `audit` mode also reads every handler for what a line pattern cannot catch: an
  expression in a string, a `Value` read by key over several lines, a step name copied by
  hand to reach what that step produced.

## 5. Report, then ask

| Kind | Meaning | Default |
|---|---|---|
| `breaking` | does not compile after the bump | apply, required to build |
| `deprecated` | compiles with a warning | apply |
| `behavior` | compiles, runs differently (env, defaults, models) | apply the code part, list the ops part |
| `idiom` | compiles, but not the typed way: a typo would pass in silence | apply |
| `adopt` | a new feature replaces a workaround | propose one by one |

One table per kind: entry, `file:line`, the fix in a few words, effort (S/M/L). Then the
versions before and after, and what needs a human outside the code (secrets to generate,
a decision provider to wire). Ask with one multi-select AskUserQuestion, one option per
kind present. In `--apply` mode, skip the question for everything but `adopt`.

## 6. Apply

- One catalogue entry at a time, in the order of the table above. After each:
  `cargo check --workspace --all-targets`. Do not start the next with new errors.
- The `+` side of each entry and the compiled snippets of
  `${CLAUDE_SKILL_DIR}/../workflow/references/steps.md` are the target form. Imports go at
  the top of the file with `use`.
- **Steps are replay keys.** A run waiting on an approval, a human input or a signal
  replays its handler by step name and order; a changed sequence fails it with
  `ReplayDivergence`. Never rename a step during a migration. An entry that adds, removes
  or reorders steps (most `adopt` ones) bumps the handler's `version()`, and the user
  drains `ironflow-cli run list --status awaiting_approval` and `--status sleeping` first.
- Never weaken a test to make it pass. A test that asserted the old behavior is updated to
  the new one, and the report says so.
- At the end: `cargo test --workspace`, and `cargo clippy --workspace --all-targets` if
  the project runs it in CI. Then offer the `ironflow:workflow-reviewer` agent on the
  handlers that changed.

## 7. Hand over

Five lines at most: versions before and after, entries applied, entries left and why, what
to do before deploying (secrets, environment, Postgres migrations run at server boot:
back up first). Suggest the commit, do not make it: `chore: bump ironflow to <engine version>`.
