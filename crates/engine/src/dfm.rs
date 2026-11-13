//! The App-side DFM entry: native sheet facts, the pure rule crate, one structured report.
//!
//! No OCP, no network and no files. The live document is only read through [`SheetBody`]; the
//! rule crate decides on the facts and data it is given.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use solidcraft_dfm_rules::{
    BendFact as RuleBend, BendGap as RuleGap, CheckInput, Declared, EdgeKind as RuleEdgeKind, Facts as RuleFacts, HoleFact as RuleHole, ModelRef,
    Override, Profile, Recognition as RuleRecognition, Report,
};
use solidcraft_doc::sheet::SheetBody;
use solidcraft_doc::sheet_dfm;

/// What can stop the App entry before a report exists.
#[derive(Debug, thiserror::Error)]
pub enum DfmError {
    #[error("sheet facts: {0}")]
    Facts(String),
    #[error("rule layer: {0}")]
    Rules(String),
    #[error("parameters: {0}")]
    Parameters(String),
    /// The sheet cannot be checked, but that is not an input error: the caller shows "not covered".
    #[error("{0}")]
    Unsupported(String),
}

/// The facts the App measured, kept so the caller can store them beside the report.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DfmOutcome {
    pub facts: sheet_dfm::SheetFacts,
    pub report: Report,
}

/// The parameters of one `dfm.sheet` check. They are frozen in the validation profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SheetCheckParameters {
    /// The sheet's index in the evaluated model, so a renamed body cannot select another one.
    pub sheet: usize,
    /// The body name, checked against the index; a mismatch is an error, not a silent pick.
    pub body: String,
    pub declared: Declared,
    pub generic: Profile,
    #[serde(default)]
    pub shop: Option<Profile>,
    #[serde(default)]
    pub overrides: Vec<Override>,
    #[serde(default)]
    pub model: ModelRef,
    /// The selector the App used, for the report: `native` or `converted <digest>`.
    #[serde(default)]
    pub source: String,
}

impl SheetCheckParameters {
    fn into_input(self, facts: RuleFacts) -> CheckInput {
        CheckInput {
            declared: self.declared,
            generic: self.generic,
            shop: self.shop,
            overrides: self.overrides,
            facts,
            model: ModelRef {
                model_version: self.model.model_version,
                body: self.model.body,
                source: if self.model.source.is_empty() { self.source } else { self.model.source },
            },
        }
    }
}

/// Parse and check the parameters before a run starts. A bad profile never reaches a report.
pub fn validate_parameters(value: &Value) -> Result<SheetCheckParameters, DfmError> {
    let parameters: SheetCheckParameters = serde_json::from_value(value.clone()).map_err(|error| DfmError::Parameters(error.to_string()))?;
    if parameters.body.trim().is_empty() {
        return Err(DfmError::Parameters("body must name the sheet metal body".into()));
    }
    if parameters.model.model_version.trim().is_empty() {
        return Err(DfmError::Parameters("the model version must be recorded".into()));
    }
    if parameters.sheet > 1024 {
        return Err(DfmError::Parameters("sheet index is out of range".into()));
    }
    let probe = parameters.clone().into_input(RuleFacts::default());
    solidcraft_dfm_rules::validate(&probe).map_err(|error| DfmError::Parameters(error.to_string()))?;
    Ok(parameters)
}

/// Convert the App's measured facts to the rule crate's data contract.
///
/// A hole with no assigned flat is marked unusable: its edge and bend facts cannot be trusted.
/// A bore is not a through hole; its count is added to the openings the rule layer says were not
/// checked, so it can never read as zero risk.
pub fn rule_facts(measured: &sheet_dfm::SheetFacts) -> RuleFacts {
    let holes = measured.holes.iter().map(|hole| RuleHole {
        diameter_mm: hole.diameter_mm,
        x_mm: hole.x_mm,
        y_mm: hole.y_mm,
        z_mm: hole.z_mm,
        depth_mm: hole.depth_mm,
        square: hole.square && hole.flat_assigned,
        edge_mm: if hole.flat_assigned { hole.edge_mm } else { None },
        edge_at_mm: if hole.flat_assigned { hole.edge_at_mm.map(|point| point.to_vec()) } else { None },
        edge_kind: hole.edge_kind.map(|kind| match kind {
            sheet_dfm::EdgeKind::Outline => RuleEdgeKind::Outline,
            sheet_dfm::EdgeKind::Opening => RuleEdgeKind::Opening,
        }),
        edge_hole: hole.edge_hole,
        bend_gaps: if hole.flat_assigned {
            hole.bend_gaps.iter().map(|gap| RuleGap { bend: gap.bend, gap_mm: gap.gap_mm, inside_radius_mm: gap.inside_radius_mm }).collect()
        } else {
            Vec::new()
        },
        flat: hole.flat,
    });
    RuleFacts {
        volume_mm3: None,
        size_mm: None,
        area_mm2: None,
        paired_mm: Some(measured.thickness_mm),
        holes: Some(holes.collect()),
        other_openings: measured.other_openings.saturating_add(measured.bores),
        unknown_faces: 0,
        recognition: Some(RuleRecognition {
            bends: measured
                .bends
                .iter()
                .map(|bend| RuleBend {
                    inside_radius_mm: bend.inside_radius_mm,
                    angle_deg: bend.angle_deg,
                    length_mm: bend.length_mm,
                    x_mm: bend.x_mm,
                    y_mm: bend.y_mm,
                    z_mm: bend.z_mm,
                    flats: bend.flats.to_vec(),
                })
                .collect(),
            unknown_faces: 0,
            cone_faces: 0,
            form_faces: 0,
        }),
    }
}

/// Measure one native sheet body and run the rule layer on it.
pub fn check_sheet(sheet: &SheetBody, parameters: SheetCheckParameters) -> Result<DfmOutcome, DfmError> {
    let facts = sheet_dfm::facts(sheet).map_err(|error| DfmError::Facts(error.to_string()))?;
    if let Some(feature) = &facts.altered_by {
        return Err(DfmError::Unsupported(format!(
            "the sheet metal body was changed after its sheet-metal features by {feature}, so it cannot be unfolded and the hole rules were not run"
        )));
    }
    if sheet.body != parameters.body {
        return Err(DfmError::Parameters(format!("sheet {} is `{}`, not `{}`", parameters.sheet, sheet.body, parameters.body)));
    }
    let rules = rule_facts(&facts);
    let report = solidcraft_dfm_rules::check(&parameters.into_input(rules)).map_err(|error| DfmError::Rules(error.to_string()))?;
    Ok(DfmOutcome { facts, report })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use solidcore_record::{Loop2, Plane, Region2, Vec2, Vec3};
    use solidcraft_dfm_rules::{Confirmation, Layer, Limit, Requirement};
    use solidcraft_doc::sheet::{AngleDisplay, ReliefShape, SheetBody, SheetFlange};
    use solidcraft_geom::PlaneGeom;

    fn plate_with_hole(diameter: f64) -> SheetBody {
        SheetBody {
            body: "Plate".into(),
            rule: "Steel (mm)".into(),
            t: 1.5,
            k: 0.44,
            frame: Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y).expect("XY plane"),
            base: Region2 {
                outer: Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0), Vec2::new(40.0, 25.0), Vec2::new(0.0, 25.0)]),
                holes: Vec::new(),
            },
            flanges: Vec::<SheetFlange>::new(),
            holes: vec![Region2 { outer: Loop2::circle(Vec2::new(20.0, 12.5), diameter / 2.0), holes: Vec::new() }],
            pockets: Vec::new(),
            bores: Vec::new(),
            perforations: Vec::new(),
            labels: Vec::new(),
            flat: false,
            gap: 0.0,
            relief: ReliefShape::Notch,
            relief_size: 0.0,
            bend_relief: (0.0, 0.0),
            angle_display: AngleDisplay::Bend,
            base_curves: Vec::new(),
            altered_by: None,
        }
    }

    fn requirement(rule: &str, value: f64, confirmation: Option<Confirmation>) -> Requirement {
        Requirement {
            rule: rule.into(),
            limit: Limit::Min,
            value: BTreeMap::from([("mm".to_string(), value)]),
            inputs: BTreeMap::new(),
            measuring_line: "TEST-ONLY".into(),
            method: "TEST-ONLY".into(),
            source: "TEST-ONLY".into(),
            rights: "TEST-ONLY".into(),
            applies: Default::default(),
            exceptions: Vec::new(),
            validation_samples: Vec::new(),
            die: None,
            bends: None,
            confirmation,
        }
    }

    fn parameters(rows: Vec<Requirement>) -> SheetCheckParameters {
        SheetCheckParameters {
            sheet: 0,
            body: "Plate".into(),
            declared: Declared {
                process: Some("press brake".into()),
                material: Some("5052-H32".into()),
                thickness_mm: Some(1.5),
                thickness_from: Some("declared".into()),
            },
            generic: Profile { format: 1, name: "generic".into(), version: "TEST-ONLY".into(), layer: Layer::Generic, requirements: rows },
            shop: None,
            overrides: Vec::new(),
            model: ModelRef { model_version: "doc-7".into(), body: "Plate".into(), source: "native".into() },
            source: "native".into(),
        }
    }

    #[test]
    fn a_native_sheet_reaches_a_structured_report() {
        let sheet = plate_with_hole(0.5);
        let outcome = check_sheet(&sheet, parameters(vec![requirement("sm.hole_min_diameter", 0.75, None)])).expect("check");
        let finding =
            outcome.report.findings.iter().find(|finding| finding.rules.iter().any(|rule| rule == "sm.hole_min_diameter")).expect("a hole result");
        assert_eq!(finding.state, solidcraft_dfm_rules::AppState::Advisory);
        assert_eq!(finding.measured_mm, Some(0.5));
        assert_eq!(outcome.report.model.model_version, "doc-7");
        assert_eq!(outcome.facts.holes.len(), 1);
    }

    #[test]
    fn a_confirmed_factory_number_can_fail_native_facts() {
        let sheet = plate_with_hole(0.5);
        let mut params = parameters(Vec::new());
        params.shop = Some(Profile {
            format: 1,
            name: "shop".into(),
            version: "TEST-ONLY".into(),
            layer: Layer::Factory,
            requirements: vec![requirement("sm.hole_min_diameter", 1.0, Some(Confirmation::FactoryConfirmed))],
        });
        let outcome = check_sheet(&sheet, params).expect("check");
        let finding =
            outcome.report.findings.iter().find(|finding| finding.rules.iter().any(|rule| rule == "sm.hole_min_diameter")).expect("a hole result");
        assert_eq!(finding.state, solidcraft_dfm_rules::AppState::Failed);
    }

    #[test]
    fn a_native_sheet_design_reaches_the_rule_report() {
        let mut session = crate::Session::default();
        session.execute("sketch.create", &serde_json::json!({"plane": "XY", "name": "Base"})).expect("sketch");
        session.execute("sketch.rectangle.two_point", &serde_json::json!({"p0": [0, 0], "p1": [40, 25]})).expect("rectangle");
        session.execute("sketch.finish", &serde_json::json!({})).expect("finish");
        session.execute("sheet.flange", &serde_json::json!({"sketch": "Base"})).expect("base flange");
        session.execute("sheet.perforate", &serde_json::json!({"diameter": "0.5", "pitch": "100", "margin": "1"})).expect("perforation");
        let state = session.model.state();
        let sheet = state.sheets.first().expect("one sheet body");
        let mut params = parameters(vec![requirement("sm.hole_min_diameter", 0.75, None)]);
        params.body = sheet.body.clone();
        // The test rule's sheet is 2.5 mm thick; the declared value must agree with the model.
        params.declared.thickness_mm = Some(2.5);
        let outcome = check_sheet(sheet, params).expect("facts and report");
        assert!(!outcome.facts.holes.is_empty(), "the perforation is a round through hole");
        let finding =
            outcome.report.findings.iter().find(|finding| finding.rules.iter().any(|rule| rule == "sm.hole_min_diameter")).expect("a hole result");
        assert_eq!(finding.state, solidcraft_dfm_rules::AppState::Advisory);
        assert!(finding.location.is_some(), "the finding points at the model");
    }

    #[test]
    fn the_second_gate_compares_app_facts_with_the_recorded_facts() {
        let recording: serde_json::Value =
            serde_json::from_str(include_str!("../../dfm-rules/tests/fixtures/intdfm/hole_min_diameter.below.json")).expect("recorded case");
        let recorded: solidcraft_dfm_rules::Facts = serde_json::from_value(recording["facts"].clone()).expect("recorded facts");
        let sheet = plate_with_hole(0.5);
        let measured = rule_facts(&sheet_dfm::facts(&sheet).expect("measured facts"));
        // The tolerance is the one the recording's contract pins, not a test-local default.
        let contract = &recording["comparison"];
        let tolerance = solidcraft_dfm_rules::second_gate::GeometryTolerance {
            pairing_mm: contract["pairing"]["max_centre_distance_mm"].as_f64().expect("pairing"),
            ambiguous_mm: contract["pairing"]["ambiguous_within_mm"].as_f64().expect("ambiguous"),
            measurement_mm: contract["measurement_abs_tolerance_mm"].as_f64().expect("measurement"),
        };
        assert_eq!(tolerance.pairing_mm, 0.05);
        assert_eq!(tolerance.ambiguous_mm, 0.001);
        assert_eq!(tolerance.measurement_mm, 0.001);
        let report = solidcraft_dfm_rules::second_gate::compare(&recorded, &measured, tolerance).expect("the same hole");
        assert_eq!(report.pairs, 1);
        assert!(report.max_diameter_delta_mm <= tolerance.measurement_mm);

        let mut changed = measured.clone();
        if let Some(holes) = &mut changed.holes {
            holes[0].diameter_mm += 0.01;
        }
        let mismatch = solidcraft_dfm_rules::second_gate::compare(&recorded, &changed, tolerance).expect_err("a diameter difference must fail");
        assert!(matches!(mismatch, solidcraft_dfm_rules::second_gate::GeometryMismatch::Diameter { .. }), "{mismatch}");

        let mut moved = measured.clone();
        if let Some(holes) = &mut moved.holes {
            holes[0].x_mm += 0.02;
        }
        let mismatch = solidcraft_dfm_rules::second_gate::compare(&recorded, &moved, tolerance).expect_err("a moved hole must fail");
        assert!(matches!(mismatch, solidcraft_dfm_rules::second_gate::GeometryMismatch::Position { .. }), "{mismatch}");

        let mut missing = measured;
        missing.holes = Some(Vec::new());
        let mismatch = solidcraft_dfm_rules::second_gate::compare(&recorded, &missing, tolerance).expect_err("a lost hole must fail");
        assert!(matches!(mismatch, solidcraft_dfm_rules::second_gate::GeometryMismatch::HoleCount { .. }), "{mismatch}");
    }

    #[test]
    fn a_wrong_body_name_is_refused() {
        let sheet = plate_with_hole(0.5);
        let mut params = parameters(vec![requirement("sm.hole_min_diameter", 0.75, None)]);
        params.body = "Other".into();
        assert!(matches!(check_sheet(&sheet, params), Err(DfmError::Parameters(_))));
    }
}
