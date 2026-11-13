//! The second gate: compare the facts the App measured with the facts INTdfm recorded.
//!
//! This is migration verification, not a rule. It pairs usable round through holes by a complete
//! one-to-one matching, pairs bends by folded position, and compares the facts the three hole
//! rules consume. The tolerances are separate from `sm.hole_min_diameter`'s threshold rounding
//! and are pinned in every recording under `comparison`, so a wider tolerance is a contract
//! change, not a quiet fix.
//!
//! # Pairing contract (schema 2)
//!
//! * Build the allowed graph with folded-centre distance, diameter difference and depth difference
//!   each within `measurement_mm` (position <= 0.001 mm, not the 0.05 mm fallback bound).
//! * Find a perfect matching in that graph. If there is more than one, the correspondence is not
//!   determined; refuse with `AmbiguousPairing`.
//! * If no complete matching exists, use the 0.05 mm position graph only to diagnose
//!   `Unpaired`/`Position`/`Diameter`/`Depth`. A mismeasured depth is a `Depth`, never a count.
//! * Compare every difference after rounding to `COMPARISON_QUANTUM_MM` (1e-6), so a nominal
//!   boundary does not depend on coordinate magnitude.
//!
//! Hole and bend numbers are implementation-local. All numbers in the report and errors are
//! canonical: the facts are sorted by geometry first, so permuting the input does not change the
//! result or its reported IDs. A recorded `edge_hole` is mapped through the hole pairing to the
//! App's measured hole number, and a recorded `bend_gaps[].bend` is mapped through the bend
//! pairing to the App's measured bend number. Raw `flat` ids are never compared: a hole's flat
//! association is the set of paired bends that bound its panel.
//!
//! A comparison that has nothing to measure says so; a measurement missing on one side is a
//! failure, never a pass. Non-finite facts and invalid tolerances are refused before pairing.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::{BendFact, BendGap, EdgeKind, Facts, HoleFact};

/// The quantum every difference is rounded to before a comparison, from the recording contract.
pub const COMPARISON_QUANTUM_MM: f64 = 1e-6;

/// Pairing and comparison distances for the second gate. Schema 2 has no numeric ambiguity
/// window: ambiguity is structural (more than one complete matching).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeometryTolerance {
    /// The farthest two centres may be and still be considered by the fallback diagnosis.
    pub pairing_mm: f64,
    /// A pair is compatible when position, diameter and depth are each within this.
    pub measurement_mm: f64,
}

impl Default for GeometryTolerance {
    fn default() -> Self {
        Self { pairing_mm: 0.05, measurement_mm: 0.001 }
    }
}

/// Which side of the comparison a fact came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactSide {
    Recorded,
    Measured,
}

/// The App's nearest other round-hole wall, keyed to the measured hole that owns it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoleWallFact {
    /// The other hole's measured number.
    pub number: usize,
    pub gap_mm: f64,
    pub at_mm: [f64; 3],
}

/// Facts the rule crate's [`Facts`] cannot hold, measured by the App.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeasuredHole {
    /// The nearest other round hole on the same panel, when there is one.
    pub wall: Option<HoleWallFact>,
    /// The panel's outward normal in the measured frame, used to compare a recorded edge point
    /// (reported on the sheet's skin) with the App's mid-depth edge point.
    pub normal: Option<[f64; 3]>,
}

/// The App's measured sheet: the rule facts plus the second gate's extra observations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeasuredSheet {
    pub facts: Facts,
    /// One entry per measured hole, in `facts.holes` order.
    pub holes: Vec<MeasuredHole>,
}

impl MeasuredSheet {
    /// Compare the rule facts alone. Edge points are then not compared unless a normal is given.
    pub fn bare(facts: Facts) -> Self {
        let count = facts.holes.as_ref().map_or(0, Vec::len);
        Self { facts, holes: vec![MeasuredHole::default(); count] }
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
    pub edge_distances_compared: usize,
    pub edge_walls_compared: usize,
    pub edge_points_compared: usize,
    /// Holes where both sides say there is no measurable edge.
    pub edge_not_compared: usize,
    /// Edge points that both sides have, but no panel normal was supplied to project them.
    pub edge_points_not_compared: usize,
    pub max_edge_delta_mm: f64,
    pub max_edge_point_delta_mm: f64,
    /// Canonical recorded bend number and the canonical measured bend number it paired with.
    pub bend_pairs: Vec<(usize, usize)>,
    /// Both sides have no bend, so there is nothing to compare.
    pub bend_not_applicable: bool,
    pub bend_gaps_compared: usize,
    pub max_bend_gap_delta_mm: f64,
    pub max_bend_radius_delta_mm: f64,
    pub max_bend_length_delta_mm: f64,
}

/// A second-gate result that is not a clean comparison.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GeometryMismatch {
    #[error("the recorded facts have no hole list")]
    RecordedHolesUnavailable,
    #[error("the measured facts have no hole list")]
    MeasuredHolesUnavailable,
    #[error("the recorded facts have no bend recognition")]
    RecordedRecognitionUnavailable,
    #[error("the measured facts have no bend recognition")]
    MeasuredRecognitionUnavailable,
    #[error("the recorded facts have no measured thickness")]
    RecordedThicknessUnavailable,
    #[error("the measured facts have no measured thickness")]
    MeasuredThicknessUnavailable,
    #[error("both sides have no applicable round through hole, so there was nothing to compare")]
    NoApplicableHole,
    #[error("the {field} tolerance is invalid: {value}")]
    InvalidTolerance { field: &'static str, value: f64 },
    #[error("{side:?} facts have a non-finite {field}")]
    NonFinite { side: FactSide, field: &'static str },
    #[error("thickness differs by {delta_mm} mm")]
    Thickness { delta_mm: f64 },
    #[error("recorded {recorded} applicable holes, measured {measured}")]
    HoleCount { recorded: usize, measured: usize },
    #[error("recorded hole {recorded} has no measured partner within {pairing_mm} mm")]
    Unpaired { recorded: usize, pairing_mm: f64 },
    #[error("measured hole {measured} has no recorded partner within {pairing_mm} mm")]
    UnpairedMeasured { measured: usize, pairing_mm: f64 },
    #[error("no one-to-one hole pairing covers every recorded and measured applicable hole")]
    NoPerfectPairing,
    #[error("more than one complete hole matching: recorded hole {recorded} could pair with measured hole {measured} or another")]
    AmbiguousPairing { recorded: usize, measured: usize },
    #[error("hole {recorded}/{measured} diameter differs by {delta_mm} mm")]
    Diameter { recorded: usize, measured: usize, delta_mm: f64 },
    #[error("hole {recorded}/{measured} depth differs by {delta_mm} mm")]
    Depth { recorded: usize, measured: usize, delta_mm: f64 },
    #[error("hole {recorded}/{measured} position differs by {delta_mm} mm")]
    Position { recorded: usize, measured: usize, delta_mm: f64 },
    #[error("recorded hole {recorded} has no edge distance")]
    RecordedEdgeUnavailable { recorded: usize },
    #[error("measured hole {measured} has no edge distance")]
    MeasuredEdgeUnavailable { measured: usize },
    #[error("hole {recorded}/{measured} edge distance differs by {delta_mm} mm")]
    EdgeDistance { recorded: usize, measured: usize, delta_mm: f64 },
    #[error("hole {recorded}/{measured} edge point differs by {delta_mm} mm")]
    EdgePoint { recorded: usize, measured: usize, delta_mm: f64 },
    #[error("hole {recorded}/{measured} edge point is missing on one side")]
    EdgePointUnavailable { recorded: usize, measured: usize },
    #[error("hole {recorded}/{measured} edge kind is {recorded_kind:?} and {measured_kind:?}")]
    EdgeKindMismatch { recorded: usize, measured: usize, recorded_kind: Option<EdgeKind>, measured_kind: Option<EdgeKind> },
    #[error("hole {recorded}/{measured} edge ownership is recorded hole {recorded_hole:?} and measured wall {measured_wall:?}")]
    EdgeOwnership { recorded: usize, measured: usize, recorded_hole: Option<usize>, measured_wall: Option<usize> },
    #[error("recorded hole {recorded}'s edge names recording hole {wall_hole}, which has no measured partner")]
    EdgeWallUnmapped { recorded: usize, wall_hole: usize },
    #[error("measured hole {measured} has no measured wall for recorded hole {recorded_hole}")]
    MeasuredEdgeWallUnavailable { measured: usize, recorded_hole: usize },
    #[error("hole {recorded}/{measured} names wall hole {expected} on the recorded side and {actual} on the measured side")]
    EdgeWallNumber { recorded: usize, measured: usize, expected: usize, actual: usize },
    #[error("recorded {recorded} bends, measured {measured}")]
    BendCount { recorded: usize, measured: usize },
    #[error("recorded bend {recorded} has no measured partner within {pairing_mm} mm")]
    BendUnpaired { recorded: usize, pairing_mm: f64 },
    #[error("measured bend {measured} has no recorded partner within {pairing_mm} mm")]
    BendUnpairedMeasured { measured: usize, pairing_mm: f64 },
    #[error("no one-to-one bend pairing covers every recorded and measured bend")]
    NoPerfectBendPairing,
    #[error("more than one complete bend matching: recorded bend {recorded} could pair with measured bend {measured} or another")]
    AmbiguousBendPairing { recorded: usize, measured: usize },
    #[error("bend {recorded}/{measured} inside radius differs by {delta_mm} mm")]
    BendRadius { recorded: usize, measured: usize, delta_mm: f64 },
    #[error("bend {recorded}/{measured} length differs by {delta_mm} mm")]
    BendLength { recorded: usize, measured: usize, delta_mm: f64 },
    #[error("hole {recorded}/{measured} is on the paired bend {bend} on one side only")]
    BendFlatAssociation { recorded: usize, measured: usize, bend: usize },
    #[error("hole {recorded} has no bend gaps although the part has bends")]
    BendGapsUnavailable { recorded: usize },
    #[error("recorded hole {recorded} has no bend gaps")]
    RecordedBendGapsUnavailable { recorded: usize },
    #[error("measured hole {measured} has no bend gaps")]
    MeasuredBendGapsUnavailable { measured: usize },
    #[error("recorded hole {recorded} names bend {bend}, which has no measured partner")]
    BendGapUnmapped { recorded: usize, bend: usize },
    #[error("hole {recorded}/{measured} has no measured gap for paired bend {bend}")]
    BendGapMissing { recorded: usize, measured: usize, bend: usize },
    #[error("hole {recorded}/{measured} has an extra measured gap for bend {bend}")]
    BendGapExtra { recorded: usize, measured: usize, bend: usize },
    #[error("hole {recorded}/{measured} bend {bend} gap differs by {delta_mm} mm")]
    BendGap { recorded: usize, measured: usize, bend: usize, delta_mm: f64 },
    #[error("hole {recorded}/{measured} bend {bend} radius differs by {delta_mm} mm")]
    BendGapRadius { recorded: usize, measured: usize, bend: usize, delta_mm: f64 },
    #[error("hole {recorded}/{measured} lists bend {bend} twice")]
    BendGapDuplicate { recorded: usize, measured: usize, bend: usize },
}

/// Round a difference to the contract's comparison quantum before deciding a boundary.
fn quantum(value: f64) -> f64 {
    (value / COMPARISON_QUANTUM_MM).round() * COMPARISON_QUANTUM_MM
}

/// Is a delta at or inside a limit, after the contract's rounding?
fn within(delta: f64, tolerance: f64) -> bool {
    delta.is_finite() && quantum(delta) <= quantum(tolerance)
}

fn check_tolerance(name: &'static str, value: f64) -> Result<(), GeometryMismatch> {
    if !value.is_finite() || value <= 0.0 {
        return Err(GeometryMismatch::InvalidTolerance { field: name, value });
    }
    Ok(())
}

fn check_finite(value: f64, side: FactSide, field: &'static str) -> Result<(), GeometryMismatch> {
    if value.is_finite() { Ok(()) } else { Err(GeometryMismatch::NonFinite { side, field }) }
}

fn check_point(point: &[f64], side: FactSide, field: &'static str) -> Result<(), GeometryMismatch> {
    for value in point {
        check_finite(*value, side, field)?;
    }
    Ok(())
}

/// Reject non-finite facts before a squared distance can turn them into a false pass or a panic.
fn validate_facts(facts: &Facts, side: FactSide) -> Result<(), GeometryMismatch> {
    if let Some(thickness) = facts.paired_mm {
        check_finite(thickness, side, "paired_mm")?;
    }
    for hole in facts.holes.iter().flatten() {
        check_finite(hole.diameter_mm, side, "diameter_mm")?;
        check_finite(hole.x_mm, side, "x_mm")?;
        check_finite(hole.y_mm, side, "y_mm")?;
        check_finite(hole.z_mm, side, "z_mm")?;
        check_finite(hole.depth_mm, side, "depth_mm")?;
        if let Some(edge) = hole.edge_mm {
            check_finite(edge, side, "edge_mm")?;
        }
        if let Some(point) = &hole.edge_at_mm {
            check_point(point, side, "edge_at_mm")?;
        }
        for gap in &hole.bend_gaps {
            check_finite(gap.gap_mm, side, "bend_gaps.gap_mm")?;
            check_finite(gap.inside_radius_mm, side, "bend_gaps.inside_radius_mm")?;
        }
    }
    if let Some(recognition) = &facts.recognition {
        for bend in &recognition.bends {
            check_finite(bend.inside_radius_mm, side, "bends.inside_radius_mm")?;
            check_finite(bend.angle_deg, side, "bends.angle_deg")?;
            check_finite(bend.length_mm, side, "bends.length_mm")?;
            check_finite(bend.x_mm, side, "bends.x_mm")?;
            check_finite(bend.y_mm, side, "bends.y_mm")?;
            check_finite(bend.z_mm, side, "bends.z_mm")?;
        }
    }
    Ok(())
}

fn validate_observations(measured: &MeasuredSheet) -> Result<(), GeometryMismatch> {
    for hole in &measured.holes {
        if let Some(wall) = &hole.wall {
            check_finite(wall.gap_mm, FactSide::Measured, "nearest_hole_wall_mm")?;
            check_point(&wall.at_mm, FactSide::Measured, "nearest_hole_wall_at_mm")?;
        }
        if let Some(normal) = &hole.normal {
            check_point(normal, FactSide::Measured, "panel_normal")?;
        }
    }
    Ok(())
}

/// A square round hole the gate can pair. Depth is compared after pairing, not used as a filter:
/// a mismeasured depth must report `Depth`, never disappear into a count change.
fn applicable(facts: &Facts, recorded: bool) -> Result<Vec<(usize, &HoleFact)>, GeometryMismatch> {
    let holes =
        facts.holes.as_ref().ok_or(if recorded { GeometryMismatch::RecordedHolesUnavailable } else { GeometryMismatch::MeasuredHolesUnavailable })?;
    Ok(holes.iter().enumerate().map(|(index, hole)| (index + 1, hole)).filter(|(_, hole)| hole.square).collect())
}

fn distance(a: &HoleFact, b: &HoleFact) -> f64 {
    let (dx, dy, dz) = (a.x_mm - b.x_mm, a.y_mm - b.y_mm, a.z_mm - b.z_mm);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn distance3(a: &[f64], b: &[f64]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn hole_key(hole: &HoleFact) -> Vec<f64> {
    vec![hole.x_mm, hole.y_mm, hole.z_mm, hole.diameter_mm, hole.depth_mm]
}

fn bend_key(bend: &BendFact) -> Vec<f64> {
    vec![bend.x_mm, bend.y_mm, bend.z_mm, bend.inside_radius_mm, bend.length_mm]
}

fn key_cmp(a: &[f64], b: &[f64]) -> std::cmp::Ordering {
    for (left, right) in a.iter().zip(b.iter()) {
        match left.total_cmp(right) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    a.len().cmp(&b.len())
}

/// Original indices sorted by geometry. This is the canonical numbering used in errors, so the
/// same facts in another input order produce the same report.
fn canonical_order(keys: &[Vec<f64>]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by(|a, b| key_cmp(&keys[*a], &keys[*b]));
    order
}

/// `order[position] = original index` -> `canonical[original index] = position + 1`.
fn canonical_numbers(order: &[usize]) -> Vec<usize> {
    let mut canonical = vec![0; order.len()];
    for (position, &original) in order.iter().enumerate() {
        canonical[original] = position + 1;
    }
    canonical
}

/// Up to this many items the fallback diagnosis enumerates every perfect matching (at most
/// 8! = 40,320) and takes the exact minimum. Above it, a nearest-first greedy pairing is used;
/// the same tolerance and geometric ordering apply, but a unique perfect matching can be missed.
/// The complete-matching search itself (the contract path) is polynomial at every size.
const EXACT_FALLBACK_LIMIT: usize = 8;

/// Kuhn's augmenting-path search. `allowed` is square and already in canonical order.
fn find_perfect(allowed: &[Vec<bool>]) -> Option<Vec<usize>> {
    let n = allowed.len();
    if n == 0 {
        return Some(Vec::new());
    }
    if allowed.iter().any(|row| row.len() != n) {
        return None;
    }
    fn augment(row: usize, allowed: &[Vec<bool>], matched: &mut [usize], seen: &mut [bool]) -> bool {
        for col in 0..allowed.len() {
            if !allowed[row][col] || seen[col] {
                continue;
            }
            seen[col] = true;
            if matched[col] == usize::MAX || augment(matched[col], allowed, matched, seen) {
                matched[col] = row;
                return true;
            }
        }
        false
    }
    let mut matched = vec![usize::MAX; n];
    for row in 0..n {
        let mut seen = vec![false; n];
        if !augment(row, allowed, &mut matched, &mut seen) {
            return None;
        }
    }
    let mut matching = vec![usize::MAX; n];
    for (col, &row) in matched.iter().enumerate() {
        if row != usize::MAX {
            matching[row] = col;
        }
    }
    Some(matching)
}

/// A perfect bipartite graph has another perfect matching exactly when the alternating digraph
/// (unmatched edges row -> column, matched edges column -> row) has a directed cycle. This is the
/// structural ambiguity test; no distance window is involved.
fn more_than_one(allowed: &[Vec<bool>], matching: &[usize]) -> bool {
    let n = allowed.len();
    let nodes = 2 * n;
    let mut adjacency = vec![Vec::new(); nodes];
    let mut indegree = vec![0usize; nodes];
    for (row, row_allowed) in allowed.iter().enumerate() {
        for (col, &is_allowed) in row_allowed.iter().enumerate() {
            if !is_allowed {
                continue;
            }
            let (from, to) = if matching[row] == col { (n + col, row) } else { (row, n + col) };
            adjacency[from].push(to);
            indegree[to] += 1;
        }
    }
    let mut queue: Vec<usize> = (0..nodes).filter(|&node| indegree[node] == 0).collect();
    let mut processed = 0;
    while let Some(node) = queue.pop() {
        processed += 1;
        for &next in &adjacency[node] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                queue.push(next);
            }
        }
    }
    processed != nodes
}

fn next_permutation(values: &mut [usize]) -> bool {
    let Some(pivot) = (0..values.len().saturating_sub(1)).rev().find(|&i| values[i] < values[i + 1]) else {
        return false;
    };
    let Some(swap) = (pivot + 1..values.len()).rev().find(|&i| values[pivot] < values[i]) else {
        return false;
    };
    values.swap(pivot, swap);
    values[pivot + 1..].reverse();
    true
}

/// Exact minimum-cost perfect matching over `Option` edges, for the fallback diagnosis only.
fn exact_fallback(costs: &[Vec<Option<f64>>], row_keys: &[Vec<f64>], col_keys: &[Vec<f64>]) -> Option<Vec<usize>> {
    let n = costs.len();
    if n == 0 {
        return Some(Vec::new());
    }
    if costs.iter().any(|row| row.len() != n) {
        return None;
    }
    let mut rows: Vec<usize> = (0..n).collect();
    rows.sort_by(|a, b| key_cmp(&row_keys[*a], &row_keys[*b]));
    let mut cols: Vec<usize> = (0..n).collect();
    cols.sort_by(|a, b| key_cmp(&col_keys[*a], &col_keys[*b]));
    let mut best_cost = f64::INFINITY;
    let mut best = vec![usize::MAX; n];
    let mut permutation: Vec<usize> = (0..n).collect();
    loop {
        let mut total = 0.0;
        let mut candidate = vec![usize::MAX; n];
        let mut complete = true;
        for (position, &row) in rows.iter().enumerate() {
            let col = cols[permutation[position]];
            match costs[row][col] {
                Some(cost) => {
                    total += cost;
                    candidate[row] = col;
                }
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if complete && total < best_cost {
            best_cost = total;
            best = candidate;
        }
        if !next_permutation(&mut permutation) {
            break;
        }
    }
    best_cost.is_finite().then_some(best)
}

fn greedy_fallback(costs: &[Vec<Option<f64>>], row_keys: &[Vec<f64>], col_keys: &[Vec<f64>]) -> Option<Vec<usize>> {
    let n = costs.len();
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for (row, row_costs) in costs.iter().enumerate() {
        for (col, cost) in row_costs.iter().enumerate() {
            if let Some(cost) = cost {
                candidates.push((*cost, row, col));
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| key_cmp(&row_keys[a.1], &row_keys[b.1]))
            .then_with(|| key_cmp(&col_keys[a.2], &col_keys[b.2]))
    });
    let mut used_rows = vec![false; n];
    let mut used_cols = vec![false; n];
    let mut matching = vec![usize::MAX; n];
    for (_, row, col) in candidates {
        if used_rows[row] || used_cols[col] {
            continue;
        }
        used_rows[row] = true;
        used_cols[col] = true;
        matching[row] = col;
    }
    (!matching.contains(&usize::MAX)).then_some(matching)
}

fn fallback_matching(
    costs: &[Vec<Option<f64>>],
    row_keys: &[Vec<f64>],
    col_keys: &[Vec<f64>],
) -> Option<Vec<usize>> {
    if costs.len() <= EXACT_FALLBACK_LIMIT {
        exact_fallback(costs, row_keys, col_keys)
    } else {
        greedy_fallback(costs, row_keys, col_keys)
    }
}

fn sorted_pairs(recorded_keys: &[Vec<f64>], measured_keys: &[Vec<f64>], matching: &[usize]) -> Vec<(usize, usize)> {
    let mut pairs: Vec<(usize, usize)> = matching
        .iter()
        .enumerate()
        .filter(|(_, col)| **col != usize::MAX)
        .map(|(row, &col)| (row, col))
        .collect();
    pairs.sort_by(|a, b| key_cmp(&recorded_keys[a.0], &recorded_keys[b.0]).then_with(|| key_cmp(&measured_keys[a.1], &measured_keys[b.1])));
    pairs
}

fn edge_point_delta(recorded_at: &[f64], measured_at: &[f64], measured_hole: [f64; 3], normal: Option<[f64; 3]>) -> Option<f64> {
    if recorded_at.len() < 3 || measured_at.len() < 3 {
        return None;
    }
    let normal = normal?;
    let length = (normal[0].powi(2) + normal[1].powi(2) + normal[2].powi(2)).sqrt();
    if !length.is_finite() || length <= 1e-12 {
        return None;
    }
    let normal = [normal[0] / length, normal[1] / length, normal[2] / length];
    // INTdfm reports the edge point on the sheet's skin; the App reports it at mid-depth. Remove
    // the component along the panel normal before comparing, so the half-thickness is not a
    // false mismatch. A real in-plane difference survives.
    let offset = (recorded_at[0] - measured_hole[0]) * normal[0]
        + (recorded_at[1] - measured_hole[1]) * normal[1]
        + (recorded_at[2] - measured_hole[2]) * normal[2];
    let projected = [recorded_at[0] - normal[0] * offset, recorded_at[1] - normal[1] * offset, recorded_at[2] - normal[2] * offset];
    Some(distance3(&projected, measured_at))
}

/// Compare recorded and measured facts. Returns the largest deltas and what was compared, or the
/// first decisive failure.
pub fn compare(recorded: &Facts, measured: &MeasuredSheet, tolerance: GeometryTolerance) -> Result<GeometryReport, GeometryMismatch> {
    check_tolerance("pairing_mm", tolerance.pairing_mm)?;
    check_tolerance("measurement_mm", tolerance.measurement_mm)?;
    if tolerance.measurement_mm > tolerance.pairing_mm {
        return Err(GeometryMismatch::InvalidTolerance { field: "measurement_mm", value: tolerance.measurement_mm });
    }
    validate_facts(recorded, FactSide::Recorded)?;
    validate_facts(&measured.facts, FactSide::Measured)?;
    validate_observations(measured)?;

    let thickness_recorded = recorded.paired_mm.ok_or(GeometryMismatch::RecordedThicknessUnavailable)?;
    let thickness_measured = measured.facts.paired_mm.ok_or(GeometryMismatch::MeasuredThicknessUnavailable)?;
    let thickness_delta = (thickness_recorded - thickness_measured).abs();
    if !within(thickness_delta, tolerance.measurement_mm) {
        return Err(GeometryMismatch::Thickness { delta_mm: thickness_delta });
    }

    let recorded_holes = applicable(recorded, true)?;
    let measured_holes = applicable(&measured.facts, false)?;
    if recorded_holes.is_empty() && measured_holes.is_empty() {
        return Err(GeometryMismatch::NoApplicableHole);
    }
    if recorded_holes.len() != measured_holes.len() {
        return Err(GeometryMismatch::HoleCount { recorded: recorded_holes.len(), measured: measured_holes.len() });
    }

    let recorded_keys: Vec<Vec<f64>> = recorded_holes.iter().map(|(_, hole)| hole_key(hole)).collect();
    let measured_keys: Vec<Vec<f64>> = measured_holes.iter().map(|(_, hole)| hole_key(hole)).collect();
    let recorded_order = canonical_order(&recorded_keys);
    let measured_order = canonical_order(&measured_keys);
    let recorded_canonical = canonical_numbers(&recorded_order);
    let measured_canonical = canonical_numbers(&measured_order);

    // The contract's allowed graph: position, diameter and depth each within `measurement_mm`.
    let allowed: Vec<Vec<bool>> = recorded_order
        .iter()
        .map(|&row| {
            measured_order
                .iter()
                .map(|&col| {
                    let recorded_hole = recorded_holes[row].1;
                    let measured_hole = measured_holes[col].1;
                    within(distance(recorded_hole, measured_hole), tolerance.measurement_mm)
                        && within((recorded_hole.diameter_mm - measured_hole.diameter_mm).abs(), tolerance.measurement_mm)
                        && within((recorded_hole.depth_mm - measured_hole.depth_mm).abs(), tolerance.measurement_mm)
                })
                .collect()
        })
        .collect();

    let matching = match find_perfect(&allowed) {
        Some(canonical_matching) => {
            if more_than_one(&allowed, &canonical_matching) {
                let row = (0..canonical_matching.len()).find(|row| canonical_matching[*row] != usize::MAX).unwrap_or(0);
                return Err(GeometryMismatch::AmbiguousPairing {
                    recorded: row + 1,
                    measured: canonical_matching.get(row).copied().unwrap_or(0) + 1,
                });
            }
            let mut matching = vec![usize::MAX; recorded_holes.len()];
            for (canonical_row, &canonical_col) in canonical_matching.iter().enumerate() {
                if canonical_col != usize::MAX {
                    matching[recorded_order[canonical_row]] = measured_order[canonical_col];
                }
            }
            matching
        }
        None => {
            // No complete matching. Use the 0.05 mm position graph only to diagnose.
            let positions: Vec<Vec<Option<f64>>> = recorded_holes
                .iter()
                .map(|(_, recorded_hole)| {
                    measured_holes
                        .iter()
                        .map(|(_, measured_hole)| {
                            let apart = distance(recorded_hole, measured_hole);
                            within(apart, tolerance.pairing_mm).then_some(apart)
                        })
                        .collect()
                })
                .collect();
            let Some(matching) = fallback_matching(&positions, &recorded_keys, &measured_keys) else {
                if let Some(row) = (0..recorded_holes.len()).find(|row| positions[*row].iter().all(Option::is_none)) {
                    return Err(GeometryMismatch::Unpaired { recorded: recorded_canonical[row], pairing_mm: tolerance.pairing_mm });
                }
                if let Some(col) = (0..measured_holes.len()).find(|col| (0..recorded_holes.len()).all(|row| positions[row][*col].is_none())) {
                    return Err(GeometryMismatch::UnpairedMeasured { measured: measured_canonical[col], pairing_mm: tolerance.pairing_mm });
                }
                return Err(GeometryMismatch::NoPerfectPairing);
            };
            matching
        }
    };

    let mut report = GeometryReport {
        pairs: recorded_holes.len(),
        thickness_delta_mm: thickness_delta,
        max_diameter_delta_mm: 0.0,
        max_depth_delta_mm: 0.0,
        max_position_delta_mm: 0.0,
        edge_distances_compared: 0,
        edge_walls_compared: 0,
        edge_points_compared: 0,
        edge_not_compared: 0,
        edge_points_not_compared: 0,
        max_edge_delta_mm: 0.0,
        max_edge_point_delta_mm: 0.0,
        bend_pairs: Vec::new(),
        bend_not_applicable: false,
        bend_gaps_compared: 0,
        max_bend_gap_delta_mm: 0.0,
        max_bend_radius_delta_mm: 0.0,
        max_bend_length_delta_mm: 0.0,
    };

    let pairs = sorted_pairs(&recorded_keys, &measured_keys, &matching);
    let mut diameter_failure: Option<(f64, usize, usize)> = None;
    let mut depth_failure: Option<(f64, usize, usize)> = None;
    let mut position_failure: Option<(f64, usize, usize)> = None;
    for &(row, col) in &pairs {
        let recorded_hole = recorded_holes[row].1;
        let measured_hole = measured_holes[col].1;
        let recorded_number = recorded_canonical[row];
        let measured_number = measured_canonical[col];
        let diameter = (recorded_hole.diameter_mm - measured_hole.diameter_mm).abs();
        if !within(diameter, tolerance.measurement_mm) {
            diameter_failure = Some(diameter_failure.map_or((diameter, recorded_number, measured_number), |current| {
                if diameter > current.0 { (diameter, recorded_number, measured_number) } else { current }
            }));
        }
        let depth = (recorded_hole.depth_mm - measured_hole.depth_mm).abs();
        if !within(depth, tolerance.measurement_mm) {
            depth_failure = Some(depth_failure.map_or((depth, recorded_number, measured_number), |current| {
                if depth > current.0 { (depth, recorded_number, measured_number) } else { current }
            }));
        }
        let position = distance(recorded_hole, measured_hole);
        if !within(position, tolerance.measurement_mm) {
            position_failure = Some(position_failure.map_or((position, recorded_number, measured_number), |current| {
                if position > current.0 { (position, recorded_number, measured_number) } else { current }
            }));
        }
        report.max_diameter_delta_mm = report.max_diameter_delta_mm.max(if diameter.is_finite() { diameter } else { 0.0 });
        report.max_depth_delta_mm = report.max_depth_delta_mm.max(if depth.is_finite() { depth } else { 0.0 });
        report.max_position_delta_mm = report.max_position_delta_mm.max(if position.is_finite() { position } else { 0.0 });
    }
    // Fixed priority, chosen after the whole pairing, so the reported variant does not depend on
    // input order.
    if let Some((delta_mm, recorded, measured)) = diameter_failure {
        return Err(GeometryMismatch::Diameter { recorded, measured, delta_mm });
    }
    if let Some((delta_mm, recorded, measured)) = depth_failure {
        return Err(GeometryMismatch::Depth { recorded, measured, delta_mm });
    }
    if let Some((delta_mm, recorded, measured)) = position_failure {
        return Err(GeometryMismatch::Position { recorded, measured, delta_mm });
    }

    // Canonical hole number maps, for edge ownership and bend associations.
    let recorded_original_to_row: BTreeMap<usize, usize> = recorded_holes.iter().enumerate().map(|(row, (number, _))| (*number, row)).collect();
    let measured_original_to_row: BTreeMap<usize, usize> = measured_holes.iter().enumerate().map(|(row, (number, _))| (*number, row)).collect();
    let hole_map: BTreeMap<usize, usize> =
        pairs.iter().map(|&(row, col)| (recorded_canonical[row], measured_canonical[col])).collect();

    let extras = |number: usize| measured.holes.get(number.saturating_sub(1)).cloned().unwrap_or_default();
    for &(row, col) in &pairs {
        let recorded_hole = recorded_holes[row].1;
        let measured_hole = measured_holes[col].1;
        let recorded_number = recorded_canonical[row];
        let measured_number = measured_canonical[col];
        let extra = extras(measured_holes[col].0);
        if let Some(wall_hole) = recorded_hole.edge_hole {
            let Some(&wall_row) = recorded_original_to_row.get(&wall_hole) else {
                return Err(GeometryMismatch::EdgeWallUnmapped { recorded: recorded_number, wall_hole });
            };
            let Some(&expected) = hole_map.get(&recorded_canonical[wall_row]) else {
                return Err(GeometryMismatch::EdgeWallUnmapped { recorded: recorded_number, wall_hole });
            };
            let Some(wall) = &extra.wall else {
                // The recording says this edge is a hole wall; the App measured a cut edge (or
                // no wall fact). That is an ownership failure, not a silent fallback.
                if measured_hole.edge_mm.is_some() {
                    return Err(GeometryMismatch::EdgeOwnership {
                        recorded: recorded_number,
                        measured: measured_number,
                        recorded_hole: Some(wall_hole),
                        measured_wall: None,
                    });
                }
                return Err(GeometryMismatch::MeasuredEdgeWallUnavailable { measured: measured_number, recorded_hole: wall_hole });
            };
            let Some(actual) = measured_original_to_row.get(&wall.number).map(|row| measured_canonical[*row]) else {
                return Err(GeometryMismatch::MeasuredEdgeWallUnavailable { measured: measured_number, recorded_hole: wall_hole });
            };
            if actual != expected {
                return Err(GeometryMismatch::EdgeWallNumber {
                    recorded: recorded_number,
                    measured: measured_number,
                    expected,
                    actual,
                });
            }
            if recorded_hole.edge_kind != Some(EdgeKind::Opening) {
                return Err(GeometryMismatch::EdgeKindMismatch {
                    recorded: recorded_number,
                    measured: measured_number,
                    recorded_kind: recorded_hole.edge_kind,
                    measured_kind: Some(EdgeKind::Opening),
                });
            }
            let Some(recorded_edge) = recorded_hole.edge_mm else {
                return Err(GeometryMismatch::RecordedEdgeUnavailable { recorded: recorded_number });
            };
            let delta = (recorded_edge - wall.gap_mm).abs();
            if !within(delta, tolerance.measurement_mm) {
                return Err(GeometryMismatch::EdgeDistance { recorded: recorded_number, measured: measured_number, delta_mm: delta });
            }
            report.max_edge_delta_mm = report.max_edge_delta_mm.max(delta);
            report.edge_distances_compared += 1;
            report.edge_walls_compared += 1;
            match (&recorded_hole.edge_at_mm, &extra.normal) {
                (Some(recorded_at), Some(normal)) => {
                    let at = wall.at_mm;
                    let Some(point_delta) = edge_point_delta(recorded_at, &at, [measured_hole.x_mm, measured_hole.y_mm, measured_hole.z_mm], Some(*normal)) else {
                        report.edge_points_not_compared += 1;
                        continue;
                    };
                    if !within(point_delta, tolerance.measurement_mm) {
                        return Err(GeometryMismatch::EdgePoint { recorded: recorded_number, measured: measured_number, delta_mm: point_delta });
                    }
                    report.max_edge_point_delta_mm = report.max_edge_point_delta_mm.max(point_delta);
                    report.edge_points_compared += 1;
                }
                (None, _) => return Err(GeometryMismatch::EdgePointUnavailable { recorded: recorded_number, measured: measured_number }),
                _ => report.edge_points_not_compared += 1,
            }
            continue;
        }

        match (recorded_hole.edge_mm, measured_hole.edge_mm) {
            (None, None) => {
                report.edge_not_compared += 1;
                continue;
            }
            (Some(_), None) => return Err(GeometryMismatch::MeasuredEdgeUnavailable { measured: measured_number }),
            (None, Some(_)) => return Err(GeometryMismatch::RecordedEdgeUnavailable { recorded: recorded_number }),
            (Some(recorded_edge), Some(measured_edge)) => {
                let delta = (recorded_edge - measured_edge).abs();
                if !within(delta, tolerance.measurement_mm) {
                    return Err(GeometryMismatch::EdgeDistance { recorded: recorded_number, measured: measured_number, delta_mm: delta });
                }
                report.max_edge_delta_mm = report.max_edge_delta_mm.max(delta);
                report.edge_distances_compared += 1;
            }
        }
        if recorded_hole.edge_kind != measured_hole.edge_kind {
            return Err(GeometryMismatch::EdgeKindMismatch {
                recorded: recorded_number,
                measured: measured_number,
                recorded_kind: recorded_hole.edge_kind,
                measured_kind: measured_hole.edge_kind,
            });
        }
        match (&recorded_hole.edge_at_mm, &measured_hole.edge_at_mm) {
            (Some(recorded_at), Some(measured_at)) => match &extra.normal {
                Some(normal) => {
                    let Some(point_delta) =
                        edge_point_delta(recorded_at, measured_at, [measured_hole.x_mm, measured_hole.y_mm, measured_hole.z_mm], Some(*normal))
                    else {
                        report.edge_points_not_compared += 1;
                        continue;
                    };
                    if !within(point_delta, tolerance.measurement_mm) {
                        return Err(GeometryMismatch::EdgePoint { recorded: recorded_number, measured: measured_number, delta_mm: point_delta });
                    }
                    report.max_edge_point_delta_mm = report.max_edge_point_delta_mm.max(point_delta);
                    report.edge_points_compared += 1;
                }
                None => report.edge_points_not_compared += 1,
            },
            (None, None) => {}
            _ => return Err(GeometryMismatch::EdgePointUnavailable { recorded: recorded_number, measured: measured_number }),
        }
    }

    // Bends: pair by folded position, then compare radius, length and flat association. Raw bend
    // numbers are never compared; all sets are carried in canonical numbers.
    let recognition_recorded = recorded.recognition.as_ref().ok_or(GeometryMismatch::RecordedRecognitionUnavailable)?;
    let recognition_measured = measured.facts.recognition.as_ref().ok_or(GeometryMismatch::MeasuredRecognitionUnavailable)?;
    let recorded_bends: Vec<&BendFact> = recognition_recorded.bends.iter().collect();
    let measured_bends: Vec<&BendFact> = recognition_measured.bends.iter().collect();
    if recorded_bends.is_empty() && measured_bends.is_empty() {
        report.bend_not_applicable = true;
    } else if recorded_bends.len() != measured_bends.len() {
        return Err(GeometryMismatch::BendCount { recorded: recorded_bends.len(), measured: measured_bends.len() });
    } else {
        let recorded_bend_keys: Vec<Vec<f64>> = recorded_bends.iter().map(|bend| bend_key(bend)).collect();
        let measured_bend_keys: Vec<Vec<f64>> = measured_bends.iter().map(|bend| bend_key(bend)).collect();
        let recorded_bend_order = canonical_order(&recorded_bend_keys);
        let measured_bend_order = canonical_order(&measured_bend_keys);
        let recorded_bend_canonical = canonical_numbers(&recorded_bend_order);
        let measured_bend_canonical = canonical_numbers(&measured_bend_order);
        let allowed: Vec<Vec<bool>> = recorded_bend_order
            .iter()
            .map(|&row| {
                measured_bend_order
                    .iter()
                    .map(|&col| {
                        let apart = distance3(
                            &[recorded_bends[row].x_mm, recorded_bends[row].y_mm, recorded_bends[row].z_mm],
                            &[measured_bends[col].x_mm, measured_bends[col].y_mm, measured_bends[col].z_mm],
                        );
                        within(apart, tolerance.pairing_mm)
                    })
                    .collect()
            })
            .collect();
        let Some(canonical_matching) = find_perfect(&allowed) else {
            if let Some(row) = (0..recorded_bend_order.len()).find(|row| allowed[*row].iter().all(|allowed| !allowed)) {
                return Err(GeometryMismatch::BendUnpaired {
                    recorded: recorded_bend_canonical[recorded_bend_order[row]],
                    pairing_mm: tolerance.pairing_mm,
                });
            }
            if let Some(col) = (0..measured_bend_order.len()).find(|col| (0..recorded_bend_order.len()).all(|row| !allowed[row][*col])) {
                return Err(GeometryMismatch::BendUnpairedMeasured {
                    measured: measured_bend_canonical[measured_bend_order[col]],
                    pairing_mm: tolerance.pairing_mm,
                });
            }
            return Err(GeometryMismatch::NoPerfectBendPairing);
        };
        if more_than_one(&allowed, &canonical_matching) {
            let row = (0..canonical_matching.len()).find(|row| canonical_matching[*row] != usize::MAX).unwrap_or(0);
            return Err(GeometryMismatch::AmbiguousBendPairing {
                recorded: row + 1,
                measured: canonical_matching.get(row).copied().unwrap_or(0) + 1,
            });
        }
        let mut matching = vec![usize::MAX; recorded_bends.len()];
        for (canonical_row, &canonical_col) in canonical_matching.iter().enumerate() {
            if canonical_col != usize::MAX {
                matching[recorded_bend_order[canonical_row]] = measured_bend_order[canonical_col];
            }
        }
        let bend_pairs = sorted_pairs(&recorded_bend_keys, &measured_bend_keys, &matching);
        let mut bend_map: BTreeMap<usize, usize> = BTreeMap::new();
        for &(row, col) in &bend_pairs {
            bend_map.insert(recorded_bend_canonical[row], measured_bend_canonical[col]);
            let recorded_bend = recorded_bends[row];
            let measured_bend = measured_bends[col];
            let radius = (recorded_bend.inside_radius_mm - measured_bend.inside_radius_mm).abs();
            if !within(radius, tolerance.measurement_mm) {
                return Err(GeometryMismatch::BendRadius {
                    recorded: recorded_bend_canonical[row],
                    measured: measured_bend_canonical[col],
                    delta_mm: radius,
                });
            }
            let length = (recorded_bend.length_mm - measured_bend.length_mm).abs();
            if !within(length, tolerance.measurement_mm) {
                return Err(GeometryMismatch::BendLength {
                    recorded: recorded_bend_canonical[row],
                    measured: measured_bend_canonical[col],
                    delta_mm: length,
                });
            }
            report.max_bend_radius_delta_mm = report.max_bend_radius_delta_mm.max(radius);
            report.max_bend_length_delta_mm = report.max_bend_length_delta_mm.max(length);
            report.bend_pairs.push((recorded_bend_canonical[row], measured_bend_canonical[col]));
        }
        report.bend_pairs.sort_unstable();

        let recorded_bend_original_to_row: BTreeMap<usize, usize> =
            recorded_bends.iter().enumerate().map(|(row, _)| (row + 1, row)).collect();
        let measured_bend_original_to_row: BTreeMap<usize, usize> =
            measured_bends.iter().enumerate().map(|(row, _)| (row + 1, row)).collect();

        // A hole's flat association is which paired bends bound its panel, plus whether the
        // bend's own flat list names that panel. Numeric flat ids themselves never cross sides.
        for &(row, col) in &pairs {
            let recorded_hole = recorded_holes[row].1;
            let measured_hole = measured_holes[col].1;
            let recorded_number = recorded_canonical[row];
            let measured_number = measured_canonical[col];
            let recorded_gaps = &recorded_hole.bend_gaps;
            let measured_gaps = &measured_hole.bend_gaps;
            if recorded_gaps.is_empty() && measured_gaps.is_empty() {
                return Err(GeometryMismatch::BendGapsUnavailable { recorded: recorded_number });
            }
            if recorded_gaps.is_empty() {
                return Err(GeometryMismatch::RecordedBendGapsUnavailable { recorded: recorded_number });
            }
            if measured_gaps.is_empty() {
                return Err(GeometryMismatch::MeasuredBendGapsUnavailable { measured: measured_number });
            }
            let mut measured_by_bend: BTreeMap<usize, &BendGap> = BTreeMap::new();
            for gap in measured_gaps {
                let Some(&bend_row) = measured_bend_original_to_row.get(&gap.bend) else {
                    return Err(GeometryMismatch::BendGapUnmapped { recorded: recorded_number, bend: gap.bend });
                };
                let canonical = measured_bend_canonical[bend_row];
                if measured_by_bend.insert(canonical, gap).is_some() {
                    return Err(GeometryMismatch::BendGapDuplicate { recorded: recorded_number, measured: measured_number, bend: canonical });
                }
            }
            let mut recorded_seen: BTreeMap<usize, &BendGap> = BTreeMap::new();
            for gap in recorded_gaps {
                let Some(&bend_row) = recorded_bend_original_to_row.get(&gap.bend) else {
                    return Err(GeometryMismatch::BendGapUnmapped { recorded: recorded_number, bend: gap.bend });
                };
                let canonical = recorded_bend_canonical[bend_row];
                let Some(&mapped) = bend_map.get(&canonical) else {
                    return Err(GeometryMismatch::BendGapUnmapped { recorded: recorded_number, bend: canonical });
                };
                if recorded_seen.insert(mapped, gap).is_some() {
                    return Err(GeometryMismatch::BendGapDuplicate { recorded: recorded_number, measured: measured_number, bend: mapped });
                }
            }
            for (&bend, gap) in &recorded_seen {
                let Some(measured_gap) = measured_by_bend.get(&bend) else {
                    return Err(GeometryMismatch::BendGapMissing { recorded: recorded_number, measured: measured_number, bend });
                };
                let delta = (gap.gap_mm - measured_gap.gap_mm).abs();
                if !within(delta, tolerance.measurement_mm) {
                    return Err(GeometryMismatch::BendGap { recorded: recorded_number, measured: measured_number, bend, delta_mm: delta });
                }
                let radius = (gap.inside_radius_mm - measured_gap.inside_radius_mm).abs();
                if !within(radius, tolerance.measurement_mm) {
                    return Err(GeometryMismatch::BendGapRadius { recorded: recorded_number, measured: measured_number, bend, delta_mm: radius });
                }
                report.max_bend_gap_delta_mm = report.max_bend_gap_delta_mm.max(delta);
                report.bend_gaps_compared += 1;
            }
            for &bend in measured_by_bend.keys() {
                if !recorded_seen.contains_key(&bend) {
                    return Err(GeometryMismatch::BendGapExtra { recorded: recorded_number, measured: measured_number, bend });
                }
            }

            if recorded_hole.flat.is_some() {
                for &(recorded_bend_row, measured_bend_row) in &bend_pairs {
                    let recorded_bend = recorded_bends[recorded_bend_row];
                    let measured_bend = measured_bends[measured_bend_row];
                    let on_recorded = recorded_bend.flats.contains(&recorded_hole.flat);
                    let on_measured = measured_bend.flats.contains(&measured_hole.flat);
                    if on_recorded != on_measured {
                        return Err(GeometryMismatch::BendFlatAssociation {
                            recorded: recorded_number,
                            measured: measured_number,
                            bend: recorded_bend_canonical[recorded_bend_row],
                        });
                    }
                }
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BendFact, BendGap, Recognition};

    fn hole_d(x: f64, diameter: f64, depth: f64) -> HoleFact {
        HoleFact {
            diameter_mm: diameter,
            x_mm: x,
            y_mm: 12.5,
            z_mm: 0.75,
            depth_mm: depth,
            square: true,
            edge_mm: None,
            edge_at_mm: None,
            edge_kind: None,
            edge_hole: None,
            bend_gaps: Vec::new(),
            flat: None,
        }
    }

    fn hole(x: f64) -> HoleFact {
        hole_d(x, 3.0, 1.5)
    }

    fn tilted(x: f64) -> HoleFact {
        let mut fact = hole(x);
        fact.square = false;
        fact
    }

    fn facts(holes: Vec<HoleFact>) -> Facts {
        Facts { paired_mm: Some(1.5), holes: Some(holes), recognition: Some(Recognition::default()), ..Facts::default() }
    }

    fn measured(holes: Vec<HoleFact>) -> MeasuredSheet {
        MeasuredSheet::bare(facts(holes))
    }

    fn bend(x: f64) -> BendFact {
        BendFact { inside_radius_mm: 3.0, angle_deg: 90.0, length_mm: 25.0, x_mm: x, y_mm: 12.5, z_mm: 0.0, flats: vec![Some(1), Some(2)] }
    }

    fn with_bends(mut facts: Facts, bends: Vec<BendFact>) -> Facts {
        facts.recognition = Some(Recognition { bends, ..Recognition::default() });
        facts
    }

    fn gap(bend: usize, gap_mm: f64) -> BendGap {
        BendGap { bend, gap_mm, inside_radius_mm: 3.0 }
    }

    #[test]
    fn equal_facts_pass_and_say_what_was_not_compared() {
        let report = compare(&facts(vec![hole(20.0)]), &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect("equal");
        assert_eq!(report.pairs, 1);
        assert_eq!(report.max_diameter_delta_mm, 0.0);
        assert_eq!(report.edge_not_compared, 1);
        assert!(report.bend_not_applicable);
    }

    #[test]
    fn a_diameter_outside_the_measurement_tolerance_fails() {
        let changed = hole_d(20.0, 3.01, 1.5);
        let mismatch = compare(&facts(vec![hole(20.0)]), &measured(vec![changed]), GeometryTolerance::default()).expect_err("diameter");
        assert!(matches!(mismatch, GeometryMismatch::Diameter { .. }));
    }

    #[test]
    fn a_position_outside_the_measurement_tolerance_fails() {
        let mismatch = compare(&facts(vec![hole(20.0)]), &measured(vec![hole(20.02)]), GeometryTolerance::default()).expect_err("position");
        assert!(matches!(mismatch, GeometryMismatch::Position { .. }));
    }

    #[test]
    fn a_missing_hole_fails() {
        let mismatch = compare(&facts(vec![hole(20.0), hole(30.0)]), &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("count");
        assert!(matches!(mismatch, GeometryMismatch::HoleCount { .. }));
    }

    #[test]
    fn two_holes_at_one_place_are_ambiguous() {
        let mismatch = compare(&facts(vec![hole(20.0), hole(20.0)]), &measured(vec![hole(20.0), hole(20.0)]), GeometryTolerance::default());
        assert!(matches!(mismatch, Err(GeometryMismatch::AmbiguousPairing { .. })));
    }

    #[test]
    fn a_thickness_outside_the_measurement_tolerance_fails() {
        let mut measured = measured(vec![hole(20.0)]);
        measured.facts.paired_mm = Some(1.51);
        let mismatch = compare(&facts(vec![hole(20.0)]), &measured, GeometryTolerance::default()).expect_err("thickness");
        assert!(matches!(mismatch, GeometryMismatch::Thickness { .. }));
    }

    #[test]
    fn no_applicable_hole_on_either_side_is_a_distinct_result() {
        let mismatch = compare(&facts(Vec::new()), &measured(Vec::new()), GeometryTolerance::default()).expect_err("nothing to measure");
        assert!(matches!(mismatch, GeometryMismatch::NoApplicableHole));
    }

    #[test]
    fn a_missing_hole_list_is_measurement_unavailable() {
        let mut recorded = facts(vec![hole(20.0)]);
        recorded.holes = None;
        let mismatch = compare(&recorded, &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("no holes");
        assert!(matches!(mismatch, GeometryMismatch::RecordedHolesUnavailable));
        let mut measured = measured(vec![hole(20.0)]);
        measured.facts.holes = None;
        let mismatch = compare(&facts(vec![hole(20.0)]), &measured, GeometryTolerance::default()).expect_err("no holes");
        assert!(matches!(mismatch, GeometryMismatch::MeasuredHolesUnavailable));
    }

    #[test]
    fn a_missing_recognition_is_measurement_unavailable() {
        let mut recorded = facts(vec![hole(20.0)]);
        recorded.recognition = None;
        let mismatch = compare(&recorded, &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("no recognition");
        assert!(matches!(mismatch, GeometryMismatch::RecordedRecognitionUnavailable));
        let mut measured = measured(vec![hole(20.0)]);
        measured.facts.recognition = None;
        let mismatch = compare(&facts(vec![hole(20.0)]), &measured, GeometryTolerance::default()).expect_err("no recognition");
        assert!(matches!(mismatch, GeometryMismatch::MeasuredRecognitionUnavailable));
    }

    #[test]
    fn thickness_unavailable_names_the_side() {
        let mut recorded = facts(vec![hole(20.0)]);
        recorded.paired_mm = None;
        let mismatch = compare(&recorded, &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("recorded thickness");
        assert!(matches!(mismatch, GeometryMismatch::RecordedThicknessUnavailable));
        let mut measured = measured(vec![hole(20.0)]);
        measured.facts.paired_mm = None;
        let mismatch = compare(&facts(vec![hole(20.0)]), &measured, GeometryTolerance::default()).expect_err("measured thickness");
        assert!(matches!(mismatch, GeometryMismatch::MeasuredThicknessUnavailable));
    }

    #[test]
    fn an_edge_on_one_side_only_is_measurement_unavailable() {
        let mut recorded = facts(vec![hole(20.0)]);
        recorded.holes.as_mut().expect("holes")[0].edge_mm = Some(11.0);
        let mismatch = compare(&recorded, &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("one edge");
        assert!(matches!(mismatch, GeometryMismatch::MeasuredEdgeUnavailable { .. }));

        let mut measured_hole = hole(20.0);
        measured_hole.edge_mm = Some(11.0);
        let mismatch = compare(&facts(vec![hole(20.0)]), &measured(vec![measured_hole]), GeometryTolerance::default()).expect_err("one edge");
        assert!(matches!(mismatch, GeometryMismatch::RecordedEdgeUnavailable { .. }));
    }

    #[test]
    fn an_edge_distance_or_kind_difference_fails() {
        let mut recorded = facts(vec![hole(20.0)]);
        let mut measured_hole = hole(20.0);
        recorded.holes.as_mut().expect("holes")[0].edge_mm = Some(11.0);
        recorded.holes.as_mut().expect("holes")[0].edge_kind = Some(EdgeKind::Outline);
        measured_hole.edge_mm = Some(11.01);
        measured_hole.edge_kind = Some(EdgeKind::Outline);
        let mismatch = compare(&recorded, &measured(vec![measured_hole.clone()]), GeometryTolerance::default()).expect_err("distance");
        assert!(matches!(mismatch, GeometryMismatch::EdgeDistance { .. }));

        measured_hole.edge_mm = Some(11.0);
        measured_hole.edge_kind = Some(EdgeKind::Opening);
        let mismatch = compare(&recorded, &measured(vec![measured_hole]), GeometryTolerance::default()).expect_err("kind");
        assert!(matches!(mismatch, GeometryMismatch::EdgeKindMismatch { .. }));
    }

    #[test]
    fn a_recorded_edge_hole_is_mapped_to_the_measured_wall() {
        let mut recorded = facts(vec![hole(20.0), hole(30.0)]);
        recorded.holes.as_mut().expect("holes")[0].edge_mm = Some(1.0);
        recorded.holes.as_mut().expect("holes")[0].edge_at_mm = Some(vec![22.5, 12.5, 0.75]);
        recorded.holes.as_mut().expect("holes")[0].edge_kind = Some(EdgeKind::Opening);
        recorded.holes.as_mut().expect("holes")[0].edge_hole = Some(2);
        let mut measured_sheet = measured(vec![hole(20.0), hole(30.0)]);
        measured_sheet.holes[0].wall = Some(HoleWallFact { number: 2, gap_mm: 1.0, at_mm: [22.5, 12.5, 0.75] });
        measured_sheet.holes[0].normal = Some([0.0, 0.0, 1.0]);
        let report = compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect("mapped wall");
        assert_eq!(report.edge_walls_compared, 1);
        assert_eq!(report.edge_points_compared, 1);

        measured_sheet.holes[0].wall = Some(HoleWallFact { number: 1, gap_mm: 1.0, at_mm: [22.5, 12.5, 0.75] });
        let mismatch = compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect_err("wrong wall");
        assert!(matches!(mismatch, GeometryMismatch::EdgeWallNumber { .. }));

        measured_sheet.holes[0].wall = None;
        let mismatch = compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect_err("no wall");
        assert!(matches!(mismatch, GeometryMismatch::MeasuredEdgeWallUnavailable { .. }));
    }

    #[test]
    fn an_edge_point_is_compared_after_removing_the_half_thickness() {
        let mut recorded = facts(vec![hole(20.0)]);
        let mut measured_hole = hole(20.0);
        recorded.holes.as_mut().expect("holes")[0].edge_mm = Some(11.0);
        recorded.holes.as_mut().expect("holes")[0].edge_at_mm = Some(vec![20.0, 0.0, 1.5]);
        measured_hole.edge_mm = Some(11.0);
        measured_hole.edge_at_mm = Some(vec![20.0, 0.0, 0.75]);
        let mut sheet = measured(vec![measured_hole.clone()]);
        sheet.holes[0].normal = Some([0.0, 0.0, 1.0]);
        let report = compare(&recorded, &sheet, GeometryTolerance::default()).expect("same point at mid-depth");
        assert_eq!(report.edge_points_compared, 1);

        sheet.facts.holes.as_mut().expect("holes")[0].edge_at_mm = Some(vec![20.0, 0.02, 0.75]);
        let mismatch = compare(&recorded, &sheet, GeometryTolerance::default()).expect_err("point moved in plane");
        assert!(matches!(mismatch, GeometryMismatch::EdgePoint { .. }));
    }

    #[test]
    fn bends_are_paired_by_position_not_number() {
        let mut recorded = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0), bend(30.0)]);
        recorded.holes.as_mut().expect("holes")[0].bend_gaps = vec![gap(1, 6.5), gap(2, 20.5)];
        let mut measured_facts = with_bends(facts(vec![hole(20.0)]), vec![bend(30.0), bend(10.0)]);
        measured_facts.holes.as_mut().expect("holes")[0].bend_gaps = vec![gap(2, 6.5), gap(1, 20.5)];
        let report = compare(&recorded, &MeasuredSheet::bare(measured_facts), GeometryTolerance::default()).expect("mapped bends");
        // Canonical numbering sorts by bent position, so the report is the identity even though
        // recorded bend 1 (at x=10) is measured bend 2; the gap comparison above proves the map.
        assert_eq!(report.bend_pairs, vec![(1, 1), (2, 2)]);
        assert_eq!(report.bend_gaps_compared, 2);

        let mut swapped = with_bends(facts(vec![hole(20.0)]), vec![bend(30.0), bend(10.0)]);
        swapped.holes.as_mut().expect("holes")[0].bend_gaps = vec![gap(1, 6.5), gap(2, 20.5)];
        let mismatch = compare(&recorded, &MeasuredSheet::bare(swapped), GeometryTolerance::default()).expect_err("raw-number association");
        assert!(matches!(mismatch, GeometryMismatch::BendGap { .. }), "{mismatch}");
    }

    #[test]
    fn a_bend_radius_or_gap_difference_fails() {
        let mut recorded = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0)]);
        recorded.holes.as_mut().expect("holes")[0].bend_gaps = vec![gap(1, 6.5)];
        let mut measured_facts = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0)]);
        measured_facts.holes.as_mut().expect("holes")[0].bend_gaps = vec![gap(1, 6.51)];
        let mismatch = compare(&recorded, &MeasuredSheet::bare(measured_facts), GeometryTolerance::default()).expect_err("gap");
        assert!(matches!(mismatch, GeometryMismatch::BendGap { .. }));

        let mut measured_facts = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0)]);
        measured_facts.holes.as_mut().expect("holes")[0].bend_gaps = vec![BendGap { bend: 1, gap_mm: 6.5, inside_radius_mm: 3.01 }];
        let mismatch = compare(&recorded, &MeasuredSheet::bare(measured_facts), GeometryTolerance::default()).expect_err("gap radius");
        assert!(matches!(mismatch, GeometryMismatch::BendGapRadius { .. }));

        let mut measured_facts = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0)]);
        measured_facts.holes.as_mut().expect("holes")[0].bend_gaps = vec![gap(1, 6.5)];
        measured_facts.recognition.as_mut().expect("recognition").bends[0].inside_radius_mm = 3.01;
        let mismatch = compare(&recorded, &MeasuredSheet::bare(measured_facts), GeometryTolerance::default()).expect_err("bend radius");
        assert!(matches!(mismatch, GeometryMismatch::BendRadius { .. }));
    }

    #[test]
    fn a_bend_association_difference_fails() {
        let mut recorded = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0), bend(30.0)]);
        recorded.holes.as_mut().expect("holes")[0].bend_gaps = vec![gap(1, 6.5)];
        let mut measured_facts = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0), bend(30.0)]);
        measured_facts.holes.as_mut().expect("holes")[0].bend_gaps = vec![gap(2, 6.5)];
        let mismatch = compare(&recorded, &MeasuredSheet::bare(measured_facts), GeometryTolerance::default()).expect_err("association");
        assert!(matches!(mismatch, GeometryMismatch::BendGapExtra { .. } | GeometryMismatch::BendGapMissing { .. }));
    }

    #[test]
    fn an_empty_gap_list_with_bends_is_measurement_unavailable() {
        let recorded = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0)]);
        let measured_facts = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0)]);
        let mismatch = compare(&recorded, &MeasuredSheet::bare(measured_facts), GeometryTolerance::default()).expect_err("no gaps");
        assert!(matches!(mismatch, GeometryMismatch::BendGapsUnavailable { .. }));
    }

    #[test]
    fn a_bend_on_one_side_only_is_a_count_mismatch() {
        let recorded = with_bends(facts(vec![hole(20.0)]), vec![bend(10.0)]);
        let measured_facts = with_bends(facts(vec![hole(20.0)]), Vec::new());
        let mismatch = compare(&recorded, &MeasuredSheet::bare(measured_facts), GeometryTolerance::default()).expect_err("count");
        assert!(matches!(mismatch, GeometryMismatch::BendCount { .. }));
    }

    /// The audit's false-ambiguity counterexample: R1=(0, d3), R2=(0.002, d5);
    /// M1=(0.0015, d5), M2=(0.0005, d3). The unique measurement-consistent matching is
    /// R1-M2 and R2-M1. A near-distance alternative R1-M1 (0.0015 vs 0.0005) is 2 mm wrong in
    /// diameter and cannot belong to a complete matching, so it must not be ambiguous.
    #[test]
    fn audit_false_ambiguity_counterexample_passes() {
        let recorded = facts(vec![hole_d(0.0, 3.0, 1.5), hole_d(0.002, 5.0, 1.5)]);
        let measured_sheet = measured(vec![hole_d(0.0015, 5.0, 1.5), hole_d(0.0005, 3.0, 1.5)]);
        let report = compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect("unique compatible matching");
        assert_eq!(report.pairs, 2);
        assert!(report.max_position_delta_mm <= 0.001);
    }

    /// The audit's missing-partner case: M2 is 10 mm away, so no complete matching exists. The
    /// fallback must name M2, not pre-empt with an ambiguity.
    #[test]
    fn audit_competing_candidates_report_the_missing_partner() {
        let recorded = facts(vec![hole(0.0), hole(0.03)]);
        let measured_sheet = measured(vec![hole(0.015), hole(10.0)]);
        let mismatch = compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect_err("one measured hole");
        assert!(matches!(mismatch, GeometryMismatch::UnpairedMeasured { measured: 2, .. }), "{mismatch}");
    }

    /// The audit's order-dependence case: the same physical failure must report the same variant
    /// (Diameter, the fixed first priority) whichever way the facts are ordered.
    #[test]
    fn audit_reported_variant_is_order_independent() {
        let recorded_a = facts(vec![hole_d(0.0, 3.0, 1.5), hole_d(1.0, 3.0, 1.5)]);
        let measured_a = measured(vec![hole_d(0.03125, 3.02, 1.5), hole_d(1.03125, 3.0, 1.5)]);
        let first = compare(&recorded_a, &measured_a, GeometryTolerance::default()).expect_err("diameter");
        assert!(matches!(first, GeometryMismatch::Diameter { .. }), "{first}");

        let recorded_b = facts(vec![hole_d(1.0, 3.0, 1.5), hole_d(0.0, 3.0, 1.5)]);
        let measured_b = measured(vec![hole_d(1.03125, 3.0, 1.5), hole_d(0.03125, 3.02, 1.5)]);
        let second = compare(&recorded_b, &measured_b, GeometryTolerance::default()).expect_err("diameter");
        assert!(matches!(second, GeometryMismatch::Diameter { .. }), "{second}");
        assert_eq!(first, second);
    }

    /// Greedy would pair R1 with M1 first and then fail; the exact fallback finds the only
    /// complete matching and reports the position mismatch.
    #[test]
    fn a_greedy_counterexample_is_solved_by_the_exact_fallback() {
        let recorded = facts(vec![hole(20.010), hole(19.951)]);
        let measured_sheet = measured(vec![hole(20.0), hole(20.049)]);
        let mismatch = compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect_err("position");
        assert!(matches!(mismatch, GeometryMismatch::Position { .. }), "{mismatch}");
    }

    #[test]
    fn a_filtered_hole_before_a_usable_one_does_not_panic_and_is_order_independent() {
        let recorded_a = facts(vec![tilted(0.0), hole(20.0)]);
        let measured_sheet = measured(vec![hole(20.0)]);
        let first = compare(&recorded_a, &measured_sheet, GeometryTolerance::default()).expect("filtered first");
        assert_eq!(first.pairs, 1);

        let recorded_b = facts(vec![hole(20.0), tilted(0.0)]);
        let second = compare(&recorded_b, &measured_sheet, GeometryTolerance::default()).expect("filtered last");
        assert_eq!(second.pairs, 1);
    }

    #[test]
    fn a_large_depth_error_is_reported_as_depth_not_a_count_change() {
        let recorded = facts(vec![hole(20.0)]);
        let measured_sheet = measured(vec![hole_d(20.0, 3.0, 1.65)]);
        let mismatch = compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect_err("depth");
        assert!(matches!(mismatch, GeometryMismatch::Depth { .. }), "{mismatch}");
    }

    #[test]
    fn pairing_and_measurement_boundaries_do_not_depend_on_coordinate_magnitude() {
        for recorded_x in [0.0, 0.09, 4.0] {
            let measured_x = recorded_x + 0.05;
            let mismatch = compare(&facts(vec![hole(recorded_x)]), &measured(vec![hole(measured_x)]), GeometryTolerance::default())
                .expect_err("0.05 is a pairing, not a measurement");
            assert!(matches!(mismatch, GeometryMismatch::Position { .. }), "{recorded_x}: {mismatch}");
        }
        for recorded_x in [0.0, 1.5, 4.0] {
            compare(&facts(vec![hole(recorded_x)]), &measured(vec![hole(recorded_x + 0.001)]), GeometryTolerance::default())
                .expect("0.001 is inclusive");
            let mismatch =
                compare(&facts(vec![hole(recorded_x)]), &measured(vec![hole(recorded_x + 0.0011)]), GeometryTolerance::default())
                    .expect_err("0.0011 is past");
            assert!(matches!(mismatch, GeometryMismatch::Position { .. }), "{recorded_x}: {mismatch}");
        }
        let mut measured_sheet = measured(vec![hole(20.0)]);
        measured_sheet.facts.paired_mm = Some(4.001);
        let mut recorded = facts(vec![hole(20.0)]);
        recorded.paired_mm = Some(4.0);
        compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect("thickness 0.001 at 4 mm is inclusive");
    }

    #[test]
    fn the_comparison_quantum_rounds_a_delta_before_deciding() {
        // 0.0010004 rounds to 0.001 at the contract's 1e-6 quantum and passes.
        compare(&facts(vec![hole(4.0)]), &measured(vec![hole(4.0010004)]), GeometryTolerance::default()).expect("rounds down");
        let mismatch = compare(&facts(vec![hole(4.0)]), &measured(vec![hole(4.0010006)]), GeometryTolerance::default()).expect_err("rounds up");
        assert!(matches!(mismatch, GeometryMismatch::Position { .. }));
    }

    #[test]
    fn invalid_tolerances_are_refused() {
        let recorded = facts(vec![hole(20.0)]);
        let measured_sheet = measured(vec![hole(20.0)]);
        let cases = [
            GeometryTolerance { pairing_mm: -0.1, measurement_mm: 0.001 },
            GeometryTolerance { pairing_mm: 0.0, measurement_mm: 0.001 },
            GeometryTolerance { pairing_mm: f64::NAN, measurement_mm: 0.001 },
            GeometryTolerance { pairing_mm: f64::INFINITY, measurement_mm: 0.001 },
            GeometryTolerance { pairing_mm: 0.05, measurement_mm: 0.0 },
            GeometryTolerance { pairing_mm: 0.05, measurement_mm: f64::NAN },
            GeometryTolerance { pairing_mm: 0.05, measurement_mm: f64::INFINITY },
            GeometryTolerance { pairing_mm: 0.05, measurement_mm: 0.1 },
        ];
        for tolerance in cases {
            let mismatch = compare(&recorded, &measured_sheet, tolerance).expect_err("bad tolerance");
            assert!(matches!(mismatch, GeometryMismatch::InvalidTolerance { .. }), "{tolerance:?}: {mismatch}");
        }
    }

    #[test]
    fn non_finite_facts_are_refused_before_pairing() {
        let mut recorded = facts(vec![hole(20.0)]);
        recorded.holes.as_mut().expect("holes")[0].x_mm = f64::NAN;
        let mismatch = compare(&recorded, &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("nan x");
        assert!(matches!(mismatch, GeometryMismatch::NonFinite { .. }), "{mismatch}");

        let mut recorded = facts(vec![hole(20.0)]);
        recorded.holes.as_mut().expect("holes")[0].diameter_mm = f64::NAN;
        let mismatch = compare(&recorded, &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("nan diameter");
        assert!(matches!(mismatch, GeometryMismatch::NonFinite { .. }), "{mismatch}");

        let mut recorded = facts(vec![hole(20.0)]);
        recorded.paired_mm = Some(f64::NAN);
        let mismatch = compare(&recorded, &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("nan thickness");
        assert!(matches!(mismatch, GeometryMismatch::NonFinite { .. }), "{mismatch}");

        let mut recorded = facts(vec![hole(20.0)]);
        recorded.holes.as_mut().expect("holes")[0].bend_gaps = vec![BendGap { bend: 1, gap_mm: f64::NAN, inside_radius_mm: 3.0 }];
        let mismatch = compare(&recorded, &measured(vec![hole(20.0)]), GeometryTolerance::default()).expect_err("nan gap");
        assert!(matches!(mismatch, GeometryMismatch::NonFinite { .. }), "{mismatch}");
    }

    #[test]
    fn no_complete_matching_is_not_reported_as_ambiguity() {
        // Audit case 13: R1 has two candidates near 0.01, but R2 has none within 0.05. There is
        // no complete matching, so the result is Unpaired, never Ambiguous.
        let recorded = facts(vec![hole(0.0), hole(1.0)]);
        let measured_sheet = measured(vec![hole(0.01), hole(0.011)]);
        let mismatch = compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect_err("no complete matching");
        assert!(matches!(mismatch, GeometryMismatch::Unpaired { .. } | GeometryMismatch::UnpairedMeasured { .. }), "{mismatch}");
    }

    #[test]
    fn a_competition_for_one_measured_hole_fails() {
        let recorded = facts(vec![hole(0.0), hole(0.01)]);
        let measured_sheet = measured(vec![hole(0.0), hole(5.0)]);
        let mismatch = compare(&recorded, &measured_sheet, GeometryTolerance::default()).expect_err("one measured");
        assert!(matches!(mismatch, GeometryMismatch::UnpairedMeasured { .. } | GeometryMismatch::NoPerfectPairing), "{mismatch}");
    }
}
