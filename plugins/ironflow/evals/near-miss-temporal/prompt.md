---
description: A Rust workflow request for another engine (Temporal) must not load any Ironflow skill.
tags: [trigger]
max_turns: 5
allowed_tools: [Read, Glob, Grep, Skill]
---

In my Rust service built on the Temporal SDK (`temporal-sdk` crate), write a workflow function that calls an activity `charge_card` with a retry policy of at most 3 attempts. Put the code in your reply, do not write files.
