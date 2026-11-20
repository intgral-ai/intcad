#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use solidcraft_engine::entitlement::{CredentialError, CredentialErrorCode, RefusalCode};

use crate::credentials::{ACCOUNT_KEYRING_SERVICE, CredentialSession, MemorySession};
use crate::device_code::{
    CompletedDeviceLogin, DeviceCodeError, DeviceCodeFlow, DeviceCodeMode, DeviceCodePoll, DeviceCodeTransport, WORKOS_DEVICE_AUTHORIZATION_URL,
    WORKOS_TOKEN_URL, WorkOsPublicClient,
};
use crate::http::WireCredentials;

fn device_body() -> Value {
    json!({
        "device_code": "device_code_SECRET_000000000000000000000000",
        "user_code": "BCDF-GHJK",
        "verification_uri": "https://auth.example/device",
        "verification_uri_complete": "https://auth.example/device?user_code=BCDF-GHJK",
        "expires_in": 300,
        "interval": 5
    })
}

fn token_body() -> Value {
    json!({
        "user": {"object": "user", "id": "user_01E4ZCR3C56J083X43JQXF3JK5", "email_verified": true},
        "organization_id": "org_01H945H0YD4F97JN9MATX7BYAG",
        "access_token": "access_token_SECRET_000000000000000000000000",
        "refresh_token": "refresh_token_SECRET_000000000000000000000000",
        "authentication_method": "Password"
    })
}
struct Request {
    url: String,
    content_type: String,
    body: Vec<u8>,
}

#[derive(Default)]
struct TransportState {
    responses: VecDeque<Result<(u16, Vec<u8>), CredentialError>>,
    requests: Vec<Request>,
}

#[derive(Clone, Default)]
struct ScriptedDeviceTransport {
    inner: Arc<Mutex<TransportState>>,
}

impl ScriptedDeviceTransport {
    fn scripted(responses: Vec<(u16, Value)>) -> Self {
        let transport = Self::default();
        {
            let mut state = transport.inner.lock().unwrap();
            for (status, body) in responses {
                state.responses.push_back(Ok((status, body.to_string().into_bytes())));
            }
        }
        transport
    }

    fn requests(&self) -> Vec<(String, String, Vec<u8>)> {
        self.inner.lock().unwrap().requests.iter().map(|request| (request.url.clone(), request.content_type.clone(), request.body.clone())).collect()
    }

    fn request_count(&self) -> usize {
        self.inner.lock().unwrap().requests.len()
    }
}

impl DeviceCodeTransport for ScriptedDeviceTransport {
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<(u16, Vec<u8>), CredentialError> {
        let mut state = self.inner.lock().map_err(|_| unavailable())?;
        state.requests.push(Request { url: url.to_owned(), content_type: content_type.to_owned(), body: body.to_vec() });
        state.responses.pop_front().unwrap_or_else(|| Err(unavailable()))
    }
}

fn unavailable() -> CredentialError {
    CredentialError { code: CredentialErrorCode::Unavailable, message: "unavailable".into() }
}

fn session_with_memory() -> (CredentialSession, MemorySession) {
    let memory = MemorySession::default();
    (CredentialSession::new(Box::new(memory.clone())), memory)
}
#[test]
fn account_keyring_service_is_independent_and_workos_urls_are_https() {
    assert_ne!(ACCOUNT_KEYRING_SERVICE, "SolidCraft agent chat");
    assert!(WORKOS_DEVICE_AUTHORIZATION_URL.starts_with("https://"));
    assert!(WORKOS_TOKEN_URL.starts_with("https://"));
}

#[test]
fn device_code_prints_url_and_code_but_noninteractive_call_returns_auth_required() {
    let transport = ScriptedDeviceTransport::scripted(vec![(200, device_body())]);
    let (session, _) = session_with_memory();
    let mut flow =
        DeviceCodeFlow::new(WorkOsPublicClient::new("client_test").unwrap(), transport.clone(), session, DeviceCodeMode::Programmatic);
    assert_eq!(flow.start().unwrap_err().code, RefusalCode::AuthRequired);
    assert_eq!(transport.request_count(), 0);

    let transport = ScriptedDeviceTransport::scripted(vec![(200, device_body())]);
    let (session, _) = session_with_memory();
    let mut flow = DeviceCodeFlow::new(WorkOsPublicClient::new("client_test").unwrap(), transport, session, DeviceCodeMode::Explicit);
    let prompt = flow.start().unwrap();
    let shown = prompt.to_string();
    assert!(shown.contains("https://auth.example/device"));
    assert!(shown.contains("BCDF-GHJK"));
    assert!(!shown.contains("device_code_SECRET"));
    assert!(!format!("{prompt:?}").contains("device_code_SECRET"));
}

#[test]
fn device_code_polls_the_documented_workos_flow_and_completes_into_memory() {
    let transport = ScriptedDeviceTransport::scripted(vec![
        (200, device_body()),
        (400, json!({"error": "authorization_pending"})),
        (400, json!({"error": "slow_down"})),
        (200, token_body()),
    ]);
    let (session, _) = session_with_memory();
    let mut flow = DeviceCodeFlow::new(WorkOsPublicClient::new("client_test").unwrap(), transport.clone(), session.clone(), DeviceCodeMode::Explicit);

    let prompt = flow.start().unwrap();
    assert_eq!(prompt.interval(), Duration::from_secs(5));
    assert_eq!(flow.poll().unwrap(), DeviceCodePoll::Pending { interval: Duration::from_secs(5) });
    assert_eq!(flow.poll().unwrap(), DeviceCodePoll::SlowDown { interval: Duration::from_secs(10) });
    assert_eq!(flow.poll().unwrap(), DeviceCodePoll::Authorized);

    let completed: CompletedDeviceLogin = flow.complete().unwrap();
    assert_eq!(completed.account_id, "user_01E4ZCR3C56J083X43JQXF3JK5");
    let product = WireCredentials::product_session(&session).unwrap();
    assert_eq!(product.expose_secret(), "access_token_SECRET_000000000000000000000000");
    assert!(!format!("{session:?}").contains("access_token_SECRET"));
    assert!(!format!("{session}").contains("refresh_token_SECRET"));

    let requests = transport.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].0, WORKOS_DEVICE_AUTHORIZATION_URL);
    assert_eq!(requests[0].1, "application/json");
    let device_request: Value = serde_json::from_slice(&requests[0].2).unwrap();
    assert_eq!(device_request, json!({"client_id": "client_test"}));
    assert_eq!(requests[1].0, WORKOS_TOKEN_URL);
    assert_eq!(requests[1].1, "application/x-www-form-urlencoded");
    let form = String::from_utf8(requests[1].2.clone()).unwrap();
    assert!(form.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"));
    assert!(form.contains("device_code=device_code_SECRET_000000000000000000000000"));
    assert!(form.contains("client_id=client_test"));
}

#[test]
fn device_code_denied_expired_and_bad_responses_fail_closed() {
    for (error, expected) in [
        ("access_denied", DeviceCodePoll::Denied),
        ("expired_token", DeviceCodePoll::Expired),
        ("invalid_grant", DeviceCodePoll::Failed(DeviceCodeError::InvalidGrant)),
        ("invalid_client", DeviceCodePoll::Failed(DeviceCodeError::InvalidClient)),
        ("invalid_request", DeviceCodePoll::Failed(DeviceCodeError::InvalidRequest)),
        ("unsupported_grant_type", DeviceCodePoll::Failed(DeviceCodeError::UnsupportedGrantType)),
    ] {
        let transport = ScriptedDeviceTransport::scripted(vec![(200, device_body()), (400, json!({"error": error}))]);
        let (session, _) = session_with_memory();
        let mut flow = DeviceCodeFlow::new(WorkOsPublicClient::new("client_test").unwrap(), transport, session, DeviceCodeMode::Explicit);
        flow.start().unwrap();
        assert_eq!(flow.poll().unwrap(), expected, "{error}");
    }

    let transport = ScriptedDeviceTransport::scripted(vec![
        (200, json!({"device_code": "d", "user_code": "U", "verification_uri": "http://insecure.example/device", "expires_in": 300, "interval": 5})),
    ]);
    let (session, _) = session_with_memory();
    let mut flow = DeviceCodeFlow::new(WorkOsPublicClient::new("client_test").unwrap(), transport, session, DeviceCodeMode::Explicit);
    assert_eq!(flow.start().unwrap_err().code, RefusalCode::AuthUnavailable);

    let transport = ScriptedDeviceTransport::scripted(vec![(500, json!({"secret": "must_not_echo"}))]);
    let (session, _) = session_with_memory();
    let mut flow = DeviceCodeFlow::new(WorkOsPublicClient::new("client_test").unwrap(), transport, session, DeviceCodeMode::Explicit);
    let error = flow.start().unwrap_err();
    assert_eq!(error.code, RefusalCode::AuthUnavailable);
    assert!(!error.message.contains("must_not_echo"));
}
