//! Skill-facing validation over the existing [`ValidationPort`].
//!
//! The native Chat skill and a headless caller use this module. It does not add a second
//! validation path: it registers the host-owned Beta profile, submits the exact captured
//! snapshot, runs the existing local runner and returns a report reference that names the
//! snapshot, document revision, profile and required checks. A report for another snapshot or
//! for a document revision that has since moved is evidence, never approval.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::error::{ValidationError, WireError};
use super::profile::{CheckProfile, MeasurementRequirement, ParameterRequirement, ProfileRef, profile_ref, task_profile, validate_profile};
use super::report::{ArtifactRef, CheckState, RunnerIdentity, ValidationState};
use super::{ValidationPort, generate_submission_key};
use crate::schema::snapshot::CandidateRef;

/// The Beta Text-to-CAD profile identity.
pub const TEXT_TO_CAD_PROFILE_ID: &str = "text-to-cad";
/// The Beta Text-to-CAD profile version.
pub const TEXT_TO_CAD_PROFILE_VERSION: &str = "1.0.0";
/// The checks the Beta Text-to-CAD profile always requires, in profile order.
pub const TEXT_TO_CAD_REQUIRED_CHECKS: [&str; 4] = ["geometry.exact", "input.integrity", "native.rebuild", "parameters.required"];

/// The live document a caller wants the report compared with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveDocument {
    pub document_id: String,
    pub revision: u64,
}

impl LiveDocument {
    pub fn new(document_id: impl Into<String>, revision: u64) -> Self {
        Self { document_id: document_id.into(), revision }
    }
}

/// One check result as a skill or a headless caller reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillCheck {
    pub check_id: String,
    pub required: bool,
    pub state: String,
    pub passed: bool,
}

/// Why a skill-facing report is not approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillFailureKind {
    Timeout,
    InfrastructureError,
    QualityFailed,
    Cancelled,
    Unknown,
}

impl SkillFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillFailureKind::Timeout => "timeout",
            SkillFailureKind::InfrastructureError => "infrastructure_error",
            SkillFailureKind::QualityFailed => "quality_failed",
            SkillFailureKind::Cancelled => "cancelled",
            SkillFailureKind::Unknown => "unknown",
        }
    }
}

/// The structured failure carried by a skill report, independent of the prose `reason`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillFailure {
    pub kind: SkillFailureKind,
    /// The exact persisted contract error code, when the report carries one (`TIMEOUT`,
    /// `INTERNAL_ERROR`, `CANCELLED`, `OUTCOME_UNKNOWN`, ...). A quality failure has no error.
    pub code: Option<String>,
    pub message: String,
}

/// The structured report reference a skill or a headless caller consumes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillValidationReport {
    pub schema_version: String,
    pub run_id: String,
    /// The immutable candidate (snapshot) identity the report is bound to.
    pub snapshot_id: String,
    pub document_id: String,
    /// The captured revision, as the canonical decimal string in the snapshot.
    pub document_revision: String,
    pub candidate_digest: String,
    pub manifest_digest: String,
    pub profile: ProfileRef,
    pub runner: RunnerIdentity,
    pub required_checks: Vec<String>,
    pub checks: Vec<SkillCheck>,
    pub state: ValidationState,
    /// True when the exact submitted snapshot completed every required check under the frozen
    /// profile, before comparing the live document. Snapshot evidence is not approval on its own.
    pub snapshot_passed: bool,
    /// True only when `snapshot_passed` and the caller supplied the captured live revision. A
    /// missing or moved live document can never be approved.
    pub approved: bool,
    /// The structured terminal failure, or `None` when the captured snapshot passed.
    pub failure: Option<SkillFailure>,
    pub reason: String,
    /// The exact persisted report bytes' identity.
    pub report: ArtifactRef,
}

impl SkillValidationReport {
    /// The report approves this exact snapshot and this exact live revision. A missing live
    /// document, nil/empty check or stale revision cannot match, so a stale report never selects
    /// or approves another design.
    pub fn approves(&self, snapshot: &CandidateRef, live: Option<&LiveDocument>) -> bool {
        self.approved
            && self.snapshot_id == snapshot.candidate_id
            && self.candidate_digest == snapshot.candidate_digest
            && self.manifest_digest == snapshot.manifest_digest
            && self.document_id == snapshot.source.document_id
            && self.document_revision == snapshot.source.revision
            && live.is_some_and(|live| live.document_id == snapshot.source.document_id && live.revision.to_string() == snapshot.source.revision)
    }
}

/// Map the persisted terminal state/error into the skill-facing failure. `TIMEOUT` stays distinct
/// from `INTERNAL_ERROR` even though both are `infrastructure_error` at the port level.
fn skill_failure(state: ValidationState, error: Option<&WireError>) -> Option<SkillFailure> {
    let kind = match state {
        ValidationState::Passed => return None,
        ValidationState::QualityFailed => SkillFailureKind::QualityFailed,
        ValidationState::InfrastructureError => {
            if error.is_some_and(|error| error.code == "TIMEOUT") {
                SkillFailureKind::Timeout
            } else {
                SkillFailureKind::InfrastructureError
            }
        }
        ValidationState::Cancelled => SkillFailureKind::Cancelled,
        ValidationState::Unknown => SkillFailureKind::Unknown,
        ValidationState::Queued | ValidationState::Running => SkillFailureKind::Unknown,
    };
    Some(SkillFailure {
        kind,
        code: error.map(|error| error.code.clone()),
        message: error.map(|error| error.message.clone()).unwrap_or_else(|| format!("the validation report ended {}", state.as_str())),
    })
}

/// The complete Beta Text-to-CAD profile: the fixed native checks plus the task's required
/// parameters and exact measurements. The id/version are host-owned and the profile is
/// validated before it can be registered.
pub fn text_to_cad_profile(
    parameters: Vec<ParameterRequirement>,
    measurements: Vec<MeasurementRequirement>,
) -> Result<CheckProfile, ValidationError> {
    let profile = task_profile(TEXT_TO_CAD_PROFILE_ID, TEXT_TO_CAD_PROFILE_VERSION, Some(parameters), Some(measurements))?;
    validate_beta_profile(&profile)?;
    Ok(profile)
}

/// Keep the Beta profile identity and its required coverage fixed. A caller cannot rename the
/// profile, drop a required check or make it optional.
pub fn validate_beta_profile(profile: &CheckProfile) -> Result<(), ValidationError> {
    if profile.profile_id != TEXT_TO_CAD_PROFILE_ID || profile.version != TEXT_TO_CAD_PROFILE_VERSION {
        return Err(ValidationError::profile_unsupported(format!(
            "the Beta skill profile must be {TEXT_TO_CAD_PROFILE_ID} {TEXT_TO_CAD_PROFILE_VERSION}"
        )));
    }
    validate_profile(profile)?;
    for id in TEXT_TO_CAD_REQUIRED_CHECKS {
        match profile.checks.iter().find(|check| check.check_id == id) {
            Some(check) if check.required => {}
            _ => return Err(ValidationError::profile_unsupported(format!("the Beta skill profile requires the {id} check"))),
        }
    }
    Ok(())
}

/// A skill/headless validation request. `required_checks` is the caller's frozen list; it must
/// equal the profile's required check ids exactly, so a caller cannot silently narrow coverage.
#[derive(Clone, Debug)]
pub struct SkillValidationRequest<'a> {
    pub candidate: &'a CandidateRef,
    pub profile: CheckProfile,
    pub required_checks: Vec<String>,
    pub request_id: String,
    pub live: Option<LiveDocument>,
}

impl<'a> SkillValidationRequest<'a> {
    pub fn new(candidate: &'a CandidateRef, profile: CheckProfile, request_id: impl Into<String>) -> Self {
        Self {
            candidate,
            profile,
            required_checks: TEXT_TO_CAD_REQUIRED_CHECKS.iter().map(|id| (*id).to_string()).collect(),
            request_id: request_id.into(),
            live: None,
        }
    }

    pub fn with_live(mut self, live: LiveDocument) -> Self {
        self.live = Some(live);
        self
    }
}

/// Submit the exact snapshot, run the local worker and return the structured report reference.
/// A quality/infrastructure/cancelled/unknown terminal is returned as a report with
/// `approved: false`; only a verified passing report over the captured revision is approved.
pub fn validate_skill_snapshot(port: &ValidationPort, request: SkillValidationRequest<'_>) -> Result<SkillValidationReport, ValidationError> {
    validate_beta_profile(&request.profile)?;
    let profile_required: BTreeSet<&str> =
        request.profile.checks.iter().filter(|check| check.required).map(|check| check.check_id.as_str()).collect();
    let supplied_required: BTreeSet<&str> = request.required_checks.iter().map(String::as_str).collect();
    if supplied_required != profile_required {
        return Err(ValidationError::profile_unsupported(
            "the skill validation request must name the profile's required checks exactly; it cannot widen or narrow them",
        ));
    }
    let profile = port.register_profile(request.profile)?;
    let submitted = port.submit_for(request.candidate, &profile, &request.request_id, "skill")?;
    port.pump()?;
    let result = port.report(&submitted.run_id)?;
    let report = result.report;
    let checks: Vec<SkillCheck> = report
        .checks
        .iter()
        .map(|check| SkillCheck {
            check_id: check.check_id.clone(),
            required: check.required,
            state: check.state.as_str().into(),
            passed: check.state == CheckState::Passed,
        })
        .collect();
    let revision_matches = request.live.as_ref().is_some_and(|live| {
        live.document_id == request.candidate.source.document_id && live.revision.to_string() == request.candidate.source.revision
    });
    let required_pass = request
        .required_checks
        .iter()
        .all(|id| report.checks.iter().any(|check| check.check_id == *id && check.required && check.state == CheckState::Passed));
    let identity_matches = report.candidate_id == request.candidate.candidate_id
        && report.candidate_digest == request.candidate.candidate_digest
        && report.manifest_digest == request.candidate.manifest_digest
        && report.profile == profile
        && report.run_id == submitted.run_id;
    let snapshot_passed = identity_matches && report.state == ValidationState::Passed && report.error.is_none() && required_pass;
    let approved = snapshot_passed && revision_matches;
    let failure = skill_failure(report.state, report.error.as_ref());
    let reason = if approved {
        "the host validated the exact captured snapshot under the versioned Beta profile".to_string()
    } else if !identity_matches {
        "the validation report does not bind the submitted snapshot and profile".to_string()
    } else if let Some(failure) = &failure {
        format!("validation ended {} ({}); it is not success", report.state.as_str(), failure.code.as_deref().unwrap_or(failure.kind.as_str()))
    } else if !required_pass {
        "a required check did not pass".to_string()
    } else if request.live.is_none() {
        "the live document was not supplied; the report is snapshot evidence, not approval".to_string()
    } else {
        "the live document moved after the snapshot was captured; the report is historical evidence".to_string()
    };
    Ok(SkillValidationReport {
        schema_version: "1.0".into(),
        run_id: report.run_id,
        snapshot_id: report.candidate_id,
        document_id: request.candidate.source.document_id.clone(),
        document_revision: request.candidate.source.revision.clone(),
        candidate_digest: report.candidate_digest,
        manifest_digest: report.manifest_digest,
        profile: report.profile,
        runner: report.runner,
        required_checks: request.required_checks,
        checks,
        state: report.state,
        snapshot_passed,
        approved,
        failure,
        reason,
        report: result.reference,
    })
}

/// The profile digest a host records in a skill descriptor or a receipt.
pub fn text_to_cad_profile_ref(profile: &CheckProfile) -> Result<ProfileRef, ValidationError> {
    validate_beta_profile(profile)?;
    profile_ref(profile)
}

/// The structured field set a skill descriptor may show. Kept here so a UI does not invent a
/// different display of the same report.
pub fn summary_fields(report: &SkillValidationReport) -> Value {
    json!({
        "runId": report.run_id,
        "snapshotId": report.snapshot_id,
        "documentId": report.document_id,
        "documentRevision": report.document_revision,
        "profile": report.profile,
        "state": report.state,
        "snapshotPassed": report.snapshot_passed,
        "approved": report.approved,
        "failure": report.failure,
        "requiredChecks": report.required_checks,
        "report": report.report,
    })
}

/// A fresh request key for a new validation run. The host, not the model, supplies it.
pub fn request_key(now_ms: u64) -> String {
    generate_submission_key(now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_beta_profile_is_fixed_and_carries_the_required_checks() {
        let profile = text_to_cad_profile(
            vec![ParameterRequirement { name: "thickness".into(), value: 8.0, abs_tolerance: 1e-6 }],
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
        assert_eq!(profile.profile_id, TEXT_TO_CAD_PROFILE_ID);
        assert_eq!(profile.version, TEXT_TO_CAD_PROFILE_VERSION);
        assert_eq!(
            profile.checks.iter().map(|check| check.check_id.as_str()).collect::<Vec<_>>(),
            ["geometry.exact", "input.integrity", "native.rebuild", "parameters.required"]
        );
        let reference = text_to_cad_profile_ref(&profile).unwrap();
        assert_eq!(reference.digest, profile_ref(&profile).unwrap().digest);

        let mut weakened = profile.clone();
        weakened.checks.iter_mut().find(|check| check.check_id == "parameters.required").unwrap().required = false;
        assert_eq!(validate_beta_profile(&weakened).unwrap_err().code, super::super::ErrorCode::ProfileUnsupported);

        let mut renamed = profile;
        renamed.profile_id = "custom".into();
        assert_eq!(validate_beta_profile(&renamed).unwrap_err().code, super::super::ErrorCode::ProfileUnsupported);
    }
}
