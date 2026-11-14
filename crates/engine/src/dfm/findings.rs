//! The one finding shape every stream shares (CONTRACT §2, `findings.json` schema 2).
//!
//! The rule layer decides what each rule saw; this module is the only place that turns its status
//! and the data layer's confirmation into a [`FindingState`] (§4.5 of the technical design). A
//! generic or team-reported number never becomes `failed`, `not_verified` becomes `checked` or
//! `unsupported` with a reason, and `not_applicable` is never a pass.
//!
//! Identical facts and configuration produce byte-identical `findings.json`: no times, no paths
//! and no map iteration order reach the serialised form.

use serde::{Deserialize, Serialize};

use solidcraft_dfm_rules::{Confirmation, FactFamily, Layer, RuleStatus};

use crate::dfm::rule_facts;
use solidcraft_dfm_rules::Report as RuleReport;

/// The findings document version.
pub const SCHEMA_VERSION: u32 = 2;

/// Where the checked geometry came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SourceRef {
    /// Sheet-metal bodies measured in this document.
    Native,
    /// A converted import: the SHA-256 of the file it was converted from.
    Converted { file_sha256: String },
}

impl SourceRef {
    /// Read the model source the App recorded, such as `native` or `converted <sha256>`.
    pub fn from_source(source: &str) -> SourceRef {
        match source.strip_prefix("converted ") {
            Some(digest) if !digest.trim().is_empty() => SourceRef::Converted { file_sha256: digest.trim().to_string() },
            _ => SourceRef::Native,
        }
    }
}

/// The state A prints and B colours. Decided here, once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingState {
    Passed,
    /// Measured against a number that is not confirmed as the factory's; not a pass.
    Checked,
    /// Needs a person to look; not a pass.
    Advisory,
    Failed,
    /// The rule could not decide, or its facts were missing; not a pass.
    Unsupported,
    /// The rule does not apply to this feature; not a pass.
    NotApplicable,
    /// The rule was not run at all; not a pass.
    NotRun,
}

impl FindingState {
    /// Only [`FindingState::Passed`] is a pass. Everything else keeps a report from summarising
    /// as one.
    pub fn is_pass(self) -> bool {
        self == FindingState::Passed
    }

    /// The worst state wins a rule row shared by several sheets.
    pub fn rank(self) -> u8 {
        match self {
            FindingState::Failed => 6,
            FindingState::Unsupported => 5,
            FindingState::NotRun => 4,
            FindingState::Advisory => 3,
            FindingState::Checked => 2,
            FindingState::NotApplicable => 1,
            FindingState::Passed => 0,
        }
    }
}

/// Which layer contributed a finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingFrom {
    Rules,
    App,
}

/// A measured value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measure {
    pub value: f64,
    pub unit: MeasureUnit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeasureUnit {
    #[serde(rename = "mm")]
    Millimetre,
    #[serde(rename = "mm2")]
    SquareMillimetre,
    #[serde(rename = "deg")]
    Degree,
}

/// The number a finding was compared against, as the rule layer resolved it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementView {
    pub value: f64,
    pub unit: MeasureUnit,
    pub layer: Layer,
    /// `None` when the number has no factory confirmation; A must not turn that red.
    pub confirmed: Option<Confirmation>,
    pub source: String,
}

/// The feature a finding belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureRef {
    pub kind: String,
    pub number: Option<usize>,
}

/// One finding, addressed by `(runId, id)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// `sheet-1/F2`: run-scoped and stable inside one run only.
    pub id: String,
    pub sheet: usize,
    pub body: String,
    pub from: FindingFrom,
    pub rules: Vec<String>,
    pub state: FindingState,
    /// The rule layer's status, verbatim.
    pub status: String,
    pub code: Option<String>,
    pub measured: Option<Measure>,
    pub requirement: Option<RequirementView>,
    pub feature: Option<FeatureRef>,
    /// Folded model space, in the same model version.
    pub anchor: Option<[f64; 3]>,
    /// Flat-pattern coordinates, when the rule layer gave them.
    pub flat: Option<[f64; 2]>,
    /// Named faces a mark may tint; may be empty.
    pub faces: Vec<String>,
    /// The flat outline of a hole or region; may be empty.
    pub outline: Vec<[f64; 3]>,
}

/// One row per rule id for the summary card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleRow {
    pub rule: String,
    pub state: FindingState,
    pub count: usize,
}

/// Everything one run's findings and identity carry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFindings {
    pub schema_version: u32,
    pub run_id: String,
    /// The model identity staleness compares.
    pub candidate_digest: String,
    /// The check-configuration identity.
    pub profile_digest: String,
    /// The project configuration identity (the frozen inputs).
    pub config_digest: String,
    pub model_version: String,
    pub project_id: String,
    pub source: SourceRef,
    pub findings: Vec<Finding>,
    pub rules: Vec<RuleRow>,
    /// What was not checked, in words, including every not-covered reason.
    pub not_checked: String,
}

/// The identity fields one findings document binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindingsIdentity {
    pub run_id: String,
    pub candidate_digest: String,
    pub profile_digest: String,
    pub config_digest: String,
    pub model_version: String,
    pub project_id: String,
    pub source: SourceRef,
}

/// One sheet body's measured facts and rule report.
#[derive(Clone, Debug)]
pub struct SheetCheck {
    pub sheet: usize,
    pub body: String,
    pub facts: solidcraft_doc::sheet_dfm::SheetFacts,
    pub report: RuleReport,
}

/// Build the run's findings from each sheet's completed rule report.
///
/// Order is sheet order, then the rule layer's own finding order; the same inputs always give the
/// same bytes. `extra_not_checked` names bodies that produced no rule report (no applicable
/// object, unmeasurable facts, a failed rebuild); they stay in the report with their reason.
pub fn build(identity: &FindingsIdentity, sheets: &[SheetCheck]) -> RunFindings {
    build_with_reasons(identity, sheets, &[])
}

/// [`build`] plus the not-covered reasons for bodies with no rule report.
pub fn build_with_reasons(identity: &FindingsIdentity, sheets: &[SheetCheck], extra_not_checked: &[String]) -> RunFindings {
    let mut findings = Vec::new();
    let mut not_checked: Vec<String> = extra_not_checked.to_vec();
    let mut rule_rows: Vec<(String, FindingState, usize)> = Vec::new();
    for sheet in sheets {
        for (index, finding) in sheet.report.findings.iter().enumerate() {
            let id = format!("sheet-{}/F{}", sheet.sheet + 1, index + 1);
            let angle_rule = finding.rules.iter().any(|rule| rule == "sm.countersink_angle");
            let state = map_state(finding);
            findings.push(Finding {
                id,
                sheet: sheet.sheet,
                body: sheet.body.clone(),
                from: from_layer(finding),
                rules: finding.rules.clone(),
                state,
                status: status_name(finding.status).to_string(),
                code: reason_code(finding, state),
                measured: finding.measured_mm.map(|value| Measure {
                    value,
                    unit: if finding.rules.iter().any(|rule| rule == "app.flat_no_overlap") {
                        MeasureUnit::SquareMillimetre
                    } else if angle_rule {
                        MeasureUnit::Degree
                    } else {
                        MeasureUnit::Millimetre
                    },
                }),
                requirement: finding.requirement.as_ref().map(|requirement| requirement_view(finding, requirement)),
                feature: Some(FeatureRef { kind: finding.feature.kind.clone(), number: finding.feature.number }),
                anchor: finding.location.as_ref().map(|location| [location.x_mm, location.y_mm, location.z_mm]),
                flat: None,
                faces: Vec::new(),
                outline: Vec::new(),
            });
            for rule in &finding.rules {
                match rule_rows.iter_mut().find(|(name, _, _)| name == rule) {
                    Some((_, worst, count)) => {
                        *worst = (*worst).max(state);
                        *count += 1;
                    }
                    None => rule_rows.push((rule.clone(), state, 1)),
                }
            }
        }
        // Every rule the layer ran gets a row, even when it contributed no finding, so the
        // summary card never looks as if a rule was skipped.
        for rule in &sheet.report.rules {
            match rule_rows.iter_mut().find(|(name, _, _)| name == &rule.rule_id) {
                Some((_, worst, _)) => *worst = (*worst).max(rule_state(rule.state)),
                None => rule_rows.push((rule.rule_id.clone(), rule_state(rule.state), 0)),
            }
        }
        let missing = missing_families(&sheet.facts);
        if !missing.is_empty() {
            not_checked.push(format!("Sheet {} (`{}`): the App did not measure {}.", sheet.sheet + 1, sheet.body, missing.join(", ")));
        }
        if sheet.facts.bores > 0 {
            not_checked.push(format!(
                "Sheet {} (`{}`): {} bore(s) are not through holes, so the hole rules did not check them.",
                sheet.sheet + 1,
                sheet.body,
                sheet.facts.bores
            ));
        }
        if sheet.facts.other_openings > 0 {
            not_checked.push(format!(
                "Sheet {} (`{}`): {} non-circular opening(s) were not checked by a hole rule.",
                sheet.sheet + 1,
                sheet.body,
                sheet.facts.other_openings
            ));
        }
        if let Some(feature) = &sheet.facts.altered_by {
            not_checked.push(format!(
                "Sheet {} (`{}`): the body was changed after its sheet-metal features by {feature}, so it was not unfolded.",
                sheet.sheet + 1,
                sheet.body
            ));
        }
        for rule in &sheet.report.rules {
            if rule.state == solidcraft_dfm_rules::AppState::Unsupported {
                not_checked.push(format!("Sheet {} (`{}`): {} was not verified.", sheet.sheet + 1, sheet.body, rule.rule_id));
            }
        }
    }
    if sheets.is_empty() {
        not_checked.push("No sheet-metal body was found, so no DFM rule had an applicable object.".into());
    }
    rule_rows.sort_by(|a, b| rule_position(&a.0).cmp(&rule_position(&b.0)).then_with(|| a.0.cmp(&b.0)));
    not_checked.insert(0, solidcraft_dfm_rules::NOT_CHECKED.to_string());
    RunFindings {
        schema_version: SCHEMA_VERSION,
        run_id: identity.run_id.clone(),
        candidate_digest: identity.candidate_digest.clone(),
        profile_digest: identity.profile_digest.clone(),
        config_digest: identity.config_digest.clone(),
        model_version: identity.model_version.clone(),
        project_id: identity.project_id.clone(),
        source: identity.source.clone(),
        findings,
        rules: rule_rows.into_iter().map(|(rule, state, count)| RuleRow { rule, state, count }).collect(),
        not_checked: not_checked.join("\n"),
    }
}

/// The rule catalog's position; a rule id it does not know sorts last.
fn rule_position(rule: &str) -> usize {
    solidcraft_dfm_rules::RULE_ORDER.iter().position(|known| *known == rule).unwrap_or(usize::MAX)
}

fn rule_state(state: solidcraft_dfm_rules::AppState) -> FindingState {
    match state {
        solidcraft_dfm_rules::AppState::Passed => FindingState::Passed,
        solidcraft_dfm_rules::AppState::Failed => FindingState::Failed,
        solidcraft_dfm_rules::AppState::Advisory => FindingState::Advisory,
        solidcraft_dfm_rules::AppState::Checked => FindingState::Checked,
        solidcraft_dfm_rules::AppState::Unsupported => FindingState::Unsupported,
        solidcraft_dfm_rules::AppState::NotRun => FindingState::NotRun,
    }
}

fn status_name(status: RuleStatus) -> &'static str {
    match status {
        RuleStatus::Pass => "pass",
        RuleStatus::Violation => "violation",
        RuleStatus::NeedsReview => "needs_review",
        RuleStatus::NotVerified => "not_verified",
        RuleStatus::NotApplicable => "not_applicable",
    }
}

fn from_layer(finding: &solidcraft_dfm_rules::Finding) -> FindingFrom {
    if finding.rules.iter().any(|rule| rule.starts_with("app.")) { FindingFrom::App } else { FindingFrom::Rules }
}

fn confirms(finding: &solidcraft_dfm_rules::Finding) -> bool {
    finding.requirement.as_ref().is_some_and(|requirement| {
        // A generic or development row is a reference value, never a factory decision, even if a
        // file mistakenly carries a confirmation on it.
        matches!(requirement.layer, Layer::Factory | Layer::Project)
            && requirement
                .confirmation
                .is_some_and(|confirmation| matches!(confirmation, Confirmation::FactoryConfirmed | Confirmation::ProductionVerified))
    })
}

/// Technical design §4.5: the rule layer's status plus the data layer's confirmation decide one
/// state, and only a confirmed number can fail a rule finding.
fn map_state(finding: &solidcraft_dfm_rules::Finding) -> FindingState {
    // An App finding, or a gate that has no requirement to confirm, decides its own state: a
    // thickness conflict is the model not matching what was declared, not a generic number.
    if from_layer(finding) == FindingFrom::App || finding.requirement.is_none() {
        return rule_state(finding.state);
    }
    match finding.status {
        RuleStatus::Pass if confirms(finding) => FindingState::Passed,
        RuleStatus::Pass => FindingState::Checked,
        RuleStatus::Violation | RuleStatus::NeedsReview if confirms(finding) => FindingState::Failed,
        RuleStatus::Violation | RuleStatus::NeedsReview => FindingState::Advisory,
        // A number that reaches the generic public value is a real check, not a pass. Every other
        // not-verified result is "not covered" with its own reason, never a checked value.
        RuleStatus::NotVerified
            if finding.reason_code.as_deref() == Some("ABOVE_PUBLIC_VALUE") || finding.state == solidcraft_dfm_rules::AppState::Checked =>
        {
            FindingState::Checked
        }
        RuleStatus::NotVerified => FindingState::Unsupported,
        RuleStatus::NotApplicable => FindingState::NotApplicable,
    }
}

fn reason_code(finding: &solidcraft_dfm_rules::Finding, state: FindingState) -> Option<String> {
    finding
        .reason_code
        .clone()
        .or_else(|| finding.error_code.clone())
        .or_else(|| (state == FindingState::Unsupported && finding.status == RuleStatus::NotVerified).then(|| "NOT_VERIFIED".to_string()))
}

fn requirement_view(finding: &solidcraft_dfm_rules::Finding, requirement: &solidcraft_dfm_rules::RequirementRef) -> RequirementView {
    // The rule layer's `value_mm` is a length except for the countersink angle, which is degrees.
    let unit = if finding.rules.iter().any(|rule| rule == "sm.countersink_angle") { MeasureUnit::Degree } else { MeasureUnit::Millimetre };
    RequirementView {
        value: requirement.value_mm,
        unit,
        layer: requirement.layer,
        confirmed: requirement.confirmation,
        source: requirement.source.clone(),
    }
}

fn missing_families(facts: &solidcraft_doc::sheet_dfm::SheetFacts) -> Vec<&'static str> {
    // The App-side measurement has these families; `rule_facts` declares which it does not.
    let rules = rule_facts(facts);
    let mut missing = Vec::new();
    for family in &rules.missing {
        missing.push(match family {
            FactFamily::BendRuns => "bend runs",
            FactFamily::ReliefEnds => "relief ends",
            FactFamily::ReliefSizes => "relief sizes",
            FactFamily::Webs => "webs",
            FactFamily::Countersinks => "countersinks",
            FactFamily::HolePanels => "hole panels",
        });
    }
    missing
}

/// The report's deterministic text: what was checked, what was not, and why this is not a
/// certificate.
pub fn summary(findings: &RunFindings) -> String {
    let mut out = String::new();
    out.push_str("# DFM check\n\n");
    out.push_str(&format!("Run: `{}`\n", findings.run_id));
    out.push_str(&format!("Project: `{}`\n", findings.project_id));
    out.push_str(&format!("Model: `{}`\n", findings.model_version));
    out.push_str(&format!("Configuration digest: `{}`\n\n", findings.config_digest));
    let mut counts = [0_usize; 7];
    for finding in &findings.findings {
        counts[state_index(finding.state)] += 1;
    }
    out.push_str(&format!(
        "Passed: {} | Checked: {} | Advisory: {} | Failed: {} | Unsupported: {} | Not applicable: {} | Not run: {}\n\n",
        counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6]
    ));
    if counts[3] > 0 {
        out.push_str(&format!("Result: check failed: {} finding(s) break a confirmed requirement.\n\n", counts[3]));
    } else if findings.findings.iter().all(|finding| finding.state.is_pass()) && !findings.findings.is_empty() {
        out.push_str("Result: every check passed against its requirement.\n\n");
    } else {
        out.push_str("Result: not a pass. At least one check has no passing, confirmed result.\n\n");
    }
    if !findings.findings.is_empty() {
        out.push_str("## Findings\n\n");
        for finding in &findings.findings {
            out.push_str(&format!("- `{}` `{}` ({})", finding.id, finding.state_code(), finding.body));
            if let Some(measured) = finding.measured {
                out.push_str(&format!(", measured {:.3} {}", measured.value, measured.unit.as_str()));
            }
            if let Some(requirement) = &finding.requirement {
                out.push_str(&format!(
                    ", required {:.3} {} ({})",
                    requirement.value,
                    requirement.unit.as_str(),
                    requirement.confirmed.map_or("unconfirmed", |confirmation| match confirmation {
                        Confirmation::TeamSelfReported => "team reported",
                        Confirmation::FactoryConfirmed => "factory confirmed",
                        Confirmation::ProductionVerified => "production verified",
                    })
                ));
            }
            if let Some(code) = &finding.code {
                out.push_str(&format!(", code `{code}`"));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str("## Not checked\n\n");
    for line in findings.not_checked.lines() {
        out.push_str(&format!("- {line}\n"));
    }
    out
}

impl Finding {
    fn state_code(&self) -> &'static str {
        match self.state {
            FindingState::Passed => "passed",
            FindingState::Checked => "checked",
            FindingState::Advisory => "advisory",
            FindingState::Failed => "failed",
            FindingState::Unsupported => "unsupported",
            FindingState::NotApplicable => "not_applicable",
            FindingState::NotRun => "not_run",
        }
    }
}

impl MeasureUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            MeasureUnit::Millimetre => "mm",
            MeasureUnit::SquareMillimetre => "mm2",
            MeasureUnit::Degree => "deg",
        }
    }
}

fn state_index(state: FindingState) -> usize {
    match state {
        FindingState::Passed => 0,
        FindingState::Checked => 1,
        FindingState::Advisory => 2,
        FindingState::Failed => 3,
        FindingState::Unsupported => 4,
        FindingState::NotApplicable => 5,
        FindingState::NotRun => 6,
    }
}

/// The exact bytes `findings.json` holds for these findings.
pub fn to_bytes(findings: &RunFindings) -> Result<Vec<u8>, String> {
    serde_json::to_vec(findings).map_err(|error| error.to_string())
}
