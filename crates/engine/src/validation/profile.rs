//! Versioned check profiles (contract §9.1).
//!
//! The host resolves a profile from its registry, verifies the digest and freezes it before a
//! worker starts. A runner cannot relax a requirement: the built-in `native-basic` checks are
//! always present and required, and task checks are additions.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ValidationError;
use super::ids::{digest, valid_digest, valid_id};
use super::jcs;

/// The built-in profile identity.
pub const NATIVE_BASIC_ID: &str = "native-basic";
/// The built-in profile version.
pub const NATIVE_BASIC_VERSION: &str = "1.0.0";
/// The contract's check-count ceiling.
pub const MAX_CHECKS: usize = 128;
/// The contract's entries-per-requirement-array ceiling.
pub const MAX_REQUIREMENTS: usize = 64;

/// A resolved profile identity: what `submit` accepts and a report binds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileRef {
    pub profile_id: String,
    pub version: String,
    pub digest: String,
}

/// One check requirement in a profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckRequirement {
    pub check_id: String,
    pub kind: String,
    pub required: bool,
    pub parameters: Value,
}

/// A complete, versioned check profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckProfile {
    pub schema_version: String,
    pub profile_id: String,
    pub version: String,
    pub checks: Vec<CheckRequirement>,
}

/// One declared parameter requirement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParameterRequirement {
    pub name: String,
    pub value: f64,
    pub abs_tolerance: f64,
}

/// One exact-geometry measurement requirement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasurementRequirement {
    pub command: String,
    pub params: Value,
    pub result_pointer: String,
    pub expected: f64,
    pub abs_tolerance: f64,
    pub rel_tolerance: f64,
}

/// The check ids the built-in minimum profile always carries.
pub const NATIVE_CHECKS: [&str; 2] = ["input.integrity", "native.rebuild"];

fn invalid(message: impl std::fmt::Display) -> ValidationError {
    ValidationError::invalid_request(message.to_string())
}

fn unsupported(message: impl std::fmt::Display) -> ValidationError {
    ValidationError::profile_unsupported(message.to_string())
}

fn valid_version(version: &str) -> bool {
    !version.is_empty() && version.len() <= 64 && version.chars().all(|c| c.is_ascii_graphic())
}

/// The `native-basic` 1.0.0 profile: input integrity and a clean native rebuild.
pub fn native_basic_profile() -> CheckProfile {
    CheckProfile {
        schema_version: "1.0".into(),
        profile_id: NATIVE_BASIC_ID.into(),
        version: NATIVE_BASIC_VERSION.into(),
        checks: vec![
            CheckRequirement { check_id: "input.integrity".into(), kind: "input.integrity".into(), required: true, parameters: json!({}) },
            CheckRequirement { check_id: "native.rebuild".into(), kind: "native.rebuild".into(), required: true, parameters: json!({}) },
        ],
    }
}

/// A task profile: the built-in checks plus the task's parameter and geometry requirements.
/// Entries are sorted by `checkId` and the result is validated.
pub fn task_profile(
    profile_id: &str,
    version: &str,
    parameters: Option<Vec<ParameterRequirement>>,
    measurements: Option<Vec<MeasurementRequirement>>,
) -> Result<CheckProfile, ValidationError> {
    let mut checks = native_basic_profile().checks;
    if let Some(requirements) = parameters {
        checks.push(CheckRequirement {
            check_id: "parameters.required".into(),
            kind: "parameters.required".into(),
            required: true,
            parameters: serde_json::to_value(json!({"requirements": requirements})).map_err(|e| invalid(format!("profile: {e}")))?,
        });
    }
    if let Some(measurements) = measurements {
        checks.push(CheckRequirement {
            check_id: "geometry.exact".into(),
            kind: "geometry.exact".into(),
            required: true,
            parameters: serde_json::to_value(json!({"measurements": measurements})).map_err(|e| invalid(format!("profile: {e}")))?,
        });
    }
    checks.sort_by(|a, b| a.check_id.cmp(&b.check_id));
    let profile = CheckProfile { schema_version: "1.0".into(), profile_id: profile_id.into(), version: version.into(), checks };
    validate_profile(&profile)?;
    Ok(profile)
}

/// Check a complete profile's shape, ordering, bounds and mandatory requirements.
pub fn validate_profile(profile: &CheckProfile) -> Result<(), ValidationError> {
    if profile.schema_version != "1.0" {
        return Err(invalid("profile schemaVersion must be \"1.0\""));
    }
    if !valid_id(&profile.profile_id) {
        return Err(invalid("profileId must be a valid Id"));
    }
    if !valid_version(&profile.version) {
        return Err(invalid("profile version must be non-empty printable text"));
    }
    if profile.checks.is_empty() || profile.checks.len() > MAX_CHECKS {
        return Err(invalid(format!("a profile has 1..={MAX_CHECKS} checks")));
    }
    let mut seen = BTreeSet::new();
    let mut previous: Option<&str> = None;
    for check in &profile.checks {
        if !valid_id(&check.check_id) {
            return Err(invalid(format!("checkId {:?} is not a valid Id", check.check_id)));
        }
        if !seen.insert(check.check_id.as_str()) {
            return Err(invalid(format!("duplicate check {}", check.check_id)));
        }
        if previous.is_some_and(|previous| previous >= check.check_id.as_str()) {
            return Err(invalid("profile checks must be ordered by checkId"));
        }
        previous = Some(&check.check_id);
        if check.kind.is_empty() || check.kind.len() > 240 {
            return Err(invalid(format!("check {} has an invalid kind", check.check_id)));
        }
        if !check.parameters.is_object() {
            return Err(invalid(format!("check {} parameters must be an object", check.check_id)));
        }
        validate_check_parameters(check)?;
    }
    // The minimum profile cannot be weakened, omitted or re-typed.
    for id in NATIVE_CHECKS {
        let Some(check) = profile.checks.iter().find(|check| check.check_id == id) else {
            return Err(unsupported(format!("profile {} is missing the required {id} check", profile.profile_id)));
        };
        if check.kind != id || !check.required || check.parameters != json!({}) {
            return Err(unsupported(format!("the {id} check is fixed and required; a profile cannot relax it")));
        }
    }
    Ok(())
}

fn validate_check_parameters(check: &CheckRequirement) -> Result<(), ValidationError> {
    match check.kind.as_str() {
        "input.integrity" | "native.rebuild" => {
            if check.check_id != check.kind {
                return Err(invalid(format!("check {} must use kind {}", check.check_id, check.check_id)));
            }
            Ok(())
        }
        "parameters.required" => {
            let requirements: Parameters =
                serde_json::from_value(check.parameters.clone()).map_err(|e| invalid(format!("parameters.required: {e}")))?;
            if requirements.requirements.is_empty() || requirements.requirements.len() > MAX_REQUIREMENTS {
                return Err(invalid(format!("parameters.required has 1..={MAX_REQUIREMENTS} entries")));
            }
            for requirement in &requirements.requirements {
                if requirement.name.is_empty() || requirement.name.len() > 240 {
                    return Err(invalid("a parameter requirement needs a name"));
                }
                if !requirement.value.is_finite() || !requirement.abs_tolerance.is_finite() || requirement.abs_tolerance < 0.0 {
                    return Err(invalid(format!("parameter {} has non-finite values or a negative tolerance", requirement.name)));
                }
            }
            Ok(())
        }
        "dfm.sheet" => {
            crate::dfm::validate_parameters(&check.parameters).map_err(|error| invalid(format!("dfm.sheet: {error}")))?;
            Ok(())
        }
        "geometry.exact" => {
            let measurements: Measurements = serde_json::from_value(check.parameters.clone()).map_err(|e| invalid(format!("geometry.exact: {e}")))?;
            if measurements.measurements.is_empty() || measurements.measurements.len() > MAX_REQUIREMENTS {
                return Err(invalid(format!("geometry.exact has 1..={MAX_REQUIREMENTS} entries")));
            }
            for measurement in &measurements.measurements {
                if !valid_id(&measurement.command) {
                    return Err(invalid("a measurement command must be a valid command id"));
                }
                if !measurement.params.is_object() {
                    return Err(invalid(format!("measurement {} params must be an object", measurement.command)));
                }
                if !measurement.result_pointer.starts_with('/') {
                    return Err(invalid("a measurement resultPointer must be a JSON pointer"));
                }
                if !measurement.expected.is_finite()
                    || !measurement.abs_tolerance.is_finite()
                    || measurement.abs_tolerance < 0.0
                    || !measurement.rel_tolerance.is_finite()
                    || measurement.rel_tolerance < 0.0
                {
                    return Err(invalid("measurement values must be finite and tolerances nonnegative"));
                }
            }
            Ok(())
        }
        // An unrecognized kind is accepted by the profile and the runner marks it unsupported.
        _ => Ok(()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Parameters {
    requirements: Vec<ParameterRequirement>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Measurements {
    measurements: Vec<MeasurementRequirement>,
}

/// Parse the requirement arrays a runner needs.
pub fn parameter_requirements(check: &CheckRequirement) -> Result<Vec<ParameterRequirement>, ValidationError> {
    let value: Parameters = serde_json::from_value(check.parameters.clone()).map_err(|e| invalid(format!("parameters.required: {e}")))?;
    Ok(value.requirements)
}

pub fn measurement_requirements(check: &CheckRequirement) -> Result<Vec<MeasurementRequirement>, ValidationError> {
    let value: Measurements = serde_json::from_value(check.parameters.clone()).map_err(|e| invalid(format!("geometry.exact: {e}")))?;
    Ok(value.measurements)
}

/// SHA-256 of the JCS encoding of the complete profile.
pub fn profile_digest(profile: &CheckProfile) -> Result<String, ValidationError> {
    let value = serde_json::to_value(profile).map_err(|e| invalid(format!("profile: {e}")))?;
    Ok(digest(&jcs::canonical_json(&value)?))
}

/// The `ProfileRef` a profile is submitted and reported under.
pub fn profile_ref(profile: &CheckProfile) -> Result<ProfileRef, ValidationError> {
    Ok(ProfileRef { profile_id: profile.profile_id.clone(), version: profile.version.clone(), digest: profile_digest(profile)? })
}

/// Check a `ProfileRef` names exactly `profile`.
pub fn verify_profile_ref(profile: &CheckProfile, reference: &ProfileRef) -> Result<(), ValidationError> {
    if !valid_digest(&reference.digest) {
        return Err(unsupported("profile digest must be sha256:<64 lowercase hex>"));
    }
    if profile.profile_id != reference.profile_id || profile.version != reference.version {
        return Err(unsupported("profile identity does not match the resolved profile"));
    }
    if profile_digest(profile)? != reference.digest {
        return Err(unsupported("profile digest does not match the resolved profile"));
    }
    Ok(())
}
