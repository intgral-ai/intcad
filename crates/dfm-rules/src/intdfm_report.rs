//! The INTdfm-shaped report: the Rust port of INTdfm's `report.py` (`report_version` 1.8).
//!
//! **This is report parity, not rule parity.** The module never runs a rule, measures geometry or
//! reads a file. It takes the findings a rule layer produced (before the report merges them) and
//! the metadata `build_report` receives, and turns them into the report INTdfm writes: it merges,
//! rounds, stamps content-derived ids, assigns `F1…` labels, sorts, counts, sets `clean`, and
//! writes the canonical JSON bytes. Which findings exist, their wording and their measured values
//! stay the rule layer's job; a report that is byte-equal here does not prove the rules agree.
//!
//! The input is the *pre-merge* finding list, because two report fields cannot be reconstructed
//! from a merged one:
//!
//! * `header` names the rules that compared the part with a public value, and `_merged` drops a
//!   finding's own `requirement` when a group's requirements differ (`report.py` `_shared`), so
//!   computing it after the merge loses rules (6 of the 13 committed recordings);
//! * `_stamp_ids` breaks a same-id tie with the canonical JSON of the finding before labels are
//!   assigned.
//!
//! What this module deliberately does not do (the next milestones):
//!
//! * `rules.py`: the other 15 rules, and `is_flat`/`bends_of`/`part_of` from measured facts;
//! * `recheck.py`: the types are here so the report can carry `recheck`, `compare` is not;
//! * the App report (`crate::Report`) keeps its own shape and `AppState` mapping.
//!
//! # Faithful details worth knowing
//!
//! * Rounding is CPython's `round(value, places)`: correctly rounded decimal rounding of the exact
//!   binary value, ties to even, then `+ 0.0` so `-0.0` becomes `0.0`. Rust's `f64::round` is
//!   half-away-from-zero and is not used.
//! * Canonical JSON is Python's `json.dumps(..., sort_keys=True, ensure_ascii=False, indent=2,
//!   allow_nan=False)` plus a newline. Floats use Python's `repr` spelling (`1e-06`, `1e+16`) and
//!   keys are sorted; `serde_json::to_string_pretty` does neither, so [`canonical_json`] writes the
//!   text itself. A non-finite number is an error, never `null` or `NaN`.
//! * `RULE_ORDER` is the catalog and also the report order; `sm.bend_radius_consistent` is last.
//! * Ordered `needs` are part of the merge key, so they must come from the rule layer unchanged.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{DfmError, Layer, NOT_CHECKED, RULE_ORDER};

/// The report contract this module reproduces.
pub const REPORT_VERSION: &str = "1.8";

/// The label a report gives the first finding, and the prefix of the rest.
const LABEL_PREFIX: &str = "F";

/// `mode` is the report's own copy of how a rule may act; a gate's pass or violation carries
/// `gate`, a rule's review carries `warn`, and a violation carries `fail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Gate,
    Warn,
    Fail,
}

/// The five outcomes a finding can have. The App's six [`crate::AppState`]s are a later mapping of
/// this plus the requirement's layer, not a replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Violation,
    NeedsReview,
    NotVerified,
    NotApplicable,
}

/// A number with the unit it was measured in. The unit differs per rule (`mm`, `mm3`, `bends`,
/// `solids`), so it cannot be a bare `f64`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
}

/// A point in INTdfm's folded sheet frame, in millimetres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Where a finding is. A whole-part finding has text and no point; a located one has both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub text: String,
    pub mm: Option<Point>,
}

/// What a finding is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub kind: String,
    pub description: String,
}

/// The number a finding was compared with, and where that number came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Requirement {
    pub text: String,
    pub value: Option<Quantity>,
    pub source: String,
    pub layer: Layer,
}

/// One result in a report. The field order here is irrelevant to the output: the canonical writer
/// sorts keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// `""` until [`build_report`] stamps it.
    pub finding_id: String,
    /// `""` until [`build_report`] stamps it.
    pub label: String,
    pub status: Status,
    pub rules: Vec<String>,
    pub mode: Option<Mode>,
    pub error_code: Option<String>,
    pub reason_code: Option<String>,
    pub message: String,
    pub feature: Feature,
    pub location: Location,
    pub measured: Option<Quantity>,
    pub requirement: Option<Requirement>,
    pub needs: Vec<String>,
}

/// How many findings have each status, so they add up to the number of findings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub pass: usize,
    pub violation: usize,
    pub needs_review: usize,
    pub not_verified: usize,
    pub not_applicable: usize,
}

impl Counts {
    fn add(&mut self, status: Status) {
        self.add_n(status, 1);
    }

    fn add_n(&mut self, status: Status, count: usize) {
        let slot = match status {
            Status::Pass => &mut self.pass,
            Status::Violation => &mut self.violation,
            Status::NeedsReview => &mut self.needs_review,
            Status::NotVerified => &mut self.not_verified,
            Status::NotApplicable => &mut self.not_applicable,
        };
        *slot += count;
    }
}

/// What the caller declared. `file` and `shop` are leaf names, as the report echoes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Declared {
    pub file: String,
    pub process: Option<String>,
    pub material: Option<String>,
    pub thickness_mm: Option<f64>,
    pub shop: Option<String>,
}

/// A dimension the designer says matters: hole centre to hole centre, plus or minus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Critical {
    pub name: String,
    pub from_mm: Point,
    pub to_mm: Point,
    pub tolerance_mm: f64,
}

/// The selected data's identity in a report. `hash` is the data file's digest; the layer is not
/// part of it, so `header` takes the shop's layer separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: Option<String>,
    pub version: Option<String>,
    pub hash: Option<String>,
}

/// What produced the report and from what. No timestamp and no absolute path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Traceability {
    pub core_version: String,
    pub critical: Vec<Critical>,
    pub declared: Declared,
    pub file_sha256: String,
    pub ocp_version: String,
    pub occt_package: String,
    pub profile: Profile,
    pub project_file: Option<String>,
    pub reader_log: Vec<String>,
}

/// One bend the recogniser found. `flanges_mm` is each flat's longest straight run from the
/// tangent, shorter first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bend {
    pub number: usize,
    pub location: Location,
    /// 0 when the inside corner is sharp.
    pub inside_radius_mm: f64,
    /// How far the sheet turns.
    pub angle_deg: f64,
    pub length_mm: f64,
    pub flanges_mm: Vec<f64>,
}

/// A countersink's sizes, when a hole has one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Countersink {
    pub major_diameter_mm: f64,
    pub depth_mm: f64,
    pub angle_deg: f64,
    pub land_mm: f64,
}

/// One round hole, numbered the way findings name it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hole {
    pub number: usize,
    pub diameter_mm: f64,
    pub mm: Point,
    pub countersink: Option<Countersink>,
}

/// Parallel bends with one flat between them. `bends` is lower first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Web {
    pub bends: Vec<usize>,
    /// Both flanges rise on one side, so a U.
    pub same_side: bool,
}

/// What a later check compares with. It says nothing about whether the part is any good.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartFacts {
    /// Every round hole found, `None` when the hole step failed.
    pub holes: Option<Vec<Hole>>,
    /// Openings in the skins that are not round holes.
    pub other_openings: usize,
    pub webs: Vec<Web>,
    /// The box round the part along the model's axes, longest first.
    pub size_mm: Option<Vec<f64>>,
    pub volume_mm3: f64,
}

/// What became of one finding. `fixed` never means a pass, and `cant_match` is never a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkWord {
    Fixed,
    StillThere,
    New,
    CantMatch,
}

/// One mark, for one finding of the earlier report or this one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub mark: MarkWord,
    /// Its label in the earlier report; `None` on a new finding.
    pub earlier: Option<String>,
    /// Its label in this report; `None` when it is not here.
    pub later: Option<String>,
    pub rules: Vec<String>,
    pub feature: String,
    pub place: String,
    /// How far its feature moved, `None` when no feature was followed.
    pub moved_mm: Option<f64>,
    pub why: String,
}

/// One thing the removal guard raised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardFlag {
    pub code: String,
    pub earlier: Option<String>,
    pub later: Option<String>,
    pub message: String,
}

/// What could not be compared, so its silence says nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Guard {
    pub flags: Vec<GuardFlag>,
    pub not_compared: Vec<String>,
}

/// How many marks of each word.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkCounts {
    pub fixed: usize,
    pub still_there: usize,
    pub new: usize,
    pub cant_match: usize,
}

/// What became of each finding since an earlier report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recheck {
    pub earlier_snapshot_id: Option<String>,
    pub earlier_file: String,
    pub counts: MarkCounts,
    pub marks: Vec<Mark>,
    pub guard: Guard,
}

/// The report INTdfm writes, field for field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub report_version: String,
    pub clean: bool,
    pub counts: Counts,
    pub rule_results: Counts,
    pub not_checked: String,
    pub header: Option<String>,
    pub bends: Vec<Bend>,
    pub part: Option<PartFacts>,
    pub findings: Vec<Finding>,
    pub recheck: Option<Recheck>,
    pub traceability: Traceability,
    pub snapshot_id: Option<String>,
}

/// The metadata [`build_report`] receives, one field per argument of INTdfm's function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportMetadata {
    pub declared: Declared,
    pub file_sha256: String,
    pub core_version: String,
    pub ocp_version: String,
    pub occt_package: String,
    pub profile: Profile,
    pub snapshot_id: Option<String>,
    pub project_file: Option<String>,
    pub reader_log: Vec<String>,
    /// `None` when no rule compared the part with a public value. Compute it with [`header`]
    /// from the pre-merge findings, never from the merged report.
    pub header: Option<String>,
    pub bends: Vec<Bend>,
    pub part: Option<PartFacts>,
    pub critical: Vec<Critical>,
}

// ---------------------------------------------------------------------------
// The one function that builds a report.

/// Merge, round, stamp IDs, sort, count and set `clean`. Raises on a broken rule list, exactly as
/// INTdfm's `build_report` does.
pub fn build_report(findings: Vec<Finding>, metadata: ReportMetadata) -> Result<Report, DfmError> {
    check_findings(&findings)?;
    let mut merged: Vec<Finding> = merge(findings).into_iter().map(rounded_copy).collect::<Result<_, _>>()?;
    stamp_ids(&mut merged)?;
    merged
        .sort_by(|left, right| rule_position_of_first(left).cmp(&rule_position_of_first(right)).then_with(|| left.finding_id.cmp(&right.finding_id)));

    let mut counts = Counts::default();
    let mut rule_results = Counts::default();
    for (index, finding) in merged.iter_mut().enumerate() {
        let _ = write!(finding.label, "{LABEL_PREFIX}{}", index + 1);
        counts.add(finding.status);
        rule_results.add_n(finding.status, finding.rules.len());
    }
    // A rule that did not run must never read as clean, so one real pass is required too.
    let clean = counts.violation == 0 && counts.needs_review == 0 && counts.not_verified == 0 && counts.pass >= 1;

    let mut bends = Vec::with_capacity(metadata.bends.len());
    for bend in &metadata.bends {
        bends.push(rounded_bend(bend)?);
    }
    let mut critical = Vec::with_capacity(metadata.critical.len());
    for item in &metadata.critical {
        critical.push(rounded_critical(item)?);
    }

    let report = Report {
        report_version: REPORT_VERSION.to_string(),
        clean,
        counts,
        rule_results,
        not_checked: NOT_CHECKED.to_string(),
        header: metadata.header,
        bends,
        part: metadata.part,
        findings: merged,
        recheck: None,
        traceability: Traceability {
            core_version: metadata.core_version,
            critical,
            declared: metadata.declared,
            file_sha256: metadata.file_sha256,
            ocp_version: metadata.ocp_version,
            occt_package: metadata.occt_package,
            profile: metadata.profile,
            project_file: metadata.project_file,
            reader_log: metadata.reader_log,
        },
        snapshot_id: metadata.snapshot_id,
    };
    ensure_finite(&report)?;
    Ok(report)
}

/// The header, from the findings *before* the merge: which rules compared the part with a public
/// value. `None` when a shop's numbers decided everything.
pub fn header(findings: &[Finding], shop_layer: Option<Layer>) -> Option<String> {
    let public: Vec<&str> = RULE_ORDER
        .iter()
        .copied()
        .filter(|rule| {
            findings.iter().any(|finding| {
                finding.rules.iter().any(|name| name == rule)
                    && finding.requirement.as_ref().is_some_and(|requirement| never_decides(requirement.layer))
            })
        })
        .collect();
    let total = RULE_ORDER.iter().filter(|rule| !is_gate(rule)).count();
    let ran = format!("{} of {total} rules ran on public values", public.len());
    let names = if public.is_empty() { String::new() } else { format!(": {}", public.join(", ")) };
    let no_factory = shop_layer.is_none_or(never_decides);
    if no_factory {
        Some(format!("no factory: {ran}{names}. None can pass."))
    } else if public.is_empty() {
        None
    } else {
        Some(format!("{ran}{names}. Those can't pass."))
    }
}

// ---------------------------------------------------------------------------
// Merge.

/// One `not_verified` finding per cause, at the whole part. Other statuses pass through.
fn merge(findings: Vec<Finding>) -> Vec<Finding> {
    let mut causes: Vec<((Option<String>, Vec<String>), Vec<Finding>)> = Vec::new();
    let mut merged: Vec<Finding> = Vec::new();
    for finding in findings {
        if finding.status == Status::NotVerified {
            let key = (finding.reason_code.clone(), finding.needs.clone());
            match causes.iter_mut().find(|(known, _)| *known == key) {
                Some((_, group)) => group.push(finding),
                None => causes.push((key, vec![finding])),
            }
        } else {
            merged.push(finding);
        }
    }
    for ((reason_code, needs), group) in causes {
        let Some(first) = group.first() else { continue };
        let message = group.iter().map(|finding| finding.message.clone()).min().unwrap_or_else(|| first.message.clone());
        let mut rules: Vec<String> = Vec::new();
        for finding in &group {
            for rule in &finding.rules {
                if !rules.contains(rule) {
                    rules.push(rule.clone());
                }
            }
        }
        rules.sort_by_key(|rule| rule_position(rule));
        merged.push(Finding {
            finding_id: String::new(),
            label: String::new(),
            status: Status::NotVerified,
            rules,
            mode: None,
            error_code: None,
            reason_code,
            message,
            feature: whole_part_feature(),
            location: whole_part_location(),
            measured: shared(group.iter().map(|finding| finding.measured.clone()).collect()),
            requirement: shared(group.iter().map(|finding| finding.requirement.clone()).collect()),
            needs,
        });
    }
    merged
}

/// The value every finding of a cause carries, or `None`. Always kept for a group of one.
fn shared<T: PartialEq + Clone>(values: Vec<Option<T>>) -> Option<T> {
    let first = values.first()?;
    let Some(first) = first else { return None };
    if values.iter().all(|value| value.as_ref() == Some(first)) { Some(first.clone()) } else { None }
}

fn whole_part_feature() -> Feature {
    Feature { kind: "part".to_string(), description: "whole part".to_string() }
}

fn whole_part_location() -> Location {
    Location { text: "whole part".to_string(), mm: None }
}

// ---------------------------------------------------------------------------
// Rounding.

/// CPython's `round(value, places) + 0.0`.
///
/// `format!` rounds the exact binary value, ties to even, as CPython does; `+ 0.0` turns `-0.0`
/// into `0.0` and an integer into a float. A non-finite value has no report form and is refused.
pub fn python_round(value: f64, places: usize) -> Result<f64, DfmError> {
    if !value.is_finite() {
        return Err(not_finite("a report number"));
    }
    let text = format!("{value:.places$}");
    let rounded = text.parse::<f64>().map_err(|_| DfmError::Invalid(format!("report: cannot round {text}")))?;
    Ok(rounded + 0.0)
}

fn rounded_number(value: f64, what: &'static str) -> Result<f64, DfmError> {
    if !value.is_finite() {
        return Err(not_finite(what));
    }
    let text = format!("{value:.4}");
    let rounded = text.parse::<f64>().map_err(|_| DfmError::Invalid(format!("report: cannot round {text}")))?;
    Ok(rounded + 0.0)
}

fn rounded_point(point: &Point) -> Result<Point, DfmError> {
    Ok(Point { x: rounded_number(point.x, "a point's x")?, y: rounded_number(point.y, "a point's y")?, z: rounded_number(point.z, "a point's z")? })
}

/// A private copy, so a report never shares a value with the caller.
fn rounded_copy(mut finding: Finding) -> Result<Finding, DfmError> {
    if let Some(mm) = finding.location.mm.as_mut() {
        *mm = rounded_point(mm)?;
    }
    if let Some(measured) = finding.measured.as_mut() {
        measured.value = rounded_number(measured.value, "a measured value")?;
    }
    if let Some(requirement) = finding.requirement.as_mut()
        && let Some(value) = requirement.value.as_mut()
    {
        value.value = rounded_number(value.value, "a requirement value")?;
    }
    Ok(finding)
}

fn rounded_critical(item: &Critical) -> Result<Critical, DfmError> {
    Ok(Critical {
        name: item.name.clone(),
        from_mm: rounded_point(&item.from_mm)?,
        to_mm: rounded_point(&item.to_mm)?,
        tolerance_mm: rounded_number(item.tolerance_mm, "a critical tolerance")?,
    })
}

fn rounded_bend(item: &Bend) -> Result<Bend, DfmError> {
    let mm = match &item.location.mm {
        None => None,
        Some(point) => Some(rounded_point(point)?),
    };
    let mut flanges = Vec::with_capacity(item.flanges_mm.len());
    for value in &item.flanges_mm {
        flanges.push(rounded_number(*value, "a flange run")?);
    }
    Ok(Bend {
        number: item.number,
        location: Location { text: item.location.text.clone(), mm },
        inside_radius_mm: rounded_number(item.inside_radius_mm, "a bend radius")?,
        angle_deg: rounded_number(item.angle_deg, "a bend angle")?,
        length_mm: rounded_number(item.length_mm, "a bend length")?,
        flanges_mm: flanges,
    })
}

// ---------------------------------------------------------------------------
// IDs, labels and order.

/// The plain ID: rules, feature kind and position only, so a finding keeps its ID when its status
/// changes.
fn base_id(finding: &Finding) -> Result<String, DfmError> {
    let position = match &finding.location.mm {
        None => "-".to_string(),
        Some(point) => format!("{},{},{}", id_number(point.x)?, id_number(point.y)?, id_number(point.z)?),
    };
    let mut rules: Vec<&str> = finding.rules.iter().map(String::as_str).collect();
    rules.sort_unstable();
    let identity = format!("{}|{}|{position}", rules.join(","), finding.feature.kind);
    let digest = Sha256::digest(identity.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex.truncate(16);
    Ok(format!("f_{hex}"))
}

/// After rounding, so `-0.0001` cannot hash differently from `0.0`; `+ 0.0` turns `-0.0` into
/// `0.0`, and three decimals are always shown.
fn id_number(value: f64) -> Result<String, DfmError> {
    let rounded = python_round(value, 3)?;
    Ok(format!("{rounded:.3}"))
}

/// The plain ID, then `-2`, `-3` for findings that share it, in an order only the inputs decide.
fn stamp_ids(findings: &mut [Finding]) -> Result<(), DfmError> {
    for finding in findings.iter_mut() {
        finding.finding_id.clear();
        finding.label.clear();
    }
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, finding) in findings.iter().enumerate() {
        groups.entry(base_id(finding)?).or_default().push(index);
    }
    for (base, members) in groups {
        let mut keyed: Vec<(usize, String, usize)> = Vec::with_capacity(members.len());
        for index in members {
            keyed.push((rule_position_of_first(&findings[index]), dumps(&findings[index])?, index));
        }
        keyed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        for (number, (_, _, index)) in keyed.into_iter().enumerate() {
            let suffix = number + 1;
            findings[index].finding_id = if number == 0 { base.clone() } else { format!("{base}-{suffix}") };
        }
    }
    Ok(())
}

fn rule_position(rule: &str) -> usize {
    RULE_ORDER.iter().position(|known| *known == rule).unwrap_or(usize::MAX)
}

fn rule_position_of_first(finding: &Finding) -> usize {
    finding.rules.first().map_or(usize::MAX, |rule| rule_position(rule))
}

fn is_gate(rule: &str) -> bool {
    matches!(rule, "sm.gate_input_solid" | "sm.gate_sheet_thickness" | "sm.gate_bends_found" | "sm.gate_profile_coverage")
}

fn never_decides(layer: Layer) -> bool {
    matches!(layer, Layer::Generic | Layer::Development)
}

// ---------------------------------------------------------------------------
// The catalog check.

/// INTdfm's `_check`: every rule in the catalog must be spoken for, each finding needs rules, and
/// a status must carry what it claims. A broken list is refused, never repaired.
pub fn check_findings(findings: &[Finding]) -> Result<(), DfmError> {
    let mut given: Vec<&str> = Vec::new();
    for finding in findings {
        for rule in &finding.rules {
            if !given.contains(&rule.as_str()) {
                given.push(rule.as_str());
            }
        }
    }
    let missing: Vec<String> = RULE_ORDER.iter().filter(|rule| !given.contains(*rule)).map(|rule| (*rule).to_string()).collect();
    let mut unknown: Vec<String> = given.iter().filter(|rule| !RULE_ORDER.contains(rule)).map(|rule| (*rule).to_string()).collect();
    unknown.sort();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(DfmError::Invalid(format!("report: rule results don't match the catalog: missing={missing:?}, unknown={unknown:?}")));
    }
    for finding in findings {
        let rules = &finding.rules;
        if rules.is_empty() {
            return Err(DfmError::Invalid("report: a finding needs at least one rule".to_string()));
        }
        let mut unique = rules.clone();
        unique.sort();
        unique.dedup();
        if unique.len() != rules.len() {
            return Err(DfmError::Invalid(format!("report: a finding needs each rule named once: {rules:?}")));
        }
        if finding.status == Status::NotVerified && (finding.reason_code.is_none() || finding.needs.is_empty()) {
            return Err(DfmError::Invalid(format!("report: not_verified needs a reason_code and needs: {rules:?}")));
        }
        if finding.status == Status::Pass {
            check_pass(finding)?;
        }
        if finding.status == Status::Violation {
            check_violation(finding)?;
        }
    }
    Ok(())
}

fn check_pass(finding: &Finding) -> Result<(), DfmError> {
    let rules = &finding.rules;
    match &finding.requirement {
        // Only a gate has no shop number to compare: its pass says what it checked, no more.
        None => {
            let all_gates = rules.iter().all(|rule| is_gate(rule));
            if !all_gates || finding.mode != Some(Mode::Gate) {
                return Err(DfmError::Invalid(format!("report: a pass needs a requirement: {rules:?}")));
            }
        }
        Some(requirement) if never_decides(requirement.layer) => {
            let layer = requirement.layer;
            return Err(DfmError::Invalid(format!("report: a {layer:?} requirement can never give pass: {rules:?}")));
        }
        Some(_) => {}
    }
    Ok(())
}

fn check_violation(finding: &Finding) -> Result<(), DfmError> {
    let rules = &finding.rules;
    if finding.mode.is_none() {
        return Err(DfmError::Invalid(format!("report: a violation needs a mode: {rules:?}")));
    }
    match &finding.requirement {
        // Only the thickness gate compares with something declared rather than a shop's number.
        None => {
            if rules.len() != 1 || rules[0] != crate::THICKNESS_GATE {
                return Err(DfmError::Invalid(format!("report: a violation needs a requirement: {rules:?}")));
            }
        }
        Some(requirement) if never_decides(requirement.layer) => {
            let layer = requirement.layer;
            return Err(DfmError::Invalid(format!("report: a {layer:?} requirement can never give violation: {rules:?}")));
        }
        Some(_) => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Finiteness: a number with no JSON form must not reach the writer.

/// Every number in a report must be finite (`json.dumps(..., allow_nan=False)`).
pub fn ensure_finite(report: &Report) -> Result<(), DfmError> {
    for finding in &report.findings {
        finite_option(finding.location.mm.as_ref().map(|point| point.x), "a finding location")?;
        finite_option(finding.location.mm.as_ref().map(|point| point.y), "a finding location")?;
        finite_option(finding.location.mm.as_ref().map(|point| point.z), "a finding location")?;
        finite_option(finding.measured.as_ref().map(|quantity| quantity.value), "a measured value")?;
        if let Some(requirement) = &finding.requirement {
            finite_option(requirement.value.as_ref().map(|quantity| quantity.value), "a requirement value")?;
        }
    }
    for bend in &report.bends {
        finite_option(bend.location.mm.as_ref().map(|point| point.x), "a bend location")?;
        finite_option(bend.location.mm.as_ref().map(|point| point.y), "a bend location")?;
        finite_option(bend.location.mm.as_ref().map(|point| point.z), "a bend location")?;
        finite(bend.inside_radius_mm, "a bend radius")?;
        finite(bend.angle_deg, "a bend angle")?;
        finite(bend.length_mm, "a bend length")?;
        for value in &bend.flanges_mm {
            finite(*value, "a flange run")?;
        }
    }
    if let Some(part) = &report.part {
        finite(part.volume_mm3, "a part volume")?;
        for value in part.size_mm.iter().flatten() {
            finite(*value, "a part size")?;
        }
        for hole in part.holes.iter().flatten() {
            finite(hole.diameter_mm, "a hole diameter")?;
            finite(hole.mm.x, "a hole position")?;
            finite(hole.mm.y, "a hole position")?;
            finite(hole.mm.z, "a hole position")?;
            if let Some(sink) = &hole.countersink {
                finite(sink.major_diameter_mm, "a countersink size")?;
                finite(sink.depth_mm, "a countersink size")?;
                finite(sink.angle_deg, "a countersink size")?;
                finite(sink.land_mm, "a countersink size")?;
            }
        }
    }
    for item in &report.traceability.critical {
        finite(item.from_mm.x, "a critical from")?;
        finite(item.from_mm.y, "a critical from")?;
        finite(item.from_mm.z, "a critical from")?;
        finite(item.to_mm.x, "a critical to")?;
        finite(item.to_mm.y, "a critical to")?;
        finite(item.to_mm.z, "a critical to")?;
        finite(item.tolerance_mm, "a critical tolerance")?;
    }
    if let Some(declared) = report.traceability.declared.thickness_mm {
        finite(declared, "a declared thickness")?;
    }
    if let Some(recheck) = &report.recheck {
        for mark in &recheck.marks {
            finite_option(mark.moved_mm, "a moved distance")?;
        }
    }
    Ok(())
}

fn finite(value: f64, what: &str) -> Result<(), DfmError> {
    if value.is_finite() { Ok(()) } else { Err(DfmError::Invalid(format!("report: {what} must be finite"))) }
}

fn finite_option(value: Option<f64>, what: &str) -> Result<(), DfmError> {
    match value {
        Some(value) => finite(value, what),
        None => Ok(()),
    }
}

fn not_finite(what: &str) -> DfmError {
    DfmError::Invalid(format!("report: {what} must be finite"))
}

// ---------------------------------------------------------------------------
// Canonical JSON: Python's json.dumps(sort_keys=True, ensure_ascii=False, indent=2,
// allow_nan=False) plus one newline.

/// The canonical JSON a report is stored and compared as, byte-for-byte as CPython writes it.
pub fn canonical_json(report: &Report) -> Result<String, DfmError> {
    ensure_finite(report)?;
    let mut text = dumps(report)?;
    text.push('\n');
    Ok(text)
}

/// The same writer without the trailing newline, as `report._dumps` uses it for tie-breaks.
fn dumps<T: Serialize>(value: &T) -> Result<String, DfmError> {
    let value = serde_json::to_value(value).map_err(|error| DfmError::Invalid(format!("report JSON: {error}")))?;
    let mut text = String::new();
    write_value(&value, &mut text, 0)?;
    Ok(text)
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_value(value: &serde_json::Value, out: &mut String, depth: usize) -> Result<(), DfmError> {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        serde_json::Value::Number(number) => out.push_str(&number_text(number)?),
        serde_json::Value::String(text) => write_string(text, out),
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return Ok(());
            }
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, depth + 1);
                write_value(item, out, depth + 1)?;
            }
            out.push('\n');
            push_indent(out, depth);
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push_str("{\n");
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, depth + 1);
                write_string(key, out);
                out.push_str(": ");
                write_value(&map[*key], out, depth + 1)?;
            }
            out.push('\n');
            push_indent(out, depth);
            out.push('}');
        }
    }
    Ok(())
}

fn number_text(number: &serde_json::Number) -> Result<String, DfmError> {
    if number.is_f64() {
        let value = number.as_f64().ok_or_else(|| DfmError::Invalid("report JSON: an unreadable number".to_string()))?;
        return python_float_text(value);
    }
    if let Some(value) = number.as_u64() {
        return Ok(value.to_string());
    }
    if let Some(value) = number.as_i64() {
        return Ok(value.to_string());
    }
    Err(DfmError::Invalid("report JSON: an unreadable number".to_string()))
}

/// CPython's `float.__repr__`, which is what `json.dumps` writes: the shortest round-trip digits,
/// a decimal point in fixed form, and `e±NN` outside `-4 <= exponent < 16`.
pub fn python_float_text(value: f64) -> Result<String, DfmError> {
    if !value.is_finite() {
        return Err(not_finite("a report number"));
    }
    if value == 0.0 {
        return Ok(if value.is_sign_negative() { "-0.0".to_string() } else { "0.0".to_string() });
    }
    let negative = value < 0.0;
    let text = format!("{:e}", value.abs());
    let (mantissa, exponent) = text.split_once('e').ok_or_else(|| DfmError::Invalid(format!("report JSON: cannot read {text} as a decimal")))?;
    let exponent: i32 = exponent.parse().map_err(|_| DfmError::Invalid(format!("report JSON: cannot read {text}'s exponent")))?;
    let digits: String = mantissa.chars().filter(|character| *character != '.').collect();
    let first = digits.chars().next().ok_or_else(|| DfmError::Invalid(format!("report JSON: {text} has no digits")))?;
    let body = if (-4..16).contains(&exponent) {
        if exponent >= 0 {
            let point = exponent as usize + 1;
            if digits.len() <= point {
                let padding = "0".repeat(point - digits.len());
                format!("{digits}{padding}.0")
            } else {
                format!("{}.{}", &digits[..point], &digits[point..])
            }
        } else {
            let padding = "0".repeat((-exponent - 1) as usize);
            format!("0.{padding}{digits}")
        }
    } else {
        let mut number = String::new();
        number.push(first);
        if digits.len() > 1 {
            number.push('.');
            number.push_str(&digits[1..]);
        }
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{number}e{sign}{:02}", exponent.abs())
    };
    Ok(if negative { format!("-{body}") } else { body })
}

fn write_string(value: &str, out: &mut String) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            character if (character as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", character as u32);
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64, z: f64) -> Point {
        Point { x, y, z }
    }

    #[test]
    fn python_round_matches_both_halves_of_even_ties() {
        // From CPython: round(0.15625, 4) == 0.1562, round(5e-05, 4) == 0.0001.
        assert_eq!(python_round(0.15625, 4).expect("finite"), 0.1562);
        assert_eq!(python_round(0.00005, 4).expect("finite"), 0.0001);
        assert_eq!(python_round(-0.0001, 3).expect("finite"), 0.0);
        assert!(!python_round(-0.0001, 3).expect("finite").is_sign_negative());
        assert_eq!(python_round(1.3180194846605358, 4).expect("finite"), 1.318);
        assert_eq!(python_round(1.3180194846605358, 3).expect("finite"), 1.318);
        assert_eq!(python_round(2835.290358249673, 4).expect("finite"), 2835.2904);
        assert!(python_round(f64::NAN, 4).is_err());
        assert!(python_round(f64::INFINITY, 4).is_err());
    }

    #[test]
    fn float_text_is_python_repr() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.5, "1.5"),
            (90.0, "90.0"),
            (100.0, "100.0"),
            (90.5, "90.5"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (0.000001, "1e-06"),
            (1e-7, "1e-07"),
            (1234567.0, "1234567.0"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (1.2345678e16, "1.2345678e+16"),
            (1e20, "1e+20"),
            (2835.290358249673, "2835.290358249673"),
            (1.3180194846605358, "1.3180194846605358"),
            (-3.1819805153394642, "-3.1819805153394642"),
            (0.30000000000000004, "0.30000000000000004"),
            (1.2345678901234568e-5, "1.2345678901234568e-05"),
        ];
        for (value, expected) in cases {
            assert_eq!(python_float_text(*value).expect("finite"), *expected, "{value}");
        }
        assert!(python_float_text(f64::NAN).is_err());
    }

    #[test]
    fn the_writer_sorts_keys_indents_and_ends_with_a_newline() {
        let mut report = minimal_report();
        report.findings[0].message = "quote \" slash \\ newline \n tab \t control \u{1}".to_string();
        let text = canonical_json(&report).expect("json");
        assert!(text.ends_with("}\n"));
        assert!(text.starts_with("{\n  \"bends\""));
        assert!(text.contains("\\\" slash \\\\ newline \\n tab \\t control \\u0001"));
        assert!(!text.contains("NaN"));
    }

    #[test]
    fn a_non_finite_number_is_refused_not_written_as_null() {
        let mut report = minimal_report();
        report.part = Some(PartFacts { holes: None, other_openings: 0, webs: Vec::new(), size_mm: None, volume_mm3: f64::NAN });
        let error = canonical_json(&report).expect_err("NaN has no JSON form");
        assert!(error.to_string().contains("finite"), "{error}");
    }

    fn minimal_report() -> Report {
        Report {
            report_version: REPORT_VERSION.to_string(),
            clean: false,
            counts: Counts::default(),
            rule_results: Counts::default(),
            not_checked: NOT_CHECKED.to_string(),
            header: None,
            bends: Vec::new(),
            part: None,
            findings: vec![Finding {
                finding_id: "f_0000000000000000".to_string(),
                label: "F1".to_string(),
                status: Status::NotApplicable,
                rules: vec!["sm.hole_spacing".to_string()],
                mode: None,
                error_code: None,
                reason_code: None,
                message: String::new(),
                feature: whole_part_feature(),
                location: whole_part_location(),
                measured: None,
                requirement: None,
                needs: Vec::new(),
            }],
            recheck: None,
            traceability: Traceability {
                core_version: "0.4.0".to_string(),
                critical: Vec::new(),
                declared: Declared { file: "part.step".to_string(), process: None, material: None, thickness_mm: None, shop: None },
                file_sha256: String::new(),
                ocp_version: String::new(),
                occt_package: String::new(),
                profile: Profile { name: None, version: None, hash: None },
                project_file: None,
                reader_log: Vec::new(),
            },
            snapshot_id: None,
        }
    }

    #[test]
    fn a_point_keeps_its_identity_through_rounding() {
        let rounded = rounded_point(&point(0.000_049, -0.000_049, 1.234_56)).expect("finite");
        assert_eq!(rounded, point(0.0, 0.0, 1.2346));
        assert!(!rounded.x.is_sign_negative());
    }
}
