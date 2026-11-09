//! Property tests for the kernel boolean (`boolean`, `measure`, `transform`).
//!
//! Each test states one invariant that every correct B-rep boolean holds. proptest picks the
//! inputs, and when a case fails it shrinks the inputs to a small case that still fails. Paste
//! that case into a normal `#[test]` as a regression test.
//!
//! Case count: `PROPTEST_CASES` (default 8). Pull requests and pushes to main skip this file
//! (.config/nextest.toml, .github/workflows/ci.yml): its random inputs differ on every run, so a
//! known kernel bug would turn main red at random. The nightly `property-tests` job runs it with
//! more cases. A long local run:
//! `PROPTEST_CASES=2000 cargo test -p solidcraft-kernel --test boolean_props`.
//! proptest saves every failing seed in `tests/boolean_props.proptest-regressions`. Commit it, and
//! copy the shrunk case into a `#[test]` below (ignored, with its issue, until the kernel is fixed).

use proptest::prelude::*;
use solidcore_record::Vec3;
use solidcraft_kernel::{Body, BoolOp, boolean, box_solid, cylinder, measure, sphere, transform};

/// Relative volume tolerance. The same 0.2 % that `fuzz::boolean_case` uses.
const REL: f64 = 2e-3;

#[derive(Clone, Debug)]
enum Prim {
    Box { at: [f64; 3], size: [f64; 3] },
    Cyl { at: [f64; 3], axis: usize, r: f64, h: f64 },
    Sph { at: [f64; 3], r: f64 },
}

impl Prim {
    fn body(&self) -> Body {
        let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        match *self {
            Prim::Box { at, size } => box_solid(v(at), v([at[0] + size[0], at[1] + size[1], at[2] + size[2]])),
            Prim::Cyl { at, axis, r, h } => cylinder(v(at), axes[axis], r, h),
            Prim::Sph { at, r } => sphere(v(at), r),
        }
        .expect("a primitive with positive size must build")
    }
}

/// Coordinates on a 2.5 mm grid: coincident faces, shared edges and tangencies come up often.
/// These are the cases that break B-rep booleans.
fn grid() -> impl Strategy<Value = f64> + Clone {
    (0u32..=12).prop_map(|i| i as f64 * 2.5)
}
/// Size on the grid, never zero.
fn grid_size() -> impl Strategy<Value = f64> + Clone {
    (2u32..=12).prop_map(|i| i as f64 * 2.5)
}
/// Generic coordinates: almost never coincident. A failure here is a plain bug, not an edge case.
fn free() -> impl Strategy<Value = f64> + Clone {
    0.0f64..30.0
}
fn free_size() -> impl Strategy<Value = f64> + Clone {
    3.0f64..30.0
}

fn prim(c: impl Strategy<Value = f64> + Clone, s: impl Strategy<Value = f64> + Clone) -> impl Strategy<Value = Prim> {
    let p = [c.clone(), c.clone(), c];
    prop_oneof![
        5 => (p.clone(), [s.clone(), s.clone(), s.clone()]).prop_map(|(at, size)| Prim::Box { at, size }),
        3 => (p.clone(), 0usize..3, s.clone(), s.clone()).prop_map(|(at, axis, r, h)| Prim::Cyl { at, axis, r: r / 2.0, h }),
        2 => (p, s).prop_map(|(at, r)| Prim::Sph { at, r: r / 2.0 }),
    ]
}
fn grid_prim() -> impl Strategy<Value = Prim> {
    prim(grid(), grid_size())
}

/// A union across only a box edge or corner is not a manifold solid: SMLib returns
/// the two boxes as separate lumps that touch along the edge or at the vertex.
fn edge_only_boxes(a: &Prim, b: &Prim) -> bool {
    let (Prim::Box { at: a, size: sa }, Prim::Box { at: b, size: sb }) = (a, b) else { return false };
    let gap = [0, 1, 2].map(|i| a[i].max(b[i]) - (a[i] + sa[i]).min(b[i] + sb[i]));
    gap.iter().all(|g| *g <= 0.0) && gap.iter().filter(|g| **g == 0.0).count() >= 2
}

/// #61: the exact Box/Sphere face point-tangency refusal the property lane accepts as safe.
///
/// This is deliberately narrow. One operand is a box and the other a sphere; the sphere is
/// outside exactly one face slab, strictly inside the other two, and its centre is exactly the
/// radius from the closest point on that face. The contact is therefore one point inside one
/// face, never an edge, a corner or a face area, and there is no volume overlap.
#[derive(Clone, Copy, Debug)]
struct PointTangency {
    /// The single contact point on the box face.
    point: [f64; 3],
    /// The sphere radius, equal to the distance from its centre to the face.
    radius: f64,
    /// The distance SMLib reports in its Bad Vertex Gap line.
    offset: f64,
}

#[derive(Clone, Copy, Debug)]
enum UnionOutcome {
    Volume(f64),
    PointTangent(PointTangency),
}

/// The operands' observable state, to prove a refused boolean did not modify them.
#[derive(Clone, Debug, PartialEq)]
struct OperandState {
    faces: usize,
    edges: usize,
    vertices: usize,
    volume_bits: Option<u64>,
    bounds_bits: Option<[u64; 6]>,
    validity: Vec<String>,
}

fn operand_state(b: &Body) -> OperandState {
    let volume_bits = measure(b).ok().map(|m| m.volume.to_bits());
    let bounds_bits = b.exact_bounds().ok().map(|bb| [bb.min.x, bb.min.y, bb.min.z, bb.max.x, bb.max.y, bb.max.z].map(f64::to_bits));
    OperandState { faces: b.face_count(), edges: b.edge_count(), vertices: b.vertex_count(), volume_bits, bounds_bits, validity: b.validity() }
}

fn box_sphere_point_tangency(a: &Prim, b: &Prim) -> Option<PointTangency> {
    let (at, size, c, r) = match (a, b) {
        (Prim::Box { at, size }, Prim::Sph { at: c, r }) => (at, size, c, *r),
        (Prim::Sph { at: c, r }, Prim::Box { at, size }) => (at, size, c, *r),
        _ => return None,
    };
    let min = *at;
    let max = [at[0] + size[0], at[1] + size[1], at[2] + size[2]];
    let outside: Vec<usize> = (0..3).filter(|&i| c[i] < min[i] || c[i] > max[i]).collect();
    if outside.len() != 1 {
        return None;
    }
    let axis = outside[0];
    // The other two coordinates strictly inside means the closest point is inside a face,
    // not on one of its edges or at a corner.
    if (0..3).any(|i| i != axis && (c[i] <= min[i] || c[i] >= max[i])) {
        return None;
    }
    let point = [c[0].clamp(min[0], max[0]), c[1].clamp(min[1], max[1]), c[2].clamp(min[2], max[2])];
    let distance = (0..3).map(|i| (c[i] - point[i]).powi(2)).sum::<f64>().sqrt();
    if distance != r {
        return None;
    }
    Some(PointTangency { point, radius: r, offset: distance })
}

const BAD_VERTEX_GAP: &str = "SmVertex::Test #1 Bad Vertex Gap: Vertex-Face Gap exceeds XSectTol3d for one (or more) Faces";

/// The offset of the exact full `SmVertex::Test #1 Bad Vertex Gap` line, or `None` for any
/// other error (including a bare `Bad Vertex Gap` substring or a different offset format).
fn bad_vertex_gap_offset(error: &str) -> Option<f64> {
    if !error.contains("SMLib boolean: SMLib's check refuses the result") {
        return None;
    }
    let line = error.lines().find(|line| line.starts_with(BAD_VERTEX_GAP))?;
    let rest = line.strip_prefix(BAD_VERTEX_GAP)?.strip_prefix(" (offset=")?;
    let (offset, tail) = rest.split_once(", tolerance= 2e-05) [owner SmVertex]")?;
    if !tail.is_empty() {
        return None;
    }
    offset.trim().parse().ok()
}

/// `Some` only for the exact #61 refusal: the point-tangency geometry, the exact SMLib
/// signature with its offset, an empty intersection, and operands unchanged across both calls.
fn point_tangency_refusal(
    a: &Body,
    b: &Body,
    prim_a: &Prim,
    prim_b: &Prim,
    error: &str,
    before: &[OperandState; 2],
    after_union: &[OperandState; 2],
) -> Option<PointTangency> {
    let proof = box_sphere_point_tangency(prim_a, prim_b)?;
    let offset = bad_vertex_gap_offset(error)?;
    if (offset - proof.offset).abs() > 1e-9 * proof.offset.abs().max(1.0) {
        return None;
    }
    if before != after_union {
        return None;
    }
    // No positive-volume intersection and no edge/face contact: the intersection must be empty.
    if !matches!(boolean(a, b, BoolOp::Intersect), Ok(None)) {
        return None;
    }
    let after_intersect = [operand_state(a), operand_state(b)];
    if before != &after_intersect {
        return None;
    }
    Some(proof)
}

fn run_union(a: &Body, b: &Body, prim_a: &Prim, prim_b: &Prim) -> Result<UnionOutcome, TestCaseError> {
    let before = [operand_state(a), operand_state(b)];
    match boolean(a, b, BoolOp::Union) {
        Ok(None) => Err(TestCaseError::fail("Union returned no body")),
        Ok(Some(body)) => {
            let after = [operand_state(a), operand_state(b)];
            prop_assert_eq!(&before, &after, "Union modified an operand");
            let bad = body.validity();
            prop_assert!(bad.is_empty(), "Union result is not a sound solid: {}", bad.join("; "));
            Ok(UnionOutcome::Volume(measure(&body).map_err(|e| TestCaseError::fail(format!("measure: {e}")))?.volume))
        }
        Err(e) => {
            let after = [operand_state(a), operand_state(b)];
            if let Some(proof) = point_tangency_refusal(a, b, prim_a, prim_b, &e.to_string(), &before, &after) {
                println!(
                    "expected safe point-tangency refusal (#61): {prim_a:?} and {prim_b:?} touch at {:?} (radius {:.6}, offset {:.6}); operands unchanged",
                    proof.point, proof.radius, proof.offset
                );
                Ok(UnionOutcome::PointTangent(proof))
            } else {
                Err(TestCaseError::fail(format!("Union returned an unexpected error: {e}")))
            }
        }
    }
}

fn translated(p: &Prim, t: Vec3) -> Prim {
    match p.clone() {
        Prim::Box { at, size } => Prim::Box { at: [at[0] + t.x, at[1] + t.y, at[2] + t.z], size },
        Prim::Cyl { at, axis, r, h } => Prim::Cyl { at: [at[0] + t.x, at[1] + t.y, at[2] + t.z], axis, r, h },
        Prim::Sph { at, r } => Prim::Sph { at: [at[0] + t.x, at[1] + t.y, at[2] + t.z], r },
    }
}

fn free_prim() -> impl Strategy<Value = Prim> {
    prim(free(), free_size())
}

/// Volume of a boolean result (an empty result has volume 0). Also fails the case when the
/// result is not a sound solid.
fn run(a: &Body, b: &Body, op: BoolOp) -> Result<f64, TestCaseError> {
    let r = boolean(a, b, op).map_err(|e| TestCaseError::fail(format!("{op:?} returned an error: {e}")))?;
    match r {
        None => Ok(0.0),
        Some(body) => {
            let bad = body.validity();
            prop_assert!(bad.is_empty(), "{op:?} result is not a sound solid: {}", bad.join("; "));
            Ok(measure(&body).map_err(|e| TestCaseError::fail(format!("measure: {e}")))?.volume)
        }
    }
}
fn vol(b: &Body) -> f64 {
    measure(b).expect("measure a primitive").volume
}
fn close(x: f64, y: f64, scale: f64) -> bool {
    (x - y).abs() <= REL * scale.max(1e-9)
}

/// Inclusion-exclusion: V(A∪B) + V(A∩B) = V(A) + V(B), and V(A−B) = V(A) − V(A∩B).
fn identity(a: &Prim, b: &Prim) -> Result<(), TestCaseError> {
    let (ba, bb) = (a.body(), b.body());
    let (va, vb) = (vol(&ba), vol(&bb));
    let union = run_union(&ba, &bb, a, b)?;
    let (vi, vc) = (run(&ba, &bb, BoolOp::Intersect)?, run(&ba, &bb, BoolOp::Cut)?);
    match union {
        UnionOutcome::Volume(vu) => {
            prop_assert!(
                close(vu + vi, va + vb, va + vb),
                "union+intersect {} != A+B {} (A {va:.3}, B {vb:.3}, U {vu:.3}, I {vi:.3})",
                vu + vi,
                va + vb
            );
        }
        UnionOutcome::PointTangent(proof) => {
            prop_assert!(close(vi, 0.0, va + vb), "point-tangent union at {:?} must have no intersection volume (got {vi:.6})", proof.point);
        }
    }
    prop_assert!(close(vc, va - vi, va + vb), "cut {vc:.3} != A-I {:.3}", va - vi);
    Ok(())
}

fn scale(a: &Prim, b: &Prim, k: f64) -> Result<(), TestCaseError> {
    let sc = |p: &Prim| match p.clone() {
        Prim::Box { at, size } => Prim::Box { at: at.map(|x| x * k), size: size.map(|x| x * k) },
        Prim::Cyl { at, axis, r, h } => Prim::Cyl { at: at.map(|x| x * k), axis, r: r * k, h: h * k },
        Prim::Sph { at, r } => Prim::Sph { at: at.map(|x| x * k), r: r * k },
    };
    let (sa, sb) = (sc(a), sc(b));
    let (ba, bb, ka, kb) = (a.body(), b.body(), sa.body(), sb.body());
    let s = vol(&ba) + vol(&bb);
    let length = s.cbrt();
    for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Cut] {
        if op == BoolOp::Union {
            match (run_union(&ba, &bb, a, b)?, run_union(&ka, &kb, &sa, &sb)?) {
                (UnionOutcome::Volume(one), UnionOutcome::Volume(big)) => {
                    prop_assert!(close(one, big / (k * k * k), s), "Union at scale 1 gives {one:.4}, at scale {k} gives {big:.4} (normalised)");
                }
                (UnionOutcome::PointTangent(one), UnionOutcome::PointTangent(big)) => {
                    prop_assert!(
                        close(one.radius, big.radius / k, length),
                        "point-tangency radius at scale 1 is {:.6}, at scale {k} {:.6} (normalised)",
                        one.radius,
                        big.radius
                    );
                    for i in 0..3 {
                        prop_assert!(
                            close(one.point[i], big.point[i] / k, length),
                            "point-tangency contact {i} at scale 1 is {:.6}, at scale {k} {:.6} (normalised)",
                            one.point[i],
                            big.point[i]
                        );
                    }
                }
                _ => prop_assert!(false, "Union refused as a point tangency at one scale only"),
            }
        } else {
            let (one, big) = (run(&ba, &bb, op)?, run(&ka, &kb, op)? / (k * k * k));
            prop_assert!(close(one, big, s), "{op:?} at scale 1 gives {one:.4}, at scale {k} gives {big:.4} (normalised)");
        }
    }
    Ok(())
}

fn cfg(default: u32) -> ProptestConfig {
    // The env var overrides the default; proptest reads PROPTEST_CASES itself only when the
    // config does not set `cases`, so read it here.
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(default);
    // Integration tests have no lib.rs or main.rs beside them, so proptest's default persistence
    // cannot place its file; name it.
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/boolean_props.proptest-regressions");
    ProptestConfig {
        cases,
        max_shrink_iters: 200,
        failure_persistence: Some(Box::new(prop::test_runner::FileFailurePersistence::Direct(file))),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(cfg(8))]

    /// Generic placement: these must all pass. A failure is a real bug.
    #[test]
    fn identity_generic(a in free_prim(), b in free_prim()) {
        identity(&a, &b)?;
    }

    /// Grid placement: coincident and tangent faces. The hard cases.
    #[test]
    fn identity_on_grid(a in grid_prim(), b in grid_prim()) {
        identity(&a, &b)?;
    }

    /// Union and intersection do not depend on operand order.
    #[test]
    fn commutative(a in grid_prim(), b in grid_prim()) {
        let (ba, bb) = (a.body(), b.body());
        let s = vol(&ba) + vol(&bb);
        let length = s.cbrt();
        for op in [BoolOp::Union, BoolOp::Intersect] {
            if op == BoolOp::Union {
                match (run_union(&ba, &bb, &a, &b)?, run_union(&bb, &ba, &b, &a)?) {
                    (UnionOutcome::Volume(ab), UnionOutcome::Volume(ba_)) => {
                        prop_assert!(close(ab, ba_, s), "{op:?}: A,B gives {ab:.4} but B,A gives {ba_:.4}");
                    }
                    (UnionOutcome::PointTangent(one), UnionOutcome::PointTangent(two)) => {
                        prop_assert!(close(one.radius, two.radius, length), "point-tangency radius: A,B gives {:.6}, B,A gives {:.6}", one.radius, two.radius);
                        for i in 0..3 {
                            prop_assert!(close(one.point[i], two.point[i], length), "point-tangency contact {i}: A,B gives {:.6}, B,A gives {:.6}", one.point[i], two.point[i]);
                        }
                    }
                    _ => prop_assert!(false, "Union refused as a point tangency in one order only"),
                }
            } else {
                let (ab, ba_) = (run(&ba, &bb, op)?, run(&bb, &ba, op)?);
                prop_assert!(close(ab, ba_, s), "{op:?}: A,B gives {ab:.4} but B,A gives {ba_:.4}");
            }
        }
    }

    /// Moving both operands together does not change the answer. A large offset exposes
    /// absolute tolerances (1e-6, 1e-9 literals) that should scale with the model.
    #[test]
    fn translation_invariant(a in grid_prim(), b in grid_prim(), off in prop::sample::select(vec![0.0, 1.0e3, 1.0e5])) {
        let (ba, bb) = (a.body(), b.body());
        let t = Vec3::new(off, -off * 0.7, off * 0.3);
        let mv = |x: &Body| transform(x, t, Vec3::ZERO, Vec3::Z, 0.0).expect("translate");
        let (ta, tb) = (mv(&ba), mv(&bb));
        let (pa, pb) = (translated(&a, t), translated(&b, t));
        let s = vol(&ba) + vol(&bb);
        let length = s.cbrt();
        let offset = [t.x, t.y, t.z];
        for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Cut] {
            if op == BoolOp::Union {
                match (run_union(&ba, &bb, &a, &b)?, run_union(&ta, &tb, &pa, &pb)?) {
                    (UnionOutcome::Volume(here), UnionOutcome::Volume(there)) => {
                        prop_assert!(close(here, there, s), "{op:?} at origin {here:.4}, moved by {off} gives {there:.4}");
                    }
                    (UnionOutcome::PointTangent(one), UnionOutcome::PointTangent(two)) => {
                        prop_assert!(close(one.radius, two.radius, length), "point-tangency radius at origin {:.6}, moved by {off} gives {:.6}", one.radius, two.radius);
                        for (i, step) in offset.iter().enumerate() {
                            prop_assert!(close(one.point[i] + step, two.point[i], length), "point-tangency contact {i} at origin {:.6}, moved by {off} gives {:.6}", one.point[i], two.point[i]);
                        }
                    }
                    _ => prop_assert!(false, "Union refused as a point tangency at the origin or moved, not both"),
                }
            } else {
                let (here, there) = (run(&ba, &bb, op)?, run(&ta, &tb, op)?);
                prop_assert!(close(here, there, s), "{op:?} at origin {here:.4}, moved by {off} gives {there:.4}");
            }
        }
    }

    /// Scaling the whole problem scales volume by k³. Small parts (0.05x) and large parts
    /// (40x) expose tolerances that do not scale.
    #[test]
    fn scale_invariant(a in grid_prim(), b in grid_prim(), k in prop::sample::select(vec![0.05, 1.0, 40.0])) {
        scale(&a, &b, k)?;
    }

    /// A∪A = A, A∩A = A, A−A = nothing.
    #[test]
    fn self_operations(a in grid_prim()) {
        let ba = a.body();
        let va = vol(&ba);
        prop_assert!(close(run(&ba, &ba, BoolOp::Union)?, va, va), "A∪A != A");
        prop_assert!(close(run(&ba, &ba, BoolOp::Intersect)?, va, va), "A∩A != A");
        prop_assert!(run(&ba, &ba, BoolOp::Cut)? <= REL * va, "A−A is not empty");
    }

    /// The same inputs give the same output, twice in a row. The boolean keeps thread-local
    /// state (the nudge guard); this catches state that leaks from one call to the next.
    #[test]
    fn deterministic(a in grid_prim(), b in grid_prim()) {
        let (ba, bb) = (a.body(), b.body());
        for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Cut] {
            let m = |r: Option<Body>| r.map(|x| { let m = measure(&x).unwrap(); (m.volume.to_bits(), m.faces, m.edges) });
            let first = boolean(&ba, &bb, op).ok().map(m);
            let second = boolean(&ba, &bb, op).ok().map(m);
            prop_assert_eq!(first, second, "{:?} gave two different answers for the same input", op);
        }
    }

    /// Two boxes whose overlap is known exactly (closed form, no tessellation guesswork).
    #[test]
    fn box_box_exact(a in [grid(), grid(), grid()], sa in [grid_size(), grid_size(), grid_size()],
                     b in [grid(), grid(), grid()], sb in [grid_size(), grid_size(), grid_size()]) {
        let overlap: f64 = (0..3).map(|i| ((a[i] + sa[i]).min(b[i] + sb[i]) - a[i].max(b[i])).max(0.0)).product();
        let (pa, pb) = (Prim::Box { at: a, size: sa }, Prim::Box { at: b, size: sb });
        let (ba, bb) = (pa.body(), pb.body());
        let (va, vb) = (sa.iter().product::<f64>(), sb.iter().product::<f64>());
        let s = va + vb;
        prop_assert!(close(run(&ba, &bb, BoolOp::Intersect)?, overlap, s), "intersect != closed-form overlap {overlap}");
        prop_assert!(close(run(&ba, &bb, BoolOp::Cut)?, va - overlap, s), "cut != closed form {}", va - overlap);
        prop_assert!(close(run(&ba, &bb, BoolOp::Union)?, s - overlap, s), "union != closed form {}", s - overlap);
    }

    /// Two boxes that share part of a face must fuse into one solid. This is the most common
    /// join in real parts (a boss on a plate).
    #[test]
    fn face_touching_boxes_fuse(x in grid(), y in grid(), w in grid_size(), d in grid_size(), h in grid_size()) {
        let plate = Prim::Box { at: [0.0, 0.0, 0.0], size: [30.0, 30.0, 5.0] }.body();
        let boss = Prim::Box { at: [x.min(27.5), y.min(27.5), 5.0], size: [w, d, h] }.body();
        let (vp, vb) = (vol(&plate), vol(&boss));
        prop_assert!(close(run(&plate, &boss, BoolOp::Union)?, vp + vb, vp + vb), "plate + boss lost or gained volume");
    }
}

// Cases the property tests found, as plain tests. Ignored until the kernel bug is fixed: run them
// with `cargo test -p solidcraft-kernel --test boolean_props -- --ignored`.

/// The saved commutativity case: SMLib returns the edge-only union as two closed
/// solids meeting along an edge, not as a refusal or one merged solid. The name is
/// historical (the pre-SMLib kernel refused this join). Intersection and cut remain valid.
#[test]
fn edge_only_box_join_is_refused_in_both_orders() {
    use solidcore_record::brep::part::{Contact, RegionContact};
    let a = Prim::Box { at: [15.0, 22.5, 0.0], size: [5.0, 5.0, 17.5] };
    let b = Prim::Box { at: [2.5, 12.5, 0.0], size: [12.5, 10.0, 5.0] };
    assert!(edge_only_boxes(&a, &b));
    assert!(!edge_only_boxes(&a, &Prim::Box { at: [15.0, 22.5, 0.0], size: [5.0, 5.0, 5.0] }));
    let (a, b) = (a.body(), b.body());
    let (va, vb) = (vol(&a), vol(&b));
    for (left, right) in [(&a, &b), (&b, &a)] {
        let body = boolean(left, right, BoolOp::Union).expect("an edge-only union is a body").expect("an edge-only union is not empty");
        assert!(body.validity().is_empty(), "the union is not sound: {:?}", body.validity());
        let lumps = body.lumps().expect("the union's lumps");
        assert_eq!(lumps.len(), 2, "an edge-only union is two solids, not {}", lumps.len());
        let mut lump_volumes: Vec<f64> = lumps.iter().map(|l| measure(l).expect("measure a lump").volume).collect();
        lump_volumes.sort_by(f64::total_cmp);
        let mut expected = [va, vb];
        expected.sort_by(f64::total_cmp);
        assert!(close(lump_volumes[0], expected[0], va + vb) && close(lump_volumes[1], expected[1], va + vb), "{lump_volumes:?} != {expected:?}");
        assert_eq!(body.region_contacts().expect("the union's contacts"), vec![RegionContact { regions: [0, 1], contact: Contact::Edge }]);
        assert!(close(measure(&body).expect("measure the union").volume, 1062.5, va + vb), "the union volume changed");
        assert_eq!(run(left, right, BoolOp::Intersect).unwrap(), 0.0);
        assert!(close(run(left, right, BoolOp::Cut).unwrap(), vol(left), vol(left)));
    }
}

/// The saved scale-invariance failure: these boxes meet at y=30, z=20 along an
/// edge, so their union stays two lumps at every scale. Intersection and cut still scale.
#[test]
fn edge_only_box_contact_is_scale_invariant() {
    let a = Prim::Box { at: [0.0, 15.0, 7.5], size: [7.5, 15.0, 12.5] };
    let b = Prim::Box { at: [0.0, 30.0, 20.0], size: [5.0, 5.0, 5.0] };
    assert!(edge_only_boxes(&a, &b));
    for k in [0.05, 1.0, 40.0] {
        check(scale(&a, &b, k));
        check(scale(&b, &a, k));
    }
}

fn check(r: Result<(), TestCaseError>) {
    if let Err(e) = r {
        panic!("{e}");
    }
}

/// #61: the point-tangency refusal is an expected safe refusal, not an ignored bug case. The
/// property tests above execute it; these plain tests pin the exact accepted case and the cases
/// that must still fail.
fn bad_vertex_gap(offset: f64) -> String {
    format!(
        "SMLib boolean: SMLib's check refuses the result: 1 failed tests:\n{BAD_VERTEX_GAP} (offset={offset}, tolerance= 2e-05) [owner SmVertex]\n"
    )
}

/// #61: the exact Box/Sphere face point-tangency case is executed and recorded as an expected
/// safe refusal, never as a successful union.
#[test]
fn box_sphere_face_point_tangency_is_a_recorded_refusal() {
    let a = Prim::Box { at: [17.5, 0.0, 0.0], size: [5.0, 25.0, 17.5] };
    let b = Prim::Sph { at: [15.0, 2.5, 2.5], r: 2.5 };
    assert!(box_sphere_point_tangency(&a, &b).is_some(), "the input is exact face point tangency");
    let (ba, bb) = (a.body(), b.body());
    let before = [operand_state(&ba), operand_state(&bb)];
    let error = match boolean(&ba, &bb, BoolOp::Union) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("the point-tangency union is expected to be refused"),
    };
    let after = [operand_state(&ba), operand_state(&bb)];
    assert_eq!(bad_vertex_gap_offset(&error), Some(2.5), "{error}");
    let proof = point_tangency_refusal(&ba, &bb, &a, &b, &error, &before, &after).expect("the exact point tangency is an accepted refusal");
    assert_eq!(proof.point, [17.5, 2.5, 2.5]);
    assert_eq!(proof.radius, 2.5);
    assert_eq!(proof.offset, 2.5);
    assert!(boolean(&ba, &bb, BoolOp::Union).is_err(), "the accepted refusal is still a refusal, not a union");
}

/// The same exact SMLib text cannot excuse a sphere that overlaps the box (positive volume).
#[test]
fn point_tangency_refusal_rejects_positive_volume_overlap() {
    let a = Prim::Box { at: [0.0, 0.0, 0.0], size: [5.0, 5.0, 5.0] };
    let b = Prim::Sph { at: [4.0, 2.5, 2.5], r: 2.0 };
    assert!(box_sphere_point_tangency(&a, &b).is_none(), "the sphere overlaps the box");
    let (ba, bb) = (a.body(), b.body());
    let before = [operand_state(&ba), operand_state(&bb)];
    let after = [operand_state(&ba), operand_state(&bb)];
    assert!(point_tangency_refusal(&ba, &bb, &a, &b, &bad_vertex_gap(2.0), &before, &after).is_none(), "positive-volume overlap must not be excused");
}

/// A contact on the box edge (the point is not inside a face) is not point tangency of a face.
#[test]
fn point_tangency_refusal_rejects_edge_contact_with_the_same_error() {
    let a = Prim::Box { at: [0.0, 0.0, 0.0], size: [5.0, 5.0, 5.0] };
    let b = Prim::Sph { at: [-2.5, 0.0, 2.5], r: 2.5 };
    assert!(box_sphere_point_tangency(&a, &b).is_none(), "the contact point lies on the box edge y = 0");
    let (ba, bb) = (a.body(), b.body());
    let before = [operand_state(&ba), operand_state(&bb)];
    let after = [operand_state(&ba), operand_state(&bb)];
    assert!(point_tangency_refusal(&ba, &bb, &a, &b, &bad_vertex_gap(2.5), &before, &after).is_none(), "an edge contact must not be excused");
}

/// The old Box/Box edge allowance stays removed: two boxes touching along an edge are not a
/// point tangency, even when the refusal text matches.
#[test]
fn point_tangency_refusal_does_not_restore_box_edge_allowance() {
    let a = Prim::Box { at: [0.0, 0.0, 0.0], size: [5.0, 5.0, 5.0] };
    let b = Prim::Box { at: [5.0, 5.0, 0.0], size: [5.0, 5.0, 5.0] };
    assert!(edge_only_boxes(&a, &b));
    let (ba, bb) = (a.body(), b.body());
    let before = [operand_state(&ba), operand_state(&bb)];
    let after = [operand_state(&ba), operand_state(&bb)];
    assert!(
        point_tangency_refusal(&ba, &bb, &a, &b, &bad_vertex_gap(2.5), &before, &after).is_none(),
        "the old Box/Box edge allowance must not return"
    );
}

/// A different error (or the same geometry with a different offset) is never excused.
#[test]
fn point_tangency_refusal_rejects_other_errors_and_wrong_offsets() {
    let a = Prim::Box { at: [17.5, 0.0, 0.0], size: [5.0, 25.0, 17.5] };
    let b = Prim::Sph { at: [15.0, 2.5, 2.5], r: 2.5 };
    let (ba, bb) = (a.body(), b.body());
    let before = [operand_state(&ba), operand_state(&bb)];
    let after = [operand_state(&ba), operand_state(&bb)];
    let other = "SMLib boolean: SMLib's check refuses the result: 1 failed tests:\nSmBSplineSurface::Test #5 Bad NonMonotonic CPts: SmBSplineSurface Control Net doubles back on itself probably making a self intersecting surface [owner SmSphere]\n";
    assert!(point_tangency_refusal(&ba, &bb, &a, &b, other, &before, &after).is_none(), "a different kernel error must not be excused");
    assert!(
        point_tangency_refusal(&ba, &bb, &a, &b, &bad_vertex_gap(3.0), &before, &after).is_none(),
        "a different Bad Vertex Gap offset must not be excused"
    );
}

/// The refusal must leave both operands unchanged; a changed state is never excused.
#[test]
fn point_tangency_refusal_rejects_changed_operands() {
    let a = Prim::Box { at: [17.5, 0.0, 0.0], size: [5.0, 25.0, 17.5] };
    let b = Prim::Sph { at: [15.0, 2.5, 2.5], r: 2.5 };
    let (ba, bb) = (a.body(), b.body());
    let before = [operand_state(&ba), operand_state(&bb)];
    let mut after = before.clone();
    after[0].faces += 1;
    assert!(point_tangency_refusal(&ba, &bb, &a, &b, &bad_vertex_gap(2.5), &before, &after).is_none(), "a CAD state change must not be excused");
}

/// A box and a cylinder in general position: the intersect is a sliver 0.5 mm thick. It was
/// refused as an implausible volume, because the meshes of the check were faceted at 0.017 mm.
#[test]
fn generic_box_cylinder_intersect() {
    let a = Prim::Box { at: [11.483767761089219, 4.801612107504224, 3.6313476467611787], size: [4.789088988834864, 3.0, 10.419335961063835] };
    let b = Prim::Cyl { at: [29.2480632732879, 8.3683289461485, 6.947922225984966], axis: 2, r: 13.502884497756853, h: 19.719218725477496 };
    check(identity(&a, &b));
}

/// A sphere and a box on the grid: the sphere touches the box's bottom edge, so the union has a
/// face pinched to a point there, which did not mesh (#1648).
#[test]
fn grid_sphere_box_union_is_sound() {
    let a = Prim::Sph { at: [2.5, 15.0, 27.5], r: 12.5 };
    let b = Prim::Box { at: [0.0, 0.0, 17.5], size: [5.0, 7.5, 5.0] };
    check(identity(&a, &b));
}

/// A small sphere on the top of a thin box: a patch of the sphere sticks out of the box's side
/// by less than the tolerance, a face of about 0.35 mm² that had no triangle when meshed at 1 % of the body.
#[test]
fn grid_small_sphere_thin_box_union_is_sound() {
    let a = Prim::Sph { at: [20.0, 10.0, 27.5], r: 3.75 };
    let b = Prim::Box { at: [15.0, 0.0, 22.5], size: [7.5, 20.0, 5.0] };
    check(identity(&a, &b));
}

/// Two crossing cylinders: the union moved 1000 mm away loses about a tenth of its volume.
#[test]
#[ignore = "kernel bug, #1649"]
fn crossing_cylinders_union_moved_far() {
    let a = Prim::Cyl { at: [17.5, 30.0, 7.5], axis: 0, r: 5.0, h: 10.0 }.body();
    let b = Prim::Cyl { at: [17.5, 30.0, 15.0], axis: 1, r: 12.5, h: 5.0 }.body();
    let t = Vec3::new(1000.0, -700.0, 300.0);
    let mv = |x: &Body| transform(x, t, Vec3::ZERO, Vec3::Z, 0.0).expect("translate");
    let s = vol(&a) + vol(&b);
    let here = run(&a, &b, BoolOp::Union).unwrap();
    let there = run(&mv(&a), &mv(&b), BoolOp::Union).unwrap();
    assert!(close(here, there, s), "union at origin {here:.4}, moved by 1000 gives {there:.4}");
}

/// The box takes half the cylinder's disc across 10 mm of its height. Its overlap is
/// pi * 2.5^2 * 10 / 2, including after a scale that makes the radius 0.125 mm.
#[test]
fn half_cylinder_intersection_scales_with_the_solid() {
    let overlap = std::f64::consts::PI * 2.5f64.powi(2) * 10.0 / 2.0;
    let cylinder_volume = std::f64::consts::PI * 2.5f64.powi(2) * 17.5;
    let box_volume = 12.5 * 5.0 * 10.0;
    for k in [1.0, 0.05] {
        let a = Prim::Cyl { at: [12.5, 5.0, 5.0].map(|x| x * k), axis: 2, r: 2.5 * k, h: 17.5 * k }.body();
        let b = Prim::Box { at: [10.0, 5.0, 7.5].map(|x| x * k), size: [12.5, 5.0, 10.0].map(|x| x * k) }.body();
        for (op, expected) in
            [(BoolOp::Union, cylinder_volume + box_volume - overlap), (BoolOp::Intersect, overlap), (BoolOp::Cut, cylinder_volume - overlap)]
        {
            let result = boolean(&a, &b, op)
                .unwrap_or_else(|e| panic!("{op:?} at scale {k}: {e}"))
                .unwrap_or_else(|| panic!("{op:?} at scale {k} is not empty"));
            assert!(result.validity().is_empty(), "{op:?} at scale {k}: {:?}", result.validity());
            let actual = measure(&result).unwrap().volume / (k * k * k);
            assert!(close(actual, expected, expected), "{op:?} at scale {k}: {actual} != {expected}");
        }
    }
}
