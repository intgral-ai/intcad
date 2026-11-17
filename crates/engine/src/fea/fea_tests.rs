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
        isolated: false,
        equilibrium: None,
        peaks: Peaks::default(),
        threads: 1,
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
    assert_eq!(r.err().as_deref(), Some(STOPPED));
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
    let e = |up: f64| Equilibrium { reaction: [0.0, 0.0, up], applied: [0.0, 0.0, -10.0], scale: 10.0 };
    assert!(e(10.0).mismatch() < 1e-12 && e(10.0).balances());
    assert!((e(9.95).mismatch() - 0.005).abs() < 1e-12 && e(9.95).balances());
    assert!(e(9.9).balances(), "1 % is within");
    assert!((e(9.0).mismatch() - 0.1).abs() < 1e-12 && !e(9.0).balances());
    let cancel = Equilibrium { reaction: [0.0, 0.0, 0.004], applied: [0.0; 3], scale: 20.0 };
    assert!((cancel.mismatch() - 0.2).abs() < 1e-12 && !cancel.balances());
    let zero = Equilibrium { reaction: [0.0; 3], applied: [0.0; 3], scale: 0.0 };
    assert!(zero.mismatch().is_finite() && zero.balances());
}

/// The solver's threads, only where they are unset (a set value is left alone): OMP_NUM_THREADS
/// gets the available parallelism; the equation solver stays on one thread, where CalculiX 2.21's
/// SPOOLES solve is reliable (see `solver_threads`).
#[test]
fn the_solver_gets_every_core_unless_the_threads_are_set_already() {
    let (env, used) = solver_threads();
    let available = std::thread::available_parallelism().map_or(1, |n| n.get());
    for (k, v) in &env {
        match *k {
            "OMP_NUM_THREADS" => assert_eq!(v, &available.to_string()),
            "CCX_NPROC_EQUATION_SOLVER" => assert_eq!(v, "1"),
            other => panic!("{other}"),
        }
        assert!(std::env::var(k).map_or(true, |x| x.trim().is_empty()), "{k} is set already and must be left alone");
    }
    assert!(used >= 1);
}
