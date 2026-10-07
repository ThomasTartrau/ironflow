# Schedules

A schedule creates a run of a workflow on a cron expression. It is declared in
code by the handler, or created through the API, the CLI, the MCP server or the
dashboard. The server fires it; workers execute the runs like any other run.

## Declaring a schedule

A handler returns its schedule from `schedule()`. The policy builders set the
timezone, the catch-up of missed occurrences and the overlap with a run still
active:

```rust,ignore
use std::sync::LazyLock;
use std::time::Duration;

use ironflow_engine::prelude::*;

static MORNING: LazyLock<CronSchedule> = LazyLock::new(|| {
    CronSchedule::new("0 9 * * *")
        .and_then(|s| s.with_timezone("Europe/Paris"))
        .map(|s| {
            s.with_catchup(CatchupPolicy::All)
                .with_catchup_max(5)
                .with_catchup_window(Duration::from_secs(3 * 86_400))
                .with_overlap(OverlapPolicy::Skip)
        })
        .expect("valid schedule")
});

impl WorkflowHandler for MorningReport {
    fn name(&self) -> &str { "morning-report" }
    fn schedule(&self) -> Option<&CronSchedule> { Some(&MORNING) }
    // ...
}
```

At startup the server syncs every handler schedule to the database: a new
handler creates its schedule, a changed expression, priority or policy updates
it, a removed handler deletes it.

## Catch-up

When no server fired a schedule for a while (a deployment, an outage), its
occurrences pile up. The catch-up policy decides what happens to them on the
next tick:

| Policy | Runs created | Default |
|---|---|---|
| `latest` | One run, for the most recent missed occurrence | yes |
| `all` | One run per missed occurrence, oldest first, at most `catchup_max` (the most recent are kept) | |
| `skip` | None. An occurrence that is on time still runs | |

- `catchup_max` bounds `all`, from 1 to 1000. Default: 10.
- `catchup_window` bounds every policy: an occurrence older than the window is
  dropped. From one minute to 30 days. Default: one day.
- An occurrence is on time when it is less than the on-time grace old: twice
  the ticker interval, and never less than a minute. The window used is never
  narrower than this grace, so an on-time occurrence is never dropped as too
  old.

Each run carries its occurrence. The occurrence is part of the run idempotency
key, so two servers firing the same backlog create each run once.

A paused schedule is never caught up: resuming it computes the next occurrence
from now, whatever the policy.

## Overlap

The overlap policy decides what an occurrence does while a run of the same
schedule is still active (pending, running or waiting):

| Policy | Behavior | Default |
|---|---|---|
| `allow` | Another run is created | yes |
| `skip` | The occurrence is dropped | |

`skip` gives every run of the schedule the concurrency key `schedule:<id>`. A
manual trigger of the schedule contends for the same key: it fails with `409
Conflict` while a run is active.

With `catchup = all` and `overlap = skip`, the first catch-up run takes the key
and the next occurrences of the same backlog are skipped as overlap.

## Timezone and daylight saving time

The expression is evaluated in an IANA timezone, `UTC` by default. `0 9 * * *`
in `Europe/Paris` fires at 9:00 Paris time all year: 7:00 UTC in summer, 8:00
UTC in winter.

- A local time skipped by the spring change (2:30 in Paris on the last Sunday
  of March) fires once, at the end of the gap (3:00).
- A local time repeated by the autumn change fires once, on its first pass.

An invalid timezone is rejected when the schedule is created. A stored timezone
the server no longer knows disables the schedule, with the reason in
`last_error`, like an invalid expression.

## Reading the occurrence in a workflow

`ctx.trigger()` returns how the run was triggered. For a scheduled run it
carries the schedule and the occurrence the run covers, which a catch-up run
needs to process the right period:

```rust,ignore
use ironflow_store::models::TriggerKind;

if let TriggerKind::Cron { scheduled_for: Some(occurrence), .. } = ctx.trigger().await? {
    // Process the day of `occurrence`, not the day of now.
}
```

`scheduled_for` is `None` for a manual trigger of the schedule.

## Missed occurrences

Every occurrence dropped by the policies is traced:

- a `warn` log with the schedule, the reason and the count;
- the `ironflow_schedule_missed_total` counter, labelled by `schedule`;
- a `schedule_occurrences_missed` event, with the reason (`outside_window`,
  `catchup_max`, `superseded`, `catchup_skip` or `overlap`), the count and the
  first and last occurrence. Subscribe to it like any other event.

## API, CLI and MCP

`POST /api/v1/schedules` accepts the optional fields `catchup`, `catchup_max`,
`catchup_window_secs`, `overlap` and `timezone`. Every schedule response
returns them.

```bash
ironflow schedule create morning-report "0 9 * * *" \
  --timezone Europe/Paris --catchup all --catchup-max 5 \
  --catchup-window 259200 --overlap skip
```

The `create_schedule` MCP tool takes the same optional fields.
