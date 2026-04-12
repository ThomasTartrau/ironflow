//! [`MessageFormatter`] trait -- platform-specific event formatting.
//!
//! Subscribers can accept an optional [`MessageFormatter`] to customise
//! how domain events are rendered for their target platform. This lets
//! the same subscriber struct (e.g. a future `SlackSubscriber`) produce
//! Slack Block Kit, Discord Embeds, or Telegram Markdown without
//! changing its delivery logic.

use super::Event;

/// A formatted message ready to be sent to an external platform.
///
/// The `body` field contains the platform-specific payload (JSON string,
/// Markdown text, etc.). `content_type` tells the subscriber which
/// `Content-Type` header to use when delivering.
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::FormattedMessage;
///
/// let message = FormattedMessage::json(r#"{"text":"hello"}"#);
/// assert_eq!(message.content_type(), "application/json");
/// ```
#[derive(Debug, Clone)]
pub struct FormattedMessage {
    body: String,
    content_type: &'static str,
}

impl FormattedMessage {
    /// Create a message with an explicit content type.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::FormattedMessage;
    ///
    /// let msg = FormattedMessage::new("hello", "text/plain");
    /// assert_eq!(msg.body(), "hello");
    /// assert_eq!(msg.content_type(), "text/plain");
    /// ```
    pub fn new(body: &str, content_type: &'static str) -> Self {
        Self {
            body: body.to_string(),
            content_type,
        }
    }

    /// Create a JSON message (`application/json`).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::FormattedMessage;
    ///
    /// let msg = FormattedMessage::json(r#"{"text":"hello"}"#);
    /// assert_eq!(msg.content_type(), "application/json");
    /// ```
    pub fn json(body: &str) -> Self {
        Self::new(body, "application/json")
    }

    /// Create a plain text message (`text/plain`).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::notify::FormattedMessage;
    ///
    /// let msg = FormattedMessage::text("hello");
    /// assert_eq!(msg.content_type(), "text/plain");
    /// ```
    pub fn text(body: &str) -> Self {
        Self::new(body, "text/plain")
    }

    /// The message body.
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The MIME content type.
    pub fn content_type(&self) -> &'static str {
        self.content_type
    }
}

/// Converts domain events into platform-specific messages.
///
/// Implement this trait to control how events appear on Slack, Discord,
/// Telegram, or any other messaging platform. The formatter is decoupled
/// from delivery: subscribers handle retries and HTTP, formatters handle
/// presentation.
///
/// Return `None` from [`format`](MessageFormatter::format) to silently
/// skip events that the formatter does not care about.
///
/// # Examples
///
/// ```
/// use ironflow_engine::notify::{Event, FormattedMessage, MessageFormatter};
///
/// struct PlainTextFormatter;
///
/// impl MessageFormatter for PlainTextFormatter {
///     fn name(&self) -> &str { "plain-text" }
///
///     fn format(&self, event: &Event) -> Option<FormattedMessage> {
///         let text = format!("[{}] event fired", event.event_type());
///         Some(FormattedMessage::text(&text))
///     }
/// }
///
/// let formatter = PlainTextFormatter;
/// let event = Event::RunCreated {
///     run_id: uuid::Uuid::now_v7(),
///     workflow_name: "deploy".to_string(),
///     at: chrono::Utc::now(),
/// };
/// let msg = formatter.format(&event).unwrap();
/// assert!(msg.body().contains("run_created"));
/// ```
pub trait MessageFormatter: Send + Sync {
    /// A short identifier for this formatter (used in logs).
    fn name(&self) -> &str;

    /// Convert an event into a platform-specific message.
    ///
    /// Return `None` to skip the event silently.
    fn format(&self, event: &Event) -> Option<FormattedMessage>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn formatted_message_json() {
        let msg = FormattedMessage::json(r#"{"text":"hello"}"#);
        assert_eq!(msg.body(), r#"{"text":"hello"}"#);
        assert_eq!(msg.content_type(), "application/json");
    }

    #[test]
    fn formatted_message_text() {
        let msg = FormattedMessage::text("hello world");
        assert_eq!(msg.body(), "hello world");
        assert_eq!(msg.content_type(), "text/plain");
    }

    #[test]
    fn formatted_message_custom_content_type() {
        let msg = FormattedMessage::new("<b>bold</b>", "text/html");
        assert_eq!(msg.body(), "<b>bold</b>");
        assert_eq!(msg.content_type(), "text/html");
    }

    struct TestFormatter;

    impl MessageFormatter for TestFormatter {
        fn name(&self) -> &str {
            "test"
        }

        fn format(&self, event: &Event) -> Option<FormattedMessage> {
            match event {
                Event::RunCreated { workflow_name, .. } => {
                    let body = format!(r#"{{"text":"Run created for {}"}}"#, workflow_name);
                    Some(FormattedMessage::json(&body))
                }
                _ => None,
            }
        }
    }

    #[test]
    fn formatter_formats_matching_event() {
        let formatter = TestFormatter;
        let event = Event::RunCreated {
            run_id: Uuid::now_v7(),
            workflow_name: "deploy".to_string(),
            at: Utc::now(),
        };

        let msg = formatter.format(&event);
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert!(msg.body().contains("deploy"));
        assert_eq!(msg.content_type(), "application/json");
    }

    #[test]
    fn formatter_skips_non_matching_event() {
        let formatter = TestFormatter;
        let event = Event::UserSignedIn {
            user_id: Uuid::now_v7(),
            username: "alice".to_string(),
            at: Utc::now(),
        };

        assert!(formatter.format(&event).is_none());
    }

    #[test]
    fn formatter_name() {
        let formatter = TestFormatter;
        assert_eq!(formatter.name(), "test");
    }
}
