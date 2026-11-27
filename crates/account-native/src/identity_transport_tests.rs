#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::http::Secret;
use crate::identity_transport::{
    IdentityTransport, RefreshHttp, RefreshHttpReply, RefreshTransportError, UreqIdentityTransport, UreqRefreshHttp, parse_retry_after,
};

const TOKEN_URL: &str = "https://api.workos.com/user_management/authenticate";

#[derive(Clone, Debug, PartialEq, Eq)]
struct PostCall {
    url: String,
    content_type: String,
    body: String,
}

#[derive(Default)]
struct ScriptState {
    calls: Vec<PostCall>,
    replies: Vec<Result<RefreshHttpReply, ()>>,
}

/// The scripted `RefreshHttp` seam: replies are queued, no network is reachable.
#[derive(Clone, Default)]
struct ScriptedRefreshHttp {
    state: Arc<Mutex<ScriptState>>,
}

impl ScriptedRefreshHttp {
    fn new(replies: Vec<Result<RefreshHttpReply, ()>>) -> Self {
        Self { state: Arc::new(Mutex::new(ScriptState { calls: Vec::new(), replies })) }
    }

    fn single(reply: RefreshHttpReply) -> Self {
        Self::new(vec![Ok(reply)])
    }

    fn calls(&self) -> Vec<PostCall> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl RefreshHttp for ScriptedRefreshHttp {
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<RefreshHttpReply, ()> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(PostCall { url: url.to_owned(), content_type: content_type.to_owned(), body: String::from_utf8(body.to_vec()).unwrap() });
        if state.replies.is_empty() {
            return Err(());
        }
        state.replies.remove(0)
    }
}

fn reply(status: u16, body: Vec<u8>) -> RefreshHttpReply {
    RefreshHttpReply { status, retry_after: None, body }
}

fn success_body() -> Vec<u8> {
    br#"{"access_token":"access_SECRET_new","refresh_token":"refresh_SECRET_new","user":{"id":"user_provider_0001"},"expires_in":300}"#.to_vec()
}

fn form_fields(body: &str) -> BTreeMap<String, String> {
    body.split('&')
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap();
            (name.to_owned(), value.to_owned())
        })
        .collect()
}

#[test]
fn refresh_posts_the_public_client_form() {
    let http = ScriptedRefreshHttp::single(reply(200, success_body()));
    let transport = UreqIdentityTransport::new(http.clone());
    let response = transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap();

    let calls = http.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, TOKEN_URL, "the refresh token goes only to the fixed WorkOS token URL");
    assert_eq!(calls[0].content_type, "application/x-www-form-urlencoded");
    assert!(!calls[0].url.contains("refresh_SECRET_old"), "the token must never appear in the URL");
    let fields = form_fields(&calls[0].body);
    assert_eq!(fields.len(), 3);
    assert_eq!(fields.get("grant_type").unwrap(), "refresh_token");
    assert_eq!(fields.get("client_id").unwrap(), "client_0123456789");
    assert_eq!(fields.get("refresh_token").unwrap(), "refresh_SECRET_old");

    assert_eq!(response.access_token.expose_secret(), "access_SECRET_new");
    assert_eq!(response.refresh_token.expose_secret(), "refresh_SECRET_new");
    assert_eq!(response.provider_account_id, "user_provider_0001");
    assert_eq!(response.expires_in, Some(Duration::from_secs(300)));
    assert!(!format!("{response:?}").contains("SECRET"));

    // Reserved characters in the token are percent-encoded, never left to change the form.
    let http = ScriptedRefreshHttp::single(reply(200, success_body()));
    let transport = UreqIdentityTransport::new(http.clone());
    transport.refresh("client_0123456789", &Secret::new("refresh+SECRET/old=")).unwrap();
    let body = &http.calls()[0].body;
    assert!(body.contains("refresh_token=refresh%2BSECRET%2Fold%3D"));
    assert!(!body.contains("refresh+SECRET/old="));

    // Invalid inputs are refused before any request is posted.
    let http = ScriptedRefreshHttp::default();
    let transport = UreqIdentityTransport::new(http.clone());
    assert_eq!(transport.refresh("", &Secret::new("refresh_SECRET_old")).unwrap_err(), RefreshTransportError::Protocol);
    assert_eq!(transport.refresh("client_0123456789", &Secret::new("")).unwrap_err(), RefreshTransportError::Protocol);
    assert_eq!(transport.refresh("bad client", &Secret::new("refresh_SECRET_old")).unwrap_err(), RefreshTransportError::Protocol);
    assert!(http.calls().is_empty());
}

#[test]
fn a_scripted_transport_maps_statuses_without_a_network() {
    // Only a 200 is parsed as success; every other status is a protocol error.
    for status in [201u16, 204, 301, 302, 401, 403, 404, 418] {
        let transport = UreqIdentityTransport::new(ScriptedRefreshHttp::single(reply(status, success_body())));
        assert_eq!(
            transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap_err(),
            RefreshTransportError::Protocol,
            "status {status}"
        );
    }
    // A transport failure is transient, never terminal.
    let transport = UreqIdentityTransport::new(ScriptedRefreshHttp::new(vec![Err(())]));
    assert_eq!(
        transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap_err(),
        RefreshTransportError::Transient { retry_after: None }
    );
    // `expires_in` is accepted only as an integer in 1..=3600; anything else yields None.
    for expires in ["0", "3601", "-1", "1.5", "\"300\""] {
        let body = format!(
            r#"{{"access_token":"access_SECRET_new","refresh_token":"refresh_SECRET_new","user":{{"id":"user_provider_0001"}},"expires_in":{expires}}}"#
        );
        let transport = UreqIdentityTransport::new(ScriptedRefreshHttp::single(reply(200, body.into_bytes())));
        let response = transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap();
        assert_eq!(response.expires_in, None, "expires_in={expires}");
    }
    // An absent expires_in is None, not an error.
    let body = br#"{"access_token":"access_SECRET_new","refresh_token":"refresh_SECRET_new","user":{"id":"user_provider_0001"}}"#.to_vec();
    let transport = UreqIdentityTransport::new(ScriptedRefreshHttp::single(reply(200, body)));
    assert_eq!(transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap().expires_in, None);

    // The real adapter constructs offline and refuses any URL but the fixed token URL before
    // any network IO, so a refresh token can never be posted elsewhere.
    let adapter = UreqRefreshHttp::new();
    assert!(adapter.post("https://evil.example.test/token", "application/x-www-form-urlencoded", b"").is_err());
    assert!(adapter.post("http://api.workos.com/user_management/authenticate", "application/x-www-form-urlencoded", b"").is_err());
}

#[test]
fn invalid_grant_is_terminal_and_transient_statuses_keep_credentials() {
    let invalid_grant = br#"{"error":"invalid_grant","error_description":"the refresh token was revoked"}"#.to_vec();
    let transport = UreqIdentityTransport::new(ScriptedRefreshHttp::single(reply(400, invalid_grant)));
    assert_eq!(transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap_err(), RefreshTransportError::Terminal);

    // Other 400 errors and malformed 400 bodies are protocol, never terminal.
    for body in [br#"{"error":"invalid_request"}"#.to_vec(), b"not json".to_vec(), b"{}".to_vec()] {
        let transport = UreqIdentityTransport::new(ScriptedRefreshHttp::single(reply(400, body)));
        assert_eq!(transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap_err(), RefreshTransportError::Protocol);
    }

    // Recoverable statuses stay transient and carry an integer Retry-After.
    for status in [408u16, 429, 500, 502, 503] {
        let mut response = reply(status, Vec::new());
        response.retry_after = Some(Duration::from_secs(17));
        let transport = UreqIdentityTransport::new(ScriptedRefreshHttp::single(response));
        assert_eq!(
            transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap_err(),
            RefreshTransportError::Transient { retry_after: Some(Duration::from_secs(17)) },
            "status {status}"
        );
    }
    // A missing or out-of-range Retry-After is ignored, not guessed.
    for retry_after in [None, Some(Duration::from_secs(3601))] {
        let mut response = reply(503, Vec::new());
        response.retry_after = retry_after;
        let transport = UreqIdentityTransport::new(ScriptedRefreshHttp::single(response));
        assert_eq!(
            transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap_err(),
            RefreshTransportError::Transient { retry_after: None }
        );
    }
    // The adapter reads only an integer Retry-After in 0..=3600 seconds.
    for (text, expected) in [
        ("17", Some(Duration::from_secs(17))),
        ("0", Some(Duration::from_secs(0))),
        ("3600", Some(Duration::from_secs(3600))),
        (" 42 ", Some(Duration::from_secs(42))),
        ("3601", None),
        ("-1", None),
        ("1.5", None),
        ("", None),
        ("Wed, 21 Oct 2015 07:28:00 GMT", None),
    ] {
        assert_eq!(parse_retry_after(text), expected, "Retry-After: {text}");
    }
}

#[test]
fn malformed_responses_are_protocol() {
    let bodies: Vec<Vec<u8>> = vec![
        b"not json".to_vec(),
        b"{}".to_vec(),
        // Missing access token, refresh token or user id.
        br#"{"refresh_token":"refresh_SECRET_new","user":{"id":"user_provider_0001"}}"#.to_vec(),
        br#"{"access_token":"access_SECRET_new","user":{"id":"user_provider_0001"}}"#.to_vec(),
        br#"{"access_token":"access_SECRET_new","refresh_token":"refresh_SECRET_new"}"#.to_vec(),
        // Empty or control-bearing tokens.
        br#"{"access_token":"","refresh_token":"refresh_SECRET_new","user":{"id":"user_provider_0001"}}"#.to_vec(),
        b"{\"access_token\":\"access\\nSECRET\",\"refresh_token\":\"refresh_SECRET_new\",\"user\":{\"id\":\"user_provider_0001\"}}".to_vec(),
        // An unsafe user id.
        br#"{"access_token":"access_SECRET_new","refresh_token":"refresh_SECRET_new","user":{"id":"bad id"}}"#.to_vec(),
        br#"{"access_token":"access_SECRET_new","refresh_token":"refresh_SECRET_new","user":{"id":7}}"#.to_vec(),
        // Oversized body.
        vec![b'a'; 64 * 1024 + 1],
    ];
    for body in bodies {
        let transport = UreqIdentityTransport::new(ScriptedRefreshHttp::single(reply(200, body)));
        assert_eq!(transport.refresh("client_0123456789", &Secret::new("refresh_SECRET_old")).unwrap_err(), RefreshTransportError::Protocol);
    }
}
