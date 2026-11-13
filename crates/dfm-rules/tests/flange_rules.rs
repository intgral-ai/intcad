//! The flange, web, U and relief rules against the INTdfm recordings, plus injected thresholds.
//!
//! The three JSON constants are compact projections of schema-2 recordings in INTdfm's
//! `tests/fixtures/dfm-replay/v1`, at the commits named beside each. Only the fields these rules
//! read are kept: the generic layer's hole rows and the hole facts are dropped, because the rules
//! look only for their own rows and read no hole. The projections are committed here, so the test
//! never needs the INTdfm checkout.
//!
//! The recordings carry no shop row for any of these rules, and neither of the bent parts has a
//! relief end, so they pin the `NO_SHOP_NUMBER`, `not_applicable` and flat answers only. Every
//! threshold, die, floor, web, U and relief case below is **TEST-ONLY**: the recording's facts are
//! mutated and a TEST-ONLY shop profile supplies the number. Those expectations are the values
//! `rules.py` computes for the same facts, not recorded output.
//!
//! The recorded comparison uses rule id, status, `error_code`, `reason_code`, `measured` and the
//! chosen limit from `expected.report.findings`; messages are not byte-compared yet.

use std::collections::BTreeMap;

use serde::Deserialize;
use solidcraft_dfm_rules::bend_rules::{bends_gate, flat_finding, is_flat};
use solidcraft_dfm_rules::flange_rules::{
    BEND_RELIEF, MIN_FLANGE, MIN_WEB, U_CHANNEL, bend_relief_present, min_flange, min_web_parallel_bends, u_channel_ratio,
};
use solidcraft_dfm_rules::{
    AppState, Applies, CheckInput, Confirmation, Declared, Die, Facts, Feature, Finding, Layer, Limit, ModelRef, Profile, Requirement, RuleStatus,
};

/// `hole_to_bend.below`, INTdfm `2f24830fcd84c8e3047eb6643139b94c27b8c1b6`. An L bracket.
const L_BRACKET: &str = r#"{
 "declared": { "process": "press brake", "material": "5052-H32", "thicknessMm": 1.5 },
 "generic": { "format": 1, "name": "generic", "version": "2026-11-10", "layer": "generic", "requirements": [] },
 "facts": {
  "paired_mm": 1.5,
  "plate_gap_mm": null,
  "recognition": {
   "bends": [
    { "inside_radius_mm": 3.0, "angle_deg": 90.0, "length_mm": 25.0,
      "x_mm": 1.3180194846605358, "y_mm": 12.5, "z_mm": 1.3180194846605358,
      "runs_mm": [[40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0,40.0],
                  [30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0]],
      "longest_mm": [40.0, 30.0], "unrelieved": [], "no_margin": [], "crossed": true, "flats": [1, 7] }
   ],
   "webs": [],
   "hems": 0, "unknown_faces": 0, "sharp_joins": 0, "cones": 0, "forms": 0, "joined": true
  }
 },
 "expected": [
  { "rules": ["sm.inside_radius", "sm.min_flange", "sm.max_bend_length", "sm.bend_relief_present", "sm.bend_radius_consistent"],
    "status": "not_verified", "error_code": null, "reason_code": "NO_SHOP_NUMBER", "measured": null, "limit": null },
  { "rules": ["sm.min_web_parallel_bends"], "status": "not_applicable", "error_code": null, "reason_code": null, "measured": null, "limit": null },
  { "rules": ["sm.u_channel_ratio"], "status": "not_applicable", "error_code": null, "reason_code": null, "measured": null, "limit": null }
 ]
}"#;

/// `hole_to_bend.u_holes`, INTdfm `79c568200d1394e276865a0c34c23e2774ad4669`. A U channel.
const U_CHANNEL_PART: &str = r#"{
 "declared": { "process": "press brake", "material": "5052-H32", "thicknessMm": 1.5 },
 "generic": { "format": 1, "name": "generic", "version": "2026-11-10", "layer": "generic", "requirements": [] },
 "facts": {
  "paired_mm": 1.5,
  "plate_gap_mm": null,
  "recognition": {
   "bends": [
    { "inside_radius_mm": 3.0, "angle_deg": 90.0, "length_mm": 25.0,
      "x_mm": -3.1819805153394642, "y_mm": 12.5, "z_mm": 1.3180194846605353,
      "runs_mm": [[30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0],
                  [20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0]],
      "longest_mm": [30.0, 20.0], "unrelieved": [], "no_margin": [], "crossed": true, "flats": [1, 7] },
    { "inside_radius_mm": 3.0, "angle_deg": 90.0, "length_mm": 25.0,
      "x_mm": 33.18198051533946, "y_mm": 12.5, "z_mm": 1.3180194846605358,
      "runs_mm": [[30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0,30.0],
                  [20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0,20.0]],
      "longest_mm": [30.0, 20.0], "unrelieved": [], "no_margin": [], "crossed": true, "flats": [1, 8] }
   ],
   "webs": [
    { "bends": [1, 2], "flanges_mm": [20.0, 20.0], "same_side": true, "web_mm": 30.0,
      "x_mm": 15.0, "y_mm": 12.5, "z_mm": -2.2204460492505e-16 }
   ],
   "hems": 0, "unknown_faces": 0, "sharp_joins": 0, "cones": 0, "forms": 0, "joined": true
  }
 },
 "expected": [
  { "rules": ["sm.inside_radius", "sm.min_flange", "sm.max_bend_length", "sm.bend_relief_present",
              "sm.min_web_parallel_bends", "sm.u_channel_ratio", "sm.bend_radius_consistent"],
    "status": "not_verified", "error_code": null, "reason_code": "NO_SHOP_NUMBER", "measured": null, "limit": null }
 ]
}"#;

/// `hole_min_diameter.below`, INTdfm `2f24830fcd84c8e3047eb6643139b94c27b8c1b6`. A flat plate,
/// whose report carries the one `not_applicable` finding for every bend rule.
const PLATE: &str = r#"{
 "declared": { "process": "press brake", "material": "5052-H32", "thicknessMm": 1.5 },
 "generic": { "format": 1, "name": "generic", "version": "2026-11-10", "layer": "generic", "requirements": [] },
 "facts": {
  "paired_mm": 1.5,
  "plate_gap_mm": 1.5,
  "recognition": {
   "bends": [], "webs": [],
   "hems": 0, "unknown_faces": 0, "sharp_joins": 0, "cones": 0, "forms": 0, "joined": true
  }
 },
 "expected": [
  { "rules": ["sm.gate_bends_found", "sm.inside_radius", "sm.min_flange", "sm.max_bend_length",
              "sm.hole_to_bend", "sm.bend_relief_present", "sm.min_web_parallel_bends",
              "sm.u_channel_ratio", "sm.bend_radius_consistent"],
    "status": "not_applicable", "error_code": null, "reason_code": null, "measured": null, "limit": null }
 ]
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

fn check_input(record: &Fixture) -> CheckInput {
    CheckInput {
        declared: record.declared.clone(),
        generic: record.generic.clone(),
        shop: None,
        overrides: Vec::new(),
        facts: record.facts.clone(),
        model: ModelRef { model_version: "intdfm:recording".into(), body: "TEST-ONLY".into(), source: "native".into() },
        critical: Vec::new(),
    }
}

fn part_feature() -> Feature {
    Feature { kind: "part".into(), number: None, description: "whole part".into() }
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

fn pass_gate() -> Finding {
    gate_finding(RuleStatus::Pass, None, None, None)
}

fn not_declared_gate() -> Finding {
    gate_finding(RuleStatus::NotVerified, None, Some("THICKNESS_NOT_DECLARED"), Some(1.5))
}

fn requirement_with(rule: &str, limit: Limit, value: &[(&str, f64)], applies: Applies) -> Requirement {
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
        applies,
        exceptions: Vec::new(),
        validation_samples: Vec::new(),
        die: None,
        bends: None,
        confirmation: Some(Confirmation::FactoryConfirmed),
    }
}

fn requirement(rule: &str, limit: Limit, value: &[(&str, f64)]) -> Requirement {
    requirement_with(rule, limit, value, Applies::default())
}

fn shop_input(record: &Fixture, rows: Vec<Requirement>) -> CheckInput {
    let mut input = check_input(record);
    input.shop = Some(Profile { format: 1, name: "TEST-ONLY shop".into(), version: "1".into(), layer: Layer::Factory, requirements: rows });
    input
}

fn die_row(opening: f64, low: f64, high: f64, share: f64, limit_mm: f64) -> Requirement {
    let mut row = requirement(MIN_FLANGE, Limit::Min, &[("mm", limit_mm)]);
    row.die = Some(Die { v_mm: opening, forms_radius_mm: vec![low, high], required_share: share });
    row
}

fn set_runs(facts: &mut Facts, index: usize, runs: Vec<Vec<f64>>) {
    if let Some(bend) = facts.recognition.as_mut().and_then(|recognition| recognition.bends.get_mut(index)) {
        bend.runs_mm = runs;
    }
}

fn set_web(facts: &mut Facts, web_mm: Option<f64>, same_side: Option<bool>) {
    if let Some(web) = facts.recognition.as_mut().and_then(|recognition| recognition.webs.first_mut()) {
        if let Some(web_mm) = web_mm {
            web.web_mm = web_mm;
        }
        if let Some(same_side) = same_side {
            web.same_side = same_side;
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

fn set_crossed(facts: &mut Facts, crossed: bool) {
    if let Some(recognition) = facts.recognition.as_mut() {
        for bend in &mut recognition.bends {
            bend.crossed = crossed;
        }
    }
}

fn run_all(input: &CheckInput, record: &Fixture) -> Vec<Finding> {
    if is_flat(&record.facts) {
        return vec![flat_finding()];
    }
    let gate = pass_gate();
    let bends = bends_gate(&gate, &record.facts);
    let mut findings = Vec::new();
    findings.extend(min_flange(input, &gate, &bends, &record.facts));
    findings.extend(min_web_parallel_bends(input, &gate, &bends, &record.facts));
    findings.extend(u_channel_ratio(input, &gate, &bends, &record.facts));
    findings.extend(bend_relief_present(input, &gate, &bends, &record.facts));
    findings
}

fn assert_matches(record: &Fixture, findings: &[Finding]) {
    let implemented = [MIN_FLANGE, MIN_WEB, U_CHANNEL, BEND_RELIEF];
    for expected in &record.expected {
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
fn recordings_match_the_recorded_findings() {
    for json in [L_BRACKET, U_CHANNEL_PART] {
        let record = fixture(json);
        assert!(!is_flat(&record.facts));
        assert_matches(&record, &run_all(&check_input(&record), &record));
    }
}

#[test]
fn a_flat_plate_is_answered_once_by_the_shared_flat_finding() {
    let record = fixture(PLATE);
    assert!(is_flat(&record.facts));
    let expected = flat_finding();
    assert_eq!(expected.status, RuleStatus::NotApplicable);
    assert_matches(&record, &run_all(&check_input(&record), &record));

    // Each rule returns the same nine-rule finding, so running one alone cannot lose the answer.
    let gate = pass_gate();
    let bends = gate_finding(RuleStatus::NotVerified, None, Some("BEND_UNRECOGNISED"), Some(0.0));
    let input = check_input(&record);
    for found in [
        min_flange(&input, &gate, &bends, &record.facts),
        min_web_parallel_bends(&input, &gate, &bends, &record.facts),
        u_channel_ratio(&input, &gate, &bends, &record.facts),
        bend_relief_present(&input, &gate, &bends, &record.facts),
    ] {
        assert_eq!(found, vec![expected.clone()]);
    }
}

#[test]
fn min_flange_uses_each_flat_and_the_required_share() {
    let record = fixture(L_BRACKET);
    let mut facts = record.facts.clone();
    set_crossed(&mut facts, false);
    let input = shop_input(&record, vec![die_row(8.0, 2.5, 3.5, 1.0, 30.0)]);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &facts);

    // Both flats reach 30 mm (40 and 30), so the shorter one meets a 30 mm limit at share 1.
    let met = min_flange(&input, &gate, &bends, &facts);
    assert_eq!(met.len(), 1);
    assert_eq!(met[0].status, RuleStatus::Pass);
    assert_eq!(met[0].measured_mm, Some(30.0));
    assert_eq!(met[0].requirement.as_ref().map(|requirement| requirement.value_mm), Some(30.0));

    // A short run on one flat shortens that flat's reach at share 1.
    let mut short_flat = vec![40.0; 20];
    short_flat[19] = 10.0;
    set_runs(&mut facts, 0, vec![short_flat, vec![40.0; 20]]);
    let short = min_flange(&input, &gate, &bends, &facts);
    let review = short.iter().find(|finding| finding.status == RuleStatus::NeedsReview).expect("one review");
    assert_eq!(review.error_code.as_deref(), Some("FLANGE_BELOW_LIMIT"));
    assert_eq!(review.measured_mm, Some(10.0), "share 1 reads the smallest of the 20 places");
    assert_eq!(review.requirement.as_ref().map(|requirement| requirement.value_mm), Some(30.0));

    // At share 0.5 only 10 of the 20 places must reach it, so the same facts meet the number.
    let half = shop_input(&record, vec![die_row(8.0, 2.5, 3.5, 0.5, 30.0)]);
    let met_half = min_flange(&half, &gate, &bends, &facts);
    assert_eq!(met_half.len(), 1);
    assert_eq!(met_half[0].status, RuleStatus::Pass);
    assert_eq!(met_half[0].measured_mm, Some(40.0), "ceil(0.5 * 20) - 1 = 9 is the 10th longest run");
}

#[test]
fn min_flange_needs_a_die_and_keeps_the_floor() {
    let record = fixture(L_BRACKET);
    let mut facts = record.facts.clone();
    set_crossed(&mut facts, false);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &facts);

    // A die whose range misses the 3 mm radius: the radius is measured and the die is missing.
    let input = shop_input(&record, vec![die_row(12.0, 4.0, 5.0, 1.0, 30.0)]);
    let found = min_flange(&input, &gate, &bends, &facts);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, RuleStatus::NeedsReview);
    assert_eq!(found[0].error_code.as_deref(), Some("FLANGE_NO_DIE"));
    assert_eq!(found[0].measured_mm, Some(3.0), "the radius is what was measured");

    // A die row that passes beside a die-less floor: the floor is quoted when the floor is missed.
    let input = shop_input(&record, vec![die_row(8.0, 2.5, 3.5, 1.0, 25.0), requirement(MIN_FLANGE, Limit::Min, &[("mm", 35.0)])]);
    let found = min_flange(&input, &gate, &bends, &facts);
    let review = found.iter().find(|finding| finding.status == RuleStatus::NeedsReview).expect("the floor review");
    assert_eq!(review.error_code.as_deref(), Some("FLANGE_BELOW_LIMIT"));
    assert_eq!(review.measured_mm, Some(30.0));
    assert_eq!(review.requirement.as_ref().map(|requirement| requirement.value_mm), Some(35.0), "the floor's number");
}

#[test]
fn min_flange_says_which_bend_crossed_a_cut_and_which_is_off_ninety() {
    let record = fixture(L_BRACKET);
    let input = shop_input(&record, vec![die_row(8.0, 2.5, 3.5, 1.0, 30.0)]);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &record.facts);

    // The recording's flange crosses a cut, so it is not measured even though the die forms it.
    let found = min_flange(&input, &gate, &bends, &record.facts);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, RuleStatus::NotVerified);
    assert_eq!(found[0].reason_code.as_deref(), Some("FLANGE_CROSSES_CUT"));
    assert_eq!(found[0].measured_mm, None);

    let mut off = record.facts.clone();
    set_angle(&mut off, 75.0);
    let bends = bends_gate(&gate, &off);
    let found = min_flange(&input, &gate, &bends, &off);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].reason_code.as_deref(), Some("BEND_NOT_90"));
}

#[test]
fn min_web_reads_the_web_and_the_larger_radius() {
    let record = fixture(U_CHANNEL_PART);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &record.facts);

    // A plain number: the 30 mm web meets 20 mm.
    let input = shop_input(&record, vec![requirement(MIN_WEB, Limit::Min, &[("mm", 20.0)])]);
    let met = min_web_parallel_bends(&input, &gate, &bends, &record.facts);
    assert_eq!(met.len(), 1);
    assert_eq!(met[0].status, RuleStatus::Pass);
    assert_eq!(met[0].measured_mm, Some(30.0));
    assert_eq!(met[0].requirement.as_ref().map(|requirement| requirement.value_mm), Some(20.0));

    // A number in terms of t and the larger bend radius: 10t + 2r at t = 1.5, r = 3 is 21 mm.
    let input = shop_input(&record, vec![requirement(MIN_WEB, Limit::Min, &[("t", 10.0), ("r", 2.0)])]);
    let met = min_web_parallel_bends(&input, &gate, &bends, &record.facts);
    assert_eq!(met[0].status, RuleStatus::Pass);
    assert_eq!(met[0].requirement.as_ref().map(|requirement| requirement.value_mm), Some(21.0));

    // A web narrower than the number is one review.
    let mut narrow = record.facts.clone();
    set_web(&mut narrow, Some(15.0), None);
    let input = shop_input(&record, vec![requirement(MIN_WEB, Limit::Min, &[("mm", 20.0)])]);
    let found = min_web_parallel_bends(&input, &gate, &bends, &narrow);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, RuleStatus::NeedsReview);
    assert_eq!(found[0].error_code.as_deref(), Some("WEB_BELOW_LIMIT"));
    assert_eq!(found[0].measured_mm, Some(15.0));
}

#[test]
fn u_channel_ratio_is_an_upper_limit_on_the_taller_flange() {
    let record = fixture(U_CHANNEL_PART);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &record.facts);

    // Half the base: 0.5 * 30 = 15 mm, and the taller flange is 20 mm, so it is too tall.
    let input = shop_input(&record, vec![requirement(U_CHANNEL, Limit::Max, &[("b", 0.5)])]);
    let found = u_channel_ratio(&input, &gate, &bends, &record.facts);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, RuleStatus::NeedsReview);
    assert_eq!(found[0].error_code.as_deref(), Some("U_FLANGE_ABOVE_LIMIT"));
    assert_eq!(found[0].measured_mm, Some(20.0));
    assert_eq!(found[0].requirement.as_ref().map(|requirement| requirement.value_mm), Some(15.0));

    // A base of 1.0 allows 30 mm, so 20 mm meets it.
    let input = shop_input(&record, vec![requirement(U_CHANNEL, Limit::Max, &[("b", 1.0)])]);
    let met = u_channel_ratio(&input, &gate, &bends, &record.facts);
    assert_eq!(met.len(), 1);
    assert_eq!(met[0].status, RuleStatus::Pass);
    assert_eq!(met[0].measured_mm, Some(20.0));

    // A lower-limit row for an upper rule is not a number this rule can work out.
    let input = shop_input(&record, vec![requirement(U_CHANNEL, Limit::Min, &[("b", 0.5)])]);
    let blocked = u_channel_ratio(&input, &gate, &bends, &record.facts);
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].reason_code.as_deref(), Some("REQUIREMENT_INPUT_UNKNOWN"));
}

#[test]
fn a_part_without_a_web_or_u_says_not_applicable() {
    let record = fixture(L_BRACKET);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &record.facts);
    let input = check_input(&record);
    for found in [min_web_parallel_bends(&input, &gate, &bends, &record.facts), u_channel_ratio(&input, &gate, &bends, &record.facts)] {
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, RuleStatus::NotApplicable);
        assert!(found[0].error_code.is_none() && found[0].reason_code.is_none());
    }

    // A web whose two flanges rise on opposite sides is not a U.
    let record = fixture(U_CHANNEL_PART);
    let mut facts = record.facts.clone();
    set_web(&mut facts, None, Some(false));
    let bends = bends_gate(&gate, &facts);
    let found = u_channel_ratio(&check_input(&record), &gate, &bends, &facts);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, RuleStatus::NotApplicable);
}

#[test]
fn relief_reports_each_end_with_its_own_code() {
    let record = fixture(U_CHANNEL_PART);
    let mut facts = record.facts.clone();
    if let Some(recognition) = facts.recognition.as_mut() {
        if let Some(bend) = recognition.bends.first_mut() {
            bend.unrelieved = vec![vec![1.0, 2.0, 3.0]];
        }
        if let Some(bend) = recognition.bends.get_mut(1) {
            bend.no_margin = vec![vec![4.0, 5.0, 6.0]];
        }
    }
    let gate = pass_gate();
    let bends = bends_gate(&gate, &facts);
    let found = bend_relief_present(&check_input(&record), &gate, &bends, &facts);
    assert_eq!(found.len(), 2, "one finding per end, with no number involved");
    assert_eq!(found[0].status, RuleStatus::NeedsReview);
    assert_eq!(found[0].error_code.as_deref(), Some("RELIEF_MISSING"));
    assert_eq!(found[1].error_code.as_deref(), Some("RELIEF_NO_MARGIN"));
    for finding in &found {
        assert_eq!(finding.measured_mm, None, "presence only: nothing is measured");
        assert!(finding.requirement.is_none(), "no shop, so no requirement to quote");
        assert!(finding.location.is_some());
    }

    // With a shop's row and no end in trouble, the rule passes and quotes the measuring line.
    let input = shop_input(&record, vec![requirement(BEND_RELIEF, Limit::Min, &[("mm", 0.0)])]);
    let found = bend_relief_present(&input, &gate, &bends, &record.facts);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, RuleStatus::Pass);
    assert_eq!(found[0].requirement.as_ref().map(|requirement| requirement.text.as_str()), Some("TEST-ONLY"));
}

#[test]
fn a_shop_row_that_does_not_apply_still_stops_the_relief_rule() {
    let record = fixture(U_CHANNEL_PART);
    let gate = pass_gate();
    let bends = bends_gate(&gate, &record.facts);
    let applies = Applies { process: vec!["laser".into()], material: Vec::new(), thickness_mm: None };
    let input = shop_input(&record, vec![requirement_with(BEND_RELIEF, Limit::Min, &[("mm", 0.0)], applies)]);
    let found = bend_relief_present(&input, &gate, &bends, &record.facts);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, RuleStatus::NotVerified);
    assert_eq!(found[0].reason_code.as_deref(), Some("COVERAGE_NOT_IN_PROFILE"));
}

#[test]
fn the_thickness_gate_blocks_all_four_rules() {
    let record = fixture(U_CHANNEL_PART);
    let input = check_input(&record);
    let gate = not_declared_gate();
    let bends = bends_gate(&pass_gate(), &record.facts);
    for found in [
        min_flange(&input, &gate, &bends, &record.facts),
        min_web_parallel_bends(&input, &gate, &bends, &record.facts),
        u_channel_ratio(&input, &gate, &bends, &record.facts),
        bend_relief_present(&input, &gate, &bends, &record.facts),
    ] {
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, RuleStatus::NotVerified);
        assert_eq!(found[0].reason_code.as_deref(), Some("THICKNESS_NOT_DECLARED"));
        assert_eq!(found[0].measured_mm, Some(1.5));
    }
}

#[test]
fn a_bend_the_gate_could_not_confirm_keeps_the_gates_reason() {
    let record = fixture(U_CHANNEL_PART);
    let mut unread = record.facts.clone();
    if let Some(recognition) = unread.recognition.as_mut() {
        recognition.unknown_faces = 1;
    }
    let gate = pass_gate();
    let bends = bends_gate(&gate, &unread);
    assert_eq!(bends.reason_code.as_deref(), Some("BEND_UNRECOGNISED"));
    let input = shop_input(&record, vec![requirement(MIN_WEB, Limit::Min, &[("mm", 20.0)])]);

    // The web meets the number, but a bend the gate could not read may be the one that misses.
    let found = min_web_parallel_bends(&input, &gate, &bends, &unread);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, RuleStatus::NotVerified);
    assert_eq!(found[0].reason_code.as_deref(), Some("BEND_UNRECOGNISED"));
    assert_eq!(found[0].measured_mm, Some(2.0), "the gate's count is shared");
}

#[test]
fn a_part_with_no_bend_shares_the_bend_gates_reason() {
    let record = fixture(PLATE);
    let mut facts = record.facts.clone();
    facts.plate_gap_mm = None; // not flat, but still no bend
    let gate = pass_gate();
    let bends = bends_gate(&gate, &facts);
    let input = check_input(&record);
    for found in [
        min_flange(&input, &gate, &bends, &facts),
        min_web_parallel_bends(&input, &gate, &bends, &facts),
        u_channel_ratio(&input, &gate, &bends, &facts),
        bend_relief_present(&input, &gate, &bends, &facts),
    ] {
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, RuleStatus::NotVerified);
        assert_eq!(found[0].reason_code.as_deref(), Some("BEND_UNRECOGNISED"));
        assert_eq!(found[0].measured_mm, Some(0.0));
    }
}
