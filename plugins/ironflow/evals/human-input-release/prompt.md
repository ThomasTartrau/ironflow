---
description: An English request for a workflow that asks an operator for text loads the workflow skill and gets a typed human_input step, the rejection branch, and the answers passed outside the command string.
tags: [workflow]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

Ironflow workflow `publish-notes`: after `cargo build --release`, the run must pause and ask an operator to type the release title and the release notes. Then it runs `./publish.sh` with that title and those notes. If the operator refuses to answer, it runs `./notify.sh` with the reason they gave instead.

Do not ask me anything and do not write files: put the complete handler in your reply.
