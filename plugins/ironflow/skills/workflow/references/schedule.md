# Scheduled workflows

A handler that runs on a cron schedule overrides `schedule()`. The runtime registers the
schedule when the handler is registered; nothing else to wire. The snippet compiles
against the current Ironflow release (CI checks it).

`schedule()` returns `Option<&CronSchedule>`, a reference, so the schedule lives in a
`static LazyLock` (or a field of the handler). The expression has 5 fields
(`min hour day month weekday`), or 6 with seconds first.

- Without `with_timezone`, it fires in UTC, which drifts by an hour twice a year.
- Without `with_overlap(OverlapPolicy::Skip)`, a slow run and the next one stack
  (`Allow` is the default).
- `with_catchup(CatchupPolicy::All)` replays the occurrences missed during an outage;
  `Latest`, the default, runs only the last one.

```rust,no_run
use std::sync::LazyLock;

use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::schedule::{CronSchedule, OverlapPolicy};

/// Every night at 02:00 Paris time; skipped while the previous run is still active.
static NIGHTLY: LazyLock<CronSchedule> = LazyLock::new(|| {
    CronSchedule::new("0 2 * * *")
        .and_then(|s| s.with_timezone("Europe/Paris"))
        .map(|s| s.with_overlap(OverlapPolicy::Skip))
        .expect("valid schedule")
});

struct NightlyBackup;

impl WorkflowHandler for NightlyBackup {
    fn name(&self) -> &str {
        "nightly-backup"
    }

    fn schedule(&self) -> Option<&CronSchedule> {
        Some(&NIGHTLY)
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("backup", ShellConfig::new("./backup.sh")).await?;
            Ok(())
        })
    }
}
```
