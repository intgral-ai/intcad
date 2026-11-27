use super::*;

fn run_with(points: Vec<(P, f64, P)>) -> Run {
    Run {
        elements: Elements::Shell,
        mesh_size: 1.0,
        nodes: points.len(),
        element_count: 1,
        max_stress: 0.0,
        max_stress_at: [0.0; 3],
        max_disp: 0.0,
        max_disp_at: [0.0; 3],
        seconds: 0.0,
        points,
        tensors: Vec::new(),
        cells: Vec::new(),
        isolated: false,
        equilibrium: None,
        reactions: Vec::new(),
        peaks: Peaks::default(),
        threads: 1,
        mesher_version: None,
        solver_version: None,
    }
}

#[test]
fn the_field_splits_long_triangles_and_takes_the_nearest_nodes_values() {
    let run = run_with(vec![([0.0, 0.0, 0.0], 10.0, [0.0; 3]), ([8.3, 0.0, 0.0], 50.0, [0.0, 0.0, -1.0])]);
    let pos = [[0.0, 0.0, 0.0], [8.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let nor = [[0.0, 0.0, 1.0]; 3];
    let (p, n, t, s, d) = field(&run, &pos, &nor, &[[0, 1, 2]], 1.0);
    assert!(t.len() > 1, "split");
    assert_eq!(p.len(), 3 * t.len());
    assert_eq!(n.len(), p.len());
    for (q, v) in p.iter().zip(&s) {
        assert_eq!(*v, if q[0] > 4.15 { 50.0 } else { 10.0 }, "{q:?}");
    }
    assert!(d.iter().any(|u| u[2] == -1.0));
}

#[test]
fn a_result_is_shown_only_for_its_own_design() {
    let field = |doc: &str| Field {
        document_id: doc.into(),
        body: "B".into(),
        fingerprint: 0,
        positions: Vec::new(),
        normals: Vec::new(),
        triangles: Vec::new(),
        stress: Vec::new(),
        disp: Vec::new(),
        max_stress: 1.0,
        max_stress_at: [0.0; 3],
        max_disp: 1.0,
        max_disp_at: [0.0; 3],
        yield_mpa: 100.0,
        scale: ScaleMode::Yield,
        quantity: Quantity::Stress,
        hidden: false,
        deform_scale: 1.0,
        heading: String::new(),
        assumptions: Vec::new(),
        warnings: Vec::new(),
        loads: Vec::new(),
        supports: Vec::new(),
        away_stress: None,
        equilibrium: None,
        mesh_check: None,
    };
    let doc = format!("fea-test-{}", revision());
    let before = revision();
    show(field(&doc));
    assert!(revision() > before);
    assert!(shown(&doc).is_some());
    assert!(shown("another design").is_none());
    assert!(set_quantity(&doc, Quantity::Deflection));
    assert_eq!(shown(&doc).map(|f| f.quantity), Some(Quantity::Deflection));
    // Hide keeps it; showing draws it again.
    assert!(update(&doc, |f| f.hidden = true));
    assert_eq!(shown(&doc).map(|f| f.hidden), Some(true));
    assert!(set_quantity(&doc, Quantity::Stress));
    assert_eq!(shown(&doc).map(|f| f.hidden), Some(false));
    assert!(!clear("another design"));
    assert!(clear(&doc));
    assert!(shown(&doc).is_none());
    assert!(!set_quantity(&doc, Quantity::Stress));
}

#[test]
fn a_bodys_fingerprint_is_the_same_on_a_copy_and_changes_with_its_shape() {
    let mut s = crate::Session::default();
    s.execute("solid.box", &serde_json::json!({"length": 60, "width": 20, "height": 10, "body_name": "Block"})).unwrap_or_else(|e| panic!("{e}"));
    let print = |s: &crate::Session| {
        let st = s.world_state().unwrap_or_else(|e| panic!("{e}"));
        st.bodies.iter().find(|b| b.name == "Block").map(fingerprint).unwrap_or(0)
    };
    let here = print(&s);
    assert_eq!(print(&s.scratch()), here, "the estimate runs on a copy: the same shape, the same fingerprint");
    assert_eq!(print(&s), here, "and again on the same design");
    s.execute("solid.hole", &serde_json::json!({"position": [30, 10, 10], "diameter": 5})).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(print(&s), here, "a hole changes it");
}

/// A sheet made thicker inward keeps its outside size and its faces: the area still tells.
#[test]
fn a_thicker_sheet_with_the_same_outside_changes_the_fingerprint() {
    let mut s = crate::Session::default();
    let run = |s: &mut crate::Session, id: &str, p: serde_json::Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run(&mut s, "sheet.manage_rules", serde_json::json!({"name": "R1", "thickness": "1.5 mm", "bend_radius": "1.5 mm", "active": true}));
    run(&mut s, "sketch.create", serde_json::json!({"plane": "XY", "name": "Base"}));
    run(&mut s, "sketch.rectangle.two_point", serde_json::json!({"p0": [0, 0], "p1": [60, 40]}));
    run(&mut s, "sketch.finish", serde_json::json!({}));
    run(&mut s, "sheet.flange", serde_json::json!({"sketch": "Base"}));
    run(&mut s, "sheet.flange", serde_json::json!({"edges": [[30, 40, 1.5]], "height": 40}));
    let print = |s: &crate::Session| {
        let st = s.world_state().unwrap_or_else(|e| panic!("{e}"));
        st.bodies.first().map(fingerprint).unwrap_or(0)
    };
    let thin = print(&s);
    run(&mut s, "sheet.manage_rules", serde_json::json!({"name": "R1", "thickness": "3 mm"}));
    assert_ne!(print(&s), thin, "3 mm is another shape");
}

/// Stop kills the one program the estimate started and nothing else: a process beside it (here
/// another `sleep`) lives on. No process group is signalled, so no other process can be hit.
#[cfg(target_os = "linux")]
#[test]
fn stop_kills_only_the_estimates_own_program() {
    let dir = work_folder().unwrap_or_else(|e| panic!("{e}"));
    let mut sibling = std::process::Command::new("sleep").arg("30").spawn().unwrap_or_else(|e| panic!("{e}"));
    let generation = cancel_generation();
    let stopper = std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        cancel();
    });
    let started = std::time::Instant::now();
    let r = run_program("sleep", &["30".to_string()], &[], &dir, "stop-test", 60, "test program", generation);
    let _ = stopper.join();
    assert!(matches!(r.err(), Some(Ended::Said(t)) if t == STOPPED));
    assert!(started.elapsed().as_secs() < 10, "stopped at once, not when the program ended");
    assert!(matches!(sibling.try_wait(), Ok(None)), "a process beside the estimate's lives on");
    let _ = sibling.kill();
    let _ = sibling.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// U2: the highest stress against the idealised supports and loads, and the peak away from them.
/// Input: synthetic solver nodes along x, 0.5 mm apart (60 MPa), with 100 MPa at x = 0.5 next to a
/// held node at x = 0 ("the clamp"), 80 at x = 2, 95 at x = 19.5 next to a loaded node at x = 20
/// ("the tip"), and 90 at x = 5; away distance 3 mm. Nothing runs; no programs.
/// Action: `peaks(points, anchors, labels, 3.0)`.
/// Observable and pass: the overall peak is flagged at "the clamp"; the away peak is the 90 at
/// x = 5 (80 and 95 lie within 3 mm of an anchor). With the anchors moved off, nothing is flagged
/// and the away peak is the overall one; with every node near an anchor there is no away peak.
/// Distances are true 3D distances, also across grid cells (a diagonal at 2.94 mm is near, at
/// 3.12 mm is away).
#[test]
fn the_peak_away_from_supports_and_loads_skips_nodes_near_them_and_names_the_one_at_the_top() {
    let stress_at = |x: f64| match x {
        x if (x - 0.5).abs() < 1e-9 => 100.0,
        x if (x - 2.0).abs() < 1e-9 => 80.0,
        x if (x - 5.0).abs() < 1e-9 => 90.0,
        x if (x - 19.5).abs() < 1e-9 => 95.0,
        _ => 60.0,
    };
    let points: Vec<(P, f64, P)> = (0..=40).map(|i| f64::from(i) * 0.5).map(|x| ([x, 0.0, 0.0], stress_at(x), [0.0; 3])).collect();
    let labels = vec!["the clamp".to_string(), "the tip".to_string()];
    let anchors = vec![([0.0, 0.0, 0.0], 0), ([0.0, 10.0, 0.0], 0), ([20.0, 0.0, 0.0], 1)];
    let p = peaks(&points, &anchors, &labels, 3.0);
    assert_eq!(p.at.as_deref(), Some("the clamp"));
    assert_eq!(p.away, Some((90.0, [5.0, 0.0, 0.0])));
    assert_eq!(p.distance, 3.0);
    let far = vec![([0.0, 50.0, 0.0], 0)];
    let p = peaks(&points, &far, &labels, 3.0);
    assert_eq!((p.at, p.away), (None, Some((100.0, [0.5, 0.0, 0.0]))), "nothing near: the away peak is the highest");
    let every: Vec<(P, usize)> = points.iter().map(|x| (x.0, 1)).collect();
    let p = peaks(&points, &every, &labels, 3.0);
    assert_eq!((p.at.as_deref(), p.away), (Some("the tip"), None), "every node is near one");
    let diagonal = vec![([1.7, 1.7, 1.7], 50.0, [0.0; 3]), ([1.8, 1.8, 1.8], 40.0, [0.0; 3])];
    let p = peaks(&diagonal, &[([0.0; 3], 0)], &labels, 3.0);
    assert_eq!(p.at.as_deref(), Some("the clamp"), "2.94 mm away is near");
    assert_eq!(p.away.map(|a| a.0), Some(40.0), "3.12 mm away is away");
    // Hostile: no points, a label index past the end, a distance that is not a number.
    assert_eq!(peaks(&[], &anchors, &labels, 3.0), Peaks { at: None, away: None, distance: 3.0 });
    assert_eq!(peaks(&points, &[([0.0; 3], 7)], &labels, f64::NAN).at, None);
}

/// U1 (part): equilibrium in numbers. Input: a reaction of 10 N up against 10 N down, then 9.95,
/// 9.9 and 9 N up; then two loads that cancel. Action: `Equilibrium::mismatch`, `balances`.
/// Pass: 0 %, 0.5 % and 1 % balance, 10 % does not; cancelling loads measure against a thousandth
/// of the forces' sizes, not zero (no division by zero, no NaN).
#[test]
fn equilibrium_measures_the_reaction_against_the_loads() {
    let e = |up: f64| Equilibrium { reaction: [0.0, 0.0, up], applied: [0.0, 0.0, -10.0], scale: 10.0, moment: None };
    assert!(e(10.0).mismatch() < 1e-12 && e(10.0).balances());
    assert!((e(9.95).mismatch() - 0.005).abs() < 1e-12 && e(9.95).balances());
    assert!(e(9.9).balances(), "1 % is within");
    assert!((e(9.0).mismatch() - 0.1).abs() < 1e-12 && !e(9.0).balances());
    let cancel = Equilibrium { reaction: [0.0, 0.0, 0.004], applied: [0.0; 3], scale: 20.0, moment: None };
    assert!((cancel.mismatch() - 0.2).abs() < 1e-12 && !cancel.balances());
    let zero = Equilibrium { reaction: [0.0; 3], applied: [0.0; 3], scale: 0.0, moment: None };
    assert!(zero.mismatch().is_finite() && zero.balances());
}

/// A7: the solver runs on one thread for everything, whatever the environment says: each thread
/// count gives its own last digits, and CalculiX 2.21's multithreaded SPOOLES solve is unreliable
/// (see `solver_threads`). Input: none. Action: `solver_threads`. Pass: both variables set to 1,
/// one thread reported.
#[test]
fn the_solver_runs_on_one_thread_so_every_computer_gets_the_same_numbers() {
    let (env, used) = solver_threads();
    let mut names: Vec<&str> = env.iter().map(|(k, _)| *k).collect();
    names.sort_unstable();
    assert_eq!(names, ["CCX_NPROC_EQUATION_SOLVER", "OMP_NUM_THREADS"]);
    assert!(env.iter().all(|(_, v)| v == "1"), "{env:?}");
    assert_eq!(used, 1);
}

/// A7: the programs' versions, read from the first lines they write.
/// Input: gmsh 4.12.1's first log line and CalculiX 2.21's banner, as they ran on zhikun; text
/// without them. Action: `mesher_version`, `solver_version`. Pass: "gmsh 4.12.1" and
/// "CalculiX 2.21"; `None` for the rest, without a panic.
#[test]
fn the_programs_versions_are_read_from_their_own_first_lines() {
    let gmsh = "Info    : Running 'gmsh part.step -3 -nt 1 -order 2 -clmax 1.5 -o solid-1.msh' [Gmsh 4.12.1, 1 node, max. 1 thread]\n\
                Info    : Started on Wed Nov 18 01:54:33 2026\n";
    assert_eq!(mesher_version(gmsh).as_deref(), Some("gmsh 4.12.1"));
    let ccx = "\n************************************************************\n\nCalculiX Version 2.21, Copyright(C) 1998-2023 Guido Dhondt\n";
    assert_eq!(solver_version(ccx).as_deref(), Some("CalculiX 2.21"));
    for junk in ["", "[Gmsh ", "CalculiX Version ", "Info : no version here", "[Gmsh é€"] {
        assert_eq!(mesher_version(junk), None, "{junk}");
        assert_eq!(solver_version(junk), None, "{junk}");
    }
}

/// A5: what each support carries, and the moment balance.
/// Input: a cantilever 100 mm long, two clamp nodes at x = 0 (z = ±0.5, so the clamp's moment is a
/// couple between them, as an expanded shell's top and bottom carry it), 10 N down at x = 100.
/// The solver's forces at the clamp: 5 N up each and ±1000 N along X. Action: `reactions`.
/// Observable: the support's force, its moment about its own middle (0, 0, 0), the balance.
/// Pass:
/// - 10 N up and −1000 N·mm about Y (the tip load's F × L = 10 × 100), balanced;
/// - without the couple (the `.dat` file's sums lose it) the moments miss by the whole moment;
/// - a load on a held node is taken off that support (2 N down on a clamp node: 12 N up);
/// - a node no support holds counts for none; a node two hold counts for the first.
#[test]
fn each_support_carries_its_force_and_moment_and_the_moments_balance() {
    let nodes = [[0.0, 0.0, 0.5], [0.0, 0.0, -0.5], [100.0, 0.0, 0.0]];
    let owner = [Some(0), Some(0), None];
    let labels = ["root".to_string()];
    let about = box_middle(&nodes);
    assert_eq!(about, [50.0, 0.0, 0.0]);
    let tip = [(2usize, [0.0, 0.0, -10.0])];
    let solver = |couple: f64, up: [f64; 2]| {
        vec![
            (nodes[0], [-couple, 0.0, up[0]], 0usize),
            (nodes[1], [couple, 0.0, up[1]], 1),
            // The load's own node: no support's.
            (nodes[2], [0.0, 0.0, -10.0], 2),
        ]
    };
    let (r, m) = reactions(&solver(1000.0, [5.0, 5.0]), &owner, &nodes, &tip, &labels, about);
    let near = |a: P, b: P| model::len(model::sub(a, b)) < 1e-9;
    assert_eq!(r.len(), 1);
    assert!(near(r[0].force, [0.0, 0.0, 10.0]), "{:?}", r[0]);
    assert!(near(r[0].moment, [0.0, -1000.0, 0.0]), "F × L about the clamp: {:?}", r[0]);
    assert!(near(r[0].about, [0.0, 0.0, 0.0]) && r[0].label == "root");
    assert!(m.mismatch() < 1e-12, "{m:?}");
    let e = Equilibrium { reaction: r[0].force, applied: [0.0, 0.0, -10.0], scale: 10.0, moment: Some(m) };
    assert!(e.balances());
    // The couple lost: the forces still balance, the moments miss by all of the clamp's.
    let (_, lost) = reactions(&solver(0.0, [5.0, 5.0]), &owner, &nodes, &tip, &labels, about);
    let e = Equilibrium { moment: Some(lost), ..e };
    assert!(e.forces_balance() && !e.moments_balance() && !e.balances(), "{lost:?}");
    assert!(lost.mismatch() > 0.3, "{lost:?}");
    // 2 N down on a clamp node: the solver's force there is the reaction less it.
    let loads = [(2usize, [0.0, 0.0, -10.0]), (0usize, [0.0, 0.0, -2.0])];
    let (r, m) = reactions(&solver(1000.0, [5.0, 5.0]), &owner, &nodes, &loads, &labels, about);
    assert!(near(r[0].force, [0.0, 0.0, 12.0]), "{:?}", r[0]);
    assert!(m.mismatch() < 1e-12, "{m:?}");
    // Two supports on one node: the first takes it; a second with no node carries nothing.
    let labels2 = ["root".to_string(), "also root".to_string()];
    let (r, _) = reactions(&solver(1000.0, [5.0, 5.0]), &owner, &nodes, &tip, &labels2, about);
    assert!(near(r[0].force, [0.0, 0.0, 10.0]) && near(r[1].force, [0.0; 3]), "{r:?}");
}

/// A5: the moment balance is measured against the moments that make it up, so supports that turn
/// the part both ways (a beam on two supports, its load in the middle: no moment about the middle
/// at all) are not called unbalanced over round-off.
/// Input: a 100 mm beam on two supports at its ends, 10 N down at its middle; the solver's 5 N up
/// at each end, then with 1e-6 N of round-off, then with one end's force 10 % short.
/// Pass: balanced, balanced, and the short one's force (and so its moment) misses.
#[test]
fn moments_are_measured_against_the_moments_that_make_them_up() {
    let nodes = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [50.0, 0.0, 0.0]];
    let owner = [Some(0), Some(1), None];
    let labels = ["left".to_string(), "right".to_string()];
    let about = box_middle(&nodes);
    let load = [(2usize, [0.0, 0.0, -10.0])];
    let at = |l: f64, r: f64| vec![(nodes[0], [0.0, 0.0, l], 0usize), (nodes[1], [0.0, 0.0, r], 1)];
    for (l, r, balanced) in [(5.0, 5.0, true), (5.0 + 1e-6, 5.0, true), (4.5, 5.0, false)] {
        let (_, m) = reactions(&at(l, r), &owner, &nodes, &load, &labels, about);
        assert_eq!(m.mismatch() <= EQUILIBRIUM_TOLERANCE, balanced, "{l} {r}: {m:?}");
    }
}

/// A5 on shells: the results' expanded nodes (top, middle and bottom, half the thickness apart)
/// each count for the shell node they came from; on solids a results node is the job's node of
/// the same number. Input: two shell nodes 1 mm apart, 1.5 mm thick, expanded nodes 0.75 mm above
/// and below each; a solid's three nodes. Action: `support_forces`. Pass: each expanded node maps
/// to its own shell node; zero forces are left out; solid ids map to index id − 1; no forces, none.
#[test]
fn an_expanded_shell_nodes_force_counts_for_its_own_shell_node() {
    let fem = model::Model { nodes: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]], shells: Vec::new(), tets: Vec::new(), on_face: Default::default() };
    let mut frd = formats::Frd::default();
    for (id, at, f) in [
        (11u64, [0.0, 0.0, 0.75], [1.0, 0.0, 0.0]),
        (12, [0.0, 0.0, -0.75], [-1.0, 0.0, 0.0]),
        (13, [1.0, 0.0, 0.75], [0.0, 0.0, 2.0]),
        (14, [1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        (15, [1.0, 0.0, -0.75], [0.0, 0.0, 3.0]),
    ] {
        frd.nodes.insert(id, at);
        frd.forces.insert(id, f);
    }
    let got = support_forces(&frd, &fem, Elements::Shell, 1.5, 1.0).unwrap_or_default();
    let owners: Vec<usize> = got.iter().map(|x| x.2).collect();
    assert_eq!(owners, [0, 0, 1, 1], "in id order, the zero force left out: {got:?}");
    let solid = model::Model { nodes: vec![[0.0; 3]; 3], ..fem };
    let mut frd = formats::Frd::default();
    for id in [1u64, 3, 9] {
        frd.nodes.insert(id, [0.0; 3]);
        frd.forces.insert(id, [1.0, 0.0, 0.0]);
    }
    let got = support_forces(&frd, &solid, Elements::Solid, 0.0, 1.0).unwrap_or_default();
    assert_eq!(got.iter().map(|x| x.2).collect::<Vec<_>>(), [0, 2], "id 9 is past the job's nodes");
    assert!(support_forces(&formats::Frd::default(), &solid, Elements::Solid, 0.0, 1.0).is_none());
}

/// A2: the mesher sizes elements from curvature (24 a turn) down to a fiftieth of the largest,
/// and straightens second-order elements; solids get a volume mesh, shells a surface one. A6: each
/// mesh check step refines the curvature sizing by the same 1.5 as the element size (36, then 54 a
/// turn), so a fillet's elements shrink with the rest.
#[test]
fn the_mesher_sizes_from_curvature_down_to_a_fiftieth_and_straightens_curved_elements() {
    let args = mesher_args(Elements::Solid, 2.5, 0, "solid-1.msh");
    let after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    assert_eq!(args.first().map(String::as_str), Some("part.step"));
    assert!(args.iter().any(|a| a == "-3") && !args.iter().any(|a| a == "-2"));
    assert_eq!(after("-nt").as_deref(), Some("1"), "one thread: the same mesh every time (A7)");
    assert_eq!(after("-order").as_deref(), Some("2"));
    assert_eq!(after("-clmax").as_deref(), Some("2.5"));
    assert_eq!(after("-clmin").as_deref(), Some("0.05"));
    assert_eq!(after("-clcurv").as_deref(), Some("24"));
    assert!(args.iter().any(|a| a == "-optimize_ho"));
    assert_eq!(after("-format").as_deref(), Some("msh22"));
    assert_eq!(after("-o").as_deref(), Some("solid-1.msh"));
    let shell = mesher_args(Elements::Shell, 3.0, 0, "shell-1.msh");
    assert!(shell.iter().any(|a| a == "-2") && !shell.iter().any(|a| a == "-3"));
    let curv = |level: u32| {
        let a = mesher_args(Elements::Solid, 2.5 / REFINEMENT_RATIO.powi(level as i32), level, "x.msh");
        a.iter().position(|x| x == "-clcurv").and_then(|i| a.get(i + 1)).cloned().unwrap_or_default()
    };
    assert_eq!([curv(0), curv(1), curv(2)], ["24", "36", "54"]);
    assert_eq!(curvature_elements(u32::MAX), curvature_elements(8), "a level past 8 is held there");
}

/// A3: the mesher's and the solver's known failures, as they print them (gmsh 4.12.1, CalculiX
/// 2.21), are read for what went wrong; the words say what to change and carry none of the
/// programs' own text.
#[test]
fn known_mesher_and_solver_failures_become_plain_advice_without_the_programs_text() {
    let failed = |code: Option<i32>, signal: Option<i32>, log: &str| Ended::Failed { code, signal, log: log.into() };
    let gmsh_overlap = "Info    : Meshing 3D...\nError   : Invalid boundary mesh (overlapping facets) on surface 7 surface 7\nError   : No elements in volume 1 \nWarning : Surface mesh: worst distortion = -1.57377\nError   : ------------------------------\n";
    assert_eq!(trouble(&failed(Some(1), None, gmsh_overlap)), Trouble::Overlap(Some(7)));
    assert_eq!(trouble(&failed(Some(1), None, "Error   : No elements in volume 1\n")), Trouble::Overlap(None));
    let ccx_jacobian = " *ERROR in e_c3d: nonpositive jacobian\n         determinant in element      1234\n";
    assert_eq!(trouble(&failed(Some(201), None, ccx_jacobian)), Trouble::InsideOut(Some(1234)));
    let banner = "************************************************************\n\nCalculiX Version 2.21\nCalculiX comes with ABSOLUTELY NO WARRANTY. This is free\nsoftware, and you are welcome to redistribute it under\ncertain conditions, see gpl.htm\n";
    assert_eq!(trouble(&failed(None, Some(9), banner)), Trouble::OutOfMemory, "killed: the system's out-of-memory killer");
    assert_eq!(trouble(&failed(Some(0xC000_0017_u32 as i32), None, "")), Trouble::OutOfMemory, "Windows: not enough memory");
    assert_eq!(trouble(&failed(Some(1), None, "*ERROR in u_calloc: error allocating memory\n")), Trouble::OutOfMemory);
    assert_eq!(trouble(&Ended::Timeout(1800)), Trouble::Timeout(1800));
    assert_eq!(trouble(&failed(Some(1), None, "Info : x\nError   : Unknown entity 3\n")), Trouble::Other(Some("Error   : Unknown entity 3".into())));
    assert_eq!(trouble(&failed(Some(1), None, banner)), Trouble::Other(None), "the banner is no reason");

    let near = |_: P| "a hole Ø1 mm".to_string();
    let words = |t: &Trouble, what: &str, at: Option<P>| {
        let mut f = Failure::of(t, what, "/usr/bin/prog", 2.5);
        f.at = at;
        f.words(near)
    };
    let overlap = words(&Trouble::Overlap(Some(7)), "mesher", Some([50.0, 10.0, 2.0]));
    assert!(overlap.starts_with("the mesher could not fill the part with elements near a hole Ø1 mm: "), "{overlap}");
    assert!(overlap.contains("about 2.5 mm here") && overlap.contains("smaller `mesh_size`, such as 1.3 mm"), "{overlap}");
    let inside = words(&Trouble::InsideOut(Some(1234)), "solver", Some([0.0; 3]));
    assert!(inside.starts_with("the mesh has an element turned inside out near a hole Ø1 mm, "), "{inside}");
    let unplaced = words(&Trouble::InsideOut(None), "solver", None);
    assert!(unplaced.starts_with("the mesh has an element turned inside out, at a tightly curved"), "{unplaced}");
    let memory = words(&Trouble::OutOfMemory, "solver", None);
    assert!(
        memory.starts_with("the solver ran out of memory on this computer with elements of about 2.5 mm") && memory.contains("such as 3.8 mm"),
        "{memory}"
    );
    let slow = words(&Trouble::Timeout(1800), "solver", None);
    assert!(slow.starts_with("the solver took longer than 30 minutes and was stopped"), "{slow}");
    let other = words(&Trouble::Other(None), "mesher", None);
    assert!(other.contains("gave no reason") && other.contains("keep_files: true"), "{other}");
    for w in [&overlap, &inside, &unplaced, &memory, &slow, &other] {
        for raw in ["gpl.htm", "WARRANTY", "*ERROR", "Error   :", "jacobian", "signal", "exit status", "facets"] {
            assert!(!w.contains(raw), "`{raw}` in: {w}");
        }
    }
    // Stop keeps its words.
    let (stop, _) = failure(Ended::Said(STOPPED.into()), "solver", "ccx", 1.0);
    assert_eq!(stop.words(near), STOPPED);
}

/// A3 and V-A3: a program the system kills (as Linux's out-of-memory killer does, signal 9) is
/// read as out of memory, and its banner stays out of the words. A shell that kills itself
/// stands in for the solver.
#[cfg(unix)]
#[test]
fn a_program_the_system_kills_reads_as_out_of_memory() {
    let dir = work_folder().unwrap_or_else(|e| panic!("{e}"));
    let args: Vec<String> = ["-c", "echo 'certain conditions, see gpl.htm'; kill -9 $$"].iter().map(|s| s.to_string()).collect();
    let ended = run_program("sh", &args, &[], &dir, "killed", 60, "solver", cancel_generation()).err();
    let _ = std::fs::remove_dir_all(&dir);
    let ended = ended.unwrap_or_else(|| panic!("the killed shell gave a result"));
    assert!(matches!(&ended, Ended::Failed { signal: Some(9), log, .. } if log.contains("gpl.htm")), "{ended:?}");
    let (f, t) = failure(ended, "solver", "sh", 1.0);
    assert_eq!(t, Trouble::OutOfMemory);
    let words = f.words(|_| String::new());
    assert!(words.starts_with("the solver ran out of memory on this computer") && !words.contains("gpl.htm"), "{words}");
}

/// A6: Richardson's extrapolation recovers the order and the limit of an exact series.
/// Input: f(h) = 100 + c h^p on h = 1, 1/1.5, 1/2.25 for p = 0.6, 1, 2, 3 and 3.9, rising (c < 0) and
/// falling (c > 0). Action: `convergence::estimate`. Pass: the observed order is p and the value 100
/// to 1e-9; the band is 1.25 times the finest value's distance from 100, and at least ±0.5 %; the
/// trend is settled when that band is within 3 % and changing when it is wider.
#[test]
fn richardson_recovers_a_known_order_and_its_limit() {
    use convergence::{Basis, Trend, estimate};
    let series = |c: f64, p: f64| [1.0, 1.0 / 1.5, 1.0 / 2.25].map(|h: f64| 100.0 + c * h.powf(p));
    for p in [0.6, 1.0, 2.0, 3.0, 3.9] {
        for c in [-4.0, 4.0, -20.0] {
            let v = series(c, p);
            let e = estimate(&v, 1.5).unwrap_or_else(|| panic!("p {p}"));
            let order = e.order.unwrap_or(f64::NAN);
            assert!((order - p).abs() < 1e-9 && (e.value - 100.0).abs() < 1e-9 && e.extrapolated(), "p {p} c {c}: {e:?}");
            assert_eq!((e.meshes, e.basis, e.finest), (3, Basis::Extrapolated, v[2]));
            let band = e.band().unwrap_or(f64::NAN);
            // 1.25 times the finest value's distance from the limit, and never under ±0.5 %.
            let want_band = (1.25 * (v[2] - 100.0).abs() / 100.0).max(convergence::MIN_BAND);
            assert!((band - want_band).abs() < 1e-12, "p {p} c {c}: {band}");
            let want = if band <= 0.03 { Trend::Settled } else { Trend::Changing };
            assert_eq!(e.trend, want, "p {p} c {c}: band {band}");
        }
    }
    // A quadratic fall from 92 to 100 (the review's Peterson fillet from below): settled within
    // about 1 %.
    let e = estimate(&series(-8.0, 2.0), 1.5).unwrap_or_else(|| panic!("quadratic"));
    assert!(e.trend == Trend::Settled && e.band().is_some_and(|b| (b - 0.0198).abs() < 1e-3), "{e:?}");
}

/// A6: a peak that keeps growing has no band, and is never settled.
/// Input: a sharp corner's peak, f = 100 h^-0.45 (the stress singularity of a right-angled inside
/// corner) on h = 1, 1/1.5, 1/2.25; a slow climb at order 0.3; the review's measured sharp bar,
/// 95.6, 102.7, 121.4 MPa; and a shrinking value at order 0.2. Action: `convergence::estimate`.
/// Pass: rising (falling for the last), no error, the finest value quoted, the order reported.
#[test]
fn richardson_calls_a_peak_that_keeps_growing_rising_with_no_band() {
    use convergence::{Basis, Trend, estimate};
    let h = [1.0, 1.0 / 1.5, 1.0 / 2.25];
    let corner = h.map(|h: f64| 100.0 * h.powf(-0.45));
    let slow = h.map(|h: f64| 100.0 - 10.0 * h.powf(0.3));
    for (v, order) in [(corner, -0.45), (slow, 0.3)] {
        let e = estimate(&v, 1.5).unwrap_or_else(|| panic!("{v:?}"));
        assert_eq!((e.trend, e.basis, e.error, e.value), (Trend::Rising, Basis::NoLimit, None, v[2]), "{v:?}");
        assert!(e.order.is_some_and(|p| (p - order).abs() < 1e-9) && e.band().is_none(), "{e:?}");
    }
    let measured = estimate(&[95.6, 102.7, 121.4], 1.5).unwrap_or_else(|| panic!("measured"));
    assert_eq!(measured.trend, Trend::Rising, "{measured:?}");
    let falling = h.map(|h: f64| 50.0 + 10.0 * h.powf(0.2));
    assert_eq!(estimate(&falling, 1.5).map(|e| e.trend), Some(Trend::Falling));
}

/// A6: with two meshes, the band is the conservative |f2 − f1| / (1.5 − 1) around the finer value.
/// Input: 100 then 101, 101.5 and 102; a fall from 100 to 99; a move of 0.1 %. Pass: the finer value
/// quoted, its band 2/101, 3/101.5 and 4/102: settled to 3 % (101.5 is at the edge), changing past
/// it; no order; the 0.1 % move's band is the ±0.5 % floor.
#[test]
fn with_two_meshes_the_band_is_twice_the_change() {
    use convergence::{Basis, Trend, estimate};
    let small = estimate(&[100.0, 100.1], 1.5).unwrap_or_else(|| panic!("small"));
    assert!(small.trend == Trend::Settled && small.band().is_some_and(|b| (b - 0.005).abs() < 1e-12), "{small:?}");
    for (finer, settled) in [(101.0, true), (101.5, true), (102.0, false), (99.0, true)] {
        let e = estimate(&[100.0, finer], 1.5).unwrap_or_else(|| panic!("{finer}"));
        assert_eq!((e.value, e.meshes, e.basis, e.order), (finer, 2, Basis::TwoMeshes, None), "{finer}");
        let band = e.band().unwrap_or(f64::NAN);
        assert!((band - 2.0 * (finer - 100.0f64).abs() / finer).abs() < 1e-12, "{finer}: {band}");
        assert_eq!(e.trend == Trend::Settled, settled, "{finer}: {e:?}");
    }
}

/// A6: series Richardson cannot read. Input: a swing (100, 103, 101.5); two steps of mesh noise
/// (100, 100.2, 100.1: each under 0.25 %); orders over 4: V-6's default study (108.32, 109.91,
/// 109.97 MPa, order 8.1) and 100, 160, 170 (order 4.4, the finer pair 10 apart); a value that does
/// not move at all; and inputs that are not a series. Pass: the swing is changing with the larger
/// step as its error and no order; the noise is settled within ±0.5 %; an order over 4 is judged
/// on the finer pair alone: V-6 settled at its finest value within ±0.5 % (the floor over twice its
/// 0.06 step), the other changing at ±11.8 %; the constant is settled at the ±0.5 % floor; fewer
/// than two or more than three values, a value that is not finite, or a ratio of 1 give none.
#[test]
fn richardson_refuses_an_order_it_cannot_read() {
    use convergence::{Basis, Trend, estimate};
    let swing = estimate(&[100.0, 103.0, 101.5], 1.5).unwrap_or_else(|| panic!("swing"));
    assert_eq!((swing.trend, swing.basis, swing.order, swing.value, swing.error), (Trend::Changing, Basis::Oscillating, None, 101.5, Some(3.0)));
    let noise = estimate(&[100.0, 100.2, 100.1], 1.5).unwrap_or_else(|| panic!("noise"));
    assert_eq!((noise.trend, noise.basis, noise.order, noise.value), (Trend::Settled, Basis::Noise, None, 100.1));
    assert!(noise.band().is_some_and(|b| (b - 0.005).abs() < 1e-12), "{noise:?}");
    let v6 = estimate(&[108.32, 109.91, 109.97], 1.5).unwrap_or_else(|| panic!("v6"));
    assert_eq!((v6.trend, v6.basis, v6.value), (Trend::Settled, Basis::FastOrder, 109.97));
    assert!(v6.order.is_some_and(|p| (p - 8.08).abs() < 0.01) && v6.band().is_some_and(|b| (b - 0.005).abs() < 1e-12), "{v6:?}");
    let fast = estimate(&[100.0, 160.0, 170.0], 1.5).unwrap_or_else(|| panic!("fast"));
    assert_eq!((fast.trend, fast.basis, fast.value, fast.error), (Trend::Changing, Basis::FastOrder, 170.0, Some(20.0)));
    let flat = estimate(&[10.0, 10.0, 10.0], 1.5).unwrap_or_else(|| panic!("flat"));
    assert!(flat.trend == Trend::Settled && flat.band().is_some_and(|b| (b - 0.005).abs() < 1e-12), "{flat:?}");
    let bad: [&[f64]; 5] = [&[], &[1.0], &[1.0, 2.0, 3.0, 4.0], &[1.0, f64::NAN], &[1.0, f64::INFINITY, 2.0]];
    for bad in bad {
        assert_eq!(estimate(bad, 1.5), None, "{bad:?}");
    }
    assert_eq!(estimate(&[1.0, 2.0], 1.0), None);
    assert_eq!(estimate(&[1.0, 2.0], f64::NAN), None);
}
