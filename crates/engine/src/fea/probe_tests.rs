use super::*;

/// A quadratic field: what a 10-node tetrahedron and a 15-node wedge reproduce exactly.
fn quadratic(p: P) -> f64 {
    let [x, y, z] = p;
    1.0 + x - 2.0 * y + 0.5 * z + x * x + x * y - y * z + 0.3 * z * z + 0.7 * x * z - 0.4 * y * y
}

/// Nodes at `pos`, with the quadratic field as each one's xx stress (the other components zero)
/// and as its displacement along X.
fn field_at(pos: &[P]) -> (Vec<(P, f64, P)>, Vec<[f64; 6]>) {
    let points = pos.iter().map(|p| (*p, 0.0, [quadratic(*p), 0.0, 0.0])).collect();
    let tensors = pos.iter().map(|p| [quadratic(*p), 0.0, 0.0, 0.0, 0.0, 0.0]).collect();
    (points, tensors)
}

/// A tetrahedron's corners, then its edges' middles in [`TET_EDGES`] order.
fn tet_nodes() -> Vec<P> {
    let c: [P; 4] = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0]];
    let mut out = c.to_vec();
    out.extend(TET_EDGES.iter().map(|(a, b)| model::mid(c[*a], c[*b])));
    out
}

/// A wedge's corners (a triangle at z = 0 and the same at z = 1.5), then its edges' middles in
/// [`WEDGE_EDGES`] order.
fn wedge_nodes() -> Vec<P> {
    let c: [P; 6] = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 1.5], [2.0, 0.0, 1.5], [0.0, 2.0, 1.5]];
    let mut out = c.to_vec();
    out.extend(WEDGE_EDGES.iter().map(|(a, b)| model::mid(c[*a], c[*b])));
    out
}

/// P1: principal stresses and their directions.
/// Input: pure shear (xy = 10 MPa); a general tensor. Action: `principal`. Pass: pure shear gives
/// 10, 0, −10 MPa along (1, 1, 0)/√2, Z and (1, −1, 0)/√2 (each pointing where its largest
/// component is positive); the general tensor's values sum to its trace, come largest first, and
/// Σ λ v vᵀ gives the tensor back to 1e-9.
#[test]
fn p1_principal_stresses_and_directions() {
    let (v, d) = principal(&[0.0, 0.0, 0.0, 10.0, 0.0, 0.0]);
    let r = std::f64::consts::FRAC_1_SQRT_2;
    let close = |a: P, b: P| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9);
    assert!((v[0] - 10.0).abs() < 1e-9 && v[1].abs() < 1e-9 && (v[2] + 10.0).abs() < 1e-9, "{v:?}");
    assert!(close(d[0], [r, r, 0.0]) && close(d[1], [0.0, 0.0, 1.0]) && (close(d[2], [r, -r, 0.0]) || close(d[2], [-r, r, 0.0])), "{d:?}");
    let s = [120.0, -35.0, 12.0, 40.0, -8.0, 25.0];
    let (v, d) = principal(&s);
    assert!(v[0] >= v[1] && v[1] >= v[2], "{v:?}");
    assert!((v.iter().sum::<f64>() - (s[0] + s[1] + s[2])).abs() < 1e-9);
    let back = |i: usize, j: usize| (0..3).map(|k| v[k] * d[k][i] * d[k][j]).sum::<f64>();
    let rebuilt = [back(0, 0), back(1, 1), back(2, 2), back(0, 1), back(1, 2), back(2, 0)];
    assert!((0..6).all(|k| (rebuilt[k] - s[k]).abs() < 1e-9), "{rebuilt:?} vs {s:?}");
}

/// P2: elastic strain by Hooke's law, and a gauge's reading along a direction.
/// Input: 210 MPa along X, then 80 MPa shear in XY, in a material of E 210 000 MPa and ν 0.3.
/// Action: `strain`, `along`. Pass: εxx = 1e-3 and εyy = εzz = −3e-4; the shear's tensor strain is
/// τ / 2G (G = E / 2(1 + ν)); along X the stress reads 210 MPa, and along (1, 1, 0)/√2 the shear
/// reads +80 MPa and its strain +τ / 2G.
#[test]
fn p2_strain_from_hookes_law_and_along_a_direction() {
    let e = strain(&[210.0, 0.0, 0.0, 0.0, 0.0, 0.0], 210_000.0, 0.3);
    assert!((e[0] - 1e-3).abs() < 1e-15 && (e[1] + 3e-4).abs() < 1e-15 && (e[2] + 3e-4).abs() < 1e-15, "{e:?}");
    assert!(e[3..].iter().all(|x| *x == 0.0));
    let shear = [0.0, 0.0, 0.0, 80.0, 0.0, 0.0];
    let g = 210_000.0 / (2.0 * 1.3);
    let es = strain(&shear, 210_000.0, 0.3);
    assert!((es[3] - 80.0 / (2.0 * g)).abs() < 1e-15 && es[..3].iter().all(|x| x.abs() < 1e-18), "{es:?}");
    assert!((along(&[210.0, 0.0, 0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0]) - 210.0).abs() < 1e-12);
    let r = std::f64::consts::FRAC_1_SQRT_2;
    assert!((along(&shear, [r, r, 0.0]) - 80.0).abs() < 1e-12);
    assert!((along(&es, [r, r, 0.0]) - 80.0 / (2.0 * g)).abs() < 1e-15);
}

/// P3: a 10-node tetrahedron reads a quadratic field exactly between its nodes, whatever order the
/// results file gives its mid-side nodes in.
/// Input: a tetrahedron with corners (0,0,0), (2,0,0), (0,3,0), (0,0,4), its mid-side nodes in
/// three orders, and a quadratic field at its nodes. Action: `cell`, then `sample` at two inside
/// points. Pass: each order gives the same cell; the field read equals the quadratic to 1e-9, for
/// the stress and the displacement; the point is inside (0 outside).
#[test]
fn p3_a_tetrahedron_reads_a_quadratic_field_exactly() {
    let pos = tet_nodes();
    let (points, tensors) = field_at(&pos);
    let at = |i: usize| points.get(i).map(|x| x.0);
    let reference = cell(FRD_TET10, &(0..10).collect::<Vec<_>>(), &at);
    assert_eq!(reference, Some(Cell::Tet([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])));
    for order in [[0, 1, 2, 3, 9, 8, 7, 6, 5, 4], [0, 1, 2, 3, 6, 4, 9, 5, 8, 7]] {
        assert_eq!(cell(FRD_TET10, &order, &at), reference, "{order:?}");
    }
    let cells = vec![reference.unwrap_or(Cell::Tet([0; 10]))];
    for p in [[0.4, 0.5, 0.6], [0.1, 1.2, 0.3]] {
        let s = sample(&cells, &points, &tensors, p).unwrap_or_else(|m| panic!("{m:?}"));
        assert!((s.stress[0] - quadratic(p)).abs() < 1e-9 && (s.disp[0] - quadratic(p)).abs() < 1e-9, "{p:?}: {s:?}");
        assert_eq!(s.outside, 0.0);
    }
}

/// P4: a 15-node wedge (a shell triangle expanded through its thickness) reads a quadratic field
/// exactly, with its mid-side nodes in the order the solver's results file gives them (the first
/// face's, the joining edges', the second face's) or any other.
/// Input: a wedge over the triangle (0,0), (2,0), (0,2), from z = 0 to 1.5; a quadratic field at
/// its nodes. Action: `cell`, then `sample` inside and on its two faces. Pass: both orders give
/// the canonical cell; the field read equals the quadratic to 1e-9.
#[test]
fn p4_a_wedge_reads_a_quadratic_field_exactly_through_the_sheet() {
    let pos = wedge_nodes();
    let (points, tensors) = field_at(&pos);
    let at = |i: usize| points.get(i).map(|x| x.0);
    let canonical = Cell::Wedge(std::array::from_fn(|i| i));
    // The results file: corners, the first face's middles, the joining edges', the second face's.
    let file = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 13, 14, 9, 10, 11];
    assert_eq!(cell(FRD_WEDGE15, &file, &at), Some(canonical.clone()));
    assert_eq!(cell(FRD_WEDGE15, &[0, 1, 2, 3, 4, 5, 14, 13, 12, 11, 10, 9, 8, 7, 6], &at), Some(canonical.clone()));
    let cells = vec![canonical];
    for p in [[0.5, 0.4, 0.3], [0.2, 0.9, 0.0], [1.1, 0.3, 1.5], [0.5, 0.5, 0.75]] {
        let s = sample(&cells, &points, &tensors, p).unwrap_or_else(|m| panic!("{m:?}"));
        assert!((s.stress[0] - quadratic(p)).abs() < 1e-9, "{p:?}: {} vs {}", s.stress[0], quadratic(p));
    }
}

/// P5: a point just outside the part is read at the part's nearest point; one farther is not.
/// Input: P3's tetrahedron; a point 0.3 mm below its z = 0 face, one 5 mm below; a mid-side node
/// that sits at no edge's middle; an element type this does not read. Action: `sample`, `cell`.
/// Pass: the first is read with 0.3 mm outside and the field at the face point above it; the
/// second is `Outside`, about 5 mm; the stray node and the other type give no cell; no cells at
/// all is `NoElements`.
#[test]
fn p5_points_outside_and_elements_that_do_not_fit() {
    let pos = tet_nodes();
    let (points, tensors) = field_at(&pos);
    let cells = vec![Cell::Tet(std::array::from_fn(|i| i))];
    let s = sample(&cells, &points, &tensors, [0.4, 0.5, -0.3]).unwrap_or_else(|m| panic!("{m:?}"));
    assert!((s.outside - 0.3).abs() < 1e-9, "{s:?}");
    assert!((s.stress[0] - quadratic([0.4, 0.5, 0.0])).abs() < 1e-9, "{s:?}");
    match sample(&cells, &points, &tensors, [0.4, 0.5, -5.0]) {
        Err(Miss::Outside(d)) => assert!((d - 5.0).abs() < 0.5, "{d}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(sample(&[], &points, &tensors, [0.4, 0.5, 0.5]), Err(Miss::NoElements));
    let mut stray = pos.clone();
    stray[4] = model::mid(stray[2], stray[3]);
    let at = |i: usize| stray.get(i).copied();
    assert_eq!(cell(FRD_TET10, &(0..10).collect::<Vec<_>>(), &at), None, "two mid-side nodes on one edge");
    let at = |i: usize| pos.get(i).copied();
    assert_eq!(cell(4, &(0..10).collect::<Vec<_>>(), &at), None);
    assert_eq!(cell(FRD_TET10, &(0..9).collect::<Vec<_>>(), &at), None);
}
