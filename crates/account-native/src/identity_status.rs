use serde_json::Value;

use crate::credentials::ProductExchangeError;
use crate::http::{MAX_RESPONSE_BYTES, PROTOCOL_VERSION, Secret, Transport, TransportFailure, WireConnection, WireRequest};
use crate::identity::{REQUEST_TIMEOUT, SessionVerifier, VerifiedSession, VerifyError};
use crate::session_verification::verified_account;

/// Verifies the product account for an access token using the configured status endpoint.
///
/// The status snapshot cannot prove that two tokens belong to the same login.
/// It supplies the product account only, even when the lifecycle's refresh policy
/// retains a previously established local binding.
pub struct StatusSessionVerifier<T: Transport> {
    transport: T,
    connection: WireConnection,
}

impl<T: Transport> StatusSessionVerifier<T> {
    pub fn new(transport: T, connection: WireConnection) -> Self {
        Self { transport, connection }
    }
}

impl<T: Transport + Send + Sync> SessionVerifier for StatusSessionVerifier<T> {
    fn verify(&self, access: &Secret) -> Result<VerifiedSession, VerifyError> {
        let request = WireRequest::new(
            "GET",
            self.connection.url("/v1/status"),
            vec![
                ("Accept".to_owned(), "application/json".to_owned()),
                ("X-Client-Version".to_owned(), self.connection.client_version().to_owned()),
                ("X-Protocol-Version".to_owned(), PROTOCOL_VERSION.to_owned()),
                ("Authorization".to_owned(), format!("Bearer {}", access.expose_secret())),
            ],
            None,
        )
        .with_timeout(REQUEST_TIMEOUT);
        let response = self.transport.send(&request).map_err(|error| match error {
            TransportFailure::BodyTooLarge => VerifyError::Protocol,
            _ => VerifyError::Transient,
        })?;
        if response.body().len() > MAX_RESPONSE_BYTES {
            return Err(VerifyError::Protocol);
        }
        match response.status() {
            408 | 429 | 500..=599 => Err(response.retry_after().map_or(VerifyError::Transient, VerifyError::RetryAfter)),
            200 => {
                let value = serde_json::from_slice(response.body()).map_err(|_| VerifyError::Protocol)?;
                if !has_status_shape(&value) {
                    return Err(VerifyError::Protocol);
                }
                let account_id = verified_account(&value).map_err(|error| match error {
                    ProductExchangeError::Rejected => VerifyError::Rejected,
                    ProductExchangeError::Unavailable | ProductExchangeError::NotConfigured => VerifyError::Protocol,
                })?;
                Ok(VerifiedSession { account_id, same_login_binding: None })
            }
            _ => Err(VerifyError::Rejected),
        }
    }
}

// The frozen business decoder also rejects some missing or mistyped fields.
// Check their shape first so P1 can distinguish malformed snapshots from denials.
fn has_status_shape(value: &Value) -> bool {
    ["/account/id", "/account/status", "/protocol/version"].into_iter().all(|path| value.pointer(path).is_some_and(Value::is_string))
        && ["/account/email_verified", "/protocol/supported", "/client/supported", "/eligible"]
            .into_iter()
            .all(|path| value.pointer(path).is_some_and(Value::is_boolean))
        && value.get("reasons").is_some_and(Value::is_array)
        && ["balance", "held", "available"].into_iter().all(|field| value.get(field).is_some_and(Value::is_u64))
}
