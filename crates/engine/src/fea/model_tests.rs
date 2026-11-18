use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::fea::formats::Msh;

fn tri6_model() -> Model {
    // One six-node triangle, 2 x 2 mm, on face 0; its edge x = 0 is face 1.
    let nodes = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
    let e = [0, 1, 2, 3, 4, 5];
    let mut on_face = BTreeMap::new();
    on_face.insert(0, ((0..6).collect::<BTreeSet<usize>>(), vec![e]));
    on_face.insert(1, ([0, 2, 5].into_iter().collect::<BTreeSet<usize>>(), Vec::new()));
    Model { nodes, shells: vec![e], tets: Vec::new(), on_face }
}

fn total(f: &[(usize, P)]) -> P {
    f.iter().fold([0.0; 3], |a, (_, x)| add(a, *x))
}

#[test]
fn a_force_over_an_area_goes_to_the_mid_edge_nodes_and_sums_to_the_total() {
    let m = tri6_model();
    let f = spread(&m, &[0], [0.0, 0.0, -30.0]).unwrap_or_default();
    assert!((total(&f)[2] + 30.0).abs() < 1e-9);
    for (n, x) in &f {
        assert!(*n >= 3, "corners take none of a six-node triangle's pressure: {f:?}");
        assert!((x[2] + 10.0).abs() < 1e-9);
    }
}

#[test]
fn a_force_on_an_edge_face_goes_a_sixth_two_thirds_a_sixth() {
    let m = tri6_model();
    let f = spread(&m, &[1], [6.0, 0.0, 0.0]).unwrap_or_default();
    let at = |n: usize| f.iter().find(|(i, _)| *i == n).map_or(0.0, |(_, x)| x[0]);
    assert!((at(0) - 1.0).abs() < 1e-9 && (at(2) - 1.0).abs() < 1e-9 && (at(5) - 4.0).abs() < 1e-9, "{f:?}");
}

#[test]
fn a_face_without_nodes_takes_no_force_and_holds_nothing() {
    let m = tri6_model();
    assert!(spread(&m, &[7], [1.0, 0.0, 0.0]).is_none());
    assert!(held(&m, &[7]).is_empty());
    assert_eq!(held(&m, &[1, 1]), vec![0, 2, 5]);
}

#[test]
fn mid_edge_nodes_are_matched_by_where_they_lie_whatever_their_order() {
    let corners = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
    let mids = [(30, [0.0, 1.0, 0.0]), (10, [1.0, 0.0, 0.0]), (20, [1.0, 1.0, 0.0])];
    assert_eq!(mids_by_edge(&corners, &mids, &TRI_EDGES), Some(vec![10, 20, 30]));
    assert_eq!(mids_by_edge(&corners, &mids[..2], &TRI_EDGES), None, "a missing node is no element");
}

#[test]
fn a_tetrahedron_listed_inside_out_is_turned_and_keeps_its_edges() {
    let mut msh = Msh::default();
    let pts = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    for (i, p) in pts.iter().enumerate() {
        msh.nodes.insert(i as u64 + 1, *p);
    }
    let mut mids = Vec::new();
    for (id, (a, b)) in (5..).zip([(0, 1), (1, 2), (2, 0), (0, 3), (2, 3), (1, 3)]) {
        msh.nodes.insert(id, mid(pts[a], pts[b]));
        mids.push(id);
    }
    let t: Vec<u64> = (1..=4).chain(mids).collect();
    msh.tet10.push(<[u64; 10]>::try_from(t).unwrap_or([0; 10]));
    msh.tri6.push((1, [1, 2, 3, 5, 6, 7]));
    // The same triangle listed the other way round, as another surface.
    msh.tri6.push((2, [1, 3, 2, 7, 6, 5]));
    let faces = BTreeMap::from([(1, 0u32), (2, 1u32)]);
    let m = solid_model(&msh, &faces).unwrap_or_else(|e| panic!("{e}"));
    let e = m.tets[0];
    let p = |i: usize| m.nodes[e[i]];
    assert!(dot(cross(sub(p(1), p(0)), sub(p(2), p(0))), sub(p(3), p(0))) > 0.0, "positive volume");
    for (k, (a, b)) in TET_EDGES.iter().enumerate() {
        assert!(len(sub(p(4 + k), mid(p(*a), p(*b)))) < 1e-12, "edge {k}");
    }
    assert_eq!(m.on_face.get(&0).map(|x| x.1.len()), Some(1));
    // Listed either way round, each surface triangle ends up with its normal out of the part,
    // its mid-edge nodes still on their edges.
    for face in [0, 1] {
        let f = m.on_face.get(&face).map(|x| x.1[0]).unwrap_or([0; 6]);
        let q = |i: usize| m.nodes[f[i]];
        let inside = [0.0, 0.0, 1.0];
        assert!(dot(cross(sub(q(1), q(0)), sub(q(2), q(0))), sub(inside, q(0))) < 0.0, "face {face}: the normal points out of the part");
        for (k, (a, b)) in TRI_EDGES.iter().enumerate() {
            assert!(len(sub(q(3 + k), mid(q(*a), q(*b)))) < 1e-12, "face {face}: mid-edge node on edge {k}");
        }
    }
    // A pressure of 3 MPa pushes in: −3 × area along the outward normal, a third at each mid-edge node.
    let f = pressure(&m, &[0], 3.0).unwrap_or_default();
    let sum = f.iter().fold([0.0; 3], |a, (_, x)| add(a, *x));
    assert!(len(sub(sum, [0.0, 0.0, 1.5])) < 1e-12, "area 0.5 at z = 0, the tetrahedron above: pushed up, into it: {f:?}");
    assert_eq!(f.len(), 3, "mid-edge nodes only");
    assert!(pressure(&m, &[9], 3.0).is_none() && pressure(&m, &[0], f64::NAN).is_none());
}

/// A pressure pushes against the normal the model stored for each element face (out of the part
/// through that face; the solid and shell models turn their faces so): turned over, it pushes
/// the other way, and its size is p × area.
#[test]
fn a_pressure_pushes_against_each_element_faces_stored_normal() {
    let mut m = tri6_model();
    for e in [[0, 1, 2, 3, 4, 5], flip([0, 1, 2, 3, 4, 5])] {
        m.on_face.insert(0, ((0..6).collect(), vec![e]));
        let n = cross(sub(m.nodes[e[1]], m.nodes[e[0]]), sub(m.nodes[e[2]], m.nodes[e[0]]));
        let f = pressure(&m, &[0], 2.0).unwrap_or_default();
        let sum = f.iter().fold([0.0; 3], |a, (_, x)| add(a, *x));
        // 2 MPa on 2 mm²: 4 N against the element face's normal.
        assert!(len(sub(sum, mul(unit(n).unwrap_or([0.0; 3]), -4.0))) < 1e-12, "{sum:?}");
    }
}

#[test]
fn the_grid_finds_the_nearest_point() {
    let pts: Vec<P> = (0..100).map(|i| [f64::from(i % 10), f64::from(i / 10), 0.0]).collect();
    let g = Grid::new(pts, 1.5);
    assert_eq!(g.nearest([3.2, 4.9, 0.1], 4), Some(53));
    assert_eq!(g.nearest([-20.0, -20.0, 0.0], 40), Some(0));
    assert_eq!(Grid::new(Vec::new(), 1.0).nearest([0.0; 3], 3), None);
}

#[test]
fn rays_and_distances_to_triangles() {
    let (a, b, c) = ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    assert_eq!(ray_triangle([0.2, 0.2, 1.0], [0.0, 0.0, -1.0], a, b, c), Some(1.0));
    assert_eq!(ray_triangle([0.2, 0.2, 1.0], [0.0, 0.0, 1.0], a, b, c), None);
    assert!((point_triangle([0.2, 0.2, 2.0], a, b, c) - 2.0).abs() < 1e-12);
    assert!((point_triangle([2.0, 0.0, 0.0], a, b, c) - 1.0).abs() < 1e-12);
}

/// A small deterministic random number source (no dependency): the same numbers every run.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn point(&mut self, scale: f64) -> P {
        [self.next() * scale, self.next() * scale, self.next() * scale * 0.2]
    }
}

/// G4: the triangle grid gives exactly a full scan's answers.
/// Input: 3 000 random triangles about 1 mm across in a 40 x 40 x 8 mm box (a seeded generator,
/// the same every run), plus one 30 mm triangle; 2 000 random points in and around the box.
/// Action: `TriGrid::distance` and `TriGrid::any_within` at reaches 0.3, 0.9 and 2.25 mm, and the
/// nearest ray hit among `TriGrid::near` against among all triangles.
/// Observable: the distances (bit for bit), the yes/no answers, the hit and which triangle.
/// Pass: every answer equals the full scan's; a hit within the reach is the same hit (same
/// distance, same triangle, the first of equals as a scan finds it).
#[test]
fn g4_the_triangle_grid_answers_exactly_as_a_scan_of_every_triangle() {
    let mut rng = Lcg(7);
    let mut tris: Vec<[P; 3]> = (0..3000)
        .map(|_| {
            let a = rng.point(40.0);
            let b = add(a, [rng.next() - 0.5, rng.next() - 0.5, rng.next() - 0.5]);
            let c = add(a, [rng.next() - 0.5, rng.next() - 0.5, rng.next() - 0.5]);
            [a, b, c]
        })
        .collect();
    tris.push([[5.0, 5.0, 4.0], [35.0, 5.0, 4.0], [5.0, 35.0, 4.0]]);
    let scan = |p: P| tris.iter().map(|[a, b, c]| point_triangle(p, *a, *b, *c)).fold(f64::INFINITY, f64::min);
    for reach in [0.3, 0.9, 2.25] {
        let grid = TriGrid::new(&tris, reach);
        for _ in 0..2000 {
            let p = sub(rng.point(48.0), [4.0, 4.0, 1.0]);
            let full = scan(p);
            assert_eq!(grid.distance(p, reach).to_bits(), full.to_bits(), "distance at {p:?}, reach {reach}");
            assert_eq!(grid.any_within(p, reach), full <= reach, "within at {p:?}, reach {reach}");
            let Some(d) = unit([rng.next() - 0.5, rng.next() - 0.5, rng.next() - 0.5]) else { continue };
            let hit = |ids: &mut dyn Iterator<Item = usize>| {
                ids.filter_map(|i| ray_triangle(p, d, tris[i][0], tris[i][1], tris[i][2]).map(|h| (h, i))).min_by(|a, b| a.0.total_cmp(&b.0))
            };
            let all = hit(&mut (0..tris.len()));
            let near = hit(&mut grid.near(p, reach).into_iter());
            if all.is_some_and(|(h, _)| h <= 0.99 * reach) {
                assert_eq!(near, all, "ray from {p:?} along {d:?}, reach {reach}");
            }
        }
    }
}

/// D3: a support over a side of the sheet (a face with element faces) holds its nodes over the
/// area; an edge among the support's faces (an end) keeps its nodes clamped; a solid model has no
/// area to pin.
#[test]
fn a_support_over_a_side_of_the_sheet_is_an_area_and_an_edge_is_not() {
    let side: BTreeSet<usize> = [0, 1, 2, 3, 4, 5].into_iter().collect();
    let end: BTreeSet<usize> = [2, 4, 6].into_iter().collect();
    let mut on_face = BTreeMap::new();
    on_face.insert(1u32, (side, vec![[0, 1, 2, 3, 4, 5]]));
    on_face.insert(2u32, (end, Vec::new()));
    let shells = Model { nodes: vec![[0.0; 3]; 7], shells: vec![[0, 1, 2, 3, 4, 5]], tets: Vec::new(), on_face };
    assert_eq!(held_area(&shells, &[1]), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(held_area(&shells, &[1, 2]), vec![0, 1, 3, 5], "the end's nodes stay clamped");
    assert!(held_area(&shells, &[2]).is_empty(), "an edge alone is clamped");
    assert!(held_area(&shells, &[9]).is_empty());
    let solid = Model { nodes: shells.nodes.clone(), shells: Vec::new(), tets: Vec::new(), on_face: shells.on_face.clone() };
    assert!(held_area(&solid, &[1]).is_empty());
}
