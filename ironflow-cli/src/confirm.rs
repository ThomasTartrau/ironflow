//! Interactive confirmation for destructive commands, and secret input.
//!
//! Destructive commands (`delete`) ask for an explicit `y` before acting.
//! `--yes` skips the prompt. When stdin is not a terminal and `--yes` was not
//! passed, the command fails instead of prompting: a prompt nobody can answer
//! would hang a CI job forever, and silently proceeding would make a forgotten
//! `--yes` destroy data without a trace.

use std::io::{BufRead, IsTerminal, Read, Write};

use anyhow::{Result, bail};

/// Ask for confirmation on stdin unless `assume_yes` is set.
///
/// # Errors
///
/// Returns an error when the answer is not affirmative, when stdin is not a
/// terminal, or when reading stdin fails.
///
/// # Examples
///
/// ```
/// use ironflow_cli::confirm::confirm;
///
/// # fn example() -> anyhow::Result<()> {
/// confirm("Delete secret 'db/password'?", true)?;
/// # Ok(())
/// # }
/// ```
pub fn confirm(prompt: &str, assume_yes: bool) -> Result<()> {
    let stdin = std::io::stdin();
    confirm_with(prompt, assume_yes, stdin.is_terminal(), &mut stdin.lock())
}

/// Confirmation logic decoupled from the process' real stdin, for testing.
///
/// # Errors
///
/// Returns an error when `interactive` is false without `assume_yes`, when the
/// answer is not affirmative, or when reading fails.
fn confirm_with<R: BufRead>(
    prompt: &str,
    assume_yes: bool,
    interactive: bool,
    reader: &mut R,
) -> Result<()> {
    if assume_yes {
        return Ok(());
    }

    if !interactive {
        bail!("refusing to prompt on a non-interactive stdin; pass --yes to confirm");
    }

    let mut stderr = std::io::stderr();
    write!(stderr, "{prompt} [y/N] ")?;
    stderr.flush()?;

    let mut answer = String::new();
    reader.read_line(&mut answer)?;

    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => bail!("aborted"),
    }
}

/// Resolve a sensitive value from an argument, falling back to stdin.
///
/// Passing the value as an argument leaks it into the shell history and into
/// `ps` output, so omitting it reads the value from stdin instead. Only the
/// trailing newline is stripped: a value may legitimately contain inner
/// newlines or leading whitespace.
///
/// # Errors
///
/// Returns an error when stdin cannot be read, or when the resolved value is
/// empty.
///
/// # Examples
///
/// ```
/// use ironflow_cli::confirm::resolve_secret_value;
///
/// # fn example() -> anyhow::Result<()> {
/// let value = resolve_secret_value(Some("hunter2"), "value")?;
/// assert_eq!(value, "hunter2");
/// # Ok(())
/// # }
/// ```
pub fn resolve_secret_value(argument: Option<&str>, label: &str) -> Result<String> {
    match argument {
        Some(value) => reject_empty(value.to_string(), label),
        None => {
            let mut buffer = String::new();
            std::io::stdin().read_to_string(&mut buffer)?;
            reject_empty(strip_trailing_newline(&buffer).to_string(), label)
        }
    }
}

/// Drop a single trailing `\n` or `\r\n`.
fn strip_trailing_newline(raw: &str) -> &str {
    raw.strip_suffix('\n')
        .map_or(raw, |s| s.strip_suffix('\r').unwrap_or(s))
}

/// Reject an empty value before it reaches the API.
fn reject_empty(value: String, label: &str) -> Result<String> {
    if value.is_empty() {
        bail!("{label} must not be empty");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn answer(input: &str) -> Result<()> {
        let mut reader = Cursor::new(input.as_bytes().to_vec());
        confirm_with("Delete?", false, true, &mut reader)
    }

    #[test]
    fn assume_yes_skips_the_prompt() {
        let mut reader = Cursor::new(Vec::new());
        assert!(confirm_with("Delete?", true, false, &mut reader).is_ok());
    }

    #[test]
    fn non_interactive_stdin_is_refused() {
        let mut reader = Cursor::new(b"y\n".to_vec());
        let err = confirm_with("Delete?", false, false, &mut reader).unwrap_err();
        assert!(err.to_string().contains("--yes"), "{err}");
    }

    #[test]
    fn affirmative_answers_are_accepted() {
        assert!(answer("y\n").is_ok());
        assert!(answer("Y\n").is_ok());
        assert!(answer("yes\n").is_ok());
        assert!(answer("  YES  \n").is_ok());
    }

    #[test]
    fn other_answers_abort() {
        for input in ["n\n", "no\n", "\n", "", "maybe\n", "yep\n"] {
            let err = answer(input).unwrap_err();
            assert_eq!(err.to_string(), "aborted", "input {input:?}");
        }
    }

    #[test]
    fn argument_value_is_used_verbatim() {
        assert_eq!(
            resolve_secret_value(Some(" spaced "), "value").unwrap(),
            " spaced "
        );
    }

    #[test]
    fn empty_argument_is_rejected() {
        let err = resolve_secret_value(Some(""), "value").unwrap_err();
        assert!(err.to_string().contains("must not be empty"), "{err}");
    }

    #[test]
    fn only_the_trailing_newline_is_stripped() {
        assert_eq!(strip_trailing_newline("secret\n"), "secret");
        assert_eq!(strip_trailing_newline("secret\r\n"), "secret");
        assert_eq!(strip_trailing_newline("secret"), "secret");
        assert_eq!(strip_trailing_newline("line1\nline2\n"), "line1\nline2");
        assert_eq!(strip_trailing_newline("secret\n\n"), "secret\n");
        assert_eq!(strip_trailing_newline("  padded  "), "  padded  ");
    }
}
