//! Decode a GitLab runner job trace: ANSI sequences, timestamp prefixes and section markers.

/// Remove ANSI escape sequences from `text`.
///
/// Strips CSI sequences (`ESC [` ... final byte, colors, erase line), OSC
/// sequences (`ESC ]` ... up to BEL or `ESC \`) and any other two-char
/// `ESC x` sequence. A dangling ESC at the end of the input is dropped. Plain
/// text and unicode are untouched.
///
/// # Examples
///
/// ```
/// use ironflow_ops_gitlab::strip_ansi;
///
/// assert_eq!(strip_ansi("\u{1b}[31;1mred\u{1b}[0m"), "red");
/// ```
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            None => {}
            Some('[') => {
                while let Some(&next) = chars.peek() {
                    match next {
                        '\u{20}'..='\u{3f}' => {
                            chars.next();
                        }
                        '\u{40}'..='\u{7e}' => {
                            chars.next();
                            break;
                        }
                        _ => break,
                    }
                }
            }
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\u{7}' {
                        break;
                    }
                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {}
        }
    }
    out
}

/// Length of the runner prefix (`2024-05-01T10:00:00.123456Z 01O `) of `line`, or 0.
fn runner_prefix_len(line: &str) -> usize {
    let bytes = line.as_bytes();
    let digit = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_digit);
    let at = |i: usize, b: u8| bytes.get(i) == Some(&b);
    let digits = |from: usize, count: usize| (from..from + count).all(digit);

    let shape = digits(0, 4)
        && at(4, b'-')
        && digits(5, 2)
        && at(7, b'-')
        && digits(8, 2)
        && at(10, b'T')
        && digits(11, 2)
        && at(13, b':')
        && digits(14, 2)
        && at(16, b':')
        && digits(17, 2);
    if !shape {
        return 0;
    }
    let mut i = 19;
    if at(i, b'.') {
        i += 1;
        let start = i;
        while digit(i) {
            i += 1;
        }
        if i == start {
            return 0;
        }
    }
    if !at(i, b'Z') || !at(i + 1, b' ') || !digits(i + 2, 2) {
        return 0;
    }
    i += 4;
    if !(at(i, b'O') || at(i, b'E')) {
        return 0;
    }
    i += 1;
    if at(i, b'+') {
        i += 1;
    }
    if at(i, b' ') { i + 1 } else { 0 }
}

/// Whether `line` is a bare `section_start:<digits>:<name>` or `section_end:...` marker.
fn is_section_marker(line: &str) -> bool {
    let Some(rest) = line
        .strip_prefix("section_start:")
        .or_else(|| line.strip_prefix("section_end:"))
    else {
        return false;
    };
    let Some((timestamp, name)) = rest.split_once(':') else {
        return false;
    };
    !timestamp.is_empty()
        && timestamp.chars().all(|c| c.is_ascii_digit())
        && !name.is_empty()
        && !name.chars().any(char::is_whitespace)
}

/// Remove the runner's own framing from every line of a trace.
///
/// Per line: the timestamp prefix (`2024-05-01T10:00:00.123456Z 01O `, with
/// an optional `+` append marker) is removed when it matches strictly, a
/// trailing `\r` is dropped and only the text after the last `\r` is kept
/// (the runner redraws progress and section markers with `\r`), and a line
/// reduced to a bare `section_start:<digits>:<name>` or `section_end:...`
/// marker is dropped. Other lines, empty ones included, are kept.
///
/// # Examples
///
/// ```
/// use ironflow_ops_gitlab::unwrap_runner_lines;
///
/// let trace = "2024-05-01T10:00:00.123456Z 01O cargo build\nsection_end:1714:build\n";
/// assert_eq!(unwrap_runner_lines(trace), "cargo build\n");
/// ```
pub fn unwrap_runner_lines(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        let line = &line[runner_prefix_len(line)..];
        let line = line.strip_suffix('\r').unwrap_or(line);
        let (redrawn, line) = match line.rsplit_once('\r') {
            Some((before, last)) => (Some(before), last),
            None => (None, line),
        };
        // The runner ends a marker with `\r` + `ESC[0K`: once the redraw is
        // applied the line looks empty, so the marker is looked for before it.
        let marker_line = redrawn.unwrap_or(line);
        let marker_only = redrawn.is_none() || strip_ansi(line).is_empty();
        if marker_only && is_section_marker(strip_ansi(marker_line).trim()) {
            continue;
        }
        out.push(line);
    }
    out.join("\n")
}

/// Decode a job trace: [`unwrap_runner_lines`] then [`strip_ansi`].
///
/// The runner framing goes first: the `\r` + `ESC[0K` pair must be split
/// before the ANSI removal, and the timestamp prefix comes before any ANSI
/// sequence.
///
/// # Examples
///
/// ```
/// use ironflow_ops_gitlab::decode_trace;
///
/// let trace = "2024-05-01T10:00:00.5Z 01O \u{1b}[32;1mOK\u{1b}[0;m\n";
/// assert_eq!(decode_trace(trace), "OK\n");
/// ```
pub fn decode_trace(trace: &str) -> String {
    strip_ansi(&unwrap_runner_lines(trace))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_colors_and_erase_line() {
        assert_eq!(
            strip_ansi("\u{1b}[0K\u{1b}[31;1mred\u{1b}[0;m ok"),
            "red ok"
        );
    }

    #[test]
    fn strip_ansi_removes_osc_and_dangling_escape() {
        assert_eq!(strip_ansi("a\u{1b}]0;title\u{7}b"), "ab");
        assert_eq!(strip_ansi("a\u{1b}]8;;url\u{1b}\\b"), "ab");
        assert_eq!(strip_ansi("a\u{1b}Mb"), "ab");
        assert_eq!(strip_ansi("end\u{1b}"), "end");
        assert_eq!(strip_ansi("end\u{1b}[31"), "end");
    }

    #[test]
    fn strip_ansi_keeps_plain_unicode() {
        assert_eq!(strip_ansi("trace é€ 日本 ✓"), "trace é€ 日本 ✓");
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn unwrap_runner_lines_removes_timestamp_and_stream() {
        let trace = "2024-05-01T10:00:00.123456Z 01O cargo build\n\
                     2024-05-01T10:00:01Z 02E error: boom\n\
                     2024-05-01T10:00:02.5Z 01O+ appended\r\n";
        assert_eq!(
            unwrap_runner_lines(trace),
            "cargo build\nerror: boom\nappended\n"
        );
    }

    #[test]
    fn unwrap_runner_lines_keeps_text_after_last_cr() {
        assert_eq!(
            unwrap_runner_lines("10%\r50%\r100% done\nplain\r\n"),
            "100% done\nplain\n"
        );
    }

    #[test]
    fn unwrap_runner_lines_drops_section_markers() {
        let trace = "section_start:1714:prepare\nsection_end:1715:prepare\nkept\n\n\
                     section_start:1714:build\r\u{1b}[0KBuilding\n\
                     section_end:1716:build\r\u{1b}[0K\n\
                     \u{1b}[0Ksection_end:1717:build\r\u{1b}[0K\n";
        assert_eq!(unwrap_runner_lines(trace), "kept\n\n\u{1b}[0KBuilding\n");
    }

    #[test]
    fn unwrap_runner_lines_leaves_non_matching_line() {
        let trace = "see 2024-05-01T10:00:00Z 01O inside\n2024-05-01 10:00:00 plain\n2024-05-01T10:00:00Z 1O short\nsection_start:abc:x\nsection_start:1:two words";
        assert_eq!(unwrap_runner_lines(trace), trace);
    }

    #[test]
    fn decode_trace_removes_ansi_and_timestamps() {
        let trace = "2024-05-01T10:00:00.000001Z 00O section_start:1714:prepare_executor\r\u{1b}[0K\u{1b}[0K\u{1b}[36;1mPreparing the \"docker\" executor\u{1b}[0;m\n\
                     2024-05-01T10:00:01.000002Z 01O \u{1b}[32;1m$ cargo test\u{1b}[0;m\n\
                     2024-05-01T10:00:02.000003Z 01E error[E0308]: mismatched types\n\
                     2024-05-01T10:00:03.000004Z 00O section_end:1715:prepare_executor\r\u{1b}[0K\n\
                     2024-05-01T10:00:04.000005Z 01O \u{1b}[31;1mERROR: Job failed: exit code 1\u{1b}[0;m\r\n";
        assert_eq!(
            decode_trace(trace),
            "Preparing the \"docker\" executor\n$ cargo test\nerror[E0308]: mismatched types\nERROR: Job failed: exit code 1\n"
        );
    }
}
