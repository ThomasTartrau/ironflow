---
type: llm
---

PASS if the reply says that the HTTP notification sent with the reqwest `Client` before `ctx.approval` is sent again (twice, or once more) because the handler is replayed or re-executed from the start after the approval, AND proposes moving it into a tracked step (`ctx.http`, a step, or similar).
FAIL if the reply does not mention the notification being sent again after the approval, or only criticises it for style, error handling or missing timeouts.
