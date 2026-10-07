---
description: An English request to test an existing handler loads the test skill and gets a black-box test that asserts the run status and the step order.
tags: [test]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

I have an Ironflow workflow handler `Deploy`, registered under the name `deploy`. It runs two shell steps in order: `build` (`cargo build`) then `ship` (`./ship.sh`). Write a test that checks a run completes and creates exactly those two steps, in that order, without spawning real processes.

Do not ask me anything and do not write files: put the complete test file in your reply.
