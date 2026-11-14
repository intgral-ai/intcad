//! The App-side DFM entry: native sheet facts, the pure rule crate, one structured report.
//!
//! No OCP, no network and no files. The live document is only read through [`SheetBody`]; the
//! rule crate decides on the facts and data it is given.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use solidcraft_dfm_rules::{
    BendFact as RuleBend, BendGap as RuleGap, CheckInput, Declared, EdgeKind as RuleEdgeKind, FactFamily, Facts as RuleFacts, HoleFact as RuleHole,
    ModelRef, Override, Profile, Recognition as RuleRecognition, Report,
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
        flat: rule_flat(hole.flat, hole.flat_assigned),
        countersink: None,
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
        missing: vec![FactFamily::BendRuns, FactFamily::ReliefEnds, FactFamily::ReliefSizes, FactFamily::Webs, FactFamily::Countersinks],
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
            joined: true,
            ..Default::default()
        }),
        ..Default::default()
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

/// The App's flat self-overlap rule id ([`CONTRACT.md`] §2). C's run layer calls
/// [`flat_no_overlap`] for one sheet and turns each observation into a finding with
/// `from = App` and `rules = ["app.flat_no_overlap"]`.
pub const FLAT_NO_OVERLAP_RULE: &str = "app.flat_no_overlap";

/// How `app.flat_no_overlap` decided one sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlatOverlapState {
    /// The flat pieces do not overlap.
    Passed,
    /// A pair of flat pieces shares more than the area tolerance.
    Failed,
    /// The flat pattern could not be measured; nothing is claimed.
    Unsupported,
}

/// One finding-shaped observation of `app.flat_no_overlap` for C's run layer.
///
/// The fields match what CONTRACT.md §2's `Finding` needs: the rule id, the App state, an optional
/// reason code, the measured area in mm², `feature.kind = "region"`, the pair of structural
/// pieces, a flat locate point, and the overlap outline.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlatOverlapObservation {
    pub rule: &'static str,
    pub state: FlatOverlapState,
    pub code: Option<String>,
    pub message: String,
    /// The intersection area in mm², set only for a failure.
    pub measured_mm2: Option<f64>,
    /// Always `"region"` when there is a measured area.
    pub feature_kind: Option<&'static str>,
    /// The area tolerance the check used, in mm².
    pub tolerance_mm2: f64,
    /// The two structural pieces, e.g. `["base", "panel 0"]`.
    pub pieces: Option<[String; 2]>,
    /// A point inside the overlap in flat coordinates, to locate it.
    pub flat: Option<[f64; 2]>,
    /// The overlap's largest loop in flat coordinates, as `[x, y, 0]`. Empty when there is none.
    pub outline: Vec<[f64; 3]>,
}

/// Run the App's flat self-overlap check on one sheet body, as finding-shaped observations.
///
/// A clear sheet gives one `Passed` observation, an overlapping sheet gives one `Failed`
/// observation per overlapping pair, and a sheet that cannot be measured gives one `Unsupported`
/// observation with a typed reason code. The check needs no factory data.
pub fn flat_no_overlap(sheet: &SheetBody) -> Result<Vec<FlatOverlapObservation>, DfmError> {
    let outcome = sheet_dfm::flat_overlap(sheet).map_err(|error| DfmError::Facts(error.to_string()))?;
    let base = FlatOverlapObservation {
        rule: FLAT_NO_OVERLAP_RULE,
        state: FlatOverlapState::Passed,
        code: None,
        message: String::new(),
        measured_mm2: None,
        feature_kind: None,
        tolerance_mm2: sheet_dfm::OVERLAP_AREA_TOLERANCE_MM2,
        pieces: None,
        flat: None,
        outline: Vec::new(),
    };
    match outcome {
        sheet_dfm::Overlap::Clear => Ok(vec![FlatOverlapObservation { message: "the flat panels and bend zones do not overlap".into(), ..base }]),
        sheet_dfm::Overlap::NotCovered { reason } => Ok(vec![FlatOverlapObservation {
            state: FlatOverlapState::Unsupported,
            code: Some(reason.code().to_string()),
            message: match reason {
                sheet_dfm::OverlapNotCovered::CannotUnfold => "the sheet has no flat pattern that matches the part".into(),
                sheet_dfm::OverlapNotCovered::CannotIntersect => "the flat panels and bend zones could not be intersected".into(),
            },
            ..base
        }]),
        sheet_dfm::Overlap::Overlapping { pairs } => Ok(pairs
            .iter()
            .map(|pair| {
                let pieces = pair.pieces.map(sheet_dfm::FlatPiece::label);
                let outline = pair.outline.first().map(|points| points.iter().map(|point| [point[0], point[1], 0.0]).collect()).unwrap_or_default();
                let flat = pair.outline.first().and_then(|points| points.first()).copied();
                FlatOverlapObservation {
                    state: FlatOverlapState::Failed,
                    code: Some("FLAT_OVERLAP".into()),
                    message: format!(
                        "{} and {} share {:.3} mm² after unfolding, above the {:.3} mm² overlap tolerance",
                        pieces[0],
                        pieces[1],
                        pair.area_mm2,
                        sheet_dfm::OVERLAP_AREA_TOLERANCE_MM2
                    ),
                    measured_mm2: Some(pair.area_mm2),
                    feature_kind: Some("region"),
                    pieces: Some(pieces),
                    flat,
                    outline,
                    ..base.clone()
                }
            })
            .collect()),
    }
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
        let sink = outcome
            .report
            .findings
            .iter()
            .find(|finding| finding.rules.iter().any(|rule| rule == "sm.countersink_depth"))
            .expect("a countersink result");
        assert_eq!(sink.state, solidcraft_dfm_rules::AppState::Unsupported);
        assert_eq!(sink.reason_code.as_deref(), Some("RULE_NOT_IMPLEMENTED"));
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

    /// A 40 x 25 plate with one flange whose flat panel is the given polygon.
    fn sheet_with_flange_panel(corners: &[Vec2]) -> SheetBody {
        let mut sheet = plate_with_holes(40.0, 25.0, &[]);
        sheet.flanges.push(flange(None, Vec2::new(0.0, 25.0), Vec2::X, Vec2::Y, 40.0, 1.5, 10.0));
        if let Some(f) = sheet.flanges.last_mut() {
            f.shape = Some(corners.to_vec());
        }
        sheet
    }

    /// `app.flat_no_overlap` is the App entry for the pure flat check: a clear sheet gives one
    /// passed observation and needs no factory data.
    #[test]
    fn the_flat_no_overlap_adapter_passes_a_clear_sheet() {
        let sheet = plate_with_holes(40.0, 25.0, &[(20.0, 12.5, 1.5)]);
        let observations = flat_no_overlap(&sheet).expect("the App check");
        assert_eq!(observations.len(), 1);
        let finding = &observations[0];
        assert_eq!(finding.rule, FLAT_NO_OVERLAP_RULE);
        assert_eq!(finding.state, FlatOverlapState::Passed);
        assert_eq!(finding.code, None);
        assert_eq!(finding.measured_mm2, None);
        assert_eq!(finding.feature_kind, None);
        assert_eq!(finding.tolerance_mm2, sheet_dfm::OVERLAP_AREA_TOLERANCE_MM2);
        assert!(finding.outline.is_empty());
    }

    /// A 5 x 4 mm panel inside the base is a 20 mm² overlap: one failed, App-sourced `region`
    /// finding with the pair, the area in mm², and an outline to locate it.
    #[test]
    fn the_flat_no_overlap_adapter_reports_the_pair_area_and_outline() {
        let corners = [Vec2::new(30.0, 5.0), Vec2::new(35.0, 5.0), Vec2::new(35.0, 9.0), Vec2::new(30.0, 9.0)];
        let sheet = sheet_with_flange_panel(&corners);
        let observations = flat_no_overlap(&sheet).expect("the App check");
        assert_eq!(observations.len(), 1);
        let finding = &observations[0];
        assert_eq!(finding.rule, FLAT_NO_OVERLAP_RULE);
        assert_eq!(finding.state, FlatOverlapState::Failed);
        assert_eq!(finding.code.as_deref(), Some("FLAT_OVERLAP"));
        assert_eq!(finding.feature_kind, Some("region"));
        assert!((finding.measured_mm2.expect("a measured area") - 20.0).abs() < 1e-9);
        assert_eq!(finding.pieces, Some(["base".to_string(), "panel 0".to_string()]));
        assert!(finding.message.contains("20.000 mm"), "{}", finding.message);
        let at = finding.flat.expect("a locate point");
        assert!((30.0..=35.0).contains(&at[0]) && (5.0..=9.0).contains(&at[1]));
        assert!(!finding.outline.is_empty(), "the overlap needs an outline");
        assert!(finding.outline.iter().all(|point| point[2] == 0.0), "the outline is in the flat frame");
    }

    /// A body that cannot unfold gives a typed not-covered reason, never a pass.
    #[test]
    fn the_flat_no_overlap_adapter_reports_not_covered() {
        let corners = [Vec2::new(30.0, 5.0), Vec2::new(35.0, 5.0), Vec2::new(35.0, 9.0), Vec2::new(30.0, 9.0)];
        let mut sheet = sheet_with_flange_panel(&corners);
        sheet.altered_by = Some("Fillet1".into());
        let observations = flat_no_overlap(&sheet).expect("the App check");
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].state, FlatOverlapState::Unsupported);
        assert_eq!(observations[0].code.as_deref(), Some("CANNOT_UNFOLD"));
        assert_eq!(observations[0].measured_mm2, None);
    }

    #[test]
    fn a_wrong_body_name_is_refused() {
        let sheet = plate_with_hole(0.5);
        let mut params = parameters(vec![requirement("sm.hole_min_diameter", 0.75, None)]);
        params.body = "Other".into();
        assert!(matches!(check_sheet(&sheet, params), Err(DfmError::Parameters(_))));
    }
}
