//! The pure DFM rule layer: facts and declared data in, one deterministic report out.
//!
//! This is the Rust side of the INTdfm handover. It does not load OCP, read a file, call a
//! model, or look at a clock. The App measures facts; this crate chooses the number that
//! applies, runs the gates and the first hole rules, and maps the rule result to the state the
//! App shows. A mesh is never a model decision here.
//!
//! The rule ids, measuring lines and reason codes follow INTdfm. `confirmation` is the proposed
//! FD-7 addition: a number only decides when a factory confirmed it or a real part verified it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

pub mod bend_rules;
pub mod intdfm_report;
pub mod second_gate;

/// The rules-crate version. It is part of every report.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The report schema version.
pub const SCHEMA_VERSION: u32 = 1;
/// The thickness gate's placeholder match, from INTdfm.
pub const THICKNESS_TOLERANCE_MM: f64 = 0.02;
/// Two hole depths are the same sheet within this distance, from INTdfm.
pub const SAME_SHEET_MM: f64 = 0.1;

pub const THICKNESS_GATE: &str = "sm.gate_sheet_thickness";
pub const HOLE_MIN_DIAMETER: &str = "sm.hole_min_diameter";
pub const HOLE_TO_EDGE: &str = "sm.hole_to_edge";
pub const HOLE_TO_BEND: &str = "sm.hole_to_bend";

/// The rule catalog in report order. Gates sort before rules, as in INTdfm.
pub const RULE_ORDER: &[&str] = &[
    "sm.gate_input_solid",
    THICKNESS_GATE,
    "sm.gate_bends_found",
    "sm.gate_profile_coverage",
    "sm.inside_radius",
    "sm.min_flange",
    HOLE_MIN_DIAMETER,
    "sm.max_bend_length",
    HOLE_TO_BEND,
    "sm.bend_relief_present",
    HOLE_TO_EDGE,
    "sm.min_web_parallel_bends",
    "sm.u_channel_ratio",
    "sm.tolerance_across_bends",
    "sm.countersink_depth",
    "sm.countersink_to_edge",
    "sm.countersink_to_bend",
    "sm.hole_spacing",
    "sm.bend_radius_consistent",
];

const FIRST_HOLE_RULES: &[&str] = &[HOLE_MIN_DIAMETER, HOLE_TO_EDGE, HOLE_TO_BEND];
/// The fixed line a report and its summary card carry.
pub const NOT_CHECKED: &str =
    "Tolerances other than declared hole-to-hole dimensions, bend sequence, collisions, grain and cut-outs that reach the outline were not checked.";

/// A bad input is refused; it never becomes a finding.
#[derive(Debug, thiserror::Error)]
pub enum DfmError {
    #[error("invalid DFM input: {0}")]
    Invalid(String),
}

fn invalid(message: impl Into<String>) -> DfmError {
    DfmError::Invalid(message.into())
}

// ---------------------------------------------------------------------------
// Facts: what the App measured on the sheet body.

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Facts {
    #[serde(default)]
    pub volume_mm3: Option<f64>,
    #[serde(default)]
    pub size_mm: Option<Vec<f64>>,
    #[serde(default)]
    pub area_mm2: Option<f64>,
    #[serde(default)]
    pub paired_mm: Option<f64>,
    #[serde(default)]
    pub holes: Option<Vec<HoleFact>>,
    #[serde(default)]
    pub other_openings: usize,
    #[serde(default)]
    pub unknown_faces: usize,
    #[serde(default)]
    pub recognition: Option<Recognition>,
    #[serde(default)]
    pub plate_gap_mm: Option<f64>,
    #[serde(default)]
    pub paired_rays: usize,
    #[serde(default)]
    pub rays_failed: bool,
    #[serde(default)]
    pub shells_closed: bool,
    #[serde(default)]
    pub analyzer_valid: bool,
    #[serde(default)]
    pub bop_valid: bool,
    #[serde(default)]
    pub faces: usize,
    #[serde(default)]
    pub triangle_faces: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HoleFact {
    pub diameter_mm: f64,
    pub x_mm: f64,
    pub y_mm: f64,
    pub z_mm: f64,
    pub depth_mm: f64,
    #[serde(default)]
    pub square: bool,
    #[serde(default)]
    pub edge_mm: Option<f64>,
    #[serde(default)]
    pub edge_at_mm: Option<Vec<f64>>,
    #[serde(default)]
    pub edge_kind: Option<EdgeKind>,
    #[serde(default)]
    pub edge_hole: Option<usize>,
    #[serde(default)]
    pub bend_gaps: Vec<BendGap>,
    #[serde(default)]
    pub flat: Option<usize>,
    #[serde(default)]
    pub countersink: Option<CountersinkFact>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CountersinkFact {
    #[serde(default)]
    pub major_diameter_mm: f64,
    #[serde(default)]
    pub depth_mm: f64,
    #[serde(default)]
    pub angle_deg: f64,
    #[serde(default)]
    pub land_mm: f64,
    #[serde(default)]
    pub x_mm: f64,
    #[serde(default)]
    pub y_mm: f64,
    #[serde(default)]
    pub z_mm: f64,
    #[serde(default)]
    pub edge_mm: Option<f64>,
    #[serde(default)]
    pub edge_at_mm: Option<Vec<f64>>,
    #[serde(default)]
    pub edge_kind: Option<EdgeKind>,
    #[serde(default)]
    pub edge_hole: Option<usize>,
    #[serde(default)]
    pub edge_rim: bool,
    #[serde(default)]
    pub bend_gaps: Vec<BendGap>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Outline,
    Opening,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BendGap {
    pub bend: usize,
    pub gap_mm: f64,
    pub inside_radius_mm: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Recognition {
    #[serde(default)]
    pub bends: Vec<BendFact>,
    #[serde(default)]
    pub unknown_faces: usize,
    #[serde(default)]
    pub cone_faces: usize,
    #[serde(default)]
    pub form_faces: usize,
    #[serde(default)]
    pub webs: Vec<WebFact>,
    #[serde(default)]
    pub hems: usize,
    #[serde(default)]
    pub cones: usize,
    #[serde(default)]
    pub countersinks: usize,
    #[serde(default)]
    pub forms: usize,
    #[serde(default)]
    pub sharp_joins: usize,
    #[serde(default)]
    pub joined: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WebFact {
    #[serde(default)]
    pub bends: Vec<usize>,
    #[serde(default)]
    pub web_mm: f64,
    #[serde(default)]
    pub same_side: bool,
    #[serde(default)]
    pub flanges_mm: Vec<f64>,
    #[serde(default)]
    pub x_mm: f64,
    #[serde(default)]
    pub y_mm: f64,
    #[serde(default)]
    pub z_mm: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BendFact {
    pub inside_radius_mm: f64,
    pub angle_deg: f64,
    pub length_mm: f64,
    pub x_mm: f64,
    pub y_mm: f64,
    pub z_mm: f64,
    #[serde(default)]
    pub flats: Vec<Option<usize>>,
    #[serde(default)]
    pub runs_mm: Vec<Vec<f64>>,
    #[serde(default)]
    pub longest_mm: Vec<f64>,
    #[serde(default)]
    pub unrelieved: Vec<Vec<f64>>,
    #[serde(default)]
    pub no_margin: Vec<Vec<f64>>,
    #[serde(default)]
    pub crossed: bool,
}

// ---------------------------------------------------------------------------
// Data: the generic layer, a factory/standard, and project overrides.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Generic,
    Factory,
    Project,
    Development,
}

/// Who confirmed a number (the FD-7 proposal). An old file has none, which never decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confirmation {
    TeamSelfReported,
    FactoryConfirmed,
    ProductionVerified,
}

impl Confirmation {
    fn decides(self) -> bool {
        matches!(self, Self::FactoryConfirmed | Self::ProductionVerified)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    Min,
    Max,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Applies {
    #[serde(default)]
    pub process: Vec<String>,
    #[serde(default)]
    pub material: Vec<String>,
    #[serde(default)]
    pub thickness_mm: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Die {
    #[serde(default)]
    pub v_mm: f64,
    #[serde(default)]
    pub forms_radius_mm: Vec<f64>,
    #[serde(default)]
    pub required_share: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Requirement {
    pub rule: String,
    pub limit: Limit,
    pub value: BTreeMap<String, f64>,
    pub inputs: BTreeMap<String, String>,
    pub measuring_line: String,
    pub method: String,
    pub source: String,
    pub rights: String,
    pub applies: Applies,
    #[serde(default)]
    pub exceptions: Vec<String>,
    #[serde(default)]
    pub validation_samples: Vec<String>,
    #[serde(default)]
    pub die: Option<Die>,
    #[serde(default)]
    pub bends: Option<Vec<Option<i64>>>,
    /// None means the file is older than FD-7; such a number never decides.
    #[serde(default)]
    pub confirmation: Option<Confirmation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Profile {
    pub format: u32,
    pub name: String,
    pub version: String,
    pub layer: Layer,
    pub requirements: Vec<Requirement>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Declared {
    #[serde(default)]
    pub process: Option<String>,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub thickness_mm: Option<f64>,
    #[serde(default)]
    pub thickness_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Override {
    pub rule: String,
    pub limit: Limit,
    pub value: BTreeMap<String, f64>,
    pub reason: String,
    pub by: String,
    #[serde(default)]
    pub date: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRef {
    #[serde(default)]
    pub model_version: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Critical {
    pub name: String,
    pub from_mm: Point3,
    pub to_mm: Point3,
    pub tolerance_mm: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckInput {
    pub declared: Declared,
    pub generic: Profile,
    #[serde(default)]
    pub shop: Option<Profile>,
    #[serde(default)]
    pub overrides: Vec<Override>,
    pub facts: Facts,
    #[serde(default)]
    pub model: ModelRef,
    #[serde(default)]
    pub critical: Vec<Critical>,
}

// ---------------------------------------------------------------------------
// Report.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleStatus {
    Pass,
    Violation,
    NeedsReview,
    NotVerified,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppState {
    Passed,
    Failed,
    Advisory,
    Checked,
    Unsupported,
    NotRun,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub passed: usize,
    pub failed: usize,
    pub advisory: usize,
    pub checked: usize,
    pub unsupported: usize,
    pub not_run: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementRef {
    pub value_mm: f64,
    pub layer: Layer,
    pub confirmation: Option<Confirmation>,
    pub source: String,
    pub text: String,
    pub overridden: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feature {
    pub kind: String,
    pub number: Option<usize>,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub x_mm: f64,
    pub y_mm: f64,
    pub z_mm: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub rules: Vec<String>,
    pub status: RuleStatus,
    pub state: AppState,
    pub error_code: Option<String>,
    pub reason_code: Option<String>,
    pub message: String,
    pub feature: Feature,
    pub location: Option<Location>,
    pub measured_mm: Option<f64>,
    pub requirement: Option<RequirementRef>,
    pub needs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileIdentity {
    pub name: String,
    pub version: String,
    pub layer: Layer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataIdentity {
    pub generic: ProfileIdentity,
    pub shop: Option<ProfileIdentity>,
    pub overrides_applied: usize,
    pub confirmed_rows: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleResult {
    pub rule_id: String,
    pub state: AppState,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema_version: u32,
    pub rules_version: String,
    pub model: ModelRef,
    pub declared: Declared,
    pub data: DataIdentity,
    pub clean: bool,
    pub counts: Counts,
    pub not_checked: String,
    pub findings: Vec<Finding>,
    pub rules: Vec<RuleResult>,
}

// ---------------------------------------------------------------------------
// Validation.

fn finite(value: f64, what: &str) -> Result<(), DfmError> {
    if value.is_finite() { Ok(()) } else { Err(invalid(format!("{what} must be finite"))) }
}

fn valid_text(value: &str, what: &str) -> Result<(), DfmError> {
    if value.is_empty() || value.len() > 300 || value.contains('\\') || value.contains('\0') {
        return Err(invalid(format!("{what} must be one short line of plain text")));
    }
    Ok(())
}

fn validate_requirement(requirement: &Requirement) -> Result<(), DfmError> {
    if !RULE_ORDER.contains(&requirement.rule.as_str()) || requirement.rule.contains("gate_") {
        return Err(invalid(format!("{} is not a rule in the catalog", requirement.rule)));
    }
    if requirement.value.is_empty() {
        return Err(invalid(format!("{} has no number", requirement.rule)));
    }
    for (name, number) in &requirement.value {
        if !matches!(name.as_str(), "mm" | "t" | "r" | "d" | "b") {
            return Err(invalid(format!("{} uses the unknown input {name}", requirement.rule)));
        }
        finite(*number, &format!("{}'s {name}", requirement.rule))?;
    }
    for (name, unit) in &requirement.inputs {
        if unit != "mm" {
            return Err(invalid(format!("{}'s input {name} must be in mm", requirement.rule)));
        }
    }
    valid_text(&requirement.measuring_line, "measuring_line")?;
    valid_text(&requirement.source, "source")?;
    valid_text(&requirement.rights, "rights")?;
    if let Some([low, high]) = requirement.applies.thickness_mm {
        finite(low, "applies.thickness_mm low")?;
        finite(high, "applies.thickness_mm high")?;
        if low > high {
            return Err(invalid("applies.thickness_mm is reversed"));
        }
    }
    Ok(())
}

fn validate_profile(profile: &Profile, what: &str) -> Result<(), DfmError> {
    if profile.format != 1 {
        return Err(invalid(format!("{what}: format must be 1")));
    }
    if profile.name.is_empty() || profile.name.len() > 64 || profile.version.is_empty() || profile.version.len() > 64 {
        return Err(invalid(format!("{what}: name and version must be short plain text")));
    }
    if profile.requirements.len() > 2000 {
        return Err(invalid(format!("{what}: more than 2000 requirements")));
    }
    for requirement in &profile.requirements {
        validate_requirement(requirement)?;
    }
    Ok(())
}

fn validate_facts(facts: &Facts) -> Result<(), DfmError> {
    if let Some(thickness) = facts.paired_mm {
        finite(thickness, "facts.paired_mm")?;
        if thickness <= 0.0 {
            return Err(invalid("facts.paired_mm must be positive"));
        }
    }
    for hole in facts.holes.iter().flatten() {
        for (value, what) in
            [(hole.diameter_mm, "diameter_mm"), (hole.x_mm, "x_mm"), (hole.y_mm, "y_mm"), (hole.z_mm, "z_mm"), (hole.depth_mm, "depth_mm")]
        {
            finite(value, what)?;
        }
        if let Some(edge) = hole.edge_mm {
            finite(edge, "edge_mm")?;
        }
        for gap in &hole.bend_gaps {
            finite(gap.gap_mm, "bend_gaps.gap_mm")?;
            finite(gap.inside_radius_mm, "bend_gaps.inside_radius_mm")?;
        }
    }
    if let Some(recognition) = &facts.recognition {
        for bend in &recognition.bends {
            for (value, what) in [
                (bend.inside_radius_mm, "bends.inside_radius_mm"),
                (bend.angle_deg, "bends.angle_deg"),
                (bend.length_mm, "bends.length_mm"),
                (bend.x_mm, "bends.x_mm"),
                (bend.y_mm, "bends.y_mm"),
                (bend.z_mm, "bends.z_mm"),
            ] {
                finite(value, what)?;
            }
        }
    }
    Ok(())
}

/// Validate everything before choosing a number. A caller must not turn a bad input into a result.
pub fn validate(input: &CheckInput) -> Result<(), DfmError> {
    if let Some(thickness) = input.declared.thickness_mm {
        finite(thickness, "declared.thickness_mm")?;
        if thickness <= 0.0 {
            return Err(invalid("declared.thickness_mm must be positive"));
        }
    }
    validate_profile(&input.generic, "generic")?;
    if let Some(shop) = &input.shop {
        validate_profile(shop, "shop")?;
    }
    for over in &input.overrides {
        if !RULE_ORDER.contains(&over.rule.as_str()) || over.rule.contains("gate_") {
            return Err(invalid(format!("override for {} is not a rule", over.rule)));
        }
        if over.reason.trim().is_empty() || over.by.trim().is_empty() {
            return Err(invalid(format!("override for {} needs a reason and a signature", over.rule)));
        }
        if over.value.is_empty() {
            return Err(invalid(format!("override for {} has no number", over.rule)));
        }
        for (name, value) in &over.value {
            if !matches!(name.as_str(), "mm" | "t" | "r" | "d" | "b") {
                return Err(invalid(format!("override for {} uses {name}", over.rule)));
            }
            finite(*value, "override")?;
        }
    }
    validate_facts(&input.facts)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Number selection.

#[derive(Debug, Clone)]
pub(crate) struct Chosen {
    pub(crate) requirement: RequirementRef,
    pub(crate) limit_mm: f64,
    pub(crate) decides: bool,
}

pub(crate) struct Unsupported {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

fn matches_name(names: &[String], value: Option<&str>) -> bool {
    if names.is_empty() {
        return true;
    }
    let Some(value) = value else { return false };
    let value = value.to_lowercase();
    names.iter().any(|entry| {
        let entry = entry.to_lowercase();
        let any_temper = !entry.contains('-');
        value == entry || (any_temper && value.starts_with(&format!("{entry}-")))
    })
}

fn applies(requirement: &Requirement, declared: &Declared) -> bool {
    if !matches_name(&requirement.applies.process, declared.process.as_deref()) {
        return false;
    }
    if !matches_name(&requirement.applies.material, declared.material.as_deref()) {
        return false;
    }
    match (requirement.applies.thickness_mm, declared.thickness_mm) {
        (None, _) => true,
        (Some([low, high]), Some(thickness)) => low <= thickness && thickness <= high,
        (Some(_), None) => false,
    }
}

pub(crate) fn limit_mm(requirement: &Requirement, thickness: f64, model_thickness: Option<f64>, radius: Option<f64>, upper: bool) -> Option<f64> {
    let model = model_thickness.unwrap_or(thickness);
    let at = |t: f64| {
        let mut known = BTreeMap::from([("t", t), ("mm", 1.0)]);
        if let Some(radius) = radius {
            known.insert("r", radius);
        }
        let mut total = 0.0;
        for (name, number) in &requirement.value {
            let value = known.get(name.as_str())?;
            total += number * value;
        }
        Some(total)
    };
    let declared = at(thickness)?;
    let measured = at(model)?;
    Some(if upper { declared.min(measured) } else { declared.max(measured) })
}

pub(crate) fn choose(rule: &str, radius: Option<f64>, upper: bool, input: &CheckInput) -> Result<Chosen, Unsupported> {
    if let Some(over) = input.overrides.iter().find(|over| over.rule == rule) {
        if over.limit != Limit::Min {
            return Err(Unsupported {
                code: "REQUIREMENT_INPUT_UNKNOWN",
                message: "This rule takes a lower limit; the override is an upper limit.".into(),
            });
        }
        let requirement = Requirement {
            rule: over.rule.clone(),
            limit: over.limit,
            value: over.value.clone(),
            inputs: over.value.keys().filter(|key| *key != "mm").map(|key| (key.clone(), "mm".into())).collect(),
            measuring_line: "A project override.".into(),
            method: "A project override.".into(),
            source: format!("{}: {}", over.by, over.reason),
            rights: "The project's own number.".into(),
            applies: Applies::default(),
            exceptions: Vec::new(),
            validation_samples: Vec::new(),
            die: None,
            bends: None,
            // A project override replaces a number somebody else confirmed; it does not inherit
            // that identity. Until a person confirms this row, it is team-reported.
            confirmation: Some(Confirmation::TeamSelfReported),
        };
        let thickness = input.declared.thickness_mm.ok_or_else(|| Unsupported {
            code: "COVERAGE_NOT_DECLARED",
            message: "No thickness was declared, so the override can't be worked out.".into(),
        })?;
        let limit = limit_mm(&requirement, thickness, input.facts.paired_mm, radius, upper).ok_or_else(|| Unsupported {
            code: "REQUIREMENT_INPUT_UNKNOWN",
            message: "The override is not a limit this version can work out.".into(),
        })?;
        return Ok(Chosen {
            requirement: RequirementRef {
                value_mm: limit,
                layer: Layer::Project,
                confirmation: Some(Confirmation::TeamSelfReported),
                source: requirement.source,
                text: format!("Project override: at least {limit} mm."),
                overridden: true,
            },
            limit_mm: limit,
            decides: false,
        });
    }
    let shop_has = input.shop.as_ref().is_some_and(|shop| shop.requirements.iter().any(|row| row.rule == rule));
    let (rows, layer) = if shop_has {
        let shop = input.shop.as_ref().ok_or_else(|| Unsupported { code: "NO_SHOP_NUMBER", message: "No shop data.".into() })?;
        (&shop.requirements, shop.layer)
    } else {
        (&input.generic.requirements, input.generic.layer)
    };
    let own: Vec<&Requirement> = rows.iter().filter(|row| row.rule == rule).collect();
    if own.is_empty() {
        return Err(Unsupported {
            code: "NO_SHOP_NUMBER",
            message: "This rule needs a factory or standard number, and the selected data has none.".into(),
        });
    }
    if shop_has && (input.declared.process.is_none() || input.declared.material.is_none()) {
        return Err(Unsupported {
            code: "COVERAGE_NOT_DECLARED",
            message: "The selected data can't be looked up without the declared process and material with temper.".into(),
        });
    }
    let applying: Vec<&Requirement> = own.into_iter().filter(|row| applies(row, &input.declared)).collect();
    if applying.is_empty() {
        return Err(Unsupported {
            // The same code INTdfm uses, so the Python and Rust reports can be compared.
            code: "COVERAGE_NOT_IN_PROFILE",
            message: "The selected data has this rule, but no row applies at the declared process, material and thickness.".into(),
        });
    }
    let thickness = input.declared.thickness_mm.ok_or_else(|| Unsupported {
        code: "COVERAGE_NOT_DECLARED",
        message: "No thickness was declared, so the number can't be selected.".into(),
    })?;
    let mut chosen: Vec<Chosen> = Vec::new();
    for row in applying {
        // The first three hole rules are lower limits; a row written the other way is not this
        // rule's number and must not be turned into one.
        if row.limit != Limit::Min {
            return Err(Unsupported {
                code: "REQUIREMENT_INPUT_UNKNOWN",
                message: "This rule takes a lower limit; the selected row is an upper limit.".into(),
            });
        }
        let Some(limit) = limit_mm(row, thickness, input.facts.paired_mm, radius, upper) else {
            return Err(Unsupported {
                code: "REQUIREMENT_INPUT_UNKNOWN",
                message: "The selected row is not a limit this version can work out.".into(),
            });
        };
        let confirmation = row.confirmation;
        // The rule layer follows INTdfm: a factory/standard row decides. FD-7 confirmation is
        // applied when mapping to the App state, not when deciding the rule status.
        let decides = layer != Layer::Generic && layer != Layer::Development;
        chosen.push(Chosen {
            requirement: RequirementRef {
                value_mm: limit,
                layer,
                confirmation,
                source: row.source.clone(),
                text: format!("At least {limit} mm. {}", row.measuring_line),
                overridden: false,
            },
            limit_mm: limit,
            decides,
        });
    }
    let strictest = if upper {
        chosen.into_iter().min_by(|a, b| a.limit_mm.total_cmp(&b.limit_mm))
    } else {
        chosen.into_iter().max_by(|a, b| a.limit_mm.total_cmp(&b.limit_mm))
    };
    strictest.ok_or_else(|| Unsupported { code: "NO_SHOP_NUMBER", message: "No number applies.".into() })
}

// ---------------------------------------------------------------------------
// Gates and findings.

pub(crate) fn state_of(status: RuleStatus, decides: bool) -> AppState {
    match status {
        RuleStatus::Pass if decides => AppState::Passed,
        RuleStatus::Pass => AppState::Checked,
        RuleStatus::NeedsReview if decides => AppState::Failed,
        RuleStatus::NeedsReview => AppState::Advisory,
        RuleStatus::Violation => AppState::Failed,
        RuleStatus::NotVerified => AppState::Unsupported,
        RuleStatus::NotApplicable => AppState::Checked,
    }
}

fn gate_finding(
    status: RuleStatus,
    state: AppState,
    error_code: Option<&str>,
    reason_code: Option<&str>,
    message: String,
    measured_mm: Option<f64>,
) -> Finding {
    Finding {
        id: format!("{THICKNESS_GATE}/{}", error_code.or(reason_code).unwrap_or("pass")),
        rules: vec![THICKNESS_GATE.to_string()],
        status,
        state,
        error_code: error_code.map(str::to_string),
        reason_code: reason_code.map(str::to_string),
        message,
        feature: Feature { kind: "sheet".into(), number: None, description: "the sheet".into() },
        location: None,
        measured_mm,
        requirement: None,
        needs: if status == RuleStatus::Pass { Vec::new() } else { vec!["The right declared thickness, or a model at that thickness.".into()] },
    }
}

fn thickness_gate(input: &CheckInput) -> Finding {
    let model = input.facts.paired_mm;
    let Some(model) = model else {
        return gate_finding(
            RuleStatus::NotVerified,
            AppState::Unsupported,
            None,
            Some("THICKNESS_NOT_MEASURED"),
            "The model's sheet thickness was not measured, so the thickness gate did not run.".into(),
            None,
        );
    };
    let Some(declared) = input.declared.thickness_mm else {
        return gate_finding(
            RuleStatus::NotVerified,
            AppState::Unsupported,
            None,
            Some("THICKNESS_NOT_DECLARED"),
            "No thickness was declared, so there is nothing to compare the model with. Rules that depend on thickness did not run.".into(),
            Some(model),
        );
    };
    if (model - declared).abs() <= THICKNESS_TOLERANCE_MM {
        gate_finding(
            RuleStatus::Pass,
            AppState::Checked,
            None,
            None,
            format!("The model's {model} mm sheet matches the declared {declared} mm within {THICKNESS_TOLERANCE_MM} mm."),
            Some(model),
        )
    } else {
        gate_finding(
            RuleStatus::Violation,
            AppState::Failed,
            Some("THICKNESS_MISMATCH"),
            None,
            format!("The model's sheet is {model} mm, not the declared {declared} mm. Rules that depend on thickness did not run."),
            Some(model),
        )
    }
}

fn blocked(rule: &str, gate: &Finding) -> Finding {
    let (status, code) = if gate.status == RuleStatus::Violation {
        (RuleStatus::NotVerified, "THICKNESS_MISMATCH")
    } else {
        (RuleStatus::NotVerified, gate.reason_code.as_deref().unwrap_or("THICKNESS_NOT_DECLARED"))
    };
    Finding {
        id: format!("{rule}/{code}"),
        rules: vec![rule.to_string()],
        status,
        state: AppState::Unsupported,
        error_code: None,
        reason_code: Some(code.to_string()),
        message: gate.message.clone(),
        feature: Feature { kind: "sheet".into(), number: None, description: "the sheet".into() },
        location: None,
        measured_mm: None,
        requirement: None,
        needs: gate.needs.clone(),
    }
}

pub(crate) fn hole_finding(
    rule: &str,
    status: RuleStatus,
    error_code: Option<&str>,
    reason_code: Option<&str>,
    hole_number: Option<usize>,
    location: Option<&HoleFact>,
    measured_mm: Option<f64>,
    requirement: Option<RequirementRef>,
    message: String,
    needs: Vec<String>,
) -> Finding {
    let feature = match (hole_number, location) {
        (Some(number), Some(hole)) => {
            Feature { kind: "hole".into(), number: Some(number), description: format!("round hole, {} mm diameter", hole.diameter_mm) }
        }
        _ => Feature { kind: "sheet".into(), number: None, description: "the sheet".into() },
    };
    let id = match (hole_number, location) {
        (Some(number), Some(hole)) => format!("{rule}/hole-{number}@{:.3},{:.3},{:.3}", hole.x_mm, hole.y_mm, hole.z_mm),
        _ => format!("{rule}/{}", reason_code.or(error_code).unwrap_or("result")),
    };
    let app_decides = requirement.as_ref().is_some_and(|requirement| {
        requirement.confirmation.is_some_and(Confirmation::decides) && requirement.layer != Layer::Generic && requirement.layer != Layer::Development
    });
    // Reaching a public value is "checked", not "pass" and not "unsupported": the rule ran and
    // only a factory's own number could say the hole is fine.
    let state = if status == RuleStatus::NotVerified && reason_code == Some("ABOVE_PUBLIC_VALUE") {
        AppState::Checked
    } else {
        state_of(status, app_decides)
    };
    Finding {
        id,
        rules: vec![rule.to_string()],
        status,
        state,
        error_code: error_code.map(str::to_string),
        reason_code: reason_code.map(str::to_string),
        message,
        feature,
        location: location.map(|hole| Location { x_mm: round3(hole.x_mm), y_mm: round3(hole.y_mm), z_mm: round3(hole.z_mm) }),
        measured_mm,
        requirement,
        needs,
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

pub(crate) fn below(value: f64, limit: f64, upper: bool) -> bool {
    let past = round6(value - limit);
    if upper { past > 0.0 } else { past < 0.0 }
}

fn usable_holes(facts: &Facts) -> Result<Vec<(usize, &HoleFact)>, Unsupported> {
    let Some(holes) = &facts.holes else {
        return Err(Unsupported { code: "HOLES_UNRECOGNISED", message: "The step that finds holes failed, so no hole was checked.".into() });
    };
    if holes.is_empty() {
        return Err(Unsupported { code: "NO_OPENINGS", message: "Every face has a label, and no opening is closed within the skins.".into() });
    }
    let thickness = facts.paired_mm.unwrap_or(0.0);
    let usable: Vec<(usize, &HoleFact)> = holes
        .iter()
        .enumerate()
        .filter(|(_, hole)| hole.square && (hole.depth_mm - thickness).abs() <= SAME_SHEET_MM)
        .map(|(index, hole)| (index + 1, hole))
        .collect();
    if usable.is_empty() {
        return Err(Unsupported {
            code: "HOLES_UNRECOGNISED",
            message: "Every round hole here is tilted, blind, or not through the sheet, so none was checked.".into(),
        });
    }
    Ok(usable)
}

pub(crate) fn unsupported_finding(rule: &str, unsupported: Unsupported) -> Finding {
    Finding {
        id: format!("{rule}/{}", unsupported.code),
        rules: vec![rule.to_string()],
        status: RuleStatus::NotVerified,
        state: AppState::Unsupported,
        error_code: None,
        reason_code: Some(unsupported.code.into()),
        message: unsupported.message,
        feature: Feature { kind: "sheet".into(), number: None, description: "the sheet".into() },
        location: None,
        measured_mm: None,
        requirement: None,
        needs: vec!["A person to look at those features, because this version can't check them.".into()],
    }
}

pub(crate) fn not_applicable(rule: &str, message: &str) -> Finding {
    Finding {
        id: format!("{rule}/not-applicable"),
        rules: vec![rule.to_string()],
        status: RuleStatus::NotApplicable,
        state: AppState::Checked,
        error_code: None,
        reason_code: None,
        message: message.into(),
        feature: Feature { kind: "sheet".into(), number: None, description: "the sheet".into() },
        location: None,
        measured_mm: None,
        requirement: None,
        needs: Vec::new(),
    }
}

fn unread_findings(rule: &str, facts: &Facts) -> Vec<Finding> {
    let mut found = Vec::new();
    let faces = facts.unknown_faces
        + facts.recognition.as_ref().map_or(0, |recognition| recognition.unknown_faces + recognition.cone_faces + recognition.form_faces);
    if facts.other_openings > 0 || faces > 0 {
        let mut message = String::new();
        if facts.other_openings > 0 {
            let _ = write!(message, "{} opening(s) that are not round holes, ", facts.other_openings);
        }
        if faces > 0 {
            let _ = write!(message, "{faces} face(s) no rule here can read, ");
        }
        message.push_str("so those can't be checked.");
        found.push(unsupported_finding(rule, Unsupported { code: "FEATURES_NOT_CHECKED", message }));
    }
    found
}

fn min_diameter(input: &CheckInput, gate: &Finding, usable: &[(usize, &HoleFact)]) -> Vec<Finding> {
    let rule = HOLE_MIN_DIAMETER;
    let chosen = match choose(rule, None, false, input) {
        Ok(chosen) => chosen,
        Err(unsupported) => return vec![unsupported_finding(rule, unsupported)],
    };
    let mut results = Vec::new();
    let mut met = Vec::new();
    for (number, hole) in usable {
        if below(hole.diameter_mm, chosen.limit_mm, false) {
            results.push(hole_finding(
                rule,
                RuleStatus::NeedsReview,
                Some("HOLE_DIAMETER_BELOW_LIMIT"),
                None,
                Some(*number),
                Some(hole),
                Some(hole.diameter_mm),
                Some(chosen.requirement.clone()),
                format!("Hole {number}: its diameter is {} mm, below {:.6} mm.", hole.diameter_mm, chosen.limit_mm),
                vec!["A person to look at this hole.".into()],
            ));
        } else {
            met.push(hole.diameter_mm);
        }
    }
    if let Some(tightest) = met.into_iter().reduce(f64::min) {
        let status = if chosen.decides { RuleStatus::Pass } else { RuleStatus::NotVerified };
        let code = if chosen.decides { None } else { Some("ABOVE_PUBLIC_VALUE") };
        let message = if chosen.decides {
            format!("The holes met the number, the tightest at {tightest} mm.")
        } else {
            "These holes are clear of the public value, so nothing is known about them: only a factory's own number can say a hole is fine.".into()
        };
        results.push(hole_finding(
            rule,
            status,
            None,
            code,
            None,
            None,
            Some(tightest),
            Some(chosen.requirement),
            message,
            if chosen.decides { Vec::new() } else { vec!["The shop's own number for this rule.".into()] },
        ));
    }
    if results.is_empty() {
        results.push(not_applicable(rule, "No round through hole was found."));
    }
    let _ = gate;
    results.extend(unread_findings(rule, &input.facts));
    results
}

fn hole_to_edge(input: &CheckInput, gate: &Finding, usable: &[(usize, &HoleFact)]) -> Vec<Finding> {
    let rule = HOLE_TO_EDGE;
    let chosen = match choose(rule, None, false, input) {
        Ok(chosen) => chosen,
        Err(unsupported) => return vec![unsupported_finding(rule, unsupported)],
    };
    let mut results = Vec::new();
    let mut met = Vec::new();
    let mut unclear = Vec::new();
    for (number, hole) in usable {
        match hole.edge_mm {
            Some(value) if below(value, chosen.limit_mm, false) => results.push(hole_finding(
                rule,
                RuleStatus::NeedsReview,
                Some("HOLE_EDGE_BELOW_LIMIT"),
                None,
                Some(*number),
                Some(hole),
                Some(value),
                Some(chosen.requirement.clone()),
                format!("Hole {number}: its wall is {value} mm from the nearest cut edge, below {:.6} mm.", chosen.limit_mm),
                vec!["A person to look at this hole.".into()],
            )),
            Some(value) => met.push(value),
            None => unclear.push(*number),
        }
    }
    if !unclear.is_empty() {
        results.push(unsupported_finding(
            rule,
            Unsupported { code: "HOLE_EDGE_UNCLEAR", message: format!("The nearest cut edge can't be told for {} round hole(s).", unclear.len()) },
        ));
    }
    if let Some(tightest) = met.into_iter().reduce(f64::min) {
        let status = if chosen.decides { RuleStatus::Pass } else { RuleStatus::NotVerified };
        let code = if chosen.decides { None } else { Some("ABOVE_PUBLIC_VALUE") };
        results.push(hole_finding(
            rule,
            status,
            None,
            code,
            None,
            None,
            Some(tightest),
            Some(chosen.requirement),
            if chosen.decides {
                format!("The holes met the number, the tightest at {tightest} mm.")
            } else {
                "These holes are clear of the public value, so nothing is known about them: only a factory's own number can say a hole is fine."
                    .into()
            },
            if chosen.decides { Vec::new() } else { vec!["The shop's own number for this rule.".into()] },
        ));
    }
    if results.is_empty() {
        results.push(not_applicable(rule, "No round through hole with a measurable edge was found."));
    }
    let _ = gate;
    results.extend(unread_findings(rule, &input.facts));
    results
}

fn hole_to_bend(input: &CheckInput, gate: &Finding, usable: &[(usize, &HoleFact)]) -> Vec<Finding> {
    let rule = HOLE_TO_BEND;
    let _ = gate;
    let Some(recognition) = &input.facts.recognition else {
        return vec![not_applicable(rule, "The bend step did not finish, so there is no bend to measure.")];
    };
    if recognition.bends.is_empty() {
        return vec![not_applicable(rule, "This part has no bend, so hole-to-bend has nothing to measure.")];
    }
    let mut results = Vec::new();
    let mut met: Vec<(f64, f64, Chosen)> = Vec::new();
    let mut unmeasured = false;
    for (number, hole) in usable {
        if hole.bend_gaps.is_empty() {
            unmeasured = true;
            continue;
        }
        let mut nearest: Option<(f64, f64, usize, Chosen)> = None;
        for gap in &hole.bend_gaps {
            let chosen = match choose(rule, Some(gap.inside_radius_mm), false, input) {
                Ok(chosen) => chosen,
                Err(unsupported) => return vec![unsupported_finding(rule, unsupported)],
            };
            let margin = gap.gap_mm - chosen.limit_mm;
            if nearest.as_ref().is_none_or(|current| margin < current.0) {
                nearest = Some((margin, gap.gap_mm, gap.bend, chosen));
            }
        }
        let Some((_, value, bend, chosen)) = nearest else {
            unmeasured = true;
            continue;
        };
        if below(value, chosen.limit_mm, false) {
            results.push(hole_finding(
                rule,
                RuleStatus::NeedsReview,
                Some("HOLE_BEND_BELOW_LIMIT"),
                None,
                Some(*number),
                Some(hole),
                Some(value),
                Some(chosen.requirement.clone()),
                format!("Hole {number}: its wall is {value} mm from bend {bend}, below {:.6} mm.", chosen.limit_mm),
                vec!["A person to look at this hole.".into()],
            ));
        } else {
            met.push((value - chosen.limit_mm, value, chosen));
        }
    }
    if unmeasured {
        results.push(unsupported_finding(
            rule,
            Unsupported {
                code: "NO_FLAT_TO_A_BEND",
                message: "A hole here isn't on a flat that ends at a bend, so its distance to a bend wasn't measured.".into(),
            },
        ));
    }
    if let Some((_, value, chosen)) = met.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)) {
        let status = if chosen.decides { RuleStatus::Pass } else { RuleStatus::NotVerified };
        let code = if chosen.decides { None } else { Some("ABOVE_PUBLIC_VALUE") };
        results.push(hole_finding(
            rule,
            status,
            None,
            code,
            None,
            None,
            Some(value),
            Some(chosen.requirement),
            if chosen.decides {
                format!("The holes met the number, the tightest at {value} mm.")
            } else {
                "These holes are clear of the public value, so nothing is known about them: only a factory's own number can say a hole is fine."
                    .into()
            },
            Vec::new(),
        ));
    }
    if results.is_empty() {
        results.push(not_applicable(rule, "No round through hole with a measurable bend was found."));
    }
    results.extend(unread_findings(rule, &input.facts));
    results
}

fn rule_order(rule: &str) -> usize {
    RULE_ORDER.iter().position(|known| *known == rule).unwrap_or(RULE_ORDER.len())
}

/// Run the thickness gate and the first three hole rules. Deterministic for one input.
pub fn check(input: &CheckInput) -> Result<Report, DfmError> {
    validate(input)?;
    let gate = thickness_gate(input);
    let mut findings = vec![gate.clone()];
    let usable = usable_holes(&input.facts);
    let flat = input.facts.recognition.as_ref().is_some_and(|recognition| recognition.bends.is_empty());
    for rule in FIRST_HOLE_RULES {
        // A flat part has no bend to measure, so this is not a thickness-blocked result.
        if *rule == HOLE_TO_BEND && flat {
            findings.push(not_applicable(rule, "This part has no bend, so hole-to-bend has nothing to measure."));
            continue;
        }
        if gate.status != RuleStatus::Pass {
            findings.push(blocked(rule, &gate));
            continue;
        }
        let unread = input.facts.other_openings > 0
            || input.facts.unknown_faces > 0
            || input
                .facts
                .recognition
                .as_ref()
                .is_some_and(|recognition| recognition.unknown_faces + recognition.cone_faces + recognition.form_faces > 0);
        if input.facts.holes.as_ref().is_some_and(Vec::is_empty) && !unread {
            findings.push(not_applicable(rule, "No opening is closed within the skins."));
            continue;
        }
        match &usable {
            Err(unsupported) => {
                findings.push(unsupported_finding(rule, Unsupported { code: unsupported.code, message: unsupported.message.clone() }))
            }
            Ok(usable) => {
                let mut found = match *rule {
                    HOLE_MIN_DIAMETER => min_diameter(input, &gate, usable),
                    HOLE_TO_EDGE => hole_to_edge(input, &gate, usable),
                    _ => hole_to_bend(input, &gate, usable),
                };
                findings.append(&mut found);
            }
        }
    }
    findings.sort_by(|a, b| {
        rule_order(&a.rule())
            .cmp(&rule_order(&b.rule()))
            .then_with(|| {
                let key =
                    |finding: &Finding| finding.location.as_ref().map_or((0.0, 0.0, 0.0), |location| (location.x_mm, location.y_mm, location.z_mm));
                key(a).0.total_cmp(&key(b).0).then(key(a).1.total_cmp(&key(b).1)).then(key(a).2.total_cmp(&key(b).2))
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut counts = Counts::default();
    for finding in &findings {
        match finding.state {
            AppState::Passed => counts.passed += 1,
            AppState::Failed => counts.failed += 1,
            AppState::Advisory => counts.advisory += 1,
            AppState::Checked => counts.checked += 1,
            AppState::Unsupported => counts.unsupported += 1,
            AppState::NotRun => counts.not_run += 1,
        }
    }
    // A public value never makes a part clean: only a confirmed number's pass can.
    let clean = counts.failed == 0 && counts.advisory == 0 && counts.unsupported == 0 && counts.passed > 0;
    let mut rules = Vec::new();
    for rule in FIRST_HOLE_RULES {
        let own: Vec<&Finding> = findings.iter().filter(|finding| finding.rules.iter().any(|name| name == rule)).collect();
        let state = own.first().map_or(AppState::NotRun, |finding| finding.state);
        rules.push(RuleResult { rule_id: (*rule).to_string(), state, count: own.len() });
    }
    let overridden: BTreeSet<&str> = input.overrides.iter().map(|over| over.rule.as_str()).collect();
    let confirmed_rows = input
        .generic
        .requirements
        .iter()
        .chain(input.shop.iter().flat_map(|shop| shop.requirements.iter()))
        .filter(|row| !overridden.contains(row.rule.as_str()) && row.confirmation.is_some_and(Confirmation::decides))
        .count();
    Ok(Report {
        schema_version: SCHEMA_VERSION,
        rules_version: VERSION.into(),
        model: input.model.clone(),
        declared: input.declared.clone(),
        data: DataIdentity {
            generic: ProfileIdentity { name: input.generic.name.clone(), version: input.generic.version.clone(), layer: input.generic.layer },
            shop: input.shop.as_ref().map(|shop| ProfileIdentity { name: shop.name.clone(), version: shop.version.clone(), layer: shop.layer }),
            overrides_applied: input.overrides.len(),
            confirmed_rows,
        },
        clean,
        counts,
        not_checked: NOT_CHECKED.into(),
        findings,
        rules,
    })
}

impl Finding {
    fn rule(&self) -> String {
        self.rules.first().cloned().unwrap_or_default()
    }
}

/// The canonical JSON an evidence file and a comparison use.
pub fn canonical_json(report: &Report) -> Result<String, DfmError> {
    serde_json::to_string_pretty(report).map_err(|error| invalid(format!("report JSON: {error}")))
}

/// The rules this crate implements today. Used by the App to say what is not covered.
pub fn implemented_rules() -> BTreeSet<&'static str> {
    [THICKNESS_GATE, HOLE_MIN_DIAMETER, HOLE_TO_EDGE, HOLE_TO_BEND].into_iter().collect()
}
