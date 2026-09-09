//! Tempo instance status and configuration operations.
//!
//! These operations query the running Tempo instance's status,
//! build information, metrics, version, services, and configuration.

mod discovery;
mod health;

pub use discovery::{GetConfig, GetEndpoints, GetServices};
pub use health::{GetBuildInfo, GetMetrics, GetReady, GetStatus, GetVersion};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
    use reqwest::Client;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::TempoClient;

    #[test]
    fn all_status_ops_return_kind_tempo() {
        let tempo = TempoClient::new("http://tempo:3200", Client::new());

        assert_eq!(GetReady::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetMetrics::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetBuildInfo::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetStatus::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetVersion::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetServices::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetEndpoints::new(tempo.clone()).kind(), "tempo");
        assert_eq!(GetConfig::new(tempo).kind(), "tempo");
    }

    #[tokio::test]
    async fn get_config_wraps_text_in_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/status/config"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("server:\n  http_listen_port: 3200"),
            )
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let result = GetConfig::new(tempo).execute(&ctx).await.unwrap();
        assert_eq!(result["config"], "server:\n  http_listen_port: 3200");
    }

    #[tokio::test]
    async fn get_ready_returns_ready_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ready"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ready"))
            .mount(&server)
            .await;

        let tempo = TempoClient::new(&server.uri(), Client::new());
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let result = GetReady::new(tempo).execute(&ctx).await.unwrap();
        assert_eq!(result["ready"], true);
        assert_eq!(result["status"], 200);
    }
}
