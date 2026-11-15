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
//!   kinds (free-form cut edges) have no exact form here. They are never approximated for an edge
//!   distance: it becomes unavailable, and the rule layer says it was not checked.
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
//! The flange, relief and web facts are measured in the same flat coordinates:
//!
//! - **Bend runs** ([`BendFact::runs_mm`]) are, for each side of a bend, 20 samples across the
//!   bend's span. From the tangent into that side's flat, perpendicular to the bend line, the
//!   run is the material length to the far edge: the first place the flat's material ends. A
//!   sample whose flat has no material at the tangent reads 0. A run cut short by a hole, an
//!   opening or a cut from a side before the flat's own far edge sets [`BendFact::crossed`], which
//!   makes `sm.min_flange` say it did not measure the flange rather than compare a cut-short run.
//!   [`BendFact::longest_mm`] is the longest run on each side.
//! - **Relief ends and notches** ([`BendFact::unrelieved`], [`BendFact::no_margin`],
//!   [`BendFact::reliefs`]) are measured at each end of the bend's span in the flat pattern. A
//!   notch is a cut that opens the bend's line at that end and reaches into the parent flat; its
//!   width is its stretch along the bend line and its depth is how far it reaches into the parent
//!   flat. With no notch, a bend end whose parent flat carries on across the bend's line is
//!   `unrelieved`; an end where another flat's outline runs along the bend's line from that end
//!   has `no_margin`. A free-form flat boundary has no exact answer, so such an end is reported as
//!   `unrelieved`: it asks a person, it never reads as relieved. A notch's width is measured a
//!   hair into the parent flat, so a round notch's chord is its widest to a few nanometres.
//! - **Webs** ([`SheetFacts::webs`]) are pairs of parallel bends that bound one common flat: the
//!   tangent-to-tangent width of that flat, whether both bends leave it on the same side, each
//!   bend's own flange run, and the middle of the web folded from the shared flat.
//!
//! The `flat` field of [`SheetFacts`] means "is a flat part" (no bends); it is unrelated to
//! [`SheetBody::flat`], which means "the body is shown unfolded". Hole numbering is stable:
//! candidates are sorted by flat y, then x, then radius, and numbered from 1.

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};
use solidcore_record::{Loop2, MAX_BEZIER_CTRL, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::Aabb2;
use solidcraft_geom::LINEAR_TOL;
use solidcraft_geom::prelude::*;

use crate::sheet::SheetBody;
use crate::{DocError, apply_point, mat_mul};
use solidcraft_kernel as kernel;

/// Two circles are the same circle when their centres and radii agree to this in flat mm.
const CIRCLE_TOL: f64 = 1e-9;

/// A probe point this far from a boundary, in flat mm: well below any feature a sheet part has,
/// and far above the noise of the coordinates.
const PROBE_MM: f64 = 1e-4;

/// Samples taken across each bend's span for [`BendFact::runs_mm`].
const RUN_SAMPLES: usize = 20;

/// A countersink record belongs to a hole when their flat centres and radii agree to this.
const COUNTERSINK_MATCH_MM: f64 = 1e-6;

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
    /// How many countersinks ([`SheetBody::countersinks`]) the model records, whether or not one
    /// sits on a round through hole. A counter that exceeds the holes carrying one
    /// ([`HoleFact::countersink`]) is a countersink this version could not read, never "none".
    pub countersinks: usize,
    /// One fact per bend, in flange order.
    pub bends: Vec<BendFact>,
    /// The flats between parallel bends, in bend-pair order.
    #[serde(default)]
    pub webs: Vec<WebFact>,
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
    /// decided; `None` if the hole has no assigned panel, the panel has no cut edge, or a
    /// free-form cut edge could be nearer than the measurable edges.
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
    /// The conical countersink this round through hole carries, when the sheet model records one
    /// on this hole's panel, centre and radius. `None` for a plain hole, a counterbore and a
    /// countersink the recogniser could not read (see [`SheetFacts::countersinks`]).
    #[serde(default)]
    pub countersink: Option<CountersinkFact>,
}

/// A countersink measured from its rim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CountersinkFact {
    /// The rim diameter at the entry skin.
    pub major_diameter_mm: f64,
    /// How far below that skin the cone narrows to the through hole's radius.
    pub depth_mm: f64,
    /// The included cone angle, in degrees.
    pub angle_deg: f64,
    /// Straight wall left between the cone and the far skin, clamped at zero.
    pub land_mm: f64,
    /// Folded position of the rim's centre on the entry skin.
    pub x_mm: f64,
    pub y_mm: f64,
    pub z_mm: f64,
    /// Least material between the rim and a cut edge on the same panel (as [`HoleFact::edge_mm`],
    /// measured from the rim).
    pub edge_mm: Option<f64>,
    pub edge_at_mm: Option<[f64; 3]>,
    pub edge_kind: Option<EdgeKind>,
    /// Distance from the rim to each adjacent bend's tangent line.
    pub bend_gaps: Vec<BendGapFact>,
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
    /// For each side in [`BendFact::flats`] order, 20 samples across the bend's span: the material
    /// length from the tangent perpendicular into that flat to the far edge, or 0 where the flat
    /// has no material at that sample.
    #[serde(default)]
    pub runs_mm: Vec<Vec<f64>>,
    /// The longest run on each side, in [`BendFact::flats`] order.
    #[serde(default)]
    pub longest_mm: Vec<f64>,
    /// Folded points of the bend ends where the parent flat carries on across the bend's line with
    /// no notch: the bend has no relief there.
    #[serde(default)]
    pub unrelieved: Vec<[f64; 3]>,
    /// Folded points of the bend ends where another flat starts right on the bend's line, with no
    /// notch past it.
    #[serde(default)]
    pub no_margin: Vec<[f64; 3]>,
    /// The notches at this bend's ends, measured in the flat pattern.
    #[serde(default)]
    pub reliefs: Vec<ReliefFact>,
    /// True when a run stopped at a hole, an opening or a cut from a side before the flat's own far
    /// edge: how far the sheet really reaches there was not measured.
    #[serde(default)]
    pub crossed: bool,
}

/// One relief notch at an end of a bend, measured in flat coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReliefFact {
    /// Folded position of the notch's middle, halfway between the bend's line and its bottom.
    pub at_mm: [f64; 3],
    /// The notch's stretch along the bend line.
    pub width_mm: f64,
    /// How far the notch reaches into the parent flat from the bend line.
    pub depth_mm: f64,
}

/// One flat between two parallel bends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebFact {
    /// The two bends (1-based, [`BendFact::number`]) that bound the flat.
    pub bends: [usize; 2],
    /// Tangent-to-tangent width of the shared flat.
    pub web_mm: f64,
    /// True when both bends leave the shared flat on the same side (a U, not an offset).
    pub same_side: bool,
    /// Each bend's own flange run, in [`WebFact::bends`] order.
    pub flanges_mm: [f64; 2],
    /// Middle of the web, folded from the shared flat.
    pub x_mm: f64,
    pub y_mm: f64,
    pub z_mm: f64,
}

/// Measure a sheet-metal body for the first hole rules. Pure and read-only; errors only when the
/// model's own flat computation refuses (currently only a perforation field can).
pub fn facts(sheet: &SheetBody) -> Result<SheetFacts, DocError> {
    let panels = panels(sheet);
    let zones: Vec<Region2> = sheet.flanges.iter().map(|f| sheet.zone(f)).collect();
    let mut cands: Vec<Candidate> = Vec::new();
    let mut other_openings = 0usize;
    let mut perforation_cuts: Vec<Region2> = Vec::new();

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
            perforation_cuts.push(Region2 { outer: Loop2::circle(centre, radius), holes: Vec::new() });
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

    let pattern = FlatPattern::new(sheet, perforation_cuts);
    let bends: Vec<BendFact> = sheet.flanges.iter().enumerate().map(|(i, f)| bend_fact(sheet, &pattern, i, f)).collect();
    let webs = pattern.webs(sheet, &bends);

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
            countersink: None,
        });
    }
    nearest_hole_walls(&mut holes);
    attach_countersinks(sheet, &panels, &cands, &mut holes);

    Ok(SheetFacts {
        thickness_mm: sheet.t,
        holes,
        bores: sheet.bores.len(),
        countersinks: sheet.countersinks.len(),
        bends,
        webs,
        other_openings,
        flat: sheet.flanges.is_empty(),
        altered_by: sheet.altered_by.clone(),
    })
}

/// Fill each round through hole's countersink from the sheet model's derived records. A record
/// that matches no round through hole is left unattached, so [`SheetFacts::countersinks`] counts
/// it and the rule layer can report it as unread rather than as no countersink at all.
fn attach_countersinks(sheet: &SheetBody, panels: &[(Option<usize>, Region2)], cands: &[Candidate], holes: &mut [HoleFact]) {
    for record in &sheet.countersinks {
        let panel = match record.panel {
            Some(i) => Panel::Flange(i),
            None => Panel::Base,
        };
        let depth = record.depth();
        let land = record.land(sheet.t);
        if !(record.centre.x.is_finite() && record.centre.y.is_finite() && record.major_radius.is_finite())
            || !(record.hole_radius.is_finite() && record.angle.is_finite() && depth.is_finite() && land.is_finite())
        {
            continue;
        }
        let mut best: Option<(usize, f64)> = None;
        for (index, cand) in cands.iter().enumerate() {
            if cand.panel != panel {
                continue;
            }
            let apart = cand.centre.dist(record.centre);
            if !apart.is_finite() || apart > COUNTERSINK_MATCH_MM || (cand.radius - record.hole_radius).abs() > COUNTERSINK_MATCH_MM {
                continue;
            }
            if best.is_none_or(|(_, nearest)| apart < nearest) {
                best = Some((index, apart));
            }
        }
        let Some((index, _)) = best else { continue };
        let Some(hole) = holes.get_mut(index) else { continue };
        if !hole.flat_assigned || !hole.square || hole.countersink.is_some() {
            continue;
        }
        let at = world_at(sheet, record.panel, record.centre, record.entry_z);
        let rim = Candidate { centre: record.centre, radius: record.major_radius, panel };
        let edge = nearest_edge(sheet, panels, &rim);
        hole.countersink = Some(CountersinkFact {
            major_diameter_mm: record.major_radius * 2.0,
            depth_mm: depth,
            angle_deg: record.angle.to_degrees(),
            land_mm: land,
            x_mm: at.x,
            y_mm: at.y,
            z_mm: at.z,
            edge_mm: edge.as_ref().map(|e| e.gap),
            edge_at_mm: edge.as_ref().map(|e| [e.at.x, e.at.y, e.at.z]),
            edge_kind: edge.as_ref().map(|e| e.kind),
            bend_gaps: bend_gaps(sheet, panel, record.centre, record.major_radius),
        });
    }
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
    world_at(sheet, panel, centre, sheet.t / 2.0)
}

/// A flat point on one panel at flat height `z`, folded to world coordinates.
fn world_at(sheet: &SheetBody, panel: Option<usize>, centre: Vec2, z: f64) -> Vec3 {
    apply_point(&sheet.world(panel), Vec3::new(centre.x, centre.y, z))
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

/// Exact distance from a point to a loop, or `None` if any segment has no exact form. A free-form
/// segment is never approximated for a model decision; the caller then treats the edge as unknown.
fn loop_distance(lp: &Loop2, p: Vec2) -> Option<f64> {
    let mut best: Option<f64> = None;
    for segment in &lp.segs {
        let (distance, _) = seg_closest(segment, p)?;
        best = Some(best.map_or(distance, |value: f64| value.min(distance)));
    }
    best
}

/// The nearest point on a segment, exact for lines and arcs. Free-form kinds have no exact form
/// and return `None`; the caller must not build a decision on a chord approximation.
fn seg_closest(seg: &Seg2, p: Vec2) -> Option<(f64, Vec2)> {
    match *seg {
        Seg2::Line { a, b } => Some(line_closest(a, b, p)),
        Seg2::Arc { center, radius, start, sweep } => arc_closest(center, radius, start, sweep, p),
        // Free-form edges are not approximated for a model decision.
        _ => None,
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

/// A lower bound on a segment's distance from a point, used only when [`seg_closest`] has no
/// exact answer. Free-form curves lie inside their control points' bounding box, so the box's
/// distance is a sound bound: when it is farther than the best measured edge, the unknown edge
/// cannot be nearer. `None` means even that bound is unavailable and the edge must stay unknown.
fn seg_distance_lower_bound(seg: &Seg2, p: Vec2) -> Option<f64> {
    let control: Vec<Vec2> = match *seg {
        Seg2::Cubic { p0, p1, p2, p3 } => vec![p0, p1, p2, p3],
        // A rational quadratic with a positive weight stays inside its three control points.
        Seg2::Conic { a, apex, b, w } if w.is_finite() && w > 0.0 => vec![a, apex, b],
        Seg2::Conic { .. } => return None,
        Seg2::Bezier { n, p: points } => {
            let degree = usize::from(n).min(MAX_BEZIER_CTRL - 1);
            points[..=degree].to_vec()
        }
        // Lines and arcs are exact, so this is only a defensive fallback. An invalid one has no
        // usable bound either.
        _ => return seg_closest(seg, p).map(|(distance, _)| distance).filter(|distance| distance.is_finite()),
    };
    if control.iter().any(|q| !q.x.is_finite() || !q.y.is_finite()) {
        return None;
    }
    let mut min = control.first().copied()?;
    let mut max = min;
    for q in control {
        min.x = min.x.min(q.x);
        min.y = min.y.min(q.y);
        max.x = max.x.max(q.x);
        max.y = max.y.max(q.y);
    }
    let dx = if p.x < min.x {
        min.x - p.x
    } else if p.x > max.x {
        p.x - max.x
    } else {
        0.0
    };
    let dy = if p.y < min.y {
        min.y - p.y
    } else if p.y > max.y {
        p.y - max.y
    } else {
        0.0
    };
    Some((dx * dx + dy * dy).sqrt())
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
    // the panel's edge beyond the end of the bend is a free cut edge. Keep every span separately:
    // two bends may share one line and leave a real cut edge between them.
    let mut excluded: Vec<(f64, f64)> = Vec::new();
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
        excluded.push((t0.min(t1), t0.max(t1)));
    }
    if excluded.is_empty() {
        out.push(seg);
        return;
    }
    let tol = LINEAR_TOL / length;
    excluded.sort_by(|left, right| left.0.total_cmp(&right.0).then_with(|| left.1.total_cmp(&right.1)));
    // Merge only spans that overlap or touch, so the cut edge between separate bends survives.
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(excluded.len());
    for (t0, t1) in excluded {
        if let Some(last) = merged.last_mut()
            && t0 <= last.1 + tol
        {
            last.1 = last.1.max(t1);
            continue;
        }
        merged.push((t0, t1));
    }
    let mut start = 0.0;
    for (t0, t1) in merged {
        if t0 > start + tol {
            out.push(Seg2::Line { a: a.lerp(b, start), b: a.lerp(b, t0) });
        }
        start = start.max(t1);
    }
    if start < 1.0 - tol {
        out.push(Seg2::Line { a: a.lerp(b, start), b });
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
    let mut unknown_lower: Option<f64> = None;
    for seg in outline_segments(sheet, region, cand.panel) {
        if !consider_segment(&mut best, &seg, cand.centre, cand.radius, EdgeKind::Outline) {
            note_unmeasured(&seg, cand.centre, &mut unknown_lower);
        }
    }
    for seg in opening_segments(sheet, region) {
        if !consider_segment(&mut best, &seg, cand.centre, cand.radius, EdgeKind::Opening) {
            note_unmeasured(&seg, cand.centre, &mut unknown_lower);
        }
    }
    let hit = best?;
    // A free-form edge that might lie inside the best edge's band makes the answer unknown: the
    // outline must never stand in for an edge that could be nearer. A zero gap is already the
    // smallest possible clearance, so no unknown edge can beat it.
    if hit.gap > LINEAR_TOL
        && let Some(lower) = unknown_lower
        && (lower - cand.radius).max(0.0) <= hit.gap + LINEAR_TOL
    {
        return None;
    }
    let at = apply_point(&sheet.world(key), Vec3::new(hit.at.x, hit.at.y, sheet.t / 2.0));
    (hit.gap.is_finite() && at.is_finite()).then_some(EdgeHit { gap: hit.gap, at, kind: hit.kind })
}

/// Keep the smallest lower bound of every segment with no exact distance. `None` from the bound
/// itself (unknown controls) is the weakest possible bound, so it forces the edge unknown.
fn note_unmeasured(seg: &Seg2, centre: Vec2, unknown_lower: &mut Option<f64>) {
    let bound = seg_distance_lower_bound(seg, centre).unwrap_or(0.0);
    *unknown_lower = Some(unknown_lower.map_or(bound, |known: f64| known.min(bound)));
}

fn panel_region(panels: &[(Option<usize>, Region2)], panel: Panel) -> Option<(Option<usize>, &Region2)> {
    let key = match panel {
        Panel::Base => None,
        Panel::Flange(i) => Some(i),
        Panel::Unassigned => return None,
    };
    panels.iter().find(|(p, _)| *p == key).map(|(p, region)| (*p, region))
}

/// Measure one segment. Returns false when it has no exact answer, so the caller can keep the
/// segment's lower bound and decide whether it could still be the nearest edge.
fn consider_segment(best: &mut Option<FlatHit>, seg: &Seg2, centre: Vec2, radius: f64, kind: EdgeKind) -> bool {
    let Some((distance, at)) = seg_closest(seg, centre) else {
        return false;
    };
    if !distance.is_finite() || !at.is_finite() {
        return false;
    }
    consider(best, FlatHit { gap: (distance - radius).max(0.0), at, kind });
    true
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

// ---------------------------------------------------------------------------
// The flat pattern: what the flange, relief and web rules measure.

/// One flat's material, and the cut-outs that remove material from it.
struct Flat {
    /// `None` for the base, `Some(i)` for flange `i`'s panel.
    panel: Option<usize>,
    region: Region2,
    /// Indices into [`FlatPattern::cuts`] of the cut-outs that meet this flat.
    cuts: Vec<usize>,
    /// True when every boundary here is a line or an arc, so a ray has an exact answer.
    exact: bool,
}

impl Flat {
    /// Whether `p` is this flat's material: inside its outline, outside its holes and cut-outs.
    /// `None` when a free-form boundary leaves the answer undecided.
    fn material(&self, pattern: &FlatPattern, p: Vec2) -> Option<bool> {
        if !region_contains(&self.region, p)? {
            return Some(false);
        }
        for &index in &self.cuts {
            match pattern.cuts.get(index).map(|cut| region_contains(cut, p)) {
                Some(Some(true)) => return Some(false),
                Some(Some(false)) => {}
                _ => return None,
            }
        }
        Some(true)
    }
}

/// The structured sheet as flat regions: each flat's material, each bend zone and every through
/// cut-out. Rays through it answer the flange run, relief and web questions exactly.
struct FlatPattern {
    flats: Vec<Flat>,
    zones: Vec<Region2>,
    cuts: Vec<Region2>,
    /// True when every boundary is a line or an arc: no free-form edge anywhere.
    exact: bool,
}

impl FlatPattern {
    fn new(sheet: &SheetBody, perforation_cuts: Vec<Region2>) -> Self {
        // A flat part has no flange, relief or web fact to measure, and need not clone its
        // cut-outs for rays it will never cast.
        if sheet.flanges.is_empty() {
            return FlatPattern { flats: Vec::new(), zones: Vec::new(), cuts: Vec::new(), exact: true };
        }
        // Cut-outs that remove material: every through hole, every through pocket and the
        // perforation field's holes.
        let mut cuts = sheet.holes.clone();
        for pocket in &sheet.pockets {
            if pocket.z0 <= LINEAR_TOL && pocket.z1 >= sheet.t - LINEAR_TOL {
                cuts.push(pocket.region.clone());
            }
        }
        cuts.extend(perforation_cuts);
        let boxes: Vec<Aabb2> = cuts.iter().map(|cut| loop_box(&cut.outer)).collect();
        let mut flats = Vec::new();
        for (panel, region) in panels(sheet) {
            let reach = loop_box(&region.outer).inflated(PROBE_MM);
            let near: Vec<usize> = boxes.iter().enumerate().filter(|(_, b)| b.meets(&reach)).map(|(index, _)| index).collect();
            let exact = region_exact(&region) && near.iter().all(|index| cuts.get(*index).is_none_or(region_exact));
            flats.push(Flat { panel, region, cuts: near, exact });
        }
        let zones: Vec<Region2> = sheet.flanges.iter().map(|f| sheet.zone(f)).collect();
        let exact = flats.iter().all(|flat| flat.exact) && zones.iter().all(region_exact) && cuts.iter().all(region_exact);
        FlatPattern { flats, zones, cuts, exact }
    }

    fn flat(&self, panel: Option<usize>) -> Option<&Flat> {
        self.flats.iter().find(|flat| flat.panel == panel)
    }

    /// Every boundary of the material: flat outlines and holes, bend zones, and cut-outs.
    fn loops(&self) -> impl Iterator<Item = &Loop2> + Clone {
        self.flats
            .iter()
            .flat_map(|flat| flat.region.loops())
            .chain(self.zones.iter().flat_map(Region2::loops))
            .chain(self.cuts.iter().flat_map(Region2::loops))
    }

    /// Whether `p` is material of any flat or bend zone and not inside a cut-out.
    fn material_at(&self, p: Vec2) -> Option<bool> {
        for cut in &self.cuts {
            if region_contains(cut, p)? {
                return Some(false);
            }
        }
        for flat in &self.flats {
            if region_contains(&flat.region, p)? {
                return Some(true);
            }
        }
        for zone in &self.zones {
            if region_contains(zone, p)? {
                return Some(true);
            }
        }
        Some(false)
    }

    /// The flats between parallel bends: for every pair of bends that bound one common flat, the
    /// tangent-to-tangent width of that flat, whether both leave it on the same side, and each
    /// bend's own flange run.
    fn webs(&self, sheet: &SheetBody, bends: &[BendFact]) -> Vec<WebFact> {
        let mut out = Vec::new();
        for i in 0..sheet.flanges.len() {
            for j in (i + 1)..sheet.flanges.len() {
                let (Some(first), Some(second), Some(bend_first), Some(bend_second)) =
                    (sheet.flanges.get(i), sheet.flanges.get(j), bends.get(i), bends.get(j))
                else {
                    continue;
                };
                let Some(d1) = first.d.normalized() else { continue };
                let Some(d2) = second.d.normalized() else { continue };
                // Only parallel (or antiparallel) bends bound a web; a corner is not one.
                if d1.cross(d2).abs() > LINEAR_TOL {
                    continue;
                }
                let flats_first = [first.parent, Some(i)];
                let flats_second = [second.parent, Some(j)];
                let Some(shared) = flats_first.into_iter().find(|flat| flats_second.contains(flat)) else {
                    continue;
                };
                // The shared flat has to be one this pattern has material for.
                if self.flat(shared).is_none() {
                    continue;
                }
                let (Some(n1), Some(n2)) = (first.n.normalized(), second.n.normalized()) else { continue };
                // Each bend's near tangent into the shared flat: the bend line itself on the
                // parent's side, or the far tangent when the shared flat is the bend's own panel.
                let own_first = shared == Some(i);
                let own_second = shared == Some(j);
                let start_first = if own_first { sheet.allowance(first) } else { 0.0 };
                let start_second = (second.p - first.p).dot(n1) + if own_second { sheet.allowance(second) * n2.dot(n1) } else { 0.0 };
                let web = (start_second - start_first).abs();
                if !(web.is_finite() && web > LINEAR_TOL) {
                    continue;
                }
                let side = |angle: f64, own: bool| {
                    let turn = if angle >= 0.0 { 1.0 } else { -1.0 };
                    if own { -turn } else { turn }
                };
                let span_first = sheet.zone_span(first);
                let span_second = sheet.zone_span(second);
                let tangent_first = first.p + n1 * start_first + d1 * ((span_first.0 + span_first.1) / 2.0);
                let tangent_second =
                    second.p + n2 * if own_second { sheet.allowance(second) } else { 0.0 } + d2 * ((span_second.0 + span_second.1) / 2.0);
                let middle = (tangent_first + tangent_second) * 0.5;
                let at = apply_point(&sheet.world(shared), Vec3::new(middle.x, middle.y, 0.0));
                if !at.is_finite() {
                    continue;
                }
                out.push(WebFact {
                    bends: [i + 1, j + 1],
                    web_mm: web,
                    same_side: side(first.angle, own_first) == side(second.angle, own_second),
                    flanges_mm: [bend_first.longest_mm.get(1).copied().unwrap_or(0.0), bend_second.longest_mm.get(1).copied().unwrap_or(0.0)],
                    x_mm: at.x,
                    y_mm: at.y,
                    z_mm: at.z,
                });
            }
        }
        out
    }
}

/// One sampled run across a flat.
struct Run {
    /// Material length from the bend tangent to the far edge.
    length: f64,
    /// True when a cut stopped the run before the flat's own far edge, or the flat had no
    /// material at the sample: how far the sheet reaches there was not measured.
    crossed: bool,
}

/// A notch at one end of a bend, in flat coordinates along the bend line.
struct Notch {
    /// Where the notch opens and closes, measured from `f.p` along `f.d`.
    from: f64,
    to: f64,
    /// How far the notch reaches into the parent flat from the bend line.
    depth: f64,
}

/// The 20 runs of one side of a bend, and whether any of them was cut short.
fn run_samples(flat: &Flat, pattern: &FlatPattern, origin: Vec2, along: Vec2, into: Vec2, span: (f64, f64)) -> (Vec<f64>, bool) {
    let length = span.1 - span.0;
    if !(length.is_finite() && length > LINEAR_TOL) {
        return (vec![0.0; RUN_SAMPLES], true);
    }
    let mut runs = Vec::with_capacity(RUN_SAMPLES);
    let mut crossed = false;
    for sample in 0..RUN_SAMPLES {
        let u = span.0 + (sample as f64 + 0.5) * length / RUN_SAMPLES as f64;
        let run = material_run(flat, pattern, origin + along * u, into);
        crossed |= run.crossed;
        runs.push(run.length);
    }
    (runs, crossed)
}

/// One run: from `origin` along `dir`, the material length to the far edge of the flat.
fn material_run(flat: &Flat, pattern: &FlatPattern, origin: Vec2, dir: Vec2) -> Run {
    let unknown = Run { length: 0.0, crossed: true };
    if !flat.exact {
        return unknown;
    }
    // The bend's tangent is on the flat's edge; a hair inside it must be material. Where it is
    // not, the flat cannot reach this sample (a cut or a side took the material away).
    if flat.material(pattern, origin + dir * PROBE_MM) != Some(true) {
        return unknown;
    }
    let Some(far) = first_hit(&[&flat.region.outer], origin, dir) else {
        return unknown;
    };
    let mut loops = vec![&flat.region.outer];
    loops.extend(flat.region.holes.iter());
    for &index in &flat.cuts {
        if let Some(cut) = pattern.cuts.get(index) {
            loops.extend(cut.loops());
        }
    }
    let mut hits = Vec::new();
    for lp in loops {
        let Some(mut found) = ray_loop_hits(lp, origin, dir) else {
            return unknown;
        };
        hits.append(&mut found);
    }
    hits.retain(|t| t.is_finite() && *t > PROBE_MM * 0.5);
    hits.sort_by(f64::total_cmp);
    // Walk the intervals between boundaries from the tangent: the run ends where material does.
    let mut stop = 0.0;
    for hit in hits {
        if hit - stop <= LINEAR_TOL {
            stop = hit;
            continue;
        }
        let middle = origin + dir * ((stop + hit) * 0.5);
        match flat.material(pattern, middle) {
            Some(true) => stop = hit,
            Some(false) => return Run { length: stop, crossed: stop < far - LINEAR_TOL },
            None => return unknown,
        }
    }
    if stop <= LINEAR_TOL {
        return unknown;
    }
    Run { length: stop, crossed: stop < far - LINEAR_TOL }
}

/// The notch that opens the bend's line at the end at `u_end`, if one is there: it is the void
/// around that end along the bend's line, bounded by material on both sides, reaching into the
/// parent flat.
fn notch_at(pattern: &FlatPattern, f: &crate::sheet::SheetFlange, u_end: f64, outward: f64, d: Vec2, n: Vec2) -> Option<Notch> {
    if !pattern.exact {
        return None;
    }
    let end = f.p + d * u_end;
    // Across the parent flat, a hair before the bend's line: there a notch opens the line and is
    // bounded by parent material on both sides.
    let line = end - n * PROBE_MM;
    let mut hits = Vec::new();
    for lp in pattern.loops() {
        let mut found = ray_loop_hits(lp, line, d)?;
        hits.append(&mut found);
    }
    hits.retain(|t| t.is_finite());
    hits.sort_by(f64::total_cmp);
    let (from, to) = interval_around(&hits, 0.0)?;
    if to - from <= LINEAR_TOL {
        return None;
    }
    // The notch has to open the end outward, not only reach back into the bend's span.
    if (outward > 0.0 && to <= LINEAR_TOL) || (outward < 0.0 && from >= -LINEAR_TOL) {
        return None;
    }
    let middle = (from + to) * 0.5;
    if pattern.material_at(line + d * middle) != Some(false) {
        return None;
    }
    // How far it reaches into the parent flat: from the bend's line at the notch's middle.
    let origin = end + d * middle;
    let depth = first_hit(&pattern.loops().collect::<Vec<_>>(), origin, n * -1.0)?;
    if depth <= LINEAR_TOL {
        return None;
    }
    Some(Notch { from: u_end + from, to: u_end + to, depth })
}

/// The interval between consecutive boundary hits that holds `at`, or `None` when it is unbounded.
fn interval_around(hits: &[f64], at: f64) -> Option<(f64, f64)> {
    let mut below = None;
    for hit in hits {
        if *hit > at {
            return below.map(|low| (low, *hit));
        }
        below = Some(*hit);
    }
    None
}

/// The nearest boundary crossing of a ray, exact for lines and arcs.
fn first_hit(loops: &[&Loop2], origin: Vec2, dir: Vec2) -> Option<f64> {
    let mut best: Option<f64> = None;
    for lp in loops {
        for t in ray_loop_hits(lp, origin, dir)? {
            if t > PROBE_MM * 0.5 && t.is_finite() && best.is_none_or(|nearest| t < nearest) {
                best = Some(t);
            }
        }
    }
    best
}

/// Where a ray crosses a loop, as distances along `dir`. `None` when a free-form segment leaves no
/// exact answer; free-form edges are never approximated for a decision.
fn ray_loop_hits(lp: &Loop2, origin: Vec2, dir: Vec2) -> Option<Vec<f64>> {
    let mut hits = Vec::new();
    for seg in &lp.segs {
        match *seg {
            Seg2::Line { a, b } => {
                let edge = b - a;
                let length = edge.len();
                if length <= LINEAR_TOL {
                    continue;
                }
                let denominator = dir.cross(edge);
                // A line along the ray has no single crossing point.
                if denominator.abs() <= LINEAR_TOL * length {
                    continue;
                }
                let t = (a - origin).cross(edge) / denominator;
                let s = (a - origin).cross(dir) / denominator;
                if t.is_finite() && s.is_finite() && s >= -LINEAR_TOL / length && s <= 1.0 + LINEAR_TOL / length {
                    hits.push(t);
                }
            }
            Seg2::Arc { center, radius, start, sweep } => {
                if !(radius.is_finite() && radius > 0.0) {
                    continue;
                }
                let from_center = origin - center;
                let along = from_center.dot(dir);
                let offset = from_center.dot(from_center) - radius * radius;
                let discriminant = along * along - offset;
                // A tangent only touches the arc: it does not cross the material's boundary.
                if discriminant <= 0.0 {
                    continue;
                }
                let root = discriminant.sqrt();
                for t in [-along - root, -along + root] {
                    let point = origin + dir * t;
                    if arc_has(center, start, sweep, point) {
                        hits.push(t);
                    }
                }
            }
            _ => return None,
        }
    }
    Some(hits)
}

/// Whether a point on an arc's circle lies within the arc's sweep.
fn arc_has(center: Vec2, start: f64, sweep: f64, point: Vec2) -> bool {
    if sweep.abs() >= TAU - CIRCLE_TOL {
        return true;
    }
    let offset = point - center;
    let distance = offset.len();
    if !(distance.is_finite() && distance > CIRCLE_TOL) {
        return false;
    }
    let angle = offset.y.atan2(offset.x);
    let delta = if sweep > 0.0 { (angle - start).rem_euclid(TAU) } else { (start - angle).rem_euclid(TAU) };
    delta <= sweep.abs() + CIRCLE_TOL
}

/// Whether `p` lies inside a loop, exactly for lines and arcs. `None` when a free-form segment has
/// no exact answer.
fn loop_contains(lp: &Loop2, p: Vec2) -> Option<bool> {
    let mut inside = false;
    for seg in &lp.segs {
        match *seg {
            Seg2::Line { a, b } => {
                if (a.y > p.y) != (b.y > p.y) {
                    let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                    if x > p.x {
                        inside = !inside;
                    }
                }
            }
            Seg2::Arc { center, radius, start, sweep } => {
                if !(radius.is_finite() && radius > 0.0) {
                    continue;
                }
                let down = p.y - center.y;
                let discriminant = radius * radius - down * down;
                if discriminant <= 0.0 {
                    continue;
                }
                let root = discriminant.sqrt();
                for x in [center.x - root, center.x + root] {
                    if x > p.x && arc_has(center, start, sweep, Vec2::new(x, p.y)) {
                        inside = !inside;
                    }
                }
            }
            _ => return None,
        }
    }
    Some(inside)
}

/// Whether `p` lies in a region's material: inside its outline and outside its holes.
fn region_contains(region: &Region2, p: Vec2) -> Option<bool> {
    if !loop_contains(&region.outer, p)? {
        return Some(false);
    }
    for hole in &region.holes {
        if loop_contains(hole, p)? {
            return Some(false);
        }
    }
    Some(true)
}

fn region_exact(region: &Region2) -> bool {
    region.loops().all(Loop2::is_line_arc)
}

/// A loop's bounding box, a little larger so an arc's bulge stays inside.
fn loop_box(lp: &Loop2) -> Aabb2 {
    let points = lp.polyline(1e-3);
    let bounds = points.bounds();
    bounds.inflated(1e-3 + 1e-9 * bounds.diagonal())
}

/// A bend's fact: its radius, angle and length, the middle of its outer surface folded through the
/// parent's transform, its flange runs, and the relief at each of its ends.
fn bend_fact(sheet: &SheetBody, pattern: &FlatPattern, i: usize, f: &crate::sheet::SheetFlange) -> BendFact {
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

    let d = f.d.normalized().unwrap_or(Vec2::X);
    let n = f.n.normalized().unwrap_or(Vec2::Y);
    let span = sheet.zone_span(f);
    let mut runs_mm = Vec::with_capacity(2);
    let mut crossed = false;
    for (panel, origin, into) in [(f.parent, f.p, n * -1.0), (Some(i), f.p + n * sheet.allowance(f), n)] {
        let (runs, cut) = match pattern.flat(panel) {
            Some(flat) => run_samples(flat, pattern, origin, d, into, span),
            None => (vec![0.0; RUN_SAMPLES], true),
        };
        runs_mm.push(runs);
        crossed |= cut;
    }
    let longest_mm = runs_mm.iter().map(|runs| runs.iter().copied().fold(0.0, f64::max)).collect();

    let mut unrelieved = Vec::new();
    let mut no_margin = Vec::new();
    let mut reliefs = Vec::new();
    for (u, outward) in [(span.0, -1.0), (span.1, 1.0)] {
        let end = f.p + d * u;
        if let Some(notch) = notch_at(pattern, f, u, outward, d, n) {
            let middle = (notch.from + notch.to) * 0.5;
            let local = f.p + d * middle - n * (notch.depth * 0.5);
            let at = apply_point(&sheet.world(f.parent), Vec3::new(local.x, local.y, 0.0));
            if at.is_finite() {
                reliefs.push(ReliefFact { at_mm: [at.x, at.y, at.z], width_mm: notch.to - notch.from, depth_mm: notch.depth });
            }
            continue;
        }
        // A free-form flat boundary leaves no exact answer, so the end asks a person rather than
        // reading as relieved.
        if !pattern.exact {
            unrelieved.push(folded_end(sheet, f, end));
            continue;
        }
        // The parent flat carrying on across the bend's line past this end is a bend with no
        // notch to free it.
        let across = end + d * outward * PROBE_MM + n * PROBE_MM;
        if pattern.flat(f.parent).is_some_and(|flat| flat.material(pattern, across) == Some(true)) {
            unrelieved.push(folded_end(sheet, f, end));
            continue;
        }
        if starts_on_line(pattern, f, i, end, d * outward) {
            no_margin.push(folded_end(sheet, f, end));
        }
    }

    BendFact {
        number: i + 1,
        inside_radius_mm: f.radius,
        angle_deg: f.angle.to_degrees(),
        length_mm: f.len,
        x_mm: at.x,
        y_mm: at.y,
        z_mm: at.z,
        flats: [f.parent, Some(i)],
        runs_mm,
        longest_mm,
        unrelieved,
        no_margin,
        reliefs,
        crossed,
    }
}

/// Two flat material pieces share more area than this and the sheet cannot be folded as drawn.
///
/// The number comes from the planar boolean, not from the hole-pair measurement tolerance: every
/// `material_boolean` call closes its loops within `1e-7` mm (`crates/doc/src/sheet.rs` passes
/// `1e-7` to the kernel). A false sliver along a shared seam of length `L` therefore has an area
/// of at most `L * 1e-7` mm²; on a 1 m seam that is `1e-4` mm². `0.01` mm² is 100 times that
/// bound and still 1/78 of a Ø1 mm hole, so any overlap a person can make is far above it.
pub const OVERLAP_AREA_TOLERANCE_MM2: f64 = 0.01;

/// The most structural flat pieces one check will intersect. More than this would make the
/// pairwise work unbounded, so the check reports that it is not covered instead.
const MAX_OVERLAP_PIECES: usize = 128;

/// A structural piece of the flat pattern: the base panel, a flange's flat panel, or a bend
/// zone. Panels are numbered by flange index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlatPiece {
    Base,
    Panel(usize),
    Zone(usize),
}

impl FlatPiece {
    /// A stable plain name for evidence and reports.
    pub fn label(self) -> String {
        match self {
            FlatPiece::Base => "base".to_string(),
            FlatPiece::Panel(i) => format!("panel {i}"),
            FlatPiece::Zone(i) => format!("bend {i}"),
        }
    }
}

/// Why the flat self-overlap could not be decided. It is a typed reason, never a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapNotCovered {
    /// The sheet has no trustworthy flat pattern: a later feature changed the body, or the
    /// bounded flat work refused it.
    CannotUnfold,
    /// The flat pattern exists, but a structural piece or a pair could not be intersected.
    CannotIntersect,
}

impl OverlapNotCovered {
    /// A stable reason code for a report.
    pub fn code(self) -> &'static str {
        match self {
            OverlapNotCovered::CannotUnfold => "CANNOT_UNFOLD",
            OverlapNotCovered::CannotIntersect => "CANNOT_INTERSECT",
        }
    }
}

/// One pair of flat pieces that occupies the same flat area.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OverlapPair {
    /// The two structural pieces, in flat-pattern order.
    pub pieces: [FlatPiece; 2],
    /// The exact intersection area in mm².
    pub area_mm2: f64,
    /// The intersection's outer loops in flat coordinates (`[x, y]`), the largest first.
    ///
    /// This is drawing evidence only: the loops are the exact boolean result sampled at 0.001 mm
    /// for display. The area above is computed from the unsampled regions.
    pub outline: Vec<Vec<[f64; 2]>>,
}

/// The result of the flat self-overlap check for one sheet body.
///
/// Only one body is ever measured, so two bodies are never projected onto each other.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Overlap {
    /// No pair of pieces shares more area than [`OVERLAP_AREA_TOLERANCE_MM2`]. Shared edges and
    /// single points are not overlap.
    Clear,
    /// At least one pair shares more area than the tolerance.
    Overlapping { pairs: Vec<OverlapPair> },
    /// The check could not be computed; nothing is claimed either way.
    NotCovered { reason: OverlapNotCovered },
}

/// Measure whether one flat sheet's structural pieces overlap each other after unfolding.
///
/// The pieces are the base panel, every flange panel with material, and every bend zone. Each
/// piece's material is the exact flat material after all cut-outs, holes and openings, clipped to
/// that piece, so a hole through the shared area is not material and is not an overlap. Every
/// pair is intersected exactly with the planar booleans the flat pattern already uses.
///
/// A pair fails when its intersection area is greater than [`OVERLAP_AREA_TOLERANCE_MM2`]. Shared
/// edges, single points and smaller areas are not an overlap. A sheet that cannot be unfolded, or
/// whose pieces cannot be intersected within the bounded flat work, returns
/// [`Overlap::NotCovered`] with a typed reason instead of a pass.
pub fn flat_overlap(sheet: &SheetBody) -> Result<Overlap, DocError> {
    if sheet.altered_by.is_some() {
        return Ok(Overlap::NotCovered { reason: OverlapNotCovered::CannotUnfold });
    }
    let material = match sheet.flat_material() {
        Ok(material) => material,
        Err(_) => return Ok(Overlap::NotCovered { reason: OverlapNotCovered::CannotUnfold }),
    };
    if material.is_empty() {
        return Ok(Overlap::NotCovered { reason: OverlapNotCovered::CannotUnfold });
    }

    let mut pieces: Vec<(FlatPiece, Region2)> = vec![(FlatPiece::Base, sheet.base.clone())];
    for (i, f) in sheet.flanges.iter().enumerate() {
        if f.leg > 1e-9 {
            pieces.push((FlatPiece::Panel(i), sheet.panel(f)));
        }
    }
    for (i, f) in sheet.flanges.iter().enumerate() {
        pieces.push((FlatPiece::Zone(i), sheet.zone(f)));
    }
    if pieces.len() > MAX_OVERLAP_PIECES {
        return Ok(Overlap::NotCovered { reason: OverlapNotCovered::CannotIntersect });
    }

    let mut work = crate::sheet::FlatWork::new();
    let mut owned: Vec<Option<Vec<Region2>>> = Vec::with_capacity(pieces.len());
    for (_, region) in &pieces {
        // A degenerate structural piece has no material to share.
        if !(region.area().is_finite() && region.area() > 0.0) {
            owned.push(None);
            continue;
        }
        let clipped = match crate::sheet::material_boolean(&material, std::slice::from_ref(region), kernel::BoolOp::Intersect, &mut work) {
            Ok(clipped) => clipped,
            Err(_) => return Ok(Overlap::NotCovered { reason: OverlapNotCovered::CannotIntersect }),
        };
        let area: f64 = clipped.iter().map(Region2::area).sum();
        if !(area.is_finite() && area >= 0.0) {
            return Ok(Overlap::NotCovered { reason: OverlapNotCovered::CannotIntersect });
        }
        owned.push((area > 0.0).then_some(clipped));
    }

    let mut pairs = Vec::new();
    for i in 0..pieces.len() {
        for j in (i + 1)..pieces.len() {
            let (Some(left), Some(right)) = (owned.get(i).and_then(Option::as_ref), owned.get(j).and_then(Option::as_ref)) else {
                continue;
            };
            let common = match crate::sheet::material_boolean(left, right, kernel::BoolOp::Intersect, &mut work) {
                Ok(common) => common,
                Err(_) => return Ok(Overlap::NotCovered { reason: OverlapNotCovered::CannotIntersect }),
            };
            let area: f64 = common.iter().map(Region2::area).sum();
            if !(area.is_finite() && area >= 0.0) {
                return Ok(Overlap::NotCovered { reason: OverlapNotCovered::CannotIntersect });
            }
            if area <= OVERLAP_AREA_TOLERANCE_MM2 {
                continue;
            }
            let (Some(piece_a), Some(piece_b)) = (pieces.get(i), pieces.get(j)) else {
                continue;
            };
            pairs.push(OverlapPair { pieces: [piece_a.0, piece_b.0], area_mm2: area, outline: outline_loops(&common) });
        }
    }

    if pairs.is_empty() { Ok(Overlap::Clear) } else { Ok(Overlap::Overlapping { pairs }) }
}

/// Every intersection region's outer loop in flat coordinates, the largest region first.
fn outline_loops(regions: &[Region2]) -> Vec<Vec<[f64; 2]>> {
    let mut loops: Vec<(f64, Vec<[f64; 2]>)> = regions
        .iter()
        .map(|region| {
            let mut points: Vec<[f64; 2]> = region.outer.polyline(1e-3).into_iter().map(|point| [point.x, point.y]).collect();
            // `polyline` repeats the first point at the end; keep one copy.
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            (region.area(), points)
        })
        .collect();
    loops.sort_by(|left, right| right.0.total_cmp(&left.0));
    loops.into_iter().map(|(_, points)| points).collect()
}

/// The folded point of one end of a bend, at mid-depth, through the parent's transform.
fn folded_end(sheet: &SheetBody, f: &crate::sheet::SheetFlange, end: Vec2) -> [f64; 3] {
    let at = apply_point(&sheet.world(f.parent), Vec3::new(end.x, end.y, sheet.t / 2.0));
    [at.x, at.y, at.z]
}

/// Whether a flat other than the bend's own two starts right on the bend's line at this end and
/// runs on past it: the sheet has no margin there for the bend to turn in.
fn starts_on_line(pattern: &FlatPattern, f: &crate::sheet::SheetFlange, i: usize, end: Vec2, outward: Vec2) -> bool {
    let n = f.n.normalized().unwrap_or(Vec2::Y);
    let own = |panel: Option<usize>| panel == f.parent || panel == Some(i);
    let others = pattern
        .flats
        .iter()
        .filter(|flat| !own(flat.panel))
        .map(|flat| &flat.region.outer)
        .chain(pattern.zones.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, zone)| &zone.outer));
    for lp in others {
        for seg in &lp.segs {
            let Seg2::Line { a, b } = *seg else { continue };
            if (a - end).dot(n).abs() > LINEAR_TOL || (b - end).dot(n).abs() > LINEAR_TOL {
                continue;
            }
            let (from, to) = ((a - end).dot(outward), (b - end).dot(outward));
            if from.min(to) <= LINEAR_TOL && from.max(to) > LINEAR_TOL {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::{AngleDisplay, ReliefShape, RuleValues, SheetCountersink, SheetFlange, SheetPerforation, SheetPocket, base_flange};
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

    /// A 40 x 25 plate with a 90° flange on the y = 25 edge and one hole on each panel.
    fn flanged_pair_sheet() -> SheetBody {
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
        let allowance = (PI / 2.0) * (radius + 0.5 * 1.5);
        cut_round(&mut sheet, 20.0, 15.0, 1.5);
        cut_round(&mut sheet, 20.0, 25.0 + allowance + 3.0, 1.5);
        sheet
    }

    /// A whole-body rigid frame: turn about Z, then translate.
    fn turned_frame(angle: f64, translation: Vec3) -> Plane {
        let (sin, cos) = angle.sin_cos();
        Plane { origin: translation, x: Vec3::new(cos, sin, 0.0), y: Vec3::new(-sin, cos, 0.0) }
    }

    fn rotate_z(angle: f64, point: Vec3) -> Vec3 {
        let (sin, cos) = angle.sin_cos();
        Vec3::new(cos * point.x - sin * point.y, sin * point.x + cos * point.y, point.z)
    }

    /// Everything except a folded position is a flat-frame fact, so a rigid whole-body move must
    /// leave it unchanged.
    fn assert_same_flat_facts(before: &SheetFacts, after: &SheetFacts) {
        assert_eq!(after.holes.len(), before.holes.len());
        assert_eq!(after.bends.len(), before.bends.len());
        assert_eq!(after.other_openings, before.other_openings);
        close(after.thickness_mm, before.thickness_mm);
        for (a, b) in before.holes.iter().zip(&after.holes) {
            assert_eq!(b.number, a.number);
            close(b.diameter_mm, a.diameter_mm);
            close(b.depth_mm, a.depth_mm);
            assert_eq!(b.flat, a.flat);
            assert_eq!(b.flat_assigned, a.flat_assigned);
            assert_eq!(b.edge_kind, a.edge_kind);
            match (a.edge_mm, b.edge_mm) {
                (Some(x), Some(y)) => close(y, x),
                (None, None) => {}
                other => panic!("the edge distance changed: {other:?}"),
            }
            assert_eq!(b.bend_gaps.len(), a.bend_gaps.len());
            for (ga, gb) in a.bend_gaps.iter().zip(&b.bend_gaps) {
                assert_eq!(gb.bend, ga.bend);
                close(gb.gap_mm, ga.gap_mm);
                close(gb.inside_radius_mm, ga.inside_radius_mm);
            }
        }
        for (a, b) in before.bends.iter().zip(&after.bends) {
            assert_eq!(b.number, a.number);
            close(b.inside_radius_mm, a.inside_radius_mm);
            close(b.angle_deg, a.angle_deg);
            close(b.length_mm, a.length_mm);
            assert_eq!(b.flats, a.flats);
        }
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

    fn free_form_loop(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> Region2 {
        Region2 { outer: Loop2 { segs: vec![Seg2::Cubic { p0, p1, p2, p3 }] }, holes: Vec::new() }
    }

    /// A free-form cut edge is not approximated, so a hole cannot be given the far outline's
    /// distance when the unknown edge may be nearer.
    #[test]
    fn a_free_form_edge_that_may_be_nearer_leaves_the_hole_edge_unknown() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        cut_round(&mut sheet, 20.0, 12.5, 1.5);
        // A teardrop cut whose cubic reaches x = 22.5, about 1 mm from the hole wall. Its only
        // segment is free-form, so no exact distance to it exists.
        sheet.holes.push(free_form_loop(Vec2::new(30.0, 12.5), Vec2::new(20.0, 11.0), Vec2::new(20.0, 14.0), Vec2::new(30.0, 12.5)));

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.holes.len(), 1);
        let hole = &measured.holes[0];
        assert_eq!(hole.edge_mm, None, "the outline must not stand in for an unmeasured nearer edge");
        assert_eq!(hole.edge_at_mm, None);
        assert_eq!(hole.edge_kind, None);
    }

    /// A free-form edge far outside the outline's distance cannot be nearer, so the exact
    /// outline answer stands.
    #[test]
    fn a_far_free_form_edge_does_not_hide_the_outline_distance() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        cut_round(&mut sheet, 5.0, 12.5, 1.5);
        sheet.holes.push(free_form_loop(Vec2::new(36.0, 12.5), Vec2::new(32.0, 11.0), Vec2::new(32.0, 14.0), Vec2::new(36.0, 12.5)));

        let measured = facts(&sheet).unwrap();
        let hole = &measured.holes[0];
        close(hole.edge_mm.unwrap(), 3.5);
        assert_eq!(hole.edge_kind, Some(EdgeKind::Outline));
    }

    /// A hole already touching an edge has the smallest possible clearance; a free-form cut near
    /// it cannot make the zero unknown.
    #[test]
    fn a_touching_hole_keeps_its_zero_clearance_beside_a_free_form_cut() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        cut_round(&mut sheet, 1.5, 12.5, 1.5);
        sheet.holes.push(free_form_loop(Vec2::new(6.0, 12.5), Vec2::new(2.0, 11.0), Vec2::new(2.0, 14.0), Vec2::new(6.0, 12.5)));

        let measured = facts(&sheet).unwrap();
        let hole = &measured.holes[0];
        close(hole.edge_mm.unwrap(), 0.0);
        assert_eq!(hole.edge_kind, Some(EdgeKind::Outline));
        close(hole.edge_at_mm.unwrap()[0], 0.0);
    }

    fn tangent(origin: Vec2, from: f64, to: f64) -> Tangent {
        Tangent { origin, dir: Vec2::X, from, to }
    }

    fn cut_lines(seg: Seg2, tangents: &[Tangent]) -> Vec<(Vec2, Vec2)> {
        let mut out = Vec::new();
        push_cut_segment(seg, tangents, &mut out);
        out.into_iter()
            .map(|seg| match seg {
                Seg2::Line { a, b } => (a, b),
                other => panic!("a cut of a line must be lines, got {other:?}"),
            })
            .collect()
    }

    fn assert_line(line: (Vec2, Vec2), a: Vec2, b: Vec2) {
        close(line.0.x, a.x);
        close(line.0.y, a.y);
        close(line.1.x, b.x);
        close(line.1.y, b.y);
    }

    /// Two separate bend spans on one line must leave the real cut edge between them.
    #[test]
    fn separated_bend_spans_keep_the_cut_edge_between_them() {
        let tangents = [tangent(Vec2::ZERO, 0.0, 10.0), tangent(Vec2::new(30.0, 0.0), 0.0, 10.0)];
        let lines = cut_lines(Seg2::Line { a: Vec2::ZERO, b: Vec2::new(40.0, 0.0) }, &tangents);
        assert_eq!(lines.len(), 1);
        assert_line(lines[0], Vec2::new(10.0, 0.0), Vec2::new(30.0, 0.0));
    }

    /// Overlapping and touching spans merge, the tangent order does not matter, and a reversed
    /// segment gives the same cut in its own direction.
    #[test]
    fn overlapping_adjacent_and_reordered_bend_spans_agree() {
        let overlapping = [tangent(Vec2::ZERO, 0.0, 20.0), tangent(Vec2::new(10.0, 0.0), 0.0, 20.0)];
        let lines = cut_lines(Seg2::Line { a: Vec2::ZERO, b: Vec2::new(40.0, 0.0) }, &overlapping);
        assert_eq!(lines.len(), 1);
        assert_line(lines[0], Vec2::new(30.0, 0.0), Vec2::new(40.0, 0.0));

        let adjacent = [tangent(Vec2::ZERO, 0.0, 10.0), tangent(Vec2::new(10.0, 0.0), 0.0, 10.0)];
        let lines = cut_lines(Seg2::Line { a: Vec2::ZERO, b: Vec2::new(40.0, 0.0) }, &adjacent);
        assert_eq!(lines.len(), 1);
        assert_line(lines[0], Vec2::new(20.0, 0.0), Vec2::new(40.0, 0.0));

        let separated = [tangent(Vec2::new(30.0, 0.0), 0.0, 10.0), tangent(Vec2::ZERO, 0.0, 10.0)];
        let lines = cut_lines(Seg2::Line { a: Vec2::new(40.0, 0.0), b: Vec2::ZERO }, &separated);
        assert_eq!(lines.len(), 1);
        assert_line(lines[0], Vec2::new(30.0, 0.0), Vec2::new(10.0, 0.0));
    }

    /// The same separated spans through the measured facts: the hole keeps the cut edge at
    /// y = 25 between two bends instead of falling back to the far outline.
    #[test]
    fn separated_bend_spans_leave_the_edge_in_measured_facts() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        let make = |p: Vec2| SheetFlange {
            parent: None,
            p,
            d: Vec2::X,
            len: 10.0,
            n: Vec2::Y,
            angle: FRAC_PI_2,
            radius: 1.5,
            leg: 10.0,
            ext: (0.0, 0.0),
            slope: (0.0, 0.0),
            tear: (false, false),
            shape: None,
            exact: None,
            zone_shape: None,
            tag: String::new(),
        };
        sheet.flanges.push(make(Vec2::new(0.0, 25.0)));
        sheet.flanges.push(make(Vec2::new(30.0, 25.0)));
        cut_round(&mut sheet, 20.0, 20.0, 1.5);

        let measured = facts(&sheet).unwrap();
        let hole = &measured.holes[0];
        assert!(hole.flat_assigned);
        close(hole.edge_mm.unwrap(), 3.5);
        assert_eq!(hole.edge_kind, Some(EdgeKind::Outline));
        let at = hole.edge_at_mm.unwrap();
        close(at[0], 20.0);
        close(at[1], 25.0);
    }

    /// A perforation field is a grid of round through holes on whichever panel holds it. Every
    /// hole has a stable number, its diameter and depth, and a cut-edge distance.
    #[test]
    fn a_perforation_array_reports_every_hole_on_the_base_panel() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        sheet.perforations.push(SheetPerforation {
            feature: 7,
            diameter: 2.0,
            pitch: 5.0,
            margin: 1.0,
            square: true,
            from_right: false,
            outline: None,
        });

        let measured = facts(&sheet).unwrap();
        assert!(measured.flat);
        // The grid starts at margin + radius = 2 mm, so x = 2, 7, ..., 37 and y = 2, 7, ..., 22.
        assert_eq!(measured.holes.len(), 8 * 5);
        assert_eq!(measured.other_openings, 0);
        assert!(measured.holes.iter().all(|hole| hole.flat_assigned && hole.flat.is_none() && hole.diameter_mm == 2.0));
        assert_eq!(measured.holes.iter().map(|hole| hole.number).collect::<Vec<_>>(), (1..=40).collect::<Vec<_>>());

        let first = &measured.holes[0];
        close(first.x_mm, 2.0);
        close(first.y_mm, 2.0);
        close(first.z_mm, 0.75);
        close(first.depth_mm, 1.5);
        assert_eq!(first.flat, None);
        // x = 0 and y = 0 are both 2 mm from the centre, so 1 mm from the Ø2 wall.
        close(first.edge_mm.unwrap(), 1.0);
        assert_eq!(first.edge_kind, Some(EdgeKind::Outline));
        assert!(first.bend_gaps.is_empty());

        let last = &measured.holes[39];
        close(last.x_mm, 37.0);
        close(last.y_mm, 22.0);
        // The far edges x = 40 and y = 25 are 3 mm away, so 2 mm of material.
        close(last.edge_mm.unwrap(), 2.0);
    }

    /// Moving the whole body through its frame moves the folded centres and edge points, but the
    /// measured hole and bend facts stay the same.
    #[test]
    fn a_translated_body_keeps_its_hole_facts() {
        let sheet = flanged_pair_sheet();
        let before = facts(&sheet).unwrap();
        let translation = Vec3::new(5.0, -3.0, 2.0);
        let mut moved = sheet;
        moved.frame = turned_frame(0.0, translation);

        let after = facts(&moved).unwrap();
        assert_same_flat_facts(&before, &after);
        for (a, b) in before.holes.iter().zip(&after.holes) {
            close(b.x_mm, a.x_mm + translation.x);
            close(b.y_mm, a.y_mm + translation.y);
            close(b.z_mm, a.z_mm + translation.z);
            let at = a.edge_at_mm.unwrap();
            let moved_at = b.edge_at_mm.unwrap();
            close(moved_at[0], at[0] + translation.x);
            close(moved_at[1], at[1] + translation.y);
            close(moved_at[2], at[2] + translation.z);
        }
        for (a, b) in before.bends.iter().zip(&after.bends) {
            close(b.x_mm, a.x_mm + translation.x);
            close(b.y_mm, a.y_mm + translation.y);
            close(b.z_mm, a.z_mm + translation.z);
        }
    }

    /// Turning the whole body about Z turns the folded centres and edge points, while the flat
    /// facts, including each bend gap, stay the same.
    #[test]
    fn a_rotated_body_keeps_its_hole_facts() {
        let sheet = flanged_pair_sheet();
        let before = facts(&sheet).unwrap();
        let angle = 0.523_598_775_598_298_8;
        let mut turned = sheet;
        turned.frame = turned_frame(angle, Vec3::ZERO);

        let after = facts(&turned).unwrap();
        assert_same_flat_facts(&before, &after);
        for (a, b) in before.holes.iter().zip(&after.holes) {
            let expected = rotate_z(angle, Vec3::new(a.x_mm, a.y_mm, a.z_mm));
            close(b.x_mm, expected.x);
            close(b.y_mm, expected.y);
            close(b.z_mm, expected.z);
            let at = a.edge_at_mm.unwrap();
            let expected = rotate_z(angle, Vec3::new(at[0], at[1], at[2]));
            let moved_at = b.edge_at_mm.unwrap();
            close(moved_at[0], expected.x);
            close(moved_at[1], expected.y);
            close(moved_at[2], expected.z);
        }
        for (a, b) in before.bends.iter().zip(&after.bends) {
            let expected = rotate_z(angle, Vec3::new(a.x_mm, a.y_mm, a.z_mm));
            close(b.x_mm, expected.x);
            close(b.y_mm, expected.y);
            close(b.z_mm, expected.z);
        }
    }

    fn sink(centre: Vec2, entry_z: f64, major_radius: f64, hole_radius: f64) -> SheetCountersink {
        SheetCountersink { panel: None, centre, entry_z, major_radius, hole_radius, angle: FRAC_PI_2 }
    }

    /// A 90° countersink of Ø4 on a Ø3 hole in a 1.5 mm plate: depth (2 - 1.5) / tan 45 = 0.5,
    /// land 1.5 - 0.5 = 1.0, rim 10.5 mm from the long edge.
    #[test]
    fn countersink_facts_are_hand_computed() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        cut_round(&mut sheet, 20.0, 12.5, 1.5);
        sheet.countersinks.push(sink(Vec2::new(20.0, 12.5), 1.5, 2.0, 1.5));

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.countersinks, 1);
        assert_eq!(measured.holes.len(), 1);
        let hole = &measured.holes[0];
        assert!(hole.flat_assigned);
        let sink = hole.countersink.as_ref().expect("the round hole carries its countersink");
        close(sink.major_diameter_mm, 4.0);
        close(sink.depth_mm, 0.5);
        close(sink.land_mm, 1.0);
        close(sink.angle_deg, 90.0);
        // The rim's centre is on the entry skin, not at the hole's mid-depth.
        close(sink.x_mm, 20.0);
        close(sink.y_mm, 12.5);
        close(sink.z_mm, 1.5);
        // y = 0 is 12.5 - 2 = 10.5 away, measured from the rim rather than the hole wall.
        close(sink.edge_mm.unwrap(), 10.5);
        assert_eq!(sink.edge_kind, Some(EdgeKind::Outline));
        assert!(sink.bend_gaps.is_empty());
    }

    /// A plain through hole has no countersink, and a counterbore is a pocket, not a cone.
    #[test]
    fn a_plain_hole_and_a_counterbore_are_not_countersinks() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        cut_round(&mut sheet, 20.0, 12.5, 1.5);
        let plain = facts(&sheet).unwrap();
        assert_eq!(plain.countersinks, 0);
        assert!(plain.holes[0].countersink.is_none());

        // A counterbore's floor is a pocket of the flat model; it is not a conical countersink.
        sheet.pockets.push(SheetPocket { region: Region2 { outer: Loop2::circle(Vec2::new(20.0, 12.5), 3.0), holes: Vec::new() }, z0: 1.0, z1: 1.5 });
        let counterbored = facts(&sheet).unwrap();
        assert_eq!(counterbored.countersinks, 0);
        assert!(counterbored.holes[0].countersink.is_none());
    }

    /// A countersink the model records but that no round through hole carries must be counted
    /// (so the rule layer says it is unread) and never attached by guessing.
    #[test]
    fn a_countersink_off_a_round_hole_is_counted_but_not_attached() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        // A square cut-out where the countersink is: an opening, not a round hole.
        sheet.holes.push(Region2 {
            outer: Loop2::polygon(&[Vec2::new(18.0, 10.5), Vec2::new(22.0, 10.5), Vec2::new(22.0, 14.5), Vec2::new(18.0, 14.5)]),
            holes: Vec::new(),
        });
        sheet.countersinks.push(sink(Vec2::new(20.0, 12.5), 1.5, 2.0, 1.5));

        let measured = facts(&sheet).unwrap();
        assert!(measured.holes.is_empty(), "a square is an opening, never a hole");
        assert_eq!(measured.other_openings, 1);
        assert_eq!(measured.countersinks, 1, "the countersink is still counted, so it cannot read as none");
    }

    /// A blind countersink is a countersink with no through hole; the far skin is left.
    #[test]
    fn a_blind_countersink_has_no_round_through_hole() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        // A blind bore is a partial pocket of the flat model, not a through hole.
        sheet.pockets.push(SheetPocket { region: Region2 { outer: Loop2::circle(Vec2::new(20.0, 12.5), 1.5), holes: Vec::new() }, z0: 0.5, z1: 1.5 });
        sheet.countersinks.push(sink(Vec2::new(20.0, 12.5), 1.5, 2.0, 1.5));

        let measured = facts(&sheet).unwrap();
        assert!(measured.holes.is_empty());
        assert_eq!(measured.countersinks, 1);
    }

    /// A countersink's rim is measured against the bend and the cut edge, not the hole wall.
    #[test]
    fn a_countersink_beside_a_bend_keeps_the_rim_distance() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        sheet.flanges.push(SheetFlange {
            parent: None,
            p: Vec2::new(0.0, 25.0),
            d: Vec2::X,
            len: 40.0,
            n: Vec2::Y,
            angle: FRAC_PI_2,
            radius: 1.5,
            leg: 10.0,
            ext: (0.0, 0.0),
            slope: (0.0, 0.0),
            tear: (false, false),
            shape: None,
            exact: None,
            zone_shape: None,
            tag: String::new(),
        });
        cut_round(&mut sheet, 20.0, 15.0, 1.5);
        sheet.countersinks.push(sink(Vec2::new(20.0, 15.0), 1.5, 2.0, 1.5));

        let measured = facts(&sheet).unwrap();
        let sink = measured.holes[0].countersink.as_ref().expect("a countersink on the base");
        // y = 0 is 15 - 2 = 13; the bend tangent at y = 25 leaves 10 - 2 = 8.
        close(sink.edge_mm.unwrap(), 13.0);
        assert_eq!(sink.bend_gaps.len(), 1);
        close(sink.bend_gaps[0].gap_mm, 8.0);
        close(sink.bend_gaps[0].inside_radius_mm, 1.5);
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

    /// A 40 x 25 plate with one flange whose flat panel is the given polygon. The bend zone sits
    /// outside the base (y = 25 to 25 + BA), so only the panel can overlap the base in flat.
    fn sheet_with_flange_panel(corners: &[Vec2]) -> SheetBody {
        let mut sheet = plate(40.0, 25.0, 1.5);
        sheet.flanges.push(SheetFlange {
            parent: None,
            p: Vec2::new(0.0, 25.0),
            d: Vec2::X,
            len: 40.0,
            n: Vec2::Y,
            angle: FRAC_PI_2,
            radius: 1.5,
            leg: 10.0,
            ext: (0.0, 0.0),
            slope: (0.0, 0.0),
            tear: (false, false),
            shape: Some(corners.to_vec()),
            exact: None,
            zone_shape: None,
            tag: String::new(),
        });
        sheet
    }

    fn rect_corners(x0: f64, y0: f64, x1: f64, y1: f64) -> [Vec2; 4] {
        [Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)]
    }

    fn overlap_pairs(outcome: &Overlap) -> &[OverlapPair] {
        match outcome {
            Overlap::Overlapping { pairs } => pairs,
            other => panic!("expected an overlap, got {other:?}"),
        }
    }

    /// The panel is far outside the base, so nothing shares material.
    #[test]
    fn separated_flat_pieces_are_clear() {
        let sheet = sheet_with_flange_panel(&rect_corners(60.0, 60.0, 70.0, 70.0));
        assert_eq!(flat_overlap(&sheet).unwrap(), Overlap::Clear);
    }

    /// A 5 x 4 mm panel lies inside the 40 x 25 base: 20 mm² of shared material.
    #[test]
    fn a_known_overlap_area_is_measured_with_its_region_pair() {
        let sheet = sheet_with_flange_panel(&rect_corners(30.0, 5.0, 35.0, 9.0));
        let outcome = flat_overlap(&sheet).unwrap();
        let pairs = overlap_pairs(&outcome);
        assert_eq!(pairs.len(), 1);
        close(pairs[0].area_mm2, 20.0);
        assert_eq!(pairs[0].pieces, [FlatPiece::Base, FlatPiece::Panel(0)]);
        assert_eq!(pairs[0].pieces.map(FlatPiece::label), ["base".to_string(), "panel 0".to_string()]);
        // Locatable evidence: the overlap is the 5 x 4 rectangle itself.
        assert_eq!(pairs[0].outline.len(), 1);
        let outline = &pairs[0].outline[0];
        assert_eq!(outline.len(), 4);
        let (min_x, max_x) = outline.iter().map(|p| p[0]).fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        let (min_y, max_y) = outline.iter().map(|p| p[1]).fold((f64::MAX, f64::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
        close(min_x, 30.0);
        close(max_x, 35.0);
        close(min_y, 5.0);
        close(max_y, 9.0);
    }

    /// Sharing the whole x = 40 edge is contact, not overlap: the intersection has no area.
    #[test]
    fn pieces_that_share_only_an_edge_are_clear() {
        let sheet = sheet_with_flange_panel(&rect_corners(40.0, 0.0, 50.0, 25.0));
        assert_eq!(flat_overlap(&sheet).unwrap(), Overlap::Clear);
    }

    /// Touching at one corner is contact, not overlap.
    #[test]
    fn pieces_that_touch_at_a_point_are_clear() {
        let sheet = sheet_with_flange_panel(&rect_corners(40.0, 25.0, 50.0, 35.0));
        assert_eq!(flat_overlap(&sheet).unwrap(), Overlap::Clear);
    }

    /// The piece outlines cross, but a hole removes the shared material: no material overlap.
    #[test]
    fn an_opening_through_the_shared_area_is_not_overlap() {
        let mut sheet = sheet_with_flange_panel(&rect_corners(30.0, 5.0, 35.0, 9.0));
        // A 9 x 8 opening from (28, 3) to (37, 11) removes the whole 5 x 4 shared rectangle.
        let hole = Loop2::polygon(&rect_corners(28.0, 3.0, 37.0, 11.0));
        sheet.holes.push(Region2 { outer: hole, holes: Vec::new() });
        assert_eq!(flat_overlap(&sheet).unwrap(), Overlap::Clear);
    }

    /// A later feature changes the body after its sheet-metal features, so the flat pattern is not
    /// the part: the check says it is not covered instead of passing.
    #[test]
    fn a_sheet_that_cannot_unfold_is_not_covered() {
        let mut sheet = sheet_with_flange_panel(&rect_corners(30.0, 5.0, 35.0, 9.0));
        sheet.altered_by = Some("Fillet1".to_string());
        assert_eq!(flat_overlap(&sheet).unwrap(), Overlap::NotCovered { reason: OverlapNotCovered::CannotUnfold });
    }

    /// Hand arithmetic either side of the tolerance: 1 x 0.005 = 0.005 mm² is below 0.01 mm²;
    /// 1 x 0.02 = 0.02 mm² is above it.
    #[test]
    fn the_overlap_area_tolerance_is_half_open_at_its_named_value() {
        assert_eq!(OVERLAP_AREA_TOLERANCE_MM2, 0.01);
        let below = sheet_with_flange_panel(&rect_corners(20.0, 20.0, 21.0, 20.005));
        assert_eq!(flat_overlap(&below).unwrap(), Overlap::Clear);

        let above = sheet_with_flange_panel(&rect_corners(20.0, 20.0, 21.0, 20.02));
        let outcome = flat_overlap(&above).unwrap();
        let pairs = overlap_pairs(&outcome);
        assert_eq!(pairs.len(), 1);
        close(pairs[0].area_mm2, 0.02);
    }

    /// The check is in the flat frame, so moving the whole body does not change it.
    #[test]
    fn flat_overlap_is_invariant_under_a_whole_body_move() {
        let sheet = sheet_with_flange_panel(&rect_corners(30.0, 5.0, 35.0, 9.0));
        let before = flat_overlap(&sheet).unwrap();
        let mut moved = sheet;
        moved.frame = turned_frame(0.523_598_775_598_298_8, Vec3::new(5.0, -3.0, 2.0));
        let after = flat_overlap(&moved).unwrap();
        assert_eq!(after, before);
    }

    /// Two bodies are separate inputs: an overlap in one never makes the other fail.
    #[test]
    fn two_sheet_bodies_are_measured_separately() {
        let overlapping = sheet_with_flange_panel(&rect_corners(30.0, 5.0, 35.0, 9.0));
        let clear = sheet_with_flange_panel(&rect_corners(60.0, 60.0, 70.0, 70.0));
        let first = flat_overlap(&overlapping).unwrap();
        let second = flat_overlap(&clear).unwrap();
        assert_eq!(overlap_pairs(&first)[0].area_mm2, 20.0);
        assert_eq!(second, Overlap::Clear);
        assert_eq!(flat_overlap(&overlapping).unwrap(), first, "the other body did not change this one");
    }

    fn body(region: Region2, thickness: f64) -> SheetBody {
        base_flange("Body1", "Test", &rule(thickness), Plane::XY, region)
    }

    fn add_flange(sheet: &mut SheetBody, parent: Option<usize>, p: Vec2, d: Vec2, n: Vec2, len: f64, leg: f64) {
        add_turned_flange(sheet, parent, p, d, n, len, leg, FRAC_PI_2);
    }

    fn add_turned_flange(sheet: &mut SheetBody, parent: Option<usize>, p: Vec2, d: Vec2, n: Vec2, len: f64, leg: f64, angle: f64) {
        sheet.flanges.push(SheetFlange {
            parent,
            p,
            d,
            len,
            n,
            angle,
            radius: 1.5,
            leg,
            ext: (0.0, 0.0),
            slope: (0.0, 0.0),
            tear: (false, false),
            shape: None,
            exact: None,
            zone_shape: None,
            tag: String::new(),
        });
    }

    /// An L-shaped base: y runs to 25 before x = 20 and to 35 after it, so the base carries on
    /// across the bend's line where a 20 mm bend ends.
    fn l_region() -> Region2 {
        Region2 {
            outer: Loop2::polygon(&[
                Vec2::new(0.0, 0.0),
                Vec2::new(40.0, 0.0),
                Vec2::new(40.0, 35.0),
                Vec2::new(20.0, 35.0),
                Vec2::new(20.0, 25.0),
                Vec2::new(0.0, 25.0),
            ])
            .ccw(),
            holes: Vec::new(),
        }
    }

    /// A 90° flange with a 10 mm leg on a 40 x 25 plate: the base side reaches 25 mm from the
    /// tangent, the flange side 10 mm, and nothing cuts either.
    #[test]
    fn flange_runs_measure_both_flats() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        add_flange(&mut sheet, None, Vec2::new(0.0, 25.0), Vec2::X, Vec2::Y, 40.0, 10.0);

        let measured = facts(&sheet).unwrap();
        let bend = &measured.bends[0];
        assert_eq!(bend.runs_mm.len(), 2);
        assert_eq!(bend.runs_mm[0].len(), RUN_SAMPLES);
        assert_eq!(bend.runs_mm[1].len(), RUN_SAMPLES);
        for run in &bend.runs_mm[0] {
            close(*run, 25.0);
        }
        for run in &bend.runs_mm[1] {
            close(*run, 10.0);
        }
        close(bend.longest_mm[0], 25.0);
        close(bend.longest_mm[1], 10.0);
        assert!(!bend.crossed);
        assert!(bend.unrelieved.is_empty(), "no material crosses the bend's line past either end");
        assert!(bend.no_margin.is_empty());
        assert!(bend.reliefs.is_empty());
        assert!(measured.webs.is_empty(), "one bend has no web beside it");
    }

    /// A U channel 30 mm wide with 20 mm legs: one web, tangent to tangent across the base, both
    /// bends leaving the base on the same side.
    #[test]
    fn a_u_channel_reports_its_web() {
        let mut sheet = plate(30.0, 25.0, 1.5);
        add_flange(&mut sheet, None, Vec2::new(0.0, 0.0), Vec2::Y, Vec2::new(-1.0, 0.0), 25.0, 20.0);
        add_flange(&mut sheet, None, Vec2::new(30.0, 0.0), Vec2::Y, Vec2::X, 25.0, 20.0);

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.webs.len(), 1);
        let web = &measured.webs[0];
        assert_eq!(web.bends, [1, 2]);
        close(web.web_mm, 30.0);
        assert!(web.same_side);
        assert_eq!(web.flanges_mm, [20.0, 20.0]);
        close(web.x_mm, 15.0);
        close(web.y_mm, 12.5);
        close(web.z_mm, 0.0);
        for bend in &measured.bends {
            close(bend.longest_mm[0], 30.0);
            close(bend.longest_mm[1], 20.0);
        }
    }

    /// A bend that ends with the base carrying on across its line, with no notch to free it.
    #[test]
    fn a_bend_end_where_the_parent_carries_on_is_unrelieved() {
        let mut sheet = body(l_region(), 1.5);
        add_flange(&mut sheet, None, Vec2::new(0.0, 25.0), Vec2::X, Vec2::Y, 20.0, 10.0);

        let measured = facts(&sheet).unwrap();
        let bend = &measured.bends[0];
        assert_eq!(bend.unrelieved, vec![[20.0, 25.0, 0.75]], "the base past x = 20 crosses the bend's line");
        assert!(bend.no_margin.is_empty());
        assert!(bend.reliefs.is_empty(), "no notch is there to measure");
        for run in &bend.runs_mm[0] {
            close(*run, 25.0);
        }
        for run in &bend.runs_mm[1] {
            close(*run, 10.0);
        }
    }

    /// A 3 mm wide, 5 mm deep notch at the same bend end: measured from the flat's own material,
    /// not invented.
    #[test]
    fn a_notch_at_a_bend_end_is_measured() {
        let mut sheet = body(l_region(), 1.5);
        add_flange(&mut sheet, None, Vec2::new(0.0, 25.0), Vec2::X, Vec2::Y, 20.0, 10.0);
        let notch = Loop2::polygon(&[Vec2::new(20.0, 20.0), Vec2::new(23.0, 20.0), Vec2::new(23.0, 30.0), Vec2::new(20.0, 30.0)]).ccw();
        sheet.holes.push(Region2 { outer: notch, holes: Vec::new() });

        let measured = facts(&sheet).unwrap();
        let bend = &measured.bends[0];
        assert_eq!(bend.reliefs.len(), 1);
        let relief = &bend.reliefs[0];
        close(relief.width_mm, 3.0);
        close(relief.depth_mm, 5.0);
        close(relief.at_mm[0], 21.5);
        close(relief.at_mm[1], 22.5);
        close(relief.at_mm[2], 0.0);
        assert!(bend.unrelieved.is_empty(), "the notch frees the end");
        assert!(bend.no_margin.is_empty());
    }

    /// A round relief notch centred on the bend end: its chord along the bend line and the depth
    /// to its far side come from the arc, not from a chord approximation of it.
    #[test]
    fn a_round_notch_at_a_bend_end_is_measured_from_its_arc() {
        let mut sheet = body(l_region(), 1.5);
        add_flange(&mut sheet, None, Vec2::new(0.0, 25.0), Vec2::X, Vec2::Y, 20.0, 10.0);
        cut_round(&mut sheet, 20.0, 25.0, 1.5);

        let measured = facts(&sheet).unwrap();
        let relief = &measured.bends[0].reliefs;
        assert_eq!(relief.len(), 1);
        assert!((relief[0].width_mm - 3.0).abs() < 1e-6, "{}", relief[0].width_mm);
        close(relief[0].depth_mm, 1.5);
        assert!(measured.bends[0].unrelieved.is_empty(), "the round notch frees the end");
    }

    /// A hole 10 mm up a 30 mm flange stops that flange's run at the hole's near wall, and the
    /// bend is marked crossed so `sm.min_flange` does not compare a cut-short run.
    #[test]
    fn a_hole_that_cuts_a_flange_run_marks_the_bend_crossed() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        add_flange(&mut sheet, None, Vec2::new(0.0, 25.0), Vec2::X, Vec2::Y, 40.0, 30.0);
        let allowance = (PI / 2.0) * (1.5 + 0.5 * 1.5);
        cut_round(&mut sheet, 20.0, 25.0 + allowance + 10.0, 1.5);

        let measured = facts(&sheet).unwrap();
        let bend = &measured.bends[0];
        assert!(bend.crossed);
        close(bend.longest_mm[0], 25.0);
        close(bend.longest_mm[1], 30.0);
        // The 40 mm bend is sampled every 2 mm at 1, 3, ..., 39: two samples run through the
        // hole, 1 mm off its centre (its wall there is sqrt(1.5^2 - 1^2) = 1.118 mm above the
        // centre), so they meet the wall 10 - 1.118 mm from the tangent.
        let short: Vec<f64> = bend.runs_mm[1].iter().copied().filter(|run| *run < 30.0 - 1e-9).collect();
        assert_eq!(short.len(), 2, "{:?}", bend.runs_mm[1]);
        let wall = (1.5f64 * 1.5 - 1.0).sqrt();
        for run in short {
            close(run, 10.0 - wall);
        }
        for run in &bend.runs_mm[0] {
            close(*run, 25.0);
        }
    }

    /// Two flanges leave the shared base on opposite sides: the web is still measured, but it is
    /// not a U.
    #[test]
    fn a_web_whose_bends_leave_the_flat_on_opposite_sides_is_not_a_u() {
        let mut sheet = plate(30.0, 25.0, 1.5);
        add_flange(&mut sheet, None, Vec2::new(0.0, 0.0), Vec2::Y, Vec2::new(-1.0, 0.0), 25.0, 20.0);
        add_turned_flange(&mut sheet, None, Vec2::new(30.0, 0.0), Vec2::Y, Vec2::X, 25.0, 20.0, -FRAC_PI_2);

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.webs.len(), 1);
        close(measured.webs[0].web_mm, 30.0);
        assert!(!measured.webs[0].same_side);
    }

    /// A zigzag: a 20 mm panel between two bends, one off the base and one off that panel. The web
    /// is that panel's tangent-to-tangent width, not its bend allowance included twice.
    #[test]
    fn a_web_between_a_flange_and_its_own_child_uses_the_far_tangent() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        add_flange(&mut sheet, None, Vec2::new(0.0, 25.0), Vec2::X, Vec2::Y, 20.0, 20.0);
        let allowance = (PI / 2.0) * (1.5 + 0.5 * 1.5);
        add_flange(&mut sheet, Some(0), Vec2::new(0.0, 25.0 + allowance + 20.0), Vec2::X, Vec2::Y, 20.0, 10.0);

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.webs.len(), 1);
        let web = &measured.webs[0];
        assert_eq!(web.bends, [1, 2]);
        close(web.web_mm, 20.0);
        assert!(!web.same_side, "the two bends leave the shared panel on opposite sides");
        close(web.flanges_mm[0], 20.0);
        close(web.flanges_mm[1], 10.0);
        assert!(web.x_mm.is_finite() && web.y_mm.is_finite() && web.z_mm.is_finite());
    }

    /// Two bends meeting at a base corner: each end has the other flange's flat starting right on
    /// its bend's line, so neither bend has a margin there.
    #[test]
    fn a_flat_starting_on_the_bend_line_has_no_margin() {
        let mut sheet = plate(40.0, 25.0, 1.5);
        add_flange(&mut sheet, None, Vec2::new(0.0, 25.0), Vec2::X, Vec2::Y, 40.0, 10.0);
        add_flange(&mut sheet, None, Vec2::new(40.0, 5.0), Vec2::Y, Vec2::X, 20.0, 10.0);

        let measured = facts(&sheet).unwrap();
        assert_eq!(measured.bends.len(), 2);
        assert_eq!(measured.bends[0].no_margin, vec![[40.0, 25.0, 0.75]]);
        assert_eq!(measured.bends[1].no_margin, vec![[40.0, 25.0, 0.75]]);
        assert!(measured.bends.iter().all(|bend| bend.unrelieved.is_empty() && bend.reliefs.is_empty()));
    }
}
