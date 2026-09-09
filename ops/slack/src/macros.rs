//! Internal macros for generating Slack operation structs.

/// Serialize a value to [`serde_json::Value`], mapping the error to [`OperationError`](ironflow_core::error::OperationError).
pub(crate) fn to_value<T: serde::Serialize>(
    val: &T,
) -> Result<serde_json::Value, ironflow_core::error::OperationError> {
    serde_json::to_value(val).map_err(|e| ironflow_core::error::OperationError::External {
        origin: "slack".to_string(),
        message: format!("failed to serialize response: {e}"),
    })
}

/// Generate a Slack operation struct that wraps a single
/// [`slack_morphism`] API method as an Ironflow [`Operation`].
///
/// # With request parameter
///
/// ```ignore
/// slack_op! {
///     /// Post a message to a channel.
///     ChatPostMessage => chat_post_message(
///         SlackApiChatPostMessageRequest
///     ) -> SlackApiChatPostMessageResponse
/// }
/// ```
///
/// # Without request parameter (parameterless API call)
///
/// ```ignore
/// slack_op! {
///     /// Test authentication.
///     AuthTest => auth_test() -> SlackApiAuthTestResponse
/// }
/// ```
macro_rules! slack_op {
    // With request parameter
    (
        $(#[$meta:meta])*
        $name:ident => $method:ident( $req:ty ) -> $resp:ty
    ) => {
        $(#[$meta])*
        pub struct $name {
            client: $crate::SlackClient,
            request: $req,
        }

        impl $name {
            /// Create a new operation.
            pub fn new(client: &$crate::SlackClient, request: $req) -> Self {
                Self {
                    client: client.clone(),
                    request,
                }
            }

            /// Execute and return a typed result.
            ///
            /// # Errors
            ///
            /// Returns [`OperationError::External`](ironflow_core::error::OperationError::External)
            /// on Slack API failure.
            #[allow(deprecated)]
            pub async fn run(
                &self,
            ) -> Result<$resp, ironflow_core::error::OperationError> {
                let session = self.client.session();
                session
                    .$method(&self.request)
                    .await
                    .map_err($crate::error::from_slack_error)
            }
        }

        #[async_trait::async_trait]
        impl ironflow_core::operation::Operation for $name {
            fn kind(&self) -> &str {
                "slack"
            }

            async fn execute(
                &self,
                _ctx: &ironflow_core::operation::OperationContext,
            ) -> Result<serde_json::Value, ironflow_core::error::OperationError> {
                let result = self.run().await?;
                $crate::macros::to_value(&result)
            }

            fn input(&self) -> Option<serde_json::Value> {
                serde_json::to_value(&self.request).ok()
            }
        }

        impl ironflow_core::operation::TypedOperation for $name {
            type Output = $resp;
        }
    };

    // Without request parameter
    (
        $(#[$meta:meta])*
        $name:ident => $method:ident() -> $resp:ty
    ) => {
        $(#[$meta])*
        pub struct $name {
            client: $crate::SlackClient,
        }

        impl $name {
            /// Create a new operation.
            pub fn new(client: &$crate::SlackClient) -> Self {
                Self {
                    client: client.clone(),
                }
            }

            /// Execute and return a typed result.
            ///
            /// # Errors
            ///
            /// Returns [`OperationError::External`](ironflow_core::error::OperationError::External)
            /// on Slack API failure.
            pub async fn run(
                &self,
            ) -> Result<$resp, ironflow_core::error::OperationError> {
                let session = self.client.session();
                session
                    .$method()
                    .await
                    .map_err($crate::error::from_slack_error)
            }
        }

        #[async_trait::async_trait]
        impl ironflow_core::operation::Operation for $name {
            fn kind(&self) -> &str {
                "slack"
            }

            async fn execute(
                &self,
                _ctx: &ironflow_core::operation::OperationContext,
            ) -> Result<serde_json::Value, ironflow_core::error::OperationError> {
                let result = self.run().await?;
                $crate::macros::to_value(&result)
            }

            fn input(&self) -> Option<serde_json::Value> {
                None
            }
        }

        impl ironflow_core::operation::TypedOperation for $name {
            type Output = $resp;
        }
    };
}

pub(crate) use slack_op;

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;

    #[test]
    fn to_value_serializes_struct() {
        #[derive(Serialize)]
        struct Msg {
            text: String,
        }
        let msg = Msg {
            text: "hello".to_string(),
        };
        let val = to_value(&msg).unwrap();
        assert_eq!(val["text"], "hello");
    }

    #[test]
    fn to_value_serializes_unit() {
        let val = to_value(&()).unwrap();
        assert!(val.is_null(), "unit type should serialize to null");
    }
}
