---
type: llm
---

Look only at the `input()` method of the Operation implementation in the reply.

PASS if `input()` returns no value derived from the webhook URL (it returns the message text, other non-secret fields, or `None`), or if the Operation does not override `input()` at all.
FAIL if `input()` returns the webhook URL, a field holding it, or the whole struct serialized with the URL inside.
