//! CLI command handlers.

use std::str::FromStr;

pub mod api_key;
pub mod audit_log;
pub mod logs;
pub mod run;
pub mod secret;
pub mod stats;
pub mod template;
pub mod user;
pub mod workflow;

/// Parse a CLI value into one of the SDK's generated enums.
///
/// The generated `FromStr` error carries no information, so it is replaced by a
/// message that tells the caller what to type instead. The generated enums
/// expose no variant iterator, hence `accepted` being passed in.
///
/// # Errors
///
/// Returns the list of accepted values when `raw` is not one of them.
pub(crate) fn parse_enum<T: FromStr + ToString>(
    raw: &str,
    accepted: &[T],
    noun: &str,
) -> Result<T, String> {
    T::from_str(raw).map_err(|_| {
        let accepted: Vec<String> = accepted.iter().map(ToString::to_string).collect();
        format!("unknown {noun} '{raw}'; accepted: {}", accepted.join(", "))
    })
}
