//! Schedule-related methods for [`IronflowClient`].

use uuid::Uuid;

use ironflow_types::ApiResponse;

use crate::client::IronflowClient;
use crate::error::Error;
use crate::types;

impl IronflowClient {
    /// List all schedules.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let schedules = client.list_schedules().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_schedules(&self) -> Result<ApiResponse<Vec<types::ScheduleResponse>>, Error> {
        self.send_envelope(self.get("/api/v1/schedules")).await
    }

    /// Create a new schedule.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use ironflow_sdk::types::CreateScheduleRequest;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let schedule = client.create_schedule(&CreateScheduleRequest {
    ///     workflow_name: "deploy".to_string(),
    ///     cron_expression: "0 * * * *".to_string(),
    ///     inputs: Some(serde_json::json!({})),
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_schedule(
        &self,
        request: &types::CreateScheduleRequest,
    ) -> Result<ApiResponse<types::ScheduleResponse>, Error> {
        self.send_envelope(self.post("/api/v1/schedules").json(request))
            .await
    }

    /// Get a schedule by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let schedule = client.get_schedule(Uuid::nil()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_schedule(
        &self,
        id: Uuid,
    ) -> Result<ApiResponse<types::ScheduleResponse>, Error> {
        self.send_envelope(self.get(&format!("/api/v1/schedules/{id}")))
            .await
    }

    /// Delete a schedule by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// client.delete_schedule(Uuid::nil()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_schedule(&self, id: Uuid) -> Result<(), Error> {
        self.send_no_content(self.delete(&format!("/api/v1/schedules/{id}")))
            .await
    }

    /// Pause a schedule.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let schedule = client.pause_schedule(Uuid::nil()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn pause_schedule(
        &self,
        id: Uuid,
    ) -> Result<ApiResponse<types::ScheduleResponse>, Error> {
        self.send_envelope(self.post(&format!("/api/v1/schedules/{id}/pause")))
            .await
    }

    /// Resume a schedule.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let schedule = client.resume_schedule(Uuid::nil()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume_schedule(
        &self,
        id: Uuid,
    ) -> Result<ApiResponse<types::ScheduleResponse>, Error> {
        self.send_envelope(self.post(&format!("/api/v1/schedules/{id}/resume")))
            .await
    }

    /// Trigger a schedule manually, creating a run.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_sdk::IronflowClient;
    /// use uuid::Uuid;
    ///
    /// # async fn example() -> Result<(), ironflow_sdk::Error> {
    /// let client = IronflowClient::new("https://ironflow.example.com", "key");
    /// let schedule = client.trigger_schedule(Uuid::nil()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn trigger_schedule(
        &self,
        id: Uuid,
    ) -> Result<ApiResponse<types::ScheduleResponse>, Error> {
        self.send_envelope(self.post(&format!("/api/v1/schedules/{id}/trigger")))
            .await
    }
}
