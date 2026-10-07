//! Cron arithmetic for schedules: timezones, missed occurrences, catch-up.
//!
//! Pure functions, no I/O: the ticker, the schedule sync and the schedule
//! routes call them, then write the result through the store.
//!
//! A cron expression is evaluated in the schedule's IANA timezone, on local
//! wall time. Around a DST change an occurrence in the skipped hour fires once
//! at the end of the gap, and an occurrence in the repeated hour fires once,
//! on its first pass.

use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use croner::Cron;
use ironflow_store::entities::{
    CatchupPolicy, Schedule, ScheduleFiringPlan, ScheduleMissReason, ScheduleNext,
};

/// Smallest delay after which an occurrence counts as late: an occurrence
/// fired within it is on time, even under [`CatchupPolicy::Skip`].
pub(crate) const MIN_ON_TIME_GRACE: Duration = Duration::from_secs(60);

/// Most occurrences enumerated for one firing. Past it, the missed count is a
/// lower bound.
pub(crate) const MAX_ENUMERATED_OCCURRENCES: usize = 100_000;

/// Most cron candidates examined to find one valid occurrence.
const MAX_CANDIDATES: usize = 1000;

/// Most minutes walked forward to leave a DST gap.
const MAX_GAP_MINUTES: usize = 24 * 60;

/// Parse a cron expression (5 fields, or 6 with seconds).
fn parse_cron(expression: &str) -> Result<Cron, String> {
    let mut cron = Cron::new(expression);
    cron.pattern.with_seconds_optional = true;
    cron.parse()
        .map_err(|e| format!("invalid cron expression: {e}"))
}

/// First instant after a DST gap that starts at `naive`.
fn end_of_gap(tz: Tz, naive: NaiveDateTime) -> Option<DateTime<Tz>> {
    let mut probe = naive;
    for _ in 0..MAX_GAP_MINUTES {
        probe += TimeDelta::minutes(1);
        if let Some(instant) = tz.from_local_datetime(&probe).earliest() {
            return Some(instant);
        }
    }
    None
}

/// Next occurrence of a parsed cron strictly after `after`.
fn next_after(cron: &Cron, tz: Tz, after: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, String> {
    // Croner walks wall time read as UTC, so it never meets a DST change;
    // each candidate is then mapped back to a real instant in `tz`.
    let mut cursor = after.with_timezone(&tz).naive_local();
    for _ in 0..MAX_CANDIDATES {
        let candidate = cron
            .find_next_occurrence(&cursor.and_utc(), false)
            .map_err(|e| format!("cannot compute next trigger: {e}"))?
            .naive_utc();
        let instant = tz
            .from_local_datetime(&candidate)
            .earliest()
            .or_else(|| end_of_gap(tz, candidate))
            .map(|t| t.with_timezone(&Utc))
            .filter(|t| *t > after);
        if instant.is_some() {
            return Ok(instant);
        }
        cursor = candidate;
    }
    Ok(None)
}

/// Next occurrence of `cron` in `tz` strictly after `after`.
///
/// `Ok(None)` when no valid occurrence is found within a bounded search.
pub(crate) fn next_occurrence(
    cron: &str,
    tz: Tz,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    next_after(&parse_cron(cron)?, tz, after)
}

/// Next trigger time of `cron` evaluated in `timezone`, from now.
pub(crate) fn next_trigger(cron: &str, timezone: Tz) -> Result<Option<DateTime<Utc>>, String> {
    next_occurrence(cron, timezone, Utc::now())
}

/// Turn the result of a next-occurrence computation into what the schedule
/// does after firing.
fn into_next(next: Result<Option<DateTime<Utc>>, String>) -> ScheduleNext {
    match next {
        Ok(Some(at)) => ScheduleNext::At(at),
        Ok(None) => ScheduleNext::Disable {
            error: "cannot compute next trigger: no next occurrence".to_string(),
        },
        Err(error) => ScheduleNext::Disable { error },
    }
}

/// What a schedule with this cron expression and timezone does after firing:
/// fire again at its next occurrence, or be disabled when that occurrence
/// cannot be computed.
pub(crate) fn schedule_next(cron: &str, timezone: Tz) -> ScheduleNext {
    into_next(next_trigger(cron, timezone))
}

/// Occurrences of one schedule dropped for the same reason during a firing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MissedOccurrences {
    /// Why they were not run.
    pub reason: ScheduleMissReason,
    /// How many were dropped.
    pub count: u64,
    /// Earliest dropped occurrence.
    pub first: DateTime<Utc>,
    /// Latest dropped occurrence.
    pub last: DateTime<Utc>,
}

impl MissedOccurrences {
    /// Group `occurrences`, oldest first, under `reason`. `None` when empty.
    pub(crate) fn group(reason: ScheduleMissReason, occurrences: &[DateTime<Utc>]) -> Option<Self> {
        let (first, last) = (occurrences.first()?, occurrences.last()?);
        Some(Self {
            reason,
            count: occurrences.len() as u64,
            first: *first,
            last: *last,
        })
    }
}

/// What the ticker hands to the store for one due schedule, and what it
/// reports as missed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Firing {
    /// Occurrences to run and the schedule's next trigger.
    pub plan: ScheduleFiringPlan,
    /// Occurrences dropped by the catch-up policy, grouped by reason.
    pub missed: Vec<MissedOccurrences>,
    /// The enumeration hit [`MAX_ENUMERATED_OCCURRENCES`]: the missed counts
    /// are lower bounds.
    pub truncated: bool,
}

/// Parse the cron expression of a stored schedule, with its timezone.
fn parse_schedule(schedule: &Schedule) -> Result<(Cron, Tz), String> {
    let cron = parse_cron(&schedule.cron_expression)?;
    Ok((cron, schedule.policy.timezone))
}

/// Decide which occurrences of a due schedule run, given its policy.
///
/// Every occurrence from `due` up to `now` is enumerated. Those older than the
/// catch-up window (never narrower than `on_time_grace`) are dropped, then the
/// catch-up policy picks what runs among the rest. A cron expression or a
/// timezone that no longer parses fires `due` and disables the schedule.
pub(crate) fn plan_firing(
    schedule: &Schedule,
    due: DateTime<Utc>,
    now: DateTime<Utc>,
    on_time_grace: TimeDelta,
) -> Firing {
    let (cron, tz) = match parse_schedule(schedule) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Firing {
                plan: ScheduleFiringPlan {
                    occurrences: vec![due],
                    next: ScheduleNext::Disable { error },
                },
                missed: Vec::new(),
                truncated: false,
            };
        }
    };

    let mut occurrences = vec![due];
    let mut truncated = false;
    let mut latest = due;
    while let Ok(Some(occurrence)) = next_after(&cron, tz, latest) {
        if occurrence > now {
            break;
        }
        if occurrences.len() == MAX_ENUMERATED_OCCURRENCES {
            truncated = true;
            break;
        }
        occurrences.push(occurrence);
        latest = occurrence;
    }

    let window_secs = TimeDelta::seconds(i64::from(schedule.policy.catchup_window_secs));
    let window_start = now
        .checked_sub_signed(window_secs.max(on_time_grace))
        .unwrap_or(DateTime::<Utc>::MIN_UTC);
    let split = occurrences.partition_point(|o| *o < window_start);
    let (outside, in_window) = occurrences.split_at(split);

    let len = in_window.len();
    let max = schedule.policy.catchup_max as usize;
    let on_time = now - latest <= on_time_grace && in_window.last() == Some(&latest);
    let (reason, kept) = match schedule.policy.catchup {
        CatchupPolicy::Latest => (ScheduleMissReason::Superseded, len.saturating_sub(1)),
        CatchupPolicy::All => (ScheduleMissReason::CatchupMax, len.saturating_sub(max)),
        CatchupPolicy::Skip if on_time => (ScheduleMissReason::CatchupSkip, len - 1),
        CatchupPolicy::Skip => (ScheduleMissReason::CatchupSkip, len),
    };
    let (dropped, run) = in_window.split_at(kept);

    let mut missed = Vec::new();
    missed.extend(MissedOccurrences::group(
        ScheduleMissReason::OutsideWindow,
        outside,
    ));
    missed.extend(MissedOccurrences::group(reason, dropped));

    Firing {
        plan: ScheduleFiringPlan {
            occurrences: run.to_vec(),
            next: into_next(next_after(&cron, tz, latest.max(now))),
        },
        missed,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use ironflow_store::entities::{SchedulePolicy, ScheduleSource};
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().expect("valid RFC 3339 instant")
    }

    fn paris() -> Tz {
        Tz::Europe__Paris
    }

    fn grace() -> TimeDelta {
        TimeDelta::seconds(60)
    }

    fn schedule(cron: &str, policy: SchedulePolicy) -> Schedule {
        let created = at("2026-01-01T00:00:00Z");
        Schedule {
            id: Uuid::now_v7(),
            workflow_name: "report".to_string(),
            cron_expression: cron.to_string(),
            inputs: json!({}),
            source: ScheduleSource::Api,
            disabled_at: None,
            last_triggered_at: None,
            next_trigger_at: None,
            last_error: None,
            created_by_user_id: None,
            created_at: created,
            updated_at: created,
            priority: 0,
            policy,
        }
    }

    fn catchup(policy: CatchupPolicy) -> SchedulePolicy {
        SchedulePolicy {
            catchup: policy,
            ..SchedulePolicy::default()
        }
    }

    /// Hourly schedule last due at 07:00: six occurrences up to 12:00.
    fn hourly_firing(policy: SchedulePolicy, now: &str) -> Firing {
        let due = at("2026-05-01T07:00:00Z");
        plan_firing(&schedule("0 * * * *", policy), due, at(now), grace())
    }

    fn hours(list: &[&str]) -> Vec<DateTime<Utc>> {
        list.iter()
            .map(|h| at(&format!("2026-05-01T{h}:00:00Z")))
            .collect()
    }

    fn disable_error(next: ScheduleNext) -> String {
        match next {
            ScheduleNext::Disable { error } => error,
            ScheduleNext::At(at) => panic!("expected a disable, got {at}"),
        }
    }

    #[test]
    fn schedule_timezone_defaults_to_utc() {
        let policy = SchedulePolicy::default();
        assert_eq!(policy.timezone, Tz::UTC);

        let next = next_occurrence("0 9 * * *", policy.timezone, at("2026-03-27T12:00:00Z"));

        assert_eq!(next, Ok(Some(at("2026-03-28T09:00:00Z"))));
    }

    #[test]
    fn schedule_timezone_paris_9am_follows_dst() {
        let before = next_occurrence("0 9 * * *", paris(), at("2026-03-27T12:00:00Z"));
        let after = next_occurrence("0 9 * * *", paris(), at("2026-03-28T12:00:00Z"));

        assert_eq!(before, Ok(Some(at("2026-03-28T08:00:00Z"))));
        assert_eq!(after, Ok(Some(at("2026-03-29T07:00:00Z"))));
    }

    #[test]
    fn schedule_timezone_skipped_hour_fires_once_after_the_gap() {
        let first = next_occurrence("30 2 * * *", paris(), at("2026-03-28T12:00:00Z"))
            .expect("valid cron")
            .expect("an occurrence");
        let second = next_occurrence("30 2 * * *", paris(), first);

        assert_eq!(first, at("2026-03-29T01:00:00Z"));
        assert_eq!(second, Ok(Some(at("2026-03-30T00:30:00Z"))));
    }

    #[test]
    fn schedule_timezone_repeated_hour_fires_once() {
        let first = next_occurrence("30 2 * * *", paris(), at("2026-10-24T12:00:00Z"))
            .expect("valid cron")
            .expect("an occurrence");
        let second = next_occurrence("30 2 * * *", paris(), first);

        assert_eq!(first, at("2026-10-25T00:30:00Z"));
        assert_eq!(second, Ok(Some(at("2026-10-26T01:30:00Z"))));
    }

    #[test]
    fn schedule_timezone_invalid_cron_is_rejected() {
        let error = next_trigger("not-a-cron", Tz::UTC).expect_err("bad cron");
        assert!(error.contains("invalid cron expression"), "{error}");
    }

    #[test]
    fn catchup_latest_runs_the_most_recent_and_traces_the_rest_as_superseded() {
        let firing = hourly_firing(catchup(CatchupPolicy::Latest), "2026-05-01T12:00:30Z");

        assert_eq!(firing.plan.occurrences, hours(&["12"]));
        assert_eq!(
            firing.missed,
            vec![MissedOccurrences {
                reason: ScheduleMissReason::Superseded,
                count: 5,
                first: at("2026-05-01T07:00:00Z"),
                last: at("2026-05-01T11:00:00Z"),
            }]
        );
        assert!(!firing.truncated);
    }

    #[test]
    fn catchup_all_runs_every_missed_occurrence_in_order() {
        let firing = hourly_firing(catchup(CatchupPolicy::All), "2026-05-01T12:00:30Z");

        assert_eq!(
            firing.plan.occurrences,
            hours(&["07", "08", "09", "10", "11", "12"])
        );
        assert!(firing.missed.is_empty(), "{:?}", firing.missed);
    }

    #[test]
    fn catchup_all_is_bounded_by_catchup_max() {
        let policy = SchedulePolicy {
            catchup: CatchupPolicy::All,
            catchup_max: 2,
            ..SchedulePolicy::default()
        };

        let firing = hourly_firing(policy, "2026-05-01T12:00:30Z");

        assert_eq!(firing.plan.occurrences, hours(&["11", "12"]));
        assert_eq!(
            firing.missed,
            vec![MissedOccurrences {
                reason: ScheduleMissReason::CatchupMax,
                count: 4,
                first: at("2026-05-01T07:00:00Z"),
                last: at("2026-05-01T10:00:00Z"),
            }]
        );
    }

    #[test]
    fn catchup_skip_runs_no_late_occurrence() {
        let firing = hourly_firing(catchup(CatchupPolicy::Skip), "2026-05-01T12:05:00Z");

        assert!(firing.plan.occurrences.is_empty());
        assert_eq!(
            firing.missed,
            vec![MissedOccurrences {
                reason: ScheduleMissReason::CatchupSkip,
                count: 6,
                first: at("2026-05-01T07:00:00Z"),
                last: at("2026-05-01T12:00:00Z"),
            }]
        );
        assert_eq!(
            firing.plan.next,
            ScheduleNext::At(at("2026-05-01T13:00:00Z"))
        );
    }

    #[test]
    fn catchup_skip_still_runs_an_on_time_occurrence() {
        let firing = hourly_firing(catchup(CatchupPolicy::Skip), "2026-05-01T12:00:30Z");

        assert_eq!(firing.plan.occurrences, hours(&["12"]));
        assert_eq!(
            firing.missed,
            vec![MissedOccurrences {
                reason: ScheduleMissReason::CatchupSkip,
                count: 5,
                first: at("2026-05-01T07:00:00Z"),
                last: at("2026-05-01T11:00:00Z"),
            }]
        );
    }

    #[test]
    fn catchup_skip_runs_a_lone_on_time_occurrence_without_missing_any() {
        let due = at("2026-05-01T12:00:00Z");
        let policy = catchup(CatchupPolicy::Skip);

        let firing = plan_firing(&schedule("0 * * * *", policy), due, due, grace());

        assert_eq!(firing.plan.occurrences, vec![due]);
        assert!(firing.missed.is_empty(), "{:?}", firing.missed);
    }

    #[test]
    fn catchup_window_excludes_older_occurrences() {
        let policy = SchedulePolicy {
            catchup: CatchupPolicy::All,
            catchup_window_secs: 2 * 3600,
            ..SchedulePolicy::default()
        };

        let firing = hourly_firing(policy, "2026-05-01T12:00:30Z");

        assert_eq!(firing.plan.occurrences, hours(&["11", "12"]));
        assert_eq!(
            firing.missed,
            vec![MissedOccurrences {
                reason: ScheduleMissReason::OutsideWindow,
                count: 4,
                first: at("2026-05-01T07:00:00Z"),
                last: at("2026-05-01T10:00:00Z"),
            }]
        );
    }

    #[test]
    fn catchup_window_is_never_narrower_than_the_grace() {
        let policy = SchedulePolicy {
            catchup_window_secs: 60,
            ..SchedulePolicy::default()
        };
        let due = at("2026-05-01T12:00:00Z");
        let now = at("2026-05-01T12:01:30Z");

        let firing = plan_firing(&schedule("0 * * * *", policy), due, now, grace() * 2);

        assert_eq!(firing.plan.occurrences, vec![due]);
        assert!(firing.missed.is_empty(), "{:?}", firing.missed);
    }

    #[test]
    fn catchup_next_trigger_is_after_now() {
        let now = at("2026-05-01T12:00:30Z");

        for policy in [
            CatchupPolicy::Latest,
            CatchupPolicy::All,
            CatchupPolicy::Skip,
        ] {
            let firing = hourly_firing(catchup(policy), "2026-05-01T12:00:30Z");
            match firing.plan.next {
                ScheduleNext::At(next) => {
                    assert!(next > now, "{policy:?}: {next}");
                    assert_eq!(next, at("2026-05-01T13:00:00Z"));
                }
                ScheduleNext::Disable { error } => panic!("{policy:?} disabled: {error}"),
            }
        }
    }

    #[test]
    fn catchup_invalid_cron_fires_due_and_disables() {
        let due = at("2026-05-01T07:00:00Z");
        let now = at("2026-05-01T12:00:00Z");
        let broken = schedule("not-a-cron", SchedulePolicy::default());

        let firing = plan_firing(&broken, due, now, grace());

        assert_eq!(firing.plan.occurrences, vec![due]);
        let error = disable_error(firing.plan.next);
        assert!(error.contains("invalid cron expression"), "{error}");
        assert!(firing.missed.is_empty());
    }

    #[test]
    fn catchup_unreachable_next_occurrence_fires_due_and_disables() {
        // February 30th never happens.
        let due = at("2026-05-01T07:00:00Z");
        let now = at("2026-05-01T07:00:10Z");
        let never = schedule("0 0 30 2 *", SchedulePolicy::default());

        let firing = plan_firing(&never, due, now, grace());

        assert_eq!(firing.plan.occurrences, vec![due]);
        let error = disable_error(firing.plan.next);
        assert!(error.contains("cannot compute next trigger"), "{error}");
    }

    #[test]
    fn catchup_missed_group_of_nothing_is_none() {
        let group = MissedOccurrences::group(ScheduleMissReason::Overlap, &[]);
        assert_eq!(group, None);
    }
}
