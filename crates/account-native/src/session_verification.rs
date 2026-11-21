//! Server-verified product session activation for the documented WorkOS scheme.
//!
//! The account server accepts a WorkOS session access token as the product credential only after
//! it verifies the token's RS256 signature against WorkOS's per-client JWKS, validates the signed
//! claims and reads the WorkOS user record with its server-side API key. The client does not
//! reimplement any of that. It sends the provider token to the configured product service origin
//! as a bearer and activates a product session only when `GET /v1/status` returns a contract-valid,
//! active snapshot for that verified identity.
//!
//! This is not implicit token promotion:
//! - a completed provider login alone never activates or persists a product session;
//! - only an explicit [`ProductSessionExchange`] call runs this verification;
//! - a non-200 response, a disabled/deleted account, an unverified email, unsupported client or
//!   protocol metadata, an invalid account identifier or a malformed response leaves the session
//!   inactive;
//! - transport, size and JSON failures fail closed as unavailable.
//!
//! The exchange deliberately does not parse the token, infer an expiry, choose a service origin or
//! mint any other credential. The product session is the exact provider token because that is the
//! reviewed server scheme; it is written only after the server's status response confirms it.

use serde_json::Value;

use crate::credentials::{ProductExchangeError, ProductSession, ProductSessionExchange, ProviderSession};
use crate::http::{MAX_RESPONSE_BYTES, PROTOCOL_VERSION, Secret, Transport, WireConnection, WireRequest};

/// A [`ProductSessionExchange`] that asks the configured product service to verify the WorkOS
/// session access token before activation.
///
/// The caller supplies an already validated HTTPS [`WireConnection`]; this type never derives an
/// origin from a token, a response or an environment heuristic. Test callers inject a scripted
/// [`Transport`]; production callers use the existing native transport.
pub struct ServerVerifiedProductSession<T: Transport> {
    transport: T,
    connection: WireConnection,
}

impl<T: Transport> ServerVerifiedProductSession<T> {
    pub fn new(transport: T, connection: WireConnection) -> Self {
        Self { transport, connection }
    }

    #[cfg(test)]
    pub(crate) fn transport(&self) -> &T {
        &self.transport
    }
}

impl<T: Transport> ProductSessionExchange for ServerVerifiedProductSession<T> {
    fn exchange(&self, provider: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
        let token = provider.access_token();
        if token.is_empty() || token.expose_secret().bytes().any(|byte| byte == b'\r' || byte == b'\n' || byte == 0) {
            return Err(ProductExchangeError::Rejected);
        }

        let request = WireRequest::new(
            "GET",
            self.connection.url("/v1/status"),
            vec![
                ("Accept".to_owned(), "application/json".to_owned()),
                ("X-Client-Version".to_owned(), self.connection.client_version().to_owned()),
                ("X-Protocol-Version".to_owned(), PROTOCOL_VERSION.to_owned()),
                ("Authorization".to_owned(), format!("Bearer {}", token.expose_secret())),
            ],
            None,
        );

        let response = self.transport.send(&request).map_err(|_| ProductExchangeError::Unavailable)?;
        if response.body().len() > MAX_RESPONSE_BYTES {
            return Err(ProductExchangeError::Unavailable);
        }
        let value: Value = serde_json::from_slice(response.body()).map_err(|_| ProductExchangeError::Unavailable)?;
        if response.status() != 200 {
            return Err(ProductExchangeError::Rejected);
        }

        verified_account(&value).map(|account| ProductSession::from_trusted_exchange(account, Secret::new(token.expose_secret().to_owned()), None))
    }
}

/// Accept only the frozen status shape that proves an active, verified account for this client and
/// protocol. Every missing, inconsistent or unsupported value is a refusal.
fn verified_account(value: &Value) -> Result<String, ProductExchangeError> {
    let account =
        value.pointer("/account/id").and_then(Value::as_str).filter(|account| valid_account(account)).ok_or(ProductExchangeError::Unavailable)?;
    if value.pointer("/account/status").and_then(Value::as_str) != Some("active")
        || !value.pointer("/account/email_verified").and_then(Value::as_bool).unwrap_or(false)
    {
        return Err(ProductExchangeError::Rejected);
    }
    if value.pointer("/protocol/version").and_then(Value::as_str) != Some(PROTOCOL_VERSION)
        || !value.pointer("/protocol/supported").and_then(Value::as_bool).unwrap_or(false)
        || !value.pointer("/client/supported").and_then(Value::as_bool).unwrap_or(false)
    {
        return Err(ProductExchangeError::Rejected);
    }
    if !value.get("eligible").and_then(Value::as_bool).unwrap_or(false) || !value.get("reasons").and_then(Value::as_array).is_some_and(Vec::is_empty)
    {
        return Err(ProductExchangeError::Rejected);
    }
    let balance = value.get("balance").and_then(Value::as_u64).ok_or(ProductExchangeError::Unavailable)?;
    let held = value.get("held").and_then(Value::as_u64).ok_or(ProductExchangeError::Unavailable)?;
    let available = value.get("available").and_then(Value::as_u64).ok_or(ProductExchangeError::Unavailable)?;
    if held > balance || available != balance.saturating_sub(held) {
        return Err(ProductExchangeError::Unavailable);
    }
    Ok(account.to_owned())
}

/// The server account id is the contract's `acct_` label. This is shape validation only; it makes
/// no claim about the WorkOS identity mapping, which only the server verified.
fn valid_account(account: &str) -> bool {
    account.len() > "acct_".len()
        && account.len() <= 128
        && account
            .strip_prefix("acct_")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'))
}
