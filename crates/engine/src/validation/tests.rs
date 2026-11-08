//! Validation port tests: state machine, profiles, idempotency, reports and the local runner.

use std::path::{Path, PathBuf};
use std::sync::Arc;
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

fn manifest(candidate_id: &str, files: &[CandidateFile]) -> Value {
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
        "engineVersion": env!("CARGO_PKG_VERSION"),
        "inputs": inputs,
    })
}

fn write_candidate(root: &Path, id: &str, design: Vec<u8>) -> crate::schema::snapshot::CandidateRef {
    let files = vec![CandidateFile { path: "design.sc".into(), bytes: design }];
    let stored = candidate::write(root, id, &files, &manifest(id, &files), &format!("artifact-{id}")).unwrap();
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
    let bytes = std::fs::read(store.join("runs").join(format!("{}.report.json", submitted.run_id))).unwrap();
    assert_eq!(report.reference.digest, super::ids::digest(&bytes));
    assert_eq!(report.reference.size_bytes, bytes.len() as u64);
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
    assert_eq!(error.code, ErrorCode::ReportMismatch);
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
    assert!(matches!(error.code, ErrorCode::InvalidCandidate | ErrorCode::DependencyChanged), "{error}");
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
    assert_eq!(port.report(&run.run_id).unwrap_err().code, ErrorCode::ReportMismatch);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}
