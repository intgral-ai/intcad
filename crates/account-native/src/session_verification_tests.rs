#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};

use solidcraft_engine::entitlement::CredentialErrorCode;

use crate::credentials::{BlockedProductExchange, CredentialSession, MemorySession};
use crate::http::{MAX_RESPONSE_BYTES, PROTOCOL_VERSION, ScriptedTransport, TransportFailure, WireConnection, WireCredentials, WireResponse};
use crate::session_verification::ServerVerifiedProductSession;

const BASE: &str = "https://auth.intcad.invalid";
const PROVIDER_TOKEN: &str = "workos_session_access_token_example_0001";
const ACCOUNT: &str = "acct_example_0001";

fn connection() -> WireConnection {
    WireConnection::new(BASE, "1.0.0").unwrap()
}

fn logged_in() -> CredentialSession {
    let session = CredentialSession::new(Box::new(MemorySession::default()));
    session.complete_provider_login("user_provider_0001", PROVIDER_TOKEN).unwrap();
    session
}

fn active_body() -> Value {
    json!({
        "account": {"id": ACCOUNT, "email_verified": true, "status": "active"},
        "client": {"version": "1.0.0", "supported": true, "min_supported_version": "1.0.0"},
        "protocol": {"version": PROTOCOL_VERSION, "supported": true, "min_supported_version": PROTOCOL_VERSION},
        "eligible": true,
        "reasons": [],
        "capabilities": {"chat": true, "harness": true, "review": true, "dfm": true, "mcp": true, "control": true, "autocomplete": false, "design_intent": false},
        "balance": 1000,
        "held": 0,
        "available": 1000,
        "snapshot_at": "2026-11-20T00:00:00.000Z",
        "policy": {}
    })
}

fn response(status: u16, body: Value) -> Result<WireResponse, TransportFailure> {
    Ok(WireResponse::new(status, body.to_string().into_bytes()))
}

fn exchange(responses: Vec<Result<WireResponse, TransportFailure>>) -> ServerVerifiedProductSession<ScriptedTransport> {
    ServerVerifiedProductSession::new(ScriptedTransport::new(responses), connection())
}

fn assert_not_activated(session: &CredentialSession) {
    assert!(!session.product_session_active(), "a failed verification must leave the product session inactive");
    assert!(WireCredentials::product_session(session).is_none());
}

#[test]
fn verified_status_activates_the_exact_documented_session_token() {
    let session = logged_in();
    assert!(!session.product_session_active(), "provider login alone is not a product session");

    let exchange = exchange(vec![response(200, active_body())]);
    let login = session.activate_product_session(&exchange).unwrap();

    assert_eq!(login.account_id, ACCOUNT, "the server account id is used, not the provider account");
    assert_ne!(login.account_id, "user_provider_0001");
    let product = WireCredentials::product_session(&session).unwrap();
    assert_eq!(product.expose_secret(), PROVIDER_TOKEN, "the reviewed server scheme uses the verified provider token as the bearer");
    assert!(session.product_session_active());
}

#[test]
fn verification_requests_the_configured_origin_with_contract_headers_and_no_body() {
    let exchange = exchange(vec![response(200, active_body())]);
    let session = logged_in();
    session.activate_product_session(&exchange).unwrap();

    let requests = exchange.transport().take_requests();
    assert_eq!(requests.len(), 1, "one explicit verification request");
    let request = &requests[0];
    assert_eq!(request.method(), "GET");
    assert_eq!(request.url(), "https://auth.intcad.invalid/v1/status");
    assert_eq!(request.header("Authorization"), Some(format!("Bearer {PROVIDER_TOKEN}").as_str()));
    assert_eq!(request.header("X-Client-Version"), Some("1.0.0"));
    assert_eq!(request.header("X-Protocol-Version"), Some(PROTOCOL_VERSION));
    assert_eq!(request.header("Accept"), Some("application/json"));
    assert!(request.body().is_none(), "status verification is a read, never a write");
}

#[test]
fn provider_login_alone_never_activates_a_product_session() {
    let session = logged_in();
    assert_eq!(session.provider_account().as_deref(), Some("user_provider_0001"));
    assert_not_activated(&session);
    assert_eq!(session.activate_product_session(&BlockedProductExchange).unwrap_err().code, CredentialErrorCode::Unavailable);
    assert_not_activated(&session);
}

#[test]
fn non_200_statuses_never_activate_a_product_session() {
    for status in [201, 301, 400, 401, 403, 429, 500, 503] {
        let session = logged_in();
        let result = session.activate_product_session(&exchange(vec![response(status, active_body())]));
        assert!(result.is_err(), "status {status} must not activate");
        assert_not_activated(&session);
    }
}

#[test]
fn inactive_or_unverified_accounts_never_activate_a_product_session() {
    let mut disabled = active_body();
    disabled["account"]["status"] = Value::String("disabled".into());
    let mut deleted = active_body();
    deleted["account"]["status"] = Value::String("deleted".into());
    let mut unverified = active_body();
    unverified["account"]["email_verified"] = Value::Bool(false);

    for body in [disabled, deleted, unverified] {
        let session = logged_in();
        assert!(
            session.activate_product_session(&exchange(vec![response(200, body)])).is_err(),
            "an inactive or unverified account must not activate"
        );
        assert_not_activated(&session);
    }
}

#[test]
fn unsupported_client_or_protocol_metadata_never_activates_a_product_session() {
    let mut wrong_protocol = active_body();
    wrong_protocol["protocol"]["version"] = Value::String("intcad-auth/2.0".into());
    let mut protocol_unsupported = active_body();
    protocol_unsupported["protocol"]["supported"] = Value::Bool(false);
    let mut client_unsupported = active_body();
    client_unsupported["client"]["supported"] = Value::Bool(false);
    let mut ineligible = active_body();
    ineligible["eligible"] = Value::Bool(false);
    let mut with_reason = active_body();
    with_reason["reasons"] = json!(["VERSION_UNSUPPORTED"]);

    for body in [wrong_protocol, protocol_unsupported, client_unsupported, ineligible, with_reason] {
        let session = logged_in();
        assert!(
            session.activate_product_session(&exchange(vec![response(200, body)])).is_err(),
            "unsupported metadata or an ineligible snapshot must not activate"
        );
        assert_not_activated(&session);
    }
}

#[test]
fn invalid_account_identifiers_and_inconsistent_balances_never_activate() {
    let mut no_prefix = active_body();
    no_prefix["account"]["id"] = Value::String("user_provider_0001".into());
    let mut control = active_body();
    control["account"]["id"] = Value::String("acct_bad\nid".into());
    let mut empty = active_body();
    empty["account"]["id"] = Value::String("acct_".into());
    let mut inconsistent = active_body();
    inconsistent["held"] = json!(1200);
    inconsistent["available"] = json!(1000);

    for body in [no_prefix, control, empty, inconsistent] {
        let session = logged_in();
        assert!(session.activate_product_session(&exchange(vec![response(200, body)])).is_err(), "invalid status metadata must not activate");
        assert_not_activated(&session);
    }
}

#[test]
fn malformed_oversized_and_transport_failures_never_activate() {
    let malformed = Ok(WireResponse::new(200, b"{".to_vec()));
    let oversized = Ok(WireResponse::new(200, vec![b' '; MAX_RESPONSE_BYTES + 1]));
    let timeout = Err(TransportFailure::Timeout);
    let redirected = Err(TransportFailure::Redirected);

    for response in [malformed, oversized, timeout, redirected] {
        let session = logged_in();
        assert!(session.activate_product_session(&exchange(vec![response])).is_err(), "a transport or response failure must not activate");
        assert_not_activated(&session);
    }
}
