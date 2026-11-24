//! Controlled CLI DFM accounting: classification from the resolved profile, one admission,
//! one settlement for a produced report, a release for an incomplete run, and a whole refusal
//! when the authority or the safe-stop bound is missing.
//!
//! Every double here is local to this test file. No production path installs a fake grant, and
//! no test sends a live request.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use solidcraft_cli::validation::{ControlledCheck, NoAuthorityBilling, ProfileScope, profile_scope, run_controlled_check};
use solidcraft_engine::entitlement::{
    BillingPort, BillingResult, BudgetTable, Capability, EntryCategory, Grant, GrantId, MonotonicInstant, OperationId, Refusal, RefusalCode,
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
}

impl Runner for FixedRunner {
    fn identity(&self) -> RunnerIdentity {
        self.identity.clone()
    }

    fn run(&self, job: &RunJob, _control: &RunControl) -> RunnerOutcome {
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
}

fn fixture(name: &str, probe: Probe) -> Fixture {
    let scratch = Scratch::new(name);
    let candidates = scratch.dir().join("candidates");
    let store = scratch.dir().join("validation");
    let candidate = write_candidate(&candidates, "cand-cli-dfm");
    let port = ValidationPort::open_with(
        store,
        candidates,
        Arc::new(FixedRunner { identity: LocalRunner::identity(), probe }),
        Limits::default(),
        Arc::new(SystemClock),
    )
    .unwrap();
    Fixture { _scratch: scratch, port, candidate }
}

fn run_check(fixture: &Fixture, billing: &mut CountingBilling) -> Result<ControlledCheck, String> {
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

/// The shipped binary has no installed authority yet, so a mixed request is refused before it
/// reaches the validation port instead of running with the DFM part silently skipped.
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
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("UNSAFE_TO_START"), "{stderr}");
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
