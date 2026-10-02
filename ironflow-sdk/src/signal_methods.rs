//! Signal methods for [`IronflowClient`].

use ironflow_types::ApiResponse;

use crate::client::{IronflowClient, ListSignalsFilter};
use crate::error::Error;
use crate::types;

impl IronflowClient {
    /// Send a signal, resuming every run waiting for its `(name, key)`.
    ///
    /// The caller must be an admin, or use an API key with the `signals_send`
    /// scope. Sending again with the same `idempotency_id` delivers nothing
    /// and returns `duplicate: true`.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the signal is invalid
    /// (empty name or key), the caller lacks the permission, or the response
    /// cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::types::SendSignalRequest;
    /// use serde_json::json;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let request: SendSignalRequest = serde_json::from_value(json!({
    ///     "name": "ci.pipeline_finished",
    ///     "key": "4f2a9c1",
    ///     "payload": {"status": "success"},
    ///     "idempotency_id": "delivery-42"
    /// }))?;
    /// let delivery = client.send_signal(&request).await?;
    /// println!("{} runs resumed", delivery.data.resumed.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_signal(
        &self,
        request: &types::SendSignalRequest,
    ) -> Result<ApiResponse<types::SignalDeliveryResponse>, Error> {
        self.send_envelope(self.post("/api/v1/signals").json(request))
            .await
    }

    /// List received signals, newest first, with filters and pagination.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::client::ListSignalsFilter;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let filter = ListSignalsFilter {
    ///     name: Some("ci.pipeline_finished".to_string()),
    ///     ..Default::default()
    /// };
    /// let signals = client.list_signals(&filter).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_signals(
        &self,
        filter: &ListSignalsFilter,
    ) -> Result<ApiResponse<Vec<types::SignalResponse>>, Error> {
        self.send_envelope(self.get("/api/v1/signals").query(filter))
            .await
    }
}
