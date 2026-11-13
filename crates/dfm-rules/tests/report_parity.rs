//! Report parity against the INTdfm recordings.
//!
//! Each fixture is one INTdfm recording's rule-layer input, projected to what `build_report`
//! needs: the **pre-merge** findings, the metadata, and the canonical bytes of the recorded
//! report. This is report parity, not rule parity: the findings are the Python rule layer's, so a
//! difference here is a defect in this crate's report layer, never a rule that disagrees.
//!
//! The oracle is `expected.report` from the recording itself; the `.canonical.json` sibling is
//! CPython's `report.canonical_json` of it, byte for byte. Regenerate the fixtures from the
//! INTdfm checkout rather than editing them (see the crate's migration notes).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use solidcraft_dfm_rules::intdfm_report::{
    self as report, Bend, Critical, Declared, Finding, PartFacts, Profile, REPORT_VERSION, ReportMetadata, Status,
};
use solidcraft_dfm_rules::{Layer, NOT_CHECKED, RULE_ORDER};

#[derive(Deserialize)]
struct Fixture {
    source: Source,
    case: Case,
    input: Input,
    expected: Expected,
}

#[derive(Deserialize)]
struct Source {
    intdfm_commit: String,
    core_source_sha256: String,
    report_version: String,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    kind: String,
    rule: String,
    description: String,
    part: Part,
}

#[derive(Deserialize)]
struct Part {
    name: String,
    sha256: String,
}

#[derive(Deserialize)]
struct Expected {
    header: Option<String>,
    report_version: String,
}

#[derive(Deserialize)]
struct Input {
    findings: Vec<Finding>,
    declared: Declared,
    file_sha256: String,
    core_version: String,
    ocp_version: String,
    occt_package: String,
    profile: Profile,
    snapshot_id: Option<String>,
    project_file: Option<String>,
    reader_log: Vec<String>,
    header: Option<String>,
    bends: Vec<Bend>,
    part: Option<PartFacts>,
    critical: Vec<Critical>,
    shop_layer: Option<Layer>,
}

impl Input {
    fn metadata(&self) -> ReportMetadata {
        ReportMetadata {
            declared: self.declared.clone(),
            file_sha256: self.file_sha256.clone(),
            core_version: self.core_version.clone(),
            ocp_version: self.ocp_version.clone(),
            occt_package: self.occt_package.clone(),
            profile: self.profile.clone(),
            snapshot_id: self.snapshot_id.clone(),
            project_file: self.project_file.clone(),
            reader_log: self.reader_log.clone(),
            header: self.header.clone(),
            bends: self.bends.clone(),
            part: self.part.clone(),
            critical: self.critical.clone(),
        }
    }
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dfm-report/v1")
}

fn fixture(name: &str) -> Fixture {
    let path = fixtures_dir().join(format!("{name}.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn canonical_bytes(name: &str) -> String {
    let path = fixtures_dir().join(format!("{name}.canonical.json"));
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// One line of context when the bytes differ, instead of two multi-kilobyte strings.
fn first_difference(name: &str, actual: &str, expected: &str) -> String {
    let mut message = String::new();
    for (index, (left, right)) in actual.lines().zip(expected.lines()).enumerate() {
        if left != right {
            let _ = write!(message, "{name}: canonical bytes differ at line {}:\n  rust:   {left}\n  python: {right}", index + 1);
            return message;
        }
    }
    format!("{name}: canonical bytes differ: rust {} bytes, python {} bytes", actual.len(), expected.len())
}

fn check_case(name: &str) {
    let fixture = fixture(name);
    assert_eq!(fixture.case.id, name);
    assert_eq!(fixture.expected.report_version, REPORT_VERSION);
    assert_eq!(fixture.source.report_version, REPORT_VERSION);
    assert!(!fixture.source.intdfm_commit.is_empty());
    assert_eq!(fixture.source.core_source_sha256.len(), 64);
    assert!(!fixture.case.description.is_empty());
    assert!(!fixture.case.kind.is_empty());
    assert!(!fixture.case.rule.is_empty());
    assert!(!fixture.case.part.name.starts_with('/'), "a fixture must not carry an absolute path");
    assert_eq!(fixture.case.part.sha256.len(), 64);
    for finding in &fixture.input.findings {
        assert!(finding.finding_id.is_empty(), "{}: a fixture finding is already stamped", fixture.case.id);
        assert!(finding.label.is_empty(), "{}: a fixture finding is already labelled", fixture.case.id);
    }

    // The header must be computed before the merge; after it, a shared requirement can be lost.
    let header = report::header(&fixture.input.findings, fixture.input.shop_layer);
    assert_eq!(header, fixture.expected.header, "{}: header from the pre-merge findings", fixture.case.id);

    let built = report::build_report(fixture.input.findings.clone(), fixture.input.metadata())
        .unwrap_or_else(|error| panic!("{}: build_report: {error}", fixture.case.id));
    assert_eq!(built.header, fixture.expected.header, "{}: report header", fixture.case.id);

    let actual = report::canonical_json(&built).unwrap_or_else(|error| panic!("{}: canonical_json: {error}", fixture.case.id));
    let expected = canonical_bytes(name);
    if actual != expected {
        panic!("{}", first_difference(name, &actual, &expected));
    }
}

macro_rules! parity_cases {
    ($($test:ident => $fixture:literal),* $(,)?) => {
        $(#[test]
        fn $test() {
            check_case($fixture);
        })*

        const FIXTURE_NAMES: &[&str] = &[$($fixture),*];
    };
}

parity_cases! {
    report_parity_hole_min_diameter_above => "hole_min_diameter.above",
    report_parity_hole_min_diameter_below => "hole_min_diameter.below",
    report_parity_hole_min_diameter_boundary => "hole_min_diameter.boundary",
    report_parity_hole_min_diameter_factory_below => "hole_min_diameter.factory.below",
    report_parity_hole_to_bend_below => "hole_to_bend.below",
    report_parity_hole_to_bend_not_applicable => "hole_to_bend.not_applicable",
    report_parity_hole_to_bend_u_holes => "hole_to_bend.u_holes",
    report_parity_hole_to_edge_below => "hole_to_edge.below",
    report_parity_hole_to_edge_opening => "hole_to_edge.opening",
    report_parity_hole_to_edge_plate_holes => "hole_to_edge.plate_holes",
    report_parity_holes_step_failed => "holes_step_failed",
    report_parity_missing_thickness => "missing_thickness",
    report_parity_shop_not_applicable => "shop_not_applicable",
    report_parity_synthetic_id_collision => "synthetic.id-collision",
}

/// A new fixture without a test, or a test without its fixture, fails here.
#[test]
fn report_parity_every_fixture_has_exactly_one_case() {
    let mut found: Vec<String> = fs::read_dir(fixtures_dir())
        .expect("fixtures folder")
        .map(|entry| entry.expect("a directory entry").file_name().into_string().expect("a UTF-8 file name"))
        .filter(|name| name.ends_with(".json") && !name.ends_with(".canonical.json"))
        .map(|name| name.trim_end_matches(".json").to_string())
        .collect();
    found.sort();
    let mut named: Vec<String> = FIXTURE_NAMES.iter().map(|name| (*name).to_string()).collect();
    named.sort();
    assert_eq!(found, named);
}

#[test]
fn report_parity_catalog_is_the_python_catalog_in_report_order() {
    const PYTHON_CATALOG: [&str; 19] = [
        "sm.gate_input_solid",
        "sm.gate_sheet_thickness",
        "sm.gate_bends_found",
        "sm.gate_profile_coverage",
        "sm.inside_radius",
        "sm.min_flange",
        "sm.hole_min_diameter",
        "sm.max_bend_length",
        "sm.hole_to_bend",
        "sm.bend_relief_present",
        "sm.hole_to_edge",
        "sm.min_web_parallel_bends",
        "sm.u_channel_ratio",
        "sm.tolerance_across_bends",
        "sm.countersink_depth",
        "sm.countersink_to_edge",
        "sm.countersink_to_bend",
        "sm.hole_spacing",
        "sm.bend_radius_consistent",
    ];
    assert_eq!(RULE_ORDER, PYTHON_CATALOG.as_slice());
    assert_eq!(RULE_ORDER[RULE_ORDER.len() - 1], "sm.bend_radius_consistent");
    assert_eq!(RULE_ORDER[RULE_ORDER.len() - 2], "sm.hole_spacing");
}

#[test]
fn report_parity_not_checked_line_is_the_python_line() {
    assert_eq!(
        NOT_CHECKED,
        "Tolerances other than declared hole-to-hole dimensions, bend sequence, collisions, grain and \
         cut-outs that reach the outline were not checked."
    );
}

// ---------------------------------------------------------------------------
// Negative cases: the test must be able to fail.

/// Moving a found feature changes its position, so its id and the bytes change.
#[test]
fn report_parity_a_moved_feature_changes_the_id_and_the_bytes() {
    let fixture = fixture("hole_min_diameter.below");
    let original = report::build_report(fixture.input.findings.clone(), fixture.input.metadata()).expect("the recorded report");
    let before = report::canonical_json(&original).expect("json");
    assert_eq!(before, canonical_bytes("hole_min_diameter.below"));

    let mut moved = fixture.input.findings.clone();
    let located = moved.iter_mut().find(|finding| finding.location.mm.is_some()).expect("a located finding");
    let offset = located.location.mm.as_mut().expect("a point");
    offset.x += 0.001;
    let built = report::build_report(moved, fixture.input.metadata()).expect("a report for the moved part");
    let after = report::canonical_json(&built).expect("json");
    assert_ne!(before, after, "a moved feature must change the canonical bytes");

    let original_ids: Vec<&str> = original.findings.iter().map(|finding| finding.finding_id.as_str()).collect();
    let moved_ids: Vec<&str> = built.findings.iter().map(|finding| finding.finding_id.as_str()).collect();
    assert!(original_ids.iter().all(|id| id.starts_with("f_")));
    assert_ne!(original_ids, moved_ids, "the id is derived from the rounded position");
}

/// Two `not_verified` findings with different `needs` are two causes, not one merged cause.
#[test]
fn report_parity_splitting_a_merge_bucket_changes_the_report() {
    let fixture = fixture("missing_thickness");
    let merged = report::build_report(fixture.input.findings.clone(), fixture.input.metadata()).expect("the recorded report");
    let merged_count = merged.findings.len();
    assert_eq!(report::canonical_json(&merged).expect("json"), canonical_bytes("missing_thickness"));

    let mut split = fixture.input.findings.clone();
    let target = split
        .iter_mut()
        .find(|finding| {
            finding.status == Status::NotVerified && finding.rules.len() > 1 && finding.reason_code.as_deref() == Some("THICKNESS_NOT_DECLARED")
        })
        .expect("a blocked finding with a shared cause");
    target.needs.push("A second need, so this is no longer the same cause.".to_string());

    let built = report::build_report(split, fixture.input.metadata()).expect("a report with two causes");
    assert_eq!(built.findings.len(), merged_count + 1, "the split cause is one finding more");
    let expected = canonical_bytes("missing_thickness");
    assert_ne!(report::canonical_json(&built).expect("json"), expected, "a split cause must change the bytes");
}

/// The catalog check refuses a broken rule list, and accepts the one documented exception.
#[test]
fn report_parity_a_broken_rule_list_is_refused() {
    let base = fixture("hole_min_diameter.below");
    assert!(report::check_findings(&base.input.findings).is_ok());

    let mut missing = base.input.findings.clone();
    missing.retain(|finding| !finding.rules.iter().any(|rule| rule == "sm.hole_spacing"));
    let error = report::check_findings(&missing).expect_err("a rule with no result");
    assert!(error.to_string().contains("catalog"), "{error}");

    let mut unknown = base.input.findings.clone();
    unknown[0].rules.push("sm.not_a_rule".to_string());
    assert!(report::check_findings(&unknown).is_err());

    let mut duplicate = base.input.findings.clone();
    let repeated = duplicate[0].rules[0].clone();
    duplicate[0].rules.push(repeated);
    assert!(report::check_findings(&duplicate).is_err());

    let mut empty = base.input.findings.clone();
    empty[0].rules.clear();
    assert!(report::check_findings(&empty).is_err());

    let mut no_needs = base.input.findings.clone();
    let not_verified = no_needs.iter_mut().find(|finding| finding.status == Status::NotVerified).expect("a not_verified finding");
    not_verified.needs.clear();
    assert!(report::check_findings(&no_needs).is_err());

    // A gate's pass needs `gate` as its mode; a pass with a public requirement is never a pass.
    let mut pass = base.input.findings.clone();
    let gate = pass.iter_mut().find(|finding| finding.status == Status::Pass).expect("a pass finding");
    gate.mode = None;
    assert!(report::check_findings(&pass).is_err());

    let mut public = base.input.findings.clone();
    let passing = public.iter_mut().find(|finding| finding.status == Status::Pass).expect("a pass finding");
    passing.requirement =
        Some(report::Requirement { text: "TEST-ONLY.".to_string(), value: None, source: "TEST-ONLY".to_string(), layer: Layer::Generic });
    assert!(report::check_findings(&public).is_err());

    // A violation needs a mode; without a requirement only the thickness gate may violate.
    let mut violation = base.input.findings.clone();
    let thickness = violation.iter_mut().find(|finding| finding.rules == ["sm.gate_sheet_thickness".to_string()]).expect("the thickness gate");
    thickness.status = Status::Violation;
    thickness.mode = None;
    thickness.requirement = None;
    assert!(report::check_findings(&violation).is_err());
    if let Some(thickness) = violation.iter_mut().find(|finding| finding.rules == ["sm.gate_sheet_thickness".to_string()]) {
        thickness.mode = Some(report::Mode::Fail);
    }
    assert!(report::check_findings(&violation).is_ok(), "only the thickness gate may violate without a requirement");
}

/// A number with no JSON form never reaches the writer: it is refused, not written as `null`.
#[test]
fn report_parity_a_non_finite_report_number_is_refused() {
    let fixture = fixture("hole_min_diameter.below");
    let mut findings = fixture.input.findings.clone();
    let measured = findings.iter_mut().find(|finding| finding.measured.is_some()).expect("a measured finding");
    measured.measured.as_mut().expect("a quantity").value = f64::NAN;
    let error = report::build_report(findings, fixture.input.metadata()).expect_err("NaN has no report form");
    assert!(error.to_string().contains("finite"), "{error}");
}

/// The same input gives the same bytes, twice.
#[test]
fn report_parity_the_report_is_deterministic() {
    let fixture = fixture("hole_to_bend.u_holes");
    let first = report::build_report(fixture.input.findings.clone(), fixture.input.metadata()).expect("first");
    let second = report::build_report(fixture.input.findings.clone(), fixture.input.metadata()).expect("second");
    assert_eq!(report::canonical_json(&first).expect("json"), report::canonical_json(&second).expect("json"));
}

/// `intdfm_report` and the App report are two shapes of the same rule results; this pins the parts
/// the App type does not carry.
#[test]
fn report_parity_carries_the_intdfm_fields_the_app_report_does_not() {
    let fixture = fixture("hole_to_bend.u_holes");
    let metadata = fixture.input.metadata();
    let built = report::build_report(fixture.input.findings.clone(), metadata).expect("the recorded report");
    assert_eq!(built.bends.len(), 2);
    assert!(built.findings.iter().all(|finding| !finding.label.is_empty()));
    assert_eq!(built.findings[0].label, "F1");
    assert_eq!(
        built.counts.pass + built.counts.violation + built.counts.needs_review + built.counts.not_verified + built.counts.not_applicable,
        built.findings.len()
    );
    assert_eq!(
        built.rule_results.pass
            + built.rule_results.violation
            + built.rule_results.needs_review
            + built.rule_results.not_verified
            + built.rule_results.not_applicable,
        built.findings.iter().map(|finding| finding.rules.len()).sum::<usize>()
    );
    assert!(built.part.is_some());
    assert!(built.recheck.is_none());
    assert_eq!(built.report_version, REPORT_VERSION);
}
