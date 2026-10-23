---
description: An English request for a read-only agent step that returns a typed verdict loads the workflow skill and gets one agent step with Read, no shell, a structured output and a valid budget.
tags: [workflow]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

Ironflow workflow `risk-review`: an agent step reads the files of the repository checked out in `/repo`. It may read files but must have no shell at all. It answers with a structured verdict: a risk score from 0 to 10 and a one-paragraph summary. When the score is above 7, an approval gate asks a human before `./merge.sh` runs; otherwise `./merge.sh` runs directly. Give the agent about 30 cents of budget.

Do not ask me anything and do not write files: put the complete handler in your reply.
