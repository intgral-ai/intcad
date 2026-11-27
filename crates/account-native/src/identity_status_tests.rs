#![allow(clippy::unwrap_used)]

use std::sync::Mutex;

use serde_json::{Value, json};

use crate::http::{
    MAX_RESPONSE_BYTES, PROTOCOL_VERSION, ScriptedTransport, Secret, Transport, TransportFailure, WireConnection, WireRequest, WireResponse,
};
use crate::identity::{SessionVerifier, VerifiedSession, VerifyError};
use crate::identity_status::StatusSessionVerifier;

const ACCOUNT: &str = "acct_example_0001";
const ACCESS: &str = "workos_session_access_token_example_0001";

// The shared test script uses interior mutability; serialize it for the Sync verifier.
struct SyncScript<'a>(&'a Mutex<ScriptedTransport>);

impl Transport for SyncScript<'_> {
    fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
        self.0.lock().unwrap().send(request)
    }
}

fn connection() -> WireConnection {
    WireConnection::new("https://auth.intcad.invalid", "1.0.0").unwrap()
}

fn active_body() -> Value {
    json!({
        "account": {"id": ACCOUNT, "email_verified": true, "status": "active"},
        "client": {"supported": true},
        "protocol": {"version": PROTOCOL_VERSION, "supported": true},
        "eligible": true,
        "reasons": [],
        "balance": 1000,
        "held": 0,
        "available": 1000
    })
}

fn verify(response: Result<WireResponse, TransportFailure>) -> Result<VerifiedSession, VerifyError> {
    let script = Mutex::new(ScriptedTransport::new(vec![response]));
    StatusSessionVerifier::new(SyncScript(&script), connection()).verify(&Secret::new(ACCESS))
}

#[test]
fn an_active_status_snapshot_verifies_the_account_without_a_session_binding() {
    let response = WireResponse::new(200, active_body().to_string().into_bytes());
    let script = Mutex::new(ScriptedTransport::new(vec![Ok(response)]));
    let verifier = StatusSessionVerifier::new(SyncScript(&script), connection());
    let verified = verifier.verify(&Secret::new(ACCESS)).unwrap();
    assert_eq!(verified.account_id, ACCOUNT);
    assert_eq!(verified.same_login_binding, None);

    let requests = script.lock().unwrap().take_requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method(), "GET");
    assert_eq!(request.url(), "https://auth.intcad.invalid/v1/status");
    assert!(request.body().is_none());
    let mut names: Vec<_> = request.header_names().collect();
    names.sort_unstable();
    assert_eq!(names, ["Accept", "Authorization", "X-Client-Version", "X-Protocol-Version"]);
    assert_eq!(request.header("Accept"), Some("application/json"));
    assert_eq!(request.header("X-Client-Version"), Some("1.0.0"));
    assert_eq!(request.header("X-Protocol-Version"), Some(PROTOCOL_VERSION));
    assert_eq!(request.header("Authorization"), Some(format!("Bearer {ACCESS}").as_str()));
    assert!(!format!("{request:?}").contains(ACCESS));
}

#[test]
fn a_status_rejection_is_not_a_terminal_login_failure() {
    for status in [201, 401, 403] {
        assert_eq!(verify(Ok(WireResponse::new(status, b"{".to_vec()))).err(), Some(VerifyError::Rejected), "status {status}");
    }
    let mut body = active_body();
    body["eligible"] = json!(false);
    assert_eq!(verify(Ok(WireResponse::new(200, body.to_string().into_bytes()))).err(), Some(VerifyError::Rejected));
}

#[test]
fn a_status_transport_failure_is_transient() {
    assert_eq!(verify(Err(TransportFailure::Timeout)).err(), Some(VerifyError::Transient));
}

#[test]
fn a_native_body_too_large_is_protocol_and_other_transport_errors_are_transient() {
    assert_eq!(verify(Err(TransportFailure::BodyTooLarge)).err(), Some(VerifyError::Protocol));
    assert_eq!(verify(Err(TransportFailure::Timeout)).err(), Some(VerifyError::Transient));
}

#[test]
fn a_malformed_status_snapshot_is_protocol() {
    assert_eq!(verify(Ok(WireResponse::new(200, b"{".to_vec()))).err(), Some(VerifyError::Protocol));
    for field in ["account", "balance"] {
        let mut body = active_body();
        body.as_object_mut().unwrap().remove(field);
        assert_eq!(verify(Ok(WireResponse::new(200, body.to_string().into_bytes()))).err(), Some(VerifyError::Protocol), "missing {field}");
    }
}

#[test]
fn malformed_business_fields_are_protocol_failures() {
    let mut missing_email_verified = active_body();
    missing_email_verified["account"].as_object_mut().unwrap().remove("email_verified");
    let mut mistyped_eligible = active_body();
    mistyped_eligible["eligible"] = json!("true");
    for (field, body) in [("missing email_verified", missing_email_verified), ("mistyped eligible", mistyped_eligible)] {
        assert_eq!(verify(Ok(WireResponse::new(200, body.to_string().into_bytes()))).err(), Some(VerifyError::Protocol), "{field}");
    }
}

#[test]
fn a_well_formed_inactive_or_unverified_account_is_rejected() {
    let mut inactive = active_body();
    inactive["account"]["status"] = json!("disabled");
    let mut unverified = active_body();
    unverified["account"]["email_verified"] = json!(false);
    for body in [inactive, unverified] {
        assert_eq!(verify(Ok(WireResponse::new(200, body.to_string().into_bytes()))).err(), Some(VerifyError::Rejected));
    }
}

#[test]
fn an_oversized_status_response_is_protocol() {
    let mut body = active_body().to_string().into_bytes();
    body.resize(MAX_RESPONSE_BYTES + 1, b' ');
    for status in [200, 500] {
        assert_eq!(verify(Ok(WireResponse::new(status, body.clone()))).err(), Some(VerifyError::Protocol), "status {status}");
    }
}

#[test]
fn a_status_500_is_transient() {
    for status in [408, 429, 500, 599] {
        assert_eq!(verify(Ok(WireResponse::new(status, b"{".to_vec()))).err(), Some(VerifyError::Transient), "status {status}");
    }
}
