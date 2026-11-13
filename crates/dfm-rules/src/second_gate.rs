//! The second gate: compare the facts the App measured with the facts INTdfm recorded.
//!
//! This is migration verification, not a rule. It pairs usable round through holes by their
//! folded centres, one to one, and compares measurements with tolerances that are separate from
//! `sm.hole_min_diameter`'s threshold rounding (1e-6 mm). The tolerances are pinned in every
//! recording under `comparison`, so a wider tolerance is a contract change, not a quiet fix.

use serde::Serialize;

use crate::{Facts, HoleFact, SAME_SHEET_MM};

/// Pairing and comparison distances for the second gate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeometryTolerance {
    /// The farthest two centres may be and still be the same hole.
    pub pairing_mm: f64,
    /// Two candidates this close to the chosen one are ambiguous; refuse rather than guess.
    pub ambiguous_mm: f64,
    /// After pairing, the most a diameter, depth, position or thickness may differ.
    pub measurement_mm: f64,
}

impl Default for GeometryTolerance {
    fn default() -> Self {
        Self { pairing_mm: 0.05, ambiguous_mm: 0.001, measurement_mm: 0.001 }
    }
}

/// What the second gate measured, for the evidence file.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeometryReport {
    pub pairs: usize,
    pub thickness_delta_mm: f64,
    pub max_diameter_delta_mm: f64,
    pub max_depth_delta_mm: f64,
    pub max_position_delta_mm: f64,
}

/// A second-gate failure. The code is stable; the message names the holes for a person.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GeometryMismatch {
    #[error("the recorded facts have no hole list")]
    RecordedHolesUnavailable,
    #[error("the measured facts have no hole list")]
    MeasuredHolesUnavailable,
    #[error("a model thickness is missing")]
    ThicknessUnavailable,
    #[error("thickness differs by {delta_mm} mm")]
    Thickness { delta_mm: f64 },
    #[error("recorded {recorded} usable holes, measured {measured}")]
    HoleCount { recorded: usize, measured: usize },
    #[error("recorded hole {recorded} has no measured partner within {pairing_mm} mm")]
    Unpaired { recorded: usize, pairing_mm: f64 },
    #[error("measured hole {measured} has no recorded partner within {pairing_mm} mm")]
    UnpairedMeasured { measured: usize, pairing_mm: f64 },
    #[error("recording hole {recorded} could pair with measured hole {measured} and another within {ambiguous_mm} mm")]
    AmbiguousPairing { recorded: usize, measured: usize, ambiguous_mm: f64 },
    #[error("hole {recorded}/{measured} diameter differs by {delta_mm} mm")]
    Diameter { recorded: usize, measured: usize, delta_mm: f64 },
    #[error("hole {recorded}/{measured} depth differs by {delta_mm} mm")]
    Depth { recorded: usize, measured: usize, delta_mm: f64 },
    #[error("hole {recorded}/{measured} position differs by {delta_mm} mm")]
    Position { recorded: usize, measured: usize, delta_mm: f64 },
}

fn usable(facts: &Facts, recorded: bool) -> Result<Vec<(usize, &HoleFact)>, GeometryMismatch> {
    let holes =
        facts.holes.as_ref().ok_or(if recorded { GeometryMismatch::RecordedHolesUnavailable } else { GeometryMismatch::MeasuredHolesUnavailable })?;
    let thickness = facts.paired_mm.ok_or(GeometryMismatch::ThicknessUnavailable)?;
    Ok(holes
        .iter()
        .enumerate()
        .map(|(index, hole)| (index + 1, hole))
        .filter(|(_, hole)| hole.square && (hole.depth_mm - thickness).abs() <= SAME_SHEET_MM)
        .collect())
}

fn distance(a: &HoleFact, b: &HoleFact) -> f64 {
    let (dx, dy, dz) = (a.x_mm - b.x_mm, a.y_mm - b.y_mm, a.z_mm - b.z_mm);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Compare recorded and measured facts. Returns the largest deltas, or the first decisive failure.
pub fn compare(recorded: &Facts, measured: &Facts, tolerance: GeometryTolerance) -> Result<GeometryReport, GeometryMismatch> {
    let thickness_recorded = recorded.paired_mm.ok_or(GeometryMismatch::ThicknessUnavailable)?;
    let thickness_measured = measured.paired_mm.ok_or(GeometryMismatch::ThicknessUnavailable)?;
    let thickness_delta = (thickness_recorded - thickness_measured).abs();
    if thickness_delta > tolerance.measurement_mm {
        return Err(GeometryMismatch::Thickness { delta_mm: thickness_delta });
    }
    let recorded_holes = usable(recorded, true)?;
    let measured_holes = usable(measured, false)?;
    if recorded_holes.len() != measured_holes.len() {
        return Err(GeometryMismatch::HoleCount { recorded: recorded_holes.len(), measured: measured_holes.len() });
    }
    if recorded_holes.is_empty() {
        return Ok(GeometryReport {
            pairs: 0,
            thickness_delta_mm: thickness_delta,
            max_diameter_delta_mm: 0.0,
            max_depth_delta_mm: 0.0,
            max_position_delta_mm: 0.0,
        });
    }
    let mut candidates: Vec<(f64, usize, usize, &HoleFact, &HoleFact)> = Vec::new();
    for (recorded_number, recorded_hole) in &recorded_holes {
        for (measured_number, measured_hole) in &measured_holes {
            let apart = distance(recorded_hole, measured_hole);
            if apart <= tolerance.pairing_mm {
                candidates.push((apart, *recorded_number, *measured_number, recorded_hole, measured_hole));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut used_recorded = vec![false; recorded_holes.len() + 1];
    let mut used_measured = vec![false; measured_holes.len() + 1];
    let mut pairs = Vec::new();
    for (index, (apart, recorded_number, measured_number, recorded_hole, measured_hole)) in candidates.iter().enumerate() {
        if used_recorded[*recorded_number] || used_measured[*measured_number] {
            continue;
        }
        let ambiguous = candidates.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && (other.1 == *recorded_number || other.2 == *measured_number)
                && (!used_recorded[other.1] && !used_measured[other.2])
                && (other.0 - *apart).abs() <= tolerance.ambiguous_mm
        });
        if ambiguous {
            return Err(GeometryMismatch::AmbiguousPairing {
                recorded: *recorded_number,
                measured: *measured_number,
                ambiguous_mm: tolerance.ambiguous_mm,
            });
        }
        used_recorded[*recorded_number] = true;
        used_measured[*measured_number] = true;
        pairs.push((*recorded_number, *measured_number, recorded_hole, measured_hole));
    }
    if let Some((recorded_number, _)) = recorded_holes.iter().find(|(number, _)| !used_recorded[*number]) {
        return Err(GeometryMismatch::Unpaired { recorded: *recorded_number, pairing_mm: tolerance.pairing_mm });
    }
    if let Some((measured_number, _)) = measured_holes.iter().find(|(number, _)| !used_measured[*number]) {
        return Err(GeometryMismatch::UnpairedMeasured { measured: *measured_number, pairing_mm: tolerance.pairing_mm });
    }
    let mut max_diameter = 0.0_f64;
    let mut max_depth = 0.0_f64;
    let mut max_position = 0.0_f64;
    for (recorded_number, measured_number, recorded_hole, measured_hole) in pairs {
        let diameter = (recorded_hole.diameter_mm - measured_hole.diameter_mm).abs();
        if diameter > tolerance.measurement_mm {
            return Err(GeometryMismatch::Diameter { recorded: recorded_number, measured: measured_number, delta_mm: diameter });
        }
        let depth = (recorded_hole.depth_mm - measured_hole.depth_mm).abs();
        if depth > tolerance.measurement_mm {
            return Err(GeometryMismatch::Depth { recorded: recorded_number, measured: measured_number, delta_mm: depth });
        }
        let position = distance(recorded_hole, measured_hole);
        if position > tolerance.measurement_mm {
            return Err(GeometryMismatch::Position { recorded: recorded_number, measured: measured_number, delta_mm: position });
        }
        max_diameter = max_diameter.max(diameter);
        max_depth = max_depth.max(depth);
        max_position = max_position.max(position);
    }
    Ok(GeometryReport {
        pairs: recorded_holes.len(),
        thickness_delta_mm: thickness_delta,
        max_diameter_delta_mm: max_diameter,
        max_depth_delta_mm: max_depth,
        max_position_delta_mm: max_position,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hole(x: f64) -> HoleFact {
        HoleFact {
            diameter_mm: 3.0,
            x_mm: x,
            y_mm: 12.5,
            z_mm: 0.75,
            depth_mm: 1.5,
            square: true,
            edge_mm: None,
            edge_at_mm: None,
            edge_kind: None,
            edge_hole: None,
            bend_gaps: Vec::new(),
            flat: None,
        }
    }

    fn facts(holes: Vec<HoleFact>) -> Facts {
        Facts { paired_mm: Some(1.5), holes: Some(holes), ..Facts::default() }
    }

    #[test]
    fn equal_facts_pass() {
        let report = compare(&facts(vec![hole(20.0)]), &facts(vec![hole(20.0)]), GeometryTolerance::default()).expect("equal");
        assert_eq!(report.pairs, 1);
        assert_eq!(report.max_diameter_delta_mm, 0.0);
    }

    #[test]
    fn a_diameter_outside_the_measurement_tolerance_fails() {
        let mut changed = hole(20.0);
        changed.diameter_mm = 3.01;
        let mismatch = compare(&facts(vec![hole(20.0)]), &facts(vec![changed]), GeometryTolerance::default()).expect_err("diameter");
        assert!(matches!(mismatch, GeometryMismatch::Diameter { .. }));
    }

    #[test]
    fn a_position_outside_the_measurement_tolerance_fails() {
        let mismatch = compare(&facts(vec![hole(20.0)]), &facts(vec![hole(20.02)]), GeometryTolerance::default()).expect_err("position");
        assert!(matches!(mismatch, GeometryMismatch::Position { .. }));
    }

    #[test]
    fn a_missing_hole_fails() {
        let mismatch = compare(&facts(vec![hole(20.0), hole(30.0)]), &facts(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("count");
        assert!(matches!(mismatch, GeometryMismatch::HoleCount { .. }));
    }

    #[test]
    fn two_holes_at_one_place_are_ambiguous() {
        let mismatch = compare(&facts(vec![hole(20.0), hole(20.0)]), &facts(vec![hole(20.0), hole(20.0)]), GeometryTolerance::default());
        assert!(matches!(mismatch, Err(GeometryMismatch::AmbiguousPairing { .. })));
    }

    #[test]
    fn a_thickness_outside_the_measurement_tolerance_fails() {
        let mut measured = facts(vec![hole(20.0)]);
        measured.paired_mm = Some(1.51);
        let mismatch = compare(&facts(vec![hole(20.0)]), &measured, GeometryTolerance::default()).expect_err("thickness");
        assert!(matches!(mismatch, GeometryMismatch::Thickness { .. }));
    }
}
