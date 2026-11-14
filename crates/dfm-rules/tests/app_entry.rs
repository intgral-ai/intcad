//! The App entry's boundary cases. These are the decisions a UI must show, not rule arithmetic.

use std::collections::BTreeMap;

use solidcraft_dfm_rules::{
    AppState, CheckInput, Confirmation, Declared, Facts, HoleFact, Layer, Limit, ModelRef, Override, Profile, Requirement, RuleStatus, check,
    check_catalog,
};

fn row(rule: &str, value: f64, confirmation: Option<Confirmation>) -> Requirement {
    Requirement {
        rule: rule.into(),
        limit: Limit::Min,
        value: BTreeMap::from([("mm".to_string(), value)]),
        inputs: BTreeMap::new(),
        measuring_line: "TEST-ONLY: a distance.".into(),
        method: "TEST-ONLY, invented.".into(),
        source: "TEST-ONLY".into(),
        rights: "TEST-ONLY".into(),
        applies: Default::default(),
        exceptions: Vec::new(),
        validation_samples: Vec::new(),
        die: None,
        bends: None,
        angles_deg: None,
        confirmation,
    }
}

fn profile(name: &str, layer: Layer, rows: Vec<Requirement>) -> Profile {
    Profile { format: 1, name: name.into(), version: "TEST-ONLY-1".into(), layer, requirements: rows }
}

fn hole(diameter: f64, edge: Option<f64>) -> Facts {
    Facts {
        paired_mm: Some(1.5),
        holes: Some(vec![HoleFact {
            diameter_mm: diameter,
            x_mm: 20.0,
            y_mm: 12.5,
            z_mm: 0.75,
            depth_mm: 1.5,
            square: true,
            edge_mm: edge,
            edge_at_mm: None,
            edge_kind: None,
            edge_hole: None,
            bend_gaps: Vec::new(),
            flat: None,
            countersink: None,
        }]),
        recognition: Some(Default::default()),
        ..Facts::default()
    }
}

/// Round holes by centre and diameter, each with the given edge distance.
fn holes_at(points: &[(f64, f64, f64, Option<f64>)]) -> Facts {
    Facts {
        paired_mm: Some(1.5),
        holes: Some(
            points
                .iter()
                .map(|(x, y, diameter, edge)| HoleFact {
                    diameter_mm: *diameter,
                    x_mm: *x,
                    y_mm: *y,
                    z_mm: 0.75,
                    depth_mm: 1.5,
                    square: true,
                    edge_mm: *edge,
                    edge_at_mm: None,
                    edge_kind: None,
                    edge_hole: None,
                    bend_gaps: Vec::new(),
                    flat: Some(1),
                    countersink: None,
                })
                .collect(),
        ),
        recognition: Some(Default::default()),
        ..Facts::default()
    }
}

fn input(generic: Profile, shop: Option<Profile>, declared: Declared, facts: Facts) -> CheckInput {
    CheckInput {
        declared,
        generic,
        shop,
        overrides: Vec::new(),
        facts,
        model: ModelRef { model_version: "test-model-1".into(), body: "Plate".into(), source: "native".into() },
        critical: Vec::new(),
    }
}

fn declared(thickness: Option<f64>) -> Declared {
    Declared {
        process: Some("press brake".into()),
        material: Some("5052-H32".into()),
        thickness_mm: thickness,
        thickness_from: Some("declared".into()),
    }
}

fn min_rule(report: &solidcraft_dfm_rules::Report) -> &solidcraft_dfm_rules::Finding {
    report.findings.iter().find(|finding| finding.rules.iter().any(|rule| rule == "sm.hole_min_diameter")).expect("a min-diameter result")
}

#[test]
fn missing_thickness_blocks_the_numeric_rule() {
    let generic = profile("generic", Layer::Generic, vec![row("sm.hole_min_diameter", 0.75, None)]);
    let report = check(&input(generic, None, declared(None), hole(0.5, Some(10.0)))).expect("check");
    let finding = min_rule(&report);
    assert_eq!(finding.state, AppState::Unsupported);
    assert_eq!(finding.reason_code.as_deref(), Some("THICKNESS_NOT_DECLARED"));
    let gate = &report.findings[0];
    assert_eq!(gate.rules, vec!["sm.gate_sheet_thickness"]);
    assert_eq!(gate.state, AppState::Unsupported);
}

#[test]
fn declared_thickness_conflicting_with_the_model_fails_the_gate() {
    let generic = profile("generic", Layer::Generic, vec![row("sm.hole_min_diameter", 0.75, None)]);
    let mut facts = hole(0.5, Some(10.0));
    facts.paired_mm = Some(2.0);
    let report = check(&input(generic, None, declared(Some(1.5)), facts)).expect("check");
    let gate = &report.findings[0];
    assert_eq!(gate.status, RuleStatus::Violation);
    assert_eq!(gate.state, AppState::Failed);
    assert_eq!(min_rule(&report).reason_code.as_deref(), Some("THICKNESS_MISMATCH"));
}

#[test]
fn an_unimplemented_edge_fact_is_unsupported_not_zero() {
    let generic = profile("generic", Layer::Generic, vec![row("sm.hole_min_diameter", 0.75, None), row("sm.hole_to_edge", 2.25, None)]);
    let report = check(&input(generic, None, declared(Some(1.5)), hole(3.0, None))).expect("check");
    let edge = report.findings.iter().find(|finding| finding.rules.iter().any(|rule| rule == "sm.hole_to_edge")).expect("edge");
    assert_eq!(edge.state, AppState::Unsupported);
    assert_eq!(edge.reason_code.as_deref(), Some("HOLE_EDGE_UNCLEAR"));
}

#[test]
fn no_factory_number_is_unsupported_and_does_not_guess() {
    let generic = profile("generic", Layer::Generic, vec![row("sm.hole_to_edge", 2.25, None)]);
    let report = check(&input(generic, None, declared(Some(1.5)), hole(3.0, Some(10.0)))).expect("check");
    let finding = min_rule(&report);
    assert_eq!(finding.state, AppState::Unsupported);
    assert_eq!(finding.reason_code.as_deref(), Some("NO_SHOP_NUMBER"));
}

#[test]
fn a_factory_row_for_another_material_is_unsupported() {
    let mut only_6061 = row("sm.hole_min_diameter", 1.0, Some(Confirmation::FactoryConfirmed));
    only_6061.applies.material = vec!["6061-T6".into()];
    let shop = profile("shop", Layer::Factory, vec![only_6061]);
    let generic = profile("generic", Layer::Generic, vec![]);
    let report = check(&input(generic, Some(shop), declared(Some(1.5)), hole(3.0, Some(10.0)))).expect("check");
    let finding = min_rule(&report);
    assert_eq!(finding.state, AppState::Unsupported);
    assert_eq!(finding.reason_code.as_deref(), Some("COVERAGE_NOT_IN_PROFILE"));
}

#[test]
fn a_generic_number_only_advises() {
    let generic = profile("generic", Layer::Generic, vec![row("sm.hole_min_diameter", 0.75, None)]);
    let report = check(&input(generic, None, declared(Some(1.5)), hole(0.5, Some(10.0)))).expect("check");
    let finding = min_rule(&report);
    assert_eq!(finding.status, RuleStatus::NeedsReview);
    assert_eq!(finding.state, AppState::Advisory);
    assert_eq!(finding.requirement.as_ref().expect("requirement").layer, Layer::Generic);
}

#[test]
fn a_confirmed_factory_number_can_fail() {
    let shop = profile("shop", Layer::Factory, vec![row("sm.hole_min_diameter", 1.0, Some(Confirmation::FactoryConfirmed))]);
    let generic = profile("generic", Layer::Generic, vec![]);
    let report = check(&input(generic, Some(shop), declared(Some(1.5)), hole(0.5, Some(10.0)))).expect("check");
    let finding = min_rule(&report);
    assert_eq!(finding.status, RuleStatus::NeedsReview);
    assert_eq!(finding.state, AppState::Failed);
}

#[test]
fn an_unconfirmed_factory_row_still_only_advises() {
    let shop = profile("shop", Layer::Factory, vec![row("sm.hole_min_diameter", 1.0, None)]);
    let generic = profile("generic", Layer::Generic, vec![]);
    let report = check(&input(generic, Some(shop), declared(Some(1.5)), hole(0.5, Some(10.0)))).expect("check");
    assert_eq!(min_rule(&report).state, AppState::Advisory);
}

#[test]
fn a_project_override_does_not_inherit_a_factory_confirmation() {
    let shop = profile("shop", Layer::Factory, vec![row("sm.hole_min_diameter", 1.0, Some(Confirmation::FactoryConfirmed))]);
    let generic = profile("generic", Layer::Generic, vec![]);
    let mut value = BTreeMap::new();
    value.insert("mm".to_string(), 1.0);
    let over = Override {
        rule: "sm.hole_min_diameter".into(),
        limit: Limit::Min,
        value,
        reason: "customer drawing".into(),
        by: "Zhang".into(),
        date: "2026-11-12".into(),
    };
    let mut check_input = input(generic, Some(shop), declared(Some(1.5)), hole(0.5, Some(10.0)));
    check_input.overrides = vec![over];
    let report = check(&check_input).expect("check");
    let finding = min_rule(&report);
    assert_eq!(finding.state, AppState::Advisory, "a project number is team-reported");
    let requirement = finding.requirement.as_ref().expect("requirement");
    assert_eq!(requirement.layer, Layer::Project);
    assert_eq!(requirement.confirmation, Some(Confirmation::TeamSelfReported));
    assert!(requirement.overridden);
    assert_eq!(report.data.confirmed_rows, 0, "the replaced factory row's confirmation is not used");
}

#[test]
fn an_override_without_a_reason_is_refused() {
    let generic = profile("generic", Layer::Generic, vec![row("sm.hole_min_diameter", 0.75, None)]);
    let mut check_input = input(generic, None, declared(Some(1.5)), hole(0.5, Some(10.0)));
    check_input.overrides = vec![Override {
        rule: "sm.hole_min_diameter".into(),
        limit: Limit::Min,
        value: BTreeMap::from([("mm".to_string(), 1.0)]),
        reason: " ".into(),
        by: "Zhang".into(),
        date: String::new(),
    }];
    assert!(check(&check_input).is_err());
}

#[test]
fn a_boundary_value_is_checked_and_never_called_passed() {
    let generic = profile("generic", Layer::Generic, vec![row("sm.hole_min_diameter", 0.75, None)]);
    let report = check(&input(generic, None, declared(Some(1.5)), hole(0.75, Some(10.0)))).expect("check");
    let finding = min_rule(&report);
    assert_eq!(finding.status, RuleStatus::NotVerified);
    assert_eq!(finding.state, AppState::Checked);
}

#[test]
fn a_met_bend_gap_uses_the_bends_radius_and_is_only_checked() {
    use solidcraft_dfm_rules::{BendFact, BendGap, Recognition};

    let generic = profile("generic", Layer::Generic, vec![row("sm.hole_to_bend", 0.0, None).with_bend_value()]);
    let mut facts = hole(3.0, Some(10.0));
    if let Some(holes) = &mut facts.holes {
        holes[0].bend_gaps = vec![BendGap { bend: 1, gap_mm: 10.0, inside_radius_mm: 1.0 }];
    }
    facts.recognition = Some(Recognition {
        bends: vec![BendFact {
            inside_radius_mm: 1.0,
            angle_deg: 90.0,
            length_mm: 20.0,
            x_mm: 0.0,
            y_mm: 0.0,
            z_mm: 0.0,
            flats: vec![None, Some(0)],
            ..Default::default()
        }],
        ..Recognition::default()
    });
    let report = check(&input(generic, None, declared(Some(1.5)), facts)).expect("check");
    let finding = report.findings.iter().find(|finding| finding.rules.iter().any(|rule| rule == "sm.hole_to_bend")).expect("bend");
    assert_eq!(finding.status, RuleStatus::NotVerified);
    assert_eq!(finding.reason_code.as_deref(), Some("ABOVE_PUBLIC_VALUE"));
    assert_eq!(finding.state, AppState::Checked);
    assert!((finding.requirement.as_ref().expect("requirement").value_mm - 4.75).abs() < 1e-9);
}

trait BendValue {
    fn with_bend_value(self) -> Self;
}

impl BendValue for Requirement {
    fn with_bend_value(mut self) -> Self {
        self.value = BTreeMap::from([("t".to_string(), 2.5), ("r".to_string(), 1.0)]);
        self.inputs = BTreeMap::from([("t".to_string(), "mm".to_string()), ("r".to_string(), "mm".to_string())]);
        self
    }
}

fn catalog_rule_state(report: &solidcraft_dfm_rules::Report, rule: &str) -> AppState {
    report.rules.iter().find(|result| result.rule_id == rule).unwrap_or_else(|| panic!("{rule} is in the catalog")).state
}

/// A pass summary carries no location and sorts first; it must never hide a failure at a real
/// location in the same rule.
#[test]
fn a_pass_and_a_failure_in_one_rule_summarize_as_failed() {
    let shop = profile("shop", Layer::Factory, vec![row("sm.hole_min_diameter", 1.0, Some(Confirmation::FactoryConfirmed))]);
    let generic = profile("generic", Layer::Generic, vec![]);
    // The first hole meets the shop's 1 mm, the second does not.
    let facts = holes_at(&[(5.0, 12.5, 3.0, Some(10.0)), (20.0, 12.5, 0.5, Some(10.0))]);
    let report = check_catalog(&input(generic, Some(shop), declared(Some(1.5)), facts)).expect("catalog");
    assert_eq!(catalog_rule_state(&report, "sm.hole_min_diameter"), AppState::Failed);
    assert!(report.counts.failed > 0);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| { finding.rules.iter().any(|rule| rule == "sm.hole_min_diameter") && finding.state == AppState::Passed })
    );
}

/// The same summary through the first-hole entry, so both report paths share one aggregation.
#[test]
fn the_first_hole_entry_also_summarizes_a_mixed_rule_as_failed() {
    let shop = profile("shop", Layer::Factory, vec![row("sm.hole_min_diameter", 1.0, Some(Confirmation::FactoryConfirmed))]);
    let generic = profile("generic", Layer::Generic, vec![]);
    let facts = holes_at(&[(5.0, 12.5, 3.0, Some(10.0)), (20.0, 12.5, 0.5, Some(10.0))]);
    let report = check(&input(generic, Some(shop), declared(Some(1.5)), facts)).expect("check");
    assert_eq!(catalog_rule_state(&report, "sm.hole_min_diameter"), AppState::Failed);
}

/// A passed hole and an unchecked hole must not read as a passed rule.
#[test]
fn a_pass_and_an_unchecked_edge_summarize_as_unsupported() {
    let shop = profile("shop", Layer::Factory, vec![row("sm.hole_to_edge", 2.0, Some(Confirmation::FactoryConfirmed))]);
    let generic = profile("generic", Layer::Generic, vec![]);
    let facts = holes_at(&[(10.0, 12.5, 3.0, Some(10.0)), (20.0, 12.5, 3.0, None)]);
    let report = check_catalog(&input(generic, Some(shop), declared(Some(1.5)), facts)).expect("catalog");
    assert_eq!(catalog_rule_state(&report, "sm.hole_to_edge"), AppState::Unsupported);
    assert!(report.counts.unsupported > 0);
}

/// Neither the input order nor a whole-model translation may change the rule summary.
#[test]
fn a_mixed_rule_keeps_its_state_when_holes_are_reordered_or_moved() {
    let shop = profile("shop", Layer::Factory, vec![row("sm.hole_min_diameter", 1.0, Some(Confirmation::FactoryConfirmed))]);
    let generic = profile("generic", Layer::Generic, vec![]);
    let points = [(5.0, 12.5, 3.0, Some(10.0)), (20.0, 12.5, 0.5, Some(10.0))];
    let reordered = [(20.0, 12.5, 0.5, Some(10.0)), (5.0, 12.5, 3.0, Some(10.0))];
    let moved = [(105.0, 112.5, 3.0, Some(10.0)), (120.0, 112.5, 0.5, Some(10.0))];
    for holes in [points, reordered, moved] {
        let report = check_catalog(&input(generic.clone(), Some(shop.clone()), declared(Some(1.5)), holes_at(&holes))).expect("catalog");
        assert_eq!(catalog_rule_state(&report, "sm.hole_min_diameter"), AppState::Failed, "{holes:?}");
    }
}

#[test]
fn a_factory_row_can_reach_the_rule_layers_pass_without_becoming_a_product_pass() {
    let shop = profile("shop", Layer::Factory, vec![row("sm.hole_min_diameter", 1.0, None)]);
    let generic = profile("generic", Layer::Generic, vec![]);
    let report = check(&input(generic, Some(shop), declared(Some(1.5)), hole(3.0, Some(10.0)))).expect("check");
    let finding = min_rule(&report);
    // INTdfm's rule layer decides on the layer alone; FD-7 only changes the App state.
    assert_eq!(finding.status, RuleStatus::Pass);
    assert_eq!(finding.state, AppState::Checked);
    assert_eq!(finding.requirement.as_ref().expect("requirement").layer, Layer::Factory);
    assert_eq!(finding.requirement.as_ref().expect("requirement").confirmation, None);
}
