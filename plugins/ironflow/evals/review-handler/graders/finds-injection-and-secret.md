---
type: llm
---

PASS if the reply flags BOTH of these:
1. `input.tag` interpolated with `format!` into a `ShellConfig::new` command string, as a command (shell) injection risk;
2. the GitLab token placed in the `glab` command line, because the command is persisted or shown (logs, dashboard, step input), with a fix that passes it through the environment (`.env(..)`) instead.
FAIL if either one is missing.
