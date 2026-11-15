//! The durable project-side record of DFM runs (CONTRACT §3).
//!
//! A run lives in `<project>/.intcad/checks/runs/<run-id>/` while its results are written, and
//! `checks/latest.json` points at the newest submitted run only after every file is complete. A
//! run that returns after a newer submission is recorded as `superseded` in its own folder and
//! never takes `latest` back; a cancelled or failed run leaves the previous record alone.
//!
//! `run.json` is the only file with times. Everything else is deterministic so the same facts and
//! configuration give the same bytes when the report is read again.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::dfm::compile::{self, RunInput, SheetRef};
use crate::dfm::findings::{self, FindingsIdentity, SheetCheck, SourceRef};
use crate::schema::snapshot::CandidateRef;
use crate::validation::CheckProfile;
use crate::validation::ValidationReport;
use crate::validation::{self, CheckState, ValidationPort, ValidationState};

use super::findings::RunFindings;

/// The folder stream A creates and discovers at the project root. The run layer builds only the
/// paths below it and never creates the folder itself.
pub const PROJECT_FOLDER: &str = ".intcad";
/// The checks folder under the project folder.
const CHECKS: &str = "checks";
/// The run folders.
const RUNS: &str = "runs";
/// The newest complete run's identity.
const LATEST: &str = "latest.json";
/// The internal submission-order marker (no times, not a report).
const SUBMITTED: &str = "submitted.json";
/// The run record's schema version.
pub const RUN_SCHEMA_VERSION: u32 = 1;

/// What happened to a run once it stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// The newest submitted run: it is what `latest.json` points at.
    Current,
    /// A newer run was submitted before this one returned.
    Superseded,
    /// The user cancelled it.
    Cancelled,
    /// It could not produce a complete record (a rebuild or write failure).
    Failed,
}

/// The reason recorded when a run cannot publish.
fn superseded_reason(meta: &RunMeta, outcome: RunOutcome) -> Option<String> {
    match outcome {
        RunOutcome::Superseded => Some("a newer DFM run was submitted before this one finished".into()),
        _ => meta.reason.clone(),
    }
}

/// One run's durable identity and times. Only this record holds times.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMeta {
    pub schema_version: u32,
    pub run_id: String,
    pub project_id: String,
    /// The model snapshot digest; what staleness compares.
    pub candidate_digest: String,
    /// The frozen project configuration digest.
    pub config_digest: String,
    pub model_version: String,
    /// The captured document revision (in-session freshness compares it without a new snapshot).
    #[serde(default)]
    pub source_revision: String,
    /// RFC 3339 UTC with milliseconds.
    pub started_at: String,
    pub ended_at: String,
    pub outcome: RunOutcome,
    /// Why a run was superseded, cancelled or failed. Never a pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl RunMeta {
    /// Build a finished run record. Times are passed in, so a host and a test decide them.
    pub fn new(
        run_id: &str,
        project_id: &str,
        candidate_digest: &str,
        config_digest: &str,
        model_version: &str,
        started_at: &str,
        ended_at: &str,
        outcome: RunOutcome,
        reason: Option<String>,
    ) -> RunMeta {
        RunMeta {
            schema_version: RUN_SCHEMA_VERSION,
            run_id: run_id.into(),
            project_id: project_id.into(),
            candidate_digest: candidate_digest.into(),
            config_digest: config_digest.into(),
            model_version: model_version.into(),
            source_revision: String::new(),
            started_at: started_at.into(),
            ended_at: ended_at.into(),
            outcome,
            reason,
        }
    }
}

/// The pointer a reader finds without scanning every run folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestRun {
    pub schema_version: u32,
    pub project_id: String,
    pub run_id: String,
    pub candidate_digest: String,
    pub config_digest: String,
    pub model_version: String,
    pub profile_digest: String,
    pub source_kind: String,
}

/// The internal submission-order marker. It holds identities, never times.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmittedRun {
    schema_version: u32,
    project_id: String,
    run_id: String,
    candidate_digest: String,
    config_digest: String,
    model_version: String,
}

/// How the report's identity relates to the live model and configuration (CONTRACT §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Freshness {
    Current,
    StaleModel,
    StaleConfig,
    StaleBoth,
    /// The live identity could not be read (no project, a damaged record, a closed design).
    Unknown,
}

/// Decide whether a persisted run still describes the live model and configuration.
///
/// `current_model` is the live candidate digest (or, in one session, a comparable model identity);
/// `current_config` is the live configuration digest. `None` means unknown, and unknown is never
/// `Current`.
pub fn freshness(run: &RunMeta, current_model: Option<&str>, current_config: Option<&str>) -> Freshness {
    let (Some(model), Some(config)) = (current_model, current_config) else { return Freshness::Unknown };
    let model_changed = model != run.candidate_digest;
    let config_changed = config != run.config_digest;
    match (model_changed, config_changed) {
        (false, false) => Freshness::Current,
        (true, false) => Freshness::StaleModel,
        (false, true) => Freshness::StaleConfig,
        (true, true) => Freshness::StaleBoth,
    }
}

impl LatestRun {
    /// [`freshness`] for a pointer, using the identities it carries.
    pub fn freshness(&self, current_model: Option<&str>, current_config: Option<&str>) -> Freshness {
        let (Some(model), Some(config)) = (current_model, current_config) else { return Freshness::Unknown };
        let model_changed = model != self.candidate_digest;
        let config_changed = config != self.config_digest;
        match (model_changed, config_changed) {
            (false, false) => Freshness::Current,
            (true, false) => Freshness::StaleModel,
            (false, true) => Freshness::StaleConfig,
            (true, true) => Freshness::StaleBoth,
        }
    }
}

/// The per-sheet evidence a complete run keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetEvidence {
    /// The sheet body's index in the evaluated model.
    pub sheet: usize,
    pub body: String,
    /// The App's measured facts.
    pub facts: serde_json::Value,
    /// The rule layer's complete report for this body.
    pub rules: serde_json::Value,
}

/// Every file a complete run writes.
#[derive(Clone, Debug)]
pub struct RunFiles {
    pub profile: CheckProfile,
    pub report: ValidationReport,
    pub findings: RunFindings,
    pub sheets: Vec<SheetEvidence>,
}

/// Why a run's record could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum RunStoreError {
    #[error("{path}: {message}")]
    Io { path: String, message: String },
    #[error("{path}: {message}")]
    Json { path: String, message: String },
    #[error("{0}")]
    Invalid(String),
    #[error("validation: {0}")]
    Validation(String),
}

impl RunStoreError {
    fn io(path: &Path, error: &std::io::Error) -> Self {
        RunStoreError::Io { path: path.display().to_string(), message: error.to_string() }
    }

    fn json(path: &Path, error: impl std::fmt::Display) -> Self {
        RunStoreError::Json { path: path.display().to_string(), message: error.to_string() }
    }
}

/// The project-side report: the validation result without the validation store's times, so only
/// `run.json` carries a time in the run folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectReport {
    pub schema_version: u32,
    pub run_id: String,
    pub project_id: String,
    pub candidate_digest: String,
    pub config_digest: String,
    pub model_version: String,
    pub profile_id: String,
    pub profile_version: String,
    pub profile_digest: String,
    /// `passed`, `quality_failed`, `infrastructure_error`, `cancelled` or `unknown`.
    pub state: String,
    pub checks: Vec<ProjectCheck>,
    pub error: Option<crate::validation::WireError>,
}

/// One check as the project report keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCheck {
    pub check_id: String,
    pub required: bool,
    pub state: String,
    pub expected: serde_json::Value,
    pub actual: serde_json::Value,
    pub evidence: Vec<crate::validation::ArtifactRef>,
}

impl ProjectReport {
    /// Project a verified validation report, keeping its checks and evidence references.
    pub fn new(meta: &RunMeta, profile: &CheckProfile, report: &ValidationReport) -> ProjectReport {
        ProjectReport {
            schema_version: RUN_SCHEMA_VERSION,
            run_id: meta.run_id.clone(),
            project_id: meta.project_id.clone(),
            candidate_digest: meta.candidate_digest.clone(),
            config_digest: meta.config_digest.clone(),
            model_version: meta.model_version.clone(),
            profile_id: profile.profile_id.clone(),
            profile_version: profile.version.clone(),
            profile_digest: report.profile.digest.clone(),
            state: report.state.as_str().to_string(),
            checks: report
                .checks
                .iter()
                .map(|check| ProjectCheck {
                    check_id: check.check_id.clone(),
                    required: check.required,
                    state: check.state.as_str().to_string(),
                    expected: check.expected.clone(),
                    actual: check.actual.clone(),
                    evidence: check.evidence.clone(),
                })
                .collect(),
            error: report.error.clone(),
        }
    }
}

/// `<project>/.intcad/checks/...`, written atomically.
#[derive(Clone, Debug)]
pub struct RunStore {
    root: PathBuf,
}

impl RunStore {
    /// A store for the project at `root` (the folder holding `.intcad/`).
    pub fn new(root: impl Into<PathBuf>) -> RunStore {
        RunStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn checks_dir(&self) -> PathBuf {
        self.root.join(PROJECT_FOLDER).join(CHECKS)
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.checks_dir().join(RUNS)
    }

    /// One run's folder. The id is checked, so a hostile id cannot escape the runs folder.
    pub fn run_dir(&self, run_id: &str) -> Result<PathBuf, RunStoreError> {
        if !valid_run_id(run_id) {
            return Err(RunStoreError::Invalid(format!("`{run_id}` is not a run identity this host writes")));
        }
        Ok(self.runs_dir().join(run_id))
    }

    /// Record this run as the newest submitted. Called once, before its work starts.
    pub fn mark_submitted(&self, meta: &RunMeta) -> Result<(), RunStoreError> {
        let submitted = SubmittedRun {
            schema_version: RUN_SCHEMA_VERSION,
            project_id: meta.project_id.clone(),
            run_id: meta.run_id.clone(),
            candidate_digest: meta.candidate_digest.clone(),
            config_digest: meta.config_digest.clone(),
            model_version: meta.model_version.clone(),
        };
        write_json(&self.checks_dir().join(SUBMITTED), &submitted)
    }

    /// The newest submitted run's identity, if a submission was recorded.
    pub fn submitted(&self) -> Result<Option<SubmittedIdentity>, RunStoreError> {
        read_optional_json(&self.checks_dir().join(SUBMITTED)).map(|record| {
            record.map(|submitted: SubmittedRun| SubmittedIdentity {
                project_id: submitted.project_id,
                run_id: submitted.run_id,
                candidate_digest: submitted.candidate_digest,
                config_digest: submitted.config_digest,
                model_version: submitted.model_version,
            })
        })
    }

    /// Write a complete run and publish it as `latest` only when it is still the newest
    /// submission.
    ///
    /// Every file is written atomically; `run.json` is last, so a folder without it is an
    /// incomplete write that never becomes the latest record. The submission is read again
    /// immediately before `run.json` and `latest.json` are written: a newer run submitted while
    /// this one was writing keeps `latest` and this run is recorded as `superseded`.
    pub fn write_complete(&self, meta: &RunMeta, files: &RunFiles) -> Result<RunOutcome, RunStoreError> {
        self.write_files(meta, files)?;
        if matches!(meta.outcome, RunOutcome::Cancelled | RunOutcome::Failed) {
            let finished = RunMeta { outcome: meta.outcome, ..meta.clone() };
            self.write_meta(&finished)?;
            return Ok(finished.outcome);
        }
        self.publish(meta, files)
    }

    /// Write the run's files (everything except `run.json` and `latest.json`).
    pub(crate) fn write_files(&self, meta: &RunMeta, files: &RunFiles) -> Result<(), RunStoreError> {
        let dir = self.run_dir(&meta.run_id)?;
        create_dir(&dir.join("evidence"))?;
        write_json(&dir.join("profile.json"), &files.profile)?;
        write_json(&dir.join("report.json"), &ProjectReport::new(meta, &files.profile, &files.report))?;
        for sheet in &files.sheets {
            write_json(&dir.join("evidence").join(format!("sheet-{}.facts.json", sheet.sheet)), &sheet.facts)?;
            write_json(&dir.join("evidence").join(format!("sheet-{}.rules.json", sheet.sheet)), &sheet.rules)?;
        }
        let bytes = findings::to_bytes(&files.findings)
            .map_err(|error| RunStoreError::Json { path: dir.join("findings.json").display().to_string(), message: error })?;
        write_bytes(&dir.join("findings.json"), &bytes)?;
        write_bytes(&dir.join("summary.md"), findings::summary(&files.findings).as_bytes())?;
        Ok(())
    }

    /// Publish files already written. The submission is read here, not earlier, and read once
    /// more before `latest.json`: a newer submission makes this run `superseded` and leaves
    /// `latest` alone.
    pub(crate) fn publish(&self, meta: &RunMeta, files: &RunFiles) -> Result<RunOutcome, RunStoreError> {
        let mut outcome = if self.is_newest(meta) { RunOutcome::Current } else { RunOutcome::Superseded };
        self.write_meta(&RunMeta { outcome, reason: superseded_reason(meta, outcome), ..meta.clone() })?;
        if outcome == RunOutcome::Current && !self.is_newest(meta) {
            // A run was submitted between the first check and this record: it wins.
            outcome = RunOutcome::Superseded;
            self.write_meta(&RunMeta { outcome, reason: superseded_reason(meta, outcome), ..meta.clone() })?;
        }
        if outcome == RunOutcome::Current {
            self.write_latest(meta, files)?;
        }
        Ok(outcome)
    }

    /// Write only a run record for a run that produced no complete result (cancelled before a
    /// report, or a rebuild failure). Never updates `latest.json`.
    pub fn write_stopped(&self, meta: &RunMeta) -> Result<RunOutcome, RunStoreError> {
        let outcome = match meta.outcome {
            RunOutcome::Cancelled | RunOutcome::Failed | RunOutcome::Superseded => meta.outcome,
            _ => RunOutcome::Failed,
        };
        let finished = RunMeta { outcome, ..meta.clone() };
        create_dir(&self.run_dir(&meta.run_id)?)?;
        self.write_meta(&finished)?;
        Ok(outcome)
    }

    fn is_newest(&self, meta: &RunMeta) -> bool {
        match self.submitted() {
            Ok(Some(submitted)) => {
                submitted.run_id == meta.run_id
                    && submitted.project_id == meta.project_id
                    && submitted.candidate_digest == meta.candidate_digest
                    && submitted.config_digest == meta.config_digest
            }
            _ => false,
        }
    }

    fn write_meta(&self, meta: &RunMeta) -> Result<(), RunStoreError> {
        write_json(&self.run_dir(&meta.run_id)?.join("run.json"), meta)
    }

    fn write_latest(&self, meta: &RunMeta, files: &RunFiles) -> Result<(), RunStoreError> {
        let latest = LatestRun {
            schema_version: RUN_SCHEMA_VERSION,
            project_id: meta.project_id.clone(),
            run_id: meta.run_id.clone(),
            candidate_digest: meta.candidate_digest.clone(),
            config_digest: meta.config_digest.clone(),
            model_version: meta.model_version.clone(),
            profile_digest: files.findings.profile_digest.clone(),
            source_kind: match files.findings.source {
                super::findings::SourceRef::Native => "native".into(),
                super::findings::SourceRef::Converted { .. } => "converted".into(),
            },
        };
        write_json(&self.checks_dir().join(LATEST), &latest)
    }

    /// The newest complete run, or `None` before the first one. A damaged pointer is an error,
    /// never a panic and never `Current`.
    pub fn latest(&self) -> Result<Option<LatestRun>, RunStoreError> {
        read_optional_json(&self.checks_dir().join(LATEST))
    }

    /// Read one run's record.
    pub fn load_meta(&self, run_id: &str) -> Result<RunMeta, RunStoreError> {
        read_json(&self.run_dir(run_id)?.join("run.json"))
    }

    /// Read one run's findings exactly as written.
    pub fn load_findings(&self, run_id: &str) -> Result<RunFindings, RunStoreError> {
        read_json(&self.run_dir(run_id)?.join("findings.json"))
    }

    /// Read one run's summary.
    pub fn load_summary(&self, run_id: &str) -> Result<String, RunStoreError> {
        let path = self.run_dir(run_id)?.join("summary.md");
        fs::read_to_string(&path).map_err(|error| RunStoreError::io(&path, &error))
    }

    /// A run folder is complete only when its `run.json` exists beside its findings.
    pub fn is_complete(&self, run_id: &str) -> bool {
        self.run_dir(run_id).is_ok_and(|dir| dir.join("run.json").is_file() && dir.join("findings.json").is_file())
    }
}

/// The identity `submitted.json` records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmittedIdentity {
    pub project_id: String,
    pub run_id: String,
    pub candidate_digest: String,
    pub config_digest: String,
    pub model_version: String,
}

/// Everything a worker needs to run one check. The host captures the candidate and freezes the
/// project before calling this, on the window thread; the worker never reads the live project.
#[derive(Clone, Debug)]
pub struct RunRequest {
    pub project_root: PathBuf,
    pub candidate: CandidateRef,
    pub input: RunInput,
    pub sheets: Vec<SheetRef>,
    /// RFC 3339 left in `run.json`; only that file carries times.
    pub started_at: String,
}

/// What a completed (or stopped) worker run reports back to the UI.
#[derive(Clone, Debug)]
pub struct RunResult {
    pub run_id: String,
    pub outcome: RunOutcome,
    pub findings: Option<RunFindings>,
    pub reason: Option<String>,
}

/// Run one check to a terminal record.
///
/// This is the background half of PIL-302: compile one `dfm.sheet` check per sheet, submit through
/// the existing [`ValidationPort`], write every complete file into the project, and only then let
/// `latest.json` move. A cancellation, a failed rebuild or a write failure leaves the previous
/// complete record and returns the reason. The caller runs this beside the window.
///
/// `on_submitted` receives the validation run id as soon as it exists, so the host can cancel it
/// from the window while the worker is still computing.
pub fn run_once(
    port: &Arc<ValidationPort>,
    request: &RunRequest,
    cancel: &AtomicBool,
    on_submitted: &dyn Fn(&str),
) -> Result<RunResult, RunStoreError> {
    let store = RunStore::new(&request.project_root);
    let compiled = compile::compile(&request.input, &request.sheets, &request.candidate.candidate_digest)
        .map_err(|error| RunStoreError::Validation(error.to_string()))?;
    let profile_ref = port.register_profile(compiled.profile.clone()).map_err(|error| RunStoreError::Validation(error.to_string()))?;
    if cancel.load(Ordering::Acquire) {
        return Ok(RunResult {
            run_id: String::new(),
            outcome: RunOutcome::Cancelled,
            findings: None,
            reason: Some("the check was cancelled before it was submitted".into()),
        });
    }
    let now = now_ms();
    let key = validation::generate_submission_key(now);
    let submitted = port.submit(&request.candidate, &profile_ref, &key).map_err(|error| RunStoreError::Validation(error.to_string()))?;
    let run_id = submitted.run_id;
    let mut meta = RunMeta::new(
        &run_id,
        &request.input.project_id,
        &request.candidate.candidate_digest,
        &request.input.config_digest,
        &request.candidate.candidate_digest,
        &request.started_at,
        &validation::timestamp(now),
        RunOutcome::Current,
        None,
    );
    meta.source_revision = request.candidate.source.revision.clone();
    store.mark_submitted(&meta)?;
    on_submitted(&run_id);
    if cancel.load(Ordering::Acquire) {
        let _ = port.cancel(&run_id);
    }
    port.pump().map_err(|error| RunStoreError::Validation(error.to_string()))?;
    let status = port.status(&run_id).map_err(|error| RunStoreError::Validation(error.to_string()))?;
    meta.ended_at = validation::timestamp(now_ms());
    match status.state {
        ValidationState::Cancelled => {
            meta.outcome = RunOutcome::Cancelled;
            meta.reason = Some("the check was cancelled".into());
            let outcome = store.write_stopped(&meta)?;
            return Ok(RunResult { run_id, outcome, findings: None, reason: meta.reason });
        }
        ValidationState::Passed | ValidationState::QualityFailed => {}
        _ => {
            let reason = match port.report(&run_id) {
                Ok(report) => report.report.error.map(|error| format!("{}: {}", error.code, error.message)),
                Err(_) => None,
            }
            .unwrap_or_else(|| format!("the check ended as {}", status.state.as_str()));
            meta.outcome = RunOutcome::Failed;
            meta.reason = Some(reason.clone());
            let outcome = store.write_stopped(&meta)?;
            return Ok(RunResult { run_id, outcome, findings: None, reason: Some(reason) });
        }
    }
    let completed = port.report(&run_id).map_err(|error| RunStoreError::Validation(error.to_string()))?;
    let report = completed.report;
    if let Some(reason) = required_check_failure(&report) {
        meta.outcome = RunOutcome::Failed;
        meta.reason = Some(reason.clone());
        let outcome = store.write_stopped(&meta)?;
        return Ok(RunResult { run_id, outcome, findings: None, reason: Some(reason) });
    }
    let mut sheet_checks = Vec::new();
    let mut evidence = Vec::new();
    let mut reasons = Vec::new();
    for requirement in compiled.profile.checks.iter().filter(|check| check.kind == "dfm.sheet") {
        let Some(result) = report.checks.iter().find(|result| result.check_id == requirement.check_id) else {
            reasons.push(format!("check {} did not run and has no result", requirement.check_id));
            meta.outcome = RunOutcome::Failed;
            meta.reason = Some(reasons.join("; "));
            let outcome = store.write_stopped(&meta)?;
            return Ok(RunResult { run_id, outcome, findings: None, reason: meta.reason });
        };
        let parameters = compiled.sheets.iter().find(|sheet| compile::check_id(sheet.sheet) == result.check_id);
        match collect_sheet(port, &run_id, result, parameters) {
            Ok(Some((check, files))) => {
                sheet_checks.push(check);
                evidence.push(files);
            }
            Ok(None) => reasons.push(sheet_reason(result, parameters)),
            Err(error) => return Err(error),
        }
    }
    let identity = FindingsIdentity {
        run_id: run_id.clone(),
        candidate_digest: request.candidate.candidate_digest.clone(),
        profile_digest: profile_ref.digest.clone(),
        config_digest: request.input.config_digest.clone(),
        model_version: request.candidate.candidate_digest.clone(),
        project_id: request.input.project_id.clone(),
        source: SourceRef::from_source(&request.input.source),
    };
    let mut findings = findings::build_with_reasons(&identity, &sheet_checks, &reasons);
    let (app, app_not_checked) = app_findings(report.checks.iter().find(|check| check.check_id == compile::FLAT_OVERLAP_CHECK));
    findings.extend_with_app(app, &app_not_checked);
    let files = RunFiles { profile: compiled.profile, report, findings: findings.clone(), sheets: evidence };
    let outcome = store.write_complete(&meta, &files)?;
    let reason = match outcome {
        RunOutcome::Superseded => Some("a newer check was submitted before this one finished".into()),
        _ => None,
    };
    Ok(RunResult { run_id, outcome, findings: Some(findings), reason })
}

/// The App's own check results (flat self-overlap) as findings, and what it could not check.
pub(super) fn app_findings(check: Option<&crate::validation::CheckResult>) -> (Vec<findings::Finding>, Vec<String>) {
    use findings::{FeatureRef, Finding, FindingFrom, FindingState, Measure, MeasureUnit, RequirementView};

    let Some(check) = check else {
        return (Vec::new(), Vec::new());
    };
    let tolerance = check.actual.get("toleranceMm2").and_then(serde_json::Value::as_f64);
    let mut out = Vec::new();
    let mut not_checked = Vec::new();
    let Some(sheets) = check.actual.get("sheets").and_then(serde_json::Value::as_array) else {
        return (out, not_checked);
    };
    for sheet in sheets {
        let index = sheet.get("sheet").and_then(serde_json::Value::as_u64).unwrap_or(0) as usize;
        let body = sheet.get("body").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
        let Some(observations) = sheet.get("observations").and_then(serde_json::Value::as_array) else { continue };
        for (position, observation) in observations.iter().enumerate() {
            let (state, status) = match observation.get("state").and_then(serde_json::Value::as_str) {
                Some("failed") => (FindingState::Failed, "violation"),
                Some("unsupported") => (FindingState::Unsupported, "not_verified"),
                _ => (FindingState::Passed, "pass"),
            };
            let measured = observation.get("measuredMm2").and_then(serde_json::Value::as_f64);
            let flat = observation
                .get("flat")
                .and_then(serde_json::Value::as_array)
                .filter(|points| points.len() == 2)
                .and_then(|points| Some([points[0].as_f64()?, points[1].as_f64()?]));
            let anchor = observation
                .get("anchor")
                .and_then(serde_json::Value::as_array)
                .filter(|points| points.len() == 3)
                .and_then(|points| Some([points[0].as_f64()?, points[1].as_f64()?, points[2].as_f64()?]));
            let outline = observation
                .get("outline")
                .and_then(serde_json::Value::as_array)
                .map(|points| {
                    points
                        .iter()
                        .filter_map(|point| {
                            let point = point.as_array()?;
                            Some([point.first()?.as_f64()?, point.get(1)?.as_f64()?, point.get(2)?.as_f64()?])
                        })
                        .collect()
                })
                .unwrap_or_default();
            if state == FindingState::Unsupported {
                not_checked.push(format!("Sheet {} (`{}`): the flat self-overlap check could not measure this body.", index + 1, body));
            }
            out.push(Finding {
                id: format!("sheet-{}/A{}", index + 1, position + 1),
                sheet: index,
                body: body.clone(),
                from: FindingFrom::App,
                rules: vec![crate::dfm::FLAT_NO_OVERLAP_RULE.to_string()],
                state,
                status: status.to_string(),
                code: observation.get("code").and_then(serde_json::Value::as_str).map(str::to_string),
                measured: measured.map(|value| Measure { value, unit: MeasureUnit::SquareMillimetre }),
                requirement: tolerance.map(|value| RequirementView {
                    value,
                    unit: MeasureUnit::SquareMillimetre,
                    layer: solidcraft_dfm_rules::Layer::Project,
                    confirmed: None,
                    source: crate::dfm::FLAT_NO_OVERLAP_RULE.to_string(),
                }),
                feature: Some(FeatureRef { kind: "region".into(), number: Some(index) }),
                anchor,
                flat,
                faces: Vec::new(),
                outline,
            });
        }
    }
    (out, not_checked)
}

/// The native checks protect the run: a candidate that does not rebuild cannot produce a report
/// that becomes `latest`.
fn required_check_failure(report: &ValidationReport) -> Option<String> {
    for id in ["input.integrity", "native.rebuild"] {
        let Some(check) = report.checks.iter().find(|check| check.check_id == id) else {
            return Some(format!("the check {id} did not run"));
        };
        if check.state != CheckState::Passed {
            return Some(format!("the check {id} ended as {}", check.state.as_str()));
        }
    }
    None
}

/// The measured facts and rule report keep their exact JSON bytes as the run's evidence.
fn collect_sheet(
    port: &Arc<ValidationPort>,
    run_id: &str,
    result: &validation::CheckResult,
    parameters: Option<&crate::dfm::SheetCheckParameters>,
) -> Result<Option<(SheetCheck, SheetEvidence)>, RunStoreError> {
    if result.state != CheckState::Passed && result.state != CheckState::Failed && result.state != CheckState::Unsupported {
        return Ok(None);
    }
    let Some(parameters) = parameters else { return Ok(None) };
    let mut facts = None;
    let mut rules = None;
    for artifact in &result.evidence {
        let bytes = port.evidence(run_id, &artifact.artifact_id).map_err(|error| RunStoreError::Validation(error.to_string()))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| RunStoreError::Json { path: format!("{run_id}/{}", artifact.artifact_id), message: error.to_string() })?;
        if value.get("thickness_mm").is_some() {
            facts = Some(value);
        } else if value.get("findings").is_some() {
            rules = Some(value);
        }
    }
    let (Some(facts), Some(rules)) = (facts, rules) else { return Ok(None) };
    let measured: solidcraft_doc::sheet_dfm::SheetFacts = serde_json::from_value(facts.clone())
        .map_err(|error| RunStoreError::Json { path: format!("{run_id}/{}", result.check_id), message: error.to_string() })?;
    let rule_report: solidcraft_dfm_rules::Report = serde_json::from_value(rules.clone())
        .map_err(|error| RunStoreError::Json { path: format!("{run_id}/{}", result.check_id), message: error.to_string() })?;
    let check = SheetCheck { sheet: parameters.sheet, body: parameters.body.clone(), facts: measured, report: rule_report };
    let files = SheetEvidence { sheet: parameters.sheet, body: parameters.body.clone(), facts, rules };
    Ok(Some((check, files)))
}

fn sheet_reason(result: &validation::CheckResult, parameters: Option<&crate::dfm::SheetCheckParameters>) -> String {
    let body = parameters.map_or_else(|| result.check_id.clone(), |parameters| format!("{} (`{}`)", parameters.sheet + 1, parameters.body));
    if let Some(reason) = result.actual.get("reason").and_then(serde_json::Value::as_str) {
        return format!("Sheet {body}: {reason}");
    }
    if let Some(error) = result.actual.get("error").and_then(serde_json::Value::as_str) {
        return format!("Sheet {body}: {error}");
    }
    format!("Sheet {body}: the check ended as {} with no rule report", result.state.as_str())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// `run-` followed by the contract's `Id` alphabet, so a run id is safe as one folder name.
fn valid_run_id(id: &str) -> bool {
    id.strip_prefix("run-").is_some_and(|rest| {
        !rest.is_empty()
            && id.len() <= 128
            && id.starts_with(|c: char| c.is_ascii_alphanumeric())
            && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-:".contains(&byte))
    })
}

fn create_dir(path: &Path) -> Result<(), RunStoreError> {
    fs::create_dir_all(path).map_err(|error| RunStoreError::io(path, &error))
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), RunStoreError> {
    if let Some(parent) = path.parent() {
        create_dir(parent)?;
    }
    solidcraft_io::safe_write::write_atomic(path, bytes, false).map_err(|error| RunStoreError::io(path, &error))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), RunStoreError> {
    let bytes = serde_json::to_vec(value).map_err(|error| RunStoreError::json(path, error))?;
    write_bytes(path, &bytes)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, RunStoreError> {
    let bytes = fs::read(path).map_err(|error| RunStoreError::io(path, &error))?;
    serde_json::from_slice(&bytes).map_err(|error| RunStoreError::json(path, error))
}

fn read_optional_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, RunStoreError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| RunStoreError::json(path, error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(RunStoreError::io(path, &error)),
    }
}
