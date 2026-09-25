# Approval Gates

An approval gate pauses a workflow run until a human approves or rejects it. This enables human-in-the-loop workflows like deploy pipelines where production deploys require sign-off.

## How it works

1. The handler calls `ctx.approval()` with a prompt message
2. The run transitions to `AwaitingApproval`
3. The worker releases the run and moves on to other work
4. A human calls `POST /api/v1/runs/:id/approve` or `POST /api/v1/runs/:id/reject`
5. On approval, the run is requeued. A worker picks it up, replays completed steps from cache, skips the approved gate, and continues execution
6. On rejection, the run transitions to `Failed`

If the gate carries an SLA deadline and nobody answers in time, step 4 is
performed by the server instead of a human -- see
[SLA timers](#sla-timers) below.

## Example

```rust,ignore
{{#include ../../../../examples/ironflow-workflows/src/deploy_approval.rs}}
```

## Configuration

```rust,ignore
use std::time::Duration;

ApprovalConfig::new("Deploy to production?")
    .assigned_to(Assignee::group("release-managers")) // Who is expected to answer
    .with_deadline(Duration::from_secs(3600))         // SLA: one hour to answer
    .on_timeout(EscalationPolicy::AutoReject)         // What happens when it expires
```

`assigned_to` takes an `Assignee` -- `Assignee::user("alice")` or
`Assignee::group("release-managers")`. It drives notification routing and audit,
and it decides who may resolve the gate:

- an admin resolves any gate;
- a gate assigned to a user is resolved by that user, admin or not, and by
  whoever holds an active [delegation](#delegation-and-absence) from them;
- a gate assigned to a group, or to nobody, is admin-only.

The assignee is matched to the caller by user ID, so an API key resolves its
owner's gates whatever the key is named.

Everything past the message is optional. Without a deadline, the run waits
indefinitely.

### SLA timers

`with_deadline` (or `with_deadline_secs`) arms a timer on the approval step. The
deadline is stored in the database next to the step, not in memory, so it
survives an API or worker restart: a fresh process picks the expired gate up on
its next pass. The API server checks for expired gates every 30 seconds.

The timer is cleared the moment the gate resolves -- approved, rejected, or
escalated -- so a gate is never escalated after a human answered it. A deadline
fires at most once, even with several API instances running.

`with_timeout_seconds` is the legacy spelling: it is now *enforced*, as a
deadline with an implicit `AutoReject` policy. Setting both keeps the explicit
`with_deadline`.

### Escalation policies

`on_timeout` takes an `EscalationPolicy`. Without one, an expired deadline
auto-rejects.

| Policy | What it does when the deadline fires |
|--------|--------------------------------------|
| `AutoApprove` | Completes the gate with `approved_by: "system:timeout"` and resumes the run. |
| `AutoReject` | Fails the step and the run with `approval timeout`. The default. |
| `Notify(targets)` | Posts the escalation event to each target, leaves the gate open, restarts the timer. |
| `Escalate(Assignee)` | Reassigns the gate to another user or group, leaves it open, restarts the timer. |
| `Chain(policies)` | Applies one policy per expiry, in order. |

`Notify` and `Escalate` do not resolve the gate: on their own, they fire again at
every expiry until a human answers. Each firing writes an audit entry, so the
loop is visible rather than silent. Wrap them in a `Chain` to advance one policy
per expiry instead:

```rust,ignore
use std::time::Duration;

ApprovalConfig::new("Deploy to production?")
    .with_deadline(Duration::from_secs(3600))
    .on_timeout(EscalationPolicy::Chain(vec![
        // After 1 h: ping the on-call channel, keep waiting.
        EscalationPolicy::Notify(vec![NotificationTarget::Slack {
            webhook_url: slack_webhook_url,
            channel: "#deploys".to_string(),
        }]),
        // After 2 h: give up.
        EscalationPolicy::AutoReject,
    ]))
```

Once a chain runs out, the gate stays open with no timer and a warning is
logged -- it is never silently auto-rejected.

`NotificationTarget` is delivered as a plain HTTP `POST`, with the engine's
shared retry and backoff: `Webhook { url }` posts the escalation event as JSON,
`Slack { webhook_url, channel }` posts a message to a Slack *incoming webhook*.
A dead endpoint is logged and never blocks the timer reset.

### Seeing the remaining time

The countdown surfaces in three places:

- the API: `approval_seconds_remaining` and `approval_assignee` on every step of
  `GET /api/v1/runs/:id` (clamped at 0, `null` without a deadline);
- the dashboard: a countdown badge on the gate in the run's step list;
- the CLI: the `SLA` column of `ironflow run steps <id>`, yellow in the last
  tenth of the window and red once expired.

Every escalation is also recorded in the audit log as an `approval_escalated`
event carrying the stage, the policy, what it did, and why it fired.

## Delegation and absence

An approval gate assigned to one person stops every run behind it the moment
that person is away. A *delegation* hands their approval power to a colleague
for a bounded window, without making anyone an admin and without reassigning
the gates one by one.

A delegation records who grants it, who receives it, the window it is valid for,
and an optional glob on the workflow name:

```bash
# Alice hands her deploy approvals to Bob for a week.
ironflow-cli delegation create <bob-user-id> \
    --until 2026-10-01T00:00:00Z \
    --workflow 'deploy-*'

# Everything Alice granted, plus everything she received (20 per page).
ironflow-cli delegation list --page 1 --per-page 20

# Back early.
ironflow-cli delegation delete <delegation-id>
```

The same three endpoints back the CLI:

| Endpoint | What it does |
|----------|--------------|
| `POST /api/v1/approval-delegations` | Grant a delegation. The delegator is always the caller. |
| `GET /api/v1/approval-delegations` | List the active delegations you granted or received, paginated with `page` and `per_page` (default 20, max 100). An admin sees them all and may filter with `from_user_id` and `to_user_id`. |
| `DELETE /api/v1/approval-delegations/{id}` | Revoke one. Only the delegator or an admin may. |

### What a delegation covers

Only a gate assigned to an individual can be delegated:

```rust,ignore
ApprovalConfig::new("Deploy to production?")
    .assigned_to(Assignee::user("alice")) // Bob can answer this through a delegation.

ApprovalConfig::new("Deploy to production?")
    .assigned_to(Assignee::group("release-managers")) // Admin-only; no single delegator.
```

A gate assigned to a group, or with no assignee at all, stays admin-only: there
is no single person whose power could have been handed over.

The `workflow_filter` glob narrows a delegation to part of the catalogue.
`"deploy-*"` covers `deploy-prod` but not `cleanup`; omitting it covers every
workflow. A pattern that does not parse matches nothing, so a corrupted row can
never widen someone's reach.

Several delegations can be active at once -- one per colleague, one per
workflow family, or from several delegators to the same person. The newest one
that matches both the gate's assignee and the run's workflow wins.

### Expiry

The window is half-open: a delegation is live at `valid_from` and already over
at `valid_until`. There is no cleanup job. Expired and not-yet-started rows are
filtered out every time delegations are read, so they can neither be listed nor
used to approve -- they are still reachable by ID, which is what makes an
expired delegation revocable.

### Audit

A delegated decision names both people. The `approval_granted` (or
`approval_rejected`) audit entry reads:

```json
{
  "type": "approval_granted",
  "run_id": "01932f...",
  "approved_by": "bob (delegated from alice)",
  "at": "2026-09-22T10:15:00Z"
}
```

An admin, or the assignee resolving their own gate, is recorded under their own
name alone.

## Requiring several approvers

A gate can require more than one approval, and restrict who may vote, depending
on the run itself: a small payment needs one approver, a large one needs two
people from finance. The handler decides in plain Rust, from its typed input and
the outputs of earlier steps, and passes the result to `requiring`:

```rust,ignore
use ironflow_engine::config::{ApprovalConfig, Approvers};

let payment: Payment = ctx.input().await?;
let approvers = match payment.amount {
    a if a > 100_000 => Approvers::at_least(3)
        .from_groups(["finance", "board"])
        .because("amount > 100k"),
    a if a > 10_000 => Approvers::at_least(2)
        .from_groups(["finance"])
        .because("amount > 10k"),
    _ => Approvers::any(),
};
ctx.approval(
    "payment-gate",
    ApprovalConfig::new("Release the payment?").requiring(approvers),
).await?;
```

| Builder | Meaning |
|---------|---------|
| `Approvers::any()` | One approval, from anyone allowed to answer the gate |
| `Approvers::at_least(n)` | `n` distinct approvals (`required_approvers`) |
| `.from_groups([..])` | Only members of these groups may vote (`approver_groups`) |
| `.because("..")` | Audit label shown on the dashboard and in the events (`reason`), never evaluated |

A typo in a field name or a comparison does not compile, and the compiler checks
every branch of the `match`. A gate without `requiring` behaves as a single
approval gate.

The approvers are stored on the step as an `ApprovalRequirement` (`reason`,
`required_approvers`, `approver_groups`) when the gate opens. That record is the
source of truth from then on: replaying or resuming the run never recomputes
it, even if the handler would now compute other approvers. `GET
/api/v1/runs/:id` exposes it on the step as `approval_requirement`, with the
votes cast so far in `approvals` and the count needed in `approvals_required`;
the dashboard shows it as an `n/m approvals` badge whose tooltip gives the
reason.

`Approvers::at_least(0)` and a blank group name panic, so a broken gate fails
when the workflow runs the builder, not when a human votes. A JSON config with
`required_approvers: 0` is rejected on deserialization.

### Voting

- **One vote per user.** Votes are counted by user ID: an API key votes as its
  owner, and the same user approving twice gets `409 Conflict`.
- **An admin's approval is one vote.** Admins may always vote, even on a gate
  restricted to groups, but they do not override the count.
- **A rejection vetoes.** One rejection from anyone allowed to vote fails the
  run, even after partial approvals.
- Until the count is reached, `POST /approve` returns `200` with the run still
  `awaiting_approval`, the gate keeps its SLA timer, and the CLI prints
  `Approval recorded; more approvals are required.`

An `EscalationPolicy::AutoApprove` still resolves the gate outright, whatever
the required count.

### Approver groups

When the approvers list `approver_groups`, only members of at least one of
those groups (and admins) may vote. The gate's assignee and approval
delegations are not consulted. A listed group without members leaves the gate
to admins.

Group membership is managed by admins:

```bash
# Put alice in finance and legal (replaces her current groups).
ironflow user set-groups <alice-id> --group finance --group legal

# Show her groups.
ironflow user groups <alice-id>

# Remove her from every group.
ironflow user set-groups <alice-id>
```

The same operations are available as `GET` and `PUT /api/v1/users/:id/groups`.
Group names are 1 to 64 characters from `[A-Za-z0-9_.-]`, at most 50 per user.

### Audit events

- `approval_requested` is published when the gate opens and carries the
  recorded `requirement`.
- `approval_granted` is published for **every** vote, with the `step_id`,
  `approvals_received`, `approvals_required` and the `requirement`. The gate
  resolves when `approvals_received >= approvals_required`.
- `approval_rejected` carries the `step_id` and the `requirement`.

```json
{
  "type": "approval_granted",
  "run_id": "01932f...",
  "step_id": "01932f...",
  "approved_by": "alice",
  "approvals_received": 1,
  "approvals_required": 2,
  "requirement": {
    "reason": "amount > 10k",
    "required_approvers": 2,
    "approver_groups": ["finance"]
  },
  "at": "2026-09-24T10:15:00Z"
}
```

## Step replay

After an approval, the engine re-executes the handler from the beginning. Completed steps return their cached output immediately -- they do not re-run. The approved gate is skipped, and execution resumes with the next step.
