---
description: An English request to pass a produced file to a later step with a stored secret loads the workflow skill and gets an artifact handle, shell input, and the secret through the environment.
tags: [workflow]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

Ironflow workflow `coverage-report`. The first step runs `cargo llvm-cov --html --output-dir out` in `/app`, which produces `out/index.html`. The second step uploads that file with `./upload.sh index.html` in `/upload`, a different directory. The upload needs our GitLab token, stored as an Ironflow secret named `gitlab_token`.

Do not ask me anything and do not write files: put the complete handler in your reply.
