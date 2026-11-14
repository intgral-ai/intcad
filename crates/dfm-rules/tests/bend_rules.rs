//! The bend rules against the INTdfm recordings, plus injected thresholds.
//!
//! The three JSON constants are compact projections of schema-2 recordings in INTdfm's
//! `tests/fixtures/dfm-replay/v1`, at the commits named beside each. Only the fields these rules
//! read are kept: the generic layer's unrelated hole rows and the hole facts are dropped because
//! `choose` only looks for the rule's own rows and these rules read no hole. The projections are
//! committed here, so the test never needs the INTdfm checkout.
//!
//! The recorded comparison uses rule id, status, `error_code`, `reason_code`, `measured` and the
//! chosen limit from `expected.report.findings`. Messages are not compared yet: `bend_rules.rs`
//! reproduces INTdfm's wording only approximately, and text parity is a later pass. The injected
//! cases have no recording, so they assert the status, code, measured value and limit that
//! `rules.py` gives for the same facts.

use std::collections::BTreeMap;

use serde::Deserialize;
use solidcraft_dfm_rules::bend_rules::{
    BEND_GATE, BEND_RADII, BEND_RULES, INSIDE_RADIUS, MAX_BEND_LENGTH, bend_radius_consistent, bends_gate, flat_finding, inside_radius, is_flat,
    max_bend_length,
};
use solidcraft_dfm_rules::{
    AppState, Applies, CheckInput, Confirmation, Declared, Facts, Feature, Finding, Layer, Limit, ModelRef, Profile, Requirement, RuleStatus,
};

/// `hole_to_bend.below`, INTdfm `2f24830fcd84c8e3047eb6643139b94c27b8c1b6`.
const BEND_BELOW: &str = r#"{
 "declared": { "material": "5052-H32", "process": "press brake", "thicknessMm": 1.5 },
 "expected": [
  { "error_code": null, "limit": null, "measured": 1.0, "reason_code": null,
    "rules": ["sm.gate_bends_found"], "status": "pass" },
  { "error_code": null, "limit": null, "measured": null, "reason_code": "NO_SHOP_NUMBER",
    "rules": ["sm.inside_radius", "sm.min_flange", "sm.max_bend_length", "sm.bend_relief_present",
              "sm.bend_radius_consistent"],
    "status": "not_verified" }
 ],
 "facts": {
  "paired_mm": 1.5,
  "plate_gap_mm": null,
  "recognition": {
   "bends": [ { "angle_deg": 90.0, "inside_radius_mm": 3.0, "length_mm": 25.0,
                "x_mm": 1.3180194846605358, "y_mm": 12.5, "z_mm": 1.3180194846605358 } ],
   "cones": 0, "forms": 0, "hems": 0, "joined": true, "sharp_joins": 0, "unknown_faces": 0
  }
 },
 "generic": { "format": 1, "layer": "generic", "name": "generic", "requirements": [], "version": "2026-11-10" }
}"#;

/// `hole_to_bend.u_holes`, INTdfm `79c568200d1394e276865a0c34c23e2774ad4669`.
const U_HOLES: &str = r#"{
 "declared": { "material": "5052-H32", "process": "press brake", "thicknessMm": 1.5 },
 "expected": [
  { "error_code": null, "limit": null, "measured": 2.0, "reason_code": null,
    "rules": ["sm.gate_bends_found"], "status": "pass" },
  { "error_code": null, "limit": null, "measured": null, "reason_code": "NO_SHOP_NUMBER",
    "rules": ["sm.inside_radius", "sm.min_flange", "sm.max_bend_length", "sm.bend_relief_present",
              "sm.min_web_parallel_bends", "sm.u_channel_ratio", "sm.bend_radius_consistent"],
    "status": "not_verified" }
 ],
 "facts": {
  "paired_mm": 1.5,
  "plate_gap_mm": null,
  "recognition": {
   "bends": [
    { "angle_deg": 90.0, "inside_radius_mm": 3.0, "length_mm": 25.0,
      "x_mm": -3.1819805153394642, "y_mm": 12.5, "z_mm": 1.3180194846605353 },
    { "angle_deg": 90.0, "inside_radius_mm": 3.0, "length_mm": 25.0,
      "x_mm": 33.18198051533946, "y_mm": 12.5, "z_mm": 1.3180194846605358 }
   ],
   "cones": 0, "forms": 0, "hems": 0, "joined": true, "sharp_joins": 0, "unknown_faces": 0
  }
 },
 "generic": { "format": 1, "layer": "generic", "name": "generic", "requirements": [], "version": "2026-11-10" }
}"#;

/// `hole_min_diameter.below`, INTdfm `2f24830fcd84c8e3047eb6643139b94c27b8c1b6`. A flat plate:
/// the recording's report carries the one `not_applicable` finding for every bend rule.
const PLATE: &str = r#"{
 "declared": { "material": "5052-H32", "process": "press brake", "thicknessMm": 1.5 },
 "expected": [
  { "error_code": null, "limit": null, "measured": null, "reason_code": null,
    "rules": ["sm.gate_bends_found", "sm.inside_radius", "sm.min_flange", "sm.max_bend_length",
              "sm.hole_to_bend", "sm.bend_relief_present", "sm.min_web_parallel_bends",
              "sm.u_channel_ratio", "sm.bend_radius_consistent", "sm.bend_relief_width",
              "sm.bend_relief_depth"],
    "status": "not_applicable" }
 ],
 "facts": {
  "paired_mm": 1.5,
  "plate_gap_mm": 1.5,
  "recognition": {
   "bends": [], "cones": 0, "forms": 0, "hems": 0, "joined": true, "sharp_joins": 0, "unknown_faces": 0
  }
 },
 "generic": { "format": 1, "layer": "generic", "name": "generic", "requirements": [], "version": "2026-11-10" }
}"#;

#[derive(Deserialize)]
struct Fixture {
    declared: Declared,
    generic: Profile,
    facts: Facts,
    expected: Vec<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    rules: Vec<String>,
    status: RuleStatus,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    reason_code: Option<String>,
    #[serde(default)]
    measured: Option<f64>,
    #[serde(default)]
    limit: Option<f64>,
}

fn fixture(json: &str) -> Fixture {
    serde_json::from_str(json).expect("the recording projection parses")
}

fn check_input(fixture: &Fixture) -> CheckInput {
    CheckInput {
        declared: fixture.declared.clone(),
        generic: fixture.generic.clone(),
        shop: None,
        overrides: Vec::new(),
        facts: fixture.facts.clone(),
        model: ModelRef { model_version: "intdfm:recording".into(), body: "TEST-ONLY".into(), source: "native".into() },
        critical: Vec::new(),
    }
}

fn part_feature() -> Feature {
    Feature { kind: "part".into(), number: None, description: "whole part".into() }
}

fn pass_gate() -> Finding {
    gate_finding(RuleStatus::Pass, None, None, None)
}

fn refused_input_gate() -> Finding {
    gate_finding(RuleStatus::NotVerified, None, Some("INPUT_NO_UNIT"), None)
}

fn not_declared_gate() -> Finding {
    gate_finding(RuleStatus::NotVerified, None, Some("THICKNESS_NOT_DECLARED"), Some(1.5))
}

fn mismatch_gate() -> Finding {
    gate_finding(RuleStatus::Violation, Some("THICKNESS_MISMATCH"), None, Some(1.8))
}

fn gate_finding(status: RuleStatus, error_code: Option<&str>, reason_code: Option<&str>, measured_mm: Option<f64>) -> Finding {
    let state = match status {
        RuleStatus::Pass => AppState::Checked,
        RuleStatus::Violation => AppState::Failed,
        _ => AppState::Unsupported,
    };
    Finding {
        id: format!("gate/{}", error_code.or(reason_code).unwrap_or("pass")),
        rules: vec!["sm.gate_input_solid".into()],
        status,
        state,
        error_code: error_code.map(str::to_string),
        reason_code: reason_code.map(str::to_string),
        message: "a gate finding".into(),
        feature: part_feature(),
        location: None,
        measured_mm,
        requirement: None,
        needs: vec!["The right declared thickness, or a model at that thickness.".into()],
    }
}

fn requirement(rule: &str, limit: Limit, value: &[(&str, f64)]) -> Requirement {
    let value: BTreeMap<String, f64> = value.iter().map(|(name, number)| ((*name).to_string(), *number)).collect();
    let inputs = value.keys().filter(|name| name.as_str() != "mm").map(|name| (name.clone(), "mm".to_string())).collect();
    Requirement {
        rule: rule.into(),
        limit,
        value,
        inputs,
        measuring_line: "TEST-ONLY".into(),
        method: "TEST-ONLY".into(),
        source: "TEST-ONLY".into(),
        rights: "TEST-ONLY".into(),
        applies: Applies::default(),
        exceptions: Vec::new(),
        validation_samples: Vec::new(),
        die: None,
        bends: None,
        angles_deg: None,
        confirmation: Some(Confirmation::FactoryConfirmed),
    }
}

fn shop_input(fixture: &Fixture, rows: Vec<Requirement>) -> CheckInput {
    let mut input = check_input(fixture);
    input.shop = Some(Profile { format: 1, name: "TEST-ONLY shop".into(), version: "1".into(), layer: Layer::Factory, requirements: rows });
    input
}

fn set_radius(facts: &mut Facts, index: usize, radius: f64) {
    if let Some(bend) = facts.recognition.as_mut().and_then(|recognition| recognition.bends.get_mut(index)) {
        bend.inside_radius_mm = radius;
    }
}

fn set_length(facts: &mut Facts, length: f64) {
    if let Some(recognition) = facts.recognition.as_mut() {
        for bend in &mut recognition.bends {
            bend.length_mm = length;
        }
    }
}

fn set_angle(facts: &mut Facts, angle: f64) {
    if let Some(recognition) = facts.recognition.as_mut() {
        for bend in &mut recognition.bends {
            bend.angle_deg = angle;
        }
    }
}

/// Every finding the three functions under test (and the flat answer) produce for one recording.
fn produced(fixture: &Fixture) -> Vec<Finding> {
    if is_flat(&fixture.facts) {
        return vec![flat_finding()];
    }
    let input = check_input(fixture);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &fixture.facts);
    let mut findings = vec![bends.clone()];
    findings.extend(inside_radius(&input, &gate, &bends, &fixture.facts));
    findings.extend(max_bend_length(&input, &gate, &bends, &fixture.facts));
    findings.extend(bend_radius_consistent(&input, &gate, &bends, &fixture.facts));
    findings
}

/// Compare the fields the recordings can pin today: rule id, status, codes, measured and limit.
fn assert_matches(fixture: &Fixture, findings: &[Finding]) {
    let implemented = [BEND_GATE, INSIDE_RADIUS, MAX_BEND_LENGTH, BEND_RADII];
    for expected in &fixture.expected {
        for rule in expected.rules.iter().filter(|rule| implemented.contains(&rule.as_str())) {
            let hits: Vec<&Finding> = findings.iter().filter(|finding| finding.rules.iter().any(|name| name == rule)).collect();
            assert_eq!(hits.len(), 1, "{rule}: expected one finding, got {}", hits.len());
            let hit = hits[0];
            assert_eq!(hit.status, expected.status, "{rule}: status");
            assert_eq!(hit.error_code, expected.error_code, "{rule}: error_code");
            assert_eq!(hit.reason_code, expected.reason_code, "{rule}: reason_code");
            match (hit.measured_mm, expected.measured) {
                (Some(actual), Some(wanted)) => assert!((actual - wanted).abs() <= 1e-9, "{rule}: measured {actual} != {wanted}"),
                (None, None) => {}
                (actual, wanted) => panic!("{rule}: measured {actual:?} != {wanted:?}"),
            }
            let actual_limit = hit.requirement.as_ref().map(|requirement| requirement.value_mm);
            match (actual_limit, expected.limit) {
                (Some(actual), Some(wanted)) => assert!((actual - wanted).abs() <= 1e-9, "{rule}: limit {actual} != {wanted}"),
                (None, None) => {}
                (actual, wanted) => panic!("{rule}: limit {actual:?} != {wanted:?}"),
            }
        }
    }
}

#[test]
fn flat_recording_is_flat_and_answers_with_one_finding() {
    let recording = fixture(PLATE);
    assert!(is_flat(&recording.facts), "the plate recording is flat");

    let finding = flat_finding();
    assert_eq!(finding.status, RuleStatus::NotApplicable);
    assert_eq!(finding.rules, BEND_RULES.iter().map(|rule| (*rule).to_string()).collect::<Vec<_>>());
    // The recording predates `sm.bend_relief_width`/`sm.bend_relief_depth`, so its expected list is
    // the older nine-rule catalog; every rule it names must still be in the flat answer.
    assert!(recording.expected[0].rules.iter().all(|rule| finding.rules.contains(rule)));
    assert!(finding.error_code.is_none() && finding.reason_code.is_none());
    assert!(finding.measured_mm.is_none() && finding.requirement.is_none());

    assert_matches(&recording, &produced(&recording));
}

#[test]
fn bent_recordings_match_the_recorded_findings() {
    for json in [BEND_BELOW, U_HOLES] {
        let recording = fixture(json);
        assert!(!is_flat(&recording.facts), "a bent part is not flat");
        assert_matches(&recording, &produced(&recording));
    }
}

#[test]
fn the_bend_gate_counts_and_names_what_it_cannot_read() {
    let recording = fixture(U_HOLES);
    let gate = bends_gate(&pass_gate(), &recording.facts);
    assert_eq!(gate.status, RuleStatus::Pass);
    assert_eq!(gate.measured_mm, Some(2.0));

    let mut unread = recording.facts.clone();
    if let Some(recognition) = unread.recognition.as_mut() {
        recognition.unknown_faces = 1;
    }
    let gate = bends_gate(&pass_gate(), &unread);
    assert_eq!(gate.status, RuleStatus::NotVerified);
    assert_eq!(gate.reason_code.as_deref(), Some("BEND_UNRECOGNISED"));
    assert_eq!(gate.measured_mm, Some(2.0));

    let mut unsupported = recording.facts.clone();
    if let Some(recognition) = unsupported.recognition.as_mut() {
        recognition.hems = 1;
    }
    let gate = bends_gate(&pass_gate(), &unsupported);
    assert_eq!(gate.status, RuleStatus::NotVerified);
    assert_eq!(gate.reason_code.as_deref(), Some("FORMS_UNSUPPORTED"));
    assert_eq!(gate.measured_mm, Some(2.0));

    let mut no_step = recording.facts.clone();
    no_step.recognition = None;
    let gate = bends_gate(&pass_gate(), &no_step);
    assert_eq!(gate.status, RuleStatus::NotVerified);
    assert_eq!(gate.reason_code.as_deref(), Some("BEND_UNRECOGNISED"));
    assert_eq!(gate.measured_mm, None);

    // A refused input gate is copied onto the bend gate, as INTdfm does with no usable solid.
    let gate = bends_gate(&refused_input_gate(), &recording.facts);
    assert_eq!(gate.status, RuleStatus::NotVerified);
    assert_eq!(gate.reason_code.as_deref(), Some("INPUT_NO_UNIT"));
}

#[test]
fn a_flat_plate_still_has_a_bend_gate_answer() {
    // The report replaces the gate with `flat_finding`, but the gate itself still answers: no
    // bend, on a part that isn't a plain flat plate.
    let recording = fixture(PLATE);
    let gate = bends_gate(&pass_gate(), &recording.facts);
    assert_eq!(gate.status, RuleStatus::NotVerified);
    assert_eq!(gate.reason_code.as_deref(), Some("BEND_UNRECOGNISED"));
    assert_eq!(gate.measured_mm, Some(0.0));
}

#[test]
fn inside_radius_at_and_below_the_shop_number() {
    let recording = fixture(U_HOLES);
    let input = shop_input(&recording, vec![requirement(INSIDE_RADIUS, Limit::Min, &[("mm", 3.0)])]);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &recording.facts);

    // Exactly at the number is met: the comparison rounds the difference to 1e-6 first.
    let met = inside_radius(&input, &gate, &bends, &recording.facts);
    assert_eq!(met.len(), 1);
    assert_eq!(met[0].status, RuleStatus::Pass);
    assert_eq!(met[0].measured_mm, Some(3.0));
    assert_eq!(met[0].requirement.as_ref().map(|requirement| requirement.value_mm), Some(3.0));

    let mut below = recording.facts.clone();
    set_radius(&mut below, 0, 2.99);
    let found = inside_radius(&input, &gate, &bends, &below);
    let review = found.iter().find(|finding| finding.status == RuleStatus::NeedsReview).expect("one radius review");
    assert_eq!(review.error_code.as_deref(), Some("RADIUS_BELOW_LIMIT"));
    assert_eq!(review.measured_mm, Some(2.99));
    assert_eq!(review.requirement.as_ref().map(|requirement| requirement.value_mm), Some(3.0));
    // The other bend still met the number, and the gate passed, so the rule says so once too.
    assert!(found.iter().any(|finding| finding.status == RuleStatus::Pass));
}

#[test]
fn max_bend_length_reads_an_upper_row() {
    let recording = fixture(U_HOLES);
    let input = shop_input(&recording, vec![requirement(MAX_BEND_LENGTH, Limit::Max, &[("mm", 20.0)])]);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &recording.facts);

    let found = max_bend_length(&input, &gate, &bends, &recording.facts);
    assert_eq!(found.len(), 2, "one review per bend");
    for review in &found {
        assert_eq!(review.status, RuleStatus::NeedsReview);
        assert_eq!(review.error_code.as_deref(), Some("BEND_LENGTH_ABOVE_LIMIT"));
        assert_eq!(review.measured_mm, Some(25.0));
        assert_eq!(review.requirement.as_ref().map(|requirement| requirement.value_mm), Some(20.0));
    }

    let mut at_limit = recording.facts.clone();
    set_length(&mut at_limit, 20.0);
    let met = max_bend_length(&input, &gate, &bends, &at_limit);
    assert_eq!(met.len(), 1);
    assert_eq!(met[0].status, RuleStatus::Pass);
    assert_eq!(met[0].measured_mm, Some(20.0));

    // A lower-limit row for an upper rule is not a number this rule can work out.
    let wrong = shop_input(&recording, vec![requirement(MAX_BEND_LENGTH, Limit::Min, &[("mm", 20.0)])]);
    let blocked = max_bend_length(&wrong, &gate, &bends, &recording.facts);
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].status, RuleStatus::NotVerified);
    assert_eq!(blocked[0].reason_code.as_deref(), Some("REQUIREMENT_INPUT_UNKNOWN"));
}

#[test]
fn bend_radius_consistent_groups_within_five_hundredths() {
    let recording = fixture(U_HOLES);
    let input = shop_input(&recording, vec![requirement(BEND_RADII, Limit::Min, &[("mm", 0.0)])]);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &recording.facts);

    let same = bend_radius_consistent(&input, &gate, &bends, &recording.facts);
    assert_eq!(same.len(), 1);
    assert_eq!(same[0].status, RuleStatus::Pass);
    assert_eq!(same[0].measured_mm, Some(1.0));

    let mut within = recording.facts.clone();
    set_radius(&mut within, 1, 3.04);
    let one_group = bend_radius_consistent(&input, &gate, &bends, &within);
    assert_eq!(one_group.len(), 1);
    assert_eq!(one_group[0].status, RuleStatus::Pass, "0.04 mm apart is one radius");

    let mut mixed = recording.facts.clone();
    set_radius(&mut mixed, 1, 3.06);
    let two = bend_radius_consistent(&input, &gate, &bends, &mixed);
    assert_eq!(two.len(), 1);
    assert_eq!(two[0].status, RuleStatus::NeedsReview);
    assert_eq!(two[0].error_code.as_deref(), Some("BEND_RADII_MIXED"));
    assert_eq!(two[0].measured_mm, Some(2.0), "measured is the number of radii");
    assert_eq!(two[0].requirement.as_ref().map(|requirement| requirement.value_mm), Some(0.0));

    // Presence only: with no shop at all, mixed radii still ask for a look, and no requirement is
    // quoted because there is no number to quote.
    let no_shop = check_input(&recording);
    let mixed_no_shop = bend_radius_consistent(&no_shop, &gate, &bends, &mixed);
    assert_eq!(mixed_no_shop.len(), 1);
    assert_eq!(mixed_no_shop[0].status, RuleStatus::NeedsReview);
    assert_eq!(mixed_no_shop[0].error_code.as_deref(), Some("BEND_RADII_MIXED"));
    assert!(mixed_no_shop[0].requirement.is_none());
}

#[test]
fn an_off_ninety_bend_is_skipped_only_when_a_number_exists() {
    let recording = fixture(U_HOLES);
    let mut off = recording.facts.clone();
    set_angle(&mut off, 75.0);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &off);

    // No number: the rule says so, and never reaches the 90-degree skip.
    let no_number = inside_radius(&check_input(&recording), &gate, &bends, &off);
    assert_eq!(no_number.len(), 1);
    assert_eq!(no_number[0].reason_code.as_deref(), Some("NO_SHOP_NUMBER"));

    let input = shop_input(&recording, vec![requirement(INSIDE_RADIUS, Limit::Min, &[("mm", 3.0)])]);
    let found = inside_radius(&input, &gate, &bends, &off);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, RuleStatus::NotVerified);
    assert_eq!(found[0].reason_code.as_deref(), Some("BEND_NOT_90"));
    assert_eq!(found[0].measured_mm, None);

    // At exactly five degrees off 90 the bend is still held to 90's limits.
    let mut near = recording.facts.clone();
    set_angle(&mut near, 85.0);
    let held = inside_radius(&input, &gate, &bends_gate(&gate, &near), &near);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].status, RuleStatus::Pass);
}

#[test]
fn the_thickness_gate_blocks_the_three_rules() {
    let recording = fixture(U_HOLES);
    let input = check_input(&recording);
    let bends = bends_gate(&pass_gate(), &recording.facts);

    let gate = not_declared_gate();
    for found in [
        inside_radius(&input, &gate, &bends, &recording.facts),
        max_bend_length(&input, &gate, &bends, &recording.facts),
        bend_radius_consistent(&input, &gate, &bends, &recording.facts),
    ] {
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, RuleStatus::NotVerified);
        assert_eq!(found[0].reason_code.as_deref(), Some("THICKNESS_NOT_DECLARED"));
        assert_eq!(found[0].measured_mm, Some(1.5), "the gate's own measure is kept");
    }

    let gate = mismatch_gate();
    for found in [
        inside_radius(&input, &gate, &bends, &recording.facts),
        max_bend_length(&input, &gate, &bends, &recording.facts),
        bend_radius_consistent(&input, &gate, &bends, &recording.facts),
    ] {
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].reason_code.as_deref(), Some("THICKNESS_MISMATCH"));
        assert_eq!(found[0].measured_mm, None, "a mismatch carries no measured value");
    }
}

#[test]
fn a_part_with_no_bend_shares_the_bend_gates_reason() {
    let recording = fixture(PLATE);
    let mut facts = recording.facts.clone();
    // Not flat: the skins are not a measured plate, but there is still no bend to measure.
    facts.plate_gap_mm = None;
    let gate = pass_gate();
    let bends = bends_gate(&gate, &facts);
    assert_eq!(bends.reason_code.as_deref(), Some("BEND_UNRECOGNISED"));
    let input = check_input(&recording);

    for found in [
        inside_radius(&input, &gate, &bends, &facts),
        max_bend_length(&input, &gate, &bends, &facts),
        bend_radius_consistent(&input, &gate, &bends, &facts),
    ] {
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, RuleStatus::NotVerified);
        assert_eq!(found[0].reason_code.as_deref(), Some("BEND_UNRECOGNISED"));
        assert_eq!(found[0].measured_mm, Some(0.0), "the gate's count is shared");
    }
}
