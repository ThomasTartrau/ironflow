//! Condense CI logs under a character budget.
//!
//! A CI log is mostly progress noise (`Compiling ...`, package manager bars)
//! around the few lines that explain a failure. [`drop_noise`] removes the
//! noise, [`truncate_around_failure`] keeps the failure blocks, the end of the
//! log and the lines leading to the first failure, and [`condense_log`] chains
//! both. Every function is pure: text in, text out.
//!
//! # Examples
//!
//! ```
//! use ironflow_ops_common::log::condense_log;
//!
//! let log = "   Compiling app v0.1.0\nerror[E0308]: mismatched types\nJob failed";
//! assert_eq!(condense_log(log, 1000, "[...]"), "error[E0308]: mismatched types\nJob failed");
//! ```

/// Share of the budget reserved for the tail of the log, as a divisor.
///
/// Heuristic: a third keeps the final summary of the tool without starving
/// the failure blocks.
const TAIL_SHARE: usize = 3;

/// Maximum number of lines kept before the first failure.
///
/// Heuristic: enough to show the command that failed, short enough to leave
/// room for the failure itself.
const LEAD_IN_LINES: usize = 30;

/// Maximum number of lines of one failure block.
///
/// Heuristic: a compiler diagnostic or a panic message rarely exceeds it, a
/// stack trace does and is cut.
const MAX_BLOCK_LINES: usize = 40;

/// Line prefixes of known progress output (cargo, pnpm).
const NOISE_PREFIXES: [&str; 15] = [
    "Compiling ",
    "Downloading ",
    "Downloaded ",
    "Updating ",
    "Fresh ",
    "Checking ",
    "Locking ",
    "Adding ",
    "Blocking waiting for file lock",
    "Finished ",
    "Progress: resolved",
    "Packages: +",
    "Lockfile is up to date",
    "Already up to date",
    "Scope: all",
];

/// Substrings that mark a line as part of a failure.
const FAILURE_MARKERS: [&str; 10] = [
    "error:",
    "error[E",
    "error TS",
    "ERROR:",
    "ERR_PNPM",
    "npm ERR!",
    "FAILED",
    "panicked at",
    "Job failed",
    "test result: FAILED",
];

/// Whether `line` is the header of a failing cargo test (`---- name stdout ----`).
fn is_test_failure_header(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("---- ") && line.ends_with(" stdout ----")
}

/// Whether `token` holds an `@` immediately followed by an ASCII digit.
fn has_version_at(token: &str) -> bool {
    token
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'@' && pair[1].is_ascii_digit())
}

/// Whether `line` is the echo of an npm/pnpm script (`> pkg@1.0.0 build /path`, `> tsc -b`).
fn is_script_echo(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("> ") else {
        return false;
    };
    match rest.split_whitespace().next() {
        Some(token) => token == "tsc" || has_version_at(token),
        None => false,
    }
}

/// Whether `line` is progress output that carries no information.
fn is_noise(line: &str) -> bool {
    if is_failure_line(line) {
        return false;
    }
    let line = line.trim_start();
    if NOISE_PREFIXES.iter().any(|prefix| line.starts_with(prefix)) {
        return true;
    }
    let trimmed = line.trim_end();
    if trimmed.starts_with("test ") && trimmed.ends_with(" ... ok") {
        return true;
    }
    if !trimmed.is_empty() && trimmed.chars().all(|c| c == '+' || c == '-') {
        return true;
    }
    is_script_echo(line)
}

/// Whether `line` reports a failure (compiler error, failed test, failed job).
///
/// Matches the markers `error:`, `error[E`, `error TS`, `ERROR:`, `ERR_PNPM`,
/// `npm ERR!`, `FAILED`, `panicked at`, `Job failed` and `test result: FAILED`
/// anywhere in the line, and the cargo test failure header
/// (`---- name stdout ----`).
///
/// # Examples
///
/// ```
/// use ironflow_ops_common::log::is_failure_line;
///
/// assert!(is_failure_line("error[E0308]: mismatched types"));
/// assert!(is_failure_line("---- tests::it_works stdout ----"));
/// assert!(!is_failure_line("   Compiling app v0.1.0"));
/// ```
pub fn is_failure_line(line: &str) -> bool {
    FAILURE_MARKERS.iter().any(|marker| line.contains(marker)) || is_test_failure_header(line)
}

/// Remove the progress lines of known tools and collapse blank runs.
///
/// Dropped lines: cargo progress (`Compiling`, `Downloading`, `Finished`,
/// `test x ... ok`, ...), pnpm progress (`Progress: resolved`, `+++---` bars,
/// ...) and the echo of an npm/pnpm script (`> pkg@1.0.0 build`, `> tsc`).
/// A line for which [`is_failure_line`] is true is never dropped. Runs of two
/// or more blank lines become one. The order of the other lines is kept.
///
/// # Examples
///
/// ```
/// use ironflow_ops_common::log::drop_noise;
///
/// let log = "   Compiling app v0.1.0\nwarning: unused\n\n\n\nerror: boom";
/// assert_eq!(drop_noise(log), "warning: unused\n\nerror: boom");
/// ```
pub fn drop_noise(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut previous_blank = false;
    for line in text.split('\n') {
        if is_noise(line) {
            continue;
        }
        let blank = line.trim().is_empty();
        if blank && previous_blank {
            continue;
        }
        previous_blank = blank;
        kept.push(line);
    }
    kept.join("\n")
}

/// Running selection of lines with the exact char cost of the assembled text.
struct Selection {
    selected: Vec<bool>,
    lens: Vec<usize>,
    /// Sum of `len + 1` over the selected lines.
    sum: usize,
    /// Number of runs of unselected lines (each becomes one marker line).
    runs: usize,
    /// Cost of one marker line with its newline.
    gap_cost: usize,
}

impl Selection {
    fn new(lines: &[&str], marker_chars: usize) -> Self {
        Self {
            selected: vec![false; lines.len()],
            lens: lines.iter().map(|l| l.chars().count()).collect(),
            sum: 0,
            runs: usize::from(!lines.is_empty()),
            gap_cost: marker_chars + 1,
        }
    }

    fn cost_of(&self, sum: usize, runs: usize) -> usize {
        (sum + runs * self.gap_cost).saturating_sub(1)
    }

    /// Select line `i` when the assembled text stays within `limit` chars.
    fn try_select(&mut self, i: usize, limit: usize) -> bool {
        if self.selected[i] {
            return true;
        }
        let previous_unselected = i > 0 && !self.selected[i - 1];
        let next_unselected = i + 1 < self.selected.len() && !self.selected[i + 1];
        let runs = match (previous_unselected, next_unselected) {
            (true, true) => self.runs + 1,
            (false, false) => self.runs - 1,
            _ => self.runs,
        };
        let sum = self.sum + self.lens[i] + 1;
        if self.cost_of(sum, runs) > limit {
            return false;
        }
        self.selected[i] = true;
        self.sum = sum;
        self.runs = runs;
        true
    }
}

/// End (exclusive) of the failure block starting at line `start`.
fn block_end(lines: &[&str], start: usize) -> usize {
    let cargo_test = is_test_failure_header(lines[start]);
    let mut end = start + 1;
    while end < lines.len()
        && end - start < MAX_BLOCK_LINES
        && !lines[end].trim().is_empty()
        && !(cargo_test && lines[end].trim() == "failures:")
    {
        end += 1;
    }
    end
}

/// Keep the relevant part of `text` within `budget` chars.
///
/// A text of at most `budget` chars is returned unchanged. Otherwise the
/// result keeps, by priority: the tail of the log (up to a third of the
/// budget), every failure block (a failure line up to the next blank line, at
/// most 40 lines), then the lines leading to the first failure (at most 30).
/// The lines are emitted in their original order and every gap of skipped
/// lines is replaced by one line holding `marker`, verbatim. The marker lines
/// count in the budget. A log without any failure keeps its tail only,
/// preceded by the marker.
///
/// The result never exceeds `budget` chars, whatever the input: when the
/// assembled text is still too long (a giant line, a marker as long as the
/// budget), it is cut on a char boundary to the marker and the last chars of
/// the text, or to the first chars of the marker alone.
///
/// # Panics
///
/// Panics if `budget` is zero.
///
/// # Examples
///
/// ```
/// use ironflow_ops_common::log::truncate_around_failure;
///
/// let text = "a\nb\nc\nd\ne\nf\ng\nh";
/// let short = truncate_around_failure(text, 7, "[...]");
/// assert!(short.chars().count() <= 7);
/// assert!(short.starts_with("[...]"));
/// assert_eq!(truncate_around_failure(text, 100, "[...]"), text);
/// ```
pub fn truncate_around_failure(text: &str, budget: usize, marker: &str) -> String {
    assert!(budget > 0, "budget must be greater than zero");
    if text.chars().count() <= budget {
        return text.to_string();
    }

    let lines: Vec<&str> = text.split('\n').collect();
    let marker_chars = marker.chars().count();
    let first_failure = lines.iter().position(|line| is_failure_line(line));
    let mut selection = Selection::new(&lines, marker_chars);

    // 1. The tail of the log.
    let tail_limit = if first_failure.is_some() {
        budget / TAIL_SHARE
    } else {
        budget
    };
    for i in (0..lines.len()).rev() {
        if !selection.try_select(i, tail_limit) {
            break;
        }
    }

    if let Some(first) = first_failure {
        // 2. Every failure block, in order.
        'blocks: for (start, line) in lines.iter().enumerate() {
            if !is_failure_line(line) {
                continue;
            }
            for i in start..block_end(&lines, start) {
                if !selection.try_select(i, budget) {
                    break 'blocks;
                }
            }
        }
        // 3. The lines leading to the first failure.
        for i in (first.saturating_sub(LEAD_IN_LINES)..first).rev() {
            if !selection.try_select(i, budget) {
                break;
            }
        }
    }

    let mut out: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if selection.selected[i] {
            out.push(line);
        } else if i == 0 || selection.selected[i - 1] {
            out.push(marker);
        }
    }
    let assembled = out.join("\n");
    // Nothing selected means no line fits the budget: the marker alone would
    // drop the whole log, so fall through to the cut below.
    let any_selected = selection.selected.iter().any(|&s| s);
    if any_selected && assembled.chars().count() <= budget {
        return assembled;
    }

    if marker_chars >= budget {
        return marker.chars().take(budget).collect();
    }
    let keep = budget - marker_chars - 1;
    let total = text.chars().count();
    let tail: String = text.chars().skip(total - keep).collect();
    format!("{marker}\n{tail}")
}

/// Remove the noise of `text`, then keep the relevant part within `budget` chars.
///
/// Equivalent to [`truncate_around_failure`] applied to [`drop_noise`]'s
/// output: noise is removed first, so a log that fits after noise removal is
/// returned without marker.
///
/// # Panics
///
/// Panics if `budget` is zero.
///
/// # Examples
///
/// ```
/// use ironflow_ops_common::log::condense_log;
///
/// let log = "Compiling a v1\nerror: boom\nJob failed";
/// assert_eq!(condense_log(log, 50, "[...]"), "error: boom\nJob failed");
/// ```
pub fn condense_log(text: &str, budget: usize, marker: &str) -> String {
    truncate_around_failure(&drop_noise(text), budget, marker)
}
