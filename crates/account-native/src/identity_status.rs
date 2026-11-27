use crate::credentials::ProductExchangeError;
use crate::http::{MAX_RESPONSE_BYTES, PROTOCOL_VERSION, Secret, Transport, WireConnection, WireRequest};
use crate::identity::{SessionVerifier, VerifiedSession, VerifyError};
use crate::session_verification::verified_account;

/// Verifies the product account for an access token using the configured status endpoint.
///
/// The status snapshot cannot prove that two tokens belong to the same login.
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
        );
        let response = self.transport.send(&request).map_err(|_| VerifyError::Transient)?;
        if response.body().len() > MAX_RESPONSE_BYTES {
            return Err(VerifyError::Protocol);
        }
        match response.status() {
            408 | 429 | 500..=599 => Err(VerifyError::Transient),
            200 => {
                let value = serde_json::from_slice(response.body()).map_err(|_| VerifyError::Protocol)?;
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
