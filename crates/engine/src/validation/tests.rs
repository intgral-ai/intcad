//! Validation port tests: state machine, profiles, idempotency, reports and the local runner.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::*;
use crate::Session;
use solidcraft_io::candidate::{self, CandidateFile};

fn scratch(name: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("solidcraft-validation-{name}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn analytic_design() -> Vec<u8> {
    let mut s = Session::default();
    s.execute("parameters.add", &json!({"name": "width", "expression": "20 mm"})).unwrap();
    s.execute("solid.box", &json!({"length": "width", "width": 10, "height": 5})).unwrap();
    solidcraft_io::write_design_container(&s.doc).unwrap()
}

fn manifest_with_engine(candidate_id: &str, files: &[CandidateFile], engine_version: &str) -> Value {
    let mut inputs: Vec<Value> = files
        .iter()
        .map(|file| {
            json!({
                "path": file.path,
                "kind": if file.path == "design.sc" { "design" } else { "component" },
                "dependencyId": if file.path == "design.sc" { "design-root".to_string() } else { format!("dep-{}", file.path.replace('/', "-")) },
                "dependencyVersion": null,
                "digest": candidate::digest(&file.bytes),
                "sizeBytes": file.bytes.len(),
            })
        })
        .collect();
    inputs.sort_by(|a, b| a["path"].as_str().unwrap_or_default().cmp(b["path"].as_str().unwrap_or_default()));
    json!({
        "schemaVersion": "1.0",
        "candidateId": candidate_id,
        "source": {"documentId": "doc-validation", "revision": "7"},
        "capturedAt": "2026-11-07T12:00:00.000Z",
        "formatVersion": "solidcraft/unknown",
        "engineVersion": engine_version,
        "inputs": inputs,
    })
}

fn write_candidate(root: &Path, id: &str, design: Vec<u8>) -> crate::schema::snapshot::CandidateRef {
    write_candidate_with_engine(root, id, design, env!("CARGO_PKG_VERSION"))
}

fn write_candidate_with_engine(root: &Path, id: &str, design: Vec<u8>, engine_version: &str) -> crate::schema::snapshot::CandidateRef {
    let files = vec![CandidateFile { path: "design.sc".into(), bytes: design }];
    let stored = candidate::write(root, id, &files, &manifest_with_engine(id, &files, engine_version), &format!("artifact-{id}")).unwrap();
    crate::schema::snapshot::CandidateRef {
        candidate_id: id.into(),
        source: crate::schema::snapshot::SnapshotSource { document_id: "doc-validation".into(), revision: "7".into() },
        candidate_digest: stored.candidate_digest,
        manifest_digest: stored.manifest_digest,
        artifact: crate::schema::snapshot::CandidateArtifact {
            artifact_id: stored.artifact.artifact_id,
            digest: stored.artifact.digest,
            size_bytes: stored.artifact.size_bytes,
            media_type: stored.artifact.media_type,
        },
    }
}

/// Simulate the crash of the host that admitted/claimed this run. A fresh process opens a new
/// host id; tests keep one process, so they record a process that is definitely gone.
fn mark_host_dead(store_root: &Path, run_id: &str) {
    let store = store::Store::open(store_root.to_path_buf()).unwrap();
    let dead = 999_999_u32;
    let mut admission: store::AdmissionRecord = store.read_json(&store.admission_path(run_id), 1 << 20).unwrap();
    admission.host_pid = dead;
    store.write_json_replace(&store.admission_path(run_id), &admission).unwrap();
    if store.worker_path(run_id).is_file() {
        let mut worker: store::WorkerRecord = store.read_json(&store.worker_path(run_id), 1 << 20).unwrap();
        worker.host_pid = dead;
        store.write_json_replace(&store.worker_path(run_id), &worker).unwrap();
    }
}

fn task() -> CheckProfile {
    task_profile(
        "task-analytic",
        "1.0.0",
        Some(vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }]),
        Some(vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 1000.0,
            abs_tolerance: 1e-6,
            rel_tolerance: 0.0,
        }]),
    )
    .unwrap()
}

fn beta_skill_profile() -> CheckProfile {
    text_to_cad_profile(
        vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }],
        vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 1000.0,
            abs_tolerance: 1e-6,
            rel_tolerance: 0.0,
        }],
    )
    .unwrap()
}

fn pass_checks(profile: &CheckProfile) -> Vec<CheckOutcome> {
    profile
        .checks
        .iter()
        .map(|requirement| CheckOutcome {
            check_id: requirement.check_id.clone(),
            state: CheckState::Passed,
            expected: requirement.parameters.clone(),
            actual: json!({"test": "pass"}),
            evidence: vec![EvidenceBytes { media_type: "application/json".into(), bytes: b"{\"verified\":true}".to_vec() }],
        })
        .collect()
}

struct FixedRunner {
    identity: RunnerIdentity,
    outcome: RunnerOutcome,
}

impl Runner for FixedRunner {
    fn identity(&self) -> RunnerIdentity {
        self.identity.clone()
    }

    fn run(&self, _job: &RunJob, _control: &RunControl) -> RunnerOutcome {
        self.outcome.clone()
    }
}

fn runner_with(checks: Vec<CheckOutcome>) -> Arc<dyn Runner> {
    Arc::new(FixedRunner { identity: LocalRunner::identity(), outcome: RunnerOutcome::Finished { started_at_ms: 1, finished_at_ms: 2, checks } })
}

fn port_with(root: &Path, candidates: &Path, runner: Arc<dyn Runner>, limits: Limits, clock: Arc<dyn Clock>) -> ValidationPort {
    ValidationPort::open_with(root.to_path_buf(), candidates.to_path_buf(), runner, limits, clock).unwrap()
}

fn native_reference(port: &ValidationPort) -> ProfileRef {
    port.native_basic_reference().unwrap()
}

#[test]
fn native_basic_is_stable_and_carries_the_two_required_checks() {
    let profile = native_basic_profile();
    assert_eq!(profile.profile_id, "native-basic");
    assert_eq!(profile.version, "1.0.0");
    assert_eq!(profile.checks.iter().map(|c| c.check_id.as_str()).collect::<Vec<_>>(), ["input.integrity", "native.rebuild"]);
    assert!(profile.checks.iter().all(|check| check.required));
    let reference = profile_ref(&profile).unwrap();
    assert_eq!(reference.digest, profile_digest(&profile).unwrap());
    verify_profile_ref(&profile, &reference).unwrap();
    assert!(native_basic_profile().checks.iter().all(|check| check.parameters == json!({})));
}

#[test]
fn a_task_profile_adds_checks_and_cannot_relax_the_native_ones() {
    let profile = task();
    assert_eq!(profile.checks.len(), 4);
    assert_eq!(
        profile.checks.iter().map(|check| check.check_id.as_str()).collect::<Vec<_>>(),
        ["geometry.exact", "input.integrity", "native.rebuild", "parameters.required"]
    );
    let mut weakened = profile.clone();
    weakened.checks.iter_mut().find(|check| check.check_id == "native.rebuild").unwrap().required = false;
    assert_eq!(validate_profile(&weakened).unwrap_err().code, ErrorCode::ProfileUnsupported);
    let mut missing = profile;
    missing.checks.retain(|check| check.check_id != "input.integrity");
    assert_eq!(validate_profile(&missing).unwrap_err().code, ErrorCode::ProfileUnsupported);
}

#[test]
fn the_local_runner_passes_the_analytic_profile_and_reports_evidence() {
    let root = scratch("local-pass");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-analytic", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());
    let profile = port.register_profile(task()).unwrap();
    let key = generate_submission_key(clock.now_ms());
    let submitted = port.submit(&candidate, &profile, &key).unwrap();
    assert_eq!(submitted.state, ValidationState::Queued);
    port.pump().unwrap();
    let status = port.status(&submitted.run_id).unwrap();
    if status.state != ValidationState::Passed {
        println!("{:#}", serde_json::to_value(&port.report(&submitted.run_id).unwrap().report).unwrap());
    }
    assert_eq!(status.state, ValidationState::Passed);
    assert!(status.report_available);
    let report = port.report(&submitted.run_id).unwrap();
    assert_eq!(report.report.state, ValidationState::Passed);
    assert_eq!(report.report.checks.len(), 4);
    for result in &report.report.checks {
        if result.required {
            assert_eq!(result.state, CheckState::Passed, "{result:?}");
            assert!(!result.evidence.is_empty(), "{result:?}");
        }
    }
    let integrity = report.report.checks.iter().find(|check| check.check_id == "input.integrity").unwrap();
    assert_eq!(integrity.actual["inputs"][0]["kind"], "design");
    assert_eq!(integrity.actual["inputs"][0]["dependencyId"], "design-root");
    assert!(integrity.actual["inputs"][0].get("dependencyVersion").is_some());
    let bytes = std::fs::read(store.join("runs").join(format!("{}.report.json", submitted.run_id))).unwrap();
    assert_eq!(report.reference.digest, super::ids::digest(&bytes));
    assert_eq!(report.reference.size_bytes, bytes.len() as u64);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_real_local_runner_failure_repairs_to_a_new_candidate_and_passes() {
    let root = scratch("local-repair");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());
    let profile =
        task_profile("task-width-25", "1.0.0", Some(vec![ParameterRequirement { name: "width".into(), value: 25.0, abs_tolerance: 1e-6 }]), None)
            .unwrap();
    let reference = port.register_profile(profile).unwrap();

    let before = write_candidate(&candidates, "cand-before-repair", analytic_design());
    let first = port.submit(&before, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    let failed = port.report(&first.run_id).unwrap().report;
    assert_eq!(failed.state, ValidationState::QualityFailed);
    assert!(failed.error.is_none(), "a measured mismatch is a quality report, not a runner error");
    let failed_check = failed.checks.iter().find(|check| check.check_id == "parameters.required").unwrap();
    assert_eq!(failed_check.state, CheckState::Failed);

    // The repair is a different native design, a different candidate and a new submission key.
    let mut repaired = Session::default();
    repaired.execute("parameters.add", &json!({"name": "width", "expression": "25 mm"})).unwrap();
    repaired.execute("solid.box", &json!({"length": "width", "width": 10, "height": 5})).unwrap();
    let after = write_candidate(&candidates, "cand-after-repair", solidcraft_io::write_design_container(&repaired.doc).unwrap());
    assert_ne!(before.candidate_digest, after.candidate_digest);
    let second_key = generate_submission_key(clock.now_ms());
    let second = port.submit(&after, &reference, &second_key).unwrap();
    port.pump().unwrap();
    assert_ne!(first.run_id, second.run_id, "a repair is a new run, not a retry of the failed one");
    let passed = port.report(&second.run_id).unwrap().report;
    assert_eq!(passed.state, ValidationState::Passed);
    assert_eq!(passed.candidate_id, "cand-after-repair");
    assert!(passed.checks.iter().all(|check| !check.required || check.state == CheckState::Passed));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_parameter_or_measurement_mismatch_is_a_quality_failure_not_an_infrastructure_one() {
    let root = scratch("local-quality");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-quality", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());
    let wrong_parameter = task_profile(
        "task-wrong-parameter",
        "1.0.0",
        Some(vec![ParameterRequirement { name: "width".into(), value: 21.0, abs_tolerance: 0.001 }]),
        None,
    )
    .unwrap();
    let reference = port.register_profile(wrong_parameter).unwrap();
    let submitted = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    assert_eq!(port.status(&submitted.run_id).unwrap().state, ValidationState::QualityFailed);
    let report = port.report(&submitted.run_id).unwrap().report;
    assert!(report.error.is_none());
    let failed = report.checks.iter().find(|check| check.check_id == "parameters.required").unwrap();
    assert_eq!(failed.state, CheckState::Failed);

    let wrong_measurement = task_profile(
        "task-wrong-measurement",
        "1.0.0",
        None,
        Some(vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 999.0,
            abs_tolerance: 0.001,
            rel_tolerance: 0.0,
        }]),
    )
    .unwrap();
    let reference = port.register_profile(wrong_measurement).unwrap();
    let submitted = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    assert_eq!(port.status(&submitted.run_id).unwrap().state, ValidationState::QualityFailed);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_lost_submit_response_retried_with_the_same_key_never_creates_a_second_job() {
    let root = scratch("idempotent");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-idem", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, runner_with(pass_checks(&native_basic_profile())), Limits::default(), clock.clone());
    let profile = native_reference(&port);
    let key = generate_submission_key(clock.now_ms());
    let first = port.submit(&candidate, &profile, &key).unwrap();
    let second = port.submit(&candidate, &profile, &key).unwrap();
    assert_eq!(first.run_id, second.run_id);
    assert_eq!(port.store.count_admissions(), 1);
    // Simulate the key record being lost after the admission was persisted.
    let parsed = SubmissionKey::parse(&key, clock.now_ms()).unwrap();
    std::fs::remove_file(port.store.key_path(&parsed.file_name())).unwrap();
    let third = port.submit(&candidate, &profile, &key).unwrap();
    assert_eq!(first.run_id, third.run_id);
    assert_eq!(port.store.count_admissions(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_same_key_with_another_payload_is_a_conflict_and_an_old_key_expires() {
    let root = scratch("conflict");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-conflict", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, runner_with(pass_checks(&native_basic_profile())), Limits::default(), clock.clone());
    let native = native_reference(&port);
    let task = port.register_profile(task()).unwrap();
    let key = generate_submission_key(clock.now_ms());
    port.submit(&candidate, &native, &key).unwrap();
    let conflict = port.submit(&candidate, &task, &key).unwrap_err();
    assert_eq!(conflict.code, ErrorCode::IdempotencyConflict);
    let old = format!("{}:{}", clock.now_ms() - 7 * 24 * 60 * 60 * 1000, keys::uuid_v4());
    let expired = port.submit(&candidate, &native, &old).unwrap_err();
    assert_eq!(expired.code, ErrorCode::RequestExpired);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn queued_running_and_terminal_states_are_durable_and_terminal_is_immutable() {
    let root = scratch("states");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-states", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    let reference = native_reference(&port);
    let submitted = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    assert_eq!(submitted.state, ValidationState::Queued);
    assert_eq!(port.status(&submitted.run_id).unwrap().state, ValidationState::Queued);
    let job = port.start_next().unwrap().expect("a worker claims the queued job");
    assert_eq!(port.status(&submitted.run_id).unwrap().state, ValidationState::Running);
    let outcome = RunnerOutcome::Finished { started_at_ms: 1, finished_at_ms: 2, checks: pass_checks(&profile) };
    assert_eq!(port.complete(&job.run_id, outcome).unwrap(), ValidationState::Passed);
    let report = port.report(&submitted.run_id).unwrap().report;
    assert_eq!(report.checks.len(), profile.checks.len());
    for check in &profile.checks {
        assert_eq!(report.checks.iter().filter(|result| result.check_id == check.check_id).count(), 1);
        assert_eq!(report.checks.iter().find(|result| result.check_id == check.check_id).unwrap().required, check.required);
    }
    // A late, different result cannot replace the terminal state.
    let late = RunnerOutcome::Finished { started_at_ms: 3, finished_at_ms: 4, checks: Vec::new() };
    assert_eq!(port.complete(&submitted.run_id, late).unwrap(), ValidationState::Passed);
    assert_eq!(port.status(&submitted.run_id).unwrap().state, ValidationState::Passed);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_missing_check_a_missing_evidence_artifact_and_a_runner_mismatch_cannot_pass() {
    let root = scratch("not-passed");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-not-passed", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();

    let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::InfrastructureError);
    assert_eq!(port.report(&run.run_id).unwrap().report.checks.len(), profile.checks.len());

    let root2 = scratch("no-evidence");
    let candidates2 = root2.join("candidates");
    let store2 = root2.join("validation");
    let candidate2 = write_candidate(&candidates2, "cand-no-evidence", analytic_design());
    let mut checks = pass_checks(&profile);
    for check in &mut checks {
        check.evidence.clear();
    }
    let port = port_with(&store2, &candidates2, runner_with(checks), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate2, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::InfrastructureError);

    let root3 = scratch("runner-mismatch");
    let candidates3 = root3.join("candidates");
    let store3 = root3.join("validation");
    let candidate3 = write_candidate(&candidates3, "cand-runner-mismatch", analytic_design());
    let port = port_with(&store3, &candidates3, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate3, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    let job = port.start_next().unwrap().unwrap();
    let wrong = RunnerIdentity { runner_id: LOCAL_RUNNER_ID.into(), version: LOCAL_RUNNER_VERSION.into(), engine_version: "0.0.0".into() };
    let outcome = RunnerOutcome::Finished { started_at_ms: 1, finished_at_ms: 2, checks: pass_checks(&profile) };
    assert_eq!(port.complete_by(&job.run_id, outcome, &wrong).unwrap(), ValidationState::InfrastructureError);
    let report = port.report(&run.run_id).unwrap().report;
    assert_eq!(report.error.as_ref().map(|error| error.code.as_str()), Some("REPORT_MISMATCH"));
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
    let _ = std::fs::remove_dir_all(root3);
}

#[test]
fn timeout_and_worker_failure_are_infrastructure_errors_distinct_from_quality_failures() {
    let root = scratch("timeout");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-timeout", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let timeout = Arc::new(FixedRunner {
        identity: LocalRunner::identity(),
        outcome: RunnerOutcome::TimedOut { started_at_ms: Some(1), finished_at_ms: 2, message: "too slow".into() },
    });
    let port = port_with(&store, &candidates, timeout, Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    let report = port.report(&run.run_id).unwrap().report;
    assert_eq!(report.state, ValidationState::InfrastructureError);
    assert_eq!(report.error.as_ref().map(|error| error.code.as_str()), Some("TIMEOUT"));
    assert_eq!(report.checks.iter().filter(|check| check.state == CheckState::NotRun).count(), profile.checks.len());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cancellation_persists_a_marker_and_a_late_pass_cannot_overwrite_it() {
    let root = scratch("cancel");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-cancel", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    let job = port.start_next().unwrap().unwrap();
    assert_eq!(port.cancel(&run.run_id).unwrap().state, ValidationState::Cancelled);
    assert!(store.join("runs").join(format!("{}.cancel", run.run_id)).is_file());
    assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::Cancelled);
    let late = RunnerOutcome::Finished { started_at_ms: 1, finished_at_ms: 2, checks: pass_checks(&profile) };
    assert_eq!(port.complete(&job.run_id, late).unwrap(), ValidationState::Cancelled);
    assert_eq!(port.report(&run.run_id).unwrap().report.state, ValidationState::Cancelled);
    let later = RunnerOutcome::Finished { started_at_ms: 3, finished_at_ms: 4, checks: pass_checks(&profile) };
    assert_eq!(port.complete(&run.run_id, later).unwrap(), ValidationState::Cancelled);
    let _ = std::fs::remove_dir_all(root);
}

/// Two processes can race to commit the same terminal state (the CLI's submit and a second
/// process's cancel). The first report is authoritative; the second must not report a mismatch.
#[test]
fn a_second_report_for_the_same_terminal_state_is_idempotent() {
    let root = scratch("cancel-report-idempotent");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-cancel-report", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    assert_eq!(port.cancel(&run.run_id).unwrap().state, ValidationState::Cancelled);
    let admission = port.load_admission(&run.run_id).unwrap();
    let profile = port.store.read_profile(&run.run_id).unwrap();
    clock.advance(1_000);
    let second = port.recovery_report(
        &admission,
        &profile,
        ValidationState::Cancelled,
        Some(ValidationError::cancelled("a second observer cancelled the job")),
    );
    assert!(port.commit_report(&run.run_id, second, ValidationState::Cancelled).is_ok());
    assert_eq!(port.report(&run.run_id).unwrap().report.state, ValidationState::Cancelled);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_queued_job_cancel_is_immediate_and_idempotent() {
    let root = scratch("cancel-queued");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-cancel-queued", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    let reference = native_reference(&port);
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    let cancelled = port.cancel(&run.run_id).unwrap();
    assert_eq!(cancelled.state, ValidationState::Cancelled);
    assert_eq!(port.cancel(&run.run_id).unwrap().state, ValidationState::Cancelled);
    assert!(port.start_next().unwrap().is_none());
    assert_eq!(port.report(&run.run_id).unwrap().report.state, ValidationState::Cancelled);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_restart_reconciles_queued_and_running_jobs_without_rerunning_them() {
    let root = scratch("restart");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-restart", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let queued = {
        let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
        port.register_profile(profile.clone()).unwrap();
        port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap()
    };
    mark_host_dead(&store, &queued.run_id);
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    assert_eq!(port.status(&queued.run_id).unwrap().state, ValidationState::InfrastructureError);
    assert!(port.start_next().unwrap().is_none());

    let root2 = scratch("restart-running");
    let candidates2 = root2.join("candidates");
    let store2 = root2.join("validation");
    let candidate2 = write_candidate(&candidates2, "cand-restart-running", analytic_design());
    let running = {
        let port = port_with(&store2, &candidates2, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
        port.register_profile(profile.clone()).unwrap();
        let run = port.submit(&candidate2, &reference, &generate_submission_key(clock.now_ms())).unwrap();
        port.start_next().unwrap().unwrap();
        run
    };
    mark_host_dead(&store2, &running.run_id);
    let port = port_with(&store2, &candidates2, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    let status = port.status(&running.run_id).unwrap();
    assert_eq!(status.state, ValidationState::Unknown);
    assert!(port.start_next().unwrap().is_none());
    assert_eq!(port.report(&running.run_id).unwrap().report.error.as_ref().map(|error| error.code.as_str()), Some("OUTCOME_UNKNOWN"));
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}

#[test]
fn changed_input_bytes_cannot_be_admitted_or_reported() {
    let root = scratch("changed");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-changed", analytic_design());
    let container = candidates.join("cand-changed.zip");
    let mut bytes = std::fs::read(&container).unwrap();
    *bytes.last_mut().unwrap() ^= 0xff;
    std::fs::write(&container, bytes).unwrap();
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, runner_with(pass_checks(&native_basic_profile())), Limits::default(), clock.clone());
    let reference = native_reference(&port);
    let error = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidCandidate);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_report_that_cannot_prove_its_checks_is_not_a_pass() {
    let root = scratch("coverage");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-coverage", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    let job = port.start_next().unwrap().unwrap();
    let outcome = RunnerOutcome::Finished {
        started_at_ms: 1,
        finished_at_ms: 2,
        checks: pass_checks(&profile).into_iter().chain(pass_checks(&profile)).collect(),
    };
    assert_eq!(port.complete(&job.run_id, outcome).unwrap(), ValidationState::InfrastructureError);
    assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::InfrastructureError);
    let report = port.report(&run.run_id).unwrap().report;
    assert_eq!(report.checks.len(), profile.checks.len());
    assert_eq!(report.error.as_ref().map(|error| error.code.as_str()), Some("REPORT_MISMATCH"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_unregistered_or_invalid_profile_is_unsupported() {
    let root = scratch("profile");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-profile", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    let unknown = ProfileRef { profile_id: "not-registered".into(), version: "1.0.0".into(), digest: format!("sha256:{}", "0".repeat(64)) };
    let error = port.submit(&candidate, &unknown, &generate_submission_key(clock.now_ms())).unwrap_err();
    assert_eq!(error.code, ErrorCode::ProfileUnsupported);
    let mut invalid = native_basic_profile();
    invalid.checks.pop();
    let error = port.register_profile(invalid).unwrap_err();
    assert_eq!(error.code, ErrorCode::ProfileUnsupported);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn evidence_is_digest_checked_and_the_report_reference_hashes_the_persisted_bytes() {
    let root = scratch("evidence");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-evidence", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    let report = port.report(&run.run_id).unwrap();
    let evidence = report.report.checks[0].evidence[0].clone();
    let path = store.join("evidence").join(&run.run_id).join(format!("{}.json", evidence.artifact_id));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(evidence.digest, super::ids::digest(&bytes));
    std::fs::write(&path, b"tampered").unwrap();
    let error = port.report(&run.run_id).unwrap_err();
    assert_eq!(error.code, ErrorCode::NotReady);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn admission_limits_reject_new_work_instead_of_evicting_active_evidence() {
    let root = scratch("limits");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-limits", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let limits = Limits { queue: 1, ..Limits::default() };
    let port = port_with(&store, &candidates, runner_with(Vec::new()), limits.clone(), clock.clone());
    let reference = native_reference(&port);
    port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    let error = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap_err();
    assert_eq!(error.code, ErrorCode::ResourceExhausted);
    // Cleanup never removes an active (queued) job.
    clock.advance(30 * 24 * 60 * 60 * 1000);
    assert_eq!(port.cleanup(clock.now_ms()), 0);
    assert_eq!(port.status(&port.store.run_ids()[0]).unwrap().state, ValidationState::Queued);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn retention_removes_only_finished_jobs() {
    let root = scratch("retention");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-retention", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let limits = Limits { retention: std::time::Duration::from_millis(10), ..Limits::default() };
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), limits, clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let done = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    let active = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    clock.advance(1000);
    assert_eq!(port.cleanup(clock.now_ms()), 1);
    assert!(port.status(&done.run_id).is_err());
    assert_eq!(port.status(&active.run_id).unwrap().state, ValidationState::Queued);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_changed_input_byte_after_a_pass_rejects_the_report() {
    let root = scratch("changed-after");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-changed-after", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::Passed);
    let container = candidates.join("cand-changed-after.zip");
    let mut bytes = std::fs::read(&container).unwrap();
    *bytes.last_mut().unwrap() ^= 0xff;
    std::fs::write(&container, bytes).unwrap();
    let error = port.report(&run.run_id).unwrap_err();
    assert!(
        matches!(error.code, ErrorCode::InvalidCandidate | ErrorCode::DependencyChanged | ErrorCode::NotFound),
        "a changed input byte keeps its permanent code, not NOT_READY: {error}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_unsupported_required_check_and_a_malformed_report_cannot_pass() {
    let root = scratch("unsupported");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-unsupported", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let mut profile = native_basic_profile();
    profile.profile_id = "task-unsupported".into();
    profile.checks.push(CheckRequirement {
        check_id: "measurement.future".into(),
        kind: "measurement.future".into(),
        required: true,
        parameters: json!({}),
    });
    profile.checks.sort_by(|a, b| a.check_id.cmp(&b.check_id));
    validate_profile(&profile).unwrap();
    let reference = profile_ref(&profile).unwrap();
    let mut checks = pass_checks(&profile);
    for check in &mut checks {
        if check.check_id == "measurement.future" {
            check.state = CheckState::Unsupported;
            check.evidence.clear();
        }
    }
    let port = port_with(&store, &candidates, runner_with(checks), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::InfrastructureError);

    // A report that cannot be parsed is never a pass.
    let root2 = scratch("malformed-report");
    let candidates2 = root2.join("candidates");
    let store2 = root2.join("validation");
    let candidate2 = write_candidate(&candidates2, "cand-malformed-report", analytic_design());
    let native = native_basic_profile();
    let reference = profile_ref(&native).unwrap();
    let port = port_with(&store2, &candidates2, runner_with(pass_checks(&native)), Limits::default(), clock.clone());
    port.register_profile(native.clone()).unwrap();
    let run = port.submit(&candidate2, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    std::fs::write(store2.join("runs").join(format!("{}.report.json", run.run_id)), b"{").unwrap();
    assert_eq!(port.report(&run.run_id).unwrap_err().code, ErrorCode::NotReady);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}

fn read_port(store: &Path, candidates: &Path, clock: Arc<dyn Clock>) -> ValidationPort {
    ValidationPort::open_read_only_with(store.to_path_buf(), candidates.to_path_buf(), Limits::default(), clock).unwrap()
}

#[test]
fn a_read_only_status_or_cancel_never_reconciles_a_live_worker() {
    let root = scratch("read-only-live");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-read-only-live", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    let job = port.start_next().unwrap().unwrap();

    let observer = read_port(&store, &candidates, clock.clone());
    assert_eq!(observer.status(&run.run_id).unwrap().state, ValidationState::Running);
    assert_eq!(observer.cancel(&run.run_id).unwrap().state, ValidationState::Cancelled);
    assert!(store.join("runs").join(format!("{}.cancel", run.run_id)).is_file());
    assert_eq!(observer.status(&run.run_id).unwrap().state, ValidationState::Cancelled);
    // The worker's late pass cannot replace the cancellation.
    let late = RunnerOutcome::Finished { started_at_ms: 1, finished_at_ms: 2, checks: pass_checks(&profile) };
    assert_eq!(port.complete(&job.run_id, late).unwrap(), ValidationState::Cancelled);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn a_foreign_live_worker_is_not_reconciled_by_another_process_port() {
    let root = scratch("foreign-live");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-foreign-live", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.start_next().unwrap().unwrap();
    let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
    let foreign_pid = child.id();

    let mut admission: store::AdmissionRecord = port.store.read_json(&port.store.admission_path(&run.run_id), 1 << 20).unwrap();
    admission.host_id = "foreign-host".into();
    admission.host_pid = foreign_pid;
    port.store.write_json_replace(&port.store.admission_path(&run.run_id), &admission).unwrap();
    let mut worker: store::WorkerRecord = port.store.read_json(&port.store.worker_path(&run.run_id), 1 << 20).unwrap();
    worker.host_id = "foreign-host".into();
    worker.host_pid = foreign_pid;
    port.store.write_json_replace(&port.store.worker_path(&run.run_id), &worker).unwrap();

    let observer = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    assert_eq!(observer.status(&run.run_id).unwrap().state, ValidationState::Running);
    assert_eq!(observer.cancel(&run.run_id).unwrap().state, ValidationState::Cancelled);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_lost_state_file_is_repaired_by_the_same_key_retry() {
    let root = scratch("repair-state");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-repair-state", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    let reference = native_reference(&port);
    let key = generate_submission_key(clock.now_ms());
    let first = port.submit(&candidate, &reference, &key).unwrap();
    std::fs::remove_file(store.join("runs").join(format!("{}.state.json", first.run_id))).unwrap();
    let retried = port.submit(&candidate, &reference, &key).unwrap();
    assert_eq!(retried.run_id, first.run_id);
    assert_eq!(retried.state, ValidationState::Queued);
    assert!(store.join("runs").join(format!("{}.state.json", first.run_id)).is_file());
    assert_eq!(port.store.count_admissions(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_lost_state_file_is_repaired_by_status_and_cleanup_sees_the_admission() {
    let root = scratch("repair-status");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-repair-status", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    let reference = native_reference(&port);
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    std::fs::remove_file(store.join("runs").join(format!("{}.state.json", run.run_id))).unwrap();
    // A fresh read-only port repairs from the admission rather than reporting NOT_FOUND.
    let observer = read_port(&store, &candidates, clock.clone());
    assert_eq!(observer.status(&run.run_id).unwrap().state, ValidationState::Queued);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_same_key_submissions_share_one_run_and_keep_the_pin() {
    let root = scratch("concurrent");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-concurrent", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = Arc::new(port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone()));
    port.register_profile(profile).unwrap();
    let key = generate_submission_key(clock.now_ms());
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let port = Arc::clone(&port);
        let candidate = candidate.clone();
        let reference = reference.clone();
        let key = key.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            port.submit(&candidate, &reference, &key)
        }));
    }
    let results: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap().unwrap()).collect();
    assert_eq!(results[0].run_id, results[1].run_id);
    assert_eq!(port.store.count_admissions(), 1);
    assert!(candidate::is_pinned(&candidates, "cand-concurrent"));
    assert_eq!(port.status(&results[0].run_id).unwrap().state, ValidationState::Queued);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn idempotent_replay_survives_a_removed_profile_and_candidate() {
    let root = scratch("replay-without-profile");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-replay-without-profile", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let task_profile = task();
    let reference = profile_ref(&task_profile).unwrap();
    let key = generate_submission_key(clock.now_ms());
    let run = {
        let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
        port.register_profile(task_profile).unwrap();
        port.submit(&candidate, &reference, &key).unwrap()
    };
    std::fs::remove_file(candidates.join("cand-replay-without-profile.zip")).unwrap();
    // The read-only port has no task profile and the candidate container is gone, but the key
    // still names the original run.
    let observer = read_port(&store, &candidates, clock.clone());
    let replay = observer.submit(&candidate, &reference, &key).unwrap();
    assert_eq!(replay.run_id, run.run_id);
    assert_eq!(replay.state, ValidationState::Queued);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_missing_or_tampered_committed_report_is_adopted_or_rejected_on_restart() {
    let root = scratch("adopt-report");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-adopt-report", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    // Simulate a crash after the report was committed but before the terminal state was written.
    let mut state: store::StateRecord = port.store.read_json(&port.store.state_path(&run.run_id), 1 << 20).unwrap();
    state.state = ValidationState::Running;
    state.report_available = false;
    port.store.write_json_replace(&port.store.state_path(&run.run_id), &state).unwrap();
    mark_host_dead(&store, &run.run_id);
    let restarted = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    assert_eq!(restarted.status(&run.run_id).unwrap().state, ValidationState::Passed);
    assert_eq!(restarted.report(&run.run_id).unwrap().report.state, ValidationState::Passed);
    assert!(!candidate::is_pinned(&candidates, "cand-adopt-report"));
    let _ = std::fs::remove_dir_all(root);

    let root2 = scratch("reject-report");
    let candidates2 = root2.join("candidates");
    let store2 = root2.join("validation");
    let candidate2 = write_candidate(&candidates2, "cand-reject-report", analytic_design());
    let port = port_with(&store2, &candidates2, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate2, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    let mut state: store::StateRecord = port.store.read_json(&port.store.state_path(&run.run_id), 1 << 20).unwrap();
    state.state = ValidationState::Running;
    state.report_available = false;
    port.store.write_json_replace(&port.store.state_path(&run.run_id), &state).unwrap();
    std::fs::write(store2.join("runs").join(format!("{}.report.json", run.run_id)), b"{").unwrap();
    mark_host_dead(&store2, &run.run_id);
    let restarted = port_with(&store2, &candidates2, runner_with(Vec::new()), Limits::default(), clock.clone());
    assert_eq!(restarted.status(&run.run_id).unwrap().state, ValidationState::Unknown);
    let _ = std::fs::remove_dir_all(root2);
}

#[test]
fn an_out_of_whitelist_geometry_command_is_unsupported() {
    let root = scratch("unsupported-command");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-unsupported-command", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = task_profile(
        "task-unsupported-command",
        "1.0.0",
        None,
        Some(vec![MeasurementRequirement {
            command: "solid.box".into(),
            params: json!({}),
            result_pointer: "/body_count".into(),
            expected: 1.0,
            abs_tolerance: 0.0,
            rel_tolerance: 0.0,
        }]),
    )
    .unwrap();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::InfrastructureError);
    let result = port.report(&run.run_id).unwrap().report;
    let geometry = result.checks.iter().find(|check| check.check_id == "geometry.exact").unwrap();
    assert_eq!(geometry.state, CheckState::Unsupported);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_worker_infrastructure_failure_is_not_a_timeout_or_quality_failure() {
    let root = scratch("worker-failure");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-worker-failure", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let failure = Arc::new(FixedRunner {
        identity: LocalRunner::identity(),
        outcome: RunnerOutcome::Infrastructure { started_at_ms: Some(1), finished_at_ms: 2, message: "worker crashed".into() },
    });
    let port = port_with(&store, &candidates, failure, Limits::default(), clock.clone());
    port.register_profile(profile).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    let report = port.report(&run.run_id).unwrap().report;
    assert_eq!(report.state, ValidationState::InfrastructureError);
    assert_eq!(report.error.as_ref().map(|error| error.code.as_str()), Some("INTERNAL_ERROR"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_engine_version_is_checked_when_a_new_job_is_admitted() {
    let root = scratch("engine-version");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate_with_engine(&candidates, "cand-engine-version", analytic_design(), "0.0.0");
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    let reference = native_reference(&port);
    let error = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidCandidate);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn limits_above_the_contract_ceilings_are_rejected() {
    Limits::default().validate().unwrap();
    assert!(Limits { workers: MAX_WORKERS + 1, ..Limits::default() }.validate().is_err());
    assert!(Limits { queue: MAX_QUEUE + 1, ..Limits::default() }.validate().is_err());
    assert!(Limits { report_bytes: MAX_REPORT_BYTES + 1, ..Limits::default() }.validate().is_err());
    assert!(Limits { checks: MAX_CHECKS_PER_PROFILE + 1, ..Limits::default() }.validate().is_err());
    assert!(Limits { evidence_artifacts: MAX_EVIDENCE_ARTIFACTS + 1, ..Limits::default() }.validate().is_err());
    assert!(Limits { evidence_bytes: MAX_EVIDENCE_BYTES + 1, ..Limits::default() }.validate().is_err());
    assert!(Limits { evidence_total_bytes: MAX_EVIDENCE_TOTAL_BYTES + 1, ..Limits::default() }.validate().is_err());
    assert!(Limits { execution: MAX_EXECUTION + std::time::Duration::from_millis(1), ..Limits::default() }.validate().is_err());
    assert!(Limits { queue_admission: MAX_QUEUE_ADMISSION + std::time::Duration::from_millis(1), ..Limits::default() }.validate().is_err());
    assert!(Limits { retention: MAX_RETENTION + std::time::Duration::from_millis(1), ..Limits::default() }.validate().is_err());
    assert!(Limits { job_records: MAX_JOB_RECORDS + 1, ..Limits::default() }.validate().is_err());
}

#[test]
fn an_error_message_is_capped_at_the_contract_limit() {
    let error = ValidationError::internal("x".repeat(5_000));
    assert!(error.message.len() <= 2_048);
    assert_eq!(error.message.len(), 2_048);
}

#[test]
fn cleanup_removes_retained_candidate_inputs_as_well_as_store_records() {
    let root = scratch("cleanup-candidate");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-cleanup-candidate", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let limits = Limits { retention: std::time::Duration::ZERO, ..Limits::default() };
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), limits, clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    assert!(candidate::lookup(&candidates, "cand-cleanup-candidate").is_ok());
    assert_eq!(port.cleanup(clock.now_ms() + 1), 1);
    assert!(candidate::lookup(&candidates, "cand-cleanup-candidate").is_err());
    assert!(port.status(&run.run_id).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_orphaned_admission_is_cleaned_up_and_does_not_linger() {
    let root = scratch("orphan-cleanup");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-orphan-cleanup", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let limits = Limits { retention: std::time::Duration::from_millis(10), ..Limits::default() };
    let port = port_with(&store, &candidates, runner_with(Vec::new()), limits, clock.clone());
    let reference = native_reference(&port);
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    std::fs::remove_file(store.join("runs").join(format!("{}.state.json", run.run_id))).unwrap();
    clock.advance(100);
    assert_eq!(port.cleanup(clock.now_ms()), 1);
    assert!(port.status(&run.run_id).is_err());
    assert_eq!(port.store.count_admissions(), 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn start_next_continues_past_a_profile_that_exceeds_the_limit() {
    let root = scratch("skip-large-profile");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-skip-large-profile", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let first = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    clock.advance(1);
    let second = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    let mut oversized = native_basic_profile();
    for index in 0..(MAX_CHECKS_PER_PROFILE - 1) {
        oversized.checks.push(CheckRequirement {
            check_id: format!("zz.{index:03}"),
            kind: "unsupported.test".into(),
            required: false,
            parameters: json!({}),
        });
    }
    oversized.checks.sort_by(|a, b| a.check_id.cmp(&b.check_id));
    port.store.write_json_replace(&port.store.profile_path(&first.run_id), &oversized).unwrap();
    let mut admission: store::AdmissionRecord = port.store.read_json(&port.store.admission_path(&first.run_id), 1 << 20).unwrap();
    admission.profile = profile_ref(&oversized).unwrap();
    port.store.write_json_replace(&port.store.admission_path(&first.run_id), &admission).unwrap();
    let job = port.start_next().unwrap().expect("the next queued job still starts");
    assert_eq!(job.run_id, second.run_id);
    assert_eq!(port.status(&first.run_id).unwrap().state, ValidationState::InfrastructureError);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_report_evidence_id_cannot_escape_its_run_directory() {
    let root = scratch("evidence-id");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-evidence-id", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.pump().unwrap();
    let report_path = store.join("runs").join(format!("{}.report.json", run.run_id));
    let mut report: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    report["checks"][0]["evidence"][0]["artifactId"] = json!("../../escape");
    std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
    let mut state: store::StateRecord = port.store.read_json(&port.store.state_path(&run.run_id), 1 << 20).unwrap();
    state.state = ValidationState::Running;
    state.report_available = false;
    port.store.write_json_replace(&port.store.state_path(&run.run_id), &state).unwrap();
    mark_host_dead(&store, &run.run_id);
    let restarted = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    assert_eq!(restarted.status(&run.run_id).unwrap().state, ValidationState::Unknown);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_same_key_submissions_from_two_ports_share_the_key_claim() {
    let root = scratch("concurrent-ports");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-concurrent-ports", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let first = Arc::new(port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone()));
    let second = Arc::new(port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone()));
    first.register_profile(profile.clone()).unwrap();
    second.register_profile(profile.clone()).unwrap();
    let key = generate_submission_key(clock.now_ms());
    let barrier = Arc::new(Barrier::new(2));
    let spawn = |port: Arc<ValidationPort>| {
        let candidate = candidate.clone();
        let reference = reference.clone();
        let key = key.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            port.submit(&candidate, &reference, &key)
        })
    };
    let a = spawn(Arc::clone(&first));
    let b = spawn(Arc::clone(&second));
    let a = a.join().unwrap().unwrap();
    let b = b.join().unwrap().unwrap();
    assert_eq!(a.run_id, b.run_id);
    assert_eq!(first.store.count_admissions(), 1);
    assert!(candidate::is_pinned(&candidates, "cand-concurrent-ports"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_terminal_state_never_pairs_with_an_incompatible_report() {
    for corrupt in [false, true] {
        let root = scratch(if corrupt { "terminal-corrupt-report" } else { "terminal-missing-report" });
        let candidates = root.join("candidates");
        let store = root.join("validation");
        let candidate = write_candidate(&candidates, "cand-terminal-report", analytic_design());
        let clock = Arc::new(ManualClock::new(1_760_000_000_000));
        let profile = native_basic_profile();
        let reference = profile_ref(&profile).unwrap();
        let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
        port.register_profile(profile.clone()).unwrap();
        let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
        port.pump().unwrap();
        assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::Passed);
        let report_path = store.join("runs").join(format!("{}.report.json", run.run_id));
        if corrupt {
            std::fs::write(&report_path, b"{").unwrap();
        } else {
            std::fs::remove_file(&report_path).unwrap();
        }
        let restarted = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
        let status = restarted.status(&run.run_id).unwrap();
        assert_eq!(status.state, ValidationState::Passed, "the terminal state is immutable");
        assert!(!status.report_available, "a missing or corrupt report is not available");
        let state: store::StateRecord = restarted.store.read_json(&restarted.store.state_path(&run.run_id), 1 << 20).unwrap();
        assert!(!state.report_available);
        assert!(!restarted.store.report_exists(&run.run_id), "an unverifiable report is removed, not replaced by an incompatible one");
        let error = restarted.report(&run.run_id).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotReady, "no REPORT_MISMATCH after reconciliation");
        assert!(!candidate::is_pinned(&candidates, "cand-terminal-report"));
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn a_pin_only_orphan_is_garbage_collected_after_retention() {
    let root = scratch("pin-only-orphan");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-pin-only", analytic_design());
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let clock = Arc::new(ManualClock::new(now));
    let limits = Limits { retention: std::time::Duration::ZERO, ..Limits::default() };
    let port = port_with(&store, &candidates, runner_with(Vec::new()), limits, clock.clone());
    let run_id = generate_run_id(now);
    let key = generate_submission_key(now);
    let parsed = SubmissionKey::parse(&key, now).unwrap();
    let profile = profile_ref(&native_basic_profile()).unwrap();
    let record = store::SubmissionRecord {
        schema_version: store::RECORD_SCHEMA.into(),
        submission_key: key.clone(),
        payload_digest: format!("sha256:{}", "0".repeat(64)),
        run_id: run_id.clone(),
        candidate: candidate.clone(),
        profile,
        owner: "local".into(),
        created_at_ms: now,
        expires_at_ms: parsed.expires_at_ms(),
    };
    port.store.write_json_new(&port.store.key_path(&parsed.file_name()), &record).unwrap();
    candidate::pin(&candidates, "cand-pin-only").unwrap();
    port.store.write_run_pin("cand-pin-only", &run_id).unwrap();
    assert!(candidate::is_pinned(&candidates, "cand-pin-only"));
    assert!(port.store.run_pin_path("cand-pin-only", &run_id).is_file());
    assert!(!port.store.admission_path(&run_id).is_file());
    assert!(!port.store.state_path(&run_id).is_file());

    clock.advance(8 * 24 * 60 * 60 * 1000);
    assert_eq!(port.cleanup(clock.now_ms()), 1);
    assert!(!candidate::is_pinned(&candidates, "cand-pin-only"));
    assert!(!port.store.run_pin_path("cand-pin-only", &run_id).is_file());
    assert!(!port.store.key_path(&parsed.file_name()).is_file());
    assert!(candidate::lookup(&candidates, "cand-pin-only").is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pin_run_and_cleanup_never_leave_a_pin_on_a_removed_candidate() {
    let root = scratch("pin-cleanup-race");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let _candidate = write_candidate(&candidates, "cand-pin-cleanup", analytic_design());
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let clock = Arc::new(ManualClock::new(now));
    let limits = Limits { retention: std::time::Duration::ZERO, ..Limits::default() };
    let port = Arc::new(port_with(&store, &candidates, runner_with(Vec::new()), limits, clock.clone()));
    let barrier = Arc::new(Barrier::new(2));
    let run_id = generate_run_id(now);
    let pin = {
        let port = Arc::clone(&port);
        let barrier = Arc::clone(&barrier);
        let run_id = run_id.clone();
        thread::spawn(move || {
            barrier.wait();
            port.pin_run(&run_id, "cand-pin-cleanup")
        })
    };
    let cleanup = {
        let port = Arc::clone(&port);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            port.cleanup(now)
        })
    };
    let pinned = pin.join().unwrap();
    let _ = cleanup.join().unwrap();
    let lookup_ok = candidate::lookup(&candidates, "cand-pin-cleanup").is_ok();
    assert_eq!(pinned.is_ok(), lookup_ok, "a pin exists exactly when the candidate still exists");
    assert_eq!(candidate::is_pinned(&candidates, "cand-pin-cleanup"), lookup_ok);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_only_status_and_report_treat_a_corrupt_terminal_report_as_unavailable() {
    for corrupt in [false, true] {
        let root = scratch(if corrupt { "read-only-corrupt" } else { "read-only-missing" });
        let candidates = root.join("candidates");
        let store = root.join("validation");
        let candidate = write_candidate(&candidates, "cand-read-only-report", analytic_design());
        let clock = Arc::new(ManualClock::new(1_760_000_000_000));
        let profile = native_basic_profile();
        let reference = profile_ref(&profile).unwrap();
        let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
        port.register_profile(profile.clone()).unwrap();
        let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
        port.pump().unwrap();
        let report_path = store.join("runs").join(format!("{}.report.json", run.run_id));
        if corrupt {
            std::fs::write(&report_path, b"{").unwrap();
        } else {
            std::fs::remove_file(&report_path).unwrap();
        }
        // No reconciliation: this is the same path as `validate status` / `validate report`.
        let observer = read_port(&store, &candidates, clock.clone());
        let status = observer.status(&run.run_id).unwrap();
        assert_eq!(status.state, ValidationState::Passed);
        assert!(!status.report_available);
        let error = observer.report(&run.run_id).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotReady, "read-only report is NOT_READY, not REPORT_MISMATCH");
        assert_eq!(store.join("runs").join(format!("{}.report.json", run.run_id)).is_file(), corrupt);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn a_terminal_same_key_replay_does_not_re_pin() {
    let root = scratch("replay-terminal-no-pin");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-replay-terminal", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = native_basic_profile();
    let reference = profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, runner_with(pass_checks(&profile)), Limits::default(), clock.clone());
    port.register_profile(profile.clone()).unwrap();
    let key = generate_submission_key(clock.now_ms());
    let first = port.submit(&candidate, &reference, &key).unwrap();
    port.pump().unwrap();
    assert_eq!(port.status(&first.run_id).unwrap().state, ValidationState::Passed);
    assert!(!candidate::is_pinned(&candidates, "cand-replay-terminal"));
    assert!(!port.store.run_pin_path("cand-replay-terminal", &first.run_id).is_file());
    let replay = port.submit(&candidate, &reference, &key).unwrap();
    assert_eq!(replay.run_id, first.run_id);
    assert_eq!(replay.state, ValidationState::Passed);
    assert!(!candidate::is_pinned(&candidates, "cand-replay-terminal"), "a terminal replay must not re-pin");
    assert!(!port.store.run_pin_path("cand-replay-terminal", &first.run_id).is_file());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn start_next_pins_a_recovered_queued_run_or_reports_missing_inputs() {
    let root = scratch("start-next-pin");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-start-next-pin", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, runner_with(Vec::new()), Limits::default(), clock.clone());
    let reference = native_reference(&port);
    let run = port.submit(&candidate, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.store.remove_run_pin("cand-start-next-pin", &run.run_id).unwrap();
    candidate::unpin(&candidates, "cand-start-next-pin").unwrap();
    assert!(!candidate::is_pinned(&candidates, "cand-start-next-pin"));
    let job = port.start_next().unwrap().expect("a recovered queued run starts");
    assert_eq!(job.run_id, run.run_id);
    assert!(candidate::is_pinned(&candidates, "cand-start-next-pin"));
    assert!(port.store.run_pin_path("cand-start-next-pin", &run.run_id).is_file());
    assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::Running);
    let _ = std::fs::remove_dir_all(root);

    let root2 = scratch("start-next-missing-input");
    let candidates2 = root2.join("candidates");
    let store2 = root2.join("validation");
    let candidate2 = write_candidate(&candidates2, "cand-start-next-missing", analytic_design());
    let port = port_with(&store2, &candidates2, runner_with(Vec::new()), Limits::default(), clock.clone());
    let reference = native_reference(&port);
    let run = port.submit(&candidate2, &reference, &generate_submission_key(clock.now_ms())).unwrap();
    port.store.remove_run_pin("cand-start-next-missing", &run.run_id).unwrap();
    candidate::unpin(&candidates2, "cand-start-next-missing").unwrap();
    std::fs::remove_file(candidates2.join("cand-start-next-missing.zip")).unwrap();
    assert!(port.start_next().unwrap().is_none());
    assert_eq!(port.status(&run.run_id).unwrap().state, ValidationState::InfrastructureError);
    let _ = std::fs::remove_dir_all(root2);
}

#[test]
fn skill_validation_binds_the_exact_snapshot_profile_and_live_revision() {
    let root = scratch("skill-binding");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-skill-a", analytic_design());
    let later = write_candidate(&candidates, "cand-skill-b", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = text_to_cad_profile(
        vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }],
        vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 1000.0,
            abs_tolerance: 1e-6,
            rel_tolerance: 0.0,
        }],
    )
    .unwrap();
    let expected_profile = text_to_cad_profile_ref(&profile).unwrap();
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());
    let live = LiveDocument::new("doc-validation", 7);
    let report = validate_skill_snapshot(
        &port,
        SkillValidationRequest::new(&candidate, profile, generate_submission_key(clock.now_ms())).with_live(live.clone()),
    )
    .unwrap();
    assert_eq!(report.state, ValidationState::Passed);
    assert!(report.snapshot_passed);
    assert!(report.approved, "{report:?}");
    assert!(report.failure.is_none());
    assert_eq!(report.snapshot_id, "cand-skill-a");
    assert_eq!(report.candidate_digest, candidate.candidate_digest);
    assert_eq!(report.manifest_digest, candidate.manifest_digest);
    assert_eq!(report.document_id, "doc-validation");
    assert_eq!(report.document_revision, "7");
    assert_eq!(report.profile, expected_profile);
    assert_eq!(report.runner, LocalRunner::identity());
    assert_eq!(report.required_checks, TEXT_TO_CAD_REQUIRED_CHECKS);
    assert_eq!(report.checks.len(), TEXT_TO_CAD_REQUIRED_CHECKS.len());
    assert!(report.checks.iter().all(|check| check.required && check.passed && check.state == "passed"));
    assert!(!report.report.digest.is_empty());
    let summary = summary_fields(&report);
    assert_eq!(summary["snapshotId"], "cand-skill-a");
    assert_eq!(summary["documentRevision"], "7");
    assert_eq!(summary["profile"]["digest"], expected_profile.digest);
    assert_eq!(summary["snapshotPassed"], true);
    assert_eq!(summary["approved"], true);
    assert!(summary["failure"].is_null());
    assert!(report.approves(&candidate, Some(&live)));
    assert!(!report.approves(&candidate, None), "omitting the live document never approves");
    assert!(!report.approves(&candidate, Some(&LiveDocument::new("doc-validation", 8))), "the live revision moved");
    assert!(!report.approves(&later, Some(&live)), "snapshot A never approves snapshot B");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skill_validation_keeps_a_stale_live_document_as_historical_evidence() {
    let root = scratch("skill-stale-live");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-skill-stale", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = text_to_cad_profile(
        vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }],
        vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 1000.0,
            abs_tolerance: 1e-6,
            rel_tolerance: 0.0,
        }],
    )
    .unwrap();
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());
    let report = validate_skill_snapshot(
        &port,
        SkillValidationRequest::new(&candidate, profile, generate_submission_key(clock.now_ms())).with_live(LiveDocument::new("doc-validation", 8)),
    )
    .unwrap();
    assert_eq!(report.state, ValidationState::Passed, "the captured snapshot still passed");
    assert!(report.snapshot_passed);
    assert!(!report.approved, "a moved live document is not approval");
    assert!(report.failure.is_none(), "a passed snapshot is historical evidence, not a failure");
    assert!(report.reason.contains("historical"), "{}", report.reason);
    assert!(!report.approves(&candidate, Some(&LiveDocument::new("doc-validation", 8))));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skill_validation_does_not_approve_a_quality_failure() {
    let root = scratch("skill-quality-failure");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-skill-failed", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = text_to_cad_profile(
        vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }],
        vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 999.0,
            abs_tolerance: 1e-6,
            rel_tolerance: 0.0,
        }],
    )
    .unwrap();
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());
    let report = validate_skill_snapshot(
        &port,
        SkillValidationRequest::new(&candidate, profile, generate_submission_key(clock.now_ms())).with_live(LiveDocument::new("doc-validation", 7)),
    )
    .unwrap();
    assert_eq!(report.state, ValidationState::QualityFailed);
    assert!(!report.approved);
    assert!(!report.snapshot_passed);
    assert_eq!(report.failure.as_ref().map(|failure| failure.kind), Some(SkillFailureKind::QualityFailed));
    assert_eq!(report.failure.as_ref().and_then(|failure| failure.code.as_deref()), None);
    assert!(!report.approves(&candidate, Some(&LiveDocument::new("doc-validation", 7))));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skill_validation_keeps_an_infrastructure_failure_distinct_from_a_quality_failure() {
    let root = scratch("skill-infrastructure-failure");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-skill-infrastructure", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    // `solid.box` is a mutating command, not one of the read-only exact-geometry commands, so
    // the real local runner reports it as unsupported even though the snapshot itself is sound.
    let profile = text_to_cad_profile(
        vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }],
        vec![MeasurementRequirement {
            command: "solid.box".into(),
            params: json!({}),
            result_pointer: "/body_count".into(),
            expected: 1.0,
            abs_tolerance: 0.0,
            rel_tolerance: 0.0,
        }],
    )
    .unwrap();
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());
    let report = validate_skill_snapshot(
        &port,
        SkillValidationRequest::new(&candidate, profile, generate_submission_key(clock.now_ms())).with_live(LiveDocument::new("doc-validation", 7)),
    )
    .unwrap();
    assert_eq!(report.state, ValidationState::InfrastructureError);
    assert_ne!(report.state, ValidationState::QualityFailed, "an unsupported check is not a geometric failure");
    assert!(!report.approved);
    assert!(!report.snapshot_passed);
    assert_eq!(report.failure.as_ref().map(|failure| failure.kind), Some(SkillFailureKind::InfrastructureError));
    assert!(report.reason.contains("infrastructure_error"), "{}", report.reason);
    let geometry = report.checks.iter().find(|check| check.check_id == "geometry.exact").unwrap();
    assert_eq!(geometry.state, "unsupported");
    assert!(!report.approves(&candidate, Some(&LiveDocument::new("doc-validation", 7))));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skill_validation_requires_live_for_approval() {
    let root = scratch("skill-no-live");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-skill-no-live", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());
    let report =
        validate_skill_snapshot(&port, SkillValidationRequest::new(&candidate, beta_skill_profile(), generate_submission_key(clock.now_ms())))
            .unwrap();
    assert_eq!(report.state, ValidationState::Passed);
    assert!(report.snapshot_passed, "the captured snapshot passed on its own");
    assert!(!report.approved, "omitting the live document can never claim approval");
    assert!(report.failure.is_none());
    assert!(report.reason.contains("snapshot evidence"), "{}", report.reason);
    assert!(!report.approves(&candidate, None));
    assert!(!report.approves(&candidate, Some(&LiveDocument::new("doc-validation", 7))), "an unapproved report cannot be approved retroactively");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skill_validation_reports_a_timeout_as_timeout_not_generic_infrastructure() {
    let root = scratch("skill-timeout");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-skill-timeout", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let timeout = Arc::new(FixedRunner {
        identity: LocalRunner::identity(),
        outcome: RunnerOutcome::TimedOut { started_at_ms: Some(1), finished_at_ms: 2, message: "too slow".into() },
    });
    let port = port_with(&store, &candidates, timeout, Limits::default(), clock.clone());
    let report = validate_skill_snapshot(
        &port,
        SkillValidationRequest::new(&candidate, beta_skill_profile(), generate_submission_key(clock.now_ms()))
            .with_live(LiveDocument::new("doc-validation", 7)),
    )
    .unwrap();
    assert_eq!(report.state, ValidationState::InfrastructureError);
    assert!(!report.snapshot_passed);
    assert!(!report.approved);
    let failure = report.failure.as_ref().expect("a timeout is a structured failure");
    assert_eq!(failure.kind, SkillFailureKind::Timeout);
    assert_eq!(failure.code.as_deref(), Some("TIMEOUT"));
    assert!(report.reason.contains("TIMEOUT"), "{}", report.reason);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skill_validation_reports_a_worker_failure_as_infrastructure_not_timeout() {
    let root = scratch("skill-worker-failure");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-skill-worker-failure", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let failure = Arc::new(FixedRunner {
        identity: LocalRunner::identity(),
        outcome: RunnerOutcome::Infrastructure { started_at_ms: Some(1), finished_at_ms: 2, message: "worker crashed".into() },
    });
    let port = port_with(&store, &candidates, failure, Limits::default(), clock.clone());
    let report = validate_skill_snapshot(
        &port,
        SkillValidationRequest::new(&candidate, beta_skill_profile(), generate_submission_key(clock.now_ms()))
            .with_live(LiveDocument::new("doc-validation", 7)),
    )
    .unwrap();
    assert_eq!(report.state, ValidationState::InfrastructureError);
    assert!(!report.approved);
    let failure = report.failure.as_ref().expect("a worker failure is a structured failure");
    assert_eq!(failure.kind, SkillFailureKind::InfrastructureError);
    assert_eq!(failure.code.as_deref(), Some("INTERNAL_ERROR"));
    assert_ne!(failure.kind, SkillFailureKind::Timeout);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn skill_validation_refuses_a_weakened_profile_or_an_unsupported_required_check() {
    let root = scratch("skill-profile-guard");
    let candidates = root.join("candidates");
    let store = root.join("validation");
    let candidate = write_candidate(&candidates, "cand-skill-profile-guard", analytic_design());
    let clock = Arc::new(ManualClock::new(1_760_000_000_000));
    let profile = text_to_cad_profile(
        vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }],
        vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 1000.0,
            abs_tolerance: 1e-6,
            rel_tolerance: 0.0,
        }],
    )
    .unwrap();
    let port = port_with(&store, &candidates, Arc::new(LocalRunner), Limits::default(), clock.clone());

    let mut weakened = profile.clone();
    weakened.checks.retain(|check| check.check_id != "geometry.exact");
    let error =
        validate_skill_snapshot(&port, SkillValidationRequest::new(&candidate, weakened, generate_submission_key(clock.now_ms()))).unwrap_err();
    assert_eq!(error.code, ErrorCode::ProfileUnsupported);

    let mut widened = SkillValidationRequest::new(&candidate, profile.clone(), generate_submission_key(clock.now_ms()));
    widened.required_checks.push("measurement.future".into());
    let error = validate_skill_snapshot(&port, widened).unwrap_err();
    assert_eq!(error.code, ErrorCode::ProfileUnsupported);

    let mut narrowed = SkillValidationRequest::new(&candidate, profile, generate_submission_key(clock.now_ms()));
    narrowed.required_checks.retain(|check| check != "geometry.exact");
    let error = validate_skill_snapshot(&port, narrowed).unwrap_err();
    assert_eq!(error.code, ErrorCode::ProfileUnsupported, "a narrowed coverage list cannot silently approve");
    assert_eq!(port.store.count_admissions(), 0, "a refused profile is never admitted");
    let _ = std::fs::remove_dir_all(root);
}
