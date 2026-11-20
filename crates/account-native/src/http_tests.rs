#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use solidcraft_engine::entitlement::{
    AccountId, AccountService, Admission, AuthenticatedHostScope, BillingResult, Capability, EntryCategory, GrantId, HostSessionBinding,
    MonotonicClock, MonotonicInstant, OperationId, OperationStatus, RefusalCode,
};

use crate::credentials::{CredentialSession, MemorySession};
use crate::http::{
    BootstrapPlatform, FixedCredentials, HttpAccountService, HttpConfigError, MAX_RESPONSE_BYTES, Secret, TransportFailure, WireConnection,
    WireCredentials, WireResponse,
};
use crate::http::{HandoffLog, RecordingHandoff, ScriptedTransport};

const BASE: &str = "https://auth.intcad.invalid";
const T0: u64 = 1_767_225_600_000;

struct TestClock {
    millis: Cell<u64>,
}

impl TestClock {
    fn at(millis: u64) -> Self {
        Self { millis: Cell::new(millis) }
    }
}

impl MonotonicClock for TestClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::from_millis(self.millis.get())
    }

    fn wall_now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.millis.get())
    }
}

fn scope() -> AuthenticatedHostScope {
    AuthenticatedHostScope::new(
        AccountId::new("acct_example_0001"),
        solidcraft_engine::entitlement::InstallationId::new("inst_example_0001"),
        HostSessionBinding::new("host_session_binding"),
    )
}

fn credentials() -> FixedCredentials {
    FixedCredentials::none()
        .with_product_session("product_session_example_0000000000000000")
        .with_installation_credential("installation_credential_example_0000000000000000")
        .with_settlement_credential("settlement_credential_example_0000000000000000")
        .with_execution_session_proof("execution_session_proof_example_000000000000000")
}

fn operation() -> OperationId {
    OperationId::parse("op_example_0001").unwrap()
}

fn scripted(responses: Vec<Result<WireResponse, TransportFailure>>) -> HttpAccountService<ScriptedTransport> {
    scripted_with(credentials(), responses)
}

fn scripted_with(credentials: FixedCredentials, responses: Vec<Result<WireResponse, TransportFailure>>) -> HttpAccountService<ScriptedTransport> {
    scripted_with_log(credentials, responses).0
}

fn scripted_with_log(
    credentials: FixedCredentials,
    responses: Vec<Result<WireResponse, TransportFailure>>,
) -> (HttpAccountService<ScriptedTransport>, Rc<RefCell<HandoffLog>>) {
    let (handoff, log) = RecordingHandoff::new();
    let service = HttpAccountService::with_transport(
        ScriptedTransport::new(responses),
        WireConnection::new(BASE, "1.0.0").unwrap(),
        Box::new(credentials),
        Box::new(handoff),
        Box::new(TestClock::at(T0)),
    );
    (service, log)
}

fn ok(status: u16, body: Value) -> Result<WireResponse, TransportFailure> {
    Ok(WireResponse::new(status, body.to_string().into_bytes()))
}

fn policy() -> Value {
    json!({
        "authorization_ttl_seconds": 60,
        "renewal_interval_seconds": 20,
        "unknown_release_grace_seconds": 600,
        "initial_credit_grant": 1000,
        "min_client_version": "1.0.0",
        "protocol_version": "intcad-auth/1.0",
        "max_clock_skew_ms": 5000,
        "deadline_basis_order": ["EARLIER_INVALIDATION", "LAST_GRANT_EXPIRY_BEFORE_DELETION", "LAST_GRANT_EXPIRY"],
        "deadline_half_open": true,
        "deletion_changes_deadline": false,
        "settlement_credential_can_run_or_renew": false,
        "reissue_requires_same_installation_and_active_original_account": true,
        "reissue_only_pre_deadline_nonterminal": true,
        "finalize_checks_auth_binding_deadline_before_lookup": true,
        "eligibility_excludes_credit": true,
        "capabilities_exclude_credit": true,
        "disabled_account_all_controlled_capabilities_false": true
    })
}

fn status_body(balance: u64, held: u64, available: u64) -> Value {
    json!({
        "account": {"id": "acct_example_0001", "email_verified": true, "status": "active"},
        "client": {"version": "1.0.0", "supported": true, "min_supported_version": "1.0.0"},
        "protocol": {"version": "intcad-auth/1.0", "supported": true, "min_supported_version": "intcad-auth/1.0"},
        "eligible": true,
        "reasons": [],
        "capabilities": {"chat": true, "harness": true, "review": true, "dfm": true, "mcp": true, "control": true, "autocomplete": false, "design_intent": false},
        "balance": balance,
        "held": held,
        "available": available,
        "snapshot_at": "2026-01-01T00:00:00.000Z",
        "policy": policy()
    })
}

fn admitted_body() -> Value {
    json!({
        "outcome": "NEWLY_ADMITTED",
        "operation_id": "op_example_0001",
        "installation_id": "inst_example_0001",
        "state": "HELD",
        "authorization_expires_at": "2026-01-01T00:01:00.000Z",
        "settlement_deadline": null,
        "execution_grant": {
            "grant_id": "grant_example_0001",
            "capability": "chat",
            "expires_at": "2026-01-01T00:01:00.000Z",
            "renew_after_seconds": 20,
            "session_proof": "execution_session_proof_example_000000000000000"
        },
        "settlement_credential": "settlement_credential_example_0000000000000000"
    })
}

fn auth_response_body(proof: Value) -> Value {
    json!({
        "authorization_id": "grant_example_0001",
        "capability": "chat",
        "operation_id": null,
        "renewed": false,
        "expires_at": "2026-01-01T00:01:00.000Z",
        "remaining_seconds": 60,
        "renew_after_seconds": 20,
        "execution_session_proof": proof
    })
}

fn already_exists_body() -> Value {
    json!({
        "outcome": "ALREADY_EXISTS",
        "operation_id": "op_example_0001",
        "installation_id": "inst_example_0001",
        "state": "HELD",
        "result": null,
        "terminal": false,
        "authorization_expires_at": "2026-01-01T00:01:00.000Z",
        "settlement_deadline": "2026-01-01T00:11:00.000Z",
        "settlement_credential": "settlement_credential_example_0000000000000000"
    })
}

fn receipt_body(state: &str, result: &str, terminal: bool) -> Value {
    json!({
        "operation_id": "op_example_0001",
        "state": state,
        "result": result,
        "terminal": terminal,
        "finalized_at": if terminal { Value::String("2026-01-01T00:00:30.000Z".into()) } else { Value::Null },
        "settlement_deadline": "2026-01-01T00:11:00.000Z"
    })
}

fn error_body(code: &str) -> Value {
    json!({"error": {"code": code, "message": "stable", "retryable": false, "request_id": "req_example_0001"}})
}

#[test]
fn status_returns_balance_held_available_from_one_snapshot() {
    let service = scripted(vec![ok(200, status_body(1000, 1, 999))]);
    let status = service.status().unwrap();
    assert_eq!((status.balance, status.held, status.available()), (1000, 1, 999));
    assert!(status.online);
    assert!(status.version_supported);
    assert!(status.model_available);
}

#[test]
fn begin_maps_newly_admitted_already_exists_owned_elsewhere_and_conflict() {
    let mut admitted = scripted(vec![ok(200, admitted_body())]);
    match admitted.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap() {
        Admission::NewlyAdmitted { operation: returned, grant } => {
            assert_eq!(returned.as_str(), "op_example_0001");
            assert_eq!(grant.capability(), Capability::Chat);
            assert_eq!(grant.id().as_str(), "grant_example_0001");
            assert_eq!(grant.expires_at(), MonotonicInstant::from_millis(T0 + 60_000));
        }
        other => panic!("unexpected admission: {other:?}"),
    }

    let mut existing = scripted(vec![ok(200, already_exists_body())]);
    assert!(matches!(
        existing.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap(),
        Admission::AlreadyExists { status: OperationStatus::Held }
    ));

    let mut owned = scripted(vec![ok(
        409,
        json!({"outcome": "OPERATION_OWNED_ELSEWHERE", "error": {"code": "OPERATION_OWNED_ELSEWHERE", "message": "stable", "retryable": false, "request_id": "req_example_0001"}}),
    )]);
    match owned.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap() {
        Admission::Refused(refusal) => assert_eq!(refusal.code, RefusalCode::OperationOwnedElsewhere),
        other => panic!("unexpected admission: {other:?}"),
    }

    let mut conflict = scripted(vec![ok(
        409,
        json!({"outcome": "OPERATION_CONFLICT", "error": {"code": "OPERATION_CONFLICT", "message": "stable", "retryable": false, "request_id": "req_example_0001"}}),
    )]);
    match conflict.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap() {
        Admission::Refused(refusal) => assert_eq!(refusal.code, RefusalCode::OperationConflict),
        other => panic!("unexpected admission: {other:?}"),
    }
}

#[test]
fn amibiguous_admission_never_becomes_new_authority() {
    let malformed = admitted_body();
    let mut malformed = malformed;
    malformed.as_object_mut().unwrap().remove("execution_grant");
    let mut service = scripted(vec![ok(200, malformed)]);
    let outcome = service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra);
    assert!(outcome.is_err(), "a NEWLY_ADMITTED without an execution grant must fail closed");
}

#[test]
fn finalize_is_idempotent_and_conflict_never_changes_ledger() {
    let mut service = scripted(vec![ok(200, receipt_body("CHARGED", "SUCCEEDED", true)), ok(200, receipt_body("CHARGED", "SUCCEEDED", true))]);
    assert_eq!(service.finalize(&scope(), &operation(), BillingResult::Succeeded).unwrap(), OperationStatus::Charged);
    assert_eq!(service.finalize(&scope(), &operation(), BillingResult::Succeeded).unwrap(), OperationStatus::Charged);
    let requests = service.transport().take_requests();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert!(request.url().ends_with("/v1/operations/op_example_0001/finalize"));
        assert_eq!(request.header("X-Settlement-Credential"), Some("settlement_credential_example_0000000000000000"));
    }

    let mut conflicting = scripted(vec![ok(409, error_body("OPERATION_FINALIZED"))]);
    let refusal = conflicting.finalize(&scope(), &operation(), BillingResult::FailedNoEffect).unwrap_err();
    assert_eq!(refusal.code, RefusalCode::OperationFinalized);
}

#[test]
fn unknown_returns_pending_with_original_id() {
    let mut service = scripted(vec![ok(200, receipt_body("PENDING", "UNKNOWN", false))]);
    assert_eq!(service.finalize(&scope(), &operation(), BillingResult::Unknown).unwrap(), OperationStatus::Pending);
    let requests = service.transport().take_requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url().ends_with("/v1/operations/op_example_0001/finalize"));
    let body: Value = serde_json::from_slice(requests[0].body().unwrap()).unwrap();
    assert_eq!(body, json!({"result": "UNKNOWN"}));
}

#[test]
fn expired_finalize_and_lookup_are_generic_no_state() {
    let mut finalize = scripted(vec![ok(401, error_body("AUTH_REQUIRED"))]);
    let finalize_error = finalize.finalize(&scope(), &operation(), BillingResult::Succeeded).unwrap_err();
    let lookup = scripted(vec![ok(401, error_body("AUTH_REQUIRED"))]);
    let lookup_error = lookup.operation(&scope(), &operation()).unwrap_err();
    assert_eq!(finalize_error.code, RefusalCode::AuthRequired);
    assert_eq!(lookup_error.code, RefusalCode::AuthRequired);
    assert_eq!(finalize_error.message, lookup_error.message);
}

#[test]
fn credentials_never_appear_in_url_logs_or_errors() {
    let credentials = FixedCredentials::none()
        .with_product_session("product_SECRET_abc")
        .with_installation_credential("install_SECRET_def")
        .with_settlement_credential("settle_SECRET_ghi")
        .with_execution_session_proof("proof_SECRET_jkl");
    let (mut service, handoff) = scripted_with_log(
        credentials,
        vec![
            ok(200, admitted_body()),
            ok(401, error_body("AUTH_REQUIRED")),
            ok(
                201,
                json!({"installation_id": "inst_example_0001", "installation_credential": "install_SECRET_def", "created_at": "2026-01-01T00:00:00.000Z"}),
            ),
        ],
    );
    let admission = service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap();
    assert!(matches!(admission, Admission::NewlyAdmitted { .. }));
    let error = service.finalize(&scope(), &operation(), BillingResult::Succeeded).unwrap_err();
    let bootstrap = service.bootstrap_installation(&scope(), BootstrapPlatform::Linux, Some("workstation")).unwrap();
    let rendered =
        format!("{:?} {:?} {} {:?} {:?} {:?}", service, service.transport().take_requests(), error.message, error.code, bootstrap, handoff.borrow());
    for secret in ["product_SECRET_abc", "install_SECRET_def", "settle_SECRET_ghi", "proof_SECRET_jkl"] {
        assert!(!rendered.contains(secret), "a credential leaked into observable output");
    }
    assert_eq!(format!("{:?}", Secret::new("super_secret")), "Secret(<redacted>)");
    assert_eq!(format!("{}", Secret::new("super_secret")), "<redacted>");
    assert!(!format!("{bootstrap:?}").contains("install_SECRET_def"));
}

#[test]
fn credentials_use_authentication_headers_not_urls() {
    let mut service = scripted(vec![ok(200, admitted_body())]);
    service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap();
    let requests = service.transport().take_requests();
    let request = &requests[0];
    assert_eq!(request.header("Authorization"), Some("Bearer product_session_example_0000000000000000"));
    assert_eq!(request.header("X-Installation-Credential"), Some("installation_credential_example_0000000000000000"));
    assert!(!request.url().contains("product_session"));
    assert!(!request.url().contains("installation_credential"));
    assert!(!format!("{request:?}").contains("product_session_example"));
}

#[test]
fn plain_http_and_credential_bearing_base_urls_are_rejected() {
    assert_eq!(WireConnection::new("http://auth.intcad.invalid", "1.0.0"), Err(HttpConfigError::NotHttps));
    assert_eq!(WireConnection::new("https://user:pass@auth.intcad.invalid", "1.0.0"), Err(HttpConfigError::MalformedBaseUrl));
    assert_eq!(WireConnection::new("https://auth.intcad.invalid/v1", "1.0.0"), Err(HttpConfigError::MalformedBaseUrl));
    assert_eq!(WireConnection::new("https://auth.intcad.invalid?x=1", "1.0.0"), Err(HttpConfigError::MalformedBaseUrl));
    assert!(WireConnection::new("https://auth.intcad.invalid/", "1.0.0").is_ok());
}

#[test]
fn missing_credentials_fail_closed_without_a_request() {
    let service = scripted_with(FixedCredentials::none(), vec![ok(200, status_body(1000, 0, 1000))]);
    assert_eq!(service.status().unwrap_err().code, RefusalCode::AuthRequired);
    assert!(service.transport().take_requests().is_empty());

    let injected = scripted_with(FixedCredentials::none().with_product_session("token\r\nX-Evil: 1"), vec![ok(200, status_body(1000, 0, 1000))]);
    assert_eq!(injected.status().unwrap_err().code, RefusalCode::AuthRequired);
    assert!(injected.transport().take_requests().is_empty());
}

#[test]
fn transport_and_integrity_failures_are_auth_unavailable_and_never_grant() {
    let timeout = scripted(vec![Err(TransportFailure::Timeout)]);
    assert_eq!(timeout.status().unwrap_err().code, RefusalCode::AuthUnavailable);
    let oversized = scripted(vec![Ok(WireResponse::new(200, vec![b' '; MAX_RESPONSE_BYTES + 1]))]);
    assert_eq!(oversized.status().unwrap_err().code, RefusalCode::AuthUnavailable);
    let malformed = scripted(vec![Ok(WireResponse::new(200, b"{".to_vec()))]);
    assert_eq!(malformed.status().unwrap_err().code, RefusalCode::AuthUnavailable);
    let mismatch = scripted(vec![ok(200, status_body(1000, 1, 998))]);
    assert_eq!(mismatch.status().unwrap_err().code, RefusalCode::AuthUnavailable);
    let mut inconsistent = status_body(1000, 0, 1000);
    inconsistent["client"]["supported"] = Value::Bool(false);
    let inconsistent = scripted(vec![ok(200, inconsistent)]);
    assert_eq!(inconsistent.status().unwrap_err().code, RefusalCode::AuthUnavailable);
    let unavailable = scripted(vec![ok(503, error_body("AUTH_UNAVAILABLE"))]);
    assert_eq!(unavailable.status().unwrap_err().code, RefusalCode::AuthUnavailable);
    let internal = scripted(vec![ok(500, error_body("INTERNAL_ERROR"))]);
    assert_eq!(internal.status().unwrap_err().code, RefusalCode::AuthUnavailable);
}

#[test]
fn authorize_and_lookup_map_frozen_statuses() {
    let mut authorize = scripted(vec![ok(
        200,
        json!({
            "authorization_id": "grant_example_0001",
            "capability": "chat",
            "operation_id": null,
            "renewed": false,
            "expires_at": "2026-01-01T00:01:00.000Z",
            "remaining_seconds": 60,
            "renew_after_seconds": 20,
            "execution_session_proof": "execution_session_proof_example_000000000000000"
        }),
    )]);
    let grant = authorize.authorize(&scope(), Capability::Chat).unwrap();
    assert_eq!(grant.capability(), Capability::Chat);
    assert_eq!(grant.expires_at(), MonotonicInstant::from_millis(T0 + 60_000));

    let lookup = scripted(vec![ok(
        200,
        json!({"operation_id": "op_example_0001", "state": "CHARGED", "result": "SUCCEEDED", "terminal": true, "authorization_expires_at": "2026-01-01T00:01:00.000Z", "settlement_deadline": "2026-01-01T00:11:00.000Z"}),
    )]);
    assert_eq!(lookup.operation(&scope(), &operation()).unwrap(), OperationStatus::Charged);

    let mut denied = scripted(vec![ok(403, error_body("CAPABILITY_DENIED"))]);
    assert_eq!(denied.authorize(&scope(), Capability::Chat).unwrap_err().code, RefusalCode::CapabilityDenied);
}

#[test]
fn provider_login_alone_does_not_supply_a_product_bearer() {
    let session = CredentialSession::new(Box::new(MemorySession::default()));
    session.complete_provider_login("user_provider_0001", "provider_access_SECRET").unwrap();
    assert!(WireCredentials::product_session(&session).is_none());
    assert!(!session.product_session_active());
    let transport = ScriptedTransport::new(Vec::new());
    let service = HttpAccountService::with_transport(
        transport,
        WireConnection::new(BASE, "1.0.0").unwrap(),
        Box::new(session.clone()),
        Box::new(session.clone()),
        Box::new(TestClock::at(T0)),
    );
    assert_eq!(service.status().unwrap_err().code, RefusalCode::AuthRequired);
    assert!(service.transport().take_requests().is_empty());
}

#[test]
fn initial_authorization_requires_nonempty_proof_and_hands_it_off() {
    let (mut service, log) = scripted_with_log(credentials(), vec![ok(200, auth_response_body(Value::String("proof_SECRET_initial".into())))]);
    service.authorize(&scope(), Capability::Chat).unwrap();
    let log = log.borrow();
    assert_eq!(log.initial_proofs.len(), 1);
    assert_eq!(log.initial_proofs[0].0, GrantId::new("grant_example_0001"));
    assert_eq!(log.initial_proofs[0].1, Capability::Chat);
    assert_eq!(log.initial_proofs[0].2.expose_secret(), "proof_SECRET_initial");
    assert!(log.proofs.is_empty());

    for proof in [Value::Null, Value::String(String::new())] {
        let mut service = scripted(vec![ok(200, auth_response_body(proof))]);
        assert_eq!(service.authorize(&scope(), Capability::Chat).unwrap_err().code, RefusalCode::AuthUnavailable);
    }
}

#[test]
fn begin_hands_off_credentials_and_already_exists_never_hands_off_execution() {
    let (mut service, log) = scripted_with_log(credentials(), vec![ok(200, admitted_body())]);
    assert!(matches!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap(), Admission::NewlyAdmitted { .. }));
    let log = log.borrow();
    assert_eq!(log.settlements.len(), 1);
    assert_eq!(log.settlements[0].0, operation());
    assert_eq!(log.settlements[0].1.expose_secret(), "settlement_credential_example_0000000000000000");
    assert_eq!(log.proofs.len(), 1);
    assert_eq!(log.proofs[0].0, Capability::Chat);
    assert_eq!(log.proofs[0].1, operation());
    assert_eq!(log.proofs[0].2.expose_secret(), "execution_session_proof_example_000000000000000");
    assert!(log.initial_proofs.is_empty());

    let (mut service, log) = scripted_with_log(credentials(), vec![ok(200, already_exists_body())]);
    assert!(matches!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap(), Admission::AlreadyExists { .. }));
    assert_eq!(log.borrow().settlements.len(), 1);
    assert!(log.borrow().proofs.is_empty());

    let mut terminal = already_exists_body();
    terminal["state"] = Value::String("CHARGED".into());
    terminal["result"] = Value::String("SUCCEEDED".into());
    terminal["terminal"] = Value::Bool(true);
    let mut service = scripted(vec![ok(200, terminal)]);
    assert_eq!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap_err().code, RefusalCode::AuthUnavailable);

    let mut with_grant = already_exists_body();
    with_grant["execution_grant"] = json!({"grant_id": "grant_forbidden"});
    let mut service = scripted(vec![ok(200, with_grant)]);
    assert_eq!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap_err().code, RefusalCode::AuthUnavailable);
}

#[test]
fn handoff_failure_fails_closed_without_authority() {
    let (handoff, _log) = RecordingHandoff::failing();
    let mut service = HttpAccountService::with_transport(
        ScriptedTransport::new(vec![ok(200, admitted_body())]),
        WireConnection::new(BASE, "1.0.0").unwrap(),
        Box::new(credentials()),
        Box::new(handoff),
        Box::new(TestClock::at(T0)),
    );
    assert_eq!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap_err().code, RefusalCode::AuthUnavailable);

    let (handoff, _log) = RecordingHandoff::failing();
    let mut service = HttpAccountService::with_transport(
        ScriptedTransport::new(vec![ok(200, auth_response_body(Value::String("proof_SECRET_initial".into())))]),
        WireConnection::new(BASE, "1.0.0").unwrap(),
        Box::new(credentials()),
        Box::new(handoff),
        Box::new(TestClock::at(T0)),
    );
    assert_eq!(service.authorize(&scope(), Capability::Chat).unwrap_err().code, RefusalCode::AuthUnavailable);
}

#[test]
fn installation_handoff_is_secret_wrapped() {
    let (mut service, log) = scripted_with_log(
        credentials(),
        vec![ok(
            201,
            json!({"installation_id": "inst_example_0001", "installation_credential": "install_SECRET_def", "created_at": "2026-01-01T00:00:00.000Z"}),
        )],
    );
    let bootstrap = service.bootstrap_installation(&scope(), BootstrapPlatform::Linux, Some("workstation")).unwrap();
    let log = log.borrow();
    assert_eq!(log.installations.len(), 1);
    assert_eq!(log.installations[0].0, "inst_example_0001");
    assert_eq!(log.installations[0].1.expose_secret(), "install_SECRET_def");
    assert_eq!(bootstrap.into_credential().expose_secret(), "install_SECRET_def");
    assert!(!format!("{log:?}").contains("install_SECRET_def"));
}

#[test]
fn begin_matches_response_installation_to_trusted_scope() {
    let mut wrong_installation = admitted_body();
    wrong_installation["installation_id"] = Value::String("inst_other".into());
    let mut service = scripted(vec![ok(200, wrong_installation)]);
    assert_eq!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap_err().code, RefusalCode::AuthUnavailable);

    let mut wrong_existing = already_exists_body();
    wrong_existing["installation_id"] = Value::String("inst_other".into());
    let mut service = scripted(vec![ok(200, wrong_existing)]);
    assert_eq!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap_err().code, RefusalCode::AuthUnavailable);
}

#[test]
fn untrusted_timestamps_are_total_and_numeric() {
    let mut malformed = admitted_body();
    malformed["execution_grant"]["expires_at"] = Value::String("2026-01-0üT00:01:00.000Z".into());
    let mut service = scripted(vec![ok(200, malformed)]);
    assert_eq!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap_err().code, RefusalCode::AuthUnavailable);

    let mut malformed = admitted_body();
    malformed["authorization_expires_at"] = Value::String("not-a-time-ü".into());
    let mut service = scripted(vec![ok(200, malformed)]);
    assert_eq!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap_err().code, RefusalCode::AuthUnavailable);

    let mut zero_year = admitted_body();
    zero_year["execution_grant"]["expires_at"] = Value::String("0000-01-01T00:00:00.000Z".into());
    let mut service = scripted(vec![ok(200, zero_year)]);
    assert_eq!(service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap_err().code, RefusalCode::AuthUnavailable);

    let mut earlier = admitted_body();
    earlier["authorization_expires_at"] = Value::String("2026-01-01T00:02:00.000Z".into());
    earlier["execution_grant"]["expires_at"] = Value::String("2026-01-01T00:00:30.000Z".into());
    let mut service = scripted(vec![ok(200, earlier)]);
    match service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap() {
        Admission::NewlyAdmitted { grant, .. } => assert_eq!(grant.expires_at(), MonotonicInstant::from_millis(T0 + 30_000)),
        other => panic!("unexpected admission: {other:?}"),
    }
}

#[test]
fn http_status_does_not_invent_business_refusals() {
    let service = scripted(vec![ok(403, json!({}))]);
    assert_eq!(service.status().unwrap_err().code, RefusalCode::AuthUnavailable);
    let mut service = scripted(vec![ok(402, json!({}))]);
    assert_eq!(service.authorize(&scope(), Capability::Chat).unwrap_err().code, RefusalCode::AuthUnavailable);
    let mut service = scripted(vec![ok(409, json!({}))]);
    assert_eq!(service.finalize(&scope(), &operation(), BillingResult::Succeeded).unwrap_err().code, RefusalCode::AuthUnavailable);
    let service = scripted(vec![ok(401, json!({}))]);
    assert_eq!(service.status().unwrap_err().code, RefusalCode::AuthRequired);
}

#[test]
fn finalize_rejects_invalid_receipt_state_result_combinations() {
    let invalid = [
        receipt_body("CHARGED", "FAILED_NO_EFFECT", true),
        receipt_body("RELEASED", "SUCCEEDED", true),
        receipt_body("PENDING", "UNKNOWN", true),
        receipt_body("CHARGED", "SUCCEEDED", false),
        receipt_body("EXPIRED", "SUCCEEDED", true),
    ];
    for body in invalid {
        let mut service = scripted(vec![ok(200, body)]);
        assert_eq!(service.finalize(&scope(), &operation(), BillingResult::Succeeded).unwrap_err().code, RefusalCode::AuthUnavailable);
    }

    let mut null_finalized = receipt_body("CHARGED", "SUCCEEDED", true);
    null_finalized["finalized_at"] = Value::Null;
    let mut service = scripted(vec![ok(200, null_finalized)]);
    assert_eq!(service.finalize(&scope(), &operation(), BillingResult::Succeeded).unwrap_err().code, RefusalCode::AuthUnavailable);

    let mut pending_with_time = receipt_body("PENDING", "UNKNOWN", false);
    pending_with_time["finalized_at"] = Value::String("2026-01-01T00:00:30.000Z".into());
    let mut service = scripted(vec![ok(200, pending_with_time)]);
    assert_eq!(service.finalize(&scope(), &operation(), BillingResult::Unknown).unwrap_err().code, RefusalCode::AuthUnavailable);
}

#[test]
fn begin_forwards_optional_authorization_id() {
    let mut service = scripted(vec![ok(200, admitted_body())]);
    service
        .begin_with_authorization(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra, Some(&GrantId::new("grant_example_0001")))
        .unwrap();
    let body: Value = serde_json::from_slice(service.transport().take_requests()[0].body().unwrap()).unwrap();
    assert_eq!(body["authorization_id"], "grant_example_0001");

    let mut service = scripted(vec![ok(200, admitted_body())]);
    service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap();
    let body: Value = serde_json::from_slice(service.transport().take_requests()[0].body().unwrap()).unwrap();
    assert!(body.get("authorization_id").is_none());
}
