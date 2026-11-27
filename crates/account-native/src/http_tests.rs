#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use solidcraft_engine::entitlement::{
    AccountId, AccountService, Admission, AuthenticatedHostScope, AuthorizationBound, Availability, BillingResult, BudgetTable, Capability,
    CommitDecision, CommitGate, EntryCategory, GrantId, HostSessionBinding, MonotonicClock, MonotonicInstant, OperationId, OperationStatus,
    RefusalCode, TrustedContext,
};

use crate::credentials::{CredentialSession, MemorySession};
use crate::http::{
    BootstrapPlatform, FixedCredentials, HttpAccountService, HttpConfigError, MAX_RESPONSE_BYTES, Secret, TransportFailure, WireConnection,
    WireCredentials, WireResponse,
};
use crate::http::{HandoffLog, RecordingHandoff, ScriptedTransport};

const BASE: &str = "https://auth.intcad.invalid";
const T0: u64 = 1_767_225_600_000;

#[derive(Clone)]
struct TestClock {
    millis: Rc<Cell<u64>>,
}

impl TestClock {
    fn at(millis: u64) -> Self {
        Self { millis: Rc::new(Cell::new(millis)) }
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
        "authorization_expires_at": "2026-01-01T00:05:00.000Z",
        "settlement_deadline": null,
        "execution_grant": {
            "grant_id": "grant_example_0001",
            "capability": "chat",
            "expires_at": "2026-01-01T00:05:00.000Z",
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

fn bound_session(store: Box<dyn solidcraft_engine::entitlement::CredentialStore + Send>) -> CredentialSession {
    use crate::credentials::{ProductExchangeError, ProductSession, ProductSessionExchange, ProviderSession};
    use crate::http::CredentialHandoff;

    struct Exchange;
    impl ProductSessionExchange for Exchange {
        fn exchange(&self, _: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
            Ok(ProductSession::from_trusted_exchange("acct_example_0001", Secret::new("fixture_bearer"), None))
        }
    }
    let mut session = CredentialSession::new(store);
    session.complete_provider_login("provider_fixture", "provider_token").unwrap();
    session.activate_product_session(&Exchange).unwrap();
    CredentialHandoff::installation_credential(&mut session, "inst_example_0001", Secret::new("fixture_installation")).unwrap();
    session
}

#[test]
fn billing_reads_the_current_bearer_for_admission_dispatch_renewal_and_commit_checks() {
    use crate::SessionBilling;
    use crate::credentials_tests::RecordingStore;
    use solidcraft_engine::entitlement::BillingPort;

    struct RotateAfterStatus {
        wire: Rc<ScriptedTransport>,
        session: CredentialSession,
        scope: AuthenticatedHostScope,
        status_reads: Cell<u8>,
    }
    impl crate::http::Transport for RotateAfterStatus {
        fn send(&self, request: &crate::http::WireRequest) -> Result<WireResponse, TransportFailure> {
            if request.method() == "GET" {
                let status_reads = self.status_reads.get() + 1;
                self.status_reads.set(status_reads);
                self.session.update_product_session(&self.scope, &Secret::new(format!("rotation_{status_reads}"))).unwrap();
            }
            self.wire.send(request)
        }
    }
    let session = bound_session(Box::new(RecordingStore::default()));
    let scope = session.authenticated_scope().unwrap();
    let clock = TestClock::at(T0);
    let mut admission = admitted_body();
    admission["execution_grant"]["capability"] = json!("dfm");
    let renewal = json!({"authorization_id":"grant_example_0001","operation_id":"op_example_0001","capability":"dfm","renewed":true,
        "remaining_seconds":300,"renew_after_seconds":20,"execution_session_proof":null});
    let wire = Rc::new(ScriptedTransport::new(vec![
        ok(200, status_body(1000, 0, 1000)),
        ok(200, admission),
        ok(200, status_body(1000, 1, 999)),
        ok(200, renewal),
        ok(200, status_body(1000, 1, 999)),
        ok(200, receipt_body("CHARGED", "SUCCEEDED", true)),
    ]));
    let path = std::env::temp_dir().join(format!("intcad-billing-rotation-{}-{:?}", std::process::id(), std::thread::current().id()));
    let mut billing = SessionBilling::new(
        session.clone(),
        WireConnection::new(BASE, "1.0.0").unwrap(),
        Box::new(RotateAfterStatus { wire: wire.clone(), session: session.clone(), scope: scope.clone(), status_reads: Cell::new(0) }),
        path.clone(),
        clock.clone(),
    );
    let operation = operation();
    billing.admit(&operation, Capability::Dfm, EntryCategory::CliDfm).unwrap();
    clock.millis.set(T0 + 21_000);
    billing.dispatch(&operation, Duration::from_secs(1)).unwrap();
    billing.check_commit(&operation).unwrap();
    assert_eq!(billing.commit(&operation, BillingResult::Succeeded), Ok(BillingResult::Succeeded));
    billing.check_publish(&operation).unwrap();
    assert_eq!(billing.scope(), Some(scope));
    let requests = wire.take_requests();
    assert_eq!(
        requests.iter().map(|request| request.header("Authorization")).collect::<Vec<_>>(),
        [
            Some("Bearer fixture_bearer"),
            Some("Bearer rotation_1"),
            Some("Bearer rotation_1"),
            Some("Bearer rotation_2"),
            Some("Bearer rotation_2"),
            None
        ]
    );
    assert_eq!(requests[3].header("X-Execution-Session-Proof"), Some("execution_session_proof_example_000000000000000"));
    assert!(requests[5].header("X-Settlement-Credential").is_some());
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn billing_preflight_rejects_a_replaced_installation_credential_even_when_scope_is_unchanged() {
    use crate::SessionBilling;
    use crate::credentials_tests::RecordingStore;
    use solidcraft_engine::entitlement::BillingPort;

    let mut session = bound_session(Box::new(RecordingStore::default()));
    let scope = session.authenticated_scope().unwrap();
    let billing = SessionBilling::new(
        session.clone(),
        WireConnection::new(BASE, "1.0.0").unwrap(),
        Box::new(ScriptedTransport::new(vec![])),
        std::env::temp_dir().join("intcad-unused-billing-journal"),
        TestClock::at(T0),
    );
    assert!(billing.preflight().is_ok());
    crate::CredentialHandoff::installation_credential(&mut session, scope.installation().as_str(), Secret::new("replacement_installation")).unwrap();
    assert_eq!(session.authenticated_scope(), Some(scope));
    assert_eq!(billing.preflight().unwrap_err().code, RefusalCode::AuthUnavailable);
}

#[test]
fn automatic_control_renewal_pins_the_proof_and_keeps_the_old_cutoff_on_401_or_late_success() {
    use crate::credentials_tests::RecordingStore;
    use crate::http::{CredentialHandoff, OperationRenewal};
    use solidcraft_engine::entitlement::{ConservativeDeadline, Grant};

    struct TimedReply {
        wire: ScriptedTransport,
        clock: TestClock,
        receive_at: u64,
        close_gate: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    }
    impl crate::http::Transport for TimedReply {
        fn send(&self, request: &crate::http::WireRequest) -> Result<WireResponse, TransportFailure> {
            self.clock.millis.set(self.receive_at);
            if let Some(gate) = &self.close_gate {
                gate.store(false, std::sync::atomic::Ordering::Release);
            }
            self.wire.send(request)
        }
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Change {
        None,
        Proof,
        Scope,
        Generation,
        ResponseOperation,
        GateBefore,
        GateAfter,
        GateDuringBindingRead,
    }
    for (status, receive_at, change) in [
        (200, 21_000, Change::None),
        (401, 21_000, Change::None),
        (200, 60_000, Change::None),
        (200, 21_000, Change::Proof),
        (200, 21_000, Change::Scope),
        (200, 21_000, Change::Generation),
        (200, 21_000, Change::ResponseOperation),
        (200, 21_000, Change::GateBefore),
        (200, 21_000, Change::GateAfter),
        (200, 21_000, Change::GateDuringBindingRead),
    ] {
        let clock = TestClock::at(0);
        let store = RecordingStore::default();
        let mut session = bound_session(Box::new(store.clone()));
        let scope = session.authenticated_scope().unwrap();
        let operation = operation();
        CredentialHandoff::execution_session_proof(&mut session, Capability::Control, &operation, Secret::new("original_proof")).unwrap();
        let grant = Grant::from_server(GrantId::new("grant_example_0001"), Capability::Control, MonotonicInstant::from_millis(60_000));
        let mut body = if status == 401 {
            error_body("AUTH_REQUIRED")
        } else {
            json!({"authorization_id":"grant_example_0001","operation_id":"op_example_0001","capability":"control","renewed":true,
                "remaining_seconds":60,"renew_after_seconds":17,"execution_session_proof":null,"expires_at":"2026-01-01T00:01:20.000Z"})
        };
        if change == Change::ResponseOperation {
            body["operation_id"] = json!("op_example_0002");
        }
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(change != Change::GateBefore));
        let transport = TimedReply {
            wire: ScriptedTransport::new(vec![ok(status, body)]),
            clock: clock.clone(),
            receive_at,
            close_gate: (change == Change::GateAfter).then(|| ready.clone()),
        };
        let deadline = ConservativeDeadline::new(clock.now(), Duration::from_secs(60));
        let mut renewal = OperationRenewal::new(
            transport,
            WireConnection::new(BASE, "1.0.0").unwrap(),
            session.clone(),
            Box::new(clock.clone()),
            scope.clone(),
            operation.clone(),
            grant,
            deadline.clone(),
            Duration::from_secs(20),
        )
        .unwrap()
        .with_gate({
            let store = store.clone();
            let checks = std::cell::Cell::new(0);
            move || {
                checks.set(checks.get() + 1);
                if change == Change::GateDuringBindingRead && checks.get() == 4 {
                    store.close_gate_on_next_read(ready.clone());
                }
                ready.load(std::sync::atomic::Ordering::Acquire).then_some((3, 7))
            }
        });
        clock.millis.set(19_999);
        assert_eq!(renewal.tick(), Ok(false));
        session.update_product_session(&scope, &Secret::new("rotated_bearer")).unwrap();
        match change {
            Change::Proof => {
                CredentialHandoff::execution_session_proof(&mut session, Capability::Control, &operation, Secret::new("replacement_proof")).unwrap();
            }
            Change::Scope => store.force_put("auth/session_binding", "replacement_binding"),
            Change::Generation => {
                session.revoke_in_process();
                session.end_revocation();
            }
            Change::None | Change::ResponseOperation | Change::GateBefore | Change::GateAfter | Change::GateDuringBindingRead => {}
        }
        clock.millis.set(20_000);
        let result = renewal.tick();
        if status == 200 && receive_at < 60_000 && change == Change::None {
            assert_eq!(result, Ok(true));
            assert_eq!(renewal.deadline().expires_at(), MonotonicInstant::from_millis(80_000));
            clock.millis.set(36_999);
            assert_eq!(renewal.tick(), Ok(false));
        } else if matches!(change, Change::GateBefore | Change::GateAfter | Change::GateDuringBindingRead) {
            assert_eq!(result, Ok(false));
            assert_eq!(renewal.deadline(), &deadline);
            assert!(renewal.wait_for_next_tick().is_some(), "verification pause does not retire the operation");
        } else {
            assert!(result.is_err());
            assert_eq!(renewal.deadline(), &deadline);
            assert_eq!(renewal.tick(), Ok(false), "a refused or late lease never retries");
        }
        let requests = renewal.transport().wire.take_requests();
        assert_eq!(
            requests.len(),
            usize::from(matches!(change, Change::None | Change::ResponseOperation | Change::GateAfter | Change::GateDuringBindingRead))
        );
        if let Some(request) = requests.first() {
            assert_eq!(request.header("Authorization"), Some("Bearer rotated_bearer"));
            assert_eq!(request.header("X-Execution-Session-Proof"), Some("original_proof"));
            assert_eq!(request.header("X-Settlement-Credential"), None);
            assert_eq!(
                serde_json::from_slice::<Value>(request.body().unwrap()).unwrap(),
                json!({"capability":"control","operation_id":"op_example_0001","renewal":{"authorization_id":"grant_example_0001"}})
            );
        }
        assert_eq!(session.renewal_refusal(&scope, &operation), (status == 401).then_some(RefusalCode::AuthRequired));
        assert_eq!(session.renewal_refusal(&scope, &OperationId::parse("op_example_0002").unwrap()), None);
        if change != Change::Scope {
            assert_eq!(session.authenticated_scope(), Some(scope));
        }
    }
}

#[test]
fn failed_renewal_cannot_extend_the_original_operation_at_the_dispatch_and_commit_gates() {
    for (response, expected, auth_rejected) in [
        (Err(TransportFailure::Timeout), RefusalCode::AuthUnavailable, false),
        (ok(401, error_body("AUTH_REQUIRED")), RefusalCode::AuthRequired, true),
    ] {
        let clock = TestClock::at(T0);
        let (handoff, _) = RecordingHandoff::new();
        let mut service = HttpAccountService::with_transport(
            ScriptedTransport::new(vec![ok(200, status_body(1000, 0, 1000)), ok(200, admitted_body()), response]),
            WireConnection::new(BASE, "1.0.0").unwrap(),
            Box::new(credentials()),
            Box::new(handoff),
            Box::new(clock.clone()),
        );
        let (scope, operation) = (scope(), operation());
        let (capability, entry) = (Capability::Chat, EntryCategory::Mastra);
        let online = service.status().unwrap();
        let Admission::NewlyAdmitted { grant, .. } = service.begin(&scope, &operation, capability, entry).unwrap() else {
            panic!("expected an original operation lease");
        };
        let trusted = TrustedContext::mint(&scope, operation.as_str(), capability, entry).unwrap();
        let mut budget = BudgetTable::unverified_default();
        budget.set_verified(capability, entry, Duration::from_secs(1));
        let gates = |availability: &Availability, now| {
            let bound = AuthorizationBound::ExpiresAt(grant.expires_at());
            (
                budget.require(capability, entry, availability, bound.remaining(now).unwrap_or_default()).map_err(|err| err.code),
                CommitGate::new(&operation, capability, entry)
                    .with_trusted(&trusted)
                    .with_current_scope(&scope)
                    .with_availability(availability)
                    .with_status(OperationStatus::Held)
                    .with_bound(bound)
                    .with_now(now)
                    .check(),
            )
        };

        clock.millis.set(T0 + 20_000);
        assert_eq!(gates(&online, clock.now()), (Ok(Duration::from_secs(1)), CommitDecision::Allow));
        let refusal = service.renew(&scope, &operation, grant.id(), capability).unwrap_err();
        assert_eq!(refusal.code, expected);
        assert_eq!(grant.expires_at(), MonotonicInstant::from_millis(T0 + 300_000));

        let requests = service.transport().take_requests();
        assert_eq!(requests.len(), 3);
        let request = &requests[2];
        assert_eq!(request.method(), "POST");
        assert_eq!(request.url(), format!("{BASE}/v1/authorizations"));
        assert_eq!(request.header("X-Execution-Session-Proof"), Some("execution_session_proof_example_000000000000000"));
        assert_eq!(
            serde_json::from_slice::<Value>(request.body().unwrap()).unwrap(),
            json!({"capability": "chat", "operation_id": "op_example_0001", "renewal": {"authorization_id": "grant_example_0001"}})
        );

        if auth_rejected {
            // Supply the observed refusal to both gates as a host would. This is not a test
            // of automatic revocation propagation, which is not wired in production.
            let mut blocked = online.clone();
            blocked.reasons.push(refusal.code);
            assert_eq!(gates(&blocked, clock.now()), (Err(expected), CommitDecision::Drop { code: expected }));
        }
        // Even with an otherwise online snapshot, no new lease means the original,
        // half-open cutoff still stops dispatch and commit.
        clock.millis.set(T0 + 300_000);
        let expired = RefusalCode::UnsafeToStart;
        assert_eq!(gates(&online, clock.now()), (Err(expired), CommitDecision::Drop { code: expired }));
    }
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
fn restart_status_requires_the_exact_account_and_bearer_without_sending_anything_else() {
    let service = scripted(vec![ok(200, status_body(1000, 1, 999))]);
    service.restart_status("acct_example_0001").unwrap();
    let requests = service.transport().take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method(), "GET");
    assert_eq!(requests[0].url(), "https://auth.intcad.invalid/v1/status");
    assert_eq!(requests[0].header("Authorization"), Some("Bearer product_session_example_0000000000000000"));
    assert!(requests[0].header("X-Installation-Credential").is_none());
    assert!(requests[0].body().is_none());

    let mismatch = scripted(vec![ok(200, status_body(1000, 1, 999))]);
    assert_eq!(mismatch.restart_status("acct_other_0001").unwrap_err().code, RefusalCode::AuthRequired);

    let missing = scripted_with(FixedCredentials::none(), vec![ok(200, status_body(1000, 1, 999))]);
    assert_eq!(missing.restart_status("acct_example_0001").unwrap_err().code, RefusalCode::AuthRequired);
    assert!(missing.transport().take_requests().is_empty(), "a missing bearer is refused before any request");
}

#[test]
fn restart_status_enforces_every_fresh_login_activation_condition() {
    let mut unverified_email = status_body(1000, 0, 1000);
    unverified_email["account"]["email_verified"] = json!(false);
    unverified_email["eligible"] = json!(false);
    unverified_email["reasons"] = json!(["EMAIL_UNVERIFIED"]);

    let mut ineligible_client = status_body(1000, 0, 1000);
    ineligible_client["client"]["supported"] = json!(false);
    ineligible_client["eligible"] = json!(false);
    ineligible_client["reasons"] = json!(["VERSION_UNSUPPORTED"]);

    let mut wrong_protocol_version = status_body(1000, 0, 1000);
    wrong_protocol_version["protocol"]["version"] = json!("intcad-auth/2.0");

    let mut unsupported_protocol = status_body(1000, 0, 1000);
    unsupported_protocol["protocol"]["supported"] = json!(false);
    unsupported_protocol["eligible"] = json!(false);
    unsupported_protocol["reasons"] = json!(["PROTOCOL_UNSUPPORTED"]);

    for (case, body) in [
        ("unverified email", unverified_email),
        ("ineligible client", ineligible_client),
        ("wrong protocol version", wrong_protocol_version),
        ("unsupported protocol", unsupported_protocol),
    ] {
        // The generic status decoder stays a permissive snapshot; only restart activation narrows.
        let status = scripted(vec![ok(200, body.clone())]);
        assert!(status.status().is_ok(), "generic status must not be narrowed for {case}");

        let restart = scripted(vec![ok(200, body)]);
        assert_eq!(
            restart.restart_status("acct_example_0001").unwrap_err().code,
            RefusalCode::AuthRequired,
            "restart must enforce the fresh-login activation condition for {case}"
        );
    }
}

#[test]
fn begin_maps_newly_admitted_already_exists_owned_elsewhere_and_conflict() {
    let mut admitted = scripted(vec![ok(200, admitted_body())]);
    match admitted.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap() {
        Admission::NewlyAdmitted { operation: returned, grant } => {
            assert_eq!(returned.as_str(), "op_example_0001");
            assert_eq!(grant.capability(), Capability::Chat);
            assert_eq!(grant.id().as_str(), "grant_example_0001");
            assert_eq!(grant.expires_at(), MonotonicInstant::from_millis(T0 + 300_000));
            let (initial, due) = grant.renewal_schedule(&scope(), &operation()).unwrap();
            assert_eq!(initial.expires_at(), grant.expires_at());
            assert_eq!(due, MonotonicInstant::from_millis(T0 + 20_000));
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
        .with_installation_credential("installation_credential_SECRET_def_0000000000000000")
        .with_settlement_credential("settle_SECRET_ghi")
        .with_execution_session_proof("proof_SECRET_jkl");
    let (mut service, handoff) = scripted_with_log(
        credentials,
        vec![
            ok(200, admitted_body()),
            ok(401, error_body("AUTH_REQUIRED")),
            ok(
                201,
                json!({"installation_id": "inst_example_0001", "installation_credential": "installation_credential_SECRET_def_0000000000000000", "created_at": "2026-01-01T00:00:00.000Z"}),
            ),
        ],
    );
    let admission = service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap();
    assert!(matches!(admission, Admission::NewlyAdmitted { .. }));
    let error = service.finalize(&scope(), &operation(), BillingResult::Succeeded).unwrap_err();
    let bootstrap = service.bootstrap_installation(&scope(), BootstrapPlatform::Linux, Some("workstation")).unwrap();
    let rendered =
        format!("{:?} {:?} {} {:?} {:?} {:?}", service, service.transport().take_requests(), error.message, error.code, bootstrap, handoff.borrow());
    for secret in ["product_SECRET_abc", "installation_credential_SECRET_def_0000000000000000", "settle_SECRET_ghi", "proof_SECRET_jkl"] {
        assert!(!rendered.contains(secret), "a credential leaked into observable output");
    }
    assert_eq!(format!("{:?}", Secret::new("super_secret")), "Secret(<redacted>)");
    assert_eq!(format!("{}", Secret::new("super_secret")), "<redacted>");
    assert!(!format!("{bootstrap:?}").contains("installation_credential_SECRET_def_0000000000000000"));
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
            json!({"installation_id": "inst_example_0001", "installation_credential": "installation_credential_SECRET_def_0000000000000000", "created_at": "2026-01-01T00:00:00.000Z"}),
        )],
    );
    let bootstrap = service.bootstrap_installation(&scope(), BootstrapPlatform::Linux, Some("workstation")).unwrap();
    let log = log.borrow();
    assert_eq!(log.installations.len(), 1);
    assert_eq!(log.installations[0].0, "inst_example_0001");
    assert_eq!(log.installations[0].1.expose_secret(), "installation_credential_SECRET_def_0000000000000000");
    assert_eq!(bootstrap.into_credential().expose_secret(), "installation_credential_SECRET_def_0000000000000000");
    assert!(!format!("{log:?}").contains("installation_credential_SECRET_def_0000000000000000"));
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
fn absolute_admission_lease_is_capped_at_rev4_without_extending_a_short_grant() {
    for (expiry, expected_ms) in [
        ("2026-01-01T00:05:00.000Z", 300_000),
        ("2026-01-01T00:00:30.000Z", 30_000),
        ("2026-01-02T00:00:00.000Z", 300_000), // The host wall clock is far behind the server's.
        ("2025-12-31T00:00:00.000Z", 0),       // An already expired server timestamp grants no room.
    ] {
        let mut body = admitted_body();
        body["execution_grant"]["expires_at"] = json!(expiry);
        body["authorization_expires_at"] = json!(expiry);
        let mut service = scripted(vec![ok(200, body)]);
        let Admission::NewlyAdmitted { grant, .. } = service.begin(&scope(), &operation(), Capability::Chat, EntryCategory::Mastra).unwrap() else {
            panic!("expected a parsed admission for {expiry}");
        };
        assert_eq!(grant.expires_at(), MonotonicInstant::from_millis(T0 + expected_ms), "{expiry}");
    }
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

const FIRST_INSTALL_CREDENTIAL: &str = "installation_credential_example_0000000000000000";

fn installation_body() -> Value {
    json!({
        "installation_id": "inst_example_0001",
        "installation_credential": FIRST_INSTALL_CREDENTIAL,
        "created_at": "2026-01-01T00:00:00.000Z"
    })
}

fn product_only_credentials() -> FixedCredentials {
    FixedCredentials::none().with_product_session("product_session_example_0000000000000000")
}

#[test]
fn first_installation_bootstraps_from_a_verified_session_without_a_scope() {
    let (mut service, log) = scripted_with_log(product_only_credentials(), vec![ok(201, installation_body())]);
    let bootstrap = service.bootstrap_first_installation(BootstrapPlatform::Linux, Some("workstation")).unwrap();
    let requests = service.transport().take_requests();
    assert_eq!(requests.len(), 1, "one POST /v1/installations");
    let request = &requests[0];
    assert_eq!(request.method(), "POST");
    assert_eq!(request.url(), "https://auth.intcad.invalid/v1/installations");
    assert_eq!(request.header("Authorization"), Some("Bearer product_session_example_0000000000000000"));
    assert!(request.header("X-Installation-Credential").is_none(), "a first installation has no credential yet");
    let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
    assert_eq!(body, json!({"platform": "linux", "label": "workstation"}));
    assert!(!request.url().contains("product_session"), "the product session never enters the URL");
    assert_eq!(bootstrap.installation_id(), "inst_example_0001");
    assert_eq!(bootstrap.created_at(), "2026-01-01T00:00:00.000Z");
    assert_eq!(bootstrap.into_credential().expose_secret(), FIRST_INSTALL_CREDENTIAL);
    let log = log.borrow();
    assert_eq!(log.installations.len(), 1);
    assert_eq!(log.installations[0].0, "inst_example_0001");
    assert_eq!(log.installations[0].1.expose_secret(), FIRST_INSTALL_CREDENTIAL);
    assert!(!format!("{log:?}").contains(FIRST_INSTALL_CREDENTIAL), "the raw credential is redacted");
}

#[test]
fn first_installation_without_a_product_session_or_after_binding_sends_nothing() {
    let (mut no_product, log) = scripted_with_log(FixedCredentials::none(), vec![ok(201, installation_body())]);
    assert_eq!(no_product.bootstrap_first_installation(BootstrapPlatform::Linux, None).unwrap_err().code, RefusalCode::AuthRequired);
    assert!(no_product.transport().take_requests().is_empty(), "a provider-only or empty session sends no bootstrap");
    assert!(log.borrow().installations.is_empty());

    let bound = product_only_credentials().with_installation_credential(FIRST_INSTALL_CREDENTIAL);
    let (mut already_bound, log) = scripted_with_log(bound, vec![ok(201, installation_body())]);
    assert_eq!(already_bound.bootstrap_first_installation(BootstrapPlatform::Linux, None).unwrap_err().code, RefusalCode::AuthRequired);
    assert!(already_bound.transport().take_requests().is_empty(), "an existing installation is never replaced");
    assert!(log.borrow().installations.is_empty());
}

#[test]
fn first_installation_rejects_a_response_outside_the_frozen_bootstrap_schema() {
    let valid = installation_body();
    let mut missing_credential = valid.clone();
    missing_credential.as_object_mut().unwrap().remove("installation_credential");
    let mut extra_field = valid.clone();
    extra_field["balance"] = json!(1000);
    let mut short_id = valid.clone();
    short_id["installation_id"] = json!("inst_short");
    let mut wrong_id_prefix = valid.clone();
    wrong_id_prefix["installation_id"] = json!("installation_example_0001");
    let mut short_credential = valid.clone();
    short_credential["installation_credential"] = json!("installation_credential");
    let mut bad_credential_character = valid.clone();
    bad_credential_character["installation_credential"] = json!("installation_credential_0000000000000000 with-space");
    let mut malformed_time = valid.clone();
    malformed_time["created_at"] = json!("not-a-time");
    let mut empty_time = valid.clone();
    empty_time["created_at"] = json!("");

    let cases = [
        (201, Value::Array(vec![])),
        (201, missing_credential),
        (201, extra_field),
        (201, short_id),
        (201, wrong_id_prefix),
        (201, short_credential),
        (201, bad_credential_character),
        (201, malformed_time),
        (201, empty_time),
        (200, valid),
    ];
    for (status, body) in cases {
        let (mut service, log) = scripted_with_log(product_only_credentials(), vec![ok(status, body)]);
        assert_eq!(
            service.bootstrap_first_installation(BootstrapPlatform::Linux, None).unwrap_err().code,
            RefusalCode::AuthUnavailable,
            "a response outside the frozen schema is never accepted"
        );
        assert_eq!(service.transport().take_requests().len(), 1, "the bootstrap request was sent once");
        assert!(log.borrow().installations.is_empty(), "no credential is handed off from a malformed response");
    }
}

#[test]
fn first_installation_refuses_when_the_response_is_an_error_or_transport_fails() {
    let (mut auth, log) = scripted_with_log(product_only_credentials(), vec![ok(401, error_body("AUTH_REQUIRED"))]);
    assert_eq!(auth.bootstrap_first_installation(BootstrapPlatform::Linux, None).unwrap_err().code, RefusalCode::AuthRequired);
    assert!(log.borrow().installations.is_empty());

    let (mut disabled, log) = scripted_with_log(product_only_credentials(), vec![ok(403, error_body("ACCOUNT_DISABLED"))]);
    assert_eq!(disabled.bootstrap_first_installation(BootstrapPlatform::Linux, None).unwrap_err().code, RefusalCode::AccountDisabled);
    assert!(log.borrow().installations.is_empty());

    let (mut unavailable, log) = scripted_with_log(product_only_credentials(), vec![ok(503, error_body("AUTH_UNAVAILABLE"))]);
    assert_eq!(unavailable.bootstrap_first_installation(BootstrapPlatform::Linux, None).unwrap_err().code, RefusalCode::AuthUnavailable);
    assert!(log.borrow().installations.is_empty());

    let (mut timeout, log) = scripted_with_log(product_only_credentials(), vec![Err(TransportFailure::Timeout)]);
    assert_eq!(timeout.bootstrap_first_installation(BootstrapPlatform::Linux, None).unwrap_err().code, RefusalCode::AuthUnavailable);
    assert!(log.borrow().installations.is_empty());
}

#[test]
fn first_installation_handoff_failure_returns_no_binding() {
    let (handoff, log) = RecordingHandoff::failing();
    let mut service = HttpAccountService::with_transport(
        ScriptedTransport::new(vec![ok(201, installation_body())]),
        WireConnection::new(BASE, "1.0.0").unwrap(),
        Box::new(product_only_credentials()),
        Box::new(handoff),
        Box::new(TestClock::at(T0)),
    );
    assert_eq!(service.bootstrap_first_installation(BootstrapPlatform::Linux, None).unwrap_err().code, RefusalCode::AuthUnavailable);
    assert!(log.borrow().installations.is_empty());
}
