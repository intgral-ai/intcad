//! Full-catalog rule parity against INTdfm's recorded reports.
//!
//! These fixtures carry the recorded input plus every expected finding. The Rust catalog is run
//! as one list, then each rule's status/code/measured/limit is compared. This is rule parity; the
//! report bytes are covered separately by `report_parity.rs`.

use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use solidcraft_dfm_rules::{CheckInput, Feature, Finding, Location, RequirementRef, RuleStatus, catalog_findings};

#[derive(Deserialize)]
struct Fixture {
    input: CheckInput,
    #[serde(rename = "inputGate")]
    input_gate: Value,
    expected: Vec<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    rules: Vec<String>,
    status: RuleStatus,
    error_code: Option<String>,
    reason_code: Option<String>,
    measured: Option<Measured>,
    requirement: Option<Requirement>,
}

#[derive(Deserialize)]
struct Measured {
    value: f64,
}

#[derive(Deserialize)]
struct Requirement {
    value: Option<Measured>,
}

fn gate(value: &Value) -> Finding {
    let status: RuleStatus = serde_json::from_value(value.get("status").cloned().unwrap_or(Value::Null)).expect("gate status");
    let measured = value.get("measured").and_then(|m| m.get("value")).and_then(Value::as_f64);
    Finding {
        id: "input".into(),
        rules: value
            .get("rules")
            .and_then(Value::as_array)
            .map(|rules| rules.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
        status,
        state: solidcraft_dfm_rules::AppState::Checked,
        error_code: value.get("error_code").and_then(Value::as_str).map(str::to_string),
        reason_code: value.get("reason_code").and_then(Value::as_str).map(str::to_string),
        message: value.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
        feature: Feature { kind: "sheet".into(), number: None, description: "the sheet".into() },
        location: None::<Location>,
        measured_mm: measured,
        requirement: None::<RequirementRef>,
        needs: value
            .get("needs")
            .and_then(Value::as_array)
            .map(|needs| needs.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
    }
}

fn fixture(name: &str) -> Fixture {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog/v1").join(format!("{name}.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn check_case(name: &str) {
    let fixture = fixture(name);
    let findings = catalog_findings(&fixture.input, &gate(&fixture.input_gate)).expect(name);
    for expected in &fixture.expected {
        for rule in &expected.rules {
            let actual: Vec<&Finding> = findings.iter().filter(|finding| finding.rules.iter().any(|name| name == rule)).collect();
            assert!(!actual.is_empty(), "{name}: no Rust finding for {rule}");
            assert!(
                actual.iter().any(|finding| finding.status == expected.status
                    && finding.error_code == expected.error_code
                    && finding.reason_code == expected.reason_code),
                "{name}: {rule} expected {:?}/{:?}/{:?}, got {:?}",
                expected.status,
                expected.error_code,
                expected.reason_code,
                actual.iter().map(|finding| (finding.status, finding.error_code.clone(), finding.reason_code.clone())).collect::<Vec<_>>()
            );
            if let Some(measured) = &expected.measured {
                let closest = actual
                    .iter()
                    .filter_map(|finding| finding.measured_mm)
                    .min_by(|a, b| (a - measured.value).abs().total_cmp(&(b - measured.value).abs()));
                if let Some(value) = closest {
                    assert!((value - measured.value).abs() <= 1e-4, "{name}: {rule} measured {value} vs {}", measured.value);
                }
            }
            if let Some(requirement) = &expected.requirement
                && let Some(limit) = requirement.value.as_ref().map(|value| value.value)
            {
                let closest = actual
                    .iter()
                    .filter_map(|finding| finding.requirement.as_ref().map(|requirement| requirement.value_mm))
                    .min_by(|a, b| (a - limit).abs().total_cmp(&(b - limit).abs()));
                if let Some(value) = closest {
                    assert!((value - limit).abs() <= 1e-4, "{name}: {rule} limit {value} vs {limit}");
                }
            }
        }
    }
}

#[test]
fn a_flat_plate_matches_the_recorded_catalog() {
    check_case("hole_min_diameter.below");
}

#[test]
fn an_l_bracket_matches_the_recorded_catalog() {
    check_case("hole_to_bend.below");
}

#[test]
fn a_u_channel_matches_the_recorded_catalog() {
    check_case("hole_to_bend.u_holes");
}

#[test]
fn a_multi_hole_plate_matches_the_recorded_catalog() {
    check_case("hole_to_edge.plate_holes");
}

#[test]
fn missing_thickness_blocks_the_recorded_catalog() {
    check_case("missing_thickness");
}

#[test]
fn the_full_catalog_report_contains_every_non_gate_rule() {
    let fixture = fixture("hole_to_bend.below");
    let report = solidcraft_dfm_rules::check_catalog(&fixture.input).expect("full catalog");
    assert!(report.rules.len() >= 15, "rules: {}", report.rules.len());
    for rule in [
        "sm.hole_min_diameter",
        "sm.hole_to_edge",
        "sm.hole_to_bend",
        "sm.hole_spacing",
        "sm.inside_radius",
        "sm.min_flange",
        "sm.max_bend_length",
        "sm.bend_relief_present",
        "sm.min_web_parallel_bends",
        "sm.u_channel_ratio",
        "sm.bend_radius_consistent",
        "sm.tolerance_across_bends",
        "sm.countersink_depth",
        "sm.countersink_to_edge",
        "sm.countersink_to_bend",
    ] {
        assert!(report.rules.iter().any(|result| result.rule_id == rule), "missing {rule}");
    }
    assert!(report.findings.iter().any(|finding| finding.rules.iter().any(|rule| rule == "sm.gate_profile_coverage")));
}
