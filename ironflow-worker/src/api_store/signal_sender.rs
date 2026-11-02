//! [`SignalSender`] for [`ApiRunStore`]: on a worker, `ctx.send_signal`
//! delivers through `POST /api/v1/internal/signals`, where the API's engine
//! stores the signal, wakes the waiting runs and writes the audit log.

use reqwest::StatusCode;
use serde::Serialize;

use ironflow_engine::error::EngineError;
use ironflow_engine::signal::{SignalDelivery, SignalOrigin, SignalSendFuture, SignalSender};
use ironflow_store::entities::NewSignal;

use super::{ApiResponse, ApiRunStore};

/// Body of `POST /internal/signals`.
#[derive(Serialize)]
struct SendSignalBody<'a> {
    #[serde(flatten)]
    signal: &'a NewSignal,
    origin: SignalOrigin,
}

impl SignalSender for ApiRunStore {
    fn send(&self, signal: NewSignal, origin: SignalOrigin) -> SignalSendFuture<'_> {
        Box::pin(async move {
            let body = SendSignalBody {
                signal: &signal,
                origin,
            };
            let resp = self
                .client
                .post(self.internal("/signals"))
                .bearer_auth(&self.token)
                .json(&body)
                .send()
                .await
                .map_err(|e| EngineError::Store(Self::err(e)))?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp
                    .text()
                    .await
                    .map_err(|e| EngineError::Store(Self::err(e)))?;
                return Err(if status == StatusCode::BAD_REQUEST {
                    EngineError::InvalidSignal(body)
                } else {
                    EngineError::Store(Self::status_err(&body))
                });
            }

            let api_resp: ApiResponse<SignalDelivery> = resp
                .json()
                .await
                .map_err(|e| EngineError::Store(Self::err(e)))?;
            Ok(api_resp.data)
        })
    }
}
