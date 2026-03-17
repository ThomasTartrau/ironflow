//! Utility functions for output handling and OOM protection.
//!
//! Shell commands and agent processes can produce arbitrarily large output.
//! This module provides [`truncate_output`] to cap captured output at
//! [`MAX_OUTPUT_SIZE`] bytes, preventing out-of-memory conditions in
//! long-running workflows.

use tracing::warn;

/// Maximum number of bytes kept from a single process output stream (10 MB).
///
/// Any output beyond this limit is silently dropped after a warning is logged.
pub const MAX_OUTPUT_SIZE: usize = 10 * 1024 * 1024; // 10 MB

/// Truncate raw process output to at most [`MAX_OUTPUT_SIZE`] bytes and
/// convert it to a UTF-8 [`String`].
///
/// If the data exceeds the limit a warning is emitted via [`tracing`] with
/// the provided `context` label. Invalid UTF-8 sequences are replaced with
/// the Unicode replacement character (U+FFFD). Trailing whitespace is trimmed.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::utils::truncate_output;
///
/// let data = b"hello world\n";
/// let output = truncate_output(data, "my-step");
/// assert_eq!(output, "hello world");
/// ```
pub fn truncate_output(data: &[u8], context: &str) -> String {
    if data.len() > MAX_OUTPUT_SIZE {
        warn!(
            bytes = data.len(),
            max = MAX_OUTPUT_SIZE,
            "{context} output truncated to limit"
        );
    }
    let truncated = &data[..data.len().min(MAX_OUTPUT_SIZE)];
    match std::str::from_utf8(truncated) {
        Ok(s) => s.trim_end().to_string(),
        Err(_) => {
            let cow = String::from_utf8_lossy(truncated);
            cow.trim_end().to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_input_returned_as_is() {
        let data = b"hello world";
        let result = truncate_output(data, "test");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn input_exactly_at_max_output_size_returned_as_is() {
        let data = vec![b'a'; MAX_OUTPUT_SIZE];
        let result = truncate_output(&data, "test");
        assert_eq!(result.len(), MAX_OUTPUT_SIZE);
        assert!(result.chars().all(|c| c == 'a'));
    }

    #[test]
    fn input_over_limit_gets_truncated() {
        let data = vec![b'x'; MAX_OUTPUT_SIZE + 100];
        let result = truncate_output(&data, "test");
        assert_eq!(result.len(), MAX_OUTPUT_SIZE);
    }

    #[test]
    fn way_over_limit_gets_truncated() {
        let data = vec![b'z'; 20 * 1024 * 1024];
        let result = truncate_output(&data, "test");
        assert_eq!(result.len(), MAX_OUTPUT_SIZE);
    }

    #[test]
    fn empty_input_returns_empty_string() {
        let result = truncate_output(b"", "test");
        assert_eq!(result, "");
    }

    #[test]
    fn trailing_whitespace_trimmed() {
        let data = b"hello   \n\n  ";
        let result = truncate_output(data, "test");
        assert_eq!(result, "hello");
    }

    #[test]
    fn only_whitespace_returns_empty() {
        let data = b"   \n\t  \r\n  ";
        let result = truncate_output(data, "test");
        assert_eq!(result, "");
    }

    #[test]
    fn invalid_utf8_uses_lossy_conversion() {
        let data: &[u8] = &[0xFF, 0xFE, b'h', b'i'];
        let result = truncate_output(data, "test");
        assert!(result.contains('\u{FFFD}'));
        assert!(result.contains("hi"));
    }

    #[test]
    fn valid_multibyte_utf8_emoji_handled() {
        let data = "\u{1F680} rocket".as_bytes();
        let result = truncate_output(data, "test");
        assert_eq!(result, "\u{1F680} rocket");
    }

    #[test]
    fn truncation_splitting_multibyte_char_handled_via_lossy() {
        // Create data at exactly MAX_OUTPUT_SIZE where the last bytes are a partial UTF-8 char.
        let mut data = vec![b'a'; MAX_OUTPUT_SIZE - 1];
        // Add the first 2 bytes of a 4-byte UTF-8 sequence (rocket emoji U+1F680 = F0 9F 9A 80)
        data.push(0xF0);
        // This is now MAX_OUTPUT_SIZE bytes, with a partial multibyte char at the end.
        // Extend past the limit so truncation occurs mid-character.
        data.push(0x9F);
        data.push(0x9A);
        data.push(0x80);
        let result = truncate_output(&data, "test");
        // The truncated partial sequence should be replaced with the replacement character.
        assert!(result.contains('\u{FFFD}'));
    }

    #[test]
    fn null_bytes_in_data() {
        let data: &[u8] = &[b'a', 0x00, b'b', 0x00, b'c'];
        let result = truncate_output(data, "test");
        assert_eq!(result, "a\0b\0c");
    }
}
