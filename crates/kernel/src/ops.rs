//! Operations on bodies: booleans, placements and splits by a plane, through SMLib. Bodies apart
//! from each other join without any intersection work, their records side by side.

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;
use solidcore_record::brep::part;
use solidcraft_geom::brep::placement::Rigid;
use solidcraft_geom::prelude::*;

use crate::body::Body;
use crate::history::{BodyHistory, History};
use crate::{KernelError, Result, guard};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoolOp {
    Union,
    Cut,
    Intersect,
}

/// Boolean of two bodies in SMLib. The result may be empty (`Ok(None)`) for a cut that removes
/// everything or an intersection of disjoint bodies. A result SMLib's check refuses is an error.
///
/// The result carries its exact history ([`Body::history`]) over the operands `[a, b]`, from
/// SMLib's own tags, or none where a body's order cannot be matched to SMLib's. A cut by a tool
/// apart from the body is the body itself (its faces and edges kept), and their intersection empty.
pub fn boolean(a: &Body, b: &Body, op: BoolOp) -> Result<Option<Body>> {
    crate::profile::timed(crate::profile::Kind::Boolean, "boolean", || {
        // A tool apart from the body (their exact boxes, grown by the tolerances, do not meet)
        // leaves a cut the body itself and an intersection nothing, with no intersection work.
        if matches!(op, BoolOp::Cut | BoolOp::Intersect) && apart(a, b)? {
            return Ok(match op {
                BoolOp::Cut => Some(crate::boolean_record::operand(a, 0, &crate::record::counts(&[a, b]))),
                _ => None,
            });
        }
        // A union whose first operand is in several pieces, only some of which meet the tool,
        // would nest the others inside the result as voids (#55): union the pieces that meet
        // and join the rest back untouched. Where that does not apply (one piece, every piece
        // meeting, or the pieces cannot be told apart) the ordinary union keeps its fast path.
        if op == BoolOp::Union
            && let Ok(Some(by_lump)) = guard("union by lump", || union_by_lumps(a, b))
        {
            return Ok(Some(by_lump));
        }
        let r = guard("SMLib boolean", || crate::smlib_boolean::smlib_boolean(a, b, op));
        with_operand_roles(a, b, op, r)
    })
}

/// `a` union `b` when `a` is in several pieces and only some of them meet `b`: the meeting
/// pieces union with `b` one at a time (the first a single piece, which SMLib handles), the
/// pieces apart from `b` join back untouched. SMLib nests a disconnected piece of a multi-lump
/// first operand inside the result as a void, losing material and failing its own check (#55).
/// `None` where the ordinary union applies: `a` is in one piece, `b` is not one piece (a tool
/// spanning pieces is outside this case), every piece meets `b`, or a piece cannot be told
/// apart; an error reading the pieces also leaves the ordinary path to fail in its own words.
fn union_by_lumps(a: &Body, b: &Body) -> Result<Option<Body>> {
    if a.is_mesh() || b.is_mesh() {
        return Ok(None);
    }
    let lumps = a.lumps()?;
    if lumps.len() < 2 || b.lumps()?.len() != 1 {
        return Ok(None);
    }
    let mut meeting = Vec::new();
    let mut apart_lumps = Vec::new();
    for lump in lumps {
        if apart(&lump, b)? {
            apart_lumps.push(lump);
        } else if lump.meets(b)? {
            meeting.push(lump);
        } else {
            apart_lumps.push(lump);
        }
    }
    // Every piece meets the tool: one connected result, which the ordinary union makes.
    if apart_lumps.is_empty() {
        return Ok(None);
    }
    // No piece meets the tool: the whole body and the tool are apart, side by side.
    if meeting.is_empty() {
        let Some(body) = join_apart(&[a.clone(), b.clone()])? else { return Ok(None) };
        return Ok(Some(body.inherit_split_lines(&[a, b])));
    }
    let Some(first) = meeting.first() else { return Ok(None) };
    let Some(first_result) = boolean(first, b, BoolOp::Union)? else { return Ok(None) };
    let mut history = compose_first(first_result.history(), first.history());
    let mut joined_meeting = first_result;
    for lump in meeting.iter().skip(1) {
        let Some(next) = boolean(&joined_meeting, lump, BoolOp::Union)? else { return Ok(None) };
        history = compose_next(history, next.history(), lump.history());
        joined_meeting = next;
    }
    // The meeting pieces are now one body; the others come back as their own pieces.
    let pieces: Vec<Body> = std::iter::once(joined_meeting).chain(apart_lumps.iter().cloned()).collect();
    let counts: Vec<(usize, usize)> = pieces.iter().map(|p| (p.face_count(), p.edge_count())).collect();
    let (body, part) = joined(&pieces)?;
    // The compound's history over [`a`, `b`]: each piece's origins are `a`'s or `b`'s.
    let parts = piece_histories(history, &apart_lumps);
    let history = parts.and_then(|parts| {
        let refs: Vec<&History> = parts.iter().collect();
        crate::history::of_part(&part, &counts).over(&refs).ok()
    });
    let body = body.with_checked_history(history, &[(a.face_count(), a.edge_count()), (b.face_count(), b.edge_count())]).inherit_split_lines(&[a, b]);
    Ok(Some(body))
}

/// The first meeting piece's history over [`a`, `b`]: its union with the tool is over the piece
/// and the tool, and the piece is `a`'s own.
fn compose_first(step: Option<&History>, lump: Option<&History>) -> Option<History> {
    let (step, lump) = (step?, lump?);
    step.clone().substituted(0, lump).ok()
}

/// A later meeting piece's history over [`a`, `b`]: the union is over the accumulator and the
/// piece, so both take their own places, the piece's (which is `a`'s) joining `a`'s again.
fn compose_next(previous: Option<History>, step: Option<&History>, lump: Option<&History>) -> Option<History> {
    let mut step = step?.clone();
    let previous = previous?;
    step.substitute(0, &previous).ok()?;
    step.substitute(2, lump?).ok()?;
    step.renumber(&[Some(0), Some(1), Some(0)], 2).ok()?;
    Some(step)
}

/// History over [`a`, `b`] of each of the pieces [`joined`] combines: the union result first
/// (`history`), then each untouched piece, whose own history is over `a` alone.
fn piece_histories(history: Option<History>, apart_lumps: &[Body]) -> Option<Vec<History>> {
    let mut parts = vec![history?];
    for lump in apart_lumps {
        let mut history = lump.history()?.clone();
        if history.operands != 1 {
            return None;
        }
        history.operands = 2;
        parts.push(history);
    }
    Some(parts)
}

/// The union of `bodies` (at least one) in one SMLib call, each body taking part in about
/// log2(n) of its unions, the result checked once. A result SMLib's check refuses is an error.
///
/// The result carries its exact history over `bodies` in order, as [`boolean`]'s does, and
/// what the operands' faces stood for (their regularization protection and the planar roles on
/// supports it kept).
pub fn union_all(bodies: &[&Body]) -> Result<Body> {
    crate::profile::timed(crate::profile::Kind::Boolean, "union_all", || {
        if bodies.is_empty() {
            return Err(KernelError::Invalid("a union of no bodies".into()));
        }
        guard("SMLib union", || crate::smlib_boolean::smlib_union_all(bodies))?
            .inherit_split_lines(bodies)
            .with_preserved_union_planar_sources(bodies)
    })
}

/// Whether B-rep bodies `a` and `b` are apart: their exact boxes, each grown by twice its
/// record's tolerance (as [`union_apart`] takes them), do not meet. Never for a mesh body. The
/// way to skip work on bodies that cannot meet: a mesh's box is not the body's.
pub fn apart(a: &Body, b: &Body) -> Result<bool> {
    if a.is_mesh() || b.is_mesh() {
        return Ok(false);
    }
    let grown = |x: &Body| x.exact_bounds().map(|bb| bb.inflated(2.0 * x.record().tolerance));
    Ok(!grown(a)?.meets(&grown(b)?))
}

/// A boolean's result with what its operands' faces stood for carried over: regularization
/// protection, the lines they were split along on purpose, and the planar construction and shell roles on supports it kept.
fn with_operand_roles(a: &Body, b: &Body, op: BoolOp, r: Result<Option<Body>>) -> Result<Option<Body>> {
    let r = r
        .map(|result| result.map(|body| body.inherit_regularization_protection(a).inherit_regularization_protection(b).inherit_split_lines(&[a, b])));
    match op {
        BoolOp::Cut => r.and_then(|result| result.map(|body| body.with_preserved_boolean_planar_sources(a, b, true)).transpose()),
        BoolOp::Union => r.and_then(|result| result.map(|body| body.with_preserved_union_planar_sources(&[a, b])).transpose()),
        // Every face of an intersection lies on one operand's face, as a union's does: the same
        // roles carry over (#565).
        BoolOp::Intersect => r.and_then(|result| result.map(|body| body.with_preserved_union_planar_sources(&[a, b])).transpose()),
    }
}

/// The union of bodies when no two of them meet or touch, as one body of several pieces (see
/// [`join_apart`]); `None` when fewer than two are given, one is a mesh, or two of them are not
/// apart (their exact boxes, grown by the bodies' tolerances, overlap).
pub fn union_apart(bodies: &[Body]) -> Result<Option<Body>> {
    crate::profile::timed(crate::profile::Kind::Boolean, "union_apart", || {
        if bodies.len() < 2 || bodies.len() > 10_000 || bodies.iter().any(Body::is_mesh) {
            return Ok(None);
        }
        let mut boxes = Vec::with_capacity(bodies.len());
        for b in bodies {
            boxes.push(b.exact_bounds()?.inflated(2.0 * b.record().tolerance));
        }
        // Sweep along x: only boxes whose x ranges overlap are compared.
        let mut order: Vec<usize> = (0..boxes.len()).collect();
        order.sort_by(|&i, &j| boxes.get(i).map(|b| b.min.x).unwrap_or(0.0).total_cmp(&boxes.get(j).map(|b| b.min.x).unwrap_or(0.0)));
        for (k, &i) in order.iter().enumerate() {
            let Some(a) = boxes.get(i) else { continue };
            for &j in order.iter().skip(k + 1) {
                let Some(b) = boxes.get(j) else { continue };
                if b.min.x > a.max.x {
                    break;
                }
                if a.meets(b) {
                    return Ok(None);
                }
            }
        }
        join_apart(bodies)
    })
}

/// Bodies the caller knows are apart (no two meet or touch), as one body of several pieces,
/// without any intersection work: [`union_apart`] without its check, for tools whose making
/// shows they are apart. Each face and edge of the result is its piece's own (its history over
/// `bodies`). `None` for fewer than two bodies or a mesh.
pub fn join_apart(bodies: &[Body]) -> Result<Option<Body>> {
    if bodies.len() < 2 || bodies.len() > 10_000 || bodies.iter().any(Body::is_mesh) {
        return Ok(None);
    }
    let (joined, part) = joined(bodies)?;
    let counts: Vec<(usize, usize)> = bodies.iter().map(|b| (b.face_count(), b.edge_count())).collect();
    let history = crate::history::of_part(&part, &counts);
    Ok(Some(joined.with_checked_history(Some(history), &counts)))
}

/// Bodies (apart from each other) as one body of several pieces, their face sources carried.
pub(crate) fn join_pieces(pieces: Vec<Body>) -> Result<Option<Body>> {
    match pieces.len() {
        0 => Ok(None),
        1 => Ok(pieces.into_iter().next()),
        _ => joined(&pieces).map(|(body, _)| Some(body)),
    }
}

/// The records of `bodies` side by side, as one body, and where its items come from.
fn joined(bodies: &[Body]) -> Result<(Body, part::Part)> {
    let records: Vec<&solidcore_record::brep::Brep> = bodies.iter().map(Body::record).collect();
    let part = part::compound(&records).map_err(|e| KernelError::Failed(format!("pieces: {e}")))?;
    // Each face's source, numbered as `compound` numbers the inputs: one after another.
    let sources: Vec<Option<crate::FaceSource>> =
        bodies.iter().flat_map(|b| (0..b.face_count()).map(move |f| b.face_sources().and_then(|s| s.get(f)).cloned().flatten())).collect();
    let mapped = bodies
        .iter()
        .any(|b| b.face_sources().is_some())
        .then(|| std::sync::Arc::new(part.faces.iter().map(|&f| sources.get(f as usize).cloned().flatten()).collect::<Vec<_>>()));
    let mut body = Body::from_record(part.brep.clone())?.with_face_sources(mapped);
    body.regularization_protected = bodies.iter().any(|b| b.regularization_protected);
    Ok((body, part))
}

/// Affine transform by a column-major 4x4 matrix (rotations, translations, reflections,
/// scalings). A rigid motion moves the record itself, every parameter and index as it was; any
/// other matrix goes through SMLib's transform. Either way the result's faces and edges are
/// the body's, in its history.
pub fn transform_matrix(body: &Body, m: [[f64; 4]; 4]) -> Result<Body> {
    let _span = solidcraft_profile::span("kernel", "transform_matrix");
    if m.iter().flatten().any(|x| !x.is_finite()) {
        return Err(KernelError::Invalid("non-finite transform".into()));
    }
    if let Some(placed) = body.mesh_transformed(&m) {
        let history = History::identity(&placed);
        return placed.with_history(history, &[body]);
    }
    let sources = body.transformed_sources(m)?;
    let (placed, sources, paint) = match Rigid::of_matrix(&m) {
        Some(motion) => {
            let moved = Body::from_record(solidcraft_geom::brep::placement::moved(body.record(), &motion))?;
            let history = History::identity(&moved);
            (moved.with_history(history, &[body])?, sources, body.paint().cloned())
        }
        None => {
            // SMLib numbers the moved body's faces afresh: each face's source and paint follow it
            // by the history (a face whose origin is not known keeps no source; with no history
            // the paint stays as it was).
            let placed = transformed(body, &m)?;
            let origin: Vec<Option<usize>> = (0..placed.face_count()).map(|f| moved_from(placed.history(), f)).collect();
            let sources = sources.map(|s| std::sync::Arc::new(origin.iter().map(|o| o.and_then(|j| s.get(j).cloned().flatten())).collect()));
            let paint = body.paint().cloned().map(|mut p| {
                if placed.history().is_none() {
                    return p;
                }
                p.faces = p
                    .faces
                    .iter()
                    .flat_map(|fp| {
                        origin.iter().enumerate().filter(|(_, o)| **o == Some(fp.face)).map(|(face, _)| crate::FacePaint { face, ..fp.clone() })
                    })
                    .collect();
                p
            });
            (placed, sources, paint)
        }
    };
    Ok(placed.with_color(body.color()).with_paint(paint).with_face_sources(sources).inherit_regularization_protection(body))
}

/// The face of the operand that face `f` of a moved body is, by the move's history.
fn moved_from(history: Option<&History>, f: usize) -> Option<usize> {
    match history?.faces.get(f)?.origins.as_slice() {
        [crate::history::FaceOrigin::Face { operand: 0, face }] => Some(*face),
        _ => None,
    }
}

/// `body` moved by a matrix that is not a rigid motion, in SMLib, its history from SMLib's tags.
fn transformed(body: &Body, m: &[[f64; 4]; 4]) -> Result<Body> {
    let failed = |what: &str, e: solidcraft_smlib::Error| KernelError::smlib(format!("SMLib transform: {what}"), e);
    let (handle, order) = crate::native::smlib_body_ordered(body)?;
    let moved = handle.tagged(0).map_err(|e| failed("tag", e))?.transformed(m).map_err(|e| failed("operation", e))?;
    let (result, result_order) = crate::native::body_from_smlib_ordered(&moved)?;
    let history = match (order, result_order) {
        (Some(o), Some(r)) => moved.history().ok().and_then(|h| h.history(&[o], &r).ok()),
        _ => None,
    };
    Ok(result.with_checked_history(history, &[(body.face_count(), body.edge_count())]))
}

/// [`transform_matrix`] for showing a body where an assembly places it (world placement).
pub fn place_matrix(body: &Body, m: [[f64; 4]; 4]) -> Result<Body> {
    transform_matrix(body, m)
}

/// Rigid transform: rotate about `axis` through `origin` by `angle` radians, then translate.
pub fn transform(body: &Body, translate: Vec3, origin: Vec3, axis: Vec3, angle: f64) -> Result<Body> {
    if !(translate.is_finite() && origin.is_finite() && angle.is_finite()) {
        return Err(KernelError::Invalid("non-finite transform".into()));
    }
    // Same motion as a matrix: rotate about the line through `origin`, then translate.
    let ax = if angle.abs() > 1e-12 { axis.normalized().ok_or_else(|| KernelError::Invalid("rotation axis".into()))? } else { Vec3::Z };
    let (c, s) = (angle.cos(), angle.sin());
    let t = 1.0 - c;
    let r = [
        [t * ax.x * ax.x + c, t * ax.x * ax.y + s * ax.z, t * ax.x * ax.z - s * ax.y],
        [t * ax.x * ax.y - s * ax.z, t * ax.y * ax.y + c, t * ax.y * ax.z + s * ax.x],
        [t * ax.x * ax.z + s * ax.y, t * ax.y * ax.z - s * ax.x, t * ax.z * ax.z + c],
    ];
    let ro = Vec3::new(
        r[0][0] * origin.x + r[1][0] * origin.y + r[2][0] * origin.z,
        r[0][1] * origin.x + r[1][1] * origin.y + r[2][1] * origin.z,
        r[0][2] * origin.x + r[1][2] * origin.y + r[2][2] * origin.z,
    );
    let tr = origin - ro + translate;
    let m = [[r[0][0], r[0][1], r[0][2], 0.0], [r[1][0], r[1][1], r[1][2], 0.0], [r[2][0], r[2][1], r[2][2], 0.0], [tr.x, tr.y, tr.z, 1.0]];
    transform_matrix(body, m)
}

/// Split a body with a plane: the parts on the positive and the negative side of the plane
/// (only the non-empty ones).
pub fn split_by_plane(body: &Body, plane: &solidcore_record::Plane) -> Result<Vec<Body>> {
    let _span = solidcraft_profile::span("kernel", "split_by_plane");
    let mut out = Vec::new();
    for positive in [true, false] {
        out.extend(half_space(body, plane, positive)?.0);
    }
    Ok(out)
}

/// The part of a body on one side of a plane (`None` when nothing is there), and the tool it is
/// cut with: the part is the body's intersection with [`half_box`] (the tool), whose history it
/// carries, over the body and the box. The box's face in the plane, the part's cut face, is its
/// start cap on the positive side and its end cap on the negative side.
pub fn half_space(body: &Body, plane: &solidcore_record::Plane, positive: bool) -> Result<(Option<Body>, Body)> {
    let _span = solidcraft_profile::span("kernel", "half_space");
    // The box's own construction roles describe no face of the design: the cut face has none.
    let tool = half_box(body, plane, positive)?.with_face_sources(None);
    Ok((boolean(body, &tool, BoolOp::Intersect)?, tool))
}

/// A box covering everything of `body` on one side of a plane, its start cap in the plane.
pub(crate) fn half_box(body: &Body, plane: &solidcore_record::Plane, positive: bool) -> Result<Body> {
    body.require_brep("split")?;
    let bb = body.exact_bounds()?;
    let reach = (bb.diagonal() + plane.origin.dist(bb.center())) * 2.0 + body.size();
    let c = plane.to_local(bb.center());
    let sq = solidcore_record::Region2 {
        outer: solidcore_record::Loop2::polygon(&[
            solidcore_record::Vec2::new(c.x - reach, c.y - reach),
            solidcore_record::Vec2::new(c.x + reach, c.y - reach),
            solidcore_record::Vec2::new(c.x + reach, c.y + reach),
            solidcore_record::Vec2::new(c.x - reach, c.y + reach),
        ]),
        holes: vec![],
    };
    let (lo, hi) = if positive { (0.0, reach) } else { (-reach, 0.0) };
    crate::build::extrude(plane, std::slice::from_ref(&sq), lo, hi)?.pop().ok_or_else(|| KernelError::Failed("half space".into()))
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod placement_tests;
