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
`Assignee::group("release-managers")`. It drives notification routing and audit.
It is not a permission on its own: an admin resolves any gate, and an assignee
who is not an admin cannot resolve their own gate. The one place it decides
authorization is [delegation](#delegation-and-absence), where it names the
person whose power may be handed over.

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

# Everything Alice granted, plus everything she received.
ironflow-cli delegation list

# Back early.
ironflow-cli delegation delete <delegation-id>
```

The same three endpoints back the CLI:

| Endpoint | What it does |
|----------|--------------|
| `POST /api/v1/approval-delegations` | Grant a delegation. The delegator is always the caller. |
| `GET /api/v1/approval-delegations` | List the active delegations you granted or received. An admin sees them all. |
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

An admin resolving a gate is still recorded under their own name alone.

## Step replay

After an approval, the engine re-executes the handler from the beginning. Completed steps return their cached output immediately -- they do not re-run. The approved gate is skipped, and execution resumes with the next step.
