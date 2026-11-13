//! The local headless runner: load the immutable candidate in a fresh session and evaluate exact
//! native checks. It never touches the live document or the user's file.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use super::ids::digest;
use super::profile::{CheckProfile, MeasurementRequirement, measurement_requirements, parameter_requirements};
use super::report::{CheckState, RunnerIdentity};
use super::{Clock, ValidationError};
use crate::Session;
use crate::schema::snapshot::CandidateRef;

/// The built-in runner identity.
pub const LOCAL_RUNNER_ID: &str = "local-solidcraft";
/// The runner protocol implementation version.
pub const LOCAL_RUNNER_VERSION: &str = "1.0.0";

/// Read-only commands whose numeric results are exact geometry, never a display mesh.
/// A measurement naming any other command is recorded as unsupported.
const EXACT_COMMANDS: &[&str] = &["inspect.measure", "model.faces", "model.edges", "document.inspect"];

/// Evidence bytes a runner produces; the port assigns the artifact identity and verifies digests.
#[derive(Clone, Debug)]
pub struct EvidenceBytes {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// One check's result as the worker found it.
#[derive(Clone, Debug)]
pub struct CheckOutcome {
    pub check_id: String,
    pub state: CheckState,
    pub expected: Value,
    pub actual: Value,
    pub evidence: Vec<EvidenceBytes>,
}

/// The worker's conclusion. The port decides the durable terminal state and report.
#[derive(Clone, Debug)]
pub enum RunnerOutcome {
    Finished { started_at_ms: u64, finished_at_ms: u64, checks: Vec<CheckOutcome> },
    Infrastructure { started_at_ms: Option<u64>, finished_at_ms: u64, message: String },
    TimedOut { started_at_ms: Option<u64>, finished_at_ms: u64, message: String },
    Cancelled { started_at_ms: Option<u64>, finished_at_ms: u64, checks: Vec<CheckOutcome> },
    Unknown { started_at_ms: Option<u64>, finished_at_ms: u64, message: String },
}

/// Everything a runner may read for one admitted job.
#[derive(Clone, Debug)]
pub struct RunJob {
    pub run_id: String,
    pub candidate: CandidateRef,
    pub candidate_root: PathBuf,
    pub profile: CheckProfile,
    pub started_at_ms: u64,
}

/// Cooperative cancellation and the execution deadline.
pub struct RunControl {
    pub(super) clock: Arc<dyn Clock>,
    pub(super) deadline_ms: u64,
    pub(super) cancel_path: PathBuf,
}

impl RunControl {
    /// Has a durable cancellation marker been persisted?
    pub fn cancelled(&self) -> bool {
        self.cancel_path.is_file()
    }

    /// Has the execution deadline passed?
    pub fn timed_out(&self) -> bool {
        self.clock.now_ms() >= self.deadline_ms
    }
}

/// A validation worker. The port owns durable state; a runner only computes checks.
pub trait Runner: Send + Sync {
    fn identity(&self) -> RunnerIdentity;
    fn run(&self, job: &RunJob, control: &RunControl) -> RunnerOutcome;
}

/// The built-in local runner.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalRunner;

impl LocalRunner {
    pub fn identity() -> RunnerIdentity {
        RunnerIdentity { runner_id: LOCAL_RUNNER_ID.into(), version: LOCAL_RUNNER_VERSION.into(), engine_version: env!("CARGO_PKG_VERSION").into() }
    }
}

impl Runner for LocalRunner {
    fn identity(&self) -> RunnerIdentity {
        Self::identity()
    }

    fn run(&self, job: &RunJob, control: &RunControl) -> RunnerOutcome {
        let started = job.started_at_ms;
        let finish = |checks: Vec<CheckOutcome>| RunnerOutcome::Finished { started_at_ms: started, finished_at_ms: control.clock.now_ms(), checks };
        if control.cancelled() {
            return RunnerOutcome::Cancelled { started_at_ms: Some(started), finished_at_ms: control.clock.now_ms(), checks: Vec::new() };
        }
        if control.timed_out() {
            return RunnerOutcome::TimedOut {
                started_at_ms: Some(started),
                finished_at_ms: control.clock.now_ms(),
                message: "the validation deadline passed before the worker started".into(),
            };
        }
        let verified = match verify_candidate(&job.candidate_root, &job.candidate) {
            Ok(verified) => verified,
            Err(error) => {
                return RunnerOutcome::Infrastructure {
                    started_at_ms: Some(started),
                    finished_at_ms: control.clock.now_ms(),
                    message: error.to_string(),
                };
            }
        };
        let workspace = match Workspace::extract(&job.run_id, &verified.files) {
            Ok(workspace) => workspace,
            Err(message) => {
                return RunnerOutcome::Infrastructure { started_at_ms: Some(started), finished_at_ms: control.clock.now_ms(), message };
            }
        };
        let mut checks = Vec::with_capacity(job.profile.checks.len());
        // `input.integrity` is a property of the loaded candidate, independent of the design.
        checks.push(integrity_check(&verified));
        if let Some(outcome) = interrupted(control, started, &checks) {
            workspace.remove();
            return outcome;
        }
        // `native.rebuild` opens the captured design in an isolated session, before any task
        // check that needs that session.
        let mut session = None;
        if job.profile.checks.iter().any(|check| check.kind == "native.rebuild") {
            if let Some(outcome) = interrupted(control, started, &checks) {
                workspace.remove();
                return outcome;
            }
            let (check, opened) = rebuild_check(&workspace, &verified, session);
            session = opened;
            checks.push(check);
            if let Some(outcome) = interrupted(control, started, &checks) {
                workspace.remove();
                return outcome;
            }
        }
        for requirement in &job.profile.checks {
            if requirement.kind == "input.integrity" || requirement.kind == "native.rebuild" {
                continue;
            }
            if let Some(outcome) = interrupted(control, started, &checks) {
                workspace.remove();
                return outcome;
            }
            let outcome = match requirement.kind.as_str() {
                "parameters.required" => parameter_check(session.as_mut(), requirement),
                "geometry.exact" => geometry_check(session.as_mut(), requirement),
                "dfm.sheet" => dfm_sheet_check(session.as_ref(), requirement),
                other => CheckOutcome {
                    check_id: requirement.check_id.clone(),
                    state: CheckState::Unsupported,
                    expected: json!({"kind": other}),
                    actual: json!({"unsupported": other}),
                    evidence: vec![evidence_json(&json!({"unsupported": other}))],
                },
            };
            checks.push(outcome);
            if let Some(outcome) = interrupted(control, started, &checks) {
                workspace.remove();
                return outcome;
            }
        }
        workspace.remove();
        finish(checks)
    }
}

fn interrupted(control: &RunControl, started: u64, checks: &[CheckOutcome]) -> Option<RunnerOutcome> {
    if control.cancelled() {
        Some(RunnerOutcome::Cancelled { started_at_ms: Some(started), finished_at_ms: control.clock.now_ms(), checks: checks.to_vec() })
    } else if control.timed_out() {
        Some(RunnerOutcome::TimedOut {
            started_at_ms: Some(started),
            finished_at_ms: control.clock.now_ms(),
            message: "the validation execution deadline passed".into(),
        })
    } else {
        None
    }
}

fn evidence_json(value: &Value) -> EvidenceBytes {
    EvidenceBytes { media_type: "application/json".into(), bytes: serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()) }
}

/// Verify a supplied `CandidateRef` against the host store and return the exact inputs.
pub fn verify_candidate(candidate_root: &Path, expected: &CandidateRef) -> Result<solidcraft_io::candidate::VerifiedCandidate, ValidationError> {
    if !super::ids::valid_id(&expected.candidate_id) {
        return Err(ValidationError::invalid_candidate("candidateId must be a valid Id"));
    }
    if !super::ids::valid_digest(&expected.candidate_digest) || !super::ids::valid_digest(&expected.manifest_digest) {
        return Err(ValidationError::invalid_candidate("candidate digests must be sha256:<64 lowercase hex>"));
    }
    let verified = solidcraft_io::candidate::lookup(candidate_root, &expected.candidate_id).map_err(|error| {
        let text = error.to_string();
        if text.contains("NOT_FOUND") { ValidationError::not_found(text) } else { ValidationError::invalid_candidate(text) }
    })?;
    if verified.candidate.candidate_digest != expected.candidate_digest
        || verified.candidate.manifest_digest != expected.manifest_digest
        || verified.candidate.artifact.artifact_id != expected.artifact.artifact_id
        || verified.candidate.artifact.digest != expected.artifact.digest
        || verified.candidate.artifact.size_bytes != expected.artifact.size_bytes
        || verified.candidate.artifact.media_type != expected.artifact.media_type
    {
        return Err(ValidationError::invalid_candidate("the supplied CandidateRef does not match the persisted candidate"));
    }
    let manifest = &verified.manifest;
    if manifest.get("candidateId").and_then(Value::as_str) != Some(expected.candidate_id.as_str()) {
        return Err(ValidationError::invalid_candidate("the manifest names another candidate"));
    }
    if manifest.get("source") != Some(&serde_json::to_value(&expected.source).unwrap_or(Value::Null)) {
        return Err(ValidationError::dependency_changed("the candidate source stamp does not match its manifest"));
    }
    Ok(verified)
}

fn integrity_check(verified: &solidcraft_io::candidate::VerifiedCandidate) -> CheckOutcome {
    let manifest_inputs = verified.manifest.get("inputs").and_then(Value::as_array).cloned().unwrap_or_default();
    let inputs: Vec<Value> = verified
        .files
        .iter()
        .map(|file| {
            let manifest = manifest_inputs.iter().find(|input| input.get("path").and_then(Value::as_str) == Some(file.path.as_str()));
            json!({
                "path": file.path,
                "kind": manifest.and_then(|input| input.get("kind")).cloned().unwrap_or(Value::Null),
                "dependencyId": manifest.and_then(|input| input.get("dependencyId")).cloned().unwrap_or(Value::Null),
                "dependencyVersion": manifest.and_then(|input| input.get("dependencyVersion")).cloned().unwrap_or(Value::Null),
                "digest": digest(&file.bytes),
                "sizeBytes": file.bytes.len(),
            })
        })
        .collect();
    let actual = json!({
        "candidateDigest": verified.candidate.candidate_digest,
        "manifestDigest": verified.candidate.manifest_digest,
        "artifactDigest": verified.candidate.artifact.digest,
        "inputs": inputs,
        "verified": true,
    });
    CheckOutcome {
        check_id: "input.integrity".into(),
        state: CheckState::Passed,
        expected: json!({"manifestDigest": verified.candidate.manifest_digest, "inputs": verified.files.len()}),
        actual: actual.clone(),
        evidence: vec![evidence_json(&actual)],
    }
}

fn rebuild_check(
    workspace: &Workspace,
    verified: &solidcraft_io::candidate::VerifiedCandidate,
    existing: Option<Session>,
) -> (CheckOutcome, Option<Session>) {
    let expected = json!({"errors": 0, "unresolvedDependencies": 0});
    let design = match workspace.design_bytes(verified) {
        Ok(bytes) => bytes,
        Err(message) => return (failed_rebuild(expected, message), existing),
    };
    let document = match solidcraft_io::read_design(&design) {
        Ok(document) => document,
        Err(error) => return (failed_rebuild(expected, format!("the captured design does not open: {error}")), existing),
    };
    // Every link must resolve to a captured input inside this isolated workspace.
    let mut unresolved = Vec::new();
    for (component, link) in &document.links {
        let path = Path::new(&link.path);
        if path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir)) {
            unresolved.push(json!({"component": component, "path": link.path, "error": "not a relative captured input"}));
            continue;
        }
        let resolved = workspace.root.join(path);
        if !resolved.starts_with(&workspace.root) || !resolved.is_file() {
            unresolved.push(json!({"component": component, "path": link.path, "error": "unresolved dependency"}));
        }
    }
    if !unresolved.is_empty() {
        let actual = json!({"errors": unresolved.len(), "unresolvedDependencies": unresolved.len(), "unresolved": unresolved});
        return (
            CheckOutcome {
                check_id: "native.rebuild".into(),
                state: CheckState::Failed,
                expected,
                actual: actual.clone(),
                evidence: vec![evidence_json(&actual)],
            },
            existing,
        );
    }
    let design_path = workspace.root.join("design.sc");
    let design_path = design_path.to_string_lossy().into_owned();
    let session = Session::opened_at(document, None, Some(&design_path));
    let param_errors: Vec<Value> = session.model.param_errors.iter().map(|(name, error)| json!({"name": name, "error": error})).collect();
    let feature_errors: Vec<Value> = session
        .model
        .results
        .iter()
        .filter_map(|result| result.error.as_ref().map(|error| json!({"feature": result.name, "error": error})))
        .collect();
    let warnings: Vec<Value> = session
        .model
        .results
        .iter()
        .filter_map(|result| result.warning.as_ref().map(|warning| json!({"feature": result.name, "warning": warning})))
        .collect();
    let world_error = session.world_state().err().map(|error| error.to_string());
    let errors = param_errors.len() + feature_errors.len() + usize::from(world_error.is_some());
    let actual = json!({
        "engineVersion": verified.manifest.get("engineVersion").cloned().unwrap_or(Value::Null),
        "formatVersion": verified.manifest.get("formatVersion").cloned().unwrap_or(Value::Null),
        "features": session.model.results.len(),
        "errors": errors,
        "unresolvedDependencies": 0,
        "paramErrors": param_errors,
        "featureErrors": feature_errors,
        "warnings": warnings,
        "worldError": world_error,
    });
    let state = if errors == 0 { CheckState::Passed } else { CheckState::Failed };
    (
        CheckOutcome { check_id: "native.rebuild".into(), state, expected, actual: actual.clone(), evidence: vec![evidence_json(&actual)] },
        Some(session),
    )
}

fn failed_rebuild(expected: Value, message: String) -> CheckOutcome {
    let actual = json!({"errors": 1, "unresolvedDependencies": 0, "error": message});
    CheckOutcome {
        check_id: "native.rebuild".into(),
        state: CheckState::Failed,
        expected,
        actual: actual.clone(),
        evidence: vec![evidence_json(&actual)],
    }
}

fn parameter_check(session: Option<&mut Session>, check: &super::profile::CheckRequirement) -> CheckOutcome {
    let requirements = match parameter_requirements(check) {
        Ok(requirements) => requirements,
        Err(error) => return error_check(check, error),
    };
    let Some(session) = session else {
        return not_run(check, "native.rebuild did not produce a session");
    };
    let rows = session.doc.param_rows();
    let mut actual_values = serde_json::Map::new();
    let mut missing = Vec::new();
    let mut failed = Vec::new();
    for requirement in &requirements {
        match rows.iter().find(|row| row.name == requirement.name).and_then(|row| row.value) {
            Some(value) if value.is_finite() => {
                actual_values.insert(requirement.name.clone(), json!(value));
                if !passes(value, requirement.value, requirement.abs_tolerance, 0.0) {
                    failed.push(json!({
                        "name": requirement.name,
                        "expected": requirement.value,
                        "actual": value,
                        "absTolerance": requirement.abs_tolerance,
                    }));
                }
            }
            _ => missing.push(requirement.name.clone()),
        }
    }
    let actual = json!({"parameters": actual_values, "missing": missing, "failed": failed});
    let state = if missing.is_empty() && failed.is_empty() { CheckState::Passed } else { CheckState::Failed };
    CheckOutcome {
        check_id: check.check_id.clone(),
        state,
        expected: check.parameters.clone(),
        actual: actual.clone(),
        evidence: vec![evidence_json(&json!({"expected": check.parameters, "actual": actual}))],
    }
}

fn geometry_check(session: Option<&mut Session>, check: &super::profile::CheckRequirement) -> CheckOutcome {
    let measurements = match measurement_requirements(check) {
        Ok(measurements) => measurements,
        Err(error) => return error_check(check, error),
    };
    let Some(session) = session else {
        return not_run(check, "native.rebuild did not produce a session");
    };
    let mut actual = Vec::with_capacity(measurements.len());
    let mut unsupported = Vec::new();
    let mut failed = false;
    for measurement in &measurements {
        match measure_one(session, measurement) {
            Ok(value) => actual.push(value),
            Err(MeasureError::Unsupported(why)) => unsupported.push(json!({"command": measurement.command, "reason": why})),
            Err(MeasureError::Failed(why)) => {
                failed = true;
                actual.push(json!({"command": measurement.command, "pointer": measurement.result_pointer, "error": why}));
            }
        }
    }
    let state = if !unsupported.is_empty() {
        CheckState::Unsupported
    } else if failed {
        CheckState::Failed
    } else {
        CheckState::Passed
    };
    let actual = json!({"measurements": actual, "unsupported": unsupported});
    CheckOutcome {
        check_id: check.check_id.clone(),
        state,
        expected: check.parameters.clone(),
        actual: actual.clone(),
        evidence: vec![evidence_json(&json!({"expected": check.parameters, "actual": actual}))],
    }
}

enum MeasureError {
    Unsupported(String),
    Failed(String),
}

fn measure_one(session: &mut Session, measurement: &MeasurementRequirement) -> Result<Value, MeasureError> {
    if !EXACT_COMMANDS.contains(&measurement.command.as_str()) {
        return Err(MeasureError::Unsupported(format!("{} is not a supported exact-geometry command", measurement.command)));
    }
    let Some(spec) = crate::find_command(&measurement.command) else {
        return Err(MeasureError::Unsupported(format!("{} is not an available command", measurement.command)));
    };
    if spec.undoable || spec.planned.is_some() {
        return Err(MeasureError::Unsupported(format!("{} is not a read-only exact measurement", measurement.command)));
    }
    let reply = session.execute(&measurement.command, &measurement.params).map_err(|error| MeasureError::Failed(error.to_string()))?;
    let value = reply
        .pointer(&measurement.result_pointer)
        .ok_or_else(|| MeasureError::Failed(format!("{} has no result at {}", measurement.command, measurement.result_pointer)))?;
    let actual =
        value.as_f64().ok_or_else(|| MeasureError::Failed(format!("{} at {} is not a number", measurement.command, measurement.result_pointer)))?;
    if !actual.is_finite() {
        return Err(MeasureError::Failed(format!("{} at {} is not finite", measurement.command, measurement.result_pointer)));
    }
    if passes(actual, measurement.expected, measurement.abs_tolerance, measurement.rel_tolerance) {
        Ok(json!({
            "command": measurement.command,
            "pointer": measurement.result_pointer,
            "expected": measurement.expected,
            "actual": actual,
            "passed": true,
        }))
    } else {
        Err(MeasureError::Failed(format!(
            "{} at {}: expected {}, measured {} (abs {}, rel {})",
            measurement.command, measurement.result_pointer, measurement.expected, actual, measurement.abs_tolerance, measurement.rel_tolerance
        )))
    }
}

fn passes(actual: f64, expected: f64, abs_tolerance: f64, rel_tolerance: f64) -> bool {
    (actual - expected).abs() <= f64::max(abs_tolerance, rel_tolerance * expected.abs())
}

/// One native sheet body through the pure DFM rule crate. The check's evidence is the measured
/// facts and the whole rule report, so a later review can recompute the decision.
fn dfm_sheet_check(session: Option<&Session>, check: &super::profile::CheckRequirement) -> CheckOutcome {
    let parameters = match crate::dfm::validate_parameters(&check.parameters) {
        Ok(parameters) => parameters,
        Err(error) => return error_check(check, ValidationError::invalid_request(error.to_string())),
    };
    let Some(session) = session else {
        return not_run(check, "native.rebuild did not produce a session");
    };
    let state = session.model.state();
    let sheet = state.sheets.get(parameters.sheet).filter(|sheet| sheet.body == parameters.body);
    let Some(sheet) = sheet else {
        return error_check(
            check,
            ValidationError::invalid_request(format!("sheet {} is not `{}` in the evaluated model", parameters.sheet, parameters.body)),
        );
    };
    let expected = json!({
        "rulesVersion": solidcraft_dfm_rules::VERSION,
        "generic": {"name": parameters.generic.name, "version": parameters.generic.version},
        "shop": parameters.shop.as_ref().map(|shop| json!({"name": shop.name, "version": shop.version})),
    });
    match crate::dfm::check_sheet(sheet, parameters) {
        Ok(outcome) => {
            let actual = json!({
                "rulesVersion": solidcraft_dfm_rules::VERSION,
                "modelVersion": outcome.report.model.model_version,
                "clean": outcome.report.clean,
                "counts": outcome.report.counts,
                "rules": outcome.report.rules,
            });
            let check_state = if outcome.report.counts.failed > 0 {
                CheckState::Failed
            } else if outcome.report.counts.unsupported > 0 {
                CheckState::Unsupported
            } else {
                CheckState::Passed
            };
            let facts = serde_json::to_value(&outcome.facts).unwrap_or(Value::Null);
            let report = serde_json::to_value(&outcome.report).unwrap_or(Value::Null);
            CheckOutcome {
                check_id: check.check_id.clone(),
                state: check_state,
                expected,
                actual,
                evidence: vec![evidence_json(&facts), evidence_json(&report)],
            }
        }
        Err(crate::dfm::DfmError::Unsupported(reason)) => CheckOutcome {
            check_id: check.check_id.clone(),
            state: CheckState::Unsupported,
            expected: expected.clone(),
            actual: json!({"reason": reason}),
            evidence: vec![evidence_json(&json!({"reason": reason}))],
        },
        Err(error) => error_check(check, ValidationError::invalid_request(error.to_string())),
    }
}

fn error_check(check: &super::profile::CheckRequirement, error: ValidationError) -> CheckOutcome {
    let actual = json!({"error": error.to_string()});
    CheckOutcome {
        check_id: check.check_id.clone(),
        state: CheckState::Error,
        expected: check.parameters.clone(),
        actual: actual.clone(),
        evidence: vec![evidence_json(&actual)],
    }
}

fn not_run(check: &super::profile::CheckRequirement, why: &str) -> CheckOutcome {
    CheckOutcome {
        check_id: check.check_id.clone(),
        state: CheckState::NotRun,
        expected: check.parameters.clone(),
        actual: json!({"reason": why}),
        evidence: Vec::new(),
    }
}

/// The candidate's exact files written to a private scratch directory, so the design's relative
/// links resolve without the live filesystem or the original container.
struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn extract(run_id: &str, files: &[solidcraft_io::candidate::CandidateFile]) -> Result<Self, String> {
        let base = solidcraft_io::vfs::temp_dir();
        let root = base.join(format!("solidcraft-validation-{run_id}-{}", std::process::id()));
        // A previous crashed run must not leak inputs into this one.
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).map_err(|error| format!("validation workspace: {error}"))?;
        for file in files {
            let path = relative_path(&root, &file.path)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| format!("validation workspace: {error}"))?;
            }
            fs::write(&path, &file.bytes).map_err(|error| format!("validation workspace: {error}"))?;
        }
        Ok(Workspace { root })
    }

    fn design_bytes(&self, verified: &solidcraft_io::candidate::VerifiedCandidate) -> Result<Vec<u8>, String> {
        verified
            .files
            .iter()
            .find(|file| file.path == "design.sc")
            .map(|file| file.bytes.clone())
            .ok_or_else(|| "the candidate has no design.sc input".to_string())
    }

    fn remove(&self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn relative_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let mut out = root.to_path_buf();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', '\0']) {
            return Err(format!("unsafe candidate input path {path:?}"));
        }
        out.push(part);
    }
    if !out.starts_with(root) {
        return Err(format!("unsafe candidate input path {path:?}"));
    }
    Ok(out)
}

/// Convenience for hosts and tests: the runner's identity and the candidate/profile it binds.
pub fn local_runner_identity() -> RunnerIdentity {
    LocalRunner::identity()
}
