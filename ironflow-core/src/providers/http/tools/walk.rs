//! Confined directory walk and bounded output shared by `grep` and `glob`.

use std::fs::canonicalize;
use std::path::Path;
use std::time::{Duration, Instant};

use ignore::{DirEntry, WalkBuilder};

use super::confinement::AllowedRoots;

/// Maximum number of results (matching lines or paths) returned.
pub(crate) const MAX_RESULTS: usize = 200;

/// Maximum size of the returned output, truncation notice excluded (64 KiB).
pub(crate) const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Wall-clock budget of one search.
pub(crate) const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Build a walker over `start` that stays inside `roots`.
///
/// Hidden files are walked, `.git` is skipped, `.gitignore` applies even
/// outside a git repository, and entries come in file-name order. A symbolic
/// link is followed only when its canonical target lies under a root: a link
/// escaping the roots is skipped, never descended into.
pub(crate) fn confined_walker(roots: &AllowedRoots, start: &Path) -> WalkBuilder {
    let roots = roots.clone();
    let mut builder = WalkBuilder::new(start);
    builder
        .hidden(false)
        .require_git(false)
        .follow_links(true)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(move |entry| stays_inside(&roots, entry));
    builder
}

/// Call `visit` on every regular file of the walk, in walk order, until it
/// returns `false` or `deadline` passes, and return the final text (`empty`
/// when nothing was pushed).
///
/// An unreadable entry (permissions, symlink loop) is skipped, as `rg` does.
pub(crate) fn walk_files(
    walker: WalkBuilder,
    deadline: Instant,
    empty: &str,
    mut visit: impl FnMut(&Path, &mut BoundedOutput) -> bool,
) -> String {
    let mut out = BoundedOutput::default();
    for entry in walker.build() {
        if Instant::now() >= deadline {
            out.truncate(Truncation::Timeout);
            break;
        }
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_some_and(|kind| kind.is_file()) && !visit(entry.path(), &mut out) {
            break;
        }
    }
    out.finish(empty)
}

fn stays_inside(roots: &AllowedRoots, entry: &DirEntry) -> bool {
    if entry.file_name() == ".git" {
        return false;
    }
    if entry.path_is_symlink() {
        return canonicalize(entry.path()).is_ok_and(|target| roots.contains(&target));
    }
    true
}

/// Why a result list was cut short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Truncation {
    /// [`MAX_RESULTS`] reached.
    Results,
    /// [`MAX_OUTPUT_BYTES`] reached.
    Bytes,
    /// [`SEARCH_TIMEOUT`] elapsed.
    Timeout,
}

/// Newline-separated output capped at [`MAX_RESULTS`] lines and
/// [`MAX_OUTPUT_BYTES`] bytes.
#[derive(Debug, Default)]
pub(crate) struct BoundedOutput {
    text: String,
    count: usize,
    truncation: Option<Truncation>,
}

impl BoundedOutput {
    /// Append one result line. Returns `false` once a bound is reached, after
    /// which every further line is dropped.
    pub(crate) fn push(&mut self, line: &str) -> bool {
        if self.truncation.is_some() {
            return false;
        }
        if self.count == MAX_RESULTS {
            self.truncation = Some(Truncation::Results);
            return false;
        }
        let separator = usize::from(!self.text.is_empty());
        if self.text.len() + separator + line.len() > MAX_OUTPUT_BYTES {
            self.truncation = Some(Truncation::Bytes);
            return false;
        }
        if separator == 1 {
            self.text.push('\n');
        }
        self.text.push_str(line);
        self.count += 1;
        true
    }

    /// Record that the search stopped early for `reason`, unless it already
    /// stopped for another one.
    pub(crate) fn truncate(&mut self, reason: Truncation) {
        self.truncation.get_or_insert(reason);
    }

    /// Final text for the model: the results, `empty` when there are none,
    /// and an explicit notice when a bound cut the list short.
    pub(crate) fn finish(self, empty: &str) -> String {
        let notice = self.truncation.map(|reason| match reason {
            Truncation::Results => {
                format!("[truncated: stopped at {MAX_RESULTS} results, narrow the pattern or the path]")
            }
            Truncation::Bytes => format!(
                "[truncated: output limit of {} KiB reached, narrow the pattern or the path]",
                MAX_OUTPUT_BYTES / 1024
            ),
            Truncation::Timeout => format!(
                "[truncated: search stopped after {} s, results are partial, narrow the pattern or the path]",
                SEARCH_TIMEOUT.as_secs()
            ),
        });
        match (self.text.is_empty(), notice) {
            (true, None) => empty.to_string(),
            (true, Some(notice)) => notice,
            (false, None) => self.text,
            (false, Some(notice)) => format!("{}\n{notice}", self.text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_output_uses_the_empty_message() {
        assert_eq!(
            BoundedOutput::default().finish("No matches found"),
            "No matches found"
        );
    }

    #[test]
    fn stops_at_max_results_and_says_so() {
        let mut out = BoundedOutput::default();
        for i in 0..MAX_RESULTS {
            assert!(out.push(&format!("line {i}")));
        }
        assert!(!out.push("one too many"));
        let text = out.finish("none");
        assert_eq!(text.lines().count(), MAX_RESULTS + 1);
        assert!(!text.contains("one too many"));
        assert!(text.ends_with("narrow the pattern or the path]"), "{text}");
        assert!(text.contains("stopped at 200 results"), "{text}");
    }

    #[test]
    fn stops_at_max_bytes_and_says_so() {
        let mut out = BoundedOutput::default();
        let line = "x".repeat(1000);
        while out.push(&line) {}
        let text = out.finish("none");
        let (results, notice) = text.rsplit_once('\n').expect("notice line");
        assert!(results.len() <= MAX_OUTPUT_BYTES);
        assert!(notice.contains("64 KiB"), "{notice}");
    }

    #[test]
    fn first_truncation_reason_wins() {
        let mut out = BoundedOutput::default();
        out.push("a");
        out.truncate(Truncation::Timeout);
        out.truncate(Truncation::Results);
        assert!(!out.push("b"));
        let text = out.finish("none");
        assert!(text.starts_with("a\n"), "{text}");
        assert!(text.contains("after 10 s"), "{text}");
    }
}
