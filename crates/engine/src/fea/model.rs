//! From gmsh's mesh to a CalculiX job: which mesh surface is which face of the body, the sheet's
//! mid-surface for shells, the elements in the solver's node order, and the loads and supports
//! as nodal forces and fixed nodes. All of it is analysis-mesh work: nothing here decides the
//! model, it only places the estimate's loads where the faces the user named lie.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::formats::{Msh, P};

pub fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn mul(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: P, b: P) -> P {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn len(a: P) -> f64 {
    dot(a, a).sqrt()
}
pub fn unit(a: P) -> Option<P> {
    let l = len(a);
    (l > 1e-300 && l.is_finite()).then(|| mul(a, 1.0 / l))
}
pub fn mid(a: P, b: P) -> P {
    mul(add(a, b), 0.5)
}

/// The distance from `p` to triangle `abc`.
pub fn point_triangle(p: P, a: P, b: P, c: P) -> f64 {
    let seg = |p: P, a: P, b: P| {
        let ab = sub(b, a);
        let l2 = dot(ab, ab);
        let t = if l2 > 0.0 { (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
        len(sub(p, add(a, mul(ab, t))))
    };
    let n = cross(sub(b, a), sub(c, a));
    let Some(nn) = unit(n) else { return seg(p, a, b).min(seg(p, b, c)).min(seg(p, c, a)) };
    let h = dot(sub(p, a), nn);
    let q = sub(p, mul(nn, h));
    let inside = [(a, b), (b, c), (c, a)].iter().all(|(u, v)| dot(cross(sub(*v, *u), sub(q, *u)), n) >= -1e-12);
    if inside { h.abs() } else { seg(p, a, b).min(seg(p, b, c)).min(seg(p, c, a)) }
}

/// Where a ray from `o` along `d` (unit) meets triangle `abc`: its distance, if it does.
fn ray_triangle(o: P, d: P, a: P, b: P, c: P) -> Option<f64> {
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let h = cross(d, e2);
    let det = dot(e1, h);
    if det.abs() < 1e-14 {
        return None;
    }
    let f = 1.0 / det;
    let s = sub(o, a);
    let u = f * dot(s, h);
    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = f * dot(d, q);
    if v < -1e-9 || u + v > 1.0 + 1e-9 {
        return None;
    }
    let t = f * dot(e2, q);
    (t > 0.0).then_some(t)
}

/// A uniform grid over points, for the nearest point to a query.
pub struct Grid {
    cell: f64,
    min: P,
    cells: HashMap<(i64, i64, i64), Vec<usize>>,
    pts: Vec<P>,
}

impl Grid {
    pub fn new(pts: Vec<P>, cell: f64) -> Grid {
        let cell = if cell.is_finite() && cell > 0.0 { cell } else { 1.0 };
        let mut min = [f64::INFINITY; 3];
        for p in &pts {
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
            }
        }
        if !min.iter().all(|x| x.is_finite()) {
            min = [0.0; 3];
        }
        let mut g = Grid { cell, min, cells: HashMap::new(), pts: Vec::new() };
        for (i, p) in pts.iter().enumerate() {
            g.cells.entry(g.key(*p)).or_default().push(i);
        }
        g.pts = pts;
        g
    }
    fn key(&self, p: P) -> (i64, i64, i64) {
        let k = |i: usize| ((p[i] - self.min[i]) / self.cell).floor().clamp(-1e12, 1e12) as i64;
        (k(0), k(1), k(2))
    }
    /// The nearest point's index, searching rings of cells outward up to `max_rings`.
    pub fn nearest(&self, p: P, max_rings: i64) -> Option<usize> {
        let (cx, cy, cz) = self.key(p);
        let mut best: Option<(f64, usize)> = None;
        for r in 0..=max_rings {
            for x in cx - r..=cx + r {
                for y in cy - r..=cy + r {
                    for z in cz - r..=cz + r {
                        if (x - cx).abs() != r && (y - cy).abs() != r && (z - cz).abs() != r {
                            continue;
                        }
                        for &i in self.cells.get(&(x, y, z)).into_iter().flatten() {
                            let d = self.pts.get(i).map_or(f64::INFINITY, |q| len(sub(*q, p)));
                            if best.is_none_or(|(bd, _)| d < bd) {
                                best = Some((d, i));
                            }
                        }
                    }
                }
            }
            // A point found within this ring's reach can't be beaten further out.
            if let Some((d, _)) = best
                && d <= r as f64 * self.cell
            {
                break;
            }
        }
        best.map(|(_, i)| i)
    }
}

/// Triangles bucketed in a uniform grid of cells, for the few near a point: what keeps matching a
/// large shell model to its faces from comparing every node with every triangle.
pub struct TriGrid<'a> {
    tris: &'a [[P; 3]],
    cell: f64,
    min: P,
    cells: HashMap<(i64, i64, i64), Vec<usize>>,
}

fn bounds(t: &[P; 3]) -> (P, P) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in t {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

impl<'a> TriGrid<'a> {
    /// Cells at least `reach` across (the distance queries will ask about) and at least a
    /// triangle's mean size, so a query or a triangle spans few cells.
    pub fn new(tris: &'a [[P; 3]], reach: f64) -> TriGrid<'a> {
        let mut min = [f64::INFINITY; 3];
        let mut extent = 0.0;
        for t in tris {
            let (lo, hi) = bounds(t);
            for k in 0..3 {
                min[k] = min[k].min(lo[k]);
            }
            let e = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
            if e.is_finite() {
                extent += e;
            }
        }
        if !min.iter().all(|x| x.is_finite()) {
            min = [0.0; 3];
        }
        let mean = if tris.is_empty() { 0.0 } else { extent / tris.len() as f64 };
        let cell = mean.max(reach);
        let cell = if cell.is_finite() && cell > 0.0 { cell } else { 1.0 };
        let mut g = TriGrid { tris, cell, min, cells: HashMap::new() };
        for (i, t) in tris.iter().enumerate() {
            let (lo, hi) = bounds(t);
            let (a, b) = (g.key(lo), g.key(hi));
            for x in a.0..=b.0 {
                for y in a.1..=b.1 {
                    for z in a.2..=b.2 {
                        g.cells.entry((x, y, z)).or_default().push(i);
                    }
                }
            }
        }
        g
    }
    fn key(&self, p: P) -> (i64, i64, i64) {
        let k = |i: usize| ((p[i] - self.min[i]) / self.cell).floor().clamp(-1e12, 1e12) as i64;
        (k(0), k(1), k(2))
    }
    /// The triangles whose bounding boxes may meet the box `p` ± `reach`: every triangle with a
    /// point within `reach` of `p`. Ascending indices, each once (the order of a scan of all).
    pub fn near(&self, p: P, reach: f64) -> Vec<usize> {
        let r = [reach; 3];
        let (a, b) = (self.key(sub(p, r)), self.key(add(p, r)));
        let mut out = Vec::new();
        for x in a.0..=b.0 {
            for y in a.1..=b.1 {
                for z in a.2..=b.2 {
                    out.extend(self.cells.get(&(x, y, z)).into_iter().flatten().copied());
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
    fn gap(&self, p: P, i: usize) -> f64 {
        self.tris.get(i).map_or(f64::INFINITY, |[a, b, c]| point_triangle(p, *a, *b, *c))
    }
    /// Whether a triangle lies within `reach` of `p`.
    pub fn any_within(&self, p: P, reach: f64) -> bool {
        self.near(p, reach).into_iter().any(|i| self.gap(p, i) <= reach)
    }
    /// The distance from `p` to the nearest triangle (infinite with none): the triangles within
    /// `reach` when one is that close, else all of them, so it is exactly a scan's answer.
    pub fn distance(&self, p: P, reach: f64) -> f64 {
        let near = self.near(p, reach).into_iter().map(|i| self.gap(p, i)).fold(f64::INFINITY, f64::min);
        if near <= reach { near } else { (0..self.tris.len()).map(|i| self.gap(p, i)).fold(f64::INFINITY, f64::min) }
    }
}

/// A body's display triangles with their face indices, for telling which face a point lies on.
pub struct Faces<'a> {
    pub positions: &'a [P],
    pub triangles: &'a [[u32; 3]],
    pub tri_face: &'a [u32],
}

impl Faces<'_> {
    fn tri(&self, t: usize) -> Option<(P, P, P, u32)> {
        let [a, b, c] = *self.triangles.get(t)?;
        Some((*self.positions.get(a as usize)?, *self.positions.get(b as usize)?, *self.positions.get(c as usize)?, *self.tri_face.get(t)?))
    }
    /// The face nearest `p`, and how far it is.
    pub fn nearest_face(&self, p: P) -> Option<(u32, f64)> {
        let mut best: Option<(u32, f64)> = None;
        for t in 0..self.triangles.len() {
            let Some((a, b, c, f)) = self.tri(t) else { continue };
            let d = point_triangle(p, a, b, c);
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((f, d));
            }
        }
        best
    }
}

/// Which face of the body each mesh surface lies on: a vote of points on its triangles.
pub fn surface_faces(msh: &Msh, faces: &Faces) -> BTreeMap<i64, u32> {
    let mut by_surface: BTreeMap<i64, Vec<&[u64; 6]>> = BTreeMap::new();
    for (s, t) in &msh.tri6 {
        by_surface.entry(*s).or_default().push(t);
    }
    let mut out = BTreeMap::new();
    for (s, tris) in by_surface {
        let step = (tris.len() / 24).max(1);
        let mut votes: BTreeMap<u32, usize> = BTreeMap::new();
        for t in tris.iter().step_by(step) {
            let corners: Vec<P> = t.iter().take(3).filter_map(|n| msh.nodes.get(n).copied()).collect();
            let [a, b, c] = corners[..] else { continue };
            if let Some((f, _)) = faces.nearest_face(mul(add(add(a, b), c), 1.0 / 3.0)) {
                *votes.entry(f).or_default() += 1;
            }
        }
        if let Some((f, _)) = votes.into_iter().max_by_key(|(_, n)| *n) {
            out.insert(s, f);
        }
    }
    out
}

/// The three corner pairs of a triangle's edges, in the solver's order (1-2, 2-3, 3-1), or the
/// six of a tetrahedron's (1-2, 2-3, 3-1, 1-4, 2-4, 3-4).
const TRI_EDGES: [(usize, usize); 3] = [(0, 1), (1, 2), (2, 0)];
const TET_EDGES: [(usize, usize); 6] = [(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)];

/// The mid-edge nodes of an element whose corners are `corners`, matched to its edges by where
/// they lie (whatever order the mesher listed them in): one per edge, in `edges` order.
fn mids_by_edge(corners: &[P], mids: &[(u64, P)], edges: &[(usize, usize)]) -> Option<Vec<u64>> {
    let mut out = Vec::with_capacity(edges.len());
    let mut used = vec![false; mids.len()];
    for (a, b) in edges {
        let m = mid(*corners.get(*a)?, *corners.get(*b)?);
        let mut best: Option<(f64, usize)> = None;
        for (k, (_, p)) in mids.iter().enumerate() {
            if used.get(k).copied().unwrap_or(true) {
                continue;
            }
            let d = len(sub(*p, m));
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, k));
            }
        }
        let (_, k) = best?;
        if let Some(u) = used.get_mut(k) {
            *u = true;
        }
        out.push(mids.get(k)?.0);
    }
    Some(out)
}

/// A six-node triangle turned over: corners 1, 3, 2 and their mid-edge nodes in the solver's
/// order (3-1, 2-3, 1-2 of the old numbering).
fn flip(e: [usize; 6]) -> [usize; 6] {
    [e[0], e[2], e[1], e[5], e[4], e[3]]
}

/// A finite-element model ready for the job file: nodes, elements, and per surface tag the
/// nodes and the element faces (six node indices) that lie on it, each turned so its normal
/// (corners, right hand) points out of the part through that face.
pub struct Model {
    pub nodes: Vec<P>,
    pub shells: Vec<[usize; 6]>,
    pub tets: Vec<[usize; 10]>,
    /// For loads and supports on faces: the model's nodes near each face, and the six-node
    /// triangles (element faces) on it.
    pub on_face: BTreeMap<u32, (BTreeSet<usize>, Vec<[usize; 6]>)>,
}

/// The ten-node tetrahedra of the mesh as a solid model, corners turned so each has a positive
/// volume, and each face's surface triangles.
pub fn solid_model(msh: &Msh, surface_face: &BTreeMap<i64, u32>) -> Result<Model, String> {
    if msh.tet10.is_empty() {
        return Err("the mesher made no tetrahedra (is the body closed?)".into());
    }
    let mut index: HashMap<u64, usize> = HashMap::new();
    let mut nodes: Vec<P> = Vec::new();
    let mut id = |n: u64, nodes: &mut Vec<P>| -> Option<usize> {
        if let Some(i) = index.get(&n) {
            return Some(*i);
        }
        let p = *msh.nodes.get(&n)?;
        nodes.push(p);
        index.insert(n, nodes.len() - 1);
        Some(nodes.len() - 1)
    };
    let mut tets = Vec::with_capacity(msh.tet10.len());
    for t in &msh.tet10 {
        let mut c: Vec<u64> = t.iter().take(4).copied().collect();
        let pts: Option<Vec<P>> = c.iter().map(|n| msh.nodes.get(n).copied()).collect();
        let Some(mut pts) = pts else { continue };
        let [a, b, cc, d] = pts[..] else { continue };
        if dot(cross(sub(b, a), sub(cc, a)), sub(d, a)) < 0.0 {
            c.swap(1, 2);
            pts.swap(1, 2);
        }
        let mids: Vec<(u64, P)> = t.iter().skip(4).filter_map(|n| Some((*n, *msh.nodes.get(n)?))).collect();
        let Some(m) = mids_by_edge(&pts, &mids, &TET_EDGES) else { continue };
        let all: Vec<u64> = c.into_iter().chain(m).collect();
        let ix: Option<Vec<usize>> = all.iter().map(|n| id(*n, &mut nodes)).collect();
        if let Some(Ok(e)) = ix.map(<[usize; 10]>::try_from) {
            tets.push(e);
        }
    }
    // Each tetrahedron face (sorted corners) and the corner opposite it: which side of a surface
    // triangle the material is on.
    let mut opposite: HashMap<[usize; 3], usize> = HashMap::with_capacity(tets.len() * 4);
    for t in &tets {
        for (k, other) in [(3usize, [0usize, 1, 2]), (2, [0, 1, 3]), (1, [0, 2, 3]), (0, [1, 2, 3])] {
            let mut key = other.map(|i| t[i]);
            key.sort_unstable();
            opposite.insert(key, t[k]);
        }
    }
    let mut on_face: BTreeMap<u32, (BTreeSet<usize>, Vec<[usize; 6]>)> = BTreeMap::new();
    for (s, t) in &msh.tri6 {
        let Some(f) = surface_face.get(s) else { continue };
        let pts: Option<Vec<P>> = t.iter().take(3).map(|n| msh.nodes.get(n).copied()).collect();
        let Some(pts) = pts else { continue };
        let mids: Vec<(u64, P)> = t.iter().skip(3).filter_map(|n| Some((*n, *msh.nodes.get(n)?))).collect();
        let Some(m) = mids_by_edge(&pts, &mids, &TRI_EDGES) else { continue };
        let ids: Option<Vec<usize>> = t.iter().take(3).copied().chain(m).map(|n| index.get(&n).copied()).collect();
        let Some(Ok(mut e)) = ids.map(<[usize; 6]>::try_from) else { continue };
        // Turned so its normal (corners, right hand) points out of the part: away from the
        // corner of its tetrahedron that is not on it.
        let mut key = [e[0], e[1], e[2]];
        key.sort_unstable();
        if let (Some(o), Some(a), Some(b), Some(c)) = (opposite.get(&key), nodes.get(e[0]), nodes.get(e[1]), nodes.get(e[2]))
            && let Some(op) = nodes.get(*o)
            && dot(cross(sub(*b, *a), sub(*c, *a)), sub(*op, *a)) > 0.0
        {
            e = flip(e);
        }
        let entry = on_face.entry(*f).or_default();
        entry.0.extend(e.iter().copied());
        entry.1.push(e);
    }
    Ok(Model { nodes, shells: Vec::new(), tets, on_face })
}

/// The sheet's mid-surface as six-node shells: one side of the sheet (the faces that have the
/// other side `t` behind them, grown from the largest across the bends), its nodes moved half
/// the thickness inward. Each face's nodes are the shell nodes within 0.6 `t` of it: a side
/// face's whole panel, an end or a hole wall's rim.
pub fn shell_model(msh: &Msh, surface_face: &BTreeMap<i64, u32>, t: f64) -> Result<Model, String> {
    // Each surface's triangles: corners, and the six ids in solver order.
    struct Tri {
        corners: [P; 3],
        ids: [u64; 6],
    }
    let mut surfaces: BTreeMap<i64, Vec<Tri>> = BTreeMap::new();
    for (s, tri) in &msh.tri6 {
        let pts: Option<Vec<P>> = tri.iter().take(3).map(|n| msh.nodes.get(n).copied()).collect();
        let Some(pts) = pts else { continue };
        let [a, b, c] = pts[..] else { continue };
        let mids: Vec<(u64, P)> = tri.iter().skip(3).filter_map(|n| Some((*n, *msh.nodes.get(n)?))).collect();
        let Some(m) = mids_by_edge(&pts, &mids, &TRI_EDGES) else { continue };
        let ids: Vec<u64> = tri.iter().take(3).copied().chain(m).collect();
        let Ok(ids) = <[u64; 6]>::try_from(ids) else { continue };
        surfaces.entry(*s).or_default().push(Tri { corners: [a, b, c], ids });
    }
    let all: Vec<(i64, &Tri)> = surfaces.iter().flat_map(|(s, v)| v.iter().map(move |t| (*s, t))).collect();
    // A hit counts only up to 1.4 t away, so only triangles that near the ray's start matter: the
    // nearest of them is the nearest of all whenever it is that close (a margin past 1.4 t for
    // the hit test's own tolerance).
    let all_corners: Vec<[P; 3]> = all.iter().map(|(_, x)| x.corners).collect();
    let ray_reach = 1.5 * t;
    let ray_grid = TriGrid::new(&all_corners, ray_reach);
    // A surface is a side of the sheet when the other side lies about `t` behind most of it;
    // `sign` says which way (along the triangles' normal, or against it).
    let mut sides: BTreeMap<i64, (f64, i64)> = BTreeMap::new();
    let mut areas: BTreeMap<i64, f64> = BTreeMap::new();
    for (s, tris) in &surfaces {
        areas.insert(*s, tris.iter().map(|x| 0.5 * len(cross(sub(x.corners[1], x.corners[0]), sub(x.corners[2], x.corners[0])))).sum());
        let step = (tris.len() / 16).max(1);
        let (mut along, mut against, mut tried) = (0usize, 0usize, 0usize);
        let mut partners: BTreeMap<i64, usize> = BTreeMap::new();
        for x in tris.iter().step_by(step) {
            let [a, b, c] = x.corners;
            let Some(n) = unit(cross(sub(b, a), sub(c, a))) else { continue };
            tried += 1;
            let o = mul(add(add(a, b), c), 1.0 / 3.0);
            let candidates = ray_grid.near(o, ray_reach);
            for (sign, count) in [(1.0, &mut along), (-1.0, &mut against)] {
                let d = mul(n, sign);
                let hit = candidates
                    .iter()
                    .filter_map(|i| all.get(*i))
                    .filter(|(other, _)| other != s)
                    .filter_map(|(other, y)| ray_triangle(o, d, y.corners[0], y.corners[1], y.corners[2]).map(|h| (h, *other)))
                    .min_by(|p, q| p.0.total_cmp(&q.0));
                if let Some((h, other)) = hit
                    && (0.6 * t..=1.4 * t).contains(&h)
                {
                    *count += 1;
                    *partners.entry(other).or_default() += 1;
                }
            }
        }
        if tried > 0 && 2 * along.max(against) > tried {
            let sign = if along >= against { 1.0 } else { -1.0 };
            let partner = partners.into_iter().max_by_key(|(_, n)| *n).map_or(-1, |(p, _)| p);
            sides.insert(*s, (sign, partner));
        }
    }
    // One side of the sheet: from the largest side surface, across shared nodes to the others
    // (a bend's cylinder meets its panels there; the two sides never share a node).
    let start = sides.keys().copied().max_by(|a, b| areas.get(a).unwrap_or(&0.0).total_cmp(areas.get(b).unwrap_or(&0.0)));
    let Some(start) = start else {
        return Err(format!("no face of the body has another face {t} mm behind it: it does not look like a sheet {t} mm thick"));
    };
    let mut node_surfaces: HashMap<u64, BTreeSet<i64>> = HashMap::new();
    for (s, tris) in &surfaces {
        if sides.contains_key(s) {
            for x in tris {
                for n in x.ids.iter().take(3) {
                    node_surfaces.entry(*n).or_default().insert(*s);
                }
            }
        }
    }
    let mut side_a: BTreeSet<i64> = BTreeSet::from([start]);
    let mut todo = vec![start];
    while let Some(s) = todo.pop() {
        for x in surfaces.get(&s).into_iter().flatten() {
            for n in x.ids.iter().take(3) {
                for other in node_surfaces.get(n).into_iter().flatten() {
                    if side_a.insert(*other) {
                        todo.push(*other);
                    }
                }
            }
        }
    }
    // Inward normals at the nodes of side A, area weighted.
    let mut normal: HashMap<u64, P> = HashMap::new();
    for s in &side_a {
        let sign = sides.get(s).map_or(1.0, |x| x.0);
        for x in surfaces.get(s).into_iter().flatten() {
            let [a, b, c] = x.corners;
            let n = mul(cross(sub(b, a), sub(c, a)), sign);
            for id in x.ids {
                let e = normal.entry(id).or_insert([0.0; 3]);
                *e = add(*e, n);
            }
        }
    }
    let mut index: HashMap<u64, usize> = HashMap::new();
    let mut nodes: Vec<P> = Vec::new();
    let mut shells = Vec::new();
    for s in &side_a {
        let sign = sides.get(s).map_or(1.0, |x| x.0);
        for x in surfaces.get(s).into_iter().flatten() {
            let mut ix = [0usize; 6];
            let mut ok = true;
            for (k, id) in x.ids.iter().enumerate() {
                let i = match index.get(id) {
                    Some(i) => *i,
                    None => {
                        let (Some(p), Some(n)) = (msh.nodes.get(id), normal.get(id).and_then(|n| unit(*n))) else {
                            ok = false;
                            break;
                        };
                        nodes.push(add(*p, mul(n, 0.5 * t)));
                        index.insert(*id, nodes.len() - 1);
                        nodes.len() - 1
                    }
                };
                if let Some(slot) = ix.get_mut(k) {
                    *slot = i;
                }
            }
            if !ok {
                continue;
            }
            // The shell's normal points inward from side A, the same way everywhere.
            if sign < 0.0 {
                ix = [ix[0], ix[2], ix[1], ix[5], ix[4], ix[3]];
            }
            shells.push(ix);
        }
    }
    if shells.is_empty() {
        return Err("the sheet's mid-surface has no elements".into());
    }
    // Each named face's nodes. A side of the sheet (a panel or a bend, either side): the shell
    // nodes within 0.6 t of its triangles (they lie half a thickness behind it). An edge of the
    // sheet (an end, a hole wall): the shell nodes it shares with the mesh, the mid-surface's
    // edge row and nothing else (a distance would take more rows as the mesh gets finer: at
    // 1 mm, a 0.9 mm band of 604 nodes for one 100 mm edge). Its element faces: shells whose six
    // nodes all are on it.
    let mut on_face: BTreeMap<u32, (BTreeSet<usize>, Vec<[usize; 6]>)> = BTreeMap::new();
    let mut face_tris: BTreeMap<u32, Vec<[P; 3]>> = BTreeMap::new();
    let mut face_side: BTreeMap<u32, bool> = BTreeMap::new();
    let mut face_shared: BTreeMap<u32, BTreeSet<usize>> = BTreeMap::new();
    for (s, tris) in &surfaces {
        if let Some(f) = surface_face.get(s) {
            face_tris.entry(*f).or_default().extend(tris.iter().map(|x| x.corners));
            *face_side.entry(*f).or_default() |= sides.contains_key(s);
            face_shared.entry(*f).or_default().extend(tris.iter().flat_map(|x| x.ids).filter_map(|id| index.get(&id).copied()));
        }
    }
    for (f, tris) in face_tris {
        let edge_row = face_shared.remove(&f).unwrap_or_default();
        if !face_side.get(&f).copied().unwrap_or(false) && !edge_row.is_empty() {
            on_face.insert(f, (edge_row, Vec::new()));
            continue;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for c in tris.iter().flatten() {
            for k in 0..3 {
                lo[k] = lo[k].min(c[k]);
                hi[k] = hi[k].max(c[k]);
            }
        }
        let reach = 0.6 * t;
        let grid = TriGrid::new(&tris, reach);
        let near: BTreeSet<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, p)| (0..3).all(|k| p[k] >= lo[k] - reach && p[k] <= hi[k] + reach))
            .filter(|(_, p)| grid.any_within(**p, reach))
            .map(|(i, _)| i)
            .collect();
        // Each turned so its normal points from the mid-surface to this face (out of the part
        // through it): the side whose point half a thickness off lies on the face.
        let elems: Vec<[usize; 6]> = shells
            .iter()
            .filter(|e| e.iter().all(|n| near.contains(n)))
            .map(|e| {
                let p = |i: usize| nodes.get(e[i]).copied().unwrap_or([0.0; 3]);
                let Some(n) = unit(cross(sub(p(1), p(0)), sub(p(2), p(0)))) else { return *e };
                let c = mul(add(add(p(0), p(1)), p(2)), 1.0 / 3.0);
                // Both points lie within about t of the face (the element's nodes are within
                // 0.6 t of it), so the grid nearly always settles the distance.
                let gap = |q: P| grid.distance(q, 1.5 * t);
                if gap(sub(c, mul(n, 0.5 * t))) < gap(add(c, mul(n, 0.5 * t))) { flip(*e) } else { *e }
            })
            .collect();
        on_face.insert(f, (near, elems));
    }
    Ok(Model { nodes, shells, tets: Vec::new(), on_face })
}

/// Nodal forces for a total `force` spread over `faces`: over the area of the element faces on
/// them (a six-node triangle's consistent share: none at its corners, a third of its area at
/// each mid-edge node); for a face that is only an edge of the shells (an end, a hole's rim),
/// along those element edges (a sixth of the length at each end, two thirds at the middle);
/// otherwise evenly over its nodes. `None` when the faces have no nodes in the model.
pub fn spread(model: &Model, faces: &[u32], force: P) -> Option<Vec<(usize, P)>> {
    let mut nodes: BTreeSet<usize> = BTreeSet::new();
    let mut elems: Vec<[usize; 6]> = Vec::new();
    for f in faces {
        if let Some((n, e)) = model.on_face.get(f) {
            nodes.extend(n.iter().copied());
            elems.extend(e.iter().copied());
        }
    }
    if nodes.is_empty() {
        return None;
    }
    let p = |i: usize| model.nodes.get(i).copied().unwrap_or([0.0; 3]);
    let mut w: BTreeMap<usize, f64> = BTreeMap::new();
    for e in &elems {
        let area = 0.5 * len(cross(sub(p(e[1]), p(e[0])), sub(p(e[2]), p(e[0]))));
        for m in &e[3..] {
            *w.entry(*m).or_default() += area / 3.0;
        }
    }
    if w.is_empty() && !model.shells.is_empty() {
        for e in &model.shells {
            for (k, (a, b)) in TRI_EDGES.iter().enumerate() {
                let (a, b, m) = (e[*a], e[*b], e[3 + k]);
                if nodes.contains(&a) && nodes.contains(&b) && nodes.contains(&m) {
                    let l = len(sub(p(b), p(a)));
                    *w.entry(a).or_default() += l / 6.0;
                    *w.entry(b).or_default() += l / 6.0;
                    *w.entry(m).or_default() += 2.0 * l / 3.0;
                }
            }
        }
        // An edge inside the sheet's mid-surface is shared by two shells: counted twice, which
        // the normalising below takes out evenly.
    }
    if w.is_empty() {
        for n in &nodes {
            w.insert(*n, 1.0);
        }
    }
    let total: f64 = w.values().sum();
    if !(total > 0.0) {
        return None;
    }
    Some(w.into_iter().map(|(n, x)| (n, mul(force, x / total))).collect())
}

/// Nodal forces for a pressure `p` (MPa) on `faces`: each element face on them takes p times its
/// area along its inward normal (a positive pressure pushes into the part), shared as [`spread`]
/// shares a force over an area: none at the corners, a third at each mid-edge node. `None` when
/// the faces have no element faces in the model (a sheet's edge face as shells has no area).
pub fn pressure(model: &Model, faces: &[u32], p: f64) -> Option<Vec<(usize, P)>> {
    if !p.is_finite() {
        return None;
    }
    let at = |i: usize| model.nodes.get(i).copied().unwrap_or([0.0; 3]);
    let mut acc: BTreeMap<usize, P> = BTreeMap::new();
    for f in faces {
        for e in model.on_face.get(f).map(|x| x.1.as_slice()).unwrap_or(&[]) {
            // Twice the area, along the outward normal: a third of p × area at each mid-edge node,
            // pushing inward.
            let n = cross(sub(at(e[1]), at(e[0])), sub(at(e[2]), at(e[0])));
            let share = mul(n, -p / 6.0);
            for m in &e[3..] {
                let x = acc.entry(*m).or_insert([0.0; 3]);
                *x = add(*x, share);
            }
        }
    }
    (!acc.is_empty()).then(|| acc.into_iter().collect())
}

/// The nodes a support on `faces` holds.
pub fn held(model: &Model, faces: &[u32]) -> Vec<usize> {
    let mut out: BTreeSet<usize> = BTreeSet::new();
    for f in faces {
        if let Some((n, _)) = model.on_face.get(f) {
            out.extend(n.iter().copied());
        }
    }
    out.into_iter().collect()
}

/// The nodes a support on `faces` holds over an area of the shells: on the faces that have
/// element faces (a side of the sheet), less those on its edges among `faces` (an end, a hole
/// wall, which a support clamps). Empty for a solid model: only shells have rotations to hold or
/// leave.
pub fn held_area(model: &Model, faces: &[u32]) -> Vec<usize> {
    if model.shells.is_empty() {
        return Vec::new();
    }
    let (mut area, mut edge): (BTreeSet<usize>, BTreeSet<usize>) = (BTreeSet::new(), BTreeSet::new());
    for f in faces {
        if let Some((n, e)) = model.on_face.get(f) {
            if e.is_empty() { &mut edge } else { &mut area }.extend(n.iter().copied());
        }
    }
    area.difference(&edge).copied().collect()
}

/// A direction held at a point: a fixed support holds each of its points along X, Y and Z, a
/// roller along its own axis only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hold {
    pub at: P,
    /// Unit.
    pub along: P,
}

/// How the supports leave the part free to move as a rigid body ([`free_motions`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Freedom {
    /// Directions (unit) the part can slide along.
    pub slides: Vec<P>,
    /// Axes the part can turn about: a unit direction and a point on the axis, mm.
    pub turns: Vec<(P, P)>,
    /// The rigid-body Gram matrix's smallest eigenvalue over its largest (0 when nothing is held).
    pub ratio: f64,
}

impl Freedom {
    /// The supports stop every slide and turn.
    pub fn held(&self) -> bool {
        self.ratio > RIGID_TOLERANCE
    }
}

/// The supports hold the part still when the rigid-body Gram matrix's smallest eigenvalue is
/// above this fraction of its largest. A truly free motion gives round-off: about 1e-16 of the
/// largest (the free cases in the tests give exactly 0). The weakest real hold is a thin face
/// resisting a turn through its own thickness; measured on the real tests' parts
/// (`stress_tests`): R1's 1.5 mm strip clamped at its end face 4.3e-5, R2's 2 mm plate held at
/// one end 2.4e-5, LE1's three rollers 4.6e-3. A 0.3 mm sheet clamped on one edge of a 3 m part
/// would give about 1e-9. 1e-12 sits three orders below that and three above round-off.
pub const RIGID_TOLERANCE: f64 = 1e-12;

/// The eigenvalues and eigenvectors (the columns of the second) of the symmetric matrix `a`, by
/// cyclic Jacobi rotations: each rotation zeroes one off-diagonal entry; sweeps repeat until the
/// off-diagonal part is round-off.
pub fn sym_eigen<const N: usize>(a: [[f64; N]; N]) -> ([f64; N], [[f64; N]; N]) {
    let mut a = a;
    let mut v: [[f64; N]; N] = std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 }));
    let squares = |a: &[[f64; N]; N], off_only: bool| -> f64 {
        a.iter().enumerate().flat_map(|(i, row)| row.iter().enumerate().filter(move |(j, _)| !off_only || *j != i).map(|(_, x)| x * x)).sum()
    };
    if !squares(&a, false).is_finite() {
        return ([0.0; N], v);
    }
    for _ in 0..64 {
        let total = squares(&a, false);
        if squares(&a, true) <= 1e-32 * total || total == 0.0 {
            break;
        }
        for p in 0..N {
            for q in p + 1..N {
                let apq = a[p][q];
                if apq == 0.0 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                // A ← Jᵀ A J and V ← V J, J the rotation in the (p, q) plane.
                for row in a.iter_mut().chain(v.iter_mut()) {
                    let (x, y) = (row[p], row[q]);
                    row[p] = c * x - s * y;
                    row[q] = s * x + c * y;
                }
                let (rp, rq) = (a[p], a[q]);
                a[p] = std::array::from_fn(|k| c * rp[k] - s * rq[k]);
                a[q] = std::array::from_fn(|k| s * rp[k] + c * rq[k]);
            }
        }
    }
    (std::array::from_fn(|i| a[i][i]), v)
}

/// Column `k` of `m`.
fn column<const N: usize>(m: &[[f64; N]; N], k: usize) -> [f64; N] {
    std::array::from_fn(|i| m[i].get(k).copied().unwrap_or(0.0))
}

/// `m` unit directions spanning the same space as `basis` (orthonormal, `m` long), the axes X, Y
/// and Z first where they lie in it, so a free motion reads "along X" when it can.
fn named_directions(basis: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for k in 0..3 {
        let e: P = std::array::from_fn(|i| if i == k { 1.0 } else { 0.0 });
        let inside = basis.iter().fold([0.0; 3], |acc, b| add(acc, mul(*b, dot(e, *b))));
        if len(sub(e, inside)) < 1e-4 && out.len() < basis.len() {
            out.push(e);
        }
    }
    for b in basis {
        if out.len() >= basis.len() {
            break;
        }
        let rest = out.iter().fold(*b, |v, o| sub(v, mul(*o, dot(v, *o))));
        if len(rest) > 1e-3
            && let Some(u) = unit(rest)
        {
            out.push(u);
        }
    }
    // A direction reads with its largest component positive.
    out.into_iter()
        .map(|d| {
            let big = d.iter().copied().fold(0.0, |a: f64, x| if x.abs() > a.abs() { x } else { a });
            if big < 0.0 { mul(d, -1.0) } else { d }
        })
        .collect()
}

/// Orthonormalise `vs` (Gram-Schmidt), dropping those left shorter than `floor`.
fn orthonormal<const N: usize>(vs: impl IntoIterator<Item = [f64; N]>, floor: f64) -> Vec<[f64; N]> {
    let mut out: Vec<[f64; N]> = Vec::new();
    for v in vs {
        let mut r = v;
        for o in &out {
            let d: f64 = r.iter().zip(o).map(|(a, b)| a * b).sum();
            r = std::array::from_fn(|i| r[i] - d * o[i]);
        }
        let l = r.iter().map(|x| x * x).sum::<f64>().sqrt();
        if l > floor && l.is_finite() {
            out.push(r.map(|x| x / l));
        }
    }
    out
}

/// How `holds` leave a part free to move as a rigid body. A hold at point p along unit e answers
/// the six rigid-body motions (a slide t, a turn ω) with e·(t + ω × (p − c)), the row [e, q × e]
/// with q = (p − c) / `size`, c the holds' centroid: scaled so turns and slides weigh alike. The
/// Gram matrix G = Σ rᵀr has an eigenvalue of (near) zero for every motion no hold resists. The
/// free slides are the translation block's own null space; what else is free is a turn, named by
/// its axis (X, Y or Z where it can) and a point on it.
pub fn free_motions(holds: &[Hold], size: f64) -> Freedom {
    let size = if size.is_finite() && size > 0.0 { size } else { 1.0 };
    let usable: Vec<&Hold> = holds.iter().filter(|h| h.at.iter().chain(&h.along).all(|x| x.is_finite())).collect();
    let n = usable.len().max(1) as f64;
    let c = mul(usable.iter().fold([0.0; 3], |a, h| add(a, h.at)), 1.0 / n);
    let mut g = [[0.0; 6]; 6];
    for h in &usable {
        let m = cross(mul(sub(h.at, c), 1.0 / size), h.along);
        let r = [h.along[0], h.along[1], h.along[2], m[0], m[1], m[2]];
        for (row, ri) in g.iter_mut().zip(r) {
            for (x, rj) in row.iter_mut().zip(r) {
                *x += ri * rj;
            }
        }
    }
    let (values, vectors) = sym_eigen(g);
    let top = values.iter().copied().fold(0.0, f64::max);
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let ratio = if top > 0.0 { (low / top).max(0.0) } else { 0.0 };
    let free = |x: f64| top <= 0.0 || x <= RIGID_TOLERANCE * top;
    let null: Vec<[f64; 6]> = (0..6).filter(|k| values.get(*k).is_some_and(|x| free(*x))).map(|k| column(&vectors, k)).collect();
    // Slides: the translations no hold resists.
    let gt: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| g[i][j]));
    let (tv, tvec) = sym_eigen(gt);
    let slide_basis: Vec<P> = (0..3).filter(|k| tv.get(*k).is_some_and(|x| free(*x))).map(|k| column(&tvec, k)).collect();
    let slides = named_directions(&slide_basis);
    // Turns: the rest of the null space, with the free slides taken out of each vector.
    let turns_only = null.iter().map(|v| {
        let t = slide_basis.iter().fold([v[0], v[1], v[2]], |t, s| sub(t, mul(*s, dot(t, *s))));
        [t[0], t[1], t[2], v[3], v[4], v[5]]
    });
    let rot = orthonormal(turns_only, 1e-3);
    let w_basis = orthonormal(rot.iter().map(|v| [v[3], v[4], v[5]]), 1e-3);
    let axes = named_directions(&w_basis);
    // For each axis, the free motion that turns about it: the combination of `rot` whose turn is
    // that axis (least squares through the pseudo-inverse of WᵀW, W the turns of `rot`).
    let w_of = |v: &[f64; 6]| -> P { [v[3], v[4], v[5]] };
    let mut wtw = [[0.0; 3]; 3];
    for (i, a) in rot.iter().enumerate().take(3) {
        for (j, b) in rot.iter().enumerate().take(3) {
            wtw[i][j] = dot(w_of(a), w_of(b));
        }
    }
    let (ev, evec) = sym_eigen(wtw);
    let big = ev.iter().copied().fold(0.0, f64::max);
    let turns = axes
        .iter()
        .map(|axis| {
            let rhs: P = std::array::from_fn(|i| rot.get(i).map_or(0.0, |v| dot(w_of(v), *axis)));
            // a = Σ_k (q_k · rhs / λ_k) q_k over the eigenpairs of WᵀW above round-off.
            let coef = (0..3).filter(|k| ev.get(*k).is_some_and(|x| *x > 1e-9 * big)).fold([0.0; 3], |acc, k| {
                let q = column(&evec, k);
                add(acc, mul(q, dot(q, rhs) / ev[k]))
            });
            let v = rot.iter().zip(coef).fold([0.0; 6], |acc, (r, a)| std::array::from_fn(|i| acc[i] + a * r[i]));
            let (t, w) = ([v[0], v[1], v[2]], [v[3], v[4], v[5]]);
            let ww = dot(w, w);
            // u(p) = t + ω × (p − c) with ω = w / size is a turn about an axis through
            // c + size (w × t) / |w|².
            let through = if ww > 1e-18 { add(c, mul(cross(w, t), size / ww)) } else { c };
            (*axis, through)
        })
        .collect();
    Freedom { slides, turns, ratio }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
