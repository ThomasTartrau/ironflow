//! SSE (Server-Sent Events) streaming helpers.
//!
//! Provides a [`Stream`] of domain events from the Ironflow
//! `GET /api/v1/events` endpoint.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_sdk::IronflowClient;
//! use futures_util::StreamExt;
//!
//! # async fn example() -> Result<(), ironflow_sdk::Error> {
//! let client = IronflowClient::new("https://ironflow.example.com", "my-api-key");
//! let mut stream = client.events(None, None).await?;
//!
//! while let Some(event) = stream.next().await {
//!     match event {
//!         Ok(ev) => {
//!             let _ = &ev.event_type;
//!             let _ = &ev.data;
//!         }
//!         Err(e) => { break; }
//!     }
//! }
//! # Ok(())
//! # }
//! ```

use std::pin::Pin;

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use futures_util::stream::Stream;
use reqwest::Response;
use uuid::Uuid;

use crate::error::Error;

/// A parsed SSE event from the Ironflow API.
///
/// # Examples
///
/// ```
/// use ironflow_sdk::sse::SseEvent;
///
/// let event = SseEvent {
///     event_type: "run_status_changed".to_string(),
///     data: r#"{"run_id":"..."}"#.to_string(),
/// };
/// assert_eq!(event.event_type, "run_status_changed");
/// ```
#[derive(Debug, Clone)]
pub struct SseEvent {
    /// Event type (from the `event:` field, defaults to `"message"`).
    pub event_type: String,
    /// JSON payload (from the `data:` field).
    pub data: String,
}

impl SseEvent {
    /// Deserialize the `data` field into a typed event.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Deserialize`] if the JSON is invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_sdk::sse::SseEvent;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct Payload { value: i32 }
    ///
    /// let event = SseEvent {
    ///     event_type: "test".to_string(),
    ///     data: r#"{"value": 42}"#.to_string(),
    /// };
    /// let payload: Payload = event.deserialize().unwrap();
    /// assert_eq!(payload.value, 42);
    /// ```
    pub fn deserialize<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        serde_json::from_str(&self.data)
            .map_err(|e| Error::Deserialize(format!("{e}: {}", self.data)))
    }
}

/// A stream of SSE events from the Ironflow API.
///
/// Created by [`IronflowClient::events`](crate::IronflowClient::events).
pub type EventStream = Pin<Box<dyn Stream<Item = Result<SseEvent, Error>> + Send>>;

/// Parse an SSE byte stream into [`SseEvent`]s.
pub(crate) fn parse_sse_stream(response: Response) -> EventStream {
    let stream = response.bytes_stream().eventsource().map(|result| {
        result
            .map(|event| SseEvent {
                event_type: event.event,
                data: event.data,
            })
            .map_err(|e| Error::Sse(e.to_string()))
    });

    Box::pin(stream)
}

impl crate::IronflowClient {
    /// Connect to the SSE event stream at `GET /api/v1/events`.
    ///
    /// Returns a [`Stream`] of [`SseEvent`]s. Optionally filter by
    /// `run_id` and/or event types.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Http`] if the connection fails, or [`Error::Api`]
    /// if the server responds with a non-2xx status code.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use futures_util::StreamExt;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let mut stream = client.events(None, None).await?;
    ///
    /// while let Some(event) = stream.next().await {
    ///     let event = event?;
    ///     # let _ = event;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn events(
        &self,
        run_id: Option<Uuid>,
        types: Option<&[&str]>,
    ) -> Result<EventStream, Error> {
        let mut request = self.get("/api/v1/events");
        if let Some(rid) = run_id {
            request = request.query(&[("run_id", rid.to_string())]);
        }
        if let Some(ts) = types {
            request = request.query(&[("types", ts.join(","))]);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(Self::into_api_error(response).await);
        }
        Ok(parse_sse_stream(response))
    }

    /// Subscribe to SSE events for a specific run.
    ///
    /// Connects to `GET /api/v1/runs/{run_id}/events` and returns a
    /// [`Stream`](futures_util::Stream) of [`SseEvent`]s with automatic
    /// reconnection on network failures.
    ///
    /// Reconnection uses exponential backoff (1s, 2s, 4s, ... capped at 30s).
    /// The API does not support `Last-Event-ID`, so events emitted during
    /// a disconnection window are lost.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Http`] if the initial connection fails, or
    /// [`Error::Api`] on a non-2xx status code.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use futures_util::StreamExt;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let mut stream = client.subscribe_run_events(Uuid::nil()).await?;
    ///
    /// while let Some(event) = stream.next().await {
    ///     let event = event?;
    ///     # let _ = event;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_run_events(&self, run_id: Uuid) -> Result<EventStream, Error> {
        let path = format!("/api/v1/runs/{run_id}/events");
        let response = self.get(&path).send().await?;
        if !response.status().is_success() {
            return Err(Self::into_api_error(response).await);
        }
        Ok(parse_sse_stream(response))
    }

    /// Subscribe to the global SSE event stream.
    ///
    /// Connects to `GET /api/v1/events` and returns a
    /// [`Stream`](futures_util::Stream) of [`SseEvent`]s for all runs.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Http`] if the connection fails, or [`Error::Api`]
    /// on a non-2xx status code.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use futures_util::StreamExt;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let mut stream = client.subscribe_global_events().await?;
    ///
    /// while let Some(event) = stream.next().await {
    ///     let event = event?;
    ///     # let _ = event;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_global_events(&self) -> Result<EventStream, Error> {
        self.events(None, None).await
    }
}
