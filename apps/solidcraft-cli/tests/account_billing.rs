//! Controlled CLI DFM accounting: classification from the resolved profile, one admission,
//! one settlement for a produced report, a release for an incomplete run, and a whole refusal
//! when the authority or the safe-stop bound is missing.
//!
//! Every double here is local to this test file. No production path installs a fake grant, and
//! no test sends a live request.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use solidcraft_account_native::{
    CredentialSession, FileOperationStore, IdentityClock, IdentityDomain, IdentityLifecycle, IdentityTransport, ProductExchangeError, ProductSession,
    ProductSessionExchange, ProviderSession, RefreshResponse, RefreshTransportError, Secret, SessionBilling, SessionVerifier, Transport,
    TransportFailure, VerifiedSession, VerifyError, WireConnection, WireRequest, WireResponse,
};
use solidcraft_cli::account::{McpAccount, McpRoute, compose_mcp_server};
use solidcraft_cli::validation::{ControlledCheck, NoAuthorityBilling, ProfileScope, profile_scope, run_controlled_check, run_with_billing};
use solidcraft_engine::entitlement::{
    BillingPort, BillingResult, BudgetTable, Capability, CredentialError, CredentialStore, EntryCategory, Grant, GrantId, MonotonicClock,
    MonotonicInstant, OperationId, OperationStatus, OperationStore, Refusal, RefusalCode, UpperBound,
};
use solidcraft_engine::io::candidate::{self, CandidateFile};
use solidcraft_engine::schema::snapshot::{CandidateArtifact, CandidateRef, SnapshotSource};
use solidcraft_engine::validation::{
    CheckOutcome, CheckProfile, CheckRequirement, CheckState, Limits, LocalRunner, RunControl, RunJob, Runner, RunnerIdentity, RunnerOutcome,
    SystemClock, ValidationPort, generate_submission_key, now_ms,
};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("intcad-cli-account-billing-{name}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn dir(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A local billing double at the host seam. It never ships and never opens a network connection.
#[derive(Default)]
struct CountingBilling {
    admits: usize,
    commits: usize,
    releases: usize,
    deny: bool,
}

impl CountingBilling {
    fn denying() -> Self {
        Self { deny: true, ..Self::default() }
    }
}

impl BillingPort for CountingBilling {
    fn admit(&mut self, operation: &OperationId, capability: Capability, _entry: EntryCategory) -> Result<Grant, Refusal> {
        self.admits += 1;
        if self.deny {
            return Err(Refusal::new(RefusalCode::AuthRequired, "the scripted admission is refused"));
        }
        Ok(Grant::from_server(GrantId::new(format!("grant_test_{}", operation.as_str())), capability, MonotonicInstant::from_millis(60_000)))
    }

    fn commit(&mut self, _operation: &OperationId, result: BillingResult) -> Result<BillingResult, Refusal> {
        self.commits += 1;
        Ok(result)
    }

    fn release(&mut self, _operation: &OperationId, _reason: RefusalCode) -> Result<(), Refusal> {
        self.releases += 1;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Probe {
    Passed,
    Violations,
    Infrastructure,
}

struct FixedRunner {
    identity: RunnerIdentity,
    probe: Probe,
    runs: Arc<AtomicU64>,
}

impl Runner for FixedRunner {
    fn identity(&self) -> RunnerIdentity {
        self.identity.clone()
    }

    fn run(&self, job: &RunJob, _control: &RunControl) -> RunnerOutcome {
        self.runs.fetch_add(1, Ordering::Relaxed);
        if matches!(self.probe, Probe::Infrastructure) {
            return RunnerOutcome::Infrastructure {
                started_at_ms: Some(job.started_at_ms),
                finished_at_ms: job.started_at_ms.saturating_add(1),
                message: "the inert fixture failed before a report".into(),
            };
        }
        let checks = job
            .profile
            .checks
            .iter()
            .map(|check| {
                let state =
                    if matches!(self.probe, Probe::Violations) && check.check_id == "dfm.stub.001" { CheckState::Failed } else { CheckState::Passed };
                CheckOutcome {
                    check_id: check.check_id.clone(),
                    state,
                    expected: json!({}),
                    actual: json!({"fixture": true}),
                    // A required passing check must carry evidence; without it the port treats the
                    // report as unproven infrastructure failure, which is a different test.
                    evidence: if state == CheckState::Passed {
                        vec![solidcraft_engine::validation::EvidenceBytes {
                            media_type: "application/json".into(),
                            bytes: b"{\"fixture\":true}".to_vec(),
                        }]
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect();
        RunnerOutcome::Finished { started_at_ms: job.started_at_ms, finished_at_ms: job.started_at_ms.saturating_add(1), checks }
    }
}

/// A valid inert profile: the fixed native checks plus one `dfm.`-named stub whose kind the
/// runner reports directly. It exercises classification and billing, not real DFM geometry.
fn dfm_profile() -> CheckProfile {
    CheckProfile {
        schema_version: "1.0".into(),
        profile_id: "task-cli-dfm".into(),
        version: "1.0.0".into(),
        checks: vec![
            CheckRequirement { check_id: "dfm.stub.001".into(), kind: "dfm.stub".into(), required: true, parameters: json!({}) },
            CheckRequirement { check_id: "input.integrity".into(), kind: "input.integrity".into(), required: true, parameters: json!({}) },
            CheckRequirement { check_id: "native.rebuild".into(), kind: "native.rebuild".into(), required: true, parameters: json!({}) },
        ],
    }
}

fn verified_budget() -> BudgetTable {
    let mut budget = BudgetTable::unverified_default();
    budget.set_verified(Capability::Dfm, EntryCategory::CliDfm, Duration::from_secs(30));
    budget
}

fn write_candidate(root: &Path, id: &str) -> CandidateRef {
    let files = vec![CandidateFile { path: "design.sc".into(), bytes: b"inert fixture: the fixed runner reads no design".to_vec() }];
    let mut inputs: Vec<Value> = files
        .iter()
        .map(|file| {
            json!({
                "path": file.path,
                "kind": "design",
                "dependencyId": "design-root",
                "dependencyVersion": null,
                "digest": candidate::digest(&file.bytes),
                "sizeBytes": file.bytes.len(),
            })
        })
        .collect();
    inputs.sort_by(|a, b| a["path"].as_str().unwrap_or_default().cmp(b["path"].as_str().unwrap_or_default()));
    let manifest = json!({
        "schemaVersion": "1.0",
        "candidateId": id,
        "source": {"documentId": "doc-cli-dfm", "revision": "7"},
        "capturedAt": "2026-11-07T12:00:00.000Z",
        "formatVersion": "solidcraft/unknown",
        "engineVersion": LocalRunner::identity().engine_version,
        "inputs": inputs,
    });
    let stored = candidate::write(root, id, &files, &manifest, &format!("artifact-{id}")).unwrap();
    CandidateRef {
        candidate_id: id.into(),
        source: SnapshotSource { document_id: "doc-cli-dfm".into(), revision: "7".into() },
        candidate_digest: stored.candidate_digest,
        manifest_digest: stored.manifest_digest,
        artifact: CandidateArtifact {
            artifact_id: stored.artifact.artifact_id,
            digest: stored.artifact.digest,
            size_bytes: stored.artifact.size_bytes,
            media_type: stored.artifact.media_type,
        },
    }
}

struct Fixture {
    _scratch: Scratch,
    port: ValidationPort,
    candidate: CandidateRef,
    runs: Arc<AtomicU64>,
}

fn fixture(name: &str, probe: Probe) -> Fixture {
    let scratch = Scratch::new(name);
    let candidates = scratch.dir().join("candidates");
    let store = scratch.dir().join("validation");
    let candidate = write_candidate(&candidates, "cand-cli-dfm");
    let runs = Arc::new(AtomicU64::new(0));
    let port = ValidationPort::open_with(
        store,
        candidates,
        Arc::new(FixedRunner { identity: LocalRunner::identity(), probe, runs: runs.clone() }),
        Limits::default(),
        Arc::new(SystemClock),
    )
    .unwrap();
    Fixture { _scratch: scratch, port, candidate, runs }
}

fn run_check(fixture: &Fixture, billing: &mut dyn BillingPort) -> Result<ControlledCheck, String> {
    let operation = OperationId::parse("op_cli_dfm_test_0001").unwrap();
    run_controlled_check(
        &fixture.port,
        &fixture.candidate,
        dfm_profile(),
        &generate_submission_key(now_ms()),
        "local",
        &operation,
        &verified_budget(),
        billing,
    )
}

#[test]
fn dfm_report_charges_once_even_with_violations() {
    let fixture = fixture("violations", Probe::Violations);
    let mut billing = CountingBilling::default();
    let result = run_check(&fixture, &mut billing).expect("the mixed check produced its report");
    assert_eq!(result.state, solidcraft_engine::validation::ValidationState::QualityFailed);
    assert!(result.report_produced, "a quality failure is still a produced report");
    assert_eq!(billing.admits, 1);
    assert_eq!(billing.commits, 1, "violations do not make the check free");
    assert_eq!(billing.releases, 0);
}

#[test]
fn cli_dfm_zero_violation_report_commits_once() {
    let fixture = fixture("zero", Probe::Passed);
    let mut billing = CountingBilling::default();
    let result = run_check(&fixture, &mut billing).expect("the mixed check produced its report");
    assert!(result.report_produced);
    assert_eq!(billing.admits, 1);
    assert_eq!(billing.commits, 1);
    assert_eq!(billing.releases, 0);
}

#[test]
fn dfm_failure_does_not_charge() {
    let fixture = fixture("failure", Probe::Infrastructure);
    let mut billing = CountingBilling::default();
    let result = run_check(&fixture, &mut billing).expect("an infrastructure report is not a CLI error");
    assert!(!result.report_produced);
    assert_eq!(billing.admits, 1);
    assert_eq!(billing.commits, 0);
    assert_eq!(billing.releases, 1, "the held admission is released exactly once");
}

#[test]
fn mixed_check_without_a_verified_bound_is_refused_before_admission() {
    let fixture = fixture("no-bound", Probe::Passed);
    let mut billing = CountingBilling::default();
    let operation = OperationId::parse("op_cli_dfm_nobound_0001").unwrap();
    let error = run_controlled_check(
        &fixture.port,
        &fixture.candidate,
        dfm_profile(),
        &generate_submission_key(now_ms()),
        "local",
        &operation,
        &BudgetTable::unverified_default(),
        &mut billing,
    )
    .unwrap_err();
    assert!(error.starts_with("UNSAFE_TO_START:"), "{error}");
    assert_eq!((billing.admits, billing.commits, billing.releases), (0, 0, 0), "no grant was requested and no work ran");
}

#[test]
fn admission_refusal_is_refused_whole_without_a_run() {
    let fixture = fixture("denied", Probe::Passed);
    let mut billing = CountingBilling::denying();
    let error = run_check(&fixture, &mut billing).unwrap_err();
    assert!(error.starts_with("AUTH_REQUIRED:"), "{error}");
    assert_eq!(billing.admits, 1);
    assert_eq!((billing.commits, billing.releases), (0, 0), "a refused admission has no hold to settle");
}

#[test]
fn classification_comes_from_the_resolved_profile_not_its_name() {
    let plain = solidcraft_engine::validation::native_basic_profile();
    assert_eq!(profile_scope(&plain), ProfileScope::Free);
    assert_eq!(profile_scope(&dfm_profile()), ProfileScope::Dfm, "the dfm.* check id is enough");
    let mut misleading = dfm_profile();
    misleading.profile_id = "native-basic".into();
    assert_eq!(profile_scope(&misleading), ProfileScope::Dfm, "a free-looking profile name cannot hide DFM work");
}

/// Without a product session the whole mixed request is refused.
#[test]
fn binary_refuses_an_unauthorized_mixed_check_whole() {
    let scratch = Scratch::new("binary-refusal");
    let candidates = scratch.dir().join("candidates");
    let validation = scratch.dir().join("validation");
    let script = scratch.dir().join("capture.json");
    let results = scratch.dir().join("results.json");
    std::fs::write(
        &script,
        json!({"commands": [
            {"command": "solid.box", "params": {"length": 20, "width": 20, "height": 10}},
            {"command": "document.snapshot.capture", "params": {}},
        ]})
        .to_string(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_intcad-cli"))
        .arg("run")
        .arg(&script)
        .args(["--results", "--json-out"])
        .arg(&results)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &candidates)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let replies: Value = serde_json::from_slice(&std::fs::read(&results).unwrap()).unwrap();
    let candidate = replies.as_array().unwrap().last().unwrap()["candidate"].clone();

    let profile = dfm_profile();
    let reference = solidcraft_engine::validation::profile_ref(&profile).unwrap();
    let profile_path = scratch.dir().join("profile.json");
    std::fs::write(&profile_path, serde_json::to_string_pretty(&profile).unwrap()).unwrap();
    let request = json!({"candidate": candidate, "profile": reference, "requestId": generate_submission_key(now_ms())});
    let request_path = scratch.dir().join("request.json");
    std::fs::write(&request_path, serde_json::to_string_pretty(&request).unwrap()).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_intcad-cli"))
        .args(["validate", "submit"])
        .arg(&request_path)
        .args(["--profile"])
        .arg(&profile_path)
        .args(["--candidate-dir"])
        .arg(&candidates)
        .args(["--validation-dir"])
        .arg(&validation)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &candidates)
        .env("SOLIDCRAFT_VALIDATION_DIR", &validation)
        .env("XDG_DATA_HOME", scratch.dir())
        .env("APPDATA", scratch.dir())
        .env("HOME", scratch.dir())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("AUTH_UNAVAILABLE"), "{stderr}");
    // Refusal happens before admission: no run record is created for this request.
    let runs = validation.join("runs");
    let created = runs.is_dir() && std::fs::read_dir(&runs).map(|entries| entries.count() > 0).unwrap_or(false);
    assert!(!created, "the mixed request must not be queued without authority");
}

/// Ordinary validation still runs with no installed authority, so plain requests stay free.
#[test]
fn binary_plain_validate_is_free_without_authority() {
    let scratch = Scratch::new("plain-free");
    let candidates = scratch.dir().join("candidates");
    let validation = scratch.dir().join("validation");
    let script = scratch.dir().join("capture.json");
    let results = scratch.dir().join("results.json");
    std::fs::write(
        &script,
        json!({"commands": [
            {"command": "solid.box", "params": {"length": 20, "width": 20, "height": 10}},
            {"command": "document.snapshot.capture", "params": {}},
        ]})
        .to_string(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_intcad-cli"))
        .arg("run")
        .arg(&script)
        .args(["--results", "--json-out"])
        .arg(&results)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &candidates)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let replies: Value = serde_json::from_slice(&std::fs::read(&results).unwrap()).unwrap();
    let candidate = replies.as_array().unwrap().last().unwrap()["candidate"].clone();
    let profile = solidcraft_engine::validation::native_basic_profile();
    let reference = solidcraft_engine::validation::profile_ref(&profile).unwrap();
    let request = json!({"candidate": candidate, "profile": reference, "requestId": generate_submission_key(now_ms())});
    let request_path = scratch.dir().join("request.json");
    std::fs::write(&request_path, serde_json::to_string_pretty(&request).unwrap()).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_intcad-cli"))
        .args(["validate", "submit"])
        .arg(&request_path)
        .args(["--candidate-dir"])
        .arg(&candidates)
        .args(["--validation-dir"])
        .arg(&validation)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &candidates)
        .env("SOLIDCRAFT_VALIDATION_DIR", &validation)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(matches!(status["state"].as_str(), Some("passed" | "quality_failed")), "{status}");
}

// `NoAuthorityBilling` is public so the integration seam and the binary use the same fail-closed
// authority; this keeps a production fake from being introduced just to satisfy a test.
#[test]
fn no_authority_billing_never_grants() {
    let mut billing = NoAuthorityBilling;
    let operation = OperationId::parse("op_cli_dfm_noauth_0001").unwrap();
    let error = billing.admit(&operation, Capability::Dfm, EntryCategory::CliDfm).unwrap_err();
    assert_eq!(error.code, RefusalCode::AuthUnavailable);
    assert_eq!(billing.release(&operation, RefusalCode::UnsafeToStart), Ok(()));
}

#[derive(Default)]
struct FakeKeyring {
    entries: BTreeMap<String, String>,
    read_delay: Option<Arc<ReadDelay>>,
}

#[derive(Default)]
struct ReadDelay {
    binding_reads: AtomicU64,
    time: AtomicU64,
}

impl CredentialStore for FakeKeyring {
    fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
        if key == "auth/session_binding"
            && let Some(delay) = &self.read_delay
        {
            let reads = delay.binding_reads.load(Ordering::Relaxed);
            delay.binding_reads.store(reads.saturating_sub(1), Ordering::Relaxed);
            if reads == 1 {
                delay.time.store(56_000, Ordering::Relaxed);
            }
        }
        Ok(self.entries.get(key).cloned())
    }
    fn set(&mut self, key: &str, secret: &str) -> Result<(), CredentialError> {
        self.entries.insert(key.into(), secret.into());
        Ok(())
    }
    fn delete(&mut self, key: &str) -> Result<(), CredentialError> {
        self.entries.remove(key);
        Ok(())
    }
    fn persistent(&self) -> bool {
        true
    }
}

struct FixtureExchange;

impl ProductSessionExchange for FixtureExchange {
    fn exchange(&self, _: &ProviderSession) -> Result<ProductSession, ProductExchangeError> {
        Ok(ProductSession::from_trusted_exchange("acct_cli_fixture", Secret::new("product-fixture"), None))
    }
}

fn fixture_session() -> CredentialSession {
    fixture_session_with_store(FakeKeyring::default())
}

fn fixture_session_with_store(store: FakeKeyring) -> CredentialSession {
    let mut session = CredentialSession::new(Box::new(store));
    session.complete_provider_login("provider_fixture", "provider-fixture").unwrap();
    session.activate_product_session(&FixtureExchange).unwrap();
    solidcraft_account_native::CredentialHandoff::installation_credential(&mut session, "ins_cli_fixture", Secret::new("installation-fixture"))
        .unwrap();
    session
}

#[derive(Clone, Default)]
struct WireClock(Rc<Cell<u64>>);

impl MonotonicClock for WireClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::from_millis(self.0.get())
    }

    fn wall_now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }
}

enum Step {
    Status,
    StatusThen(Box<dyn FnOnce()>),
    WrongAccount(Box<dyn FnOnce()>),
    Denied,
    DeniedThen(Box<dyn FnOnce()>),
    Withdraw(CredentialSession),
    Admit,
    BadGrant,
    LoseAdmit,
    RecoverAdmit,
    Renew,
    RenewThen(u16, Box<dyn FnOnce()>),
    Finalize(BillingResult),
    InspectFinalize(Box<dyn FnOnce()>),
    LoseFinalize,
    Lookup,
    LookupHeld,
}

#[derive(Clone)]
struct Script {
    steps: Rc<RefCell<VecDeque<Step>>>,
    operation: Rc<RefCell<Option<OperationId>>>,
    capability: Capability,
    entry: EntryCategory,
    authorization: &'static str,
}

impl Script {
    fn new(capability: Capability, entry: EntryCategory, steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            steps: Rc::new(RefCell::new(steps.into_iter().collect())),
            operation: Rc::default(),
            capability,
            entry,
            authorization: "Bearer product-fixture",
        }
    }

    fn expecting_rotated_bearer(mut self) -> Self {
        self.authorization = "Bearer rotated-product-fixture";
        self
    }

    fn operation(&self) -> OperationId {
        self.operation.borrow().clone().expect("admission observed")
    }

    fn done(&self) {
        assert!(self.steps.borrow().is_empty(), "unconsumed product requests");
    }
}

impl Transport for Script {
    fn send(&self, request: &WireRequest) -> Result<WireResponse, TransportFailure> {
        let step = self.steps.borrow_mut().pop_front().expect("unexpected product request");
        assert_eq!(request.header("X-Protocol-Version"), Some("intcad-auth/1.0"));
        let url = request.url().strip_prefix("https://test.invalid").unwrap();
        let body = || serde_json::from_slice::<Value>(request.body().unwrap()).unwrap();
        let (status, response) = match step {
            Step::Status | Step::StatusThen(_) | Step::Denied | Step::DeniedThen(_) | Step::Withdraw(_) | Step::WrongAccount(_) => {
                assert_eq!((request.method(), url), ("GET", "/v1/status"));
                assert_eq!(request.header("Authorization"), Some(self.authorization));
                assert_eq!(request.header("X-Installation-Credential"), None);
                let account = if matches!(step, Step::WrongAccount(_)) { "acct_other_fixture" } else { "acct_cli_fixture" };
                if let Step::StatusThen(after) | Step::WrongAccount(after) = step {
                    after();
                } else if let Step::Withdraw(session) = step {
                    session.revoke_in_process();
                } else if let Step::DeniedThen(after) = step {
                    after();
                    return Ok(WireResponse::new(401, br#"{"error":{"code":"AUTH_REQUIRED"}}"#.to_vec()));
                } else if matches!(step, Step::Denied) {
                    return Ok(WireResponse::new(401, br#"{"error":{"code":"AUTH_REQUIRED"}}"#.to_vec()));
                }
                (
                    200,
                    json!({
                        "account":{"id":account,"status":"active","email_verified":true},"eligible":true,"reasons":[],
                        "balance":10,"held":0,"available":10,"client":{"supported":true},
                        "protocol":{"supported":true,"version":"intcad-auth/1.0"},"capabilities":{"chat":true,"mcp":true,"dfm":true}
                    }),
                )
            }
            Step::Admit | Step::BadGrant | Step::LoseAdmit | Step::RecoverAdmit => {
                assert_eq!((request.method(), url), ("POST", "/v1/operations"));
                assert_eq!(request.header("Authorization"), Some(self.authorization));
                assert_eq!(request.header("X-Installation-Credential"), Some("installation-fixture"));
                let data = body();
                let id = OperationId::parse(data["operation_id"].as_str().unwrap()).unwrap();
                assert_eq!(
                    data,
                    json!({
                        "operation_id":id.as_str(),"capability":self.capability.wire(),
                        "entry_category":self.entry.wire_entry_category().unwrap().wire()
                    })
                );
                if matches!(step, Step::RecoverAdmit) {
                    assert_eq!(self.operation(), id);
                    return Ok(WireResponse::new(
                        200,
                        json!({
                            "outcome":"ALREADY_EXISTS","operation_id":id.as_str(),"installation_id":"ins_cli_fixture","state":"HELD",
                            "terminal":false,"settlement_credential":"settlement-fixture"
                        })
                        .to_string()
                        .into_bytes(),
                    ));
                }
                *self.operation.borrow_mut() = Some(id.clone());
                if matches!(step, Step::LoseAdmit) {
                    return Err(TransportFailure::Timeout);
                }
                let expiry = if matches!(step, Step::BadGrant) { "unparseable" } else { "2027-01-15T08:01:00Z" };
                (
                    200,
                    json!({
                        "outcome":"NEWLY_ADMITTED","operation_id":id.as_str(),"installation_id":"ins_cli_fixture","state":"HELD",
                        "settlement_deadline":null,"authorization_expires_at":"2027-01-15T08:01:00Z","settlement_credential":"settlement-fixture",
                        "execution_grant":{"grant_id":"grant_cli_fixture","capability":self.capability.wire(),
                            "expires_at":expiry,"session_proof":"proof-fixture","renew_after_seconds":20}
                    }),
                )
            }
            Step::Renew | Step::RenewThen(_, _) => {
                assert_eq!((request.method(), url), ("POST", "/v1/authorizations"));
                assert_eq!(request.header("Authorization"), Some(self.authorization));
                assert_eq!(request.header("X-Execution-Session-Proof"), Some("proof-fixture"));
                assert_eq!(request.header("X-Settlement-Credential"), None);
                let id = self.operation();
                assert_eq!(
                    body(),
                    json!({
                        "operation_id":id.as_str(),"capability":self.capability.wire(),"renewal":{"authorization_id":"grant_cli_fixture"}
                    })
                );
                if let Step::RenewThen(status, after) = step {
                    after();
                    if status != 200 {
                        return Ok(WireResponse::new(status, br#"{"error":{"code":"AUTH_REQUIRED"}}"#.to_vec()));
                    }
                }
                (
                    200,
                    json!({
                        "operation_id":id.as_str(),"capability":self.capability.wire(),"authorization_id":"grant_cli_fixture",
                        "remaining_seconds":60,"renewed":true,"execution_session_proof":null,"renew_after_seconds":20
                    }),
                )
            }
            Step::Finalize(_) | Step::LoseFinalize | Step::InspectFinalize(_) => {
                let (result, lose) = match step {
                    Step::Finalize(result) => (result, false),
                    Step::InspectFinalize(inspect) => {
                        inspect();
                        (BillingResult::Succeeded, false)
                    }
                    _ => (BillingResult::Succeeded, true),
                };
                let id = self.operation();
                assert_eq!(request.method(), "POST");
                assert_eq!(url, format!("/v1/operations/{}/finalize", id.as_str()));
                assert_eq!(request.header("X-Settlement-Credential"), Some("settlement-fixture"));
                assert_eq!(request.header("Authorization"), None);
                assert_eq!(request.header("X-Execution-Session-Proof"), None);
                let wire = match result {
                    BillingResult::Succeeded => "SUCCEEDED",
                    BillingResult::FailedNoEffect => "FAILED_NO_EFFECT",
                    _ => panic!("unplanned settlement"),
                };
                assert_eq!(body(), json!({"result":wire}));
                if lose {
                    return Err(TransportFailure::Timeout);
                }
                let state = if result == BillingResult::FailedNoEffect { "RELEASED" } else { "CHARGED" };
                (
                    200,
                    json!({
                    "operation_id":id.as_str(),"state":state,"terminal":true,"result":wire,
                        "settlement_deadline":"2027-01-15T08:11:00Z","finalized_at":"2027-01-15T08:00:01Z"
                    }),
                )
            }
            Step::Lookup | Step::LookupHeld => {
                let id = self.operation();
                assert_eq!(request.method(), "GET");
                assert_eq!(url, format!("/v1/operations/{}", id.as_str()));
                assert_eq!(request.header("X-Settlement-Credential"), Some("settlement-fixture"));
                assert_eq!(request.header("Authorization"), None);
                let held = matches!(step, Step::LookupHeld);
                (
                    200,
                    json!({
                        "operation_id":id.as_str(),"state":if held { "HELD" } else { "CHARGED" },
                        "terminal":!held,"result":if held { Value::Null } else { json!("SUCCEEDED") },
                        "authorization_expires_at":"2027-01-15T08:01:00Z",
                        "settlement_deadline":if held { Value::Null } else { json!("2027-01-15T08:11:00Z") }
                    }),
                )
            }
        };
        Ok(WireResponse::new(status, response.to_string().into_bytes()))
    }
}

fn live_billing(session: CredentialSession, script: Script, journal: PathBuf, clock: impl MonotonicClock + 'static) -> SessionBilling {
    SessionBilling::new(session, WireConnection::new("https://test.invalid", "test").unwrap(), Box::new(script), journal, clock)
}

#[derive(Clone, Default)]
struct IdentityProbe {
    seconds: Arc<AtomicU64>,
    offline: Arc<AtomicBool>,
    signed_out: Arc<AtomicBool>,
    requests: Arc<AtomicU64>,
}

impl IdentityClock for IdentityProbe {
    fn now_unix_seconds(&self) -> u64 {
        1_800_000_000 + self.seconds.load(Ordering::Relaxed)
    }

    fn monotonic_now(&self) -> Duration {
        Duration::from_secs(self.seconds.load(Ordering::Relaxed))
    }
}

impl IdentityProbe {
    fn expire_during_status(&self) -> Step {
        let seconds = self.seconds.clone();
        Step::StatusThen(Box::new(move || seconds.store(620, Ordering::Relaxed)))
    }

    fn recover(&self) {
        self.offline.store(false, Ordering::Relaxed);
        self.seconds.store(660, Ordering::Relaxed);
    }
}

fn fixture_identity(session: &CredentialSession, root: &Path, offline: bool) -> IdentityLifecycle {
    let probe = IdentityProbe::default();
    probe.offline.store(offline, Ordering::Relaxed);
    scripted_identity(session, root, probe)
}

fn scripted_identity(session: &CredentialSession, root: &Path, probe: IdentityProbe) -> IdentityLifecycle {
    struct Refresh(IdentityProbe);
    impl IdentityTransport for Refresh {
        fn refresh(&self, _: &str, _: &Secret) -> Result<RefreshResponse, RefreshTransportError> {
            self.0.requests.fetch_add(1, Ordering::Relaxed);
            if self.0.signed_out.load(Ordering::Relaxed) {
                return Err(RefreshTransportError::Terminal);
            }
            if self.0.offline.load(Ordering::Relaxed) {
                return Err(RefreshTransportError::Transient { retry_after: None });
            }
            Ok(RefreshResponse {
                access_token: Secret::new("rotated-product-fixture"),
                refresh_token: Secret::new("rotated-refresh-fixture"),
                provider_account_id: "provider_fixture".into(),
                expires_in: Some(Duration::from_secs(600)),
            })
        }
    }

    struct Verifier;
    impl SessionVerifier for Verifier {
        fn verify(&self, access: &Secret) -> Result<VerifiedSession, VerifyError> {
            assert!(access == &Secret::new("rotated-product-fixture"));
            Ok(VerifiedSession { account_id: "acct_cli_fixture".into(), same_login_binding: None })
        }
    }

    let connection = WireConnection::new("https://test.invalid", "test").unwrap();
    let mut identity = IdentityLifecycle::new_with_transport(
        IdentityDomain::new("desktop", "client_fixture", connection.base_url()).unwrap(),
        Box::new(FakeKeyring::default()),
        connection,
        Box::new(Refresh(probe.clone())),
        Box::new(Verifier),
        Box::new(probe.clone()),
        root.to_owned(),
    )
    .unwrap();
    identity.set_refresh_preserves_binding(true);
    let now = probe.now_unix_seconds();
    let scope = session.authenticated_scope().unwrap();
    identity
        .adopt_verified_login(
            "provider_fixture",
            scope.account().as_str(),
            Secret::new("product-fixture"),
            Secret::new("refresh-fixture"),
            now - 60,
            now - 1,
            scope.session_binding().as_str(),
        )
        .unwrap();
    identity
}

#[test]
fn dfm_charge_uses_the_refreshed_bearer() {
    let fixture = fixture("identity-dfm", Probe::Passed);
    let root = fixture._scratch.dir();
    let journal = root.join("journal");
    let session = fixture_session();
    let identity = fixture_identity(&session, root, false);
    let script = Script::new(
        Capability::Dfm,
        EntryCategory::CliDfm,
        [Step::Status, Step::Admit, Step::Status, Step::Status, Step::Finalize(BillingResult::Succeeded)],
    )
    .expecting_rotated_bearer();
    let mut billing = live_billing(session, script.clone(), journal.clone(), WireClock::default()).with_identity(identity);
    run_check(&fixture, &mut billing).unwrap();
    assert_eq!(FileOperationStore::open(journal).unwrap().load(&script.operation()).unwrap().unwrap().status, OperationStatus::Charged);
    script.done();
}

#[test]
fn failed_identity_refresh_stops_dfm_before_product_or_ledger_work() {
    let fixture = fixture("identity-offline", Probe::Passed);
    let root = fixture._scratch.dir();
    let journal = root.join("journal");
    let session = fixture_session();
    let identity = fixture_identity(&session, root, true);
    let script = Script::new(Capability::Dfm, EntryCategory::CliDfm, []);
    let mut billing = live_billing(session, script.clone(), journal.clone(), WireClock::default()).with_identity(identity);
    assert!(run_check(&fixture, &mut billing).unwrap_err().starts_with("AUTH_UNAVAILABLE:"));
    assert!(!journal.exists());
    assert_eq!(std::fs::read_dir(root.join("validation/runs")).unwrap().count(), 0);
    script.done();
}

#[test]
fn dfm_commit_identity_pause_keeps_the_hold_for_same_operation_settlement() {
    let fixture = fixture("paused-dfm", Probe::Passed);
    let root = fixture._scratch.dir();
    let journal = root.join("journal");
    let session = fixture_session();
    let probe = IdentityProbe::default();
    let identity = scripted_identity(&session, root, probe.clone());
    let script = Script::new(
        Capability::Dfm,
        EntryCategory::CliDfm,
        [Step::Status, Step::Admit, Step::Status, probe.expire_during_status(), Step::Status, Step::Finalize(BillingResult::Succeeded)],
    )
    .expecting_rotated_bearer();
    let mut billing = live_billing(session, script.clone(), journal.clone(), WireClock::default()).with_identity(identity);
    let error = run_check(&fixture, &mut billing).unwrap_err();
    assert!(error.starts_with("AUTH_UNAVAILABLE:"), "{error}");
    let operation = script.operation();
    assert!(error.contains(operation.as_str()));
    let ledger = FileOperationStore::open(&journal).unwrap();
    assert!(!ledger.load(&operation).unwrap().unwrap().status.is_terminal());
    assert_eq!(ledger.settlement_intent(&operation).unwrap(), None);

    // Recovery uses the retained admission and never calls run_controlled_check again.
    probe.recover();
    billing.check_commit(&operation).unwrap();
    billing.commit(&operation, BillingResult::Succeeded).unwrap();
    assert_eq!(ledger.load(&operation).unwrap().unwrap().status, OperationStatus::Charged);
    assert_eq!(fixture.runs.load(Ordering::Relaxed), 1);
    script.done();
}

#[test]
fn mcp_pending_resumes_the_original_gate_without_replaying_the_tool() {
    for pause_at_dispatch in [true, false] {
        let scratch = Scratch::new("paused-mcp");
        let journal = scratch.dir().join("journal");
        let session = fixture_session();
        let probe = IdentityProbe::default();
        let identity = scripted_identity(&session, scratch.dir(), probe.clone());
        let script = Script::new(
            Capability::Mcp,
            EntryCategory::HeadlessMcp,
            if pause_at_dispatch {
                vec![Step::Status, Step::Admit, probe.expire_during_status(), Step::Status, Step::Status, Step::Finalize(BillingResult::Succeeded)]
            } else {
                vec![Step::Status, Step::Admit, Step::Status, probe.expire_during_status(), Step::Status, Step::Finalize(BillingResult::Succeeded)]
            },
        )
        .expecting_rotated_bearer();
        let billing = live_billing(session, script.clone(), journal.clone(), WireClock::default()).with_identity(identity);
        let mut budget = BudgetTable::unverified_default();
        budget.set_verified(Capability::Mcp, EntryCategory::HeadlessMcp, Duration::from_secs(1));
        let mut server =
            compose_mcp_server(Box::new(solidcraft_mcp::Headless::default()), None, McpAccount::from_session(billing), McpRoute::Headless, budget);
        let send = |server: &mut solidcraft_mcp::Server, name, arguments| {
            let line = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}).to_string();
            serde_json::from_str::<Value>(&server.handle_line(&line).unwrap()).unwrap()["result"].clone()
        };
        let destination = scratch.dir().join("paused.sc");
        let result = send(&mut server, "save", json!({"path":destination}));
        let operation = script.operation();
        assert_eq!(result["structuredContent"]["errorDetails"]["reason"], "AUTH_UNAVAILABLE", "{result}");
        assert_eq!(result["structuredContent"]["operationId"], operation.as_str());
        assert_eq!(destination.exists(), !pause_at_dispatch);
        let ledger = FileOperationStore::open(&journal).unwrap();
        assert_eq!(ledger.settlement_intent(&operation).unwrap(), None);
        let blocked = send(&mut server, "measure", json!({}));
        assert_eq!(blocked["structuredContent"]["operationId"], operation.as_str(), "new calls cannot replace a pending operation");
        if !pause_at_dispatch {
            std::fs::remove_file(&destination).unwrap();
        }
        probe.offline.store(true, Ordering::Relaxed);
        let still_paused = send(&mut server, "pending", json!({}));
        assert_eq!(still_paused["structuredContent"]["operationId"], operation.as_str());
        assert_eq!(still_paused["structuredContent"]["errorDetails"]["reason"], "AUTH_UNAVAILABLE");
        assert_eq!(ledger.settlement_intent(&operation).unwrap(), None);
        probe.recover();
        let result = send(&mut server, "pending", json!({}));
        assert_ne!(result["isError"], true, "{result}");
        assert_eq!(destination.exists(), pause_at_dispatch, "a completed save must never be replayed");
        assert_eq!(ledger.load(&operation).unwrap().unwrap().status, OperationStatus::Charged);
        script.done();
    }
}

#[test]
fn active_identity_recovery_never_sends_with_less_than_one_request_left() {
    let scratch = Scratch::new("short-identity");
    let session = fixture_session();
    let probe = IdentityProbe::default();
    let identity = scripted_identity(&session, scratch.dir(), probe.clone());
    let script = Script::new(Capability::Dfm, EntryCategory::CliDfm, [Step::Status, Step::Admit]).expecting_rotated_bearer();
    let clock = WireClock::default();
    let mut billing = live_billing(session, script.clone(), scratch.dir().join("journal"), clock.clone()).with_identity(identity);
    let operation = OperationId::parse("op_cli_short_window").unwrap();
    billing.admit(&operation, Capability::Dfm, EntryCategory::CliDfm).unwrap();
    probe.recover(); // The original identity is no longer fresh; a refresh would be sent.
    clock.0.set(56_000);
    assert_eq!(billing.dispatch(&operation, Duration::from_secs(1)).unwrap_err().code, RefusalCode::AuthUnavailable);
    assert_eq!(probe.requests.load(Ordering::Relaxed), 1, "only the pre-admission refresh was sent");
    clock.0.set(60_000);
    assert_eq!(billing.dispatch(&operation, Duration::from_secs(1)).unwrap_err().code, RefusalCode::UnsafeToStart);
    script.done();
}

#[test]
fn renewal_uses_the_candidate_window_but_never_masks_unauthorized() {
    for (status, close_readiness) in [(200, false), (401, true), (200, true)] {
        let scratch = Scratch::new("renewal-final-gate");
        let session = fixture_session();
        let probe = IdentityProbe::default();
        let identity = scripted_identity(&session, scratch.dir(), probe.clone());
        let clock = WireClock::default();
        let after = {
            let clock = clock.clone();
            let probe = probe.clone();
            move || {
                clock.0.set(56_000);
                if close_readiness {
                    probe.seconds.store(620, Ordering::Relaxed);
                }
            }
        };
        let mut steps = vec![Step::Status, Step::Admit, Step::Status, Step::RenewThen(status, Box::new(after))];
        if !close_readiness {
            steps.push(Step::Status);
        }
        let script = Script::new(Capability::Dfm, EntryCategory::CliDfm, steps).expecting_rotated_bearer();
        let mut billing = live_billing(session, script.clone(), scratch.dir().join("journal"), clock.clone()).with_identity(identity);
        let operation = OperationId::parse("op_cli_renewal_final_gate").unwrap();
        billing.admit(&operation, Capability::Dfm, EntryCategory::CliDfm).unwrap();
        clock.0.set(54_000);
        let result = billing.dispatch(&operation, Duration::from_secs(10));
        if status == 401 {
            assert_eq!(result.unwrap_err().code, RefusalCode::AuthRequired);
            assert_eq!(billing.dispatch(&operation, Duration::ZERO).unwrap_err().code, RefusalCode::AuthRequired);
        } else if close_readiness {
            assert_eq!(result.unwrap_err().code, RefusalCode::AuthUnavailable);
            clock.0.set(60_000);
            assert_eq!(billing.dispatch(&operation, Duration::ZERO).unwrap_err().code, RefusalCode::UnsafeToStart);
        } else {
            result.unwrap();
            clock.0.set(61_000);
            billing.check_commit(&operation).unwrap();
        }
        script.done();
    }
}

#[test]
fn wrong_account_status_releases_without_revalidating_identity() {
    let fixture = fixture("wrong-status-account", Probe::Passed);
    let root = fixture._scratch.dir();
    let session = fixture_session();
    let probe = IdentityProbe::default();
    let identity = scripted_identity(&session, root, probe.clone());
    let expire = {
        let probe = probe.clone();
        move || {
            probe.seconds.store(620, Ordering::Relaxed);
            probe.offline.store(true, Ordering::Relaxed);
        }
    };
    let script = Script::new(
        Capability::Dfm,
        EntryCategory::CliDfm,
        [Step::Status, Step::Admit, Step::WrongAccount(Box::new(expire)), Step::Finalize(BillingResult::FailedNoEffect)],
    )
    .expecting_rotated_bearer();
    let journal = root.join("journal");
    let mut billing = live_billing(session, script.clone(), journal.clone(), WireClock::default()).with_identity(identity);
    assert!(run_check(&fixture, &mut billing).unwrap_err().starts_with("AUTH_REQUIRED:"));
    assert_eq!(probe.requests.load(Ordering::Relaxed), 1, "only the initial refresh");
    assert_eq!(fixture.runs.load(Ordering::Relaxed), 0);
    assert_eq!(FileOperationStore::open(journal).unwrap().load(&script.operation()).unwrap().unwrap().status, OperationStatus::Released);
    script.done();
}

#[test]
fn dfm_status_401_during_expiry_retries_once_or_preserves_the_correct_disposition() {
    for (terminal, offline) in [(false, false), (true, false), (false, true)] {
        let fixture = fixture("status-401", Probe::Passed);
        let root = fixture._scratch.dir();
        let journal = root.join("journal");
        let session = fixture_session();
        let probe = IdentityProbe::default();
        let identity = scripted_identity(&session, root, probe.clone());
        let mut steps = vec![
            Step::Status,
            Step::Admit,
            Step::DeniedThen(Box::new({
                let probe = probe.clone();
                move || {
                    probe.seconds.store(620, Ordering::Relaxed);
                    probe.signed_out.store(terminal, Ordering::Relaxed);
                    probe.offline.store(offline, Ordering::Relaxed);
                }
            })),
        ];
        if terminal {
            steps.push(Step::Finalize(BillingResult::FailedNoEffect));
        } else if offline {
            steps.push(Step::Status);
        } else {
            steps.extend([Step::Status, Step::Status, Step::Finalize(BillingResult::Succeeded)]);
        }
        let script = Script::new(Capability::Dfm, EntryCategory::CliDfm, steps).expecting_rotated_bearer();
        let mut billing = live_billing(session, script.clone(), journal.clone(), WireClock::default()).with_identity(identity);
        let result = run_check(&fixture, &mut billing);
        let ledger = FileOperationStore::open(journal).unwrap();
        let operation = script.operation();
        let status = ledger.load(&operation).unwrap().unwrap().status;
        if terminal {
            assert!(result.unwrap_err().starts_with("AUTH_REQUIRED:"));
            assert_eq!(status, OperationStatus::Released);
        } else if offline {
            assert!(result.unwrap_err().starts_with("AUTH_UNAVAILABLE:"));
            assert_eq!(ledger.settlement_intent(&operation).unwrap(), None);
            assert!(!status.is_terminal());
            assert_eq!(billing.dispatch(&operation, Duration::from_secs(1)).unwrap_err().code, RefusalCode::AuthUnavailable);
            assert_eq!(probe.requests.load(Ordering::Relaxed), 2, "an immediate retry honors cooldown");
            probe.recover();
            billing.dispatch(&operation, Duration::from_secs(1)).unwrap();
        } else {
            assert!(result.unwrap().report_produced);
            assert_eq!(status, OperationStatus::Charged);
        }
        assert_eq!(probe.requests.load(Ordering::Relaxed), if offline { 3 } else { 2 });
        assert_eq!(fixture.runs.load(Ordering::Relaxed), u64::from(!terminal && !offline));
        script.done();
    }
}

#[test]
fn a_slow_credential_read_can_exhaust_the_status_or_renewal_send_window() {
    struct DelayedClock(Arc<ReadDelay>);
    impl MonotonicClock for DelayedClock {
        fn now(&self) -> MonotonicInstant {
            MonotonicInstant::from_millis(self.0.time.load(Ordering::Relaxed))
        }
        fn wall_now(&self) -> SystemTime {
            WireClock::default().wall_now()
        }
    }
    for during_renewal in [false, true] {
        let scratch = Scratch::new("slow-credential");
        let delay = Arc::new(ReadDelay::default());
        let session = fixture_session_with_store(FakeKeyring { read_delay: Some(delay.clone()), ..FakeKeyring::default() });
        let mut steps = vec![Step::Status, Step::Admit];
        if during_renewal {
            let delay = delay.clone();
            // Post-status binding validation reads three times. The next read
            // is in the renewal's credential lookup, after the early reserve check.
            steps.push(Step::StatusThen(Box::new(move || delay.binding_reads.store(4, Ordering::Relaxed))));
        }
        let script = Script::new(Capability::Dfm, EntryCategory::CliDfm, steps);
        let mut billing = live_billing(session, script.clone(), scratch.dir().join("journal"), DelayedClock(delay.clone()));
        let operation = OperationId::parse("op_cli_read_consumes_reserve").unwrap();
        billing.admit(&operation, Capability::Dfm, EntryCategory::CliDfm).unwrap();
        if during_renewal {
            delay.time.store(21_000, Ordering::Relaxed);
        } else {
            // Same three binding reads before the status credential lookup.
            delay.binding_reads.store(4, Ordering::Relaxed);
        }
        assert_eq!(billing.dispatch(&operation, Duration::from_secs(1)).unwrap_err().code, RefusalCode::AuthUnavailable);
        assert_eq!(delay.time.load(Ordering::Relaxed), 56_000);
        delay.time.store(60_000, Ordering::Relaxed);
        assert_eq!(billing.dispatch(&operation, Duration::from_secs(1)).unwrap_err().code, RefusalCode::UnsafeToStart);
        script.done();
    }
}

#[test]
fn server_revoked_session_is_refused_without_a_run_or_journal() {
    let fixture = fixture("revoked", Probe::Passed);
    let journal = fixture._scratch.dir().join("journal");
    let script = Script::new(Capability::Dfm, EntryCategory::CliDfm, [Step::Denied]);
    let mut billing = live_billing(fixture_session(), script.clone(), journal.clone(), WireClock::default());
    assert!(run_check(&fixture, &mut billing).unwrap_err().starts_with("AUTH_REQUIRED:"));
    assert!(!journal.exists());
    assert_eq!(std::fs::read_dir(fixture._scratch.dir().join("validation/runs")).unwrap().count(), 0);
    script.done();
}

#[test]
fn withdrawal_at_report_commit_releases_and_persists_only_the_refusal() {
    let fixture = fixture("withdrawal", Probe::Passed);
    let journal = fixture._scratch.dir().join("journal");
    let session = fixture_session();
    let script = Script::new(
        Capability::Dfm,
        EntryCategory::CliDfm,
        [Step::Status, Step::Admit, Step::Status, Step::Withdraw(session.clone()), Step::Finalize(BillingResult::FailedNoEffect)],
    );
    let mut billing = live_billing(session, script.clone(), journal.clone(), WireClock::default());
    let error = run_check(&fixture, &mut billing).unwrap_err();
    assert!(error.starts_with("AUTH_REQUIRED:"), "{error}");
    let run_id = error.rsplit_once('(').unwrap().1.trim_end_matches(')');
    let report = serde_json::to_value(fixture.port.report(run_id).unwrap()).unwrap();
    assert_eq!(report["report"]["state"], "infrastructure_error", "{report}");
    assert_eq!(report["report"]["error"]["details"]["reason"], "AUTH_REQUIRED", "{report}");
    assert!(!report.to_string().contains("\"fixture\":true"));
    assert_eq!(FileOperationStore::open(journal).unwrap().load(&script.operation()).unwrap().unwrap().status, OperationStatus::Released);
    script.done();
}

#[test]
fn scripted_product_grant_allows_validate_submit_and_one_settlement() {
    let scratch = Scratch::new("live-submit");
    let root = scratch.dir();
    let candidates = root.join("candidates");
    let snapshot = root.join("snapshot.json");
    let mut capture = std::process::Command::new(env!("CARGO_BIN_EXE_intcad-cli"));
    let recipe = root.join("capture.json");
    std::fs::write(
        &recipe,
        json!({"commands":[
            {"command":"sketch.create","params":{"plane":"XY","name":"Base"}},
            {"command":"sketch.rectangle.two_point","params":{"p0":[0,0],"p1":[20,20]}},
            {"command":"sketch.finish","params":{}},
            {"command":"sheet.flange","params":{"sketch":"Base","body_name":"Plate"}},
            {"command":"document.snapshot.capture","params":{}}
        ]})
        .to_string(),
    )
    .unwrap();
    let out = capture
        .arg("run")
        .arg(recipe)
        .args(["--results", "--json-out"])
        .arg(&snapshot)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &candidates)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let capture: Value = serde_json::from_slice(&std::fs::read(&snapshot).unwrap()).unwrap();
    let mut profile = solidcraft_engine::validation::native_basic_profile();
    profile.profile_id = "task-cli-dfm".into();
    profile.checks.insert(
        0,
        CheckRequirement {
            check_id: "dfm.sheet.flat_overlap".into(),
            kind: "dfm.sheet.flat_overlap".into(),
            required: true,
            parameters: json!({
                "modelVersion":"doc-1","sheets":[{"sheet":0,"body":"Plate"}],"toleranceMm2":0.01,
            }),
        },
    );
    let reference = solidcraft_engine::validation::profile_ref(&profile).unwrap();
    let request_path = root.join("request.json");
    let profile_path = root.join("profile.json");
    let validation = root.join("validation");
    let status_path = root.join("status.json");
    std::fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
    std::fs::write(
        &request_path,
        json!({
            "candidate":capture.as_array().unwrap().last().unwrap()["candidate"],"profile":reference,"requestId":generate_submission_key(now_ms())
        })
        .to_string(),
    )
    .unwrap();
    let args = vec![
        "submit".into(),
        request_path.display().to_string(),
        "--profile".into(),
        profile_path.display().to_string(),
        "--candidate-dir".into(),
        candidates.display().to_string(),
        "--validation-dir".into(),
        validation.display().to_string(),
        "--json-out".into(),
        status_path.display().to_string(),
    ];
    let denied = Script::new(Capability::Dfm, EntryCategory::CliDfm, [Step::Denied]);
    let billing = live_billing(fixture_session(), denied.clone(), root.join("journal"), WireClock::default());
    let error = run_with_billing(&args, &verified_budget(), || Some((billing, None))).unwrap_err();
    assert!(error.starts_with("AUTH_REQUIRED:"), "{error}");
    assert!(!validation.exists() && !status_path.exists() && !root.join("journal").exists());
    denied.done();

    let script = Script::new(
        Capability::Dfm,
        EntryCategory::CliDfm,
        [Step::Status, Step::Admit, Step::Status, Step::Status, Step::Finalize(BillingResult::Succeeded)],
    );
    let billing = live_billing(fixture_session(), script.clone(), root.join("journal"), WireClock::default());
    run_with_billing(&args, &verified_budget(), || Some((billing, None))).unwrap();
    let status: Value = serde_json::from_slice(&std::fs::read(&status_path).unwrap()).unwrap();
    let port = ValidationPort::open_read_only(validation, candidates).unwrap();
    let report = serde_json::to_value(port.report(status["runId"].as_str().unwrap()).unwrap()).unwrap();
    let dfm = report["report"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["checkId"] == "dfm.sheet.flat_overlap")
        .expect("DFM was not skipped");
    assert_eq!(dfm["state"], "passed", "{report}");
    assert!(!dfm["evidence"].as_array().unwrap().is_empty(), "{report}");
    script.done();
}

#[test]
fn retry_original_id_does_not_admit_execute_or_finalize_again() {
    let scratch = Scratch::new("same-id");
    let session = fixture_session();
    let journal = scratch.dir().join("journal");
    let clock = WireClock::default();
    let script = Script::new(
        Capability::Dfm,
        EntryCategory::CliDfm,
        [Step::Status, Step::Admit, Step::Status, Step::Renew, Step::Status, Step::Finalize(BillingResult::Succeeded), Step::Status],
    );
    let operation = OperationId::parse("op_cli_retry_original").unwrap();
    let mut billing = live_billing(session.clone(), script.clone(), journal.clone(), clock.clone());
    billing.admit(&operation, Capability::Dfm, EntryCategory::CliDfm).unwrap();
    clock.0.set(20_000);
    billing.dispatch(&operation, Duration::from_secs(1)).unwrap();
    assert_eq!(billing.commit(&operation, BillingResult::Succeeded), Ok(BillingResult::Succeeded));
    drop(billing);

    let mut reopened = live_billing(session, script.clone(), journal.clone(), clock);
    assert_eq!(reopened.admit(&operation, Capability::Dfm, EntryCategory::CliDfm).unwrap_err().code, RefusalCode::OperationConflict);
    assert_eq!(reopened.commit(&operation, BillingResult::Succeeded), Ok(BillingResult::Succeeded));
    assert_eq!(reopened.release(&operation, RefusalCode::UnsafeToStart).unwrap_err().code, RefusalCode::OperationFinalized);
    assert_eq!(FileOperationStore::open(journal).unwrap().load(&operation).unwrap().unwrap().status, OperationStatus::Charged);
    script.done();
}

#[test]
fn lost_finalize_is_resolved_by_receipt_without_a_second_finalize() {
    let scratch = Scratch::new("lost-receipt");
    let session = fixture_session();
    let journal = scratch.dir().join("journal");
    let script = Script::new(
        Capability::Dfm,
        EntryCategory::CliDfm,
        [Step::Status, Step::Admit, Step::Status, Step::Status, Step::LoseFinalize, Step::Lookup],
    );
    let operation = OperationId::parse("op_cli_lost_receipt").unwrap();
    let mut billing = live_billing(session.clone(), script.clone(), journal.clone(), WireClock::default());
    billing.admit(&operation, Capability::Dfm, EntryCategory::CliDfm).unwrap();
    billing.dispatch(&operation, Duration::from_secs(1)).unwrap();
    assert_eq!(billing.commit(&operation, BillingResult::Succeeded).unwrap_err().code, RefusalCode::AuthUnavailable);
    drop(billing);
    let mut reopened = live_billing(session, script.clone(), journal.clone(), WireClock::default());
    assert_eq!(reopened.commit(&operation, BillingResult::Succeeded), Ok(BillingResult::Succeeded));
    assert_eq!(FileOperationStore::open(journal).unwrap().load(&operation).unwrap().unwrap().status, OperationStatus::Charged);
    script.done();
}

#[test]
fn an_expired_execution_grant_cannot_commit_or_renew() {
    let scratch = Scratch::new("expired-grant");
    let clock = WireClock::default();
    let script =
        Script::new(Capability::Dfm, EntryCategory::CliDfm, [Step::Status, Step::Admit, Step::Status, Step::Finalize(BillingResult::FailedNoEffect)]);
    let operation = OperationId::parse("op_cli_expired_grant").unwrap();
    let mut billing = live_billing(fixture_session(), script.clone(), scratch.dir().join("journal"), clock.clone());
    billing.admit(&operation, Capability::Dfm, EntryCategory::CliDfm).unwrap();
    billing.dispatch(&operation, Duration::from_secs(1)).unwrap();
    clock.0.set(60_000);
    assert_eq!(billing.commit(&operation, BillingResult::Succeeded).unwrap_err().code, RefusalCode::UnsafeToStart);
    script.done();
}

#[test]
fn only_headless_mcp_dispatches_and_settles_via_the_credential_session() {
    let scratch = Scratch::new("mcp");
    let mut budget = BudgetTable::unverified_default();
    budget.set_bound(Capability::Mcp, EntryCategory::HeadlessMcp, UpperBound::Verified(Duration::from_secs(1)));
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"measure","arguments":{}}}).to_string();
    let journal = scratch.dir().join("journal");
    let offline = Script::new(Capability::Mcp, EntryCategory::HeadlessMcp, []);
    let account = McpAccount::from_session(live_billing(fixture_session(), offline.clone(), journal.clone(), WireClock::default()));
    let mut gui = compose_mcp_server(Box::new(solidcraft_mcp::Headless::default()), None, account, McpRoute::GuiBridge, budget.clone());
    let reply: Value = serde_json::from_str(&gui.handle_line(&request).unwrap()).unwrap();
    assert_eq!(reply["result"]["structuredContent"]["errorDetails"]["reason"], "AUTH_UNAVAILABLE");
    assert!(!journal.exists());
    offline.done();

    let script = Script::new(
        Capability::Mcp,
        EntryCategory::HeadlessMcp,
        (0..3).flat_map(|_| [Step::Status, Step::Admit, Step::Status, Step::Status, Step::Finalize(BillingResult::Succeeded)]),
    )
    .expecting_rotated_bearer();
    let session = fixture_session();
    let identity = fixture_identity(&session, scratch.dir(), false);
    let account = McpAccount::from_session(live_billing(session, script.clone(), journal, WireClock::default()).with_identity(identity));
    let mut headless = compose_mcp_server(Box::new(solidcraft_mcp::Headless::default()), None, account, McpRoute::Headless, budget);
    let reply: Value = serde_json::from_str(&headless.handle_line(&request).unwrap()).unwrap();
    assert_ne!(reply["result"]["isError"], true, "{reply}");
    assert!(reply["result"].is_object(), "{reply}");

    let batch = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"batch","arguments":{"commands":[
        {"command":"solid.box","params":{"length":20,"width":20,"height":10}}
    ]}}})
    .to_string();
    let reply: Value = serde_json::from_str(&headless.handle_line(&batch).unwrap()).unwrap();
    assert_ne!(reply["result"]["isError"], true, "{reply}");
    let destination = scratch.dir().join("saved.sc");
    let save = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"save","arguments":{"path":destination}}}).to_string();
    let reply: Value = serde_json::from_str(&headless.handle_line(&save).unwrap()).unwrap();
    assert_ne!(reply["result"]["isError"], true, "{reply}");
    assert!(destination.is_file());
    let destination = scratch.dir().join("unauthorized.png");
    let screenshot =
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"screenshot","arguments":{"path":destination}}}).to_string();
    let reply: Value = serde_json::from_str(&headless.handle_line(&screenshot).unwrap()).unwrap();
    assert_eq!(reply["result"]["structuredContent"]["errorDetails"]["reason"], "CAPABILITY_DENIED");
    assert!(!destination.exists());
    script.done();
}

#[test]
fn dfm_has_no_readable_output_during_settlement_and_withholds_a_late_receipt() {
    let fixture = fixture("late-finalize", Probe::Passed);
    let dir = fixture._scratch.dir().join("validation");
    let clock = WireClock::default();
    let expiry = clock.clone();
    let inspect = Box::new(move || {
        assert!(!std::fs::read_dir(dir.join("runs")).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().ends_with(".report.json")));
        assert_eq!(std::fs::read_dir(dir.join("evidence")).unwrap().count(), 0);
        expiry.0.set(60_000);
    });
    let script =
        Script::new(Capability::Dfm, EntryCategory::CliDfm, [Step::Status, Step::Admit, Step::Status, Step::Status, Step::InspectFinalize(inspect)]);
    let mut billing = live_billing(fixture_session(), script.clone(), fixture._scratch.dir().join("journal"), clock);
    let error = run_check(&fixture, &mut billing).unwrap_err();
    assert!(error.starts_with("UNSAFE_TO_START:"), "{error}");
    let id = error.rsplit_once('(').unwrap().1.trim_end_matches(')');
    let report = fixture.port.report(id).unwrap().report;
    assert!(report.checks.iter().all(|check| check.state == CheckState::NotRun && check.evidence.is_empty()));
    script.done();
}

#[test]
fn mcp_withholds_a_late_receipt_but_does_not_roll_back_a_file_write() {
    let scratch = Scratch::new("mcp-late-finalize");
    let destination = scratch.dir().join("late.sc");
    let mut model = solidcraft_mcp::Headless::default();
    model.session.execute("solid.box", &json!({"length":20,"width":20,"height":10})).unwrap();
    let clock = WireClock::default();
    let expiry = clock.clone();
    let inspect = Box::new(move || expiry.0.set(60_000));
    let script = Script::new(
        Capability::Mcp,
        EntryCategory::HeadlessMcp,
        [Step::Status, Step::Admit, Step::Status, Step::Status, Step::InspectFinalize(inspect)],
    );
    let billing = live_billing(fixture_session(), script.clone(), scratch.dir().join("journal"), clock);
    let mut budget = BudgetTable::unverified_default();
    budget.set_bound(Capability::Mcp, EntryCategory::HeadlessMcp, UpperBound::Verified(Duration::from_secs(1)));
    let mut server = compose_mcp_server(Box::new(model), None, McpAccount::from_session(billing), McpRoute::Headless, budget);
    let save = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"save","arguments":{"path":destination}}}).to_string();
    let reply: Value = serde_json::from_str(&server.handle_line(&save).unwrap()).unwrap();
    assert_eq!(reply["result"]["structuredContent"]["errorDetails"]["reason"], "UNSAFE_TO_START");
    assert!(!reply.to_string().contains("late.sc"), "{reply}");
    assert!(destination.is_file());
    script.done();
}

#[test]
fn failure_writing_final_intent_still_releases_original_hold() {
    let scratch = Scratch::new("intent-write");
    let journal = scratch.dir().join("journal");
    let script = Script::new(
        Capability::Dfm,
        EntryCategory::CliDfm,
        [Step::Status, Step::Admit, Step::Status, Step::Status, Step::Finalize(BillingResult::FailedNoEffect)],
    );
    let op = OperationId::parse("op_cli_write_failure").unwrap();
    let mut billing = live_billing(fixture_session(), script.clone(), journal.clone(), WireClock::default());
    billing.admit(&op, Capability::Dfm, EntryCategory::CliDfm).unwrap();
    billing.dispatch(&op, Duration::from_secs(1)).unwrap();
    let intent = journal.join("records").join(format!("{}.settlement-intent.json", op.as_str()));
    // A dangling name reads as absent, but the create-only publication fails.
    #[cfg(unix)]
    std::os::unix::fs::symlink(scratch.dir().join("missing"), &intent).unwrap();
    #[cfg(not(unix))]
    std::fs::create_dir(&intent).unwrap();
    assert_eq!(billing.commit(&op, BillingResult::Succeeded).unwrap_err().code, RefusalCode::AuthUnavailable);
    script.done();
}

#[test]
fn lost_or_malformed_admission_releases_without_authorizing_execution() {
    for (name, steps) in [
        ("bad-grant", vec![Step::Status, Step::BadGrant, Step::Finalize(BillingResult::FailedNoEffect)]),
        ("lost-grant", vec![Step::Status, Step::LoseAdmit, Step::RecoverAdmit, Step::LookupHeld, Step::Finalize(BillingResult::FailedNoEffect)]),
    ] {
        let scratch = Scratch::new(name);
        let script = Script::new(Capability::Dfm, EntryCategory::CliDfm, steps);
        let journal = scratch.dir().join("journal");
        let op = OperationId::parse("op_cli_lost_admission").unwrap();
        let mut billing = live_billing(fixture_session(), script.clone(), journal.clone(), WireClock::default());
        assert_eq!(billing.admit(&op, Capability::Dfm, EntryCategory::CliDfm).unwrap_err().code, RefusalCode::AuthUnavailable);
        assert_eq!(FileOperationStore::open(journal).unwrap().load(&op).unwrap().unwrap().status, OperationStatus::Released);
        script.done();
    }
}
