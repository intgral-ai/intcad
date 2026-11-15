//! Validation states, reports and their wire shapes (contract §9.2–§9.3).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::profile::{CheckProfile, ProfileRef};
use crate::schema::snapshot::CandidateRef;

/// One validation run's state. The first four are live; the rest are terminal and immutable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationState {
    Queued,
    Running,
    Passed,
    QualityFailed,
    InfrastructureError,
    Cancelled,
    Unknown,
}

impl ValidationState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, ValidationState::Queued | ValidationState::Running)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ValidationState::Queued => "queued",
            ValidationState::Running => "running",
            ValidationState::Passed => "passed",
            ValidationState::QualityFailed => "quality_failed",
            ValidationState::InfrastructureError => "infrastructure_error",
            ValidationState::Cancelled => "cancelled",
            ValidationState::Unknown => "unknown",
        }
    }
}

/// A check result state (contract §9.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Passed,
    Failed,
    Error,
    Unsupported,
    NotRun,
}

impl CheckState {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckState::Passed => "passed",
            CheckState::Failed => "failed",
            CheckState::Error => "error",
            CheckState::Unsupported => "unsupported",
            CheckState::NotRun => "not_run",
        }
    }
}

/// The runner identity a report binds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerIdentity {
    pub runner_id: String,
    pub version: String,
    pub engine_version: String,
}

/// A host-owned immutable evidence or report artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub digest: String,
    pub size_bytes: u64,
    pub media_type: String,
}

/// One profile check's result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckResult {
    pub check_id: String,
    pub required: bool,
    pub state: CheckState,
    pub expected: Value,
    pub actual: Value,
    pub evidence: Vec<ArtifactRef>,
}

/// A complete validation report (contract §9.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationReport {
    pub schema_version: String,
    pub run_id: String,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub manifest_digest: String,
    pub profile: ProfileRef,
    pub runner: RunnerIdentity,
    pub state: ValidationState,
    pub started_at: Option<String>,
    pub finished_at: String,
    pub checks: Vec<CheckResult>,
    pub error: Option<super::error::WireError>,
}

impl ValidationReport {
    /// Build an immutable report record. `checks` must already cover the profile exactly.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run_id: &str,
        candidate: &CandidateRef,
        profile: &ProfileRef,
        runner: &RunnerIdentity,
        state: ValidationState,
        started_at: Option<String>,
        finished_at: String,
        checks: Vec<CheckResult>,
        error: Option<super::error::WireError>,
    ) -> Self {
        ValidationReport {
            schema_version: "1.0".into(),
            run_id: run_id.into(),
            candidate_id: candidate.candidate_id.clone(),
            candidate_digest: candidate.candidate_digest.clone(),
            manifest_digest: candidate.manifest_digest.clone(),
            profile: profile.clone(),
            runner: runner.clone(),
            state,
            started_at,
            finished_at,
            checks,
            error,
        }
    }

    /// Every profile check's result, with `not_run` for anything the runner did not report.
    pub fn cover_profile(&mut self, profile: &CheckProfile) {
        let mut covered = Vec::with_capacity(profile.checks.len());
        for requirement in &profile.checks {
            match self.checks.iter().find(|result| result.check_id == requirement.check_id) {
                Some(result) => covered.push(result.clone()),
                None => covered.push(CheckResult {
                    check_id: requirement.check_id.clone(),
                    required: requirement.required,
                    state: CheckState::NotRun,
                    expected: requirement.parameters.clone(),
                    actual: json!({}),
                    evidence: Vec::new(),
                }),
            }
        }
        self.checks = covered;
    }
}

/// `status(runId)`'s successful result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunStatus {
    pub run_id: String,
    pub state: ValidationState,
    pub candidate: CandidateRef,
    pub profile: ProfileRef,
    pub updated_at: String,
    pub report_available: bool,
}

/// `submit`'s successful result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitResult {
    pub run_id: String,
    pub state: ValidationState,
    pub candidate: CandidateRef,
    pub profile: ProfileRef,
}

/// `report(runId)`'s successful result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportResult {
    pub report: ValidationReport,
    pub reference: ArtifactRef,
}

/// `cancel(runId)`'s successful result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelResult {
    pub run_id: String,
    pub state: ValidationState,
}
