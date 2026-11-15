//! The App-side DFM entry: native sheet facts, the pure rule crate, one structured report.
//!
//! No OCP, no network and no files. The live document is only read through [`SheetBody`]; the
//! rule crate decides on the facts and data it is given.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use solidcraft_dfm_rules::{
    BendFact as RuleBend, BendGap as RuleGap, CheckInput, CountersinkFact as RuleCountersink, Declared, EdgeKind as RuleEdgeKind, FactFamily,
    Facts as RuleFacts, HoleFact as RuleHole, ModelRef, Override, Profile, Recognition as RuleRecognition, Report,
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
            critical: Vec::new(),
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

/// The rule layer has no "base" sentinel: its `None` means an unknown flat, and a hole on it is
/// left out of `sm.hole_spacing`. The App's `None` means the base panel, so the adapter gives the
/// base an explicit id and shifts flange ids past it.
fn rule_flat(flat: Option<usize>, assigned: bool) -> Option<usize> {
    assigned.then(|| rule_side_flat(flat))
}

/// A panel's rule-side flat id: the base is 1, flange `i` is `i + 2`. Bends use the same ids.
fn rule_side_flat(flat: Option<usize>) -> usize {
    flat.map_or(1, |index| index.saturating_add(2))
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
        edge_kind: hole.edge_kind.map(rule_edge_kind),
        edge_hole: hole.edge_hole,
        bend_gaps: if hole.flat_assigned { rule_gaps(&hole.bend_gaps) } else { Vec::new() },
        flat: rule_flat(hole.flat, hole.flat_assigned),
        countersink: if hole.flat_assigned { hole.countersink.as_ref().map(rule_countersink) } else { None },
    });
    RuleFacts {
        volume_mm3: None,
        size_mm: None,
        area_mm2: None,
        paired_mm: Some(measured.thickness_mm),
        plate_gap_mm: Some(measured.thickness_mm),
        holes: Some(holes.collect()),
        other_openings: measured.other_openings.saturating_add(measured.bores),
        unknown_faces: 0,
        missing: vec![FactFamily::BendRuns, FactFamily::ReliefEnds, FactFamily::ReliefSizes, FactFamily::Webs],
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
                    flats: bend.flats.iter().map(|flat| Some(rule_side_flat(*flat))).collect(),
                    ..Default::default()
                })
                .collect(),
            unknown_faces: 0,
            cone_faces: 0,
            form_faces: 0,
            // Every recorded countersink counts here, whether or not it sits on a read hole: a
            // count above the holes that carry one is unread, never "no countersink".
            countersinks: measured.countersinks,
            joined: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn rule_edge_kind(kind: sheet_dfm::EdgeKind) -> RuleEdgeKind {
    match kind {
        sheet_dfm::EdgeKind::Outline => RuleEdgeKind::Outline,
        sheet_dfm::EdgeKind::Opening => RuleEdgeKind::Opening,
    }
}

fn rule_gaps(gaps: &[sheet_dfm::BendGapFact]) -> Vec<RuleGap> {
    gaps.iter().map(|gap| RuleGap { bend: gap.bend, gap_mm: gap.gap_mm, inside_radius_mm: gap.inside_radius_mm }).collect()
}

/// A measured countersink as the rule layer reads it. The App does not yet own a hole wall with
/// its own countersink, so `edge_hole` and `edge_rim` stay empty and false.
fn rule_countersink(sink: &sheet_dfm::CountersinkFact) -> RuleCountersink {
    RuleCountersink {
        major_diameter_mm: sink.major_diameter_mm,
        depth_mm: sink.depth_mm,
        angle_deg: sink.angle_deg,
        land_mm: sink.land_mm,
        x_mm: sink.x_mm,
        y_mm: sink.y_mm,
        z_mm: sink.z_mm,
        edge_mm: sink.edge_mm,
        edge_at_mm: sink.edge_at_mm.map(|point| point.to_vec()),
        edge_kind: sink.edge_kind.map(rule_edge_kind),
        edge_hole: None,
        edge_rim: false,
        bend_gaps: rule_gaps(&sink.bend_gaps),
    }
}

/// Compare two INTdfm-shaped reports with the migrated recheck layer. This is the production
/// entry for a later automatic-recheck caller; it does not need OCP or a file.
pub fn recheck_reports(
    earlier: &solidcraft_dfm_rules::intdfm_report::Report,
    later: &solidcraft_dfm_rules::intdfm_report::Report,
) -> Result<solidcraft_dfm_rules::intdfm_report::Recheck, DfmError> {
    solidcraft_dfm_rules::recheck::compare(earlier, later).map_err(|error| DfmError::Rules(error.to_string()))
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
    let report = solidcraft_dfm_rules::check_catalog(&parameters.into_input(rules)).map_err(|error| DfmError::Rules(error.to_string()))?;
    Ok(DfmOutcome { facts, report })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};
    use solidcraft_dfm_rules::second_gate::{
        COMPARISON_QUANTUM_MM, GeometryMismatch, GeometryTolerance, HoleWallFact, MeasuredHole, MeasuredSheet, compare,
    };
    use solidcraft_dfm_rules::{Confirmation, EdgeKind as RuleEdgeKind, Facts as RuleFacts, Layer, Limit, Requirement};
    use solidcraft_doc::sheet::{AngleDisplay, ReliefShape, SheetBody, SheetFlange};
    use solidcraft_geom::PlaneGeom;
    use std::f64::consts::{FRAC_PI_2, PI};

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
            countersinks: Vec::new(),
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

    fn empty_sheet(base: Region2) -> SheetBody {
        SheetBody {
            body: "Plate".into(),
            rule: "Steel (mm)".into(),
            t: 1.5,
            k: 0.44,
            frame: Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y).expect("XY plane"),
            base,
            flanges: Vec::<SheetFlange>::new(),
            holes: Vec::new(),
            pockets: Vec::new(),
            countersinks: Vec::new(),
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

    fn circle(x: f64, y: f64, radius: f64) -> Region2 {
        Region2 { outer: Loop2::circle(Vec2::new(x, y), radius), holes: Vec::new() }
    }

    fn plate_with_holes(width: f64, height: f64, holes: &[(f64, f64, f64)]) -> SheetBody {
        let mut sheet = empty_sheet(Region2 {
            outer: Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(width, 0.0), Vec2::new(width, height), Vec2::new(0.0, height)]),
            holes: Vec::new(),
        });
        sheet.holes = holes.iter().map(|(x, y, radius)| circle(*x, *y, *radius)).collect();
        sheet
    }

    fn flange(parent: Option<usize>, p: Vec2, d: Vec2, n: Vec2, len: f64, radius: f64, leg: f64) -> SheetFlange {
        SheetFlange {
            parent,
            p,
            d,
            len,
            n,
            angle: FRAC_PI_2,
            radius,
            leg,
            ext: (0.0, 0.0),
            slope: (0.0, 0.0),
            tear: (false, false),
            shape: None,
            exact: None,
            zone_shape: None,
            tag: String::new(),
        }
    }

    /// The recorded `hole_to_bend.below` part: an L whose long leg is the base, with the bend at
    /// x = 4.5 and the hole 2.5 mm from its tangent. The base polygon starts at the far top edge
    /// so the tie between y = 0 and y = 25 resolves to the recorded y = 25 edge.
    fn l_bracket_sheet() -> SheetBody {
        let mut sheet = empty_sheet(Region2 {
            outer: Loop2::polygon(&[Vec2::new(44.5, 25.0), Vec2::new(4.5, 25.0), Vec2::new(4.5, 0.0), Vec2::new(44.5, 0.0)]),
            holes: Vec::new(),
        });
        sheet.flanges.push(flange(None, Vec2::new(4.5, 0.0), Vec2::Y, Vec2::new(-1.0, 0.0), 25.0, 3.0, 30.0));
        sheet.holes.push(circle(8.5, 12.5, 1.5));
        sheet
    }

    /// The recorded `hole_to_bend.u_holes` part: base 30 x 25, one leg hole and one base hole.
    /// `reversed` lists the right flange first so the measured bend numbering differs from the
    /// recording and the gate must map by position.
    fn u_holes_sheet(reversed: bool) -> SheetBody {
        let mut sheet = empty_sheet(Region2 {
            outer: Loop2::polygon(&[Vec2::new(30.0, 25.0), Vec2::new(0.0, 25.0), Vec2::new(0.0, 0.0), Vec2::new(30.0, 0.0)]),
            holes: Vec::new(),
        });
        let left = flange(None, Vec2::new(0.0, 0.0), Vec2::Y, Vec2::new(-1.0, 0.0), 25.0, 3.0, 20.0);
        let right = flange(None, Vec2::new(30.0, 0.0), Vec2::Y, Vec2::X, 25.0, 3.0, 20.0);
        let left_index = if reversed {
            sheet.flanges.push(right);
            sheet.flanges.push(left);
            1
        } else {
            sheet.flanges.push(left);
            sheet.flanges.push(right);
            0
        };
        // The recorded leg hole is 10.5 mm up from the near tangent; the flat panel starts at the
        // bend allowance.
        let allowance = sheet.allowance(&sheet.flanges[left_index]);
        sheet.holes.push(circle(-allowance - 10.5, 12.5, 1.5));
        sheet.holes.push(circle(8.0, 12.5, 1.5));
        sheet
    }

    /// The recorded `hole_to_edge.opening` part: a round hole 2 mm from a slot's end cap.
    fn slot_sheet() -> SheetBody {
        let mut sheet = plate_with_holes(40.0, 25.0, &[(28.5, 12.5, 1.5)]);
        let slot = Loop2 {
            segs: vec![
                Seg2::Line { a: Vec2::new(17.0, 10.5), b: Vec2::new(23.0, 10.5) },
                Seg2::Arc { center: Vec2::new(23.0, 12.5), radius: 2.0, start: -FRAC_PI_2, sweep: PI },
                Seg2::Line { a: Vec2::new(23.0, 14.5), b: Vec2::new(17.0, 14.5) },
                Seg2::Arc { center: Vec2::new(17.0, 12.5), radius: 2.0, start: FRAC_PI_2, sweep: PI },
            ],
        };
        sheet.holes.push(Region2 { outer: slot, holes: Vec::new() });
        sheet
    }

    fn panel_normal(sheet: &SheetBody, panel: Option<usize>) -> [f64; 3] {
        // `Mat` is column-major: column 2 is the image of the sheet's normal.
        let world = sheet.world(panel);
        [world[2][0], world[2][1], world[2][2]]
    }

    fn measured_sheet(sheet: &SheetBody, facts: &sheet_dfm::SheetFacts) -> MeasuredSheet {
        let rules = rule_facts(facts);
        let holes = facts
            .holes
            .iter()
            .map(|hole| MeasuredHole {
                wall: hole.nearest_hole_wall_number.map(|number| HoleWallFact {
                    number,
                    gap_mm: hole.nearest_hole_wall_mm.unwrap_or(0.0),
                    at_mm: hole.nearest_hole_wall_at_mm.unwrap_or([0.0, 0.0, 0.0]),
                }),
                normal: hole.flat_assigned.then(|| panel_normal(sheet, hole.flat)),
            })
            .collect();
        MeasuredSheet { facts: rules, holes }
    }

    fn recording(text: &str) -> serde_json::Value {
        serde_json::from_str(text).expect("recorded case")
    }

    fn tolerance(recording: &serde_json::Value) -> GeometryTolerance {
        let contract = &recording["comparison"];
        assert_eq!(contract["pairing"]["method"], "exact_complete_matching");
        assert_eq!(contract["pairing"]["ambiguity"], "more_than_one_complete_matching");
        assert_eq!(contract["comparison_quantum_mm"].as_f64(), Some(COMPARISON_QUANTUM_MM));
        assert_eq!(contract["pairing"]["max_centre_distance_mm"].as_f64(), Some(0.05));
        assert_eq!(contract["measurement_abs_tolerance_mm"].as_f64(), Some(0.001));
        GeometryTolerance {
            pairing_mm: contract["pairing"]["max_centre_distance_mm"].as_f64().expect("pairing"),
            measurement_mm: contract["measurement_abs_tolerance_mm"].as_f64().expect("measurement"),
        }
    }

    fn recorded_facts(recording: &serde_json::Value) -> RuleFacts {
        serde_json::from_value(recording["facts"].clone()).expect("recorded facts")
    }

    fn rotate(angle: f64, point: Vec3) -> Vec3 {
        let (sin, cos) = angle.sin_cos();
        Vec3::new(cos * point.x - sin * point.y, sin * point.x + cos * point.y, point.z)
    }

    /// Transform the whole native sheet body by a rotation about Z and a translation.
    fn transform_sheet(mut sheet: SheetBody, angle: f64, translation: Vec3) -> SheetBody {
        let origin = rotate(angle, sheet.frame.origin) + translation;
        let x = rotate(angle, sheet.frame.x);
        let y = rotate(angle, sheet.frame.y);
        sheet.frame = Plane::new(origin, x, y).expect("rotated plane");
        sheet
    }

    fn inverse_point(point: [f64; 3], angle: f64, translation: Vec3) -> [f64; 3] {
        let moved = rotate(-angle, Vec3::new(point[0] - translation.x, point[1] - translation.y, point[2] - translation.z));
        [moved.x, moved.y, moved.z]
    }

    fn inverse_vector(vector: [f64; 3], angle: f64) -> [f64; 3] {
        let moved = rotate(-angle, Vec3::new(vector[0], vector[1], vector[2]));
        [moved.x, moved.y, moved.z]
    }

    /// Return measured facts to the recorded folded frame (the inverse of `transform_sheet`).
    fn inverse_transform(measured: &mut MeasuredSheet, angle: f64, translation: Vec3) {
        for hole in measured.facts.holes.iter_mut().flatten() {
            let point = inverse_point([hole.x_mm, hole.y_mm, hole.z_mm], angle, translation);
            hole.x_mm = point[0];
            hole.y_mm = point[1];
            hole.z_mm = point[2];
            if let Some(at) = &mut hole.edge_at_mm {
                let moved = inverse_point([at[0], at[1], at[2]], angle, translation);
                at.copy_from_slice(&moved);
            }
        }
        for bend in measured.facts.recognition.iter_mut().flat_map(|recognition| recognition.bends.iter_mut()) {
            let point = inverse_point([bend.x_mm, bend.y_mm, bend.z_mm], angle, translation);
            bend.x_mm = point[0];
            bend.y_mm = point[1];
            bend.z_mm = point[2];
        }
        for hole in &mut measured.holes {
            if let Some(wall) = &mut hole.wall {
                wall.at_mm = inverse_point(wall.at_mm, angle, translation);
            }
            if let Some(normal) = &mut hole.normal {
                *normal = inverse_vector(*normal, angle);
            }
        }
    }

    fn u_holes_case() -> (RuleFacts, MeasuredSheet, GeometryTolerance) {
        let recording = recording(include_str!("../../dfm-rules/tests/fixtures/intdfm/hole_to_bend.u_holes.json"));
        let recorded = recorded_facts(&recording);
        let tolerance = tolerance(&recording);
        let sheet = u_holes_sheet(true);
        let facts = sheet_dfm::facts(&sheet).expect("measured facts");
        let measured = measured_sheet(&sheet, &facts);
        (recorded, measured, tolerance)
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
            angles_deg: None,
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

    fn shop_parameters(rule: &str, value: f64) -> SheetCheckParameters {
        let mut params = parameters(Vec::new());
        params.shop = Some(Profile {
            format: 1,
            name: "shop".into(),
            version: "TEST-ONLY".into(),
            layer: Layer::Factory,
            requirements: vec![requirement(rule, value, Some(Confirmation::FactoryConfirmed))],
        });
        params
    }

    fn spacing_finding(report: &Report) -> &solidcraft_dfm_rules::Finding {
        report.findings.iter().find(|finding| finding.rules.iter().any(|rule| rule == "sm.hole_spacing")).expect("a spacing result")
    }

    /// The base panel is `flat = None` in the App. The rule layer's `None` means unknown, so the
    /// adapter must give the base an explicit id and keep both holes on it.
    #[test]
    fn base_panel_holes_keep_their_flat_and_reach_hole_spacing() {
        let sheet = plate_with_holes(40.0, 25.0, &[(10.0, 12.5, 1.5), (20.0, 12.5, 1.5)]);
        let facts = sheet_dfm::facts(&sheet).expect("facts");
        let rules = rule_facts(&facts);
        let holes = rules.holes.as_ref().expect("holes");
        assert_eq!(holes.iter().map(|hole| hole.flat).collect::<Vec<_>>(), vec![Some(1), Some(1)], "the base is an explicit flat, not unknown");

        let outcome = check_sheet(&sheet, shop_parameters("sm.hole_spacing", 8.0)).expect("check");
        let spacing = spacing_finding(&outcome.report);
        assert_eq!(spacing.state, solidcraft_dfm_rules::AppState::Failed);
        assert_eq!(spacing.measured_mm, Some(7.0), "10 mm centres less two 1.5 mm radii");
    }

    /// A flange hole and its bend must use the same rule-side id, and a hole on the base must not
    /// pair with a hole on a flange.
    #[test]
    fn flange_holes_share_the_bends_flat_and_do_not_pair_with_the_base() {
        let sheet = u_holes_sheet(true);
        let facts = sheet_dfm::facts(&sheet).expect("facts");
        let rules = rule_facts(&facts);
        let holes = rules.holes.as_ref().expect("holes");
        assert_eq!(holes.iter().map(|hole| hole.flat).collect::<Vec<_>>(), vec![Some(3), Some(1)]);
        let bends = &rules.recognition.as_ref().expect("recognition").bends;
        assert_eq!(bends[1].flats, vec![Some(1), Some(3)], "the left bend names the same flange flat as its hole");

        let outcome = check_sheet(&sheet, shop_parameters("sm.hole_spacing", 4.0)).expect("check");
        let spacing = spacing_finding(&outcome.report);
        assert_eq!(spacing.state, solidcraft_dfm_rules::AppState::Checked, "one hole per flat has no pair");
        assert!(spacing.reason_code.is_none());
        assert!(!spacing.message.contains("flat can't be told"), "{}", spacing.message);
    }

    /// Two holes on one flange are measured against each other; the base hole stays out of it.
    #[test]
    fn same_flange_holes_reach_hole_spacing() {
        let mut sheet = u_holes_sheet(true);
        let allowance = sheet.allowance(&sheet.flanges[1]);
        sheet.holes.push(circle(-allowance - 16.0, 12.5, 1.5));
        let outcome = check_sheet(&sheet, shop_parameters("sm.hole_spacing", 4.0)).expect("check");
        let spacing = spacing_finding(&outcome.report);
        assert_eq!(spacing.state, solidcraft_dfm_rules::AppState::Failed);
        assert_eq!(spacing.measured_mm, Some(2.5), "5.5 mm centres less two 1.5 mm radii");
    }

    /// An unassigned hole must stay unknown instead of being mistaken for a base hole.
    #[test]
    fn a_hole_with_an_unknown_flat_stays_unknown() {
        let mut sheet = u_holes_sheet(true);
        let allowance = sheet.allowance(&sheet.flanges[1]);
        sheet.holes.push(circle(-allowance / 2.0, 12.5, 1.5));
        let facts = sheet_dfm::facts(&sheet).expect("facts");
        let unknown = facts.holes.iter().position(|hole| !hole.flat_assigned).expect("a hole in the bend zone");
        let rules = rule_facts(&facts);
        let holes = rules.holes.as_ref().expect("holes");
        assert_eq!(holes[unknown].flat, None);
        assert!(!holes[unknown].square, "an unassigned hole is not a usable through hole");
        assert!(holes.iter().enumerate().any(|(index, hole)| index != unknown && hole.flat == Some(1)));
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
        assert!(outcome.report.rules.len() >= 15);
        // A plain through hole carries no countersink: the five countersink rules say so rather
        // than reporting that the App cannot measure countersinks at all.
        let sink = sink_finding(&outcome.report);
        assert_eq!(sink.status, solidcraft_dfm_rules::RuleStatus::NotApplicable);
        assert_eq!(sink.state, solidcraft_dfm_rules::AppState::Checked);
        assert!(sink.reason_code.is_none());
        for rule in solidcraft_dfm_rules::sink_rules::COUNTERSINK_RULES {
            assert!(sink.rules.iter().any(|found| found == rule), "{rule} is in the one finding");
        }
    }

    fn sink_finding(report: &Report) -> &solidcraft_dfm_rules::Finding {
        report.findings.iter().find(|finding| finding.rules.iter().any(|rule| rule == "sm.countersink_depth")).expect("a countersink result")
    }

    fn finding_for<'a>(report: &'a Report, rule: &str) -> &'a solidcraft_dfm_rules::Finding {
        report.findings.iter().find(|finding| finding.rules.iter().any(|found| found == rule)).unwrap_or_else(|| panic!("{rule} result"))
    }

    /// The five countersink rows of the shared TEST-ONLY full-catalogue shop profile
    /// (`examples/dfm/test-only-full-catalogue-shop.json`, another branch): depth max 1.0 mm,
    /// rim to edge and to bend min 3.0 mm, land min 0.5 mm, and within 1° of a 90° tool. A
    /// factory-confirmed row decides, so a miss is `failed` rather than an advisory.
    fn sink_parameters() -> SheetCheckParameters {
        let millimetre = |rule: &str, limit: Limit, value: f64| {
            let mut row = requirement(rule, value, Some(Confirmation::FactoryConfirmed));
            row.limit = limit;
            row.inputs = BTreeMap::from([("mm".to_string(), "mm".to_string())]);
            row
        };
        let mut angle = requirement("sm.countersink_angle", 0.0, Some(Confirmation::FactoryConfirmed));
        angle.limit = Limit::Max;
        angle.inputs = BTreeMap::from([("deg".to_string(), "deg".to_string())]);
        angle.value = BTreeMap::from([("deg".to_string(), 1.0)]);
        angle.angles_deg = Some(vec![90.0]);
        let mut params = parameters(Vec::new());
        params.shop = Some(Profile {
            format: 1,
            name: "TEST-ONLY full catalogue shop".into(),
            version: "2026-11-15".into(),
            layer: Layer::Factory,
            requirements: vec![
                millimetre("sm.countersink_depth", Limit::Max, 1.0),
                millimetre("sm.countersink_to_edge", Limit::Min, 3.0),
                millimetre("sm.countersink_to_bend", Limit::Min, 3.0),
                millimetre("sm.countersink_land", Limit::Min, 0.5),
                angle,
            ],
        });
        params
    }

    /// One 100 x 60 base with a 20 mm flange on its y = 0 edge and a Ø7.5, 90° countersink at
    /// (50, 30) widening a Ø3.4 through hole. The commands build the native sheet body.
    fn countersunk_design() -> crate::Session {
        let mut session = crate::Session::default();
        for (command, params) in [
            (
                "sheet.manage_rules",
                serde_json::json!({"name": "AL2", "thickness": "2.5 mm", "k_factor": 0.5, "bend_radius": "2.5 mm", "gap": "1 mm", "active": true}),
            ),
            ("sketch.create", serde_json::json!({"plane": "XY", "name": "Base"})),
            ("sketch.rectangle.two_point", serde_json::json!({"p0": [0, 0], "p1": [100, 60]})),
            ("sketch.finish", serde_json::json!({})),
            ("sheet.flange", serde_json::json!({"sketch": "Base"})),
            ("sheet.flange", serde_json::json!({"edges": [[50, 0, 2.5]], "height": 20})),
            (
                "solid.hole",
                serde_json::json!({
                    "position": [50, 30, 2.5],
                    "diameter": 3.4,
                    "type": "countersink",
                    "cs_diameter": 7.5,
                    "cs_angle": "90 deg"
                }),
            ),
        ] {
            session.execute(command, &params).unwrap_or_else(|error| panic!("{command}: {error}"));
        }
        session
    }

    /// The production adapter turns the measured cone into the five countersink findings, with
    /// hand-computed depth, land, rim distances and angle.
    #[test]
    fn a_countersunk_native_sheet_reaches_the_five_countersink_rules() {
        let session = countersunk_design();
        let state = session.model.state();
        let sheet = state.sheets.first().expect("one sheet body");
        assert_eq!(sheet.altered_by, None, "the flat model keeps the countersink");
        let mut params = sink_parameters();
        params.body = sheet.body.clone();
        params.declared.thickness_mm = Some(2.5);
        let outcome = check_sheet(sheet, params).expect("facts and report");
        assert_eq!(outcome.facts.countersinks, 1);
        let sink = outcome.facts.holes[0].countersink.as_ref().expect("the hole carries its countersink");
        assert!((sink.depth_mm - 2.05).abs() < 1e-9, "{}", sink.depth_mm);
        assert!((sink.land_mm - 0.45).abs() < 1e-9, "{}", sink.land_mm);

        // (rule, status, measured): the shop's confirmed rows decide. The depth is past its
        // 1.0 mm most and the 0.45 mm land is under its 0.5 mm least, so both fail; the rim is
        // clear of the edge and the bend and the cone is the held 90°, so those pass.
        for (rule, status, measured) in [
            ("sm.countersink_depth", solidcraft_dfm_rules::RuleStatus::NeedsReview, 2.05),
            ("sm.countersink_to_edge", solidcraft_dfm_rules::RuleStatus::Pass, 26.25),
            // The picked y = 0 edge is inset by the 90° setback R + t = 5 mm, so the rim is
            // 30 - 5 - 3.75 = 21.25 mm from the bend tangent.
            ("sm.countersink_to_bend", solidcraft_dfm_rules::RuleStatus::Pass, 21.25),
            ("sm.countersink_land", solidcraft_dfm_rules::RuleStatus::NeedsReview, 0.45),
            ("sm.countersink_angle", solidcraft_dfm_rules::RuleStatus::Pass, 90.0),
        ] {
            let finding = finding_for(&outcome.report, rule);
            assert_eq!(finding.status, status, "{rule}: {}", finding.message);
            let expected_state = if status == solidcraft_dfm_rules::RuleStatus::Pass {
                solidcraft_dfm_rules::AppState::Passed
            } else {
                solidcraft_dfm_rules::AppState::Failed
            };
            assert_eq!(finding.state, expected_state, "{rule}: {}", finding.message);
            let value = finding.measured_mm.unwrap_or(f64::NAN);
            assert!((value - measured).abs() < 1e-6, "{rule}: {value} != {measured}");
            if status == solidcraft_dfm_rules::RuleStatus::NeedsReview {
                assert!(finding.location.is_some(), "{rule} points at the rim");
            }
        }
    }

    /// A counterbore is a flat pocket of the sheet model, never a conical countersink.
    #[test]
    fn a_counterbore_in_a_sheet_is_not_a_countersink() {
        let mut session = crate::Session::default();
        for (command, params) in [
            ("sketch.create", serde_json::json!({"plane": "XY", "name": "Base"})),
            ("sketch.rectangle.two_point", serde_json::json!({"p0": [0, 0], "p1": [100, 60]})),
            ("sketch.finish", serde_json::json!({})),
            ("sheet.flange", serde_json::json!({"sketch": "Base"})),
            (
                "solid.hole",
                serde_json::json!({"position": [30, 30, 2.5], "diameter": 3.4, "type": "counterbore", "cb_diameter": 7.5, "cb_depth": 1.0}),
            ),
        ] {
            session.execute(command, &params).unwrap_or_else(|error| panic!("{command}: {error}"));
        }
        let state = session.model.state();
        let sheet = state.sheets.first().expect("one sheet body");
        let facts = sheet_dfm::facts(sheet).expect("facts");
        assert_eq!(facts.countersinks, 0, "a counterbore is not a countersink");
        assert!(facts.holes[0].countersink.is_none());
        let mut params = sink_parameters();
        params.body = sheet.body.clone();
        params.declared.thickness_mm = Some(2.5);
        let outcome = check_sheet(sheet, params).expect("check");
        let sink = sink_finding(&outcome.report);
        assert_eq!(sink.status, solidcraft_dfm_rules::RuleStatus::NotApplicable);
        assert!(sink.reason_code.is_none());
    }

    /// A countersink with no round through hole under it can never read as "no countersink":
    /// the five rules say the countersink was not read.
    #[test]
    fn a_blind_countersink_is_unrecognised_not_not_applicable() {
        let mut session = crate::Session::default();
        for (command, params) in [
            ("sketch.create", serde_json::json!({"plane": "XY", "name": "Base"})),
            ("sketch.rectangle.two_point", serde_json::json!({"p0": [0, 0], "p1": [100, 60]})),
            ("sketch.finish", serde_json::json!({})),
            ("sheet.flange", serde_json::json!({"sketch": "Base"})),
            (
                "solid.hole",
                serde_json::json!({
                    "position": [30, 30, 2.5],
                    "diameter": 3.4,
                    "depth": 1.0,
                    "type": "countersink",
                    "cs_diameter": 7.5,
                    "cs_angle": "90 deg"
                }),
            ),
        ] {
            session.execute(command, &params).unwrap_or_else(|error| panic!("{command}: {error}"));
        }
        let state = session.model.state();
        let sheet = state.sheets.first().expect("one sheet body");
        let facts = sheet_dfm::facts(sheet).expect("facts");
        assert_eq!(facts.countersinks, 1);
        assert!(facts.holes.is_empty(), "the blind bore is a pocket, not a round through hole");
        let mut params = sink_parameters();
        params.body = sheet.body.clone();
        params.declared.thickness_mm = Some(2.5);
        let outcome = check_sheet(sheet, params).expect("check");
        let sink = sink_finding(&outcome.report);
        assert_eq!(sink.status, solidcraft_dfm_rules::RuleStatus::NotVerified);
        assert_eq!(sink.reason_code.as_deref(), Some("HOLES_UNRECOGNISED"));
        assert_eq!(sink.state, solidcraft_dfm_rules::AppState::Unsupported);
        assert!(sink.measured_mm.is_none(), "nothing is measured or guessed");
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
        let recording = recording(include_str!("../../dfm-rules/tests/fixtures/intdfm/hole_min_diameter.below.json"));
        let recorded = recorded_facts(&recording);
        let tolerance = tolerance(&recording);
        let sheet = plate_with_hole(0.5);
        let facts = sheet_dfm::facts(&sheet).expect("measured facts");
        let measured = measured_sheet(&sheet, &facts);
        let report = compare(&recorded, &measured, tolerance).expect("the same hole");
        assert_eq!(report.pairs, 1);
        assert_eq!(report.edge_distances_compared, 1);
        assert_eq!(report.edge_points_compared, 1);
        assert!(report.bend_not_applicable);
        assert!(report.max_diameter_delta_mm <= tolerance.measurement_mm);

        let mut changed = measured.clone();
        if let Some(holes) = &mut changed.facts.holes {
            holes[0].diameter_mm += 0.01;
        }
        let mismatch = compare(&recorded, &changed, tolerance).expect_err("a diameter difference must fail");
        assert!(matches!(mismatch, GeometryMismatch::Diameter { .. }), "{mismatch}");

        let mut moved = measured.clone();
        if let Some(holes) = &mut moved.facts.holes {
            holes[0].x_mm += 0.02;
        }
        let mismatch = compare(&recorded, &moved, tolerance).expect_err("a moved hole must fail");
        assert!(matches!(mismatch, GeometryMismatch::Position { .. }), "{mismatch}");

        let mut missing = measured;
        missing.facts.holes = Some(Vec::new());
        let mismatch = compare(&recorded, &missing, tolerance).expect_err("a lost hole must fail");
        assert!(matches!(mismatch, GeometryMismatch::HoleCount { .. }), "{mismatch}");
    }

    #[test]
    fn the_second_gate_compares_a_multi_hole_plate() {
        let recording = recording(include_str!("../../dfm-rules/tests/fixtures/intdfm/hole_to_edge.plate_holes.json"));
        let recorded = recorded_facts(&recording);
        let tolerance = tolerance(&recording);
        let sheet = plate_with_holes(40.0, 25.0, &[(3.0, 12.5, 1.5), (20.0, 12.5, 0.25), (32.0, 12.5, 2.0)]);
        let facts = sheet_dfm::facts(&sheet).expect("measured facts");
        let measured = measured_sheet(&sheet, &facts);
        let report = compare(&recorded, &measured, tolerance).expect("three holes");
        assert_eq!(report.pairs, 3);
        assert_eq!(report.edge_distances_compared, 3);
        assert_eq!(report.edge_points_compared, 3);
        assert!(report.max_position_delta_mm <= tolerance.measurement_mm);
    }

    #[test]
    fn the_second_gate_compares_the_l_bracket_hole_bend_and_edge() {
        let recording = recording(include_str!("../../dfm-rules/tests/fixtures/intdfm/hole_to_bend.below.json"));
        let recorded = recorded_facts(&recording);
        let tolerance = tolerance(&recording);
        let sheet = l_bracket_sheet();
        let facts = sheet_dfm::facts(&sheet).expect("measured facts");
        let measured = measured_sheet(&sheet, &facts);
        let report = compare(&recorded, &measured, tolerance).expect("the L bracket");
        assert_eq!(report.pairs, 1);
        assert_eq!(report.bend_pairs, vec![(1, 1)]);
        assert_eq!(report.bend_gaps_compared, 1);
        assert_eq!(report.edge_distances_compared, 1);
        assert_eq!(report.edge_points_compared, 1);
        assert!(!report.bend_not_applicable);
    }

    #[test]
    fn the_second_gate_compares_flange_holes_and_maps_bend_numbers() {
        let (recorded, measured, tolerance) = u_holes_case();
        let report = compare(&recorded, &measured, tolerance).expect("the U channel");
        assert_eq!(report.pairs, 2);
        assert_eq!(report.bend_pairs, vec![(1, 1), (2, 2)]);
        assert_eq!(report.bend_gaps_compared, 3);
        assert_eq!(report.edge_distances_compared, 2);
        assert_eq!(report.edge_points_compared, 2);
        assert!(report.max_bend_gap_delta_mm <= tolerance.measurement_mm);
    }

    #[test]
    fn the_second_gate_compares_an_opening_edge_kind() {
        let recording = recording(include_str!("../../dfm-rules/tests/fixtures/intdfm/hole_to_edge.opening.json"));
        let recorded = recorded_facts(&recording);
        let tolerance = tolerance(&recording);
        let sheet = slot_sheet();
        let facts = sheet_dfm::facts(&sheet).expect("measured facts");
        assert_eq!(facts.holes[0].edge_kind, Some(sheet_dfm::EdgeKind::Opening));
        let measured = measured_sheet(&sheet, &facts);
        let report = compare(&recorded, &measured, tolerance).expect("the opening");
        assert_eq!(report.pairs, 1);
        assert_eq!(report.edge_distances_compared, 1);
        assert_eq!(report.edge_points_compared, 1);
    }

    #[test]
    fn the_second_gate_compares_in_the_recorded_frame_after_a_whole_model_transform() {
        let (recorded, measured, tolerance) = u_holes_case();
        let before = compare(&recorded, &measured, tolerance).expect("untransformed");
        let angle = 0.523_598_775_598_298_8;
        let translation = Vec3::new(5.0, -3.0, 2.0);
        let moved = transform_sheet(u_holes_sheet(true), angle, translation);
        let moved_facts = sheet_dfm::facts(&moved).expect("transformed facts");
        let mut moved_measured = measured_sheet(&moved, &moved_facts);
        // Compare in the recorded folded frame: inverse-transform the measured facts, including
        // the edge points and panel normals, then run the same gate.
        inverse_transform(&mut moved_measured, angle, translation);
        let after = compare(&recorded, &moved_measured, tolerance).expect("transformed");
        assert_eq!(after.pairs, before.pairs);
        assert_eq!(after.bend_pairs, before.bend_pairs);
        assert!((after.max_position_delta_mm - before.max_position_delta_mm).abs() < 1e-9);
        assert!((after.max_edge_point_delta_mm - before.max_edge_point_delta_mm).abs() < 1e-9);
        assert!((after.max_bend_gap_delta_mm - before.max_bend_gap_delta_mm).abs() < 1e-9);
    }

    #[test]
    fn the_second_gate_catches_a_wrong_edge_distance() {
        let (mut recorded, measured, tolerance) = u_holes_case();
        let leg = recorded.holes.as_ref().expect("holes").iter().position(|hole| hole.x_mm < 0.0).expect("leg hole");
        recorded.holes.as_mut().expect("holes")[leg].edge_mm = Some(8.01);
        let mismatch = compare(&recorded, &measured, tolerance).expect_err("edge distance");
        assert!(matches!(mismatch, GeometryMismatch::EdgeDistance { .. }), "{mismatch}");
    }

    #[test]
    fn the_second_gate_catches_a_wrong_edge_kind() {
        let (mut recorded, measured, tolerance) = u_holes_case();
        let leg = recorded.holes.as_ref().expect("holes").iter().position(|hole| hole.x_mm < 0.0).expect("leg hole");
        recorded.holes.as_mut().expect("holes")[leg].edge_kind = Some(RuleEdgeKind::Opening);
        let mismatch = compare(&recorded, &measured, tolerance).expect_err("edge kind");
        assert!(matches!(mismatch, GeometryMismatch::EdgeKindMismatch { .. }), "{mismatch}");
    }

    #[test]
    fn the_second_gate_catches_a_wrong_edge_owner() {
        let (mut recorded, measured, tolerance) = u_holes_case();
        let leg = recorded.holes.as_ref().expect("holes").iter().position(|hole| hole.x_mm < 0.0).expect("leg hole");
        // The base hole is recorded hole 2; the App's leg hole has no wall to it, so this is an
        // ownership failure, not a pass.
        recorded.holes.as_mut().expect("holes")[leg].edge_hole = Some(2);
        let mismatch = compare(&recorded, &measured, tolerance).expect_err("edge owner");
        assert!(matches!(mismatch, GeometryMismatch::EdgeOwnership { .. } | GeometryMismatch::MeasuredEdgeWallUnavailable { .. }), "{mismatch}");
    }

    #[test]
    fn the_second_gate_catches_a_wrong_bend_gap() {
        let (recorded, mut measured, tolerance) = u_holes_case();
        let base = measured.facts.holes.as_ref().expect("holes").iter().position(|hole| hole.x_mm > 0.0).expect("base hole");
        measured.facts.holes.as_mut().expect("holes")[base].bend_gaps[0].gap_mm += 0.01;
        let mismatch = compare(&recorded, &measured, tolerance).expect_err("bend gap");
        assert!(matches!(mismatch, GeometryMismatch::BendGap { .. }), "{mismatch}");
    }

    #[test]
    fn the_second_gate_catches_a_wrong_bend_radius() {
        let (mut recorded, measured, tolerance) = u_holes_case();
        recorded.recognition.as_mut().expect("recognition").bends[0].inside_radius_mm += 0.01;
        let mismatch = compare(&recorded, &measured, tolerance).expect_err("bend radius");
        assert!(matches!(mismatch, GeometryMismatch::BendRadius { .. }), "{mismatch}");
    }

    #[test]
    fn the_second_gate_catches_a_wrong_bend_association() {
        let (recorded, mut measured, tolerance) = u_holes_case();
        let leg = measured.facts.holes.as_ref().expect("holes").iter().position(|hole| hole.x_mm < 0.0).expect("leg hole");
        // The recorded leg hole's gap is on the left bend; point the measured gap at the right one.
        measured.facts.holes.as_mut().expect("holes")[leg].bend_gaps[0].bend = 1;
        let mismatch = compare(&recorded, &measured, tolerance).expect_err("bend association");
        assert!(matches!(mismatch, GeometryMismatch::BendGapMissing { .. } | GeometryMismatch::BendGapExtra { .. }), "{mismatch}");
    }

    #[test]
    fn the_recheck_entry_point_matches_the_recorded_pair() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../dfm-rules/tests/fixtures/recheck/v1/synthetic.two_holes_hole_moved.json")).expect("fixture");
        let earlier: solidcraft_dfm_rules::intdfm_report::Report = serde_json::from_value(fixture["earlier"].clone()).expect("earlier");
        let later: solidcraft_dfm_rules::intdfm_report::Report = serde_json::from_value(fixture["later"].clone()).expect("later");
        let expected: solidcraft_dfm_rules::intdfm_report::Recheck = serde_json::from_value(fixture["expected"].clone()).expect("expected");
        assert_eq!(recheck_reports(&earlier, &later).expect("recheck"), expected);
    }

    #[test]
    fn a_wrong_body_name_is_refused() {
        let sheet = plate_with_hole(0.5);
        let mut params = parameters(vec![requirement("sm.hole_min_diameter", 0.75, None)]);
        params.body = "Other".into();
        assert!(matches!(check_sheet(&sheet, params), Err(DfmError::Parameters(_))));
    }
}
