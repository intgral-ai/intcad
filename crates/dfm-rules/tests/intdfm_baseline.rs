//! The Rust rules against the INTdfm rule-layer recordings.
//!
//! These fixtures are the compact projections of `tests/fixtures/dfm-replay/v1` in the INTdfm
//! checkout at the commit in each file. They hold the facts, declared data and the findings the
//! Python rule layer produced. A difference here is a translation defect, not a tolerance to widen.

use std::fs;
use std::path::Path;

use serde::Deserialize;
use solidcraft_dfm_rules::{AppState, CheckInput, Declared, Facts, Finding, Layer, ModelRef, Profile, Report, RuleStatus, canonical_json, check};

#[derive(Deserialize)]
struct Fixture {
    source: Source,
    case: Case,
    declared: Declared,
    generic: Profile,
    shop: Option<Profile>,
    facts: Facts,
    expected: Expected,
}

#[derive(Deserialize)]
struct Source {
    intdfm_commit: String,
}

#[derive(Deserialize)]
struct Case {
    id: String,
}

#[derive(Deserialize)]
struct Expected {
    findings: Vec<ExpectedFinding>,
}

#[derive(Deserialize)]
struct ExpectedFinding {
    rules: Vec<String>,
    status: RuleStatus,
    error_code: Option<String>,
    reason_code: Option<String>,
    measured: Option<Measured>,
    requirement: Option<ExpectedRequirement>,
}

#[derive(Deserialize)]
struct Measured {
    value: f64,
}

#[derive(Deserialize)]
struct ExpectedRequirement {
    value: Value,
}

#[derive(Deserialize)]
struct Value {
    value: f64,
}

fn fixture(name: &str) -> Fixture {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/intdfm").join(format!("{name}.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn run(name: &str) -> (Fixture, Report) {
    let fixture = fixture(name);
    let input = CheckInput {
        declared: fixture.declared.clone(),
        generic: fixture.generic.clone(),
        shop: fixture.shop.clone(),
        overrides: Vec::new(),
        facts: fixture.facts.clone(),
        model: ModelRef { model_version: format!("intdfm:{}", fixture.source.intdfm_commit), body: "TEST-ONLY".into(), source: "native".into() },
    };
    let report = check(&input).unwrap_or_else(|error| panic!("{}: {error}", fixture.case.id));
    (fixture, report)
}

fn rust_findings<'a>(report: &'a Report, rule: &str) -> Vec<&'a Finding> {
    report.findings.iter().filter(|finding| finding.rules.iter().any(|name| name == rule)).collect()
}

fn expected_findings<'a>(fixture: &'a Fixture, rule: &str) -> Vec<&'a ExpectedFinding> {
    fixture.expected.findings.iter().filter(|finding| finding.rules.iter().any(|name| name == rule)).collect()
}

/// A needs_review finding is the one place the measured value and the limit must agree exactly.
fn compare_reviews(fixture: &Fixture, report: &Report, rule: &str) {
    let rust: Vec<&Finding> = rust_findings(report, rule).into_iter().filter(|finding| finding.status == RuleStatus::NeedsReview).collect();
    let expected: Vec<&ExpectedFinding> =
        expected_findings(fixture, rule).into_iter().filter(|finding| finding.status == RuleStatus::NeedsReview).collect();
    assert_eq!(rust.len(), expected.len(), "{}: {} needs_review count", fixture.case.id, rule);
    for (actual, wanted) in rust.iter().zip(expected.iter()) {
        assert_eq!(actual.error_code, wanted.error_code, "{}: {} error code", fixture.case.id, rule);
        let measured = wanted.measured.as_ref().map(|measured| measured.value).expect("a review has a measured value");
        assert!((actual.measured_mm.expect("rust measured") - measured).abs() <= 1e-9, "{}: {} measured", fixture.case.id, rule);
        let limit = wanted.requirement.as_ref().map(|requirement| requirement.value.value).expect("a review has a requirement");
        assert!((actual.requirement.as_ref().expect("rust requirement").value_mm - limit).abs() <= 1e-9, "{}: {} limit", fixture.case.id, rule);
    }
}

/// The expected status and code of the merged finding that names this rule. A merged
/// `not_verified` finding may name several rules and carry no shared measurement.
fn compare_status(fixture: &Fixture, report: &Report, rule: &str) {
    let expected = expected_findings(fixture, rule);
    assert!(!expected.is_empty(), "{}: no expected finding for {rule}", fixture.case.id);
    let statuses: Vec<(RuleStatus, Option<String>, Option<String>)> =
        expected.iter().map(|finding| (finding.status, finding.error_code.clone(), finding.reason_code.clone())).collect();
    for actual in rust_findings(report, rule) {
        assert!(
            statuses.iter().any(|(status, error, reason)| *status == actual.status && *error == actual.error_code && *reason == actual.reason_code),
            "{}: {rule} produced {:?}/{:?}/{:?}, expected {statuses:?}",
            fixture.case.id,
            actual.status,
            actual.error_code,
            actual.reason_code
        );
    }
}

#[test]
fn every_recorded_case_matches_the_python_rule_layer() {
    let manifest_text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/intdfm/manifest.json")).expect("manifest");
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text).expect("manifest JSON");
    let cases: Vec<String> =
        manifest["required"].as_array().expect("required").iter().filter_map(|value| value.as_str().map(str::to_string)).collect();
    assert!(cases.len() >= 10);
    for name in &cases {
        let (fixture, report) = run(name);
        for rule in ["sm.hole_min_diameter", "sm.hole_to_edge", "sm.hole_to_bend"] {
            compare_status(&fixture, &report, rule);
            compare_reviews(&fixture, &report, rule);
        }
        assert_eq!(report.rules_version, solidcraft_dfm_rules::VERSION);
        assert_eq!(report.model.model_version, format!("intdfm:{}", fixture.source.intdfm_commit));
        assert_eq!(report.not_checked, solidcraft_dfm_rules::NOT_CHECKED);
    }
}

#[test]
fn the_same_input_gives_the_same_bytes() {
    let (_, first) = run("hole_min_diameter.below");
    let (_, second) = run("hole_min_diameter.below");
    assert_eq!(canonical_json(&first).expect("json"), canonical_json(&second).expect("json"));
}

#[test]
fn generic_numbers_only_advise_and_never_fail() {
    let (fixture, report) = run("hole_min_diameter.below");
    let finding =
        rust_findings(&report, "sm.hole_min_diameter").into_iter().find(|finding| finding.status == RuleStatus::NeedsReview).expect("below");
    assert_eq!(finding.state, AppState::Advisory);
    assert_eq!(finding.requirement.as_ref().expect("requirement").layer, Layer::Generic);
    assert!(!report.clean, "{}", fixture.case.id);
}

#[test]
fn a_boundary_value_is_checked_not_advised() {
    let (_, report) = run("hole_min_diameter.boundary");
    let finding = rust_findings(&report, "sm.hole_min_diameter").into_iter().next().expect("finding");
    assert_eq!(finding.status, RuleStatus::NotVerified);
    assert_eq!(finding.reason_code.as_deref(), Some("ABOVE_PUBLIC_VALUE"));
    assert_eq!(finding.state, AppState::Checked);
}

#[test]
fn a_factory_row_without_confirmation_still_only_advises() {
    let (_, report) = run("hole_min_diameter.factory.below");
    let finding =
        rust_findings(&report, "sm.hole_min_diameter").into_iter().find(|finding| finding.status == RuleStatus::NeedsReview).expect("below");
    assert_eq!(finding.requirement.as_ref().expect("requirement").layer, Layer::Factory);
    assert_eq!(finding.requirement.as_ref().expect("requirement").confirmation, None);
    assert_eq!(finding.state, AppState::Advisory);
}

#[test]
fn missing_thickness_and_failed_hole_facts_are_unsupported() {
    let (_, missing) = run("missing_thickness");
    for rule in ["sm.hole_min_diameter", "sm.hole_to_edge"] {
        assert_eq!(rust_findings(&missing, rule).first().expect(rule).state, AppState::Unsupported, "{rule}");
    }
    assert_eq!(rust_findings(&missing, "sm.hole_to_bend").first().expect("bend").status, RuleStatus::NotApplicable);
    let (_, failed) = run("holes_step_failed");
    for rule in ["sm.hole_min_diameter", "sm.hole_to_edge"] {
        assert_eq!(rust_findings(&failed, rule).first().expect(rule).reason_code.as_deref(), Some("HOLES_UNRECOGNISED"), "{rule}");
    }
    // The plate is flat, so hole-to-bend has no bend to measure even when the hole step failed.
    assert_eq!(rust_findings(&failed, "sm.hole_to_bend").first().expect("bend").status, RuleStatus::NotApplicable);
}

#[test]
fn a_shop_row_for_another_material_does_not_fall_back() {
    let (_, report) = run("shop_not_applicable");
    let finding = rust_findings(&report, "sm.hole_min_diameter").into_iter().next().expect("finding");
    assert_eq!(finding.reason_code.as_deref(), Some("COVERAGE_NOT_IN_PROFILE"));
    assert_eq!(finding.state, AppState::Unsupported);
}

#[test]
fn every_case_keeps_the_model_version_and_its_location() {
    let (_, report) = run("hole_to_bend.below");
    let finding = rust_findings(&report, "sm.hole_to_bend").into_iter().find(|finding| finding.status == RuleStatus::NeedsReview).expect("below");
    assert!(finding.location.is_some(), "a finding points at the model");
    assert_eq!(finding.feature.kind, "hole");
    assert!(finding.id.contains("hole-"));
    assert!(report.model.model_version.starts_with("intdfm:"));
}
