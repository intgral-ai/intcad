//! Reading a solved field at a point (PIL-294 C1, C2, C8): the stress tensor and displacement
//! interpolated inside the solver's own element with that element's shape functions (not taken
//! from the nearest node), the tensor's principal values and directions, and the elastic strain.
//!
//! The elements are the ones the solver wrote to its results file: 10-node tetrahedra for solids,
//! and for shells the 15-node wedges it expands each 6-node shell triangle into, through the
//! sheet's thickness, so a point on the sheet's top or bottom face reads that face's stress. The
//! results file lists an element's corners first; its mid-side nodes are matched to the edges by
//! where they lie (each sits at its edge's middle), so nothing here depends on the file's
//! mid-side order.

use super::formats::P;
use super::model;

/// The tetrahedron's edges, in the order [`Cell::Tet`] keeps its mid-side nodes.
pub const TET_EDGES: [(usize, usize); 6] = [(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)];
/// The wedge's edges, in the order [`Cell::Wedge`] keeps its mid-side nodes: the first triangle's
/// three, the second's three, then the three joining them (corner `i` to corner `i + 3`).
pub const WEDGE_EDGES: [(usize, usize); 9] = [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3), (0, 3), (1, 4), (2, 5)];

/// A point this far outside the part (mm) is read at the nearest point of the part; farther, it
/// is not read. The same reach as a point naming a face.
pub const REACH: f64 = 1.0;

/// One element of the solver's results: indices into the run's nodes, corners first, then one
/// mid-side node per edge in [`TET_EDGES`] or [`WEDGE_EDGES`] order.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    /// A 10-node tetrahedron (a solid's element).
    Tet([usize; 10]),
    /// A 15-node wedge: a shell triangle expanded through the sheet's thickness, corners 0–2 on
    /// one face of the sheet and 3–5 on the other.
    Wedge([usize; 15]),
}

/// The results file's element type numbers this reads: a 10-node tetrahedron and a 15-node wedge.
pub const FRD_TET10: u32 = 6;
pub const FRD_WEDGE15: u32 = 5;

/// Orders an element's nodes as [`Cell`] keeps them: `nodes` are indices into the run's nodes,
/// corners first (`corners` of them), then the mid-side nodes in any order; each mid-side node
/// goes to the edge whose middle is nearest it. `None` when two go to one edge or a node has no
/// position: the element is left out.
fn ordered(nodes: &[usize], corners: usize, edges: &[(usize, usize)], pos: &dyn Fn(usize) -> Option<P>) -> Option<Vec<usize>> {
    if nodes.len() != corners + edges.len() {
        return None;
    }
    let corner_pos: Vec<P> = nodes.get(..corners)?.iter().map(|n| pos(*n)).collect::<Option<_>>()?;
    let middles: Vec<P> = edges.iter().map(|(a, b)| Some(model::mid(*corner_pos.get(*a)?, *corner_pos.get(*b)?))).collect::<Option<_>>()?;
    let mut slots: Vec<Option<usize>> = vec![None; edges.len()];
    for n in nodes.get(corners..)? {
        let at = pos(*n)?;
        let (e, _) = middles.iter().enumerate().map(|(e, m)| (e, model::len(model::sub(at, *m)))).min_by(|a, b| a.1.total_cmp(&b.1))?;
        let slot = slots.get_mut(e)?;
        if slot.is_some() {
            return None;
        }
        *slot = Some(*n);
    }
    let mut out: Vec<usize> = nodes.get(..corners)?.to_vec();
    for s in slots {
        out.push(s?);
    }
    Some(out)
}

/// The cell of a results-file element of type `kind` with `nodes` (indices into the run's nodes,
/// in the file's order); `None` for another type or an element whose nodes do not fit it.
pub fn cell(kind: u32, nodes: &[usize], pos: &dyn Fn(usize) -> Option<P>) -> Option<Cell> {
    match kind {
        FRD_TET10 => ordered(nodes, 4, &TET_EDGES, pos)?.try_into().ok().map(Cell::Tet),
        FRD_WEDGE15 => ordered(nodes, 6, &WEDGE_EDGES, pos)?.try_into().ok().map(Cell::Wedge),
        _ => None,
    }
}

/// The 10-node tetrahedron's shape functions at volume coordinates `l` (summing to 1).
pub fn tet_shape(l: [f64; 4]) -> [f64; 10] {
    let mut n = [0.0; 10];
    for i in 0..4 {
        n[i] = l[i] * (2.0 * l[i] - 1.0);
    }
    for (k, (a, b)) in TET_EDGES.iter().enumerate() {
        n[4 + k] = 4.0 * l[*a] * l[*b];
    }
    n
}

/// The 15-node wedge's shape functions at triangle coordinates `l` (summing to 1) and `z` through
/// it (−1 on corners 0–2's face, +1 on corners 3–5's).
pub fn wedge_shape(l: [f64; 3], z: f64) -> [f64; 15] {
    let mut n = [0.0; 15];
    let (lo, hi, bulge) = (1.0 - z, 1.0 + z, 1.0 - z * z);
    for i in 0..3 {
        n[i] = 0.5 * l[i] * (2.0 * l[i] - 1.0) * lo - 0.5 * l[i] * bulge;
        n[3 + i] = 0.5 * l[i] * (2.0 * l[i] - 1.0) * hi - 0.5 * l[i] * bulge;
    }
    for (k, (a, b)) in WEDGE_EDGES.iter().enumerate() {
        n[6 + k] = match k {
            0..=2 => 2.0 * l[*a] * l[*b] * lo,
            3..=5 => 2.0 * l[*a - 3] * l[*b - 3] * hi,
            _ => l[*a] * bulge,
        };
    }
    n
}

/// `x` solved from the 3×3 system with columns `c` (Cramer's rule); `None` when it is singular.
fn solve3(c: [P; 3], rhs: P) -> Option<P> {
    let det = model::dot(c[0], model::cross(c[1], c[2]));
    if !(det.abs() > 1e-300) || !det.is_finite() {
        return None;
    }
    Some([
        model::dot(rhs, model::cross(c[1], c[2])) / det,
        model::dot(c[0], model::cross(rhs, c[2])) / det,
        model::dot(c[0], model::cross(c[1], rhs)) / det,
    ])
}

/// Coordinates summing to 1, with any below zero raised to zero: the nearest corner of the
/// simplex for a point just outside it.
fn clamp_simplex<const N: usize>(l: [f64; N]) -> [f64; N] {
    let mut c = l.map(|x| x.max(0.0));
    let sum: f64 = c.iter().sum();
    if sum > 0.0 {
        c = c.map(|x| x / sum);
    } else {
        c = [1.0 / N as f64; N];
    }
    c
}

/// The point of triangle `a b c` nearest `p` (by the triangle's Voronoi regions: a corner, an
/// edge or the inside).
fn nearest_on_triangle(p: P, a: P, b: P, c: P) -> P {
    use model::{add, dot, mul, sub};
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return add(a, mul(ab, d1 / (d1 - d3)));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return add(a, mul(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return add(b, mul(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))));
    }
    let denom = va + vb + vc;
    if !(denom.abs() > 0.0) || !denom.is_finite() {
        return a;
    }
    add(a, add(mul(ab, vb / denom), mul(ac, vc / denom)))
}

/// The triangles bounding a straight-sided cell, by corner: a tetrahedron's four faces; a wedge's
/// two triangles and its three sides, each split in two.
const TET_FACES: [[usize; 3]; 4] = [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]];
const WEDGE_FACES: [[usize; 3]; 8] = [[0, 1, 2], [3, 4, 5], [0, 1, 4], [0, 4, 3], [1, 2, 5], [1, 5, 4], [2, 0, 3], [2, 3, 5]];

/// A point's coordinates in a cell: volume coordinates in a tetrahedron; triangle coordinates and
/// `z` (−1 to 1) in a wedge.
enum Natural {
    Tet([f64; 4]),
    Wedge([f64; 3], f64),
}

impl Natural {
    /// Inside the cell, give or take round-off.
    fn inside(&self) -> bool {
        const EPS: f64 = 1e-9;
        match self {
            Natural::Tet(l) => l.iter().all(|x| *x >= -EPS),
            Natural::Wedge(l, z) => l.iter().all(|x| *x >= -EPS) && z.abs() <= 1.0 + EPS,
        }
    }
    /// The shape functions' weights here, the coordinates pulled into the cell first.
    fn weights(&self) -> Vec<f64> {
        match self {
            Natural::Tet(l) => tet_shape(clamp_simplex(*l)).to_vec(),
            Natural::Wedge(l, z) => wedge_shape(clamp_simplex(*l), z.clamp(-1.0, 1.0)).to_vec(),
        }
    }
}

/// `p`'s coordinates in the straight-sided cell with these corners (four for a tetrahedron, six
/// for a wedge): exact for a tetrahedron, by Newton's method for a wedge.
fn natural(corners: &[P], p: P) -> Option<Natural> {
    match corners {
        [x0, x1, x2, x3] => {
            let b = solve3([model::sub(*x1, *x0), model::sub(*x2, *x0), model::sub(*x3, *x0)], model::sub(p, *x0))?;
            Some(Natural::Tet([1.0 - b[0] - b[1] - b[2], b[0], b[1], b[2]]))
        }
        [_, _, _, _, _, _] => {
            let x = corners;
            let side = |i: usize, z: f64| model::add(model::mul(x[i], 0.5 * (1.0 - z)), model::mul(x[i + 3], 0.5 * (1.0 + z)));
            let (mut a, mut b, mut z) = (1.0 / 3.0, 1.0 / 3.0, 0.0);
            for _ in 0..30 {
                let l = [1.0 - a - b, a, b];
                let at = (0..3).fold([0.0; 3], |acc, i| model::add(acc, model::mul(side(i, z), l[i])));
                let da = model::sub(side(1, z), side(0, z));
                let db = model::sub(side(2, z), side(0, z));
                let dz = (0..3).fold([0.0; 3], |acc, i| model::add(acc, model::mul(model::sub(x[i + 3], x[i]), 0.5 * l[i])));
                let step = solve3([da, db, dz], model::sub(p, at))?;
                a += step[0];
                b += step[1];
                z += step[2];
                if !(a.is_finite() && b.is_finite() && z.is_finite()) {
                    return None;
                }
                if model::len(step) < 1e-13 {
                    break;
                }
            }
            Some(Natural::Wedge([1.0 - a - b, a, b], z))
        }
        _ => None,
    }
}

/// Where `p` falls in a cell: its shape-function weights there, and how far `p` is from the cell
/// (0 inside), mm. The cell's corners place it (its straight-sided shape); a point outside is read
/// at the cell's nearest point.
fn locate(cell: &Cell, p: P, pos: &dyn Fn(usize) -> Option<P>) -> Option<(Vec<f64>, f64)> {
    let (corners, faces): (Vec<P>, &[[usize; 3]]) = match cell {
        Cell::Tet(n) => (n[..4].iter().map(|i| pos(*i)).collect::<Option<_>>()?, &TET_FACES),
        Cell::Wedge(n) => (n[..6].iter().map(|i| pos(*i)).collect::<Option<_>>()?, &WEDGE_FACES),
    };
    let here = natural(&corners, p)?;
    if here.inside() {
        return Some((here.weights(), 0.0));
    }
    let q = faces
        .iter()
        .filter_map(|[a, b, c]| Some(nearest_on_triangle(p, *corners.get(*a)?, *corners.get(*b)?, *corners.get(*c)?)))
        .min_by(|a, b| model::len(model::sub(p, *a)).total_cmp(&model::len(model::sub(p, *b))))?;
    Some((natural(&corners, q)?.weights(), model::len(model::sub(p, q))))
}

/// The solved field read at a point.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Stress, MPa: xx, yy, zz, xy, yz, zx.
    pub stress: [f64; 6],
    /// Displacement, mm.
    pub disp: P,
    /// How far the point was outside the part, mm (0 inside): it was read at the part's nearest
    /// point.
    pub outside: f64,
}

/// Why a point was not read.
#[derive(Clone, Debug, PartialEq)]
pub enum Miss {
    /// The nearest element is this far away, mm (more than [`REACH`]).
    Outside(f64),
    /// No element could be read (none of the types this reads, or the run has no elements).
    NoElements,
}

/// The run's stress and displacement at `p`: inside the element that holds it, or, for a point up
/// to [`REACH`] outside the part, at the nearest element's nearest point. `points` are the run's
/// nodes (position, von Mises, displacement) and `tensors` their stress tensors, in the same order.
pub fn sample(cells: &[Cell], points: &[(P, f64, P)], tensors: &[[f64; 6]], p: P) -> Result<Sample, Miss> {
    let pos = |i: usize| points.get(i).map(|x| x.0);
    let mut best: Option<(f64, &Cell, Vec<f64>)> = None;
    for c in cells {
        let corners: &[usize] = match c {
            Cell::Tet(n) => &n[..4],
            Cell::Wedge(n) => &n[..6],
        };
        // Skip cells whose box, widened by the reach (or the best distance so far), misses `p`.
        let reach = best.as_ref().map_or(REACH, |b| b.0.min(REACH));
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for i in corners {
            let Some(q) = pos(*i) else { continue };
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        if (0..3).any(|k| p[k] < lo[k] - reach || p[k] > hi[k] + reach) {
            continue;
        }
        let Some((weights, d)) = locate(c, p, &pos) else { continue };
        if best.as_ref().is_none_or(|b| d < b.0) {
            best = Some((d, c, weights));
            if d == 0.0 {
                break;
            }
        }
    }
    let Some((d, c, w)) = best else {
        return Err(nearest_cell_distance(cells, &pos, p).map_or(Miss::NoElements, Miss::Outside));
    };
    if d > REACH {
        return Err(Miss::Outside(d));
    }
    let nodes: &[usize] = match c {
        Cell::Tet(n) => n,
        Cell::Wedge(n) => n,
    };
    let mut stress = [0.0; 6];
    let mut disp = [0.0; 3];
    for (n, wt) in nodes.iter().zip(w) {
        let (Some(s), Some(u)) = (tensors.get(*n), points.get(*n)) else { return Err(Miss::NoElements) };
        for k in 0..6 {
            stress[k] += wt * s[k];
        }
        disp = model::add(disp, model::mul(u.2, wt));
    }
    Ok(Sample { stress, disp, outside: d })
}

/// How far `p` is from the nearest cell's nearest corner, mm, for a point no cell is within reach
/// of: what the refusal says.
fn nearest_cell_distance(cells: &[Cell], pos: &dyn Fn(usize) -> Option<P>, p: P) -> Option<f64> {
    cells
        .iter()
        .flat_map(|c| match c {
            Cell::Tet(n) => n[..4].to_vec(),
            Cell::Wedge(n) => n[..6].to_vec(),
        })
        .filter_map(pos)
        .map(|q| model::len(model::sub(p, q)))
        .min_by(f64::total_cmp)
}

/// A tensor (xx, yy, zz, xy, yz, zx) as a symmetric 3×3 matrix.
fn matrix(s: &[f64; 6]) -> [[f64; 3]; 3] {
    let [xx, yy, zz, xy, yz, zx] = *s;
    [[xx, xy, zx], [xy, yy, yz], [zx, yz, zz]]
}

/// A tensor's principal values, largest first, and their directions (unit vectors, each pointing
/// to the side where its largest component is positive, so the same tensor gives the same
/// directions).
pub fn principal(s: &[f64; 6]) -> ([f64; 3], [P; 3]) {
    let (values, vectors) = model::sym_eigen(matrix(s));
    let mut pairs: Vec<(f64, P)> = (0..3)
        .map(|k| {
            let v: P = [vectors[0][k], vectors[1][k], vectors[2][k]];
            let big = v.iter().copied().fold(0.0_f64, |a, x| if x.abs() > a.abs() { x } else { a });
            (values[k], if big < 0.0 { model::mul(v, -1.0) } else { v })
        })
        .collect();
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let get = |k: usize| pairs.get(k).copied().unwrap_or((0.0, [0.0; 3]));
    ([get(0).0, get(1).0, get(2).0], [get(0).1, get(1).1, get(2).1])
}

/// The elastic strain of a stress (xx, yy, zz, xy, yz, zx, MPa) in a linear isotropic material
/// of Young's modulus `e` (MPa) and Poisson's ratio `nu` (Hooke's law): tensor components, so the
/// shears are half the engineering shear strains (γ = 2ε). For such a material it is the strain
/// the solver computes, the same linear map of the same field.
pub fn strain(s: &[f64; 6], e: f64, nu: f64) -> [f64; 6] {
    let trace = s[0] + s[1] + s[2];
    let normal = |x: f64| ((1.0 + nu) * x - nu * trace) / e;
    let shear = |x: f64| (1.0 + nu) * x / e;
    [normal(s[0]), normal(s[1]), normal(s[2]), shear(s[3]), shear(s[4]), shear(s[5])]
}

/// A tensor's normal component along the unit vector `n`: nᵀ T n (a strain gauge's reading, for a
/// strain).
pub fn along(s: &[f64; 6], n: P) -> f64 {
    let m = matrix(s);
    (0..3).map(|i| (0..3).map(|j| n[i] * m[i][j] * n[j]).sum::<f64>()).sum()
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;
