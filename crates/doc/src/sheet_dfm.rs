//! Sheet-metal facts for design-for-manufacturing (DFM) rules.
//!
//! [`facts`] is a pure, read-only measurement of a [`SheetBody`]: what the rule layer needs to
//! check hole diameter, hole-to-edge distance and hole-to-bend distance. It uses the structured
//! sheet model (flat panels, bends, cut-outs) and exact lines and arcs only: no kernel, no file
//! I/O, no mesh, no dialogs, and no panics. It compiles to wasm like the rest of this crate.
//!
//! What is measured, and how conservatively:
//!
//! - **Round through holes** are the regions in [`SheetBody::holes`] whose outer loop is one
//!   whole circle, plus the holes of every perforation field ([`SheetBody::perforation_holes`]).
//!   The circle test is the engine's (`crates/engine/src/cmd/sheet.rs::loop_circle`), copied
//!   here: one or more arcs with the same centre, radius and sweep sign, adding up to a full
//!   turn. Any other cut-out counts as an opening, never as a hole.
//! - **Bores** ([`SheetBody::bores`]) are holes drilled into a rim, not through holes, so they
//!   are only counted ([`SheetFacts::bores`]) and marked unusable; the rule layer can say they
//!   were not checked.
//! - **A hole has a panel** only when its whole circle lies in exactly one panel (`base` or one
//!   flange's `panel`) and does not reach a bend zone. Then `flat_assigned` is true and all
//!   per-panel facts (edge distance, bend gaps) are available. A hole that spans a bend or a
//!   panel boundary gets `flat_assigned = false`, no bend gaps and no edge distance: nothing is
//!   guessed.
//! - **Edge distance** is the least material left between the hole wall and a cut edge on the
//!   same panel: the panel outline, other openings (non-circular cut-outs, every pocket). The
//!   edge shared with an adjacent bend zone is not a cut edge and is left out. Another round
//!   hole's wall is deliberately **not** a cut edge here: INTdfm's `sm.hole_to_edge` does not
//!   count one, and its `sm.hole_spacing` rule owns hole-to-hole walls (so one wall gets one
//!   answer). The nearest other round hole on the same panel is reported separately as
//!   [`HoleFact::nearest_hole_wall_mm`], so the second gate can check an `edge_hole` ownership
//!   without changing `edge_mm`'s meaning. Distances to lines and arcs are exact; other segment
//!   kinds (free-form cut edges) follow their `polyline(1e-3)` approximation, which is the flat
//!   pattern's own chord tolerance.
//! - **Bend gaps** use each flange's two tangents, the near one at `f.p` (against the parent
//!   panel) and the far one at `f.p + f.n * allowance(f)` (against the flange's own panel). A
//!   hole on panel P gets one gap per tangent that bounds P. The gap is the perpendicular
//!   distance from the hole's centre to the tangent line less the hole radius, clamped at zero:
//!   the distance to the line, not to the bend's end, so a hole beside the end of a short bend
//!   can read closer than it is. That is conservative for a minimum-gap rule.
//! - **Bend position** is on the outer surface of the bend, halfway round it and halfway along
//!   its length (`f.len`, the bend line's full length before any trimming), folded through the
//!   parent's transform to world coordinates. The outer surface is the one farther from the bend
//!   axis (the bottom face of an up bend, the top face of a down bend).
//! - **Flatness** is `flanges.is_empty()`: the structured body has no bends, hems or folds, so
//!   the base is a single plane. This is the first-chain definition; a future version may need
//!   to look at other features.
//!
//! The `flat` field of [`SheetFacts`] means "is a flat part" (no bends); it is unrelated to
//! [`SheetBody::flat`], which means "the body is shown unfolded". Hole numbering is stable:
//! candidates are sorted by flat y, then x, then radius, and numbered from 1.

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};
use solidcore_record::{Loop2, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::LINEAR_TOL;
use solidcraft_geom::prelude::*;

use crate::sheet::SheetBody;
use crate::{DocError, apply_point, mat_mul};

/// Two circles are the same circle when their centres and radii agree to this in flat mm.
const CIRCLE_TOL: f64 = 1e-9;

/// A summary of a sheet-metal body for DFM rules. Serialized snake_case, like the sheet flat
/// pattern JSON, so the engine can put it straight into evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetFacts {
    /// Sheet thickness in mm.
    pub thickness_mm: f64,
    /// Round through holes, numbered by [`HoleFact::number`].
    pub holes: Vec<HoleFact>,
    /// How many bores ([`SheetBody::bores`]) there are. They are not through holes, so they are
    /// not in `holes`; a rule that checks holes must say they were not checked.
    pub bores: usize,
    /// One fact per bend, in flange order.
    pub bends: Vec<BendFact>,
    /// Non-circular cut-outs (plus through pockets): openings a hole rule never measured.
    pub other_openings: usize,
    /// True when the body has no flanges, so the base is one flat panel.
    pub flat: bool,
    /// The feature that changed the body after its sheet-metal features, when set. The facts are
    /// still reported, but the caller marks them unsupported (`SheetBody::flat_matches_body`).
    pub altered_by: Option<String>,
}

/// The kind of cut edge nearest a hole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// The panel's own outline (or a hole in the panel region itself).
    Outline,
    /// Another opening: a non-circular cut-out, a pocket, or another round hole's wall.
    Opening,
}

/// One round through hole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleFact {
    /// 1-based, stable: sorted by flat y, then x, then radius.
    pub number: usize,
    pub diameter_mm: f64,
    /// Folded position of the hole's axis at mid-depth, in world coordinates.
    pub x_mm: f64,
    pub y_mm: f64,
    pub z_mm: f64,
    /// Through the sheet: the thickness.
    pub depth_mm: f64,
    /// True for a through round cut, which is what a hole rule may check.
    pub square: bool,
    /// Least material between the hole wall and a cut edge on the same panel, when it can be
    /// decided; `None` if the hole has no assigned panel or the panel has no cut edge.
    pub edge_mm: Option<f64>,
    /// The nearest point of that cut edge, folded to world coordinates.
    pub edge_at_mm: Option<[f64; 3]>,
    pub edge_kind: Option<EdgeKind>,
    /// Another round hole whose wall is that edge. Always `None` for the first hole rules:
    /// `sm.hole_to_edge` does not count another hole's wall; `sm.hole_spacing` will supply that
    /// later. Kept in the evidence shape so the rule layer can fill it then.
    pub edge_hole: Option<usize>,
    /// Wall-to-wall material between this hole and the nearest other round hole on the same
    /// assigned panel, clamped at zero. Separate from [`HoleFact::edge_mm`] on purpose:
    /// `sm.hole_to_edge` owns cut edges and `sm.hole_spacing` owns hole walls.
    pub nearest_hole_wall_mm: Option<f64>,
    /// The nearest point of that other hole's wall, folded to world coordinates at mid-depth.
    pub nearest_hole_wall_at_mm: Option<[f64; 3]>,
    /// The other hole's number, in [`SheetFacts::holes`].
    pub nearest_hole_wall_number: Option<usize>,
    /// Distance to each adjacent bend's tangent line (see [`BendGapFact`]).
    pub bend_gaps: Vec<BendGapFact>,
    /// `Some(i)` for flange `i`'s panel, `None` for the base panel. Only meaningful when
    /// `flat_assigned` is true.
    pub flat: Option<usize>,
    /// False when the cut spans a bend or a panel boundary: `flat`, `bend_gaps`, `edge_mm`,
    /// `edge_at_mm`, `edge_kind` and `edge_hole` are then unavailable and must not be used.
    /// (The flat `Option` alone cannot say this, because `None` also means the base panel.)
    pub flat_assigned: bool,
}

/// How close a hole comes to one tangent of one bend.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BendGapFact {
    /// The bend's number ([`BendFact::number`]).
    pub bend: usize,
    /// Material between the hole wall and the bend tangent line, clamped at zero.
    pub gap_mm: f64,
    /// The bend's inner radius, for the rule's bend-radius allowance.
    pub inside_radius_mm: f64,
}

/// One bend, as a hole-to-bend rule needs it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BendFact {
    /// 1-based: flange `i` is bend `i + 1`.
    pub number: usize,
    pub inside_radius_mm: f64,
    /// How far the sheet turns, in degrees (always positive).
    pub angle_deg: f64,
    /// Length of the bend line.
    pub length_mm: f64,
    /// Folded position of the middle of the outer surface, halfway round the bend.
    pub x_mm: f64,
    pub y_mm: f64,
    pub z_mm: f64,
    /// The panel on each side: `[parent, flange i's panel]`.
    pub flats: [Option<usize>; 2],
}

/// Measure a sheet-metal body for the first hole rules. Pure and read-only; errors only when the
/// model's own flat computation refuses (currently only a perforation field can).
pub fn facts(sheet: &SheetBody) -> Result<SheetFacts, DocError> {
    let panels = panels(sheet);
    let zones: Vec<Region2> = sheet.flanges.iter().map(|f| sheet.zone(f)).collect();
    let mut cands: Vec<Candidate> = Vec::new();
    let mut other_openings = 0usize;

    for hole in &sheet.holes {
        match loop_circle(&hole.outer) {
            Some((centre, radius)) if centre.is_finite() && radius.is_finite() && radius > 0.0 => {
                let panel = assign_panel(&panels, &zones, centre, radius);
                cands.push(Candidate { centre, radius, panel });
            }
            // A loop that is not a whole circle is an opening, never a hole. A degenerate
            // "circle" is counted there too, so nothing silently disappears.
            _ => other_openings = other_openings.saturating_add(1),
        }
    }

    for perforation in &sheet.perforations {
        let radius = perforation.diameter / 2.0;
        if !(radius.is_finite() && radius > 0.0) {
            continue;
        }
        for centre in sheet.perforation_holes(perforation)? {
            if !centre.is_finite() {
                continue;
            }
            let panel = assign_panel(&panels, &zones, centre, radius);
            cands.push(Candidate { centre, radius, panel });
        }
    }

    // A pocket that goes through the sheet is an opening like a cut-out. Partial pockets are
    // cut edges for edge distance (below) but not "openings" in the count.
    for pocket in &sheet.pockets {
        if pocket.z0 <= LINEAR_TOL && pocket.z1 >= sheet.t - LINEAR_TOL {
            other_openings = other_openings.saturating_add(1);
        }
    }

    cands.sort_by(|a, b| {
        a.centre.y.total_cmp(&b.centre.y).then_with(|| a.centre.x.total_cmp(&b.centre.x)).then_with(|| a.radius.total_cmp(&b.radius))
    });

    let bends: Vec<BendFact> = sheet.flanges.iter().enumerate().map(|(i, f)| bend_fact(sheet, i, f)).collect();

    let mut holes = Vec::with_capacity(cands.len());
    for (index, cand) in cands.iter().enumerate() {
        let assigned = cand.panel != Panel::Unassigned;
        let panel_index = match cand.panel {
            Panel::Flange(i) => Some(i),
            Panel::Base | Panel::Unassigned => None,
        };
        // An unassigned hole's position is only its flat point in the flat frame; the caller is
        // told by `flat_assigned` that it is not usable.
        let at = world_point(sheet, panel_index, cand.centre);
        let bend_gaps = if assigned { bend_gaps(sheet, cand.panel, cand.centre, cand.radius) } else { Vec::new() };
        let edge = if assigned { nearest_edge(sheet, &panels, cand) } else { None };
        holes.push(HoleFact {
            number: index + 1,
            diameter_mm: cand.radius * 2.0,
            x_mm: at.x,
            y_mm: at.y,
            z_mm: at.z,
            depth_mm: sheet.t,
            square: true,
            edge_mm: edge.as_ref().map(|e| e.gap),
            edge_at_mm: edge.as_ref().map(|e| [e.at.x, e.at.y, e.at.z]),
            edge_kind: edge.as_ref().map(|e| e.kind),
            // `sm.hole_to_edge` does not count another hole's wall as a cut edge; the later
            // `sm.hole_spacing` rule owns hole-to-hole walls.
            edge_hole: None,
            nearest_hole_wall_mm: None,
            nearest_hole_wall_at_mm: None,
            nearest_hole_wall_number: None,
            bend_gaps,
            flat: panel_index,
            flat_assigned: assigned,
        });
    }
    nearest_hole_walls(&mut holes);

    Ok(SheetFacts {
        thickness_mm: sheet.t,
        holes,
        bores: sheet.bores.len(),
        bends,
        other_openings,
        flat: sheet.flanges.is_empty(),
        altered_by: sheet.altered_by.clone(),
    })
}

/// Fill each hole's nearest other round-hole wall. Only holes on the same assigned panel are
/// considered, so the wall is measured in that panel's plane. This never changes `edge_mm`.
fn nearest_hole_walls(holes: &mut [HoleFact]) {
    for i in 0..holes.len() {
        if !holes[i].flat_assigned {
            continue;
        }
        let mut best: Option<(f64, usize, [f64; 3])> = None;
        for j in 0..holes.len() {
            if i == j || !holes[j].flat_assigned || holes[j].flat != holes[i].flat {
                continue;
            }
            let delta = Vec3::new(holes[j].x_mm - holes[i].x_mm, holes[j].y_mm - holes[i].y_mm, holes[j].z_mm - holes[i].z_mm);
            let apart = delta.len();
            if !apart.is_finite() {
                continue;
            }
            let gap = (apart - holes[i].diameter_mm / 2.0 - holes[j].diameter_mm / 2.0).max(0.0);
            let at = if apart > LINEAR_TOL {
                let to_wall = apart - holes[j].diameter_mm / 2.0;
                [holes[i].x_mm + delta.x / apart * to_wall, holes[i].y_mm + delta.y / apart * to_wall, holes[i].z_mm + delta.z / apart * to_wall]
            } else {
                [holes[i].x_mm, holes[i].y_mm, holes[i].z_mm]
            };
            if !gap.is_finite() || !at.iter().all(|value| value.is_finite()) {
                continue;
            }
            // `<` keeps the lower-numbered hole on a tie, so the fact is deterministic.
            if best.is_none_or(|(current, _, _)| gap < current) {
                best = Some((gap, j, at));
            }
        }
        if let Some((gap, other, at)) = best {
            holes[i].nearest_hole_wall_mm = Some(gap);
            holes[i].nearest_hole_wall_at_mm = Some(at);
            holes[i].nearest_hole_wall_number = Some(holes[other].number);
        }
    }
}

/// A hole candidate before numbering and per-panel measurement.
#[derive(Clone, Copy)]
struct Candidate {
    centre: Vec2,
    radius: f64,
    panel: Panel,
}

/// The panel a hole lies on, or that it lies on none.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Unassigned,
    Base,
    Flange(usize),
}

/// The hole's axis at mid-depth, folded to world coordinates.
fn world_point(sheet: &SheetBody, panel: Option<usize>, centre: Vec2) -> Vec3 {
    apply_point(&sheet.world(panel), Vec3::new(centre.x, centre.y, sheet.t / 2.0))
}

/// The base panel and every flange panel with material, as [`SheetBody::panels`] has them.
fn panels(sheet: &SheetBody) -> Vec<(Option<usize>, Region2)> {
    let mut out = vec![(None, sheet.base.clone())];
    for (i, f) in sheet.flanges.iter().enumerate() {
        if f.leg > 1e-9 {
            out.push((Some(i), sheet.panel(f)));
        }
    }
    out
}

/// The circle a loop is, if it is one whole circle: the engine's `loop_circle` algorithm.
fn loop_circle(lp: &Loop2) -> Option<(Vec2, f64)> {
    let Some(Seg2::Arc { center, radius, sweep, .. }) = lp.segs.first() else {
        return None;
    };
    let total: f64 = lp.segs.iter().filter_map(|s| if let Seg2::Arc { sweep, .. } = s { Some(*sweep) } else { None }).sum();
    let circular = lp.segs.iter().all(
        |s| matches!(s, Seg2::Arc { center: c, radius: r, sweep: a, .. } if c.dist(*center) < CIRCLE_TOL && (*r - *radius).abs() < CIRCLE_TOL && a.signum() == sweep.signum()),
    );
    (circular && (total.abs() - TAU).abs() < CIRCLE_TOL).then_some((*center, *radius))
}

/// Which panel holds the whole circle, if exactly one does and no bend zone is reached.
fn assign_panel(panels: &[(Option<usize>, Region2)], zones: &[Region2], centre: Vec2, radius: f64) -> Panel {
    if zones.iter().any(|zone| disk_hits_region(zone, centre, radius)) {
        return Panel::Unassigned;
    }
    let mut found = Vec::new();
    for (panel, region) in panels {
        if region_has_disk(region, centre, radius) {
            found.push(*panel);
        }
    }
    let mut found = found.into_iter();
    match (found.next(), found.next()) {
        (Some(None), None) => Panel::Base,
        (Some(Some(i)), None) => Panel::Flange(i),
        _ => Panel::Unassigned,
    }
}

/// Whether the closed disk of `radius` around `centre` lies wholly in the region's material.
fn region_has_disk(region: &Region2, centre: Vec2, radius: f64) -> bool {
    if region.outer.segs.is_empty() || !region.outer.contains(centre) || !loop_clear(&region.outer, centre, radius) {
        return false;
    }
    for hole in &region.holes {
        if hole.segs.is_empty() {
            continue;
        }
        // The disk must not reach a hole, and its centre must not be over the hole at all (a disk
        // wholly inside a hole is not in the material).
        if hole.contains(centre) || !loop_clear(hole, centre, radius) {
            return false;
        }
    }
    true
}

/// Whether the closed disk around `centre` touches or enters the region.
fn disk_hits_region(region: &Region2, centre: Vec2, radius: f64) -> bool {
    if region.outer.segs.is_empty() {
        return false;
    }
    let inside = region.outer.contains(centre) && !region.holes.iter().any(|h| h.contains(centre));
    if inside {
        return true;
    }
    let boundary = std::iter::once(&region.outer).chain(&region.holes).filter_map(|lp| loop_distance(lp, centre)).reduce(f64::min);
    boundary.is_some_and(|d| d < radius - LINEAR_TOL)
}

/// The loop's boundary is at least `radius` from `centre` (within tolerance).
fn loop_clear(lp: &Loop2, centre: Vec2, radius: f64) -> bool {
    loop_distance(lp, centre).is_some_and(|d| d + LINEAR_TOL >= radius)
}

/// Exact distance from a point to a loop, or `None` if a segment has no usable form.
fn loop_distance(lp: &Loop2, p: Vec2) -> Option<f64> {
    lp.segs.iter().filter_map(|s| seg_closest(s, p)).map(|(d, _)| d).reduce(f64::min)
}

/// The nearest point on a segment, exact for lines and arcs. Other kinds are followed by their
/// `polyline(1e-3)` (the flat pattern's chord tolerance); only free-form cut edges reach that.
fn seg_closest(seg: &Seg2, p: Vec2) -> Option<(f64, Vec2)> {
    match *seg {
        Seg2::Line { a, b } => Some(line_closest(a, b, p)),
        Seg2::Arc { center, radius, start, sweep } => arc_closest(center, radius, start, sweep, p),
        _ => {
            let points = seg.polyline(1e-3);
            let mut best: Option<(f64, Vec2)> = None;
            for pair in points.windows(2) {
                let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else {
                    continue;
                };
                let hit = line_closest(a, b, p);
                if best.as_ref().is_none_or(|(d, _)| hit.0 < *d) {
                    best = Some(hit);
                }
            }
            best
        }
    }
}

fn line_closest(a: Vec2, b: Vec2, p: Vec2) -> (f64, Vec2) {
    let d = b - a;
    let len2 = d.dot(d);
    let t = if len2 > 0.0 { ((p - a).dot(d) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let q = a + d * t;
    (p.dist(q), q)
}

/// Nearest point on a circular arc: the radial point when it is within the sweep, else the
/// nearer end.
fn arc_closest(center: Vec2, radius: f64, start: f64, sweep: f64, p: Vec2) -> Option<(f64, Vec2)> {
    if !(radius.is_finite() && radius > 0.0 && start.is_finite() && sweep.is_finite() && sweep != 0.0) {
        return None;
    }
    let v = p - center;
    let d = v.len();
    let start_pt = center + Vec2::new(start.cos(), start.sin()) * radius;
    let end = start + sweep;
    let end_pt = center + Vec2::new(end.cos(), end.sin()) * radius;
    let mut best = (start_pt.dist(p), start_pt);
    let end_hit = (end_pt.dist(p), end_pt);
    if end_hit.0 < best.0 {
        best = end_hit;
    }
    let full = sweep.abs() >= TAU - CIRCLE_TOL;
    let within = if full {
        true
    } else if d <= CIRCLE_TOL {
        false
    } else {
        let angle = v.y.atan2(v.x);
        let delta = if sweep > 0.0 { (angle - start).rem_euclid(TAU) } else { (start - angle).rem_euclid(TAU) };
        delta <= sweep.abs() + CIRCLE_TOL
    };
    if within && d > CIRCLE_TOL {
        let q = center + v * (radius / d);
        let hit = ((d - radius).abs(), q);
        if hit.0 < best.0 {
            best = hit;
        }
    }
    Some(best)
}

/// One bend tangent line, and the part of it over the bend (as distance along `dir` from
/// `origin`) that is the shared edge with the panel rather than a cut edge.
struct Tangent {
    origin: Vec2,
    dir: Vec2,
    from: f64,
    to: f64,
}

/// The bend tangents that bound a panel: the near line for a flange's parent, the far line for
/// the flange's own panel.
fn bend_tangents(sheet: &SheetBody, panel: Panel) -> Vec<Tangent> {
    let key = match panel {
        Panel::Base => None,
        Panel::Flange(i) => Some(i),
        Panel::Unassigned => return Vec::new(),
    };
    let mut out = Vec::new();
    for (i, f) in sheet.flanges.iter().enumerate() {
        let span = sheet.zone_span(f);
        if f.parent == key {
            out.push(Tangent { origin: f.p, dir: f.d, from: span.0, to: span.1 });
        }
        if Some(i) == key {
            let origin = f.p + f.n * sheet.allowance(f);
            out.push(Tangent { origin, dir: f.d, from: span.0, to: span.1 });
        }
    }
    out
}

/// Push the cut parts of one outline segment, dropping the stretch that is a shared bend edge.
fn push_cut_segment(seg: Seg2, tangents: &[Tangent], out: &mut Vec<Seg2>) {
    let Seg2::Line { a, b } = seg else {
        out.push(seg);
        return;
    };
    let d = b - a;
    let length = d.len();
    if length <= LINEAR_TOL {
        return;
    }
    // Only the part of a line on the tangent line and over the bend's span is the shared edge;
    // the panel's edge beyond the end of the bend is a free cut edge.
    let mut excluded: Option<(f64, f64)> = None;
    for tangent in tangents {
        let off_a = (a - tangent.origin).cross(tangent.dir).abs();
        let off_b = (b - tangent.origin).cross(tangent.dir).abs();
        if off_a > LINEAR_TOL || off_b > LINEAR_TOL {
            continue;
        }
        let ua = (a - tangent.origin).dot(tangent.dir);
        let ub = (b - tangent.origin).dot(tangent.dir);
        let span = ub - ua;
        if span.abs() <= f64::EPSILON * (1.0 + ub.abs()) {
            continue;
        }
        let (lo, hi) = (ua.min(ub), ua.max(ub));
        let (from, to) = (tangent.from.max(lo), tangent.to.min(hi));
        if to <= from + LINEAR_TOL {
            continue;
        }
        // Segment parameters, so a shared edge that spans only part of the line is cut exactly.
        let t0 = ((from - ua) / span).clamp(0.0, 1.0);
        let t1 = ((to - ua) / span).clamp(0.0, 1.0);
        let (t0, t1) = (t0.min(t1), t0.max(t1));
        excluded = Some(excluded.map_or((t0, t1), |(x, y)| (x.min(t0), y.max(t1))));
    }
    let Some((t0, t1)) = excluded else {
        out.push(seg);
        return;
    };
    let tol = LINEAR_TOL / length;
    if t0 > tol {
        out.push(Seg2::Line { a, b: a.lerp(b, t0) });
    }
    if t1 < 1.0 - tol {
        out.push(Seg2::Line { a: a.lerp(b, t1), b });
    }
}

/// The panel's cut edges: its outline less the stretches shared with bend zones.
fn outline_segments(sheet: &SheetBody, region: &Region2, panel: Panel) -> Vec<Seg2> {
    let tangents = bend_tangents(sheet, panel);
    let mut out = Vec::new();
    for lp in std::iter::once(&region.outer).chain(&region.holes) {
        for &seg in &lp.segs {
            push_cut_segment(seg, &tangents, &mut out);
        }
    }
    out
}

/// The openings of the sheet that lie on this panel: non-circular cut-outs and every pocket
/// (a partial pocket's wall is still a cut edge in the layers it covers).
fn opening_segments(sheet: &SheetBody, region: &Region2) -> Vec<Seg2> {
    let mut out = Vec::new();
    for hole in &sheet.holes {
        if loop_circle(&hole.outer).is_some() {
            continue;
        }
        if loop_touches_region(&hole.outer, region) {
            push_region_segments(hole, &mut out);
        }
    }
    for pocket in &sheet.pockets {
        if loop_touches_region(&pocket.region.outer, region) {
            push_region_segments(&pocket.region, &mut out);
        }
    }
    out
}

fn push_region_segments(region: &Region2, out: &mut Vec<Seg2>) {
    for lp in std::iter::once(&region.outer).chain(&region.holes) {
        out.extend(lp.segs.iter().copied());
    }
}

/// Whether any of a loop's boundary lies inside the panel region. A cut-out wholly inside the
/// panel (the usual case) is found at its first vertex; one that crosses the panel boundary is
/// still found on the part that remains.
fn loop_touches_region(lp: &Loop2, region: &Region2) -> bool {
    if region.outer.segs.is_empty() {
        return false;
    }
    let inside = |p: Vec2| region.outer.contains(p) && !region.holes.iter().any(|h| h.contains(p));
    lp.polyline(1e-3).iter().any(|p| inside(*p))
}

/// The nearest cut edge of a hole's panel and its distance, in flat coordinates.
struct FlatHit {
    gap: f64,
    at: Vec2,
    kind: EdgeKind,
}

/// A decided edge in world coordinates.
struct EdgeHit {
    gap: f64,
    at: Vec3,
    kind: EdgeKind,
}

/// The nearest *cut* edge of the hole's panel.
///
/// Another round hole's wall is deliberately not a candidate: INTdfm's `sm.hole_to_edge` does
/// not count one (its `sm.hole_spacing` rule owns hole-to-hole walls, so one wall gets one
/// answer). `sm.hole_spacing` will supply those facts later; here [`HoleFact::edge_hole`] stays
/// `None`.
fn nearest_edge(sheet: &SheetBody, panels: &[(Option<usize>, Region2)], cand: &Candidate) -> Option<EdgeHit> {
    let (key, region) = panel_region(panels, cand.panel)?;
    let mut best: Option<FlatHit> = None;
    for seg in outline_segments(sheet, region, cand.panel) {
        consider_segment(&mut best, &seg, cand.centre, cand.radius, EdgeKind::Outline);
    }
    for seg in opening_segments(sheet, region) {
        consider_segment(&mut best, &seg, cand.centre, cand.radius, EdgeKind::Opening);
    }
    let hit = best?;
    let at = apply_point(&sheet.world(key), Vec3::new(hit.at.x, hit.at.y, sheet.t / 2.0));
    (hit.gap.is_finite() && at.is_finite()).then_some(EdgeHit { gap: hit.gap, at, kind: hit.kind })
}

fn panel_region(panels: &[(Option<usize>, Region2)], panel: Panel) -> Option<(Option<usize>, &Region2)> {
    let key = match panel {
        Panel::Base => None,
        Panel::Flange(i) => Some(i),
        Panel::Unassigned => return None,
    };
    panels.iter().find(|(p, _)| *p == key).map(|(p, region)| (*p, region))
}

fn consider_segment(best: &mut Option<FlatHit>, seg: &Seg2, centre: Vec2, radius: f64, kind: EdgeKind) {
    let Some((distance, at)) = seg_closest(seg, centre) else {
        return;
    };
    if !distance.is_finite() || !at.is_finite() {
        return;
    }
    consider(best, FlatHit { gap: (distance - radius).max(0.0), at, kind });
}

/// Keep the smallest gap; ties keep the first (outline, then openings).
fn consider(best: &mut Option<FlatHit>, hit: FlatHit) {
    if best.as_ref().is_none_or(|b| hit.gap < b.gap) {
        *best = Some(hit);
    }
}

/// One gap per tangent that bounds the hole's panel.
fn bend_gaps(sheet: &SheetBody, panel: Panel, centre: Vec2, radius: f64) -> Vec<BendGapFact> {
    let key = match panel {
        Panel::Base => None,
        Panel::Flange(i) => Some(i),
        Panel::Unassigned => return Vec::new(),
    };
    let mut out = Vec::new();
    for (i, f) in sheet.flanges.iter().enumerate() {
        if f.parent == key
            && let Some(gap) = tangent_gap(centre, radius, f.p, f.n)
        {
            out.push(BendGapFact { bend: i + 1, gap_mm: gap, inside_radius_mm: f.radius });
        }
        if Some(i) == key {
            let origin = f.p + f.n * sheet.allowance(f);
            if let Some(gap) = tangent_gap(centre, radius, origin, f.n) {
                out.push(BendGapFact { bend: i + 1, gap_mm: gap, inside_radius_mm: f.radius });
            }
        }
    }
    out
}

fn tangent_gap(centre: Vec2, radius: f64, origin: Vec2, normal: Vec2) -> Option<f64> {
    let offset = (centre - origin).dot(normal).abs();
    (offset.is_finite()).then(|| (offset - radius).max(0.0))
}

/// A bend's fact: its radius, angle and length, and the middle of its outer surface folded
/// through the parent's transform.
fn bend_fact(sheet: &SheetBody, i: usize, f: &crate::sheet::SheetFlange) -> BendFact {
    let up = f.angle >= 0.0;
    // The flat point on the bend's start line at half its length; the bend zone drops its width
    // and turns it by half the angle. The outer surface is the one farther from the bend axis:
    // the bottom face of an up bend, the top face of a down bend.
    let centre = f.p + f.d * (f.len / 2.0);
    let z = if up { 0.0 } else { sheet.t };
    let axis_z = if up { sheet.t + f.radius } else { -f.radius };
    let axis = Vec3::new(f.p.x, f.p.y, axis_z);
    let direction = Vec3::new(f.n.x, f.n.y, 0.0).cross(Vec3::Z);
    let rot = crate::rigid(Vec3::ZERO, axis, direction, f.angle / 2.0);
    let at = apply_point(&mat_mul(&sheet.world(f.parent), &rot), Vec3::new(centre.x, centre.y, z));
    BendFact {
        number: i + 1,
        inside_radius_mm: f.radius,
        angle_deg: f.angle.to_degrees(),
        length_mm: f.len,
        x_mm: at.x,
        y_mm: at.y,
        z_mm: at.z,
        flats: [f.parent, Some(i)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::{AngleDisplay, ReliefShape, RuleValues, SheetFlange, base_flange};
    use solidcore_record::Plane;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn rule(thickness: f64) -> RuleValues {
        RuleValues {
            thickness,
            k_factor: 0.5,
            bend_radius: 1.5,
            relief_width: 1.5,
            relief_depth: 0.75,
            corner_relief: 6.0,
            corner_relief_shape: ReliefShape::Notch,
            hem_gap: 0.01,
            gap: 1.0,
            angle_display: AngleDisplay::Bend,
        }
    }

    fn rect_region(w: f64, h: f64) -> Region2 {
        Region2 { outer: Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)]).ccw(), holes: Vec::new() }
    }

    fn plate(w: f64, h: f64, thickness: f64) -> SheetBody {
        base_flange("Body1", "Test", &rule(thickness), Plane::XY, rect_region(w, h))
    }

    fn cut_round(sheet: &mut SheetBody, x: f64, y: f64, radius: f64) {
        sheet.holes.push(Region2 { outer: Loop2::circle(Vec2::new(x, y), radius), holes: Vec::new() });
    }

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    /// 40 x 25 x 1.5 flat plate, one Ø3 hole 12.5 mm from the long edges.
    #[test]
    fn hole_facts_on_a_flat_plate() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        cut_round(&mut sheet, 20.0, 12.5, 1.5);

        let measured = facts(&sheet).unwrap();
        assert!(measured.flat);
        close(measured.thickness_mm, 1.5);
        assert!(measured.bends.is_empty());
        assert_eq!(measured.other_openings, 0);
        assert_eq!(measured.bores, 0);
        assert_eq!(measured.holes.len(), 1);

        let hole = &measured.holes[0];
        assert_eq!(hole.number, 1);
        close(hole.diameter_mm, 3.0);
        assert_eq!(hole.flat, None);
        assert!(hole.flat_assigned);
        // Plane::XY is the identity, so flat (20, 12.5, 0.75) is world (20, 12.5, 0.75).
        close(hole.x_mm, 20.0);
        close(hole.y_mm, 12.5);
        close(hole.z_mm, 0.75);
        close(hole.depth_mm, 1.5);
        assert!(hole.square);
        // y = 0 is 12.5 - 1.5 = 11 away; x = 0 and 40 are 18.5; y = 25 is 11 (a tie the bottom
        // edge wins by coming first in the outline).
        close(hole.edge_mm.unwrap(), 11.0);
        assert_eq!(hole.edge_kind, Some(EdgeKind::Outline));
        assert_eq!(hole.edge_hole, None);
        let at = hole.edge_at_mm.unwrap();
        close(at[0], 20.0);
        close(at[1], 0.0);
        close(at[2], 0.75);
        assert!(hole.bend_gaps.is_empty());
    }

    /// Two holes 3 mm and 8 mm from the end edges: the nearest outline edge wins.
    #[test]
    fn hole_edge_distances_use_cut_edges_only() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        cut_round(&mut sheet, 3.0, 12.5, 1.5);
        cut_round(&mut sheet, 32.0, 12.5, 1.5);

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.holes.len(), 2);
        let first = &measured.holes[0];
        let second = &measured.holes[1];
        assert_eq!(first.number, 1);
        assert_eq!(second.number, 2);
        close(first.x_mm, 3.0);
        close(second.x_mm, 32.0);
        // First: x = 0 is 3 - 1.5 = 1.5; the other hole is 29 - 3 = 26 away.
        close(first.edge_mm.unwrap(), 1.5);
        assert_eq!(first.edge_kind, Some(EdgeKind::Outline));
        assert_eq!(first.edge_hole, None);
        // Second: x = 40 is 40 - 32 - 1.5 = 6.5; the other hole is 26 away.
        close(second.edge_mm.unwrap(), 6.5);
        assert_eq!(second.edge_kind, Some(EdgeKind::Outline));
        assert_eq!(second.edge_hole, None);
    }

    /// INTdfm's `sm.hole_to_edge` does not count another round hole's wall as a cut edge (its
    /// `sm.hole_spacing` rule owns hole-to-hole walls). The outline must win even though the
    /// other wall is nearer.
    #[test]
    fn hole_to_edge_ignores_another_holes_wall() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        // Centres 6 mm apart, both Ø3: 6 - 1.5 - 1.5 = 3 mm between the walls. Each hole is
        // 10 - 1.5 = 8.5 mm from the y = 0 outline, which is farther than that wall but is the
        // answer `sm.hole_to_edge` must give.
        cut_round(&mut sheet, 10.0, 10.0, 1.5);
        cut_round(&mut sheet, 16.0, 10.0, 1.5);

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.holes.len(), 2);
        let left = &measured.holes[0];
        let right = &measured.holes[1];
        assert_eq!(left.number, 1);
        assert_eq!(right.number, 2);
        close(left.edge_mm.unwrap(), 8.5);
        close(right.edge_mm.unwrap(), 8.5);
        assert_eq!(left.edge_kind, Some(EdgeKind::Outline));
        assert_eq!(right.edge_kind, Some(EdgeKind::Outline));
        assert_eq!(left.edge_hole, None);
        assert_eq!(right.edge_hole, None);
        // Both nearest edges are the y = 0 outline, not the neighbouring hole's wall.
        close(left.edge_at_mm.unwrap()[1], 0.0);
        close(right.edge_at_mm.unwrap()[1], 0.0);
        close(right.edge_at_mm.unwrap()[0], 16.0);
    }

    /// Two Ø3 holes 4 mm apart: the wall between them is reported separately, while `edge_mm`
    /// keeps the outline distance that `sm.hole_to_edge` must use.
    #[test]
    fn nearest_other_hole_wall_is_separate_from_the_cut_edge() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        // Centres 4 mm apart, both Ø3, vertical 1 mm of material between the walls.
        cut_round(&mut sheet, 10.0, 10.0, 1.5);
        cut_round(&mut sheet, 14.0, 10.0, 1.5);

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.holes.len(), 2);
        let left = &measured.holes[0];
        let right = &measured.holes[1];
        // y = 0 is 10 - 1.5 = 8.5 away for both; the other wall is not the cut edge.
        close(left.edge_mm.unwrap(), 8.5);
        close(right.edge_mm.unwrap(), 8.5);
        assert_eq!(left.edge_kind, Some(EdgeKind::Outline));
        assert_eq!(right.edge_kind, Some(EdgeKind::Outline));
        assert_eq!(left.edge_hole, None);
        assert_eq!(right.edge_hole, None);

        close(left.nearest_hole_wall_mm.unwrap(), 1.0);
        close(right.nearest_hole_wall_mm.unwrap(), 1.0);
        assert_eq!(left.nearest_hole_wall_number, Some(2));
        assert_eq!(right.nearest_hole_wall_number, Some(1));
        // The nearest point is on the other hole's wall, at mid-depth.
        let at = left.nearest_hole_wall_at_mm.unwrap();
        close(at[0], 12.5);
        close(at[1], 10.0);
        close(at[2], 0.75);
    }

    /// A 90° flange on the y = 25 edge: one hole on the base, one on the flange panel. The bend
    /// lines are not cut edges, so the holes' nearest edges are the far sides of their panels.
    #[test]
    fn bend_gaps_on_a_flanged_plate() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        let radius = 1.5;
        let leg = 10.0;
        sheet.flanges.push(SheetFlange {
            parent: None,
            p: Vec2::new(0.0, 25.0),
            d: Vec2::X,
            len: 40.0,
            n: Vec2::Y,
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
        });
        // BA = (π/2)(R + K t) = (π/2)(1.5 + 0.75) = 1.125π.
        let allowance = (PI / 2.0) * (radius + 0.5 * 1.5);
        // 15 mm up the base: 8.5 mm of material to the bend, 13.5 to the far edge.
        cut_round(&mut sheet, 20.0, 15.0, 1.5);
        // 3 mm up the flange panel: 1.5 mm to the bend's far tangent, 5.5 to the panel's far edge.
        cut_round(&mut sheet, 20.0, 25.0 + allowance + 3.0, 1.5);

        let measured = facts(&sheet).unwrap();
        assert!(!measured.flat);
        assert_eq!(measured.bends.len(), 1);
        let bend = &measured.bends[0];
        assert_eq!(bend.number, 1);
        close(bend.inside_radius_mm, 1.5);
        close(bend.angle_deg, 90.0);
        close(bend.length_mm, 40.0);
        assert_eq!(bend.flats, [None, Some(0)]);
        // Outer surface (z = 0) at the bend's start line, turned 45° about the axis at z = 3:
        // y = 25 + 3/√2, z = 3 - 3/√2.
        close(bend.x_mm, 20.0);
        close(bend.y_mm, 25.0 + 3.0 / 2.0f64.sqrt());
        close(bend.z_mm, 3.0 - 3.0 / 2.0f64.sqrt());

        assert_eq!(measured.holes.len(), 2);
        let base = &measured.holes[0];
        assert_eq!(base.number, 1);
        assert_eq!(base.flat, None);
        assert!(base.flat_assigned);
        assert_eq!(base.bend_gaps.len(), 1);
        assert_eq!(base.bend_gaps[0].bend, 1);
        close(base.bend_gaps[0].gap_mm, 8.5);
        close(base.bend_gaps[0].inside_radius_mm, 1.5);
        // The bend edge at y = 25 is not a cut edge; y = 0 is.
        close(base.edge_mm.unwrap(), 13.5);
        assert_eq!(base.edge_kind, Some(EdgeKind::Outline));
        close(base.edge_at_mm.unwrap()[1], 0.0);

        let flange = &measured.holes[1];
        assert_eq!(flange.number, 2);
        assert_eq!(flange.flat, Some(0));
        assert!(flange.flat_assigned);
        // Flat (20, 25 + BA + 3, 0.75) folds to (20, 27.25, 6).
        close(flange.x_mm, 20.0);
        close(flange.y_mm, 27.25);
        close(flange.z_mm, 6.0);
        assert_eq!(flange.bend_gaps.len(), 1);
        assert_eq!(flange.bend_gaps[0].bend, 1);
        close(flange.bend_gaps[0].gap_mm, 1.5);
        // The shared root edge is not a cut edge; the far edge, 7 mm past the hole, is.
        close(flange.edge_mm.unwrap(), 5.5);
        assert_eq!(flange.edge_kind, Some(EdgeKind::Outline));
        // Flat (20, 25 + BA + 10, 0.75) folds to (20, 27.25, 13).
        close(flange.edge_at_mm.unwrap()[2], 13.0);
    }

    /// A square cut-out is an opening, not a hole.
    #[test]
    fn a_non_circular_cut_is_an_opening() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        let square = Loop2::polygon(&[Vec2::new(15.0, 10.0), Vec2::new(25.0, 10.0), Vec2::new(25.0, 15.0), Vec2::new(15.0, 15.0)]);
        sheet.holes.push(Region2 { outer: square, holes: Vec::new() });

        let measured = facts(&sheet).unwrap();
        assert!(measured.holes.is_empty());
        assert_eq!(measured.other_openings, 1);
    }

    /// A body a later feature altered still reports facts; the caller decides they are unusable.
    #[test]
    fn an_altered_body_still_reports_facts() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        cut_round(&mut sheet, 20.0, 12.5, 1.5);
        sheet.altered_by = Some("Fillet1".to_string());

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.altered_by.as_deref(), Some("Fillet1"));
        assert_eq!(measured.holes.len(), 1);
        close(measured.holes[0].diameter_mm, 3.0);
    }
}
