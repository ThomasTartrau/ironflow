# Implementation notes

Detailed write-ups of how a shipped issue was implemented: the decisions taken, the
alternatives dropped, and what was deliberately left out of scope.

## Rules

- **One file per issue**, named `<issue-number>-<slug>.md` - for example
  `14-worker-lease-and-reaper.md`.
- **Optional.** Most issues need nothing more than their merge request. Write a note when
  the implementation carries decisions a future reader would otherwise have to
  reverse-engineer from the diff.
- **Merged code only.** A note may only be added for code that is already on `main`. A note
  describing an unmerged design is a lie the repo tells to its next reader, and it is what
  motivated [#22](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/22).

## Suggested structure

```markdown
# Issue #<number>: <title>

## Summary
What changed, in two or three sentences.

## Decisions
Numbered list. Each entry: the choice, and why the alternatives were dropped.

## What shipped
Per crate, what was added or modified.

## Out of scope
What was deliberately left out, and the issue tracking it if there is one.
```

Status lives in [ROADMAP.md](../../ROADMAP.md) and in the GitLab issue, not here.
