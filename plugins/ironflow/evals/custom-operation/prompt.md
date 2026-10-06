---
description: A request for a Slack integration tracked as a step loads the operation skill and gets an Operation implementation called through ctx.operation.
tags: [operation]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

In my Ironflow project I want a reusable step that posts a message to a Slack channel through an incoming webhook, with its own status and duration in the run like any other step. The webhook URL comes from the worker environment and must never end up in the persisted step input.

Do not ask me anything and do not write files: put the Rust code of the integration, and the line that calls it from a handler, in your reply.
