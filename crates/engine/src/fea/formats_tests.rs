use super::*;

#[test]
fn a_gmsh_mesh_reads_its_nodes_and_second_order_elements_and_skips_the_rest() {
    let text = "$MeshFormat\n2.2 0 8\n$EndMeshFormat\n$Nodes\n4\n1 0 0 0\n2 1 0 0\n3 0 1 0\n4 0 0 1\n$EndNodes\n$Elements\n4\n\
                1 15 2 0 1 1\n2 9 2 0 7 1 2 3 1 2 3\n3 11 2 0 1 1 2 3 4 1 2 3 4 1 2\n4 9 2 0 7 1 2 bad 1 2 3\n$EndElements\n";
    let m = read_msh(text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(m.nodes.len(), 4);
    assert_eq!(m.tri6.len(), 1, "a point skipped, a malformed line skipped");
    assert_eq!(m.tri6[0].0, 7, "the surface is the elementary tag");
    assert_eq!(m.tet10.len(), 1);
}

#[test]
fn a_mesh_in_another_format_or_without_triangles_is_refused() {
    assert!(read_msh("$MeshFormat\n4.1 0 8\n$EndMeshFormat\n").is_err());
    assert!(read_msh("$Nodes\n1\n1 0 0 0\n$EndNodes\n").is_err());
    assert!(read_msh("").is_err());
    assert!(read_msh("$Nodes\n99999999999\n").is_err());
}

/// A results line as the solver writes it: ` -1`, the id in `width` characters, values `%12.5E`.
fn line(width: usize, id: u64, values: &[f64]) -> String {
    let mut s = format!(" -1{id:>width$}");
    for v in values {
        let e = format!("{v:.5E}");
        // `1.00000E0` as `1.00000E+00`, the solver's form.
        let (m, x) = e.split_once('E').unwrap_or((&e, "0"));
        let x: i32 = x.parse().unwrap_or(0);
        s.push_str(&format!("{:>12}", format!("{m}E{}{:02}", if x < 0 { '-' } else { '+' }, x.abs())));
    }
    s
}

#[test]
fn solver_results_read_in_short_and_long_form() {
    for width in [5, 10] {
        let text = [
            "    1C".to_string(),
            "    2C                             2                                     1".into(),
            line(width, 1, &[0.0, 0.0, 0.0]),
            line(width, 2, &[100.0, -2.5, 3.25e-3]),
            " -3".into(),
            "  100CL  101 1.00000E+00         2                     0    1           1".into(),
            " -4  DISP        4    1".into(),
            " -5  D1          1    2    1    0".into(),
            line(width, 1, &[0.0, 0.0, 0.0]),
            line(width, 2, &[-1.5, 0.0, 2.0]),
            " -3".into(),
            " -4  STRESS      6    1".into(),
            line(width, 1, &[10.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            line(width, 2, &[-200.0, 5.0, 0.0, 1.0, 0.0, -1.0]),
            " -3".into(),
            // The external forces (`RF` in the job), headed as CalculiX 2.21 heads them, then a block
            // that is not read.
            " -4  FORC        4    1".into(),
            " -5  F1          1    2    1    0".into(),
            " -5  ALL         1    2    0    0    1ALL".into(),
            line(width, 1, &[5.97022e-14, 0.0, 1.32004]),
            line(width, 2, &[0.0, 0.0, -10.0]),
            " -3".into(),
            " -4  ERROR       1    1".into(),
            line(width, 1, &[16.0905]),
            " -3".into(),
            "9999".into(),
        ]
        .join("\n");
        let r = read_frd(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.nodes.get(&2), Some(&[100.0, -2.5, 3.25e-3]), "width {width}");
        assert_eq!(r.disp.get(&2), Some(&[-1.5, 0.0, 2.0]));
        assert_eq!(r.stress.get(&2).map(|s| s[0]), Some(-200.0));
        assert_eq!(r.stress.len(), 2);
        assert_eq!(r.forces.get(&1), Some(&[5.97022e-14, 0.0, 1.32004]));
        assert_eq!(r.forces.get(&2), Some(&[0.0, 0.0, -10.0]));
        assert_eq!(r.forces.len(), 2, "the error estimate's block is not a force");
    }
}

/// The results file's elements: each one's type and its node ids, over as many ` -2` lines as it
/// takes. Input: an element block as CalculiX 2.21 wrote it for a shell (a 15-node wedge over two
/// lines, copied from a solved job) and a 10-node tetrahedron on one line, with stray lines
/// around. Action: `read_frd`. Pass: two elements, types 5 and 6, with their 15 and 10 ids in the
/// file's order.
#[test]
fn solver_results_list_their_elements() {
    let text = [
        "    2C                             2                                     1".to_string(),
        line(10, 7, &[0.0, 0.0, -0.5]),
        " -3".into(),
        "    3C                             2                                     1".into(),
        " -1         1    5    0    1".into(),
        " -2         7        10        13         9        12        15        16        19        22         8".into(),
        " -2        11        14        18        21        24".into(),
        " -1         2    6    0    1".into(),
        " -2         1         2         3         4         5         6         7         8         9        10".into(),
        " -2 x".into(),
        " -3".into(),
        " -4  DISP        4    1".into(),
        line(10, 7, &[0.0, 0.0, 0.0]),
        " -3".into(),
        " -4  STRESS      6    1".into(),
        line(10, 7, &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        " -3".into(),
    ]
    .join("\n");
    let r = read_frd(&text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r.elements, vec![(5, vec![7, 10, 13, 9, 12, 15, 16, 19, 22, 8, 11, 14, 18, 21, 24]), (6, (1..=10).collect::<Vec<u64>>())],);
}

#[test]
fn results_without_stresses_and_hostile_lines_are_refused_without_a_panic() {
    assert!(read_frd("").is_err());
    assert!(read_frd("    2C\n -1    1 1.00000E+00\n -3\n").is_err());
    for junk in [" -1", " -1 é€€€€€€€€€€€€€€€€€€€€€€€€€€€€", " -4", " -4  STRESS\n -1    1 nan nan nan nan nan nan"]
    {
        assert!(read_frd(junk).is_err(), "{junk}");
    }
}

#[test]
fn von_mises_of_simple_states() {
    assert!((von_mises(&[120.0, 0.0, 0.0, 0.0, 0.0, 0.0]) - 120.0).abs() < 1e-9, "uniaxial: the stress itself");
    assert!((von_mises(&[0.0, 0.0, 0.0, 10.0, 0.0, 0.0]) - 10.0 * 3f64.sqrt()).abs() < 1e-9, "pure shear: √3 τ");
    assert!(von_mises(&[50.0, 50.0, 50.0, 0.0, 0.0, 0.0]).abs() < 1e-9, "hydrostatic: none");
}

#[test]
fn a_shell_job_holds_rotations_and_a_solid_job_translations() {
    let nodes = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.0, 0.0], [0.5, 0.5, 0.0], [0.0, 0.5, 0.0]];
    let m = InpMaterial { e_mpa: 70000.0, nu: 0.33 };
    let holds = InpHolds { fixed: &[0, 2, 5], pinned: &[], rollers: &[], untilted: &[] };
    let shell = write_inp(&nodes, &InpElements::Shell { tris: &[[0, 1, 2, 3, 4, 5]], thickness: 1.5 }, &m, &holds, &[(1, [0.0, 0.0, -10.0])]);
    assert!(shell.contains("TYPE=S6") && shell.contains("FIXED, 1, 6") && shell.contains("2, 3, -1.0"), "{shell}");
    assert!(shell.contains("*SHELL SECTION") && shell.contains("1.5"));
    assert!(shell.contains("*NODE FILE, OUTPUT=3D\nU,RF\n"), "every expanded node's force: {shell}");
    let solid = write_inp(
        &nodes,
        &InpElements::Solid { tets: &[[0, 1, 2, 3, 4, 5, 0, 1, 2, 3]] },
        &m,
        &InpHolds { fixed: &[0], pinned: &[], rollers: &[], untilted: &[] },
        &[],
    );
    assert!(solid.contains("TYPE=C3D10") && solid.contains("FIXED, 1, 3") && solid.contains("*SOLID SECTION"), "{solid}");
    assert!(solid.contains("*NODE FILE\nU,RF\n"), "every node's force: {solid}");
}

/// D3: a shell support over an area holds translations only (PINNED, 1, 3); a node that is also
/// clamped stays clamped; pinned nodes count in HELD's reaction total.
#[test]
fn a_shell_held_over_an_area_keeps_its_rotations() {
    let nodes = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.0, 0.0], [0.5, 0.5, 0.0], [0.0, 0.5, 0.0]];
    let m = InpMaterial { e_mpa: 70000.0, nu: 0.33 };
    let holds = InpHolds { fixed: &[0], pinned: &[0, 1, 3], rollers: &[], untilted: &[] };
    let shell = write_inp(&nodes, &InpElements::Shell { tris: &[[0, 1, 2, 3, 4, 5]], thickness: 1.5 }, &m, &holds, &[(2, [0.0, 0.0, -10.0])]);
    assert!(
        shell.contains(
            "*NSET, NSET=PINNED
2, 4,
"
        ) && shell.contains(
            "PINNED, 1, 3
"
        ),
        "{shell}"
    );
    assert!(
        shell.contains("FIXED, 1, 6")
            && shell.contains(
                "*NSET, NSET=HELD
1, 2, 4,
"
            ),
        "{shell}"
    );
    let none = write_inp(
        &nodes,
        &InpElements::Shell { tris: &[[0, 1, 2, 3, 4, 5]], thickness: 1.5 },
        &m,
        &InpHolds { fixed: &[0], ..InpHolds::default() },
        &[],
    );
    assert!(!none.contains("PINNED"), "{none}");
}

/// Rollers hold one translation each, in a set per axis; every held node is in HELD, whose
/// reaction totals the job prints; an empty set is never written (the solver refuses one).
#[test]
fn rollers_hold_one_axis_and_the_job_prints_the_held_nodes_reaction_totals() {
    let nodes = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.0, 0.0], [0.5, 0.5, 0.0], [0.0, 0.5, 0.0]];
    let m = InpMaterial { e_mpa: 70000.0, nu: 0.33 };
    let tets = [[0, 1, 2, 3, 4, 5, 0, 1, 2, 3]];
    let job = write_inp(
        &nodes,
        &InpElements::Solid { tets: &tets },
        &m,
        &InpHolds { fixed: &[], pinned: &[], rollers: &[(0, 0), (2, 1), (5, 1), (2, 1)], untilted: &[] },
        &[],
    );
    assert!(!job.contains("NSET=FIXED") && !job.contains("FIXED, 1"), "no empty fixed set: {job}");
    assert!(job.contains("*NSET, NSET=ROLLX\n1,") && job.contains("*NSET, NSET=ROLLY\n3, 6,") && !job.contains("ROLLZ"), "{job}");
    assert!(job.contains("ROLLX, 1, 1") && job.contains("ROLLY, 2, 2"), "{job}");
    assert!(job.contains("*NSET, NSET=HELD\n1, 3, 6,") && job.contains("*NODE PRINT, NSET=HELD, TOTALS=ONLY\nRF"), "{job}");
}

/// A symmetry plane on shells: its nodes hold the translation across the plane and the turns
/// about the plane's two other axes (a plane across Y: turns about X and Z, degrees of freedom 4
/// and 6, not 5); on solids it is a roller only (their nodes have no turns).
#[test]
fn a_symmetry_plane_on_shells_also_keeps_the_edge_from_tilting() {
    let nodes = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.0, 0.0], [0.5, 0.5, 0.0], [0.0, 0.5, 0.0]];
    let m = InpMaterial { e_mpa: 70000.0, nu: 0.33 };
    let holds = InpHolds { fixed: &[0], pinned: &[], rollers: &[(2, 1), (5, 1)], untilted: &[(2, 1), (5, 1)] };
    let shell = write_inp(&nodes, &InpElements::Shell { tris: &[[0, 1, 2, 3, 4, 5]], thickness: 1.5 }, &m, &holds, &[]);
    assert!(shell.contains("*NSET, NSET=SYMY\n3, 6,") && shell.contains("ROLLY, 2, 2"), "{shell}");
    assert!(
        shell.contains("SYMY, 4, 4\nSYMY, 6, 6") && !shell.contains("SYMY, 5, 5") && !shell.contains("SYMX") && !shell.contains("SYMZ"),
        "{shell}"
    );
    let solid = write_inp(&nodes, &InpElements::Solid { tets: &[[0, 1, 2, 3, 4, 5, 0, 1, 2, 3]] }, &m, &holds, &[]);
    assert!(!solid.contains("SYM") && solid.contains("ROLLY, 2, 2"), "{solid}");
}

/// U1 (part): reaction totals read from the solver's `.dat` file.
/// Input: a `.dat` laid out as CalculiX 2.21 wrote it for `*NODE PRINT, NSET=HELD, TOTALS=ONLY`
/// with RF in a real run on zhikun (numbers copied), plus hostile variants. Action:
/// `read_dat_total(text, "HELD")`. Observable: the three numbers of the last block for that set.
/// Pass: exact values; another set, no block, a short line, junk or non-finite numbers give
/// `None` without a panic. Nothing is faked but the text.
#[test]
fn reaction_totals_read_from_the_solvers_dat_file() {
    let dat = "\n                        S T E P       1\n\n\n                                INCREMENT     1\n\n\n total force \
               (fx,fy,fz) for set HELD and time  0.1000000E+01\n\n       -8.881784E-16  4.440892E-16  1.000000E+01\n";
    assert_eq!(read_dat_total(dat, "HELD"), Some([-8.881784e-16, 4.440892e-16, 10.0]));
    assert_eq!(read_dat_total(dat, "held"), Some([-8.881784e-16, 4.440892e-16, 10.0]), "set names are upper case in the file");
    assert_eq!(read_dat_total(dat, "FIXED"), None, "another set");
    let two = format!("{dat}{}", dat.replace("1.000000E+01", "1.250000E+01"));
    assert_eq!(read_dat_total(&two, "HELD").map(|v| v[2]), Some(12.5), "the last block counts");
    for junk in [
        "",
        "total force (fx,fy,fz) for set HELD and time 1",
        "total force (fx,fy,fz) for set HELD and time 1\n\n 1 2",
        "total force for set HELD x\n nan 1 2",
        "total force for set HELD x\n é 1 2 3",
        "total force for set HELD x\n 1 2 inf",
    ] {
        assert_eq!(read_dat_total(junk, "HELD"), None, "{junk}");
    }
}
