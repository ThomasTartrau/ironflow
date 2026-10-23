---
description: An English request for a parent workflow that runs the same child once per item loads the workflow skill and gets typed children with distinct step names and a typed child output.
tags: [workflow]
max_turns: 15
allowed_tools: [Read, Glob, Grep, Skill]
---

In my Ironflow project I already have a workflow `scan-repo`, struct `ScanRepo`. Its input is `ScanInput { repo: String }` and it sets its run output to `ScanReport { vulnerabilities: u32 }`.

Write a parent workflow `scan-all` whose input is a list of repositories. It runs `scan-repo` as a child run for each repository, one after another, sums the vulnerabilities reported by the children and sets the total as its own run output. Also show what the child needs so that the parent can call it.

Do not ask me anything and do not write files: put the Rust code in your reply.
