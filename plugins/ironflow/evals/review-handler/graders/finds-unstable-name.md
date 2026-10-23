---
type: llm
---

PASS if the reply flags the step name built from `Utc::now().timestamp()` because it changes between executions, so the cached step is not found on replay and the build runs again (or the name is not stable), and proposes a constant name such as `"build"`.
FAIL if the timestamp-based step name is not mentioned, or is only criticised for readability.
