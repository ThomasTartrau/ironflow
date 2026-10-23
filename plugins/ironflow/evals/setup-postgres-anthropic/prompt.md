---
description: An English request to start a project with Postgres and the Anthropic API loads the setup skill and gets the store and provider features, the provider constructor and the required variables.
tags: [setup]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

I want to start a new Ironflow project from scratch: runs stored in Postgres, and agent steps going through the Anthropic API rather than the Claude CLI. Do not run any command and do not write files. In your reply, give me the commands to create the project, the worker's agent provider code, and every environment variable I must set before the first run.
