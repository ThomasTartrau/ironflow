//! Output stream classification for run/step log lines.

use serde::{Deserialize, Serialize};

/// Output stream for a log line.
///
/// # Examples
///
/// ```
/// use ironflow_store::entities::LogStream;
///
/// let stream: LogStream = "stdout".parse().unwrap();
/// assert_eq!(stream.as_str(), "stdout");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
    /// System-level messages (e.g. step start/stop notifications).
    System,
}

impl LogStream {
    /// Returns the wire-format string for this stream.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::System => "system",
        }
    }
}

impl std::fmt::Display for LogStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for LogStream {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            "system" => Ok(Self::System),
            _ => Err(format!("unknown log stream: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let stream = LogStream::Stdout;
        let json = serde_json::to_string(&stream).unwrap();
        assert_eq!(json, r#""stdout""#);
        let back: LogStream = serde_json::from_str(&json).unwrap();
        assert_eq!(back, LogStream::Stdout);
    }

    #[test]
    fn from_str_all_variants() {
        assert_eq!("stdout".parse::<LogStream>().unwrap(), LogStream::Stdout);
        assert_eq!("stderr".parse::<LogStream>().unwrap(), LogStream::Stderr);
        assert_eq!("system".parse::<LogStream>().unwrap(), LogStream::System);
    }

    #[test]
    fn from_str_unknown_errors() {
        assert!("unknown".parse::<LogStream>().is_err());
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(LogStream::Stdout.to_string(), "stdout");
        assert_eq!(LogStream::Stderr.to_string(), "stderr");
        assert_eq!(LogStream::System.to_string(), "system");
    }

    #[test]
    fn as_str_values() {
        assert_eq!(LogStream::Stdout.as_str(), "stdout");
        assert_eq!(LogStream::Stderr.as_str(), "stderr");
        assert_eq!(LogStream::System.as_str(), "system");
    }
}
