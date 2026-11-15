//! Turn one frozen project configuration plus its sheet bodies into the validation profile a run
//! submits.
//!
//! One `dfm.sheet` check is compiled per sheet body, so a body that cannot be measured says so in
//! its own result and a thickness conflict cannot block another body. The built-in
//! `native-basic` checks stay in the profile: `input.integrity` and `native.rebuild` prove the
//! captured candidate is the model that was measured.

use serde::{Deserialize, Serialize};
use serde_json::json;

use solidcraft_dfm_rules::{Critical, Declared, Override, Point3, Profile};

use crate::dfm::{DfmError, SheetCheckParameters};
use crate::validation::{CheckProfile, CheckRequirement, native_basic_profile, validate_profile};

/// The profile identity every compiled DFM run uses.
pub const PROFILE_ID: &str = "dfm-project";
/// The compiled profile's version.
pub const PROFILE_VERSION: &str = "1.0.0";
/// The App-wide overlap check B registers. Compiled only when B's adapter is present.
pub const FLAT_OVERLAP_CHECK: &str = "dfm.sheet.flat_overlap";

/// The frozen, project-level inputs one run uses. A builds this from `FrozenProject` (CONTRACT
/// §1); C never parses TOML and never re-reads the project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunInput {
    pub project_id: String,
    pub project_name: String,
    /// SHA-256 over the frozen configuration; staleness compares it.
    pub config_digest: String,
    /// `project.toml`'s declared process, material and thickness as written. `None` means the
    /// project declares no thickness; it does not mean the data rows do not apply. Each native
    /// sheet then falls back to its own model thickness in [`compile`].
    pub declared: Declared,
    /// The generic rows exactly as the rule layer parsed them.
    pub generic: Profile,
    /// The selected factory or standard rows, when the rows were parsed. A copy that is
    /// unavailable only because a declaration is missing still passes its rows; an unreadable,
    /// digest-mismatched or unparsable copy is the caller's `None`.
    #[serde(default)]
    pub shop: Option<Profile>,
    #[serde(default)]
    pub overrides: Vec<Override>,
    /// The project's critical hole-to-hole dimensions, frozen with the rest of the run.
    #[serde(default)]
    pub critical: Vec<crate::project::CriticalDimension>,
    /// The App's model source: `native` or `converted <sha256>`.
    pub source: String,
}

impl RunInput {
    /// Copy the frozen configuration one run uses. `source` is the model source for the report:
    /// `native` or `converted <sha256>`.
    pub fn from_frozen(frozen: &crate::project::FrozenProject, source: &str) -> RunInput {
        RunInput {
            project_id: frozen.project_id.clone(),
            project_name: frozen.project_name.clone(),
            config_digest: frozen.config_digest.clone(),
            declared: frozen.declared.clone(),
            generic: frozen.generic.clone(),
            shop: frozen.shop.clone(),
            overrides: frozen.overrides.clone(),
            critical: frozen.critical.clone(),
            source: source.to_string(),
        }
    }
}

/// One sheet body a run checks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SheetRef {
    /// The body's index in the evaluated model; a rename cannot select another body.
    pub sheet: usize,
    pub body: String,
    /// The body's own model thickness. When the project declares no thickness, a native body's
    /// declared value falls back to this and records `thicknessFrom = "model"`; the rule layer
    /// then selects rows for that thickness. `None` means the caller could not measure it.
    #[serde(default)]
    pub thickness_mm: Option<f64>,
    /// A converted or imported body. It never gets the native thickness fallback: the rules run
    /// on the converted model, and its thickness must come from the project.
    #[serde(default)]
    pub converted: bool,
}

/// The checks and parameters one run submits.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledProfile {
    pub profile: CheckProfile,
    /// One entry per sheet, in the profile's check order.
    pub sheets: Vec<SheetCheckParameters>,
}

/// Compile the project's checks for the given sheet bodies.
///
/// `model_version` is the captured candidate digest: the exact model the facts are measured on.
/// An empty sheet list is a valid run: the profile carries only the native checks and the report
/// says no DFM rule had an applicable object.
pub fn compile(input: &RunInput, sheets: &[SheetRef], model_version: &str) -> Result<CompiledProfile, DfmError> {
    if model_version.trim().is_empty() {
        return Err(DfmError::Parameters("the model version must be recorded".into()));
    }
    if input.project_id.trim().is_empty() {
        return Err(DfmError::Parameters("the project identity must be recorded".into()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for sheet in sheets {
        if sheet.body.trim().is_empty() {
            return Err(DfmError::Parameters(format!("sheet {} has no body name", sheet.sheet)));
        }
        if !seen.insert(sheet.sheet) {
            return Err(DfmError::Parameters(format!("sheet {} is listed twice", sheet.sheet)));
        }
    }
    let mut profile = native_basic_profile();
    profile.profile_id = PROFILE_ID.into();
    // The sheet parameters carry the captured model, so the same profile id/version cannot cover
    // two models. A stable suffix keeps a check idempotent for one candidate+config while a new
    // model gets its own immutable profile identity.
    let mut seed = Vec::new();
    seed.extend_from_slice(model_version.as_bytes());
    seed.push(0);
    seed.extend_from_slice(input.project_id.as_bytes());
    seed.push(0);
    seed.extend_from_slice(input.config_digest.as_bytes());
    for sheet in sheets {
        seed.push(0);
        seed.extend_from_slice(&sheet.sheet.to_le_bytes());
        seed.extend_from_slice(sheet.body.as_bytes());
    }
    let suffix: String = solidcraft_io::candidate::digest_hex(&seed).chars().take(16).collect();
    profile.version = format!("{PROFILE_VERSION}+{suffix}");
    let mut parameters = Vec::with_capacity(sheets.len());
    for sheet in sheets {
        // A project without a declared thickness still checks a native body: the body's own model
        // thickness is the declared value and the report says where it came from. Converted
        // bodies get no fallback. An explicit project thickness is used as written, even when it
        // conflicts with one body; that body's own gate then reports the conflict.
        let mut declared = input.declared.clone();
        if declared.thickness_mm.is_none() && !sheet.converted {
            declared.thickness_mm = sheet.thickness_mm;
            if declared.thickness_mm.is_some() {
                declared.thickness_from = Some("model".into());
            }
        }
        let check = SheetCheckParameters {
            sheet: sheet.sheet,
            body: sheet.body.clone(),
            declared,
            generic: input.generic.clone(),
            shop: input.shop.clone(),
            overrides: input.overrides.clone(),
            critical: input
                .critical
                .iter()
                .map(|dimension| Critical {
                    name: dimension.name.clone(),
                    from_mm: Point3 { x: dimension.from_mm[0], y: dimension.from_mm[1], z: dimension.from_mm[2] },
                    to_mm: Point3 { x: dimension.to_mm[0], y: dimension.to_mm[1], z: dimension.to_mm[2] },
                    tolerance_mm: dimension.tolerance_mm,
                })
                .collect(),
            model: solidcraft_dfm_rules::ModelRef {
                model_version: model_version.to_string(),
                body: sheet.body.clone(),
                source: input.source.clone(),
            },
            source: input.source.clone(),
        };
        // Refuse a bad configuration before a run starts; a bad profile never reaches a report.
        let value = serde_json::to_value(&check).map_err(|error| DfmError::Parameters(error.to_string()))?;
        crate::dfm::validate_parameters(&value)?;
        profile.checks.push(CheckRequirement {
            // Zero-padded so the profile's required sort matches sheet order past sheet 9.
            check_id: format!("dfm.sheet.{:03}", sheet.sheet),
            kind: "dfm.sheet".into(),
            // A body that cannot be checked is "not covered", not an infrastructure failure of
            // the whole run; the findings and the summary carry that state.
            required: false,
            parameters: value,
        });
        parameters.push(check);
    }
    if !sheets.is_empty() {
        let parameters = flat_overlap_parameters(model_version, sheets);
        crate::dfm::validate_flat_parameters(&parameters)?;
        profile.checks.push(CheckRequirement { check_id: FLAT_OVERLAP_CHECK.into(), kind: FLAT_OVERLAP_CHECK.into(), required: false, parameters });
    }
    profile.checks.sort_by(|a, b| a.check_id.cmp(&b.check_id));
    validate_profile(&profile).map_err(|error| DfmError::Parameters(error.to_string()))?;
    Ok(CompiledProfile { profile, sheets: parameters })
}

/// The check id one sheet's `dfm.sheet` requirement uses.
pub fn check_id(sheet: usize) -> String {
    format!("dfm.sheet.{sheet:03}")
}

/// The parameter object of the App-wide overlap check, once B's adapter registers it. Kept here so
/// the profile shape, the runner branch and the finding identity stay in one place.
pub fn flat_overlap_parameters(model_version: &str, sheets: &[SheetRef]) -> serde_json::Value {
    json!({
        "modelVersion": model_version,
        "sheets": sheets.iter().map(|sheet| json!({"sheet": sheet.sheet, "body": sheet.body})).collect::<Vec<_>>(),
        "toleranceMm2": 0.01,
    })
}
