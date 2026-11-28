use serde_json::{Value, json};

use super::*;
use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    // A dry run draws the app's one setup preview ([`PREVIEW_LOCK`]).
    let _preview = (id == CMD && p.get("dry_run").and_then(Value::as_bool) == Some(true)).then(hold_preview);
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A flat sheet `length` x 20 mm, 1.5 mm thick, from the origin along X: the body's name, and
/// the indices of its end faces at x = 0 and x = `length`.
fn strip(s: &mut Session, length: f64) -> (String, u64, u64) {
    strip_of(s, length, 20.0)
}

/// The same sheet, `width` wide (along Y).
fn strip_of(s: &mut Session, length: f64, width: f64) -> (String, u64, u64) {
    run(s, "sheet.manage_rules", json!({"name": "R1", "thickness": "1.5 mm", "bend_radius": "1.5 mm", "active": true}));
    run(s, "sketch.create", json!({"plane": "XY", "name": "Base"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [length, width]}));
    run(s, "sketch.finish", json!({}));
    run(s, "sheet.flange", json!({"sketch": "Base"}));
    let body = s.model.state().bodies.first().map(|b| b.name.clone()).unwrap_or_default();
    let faces = run(s, "model.faces", json!({"body": body}));
    let end = |x: f64| {
        faces["faces"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| {
                (f["centroid"][0].as_f64().unwrap_or(f64::NAN) - x).abs() < 1e-6 && f["plane_normal"][0].as_f64().is_some_and(|n| n.abs() > 0.99)
            })
            .and_then(|f| f["index"].as_u64())
            .unwrap_or_else(|| panic!("no end face at x = {x}: {faces}"))
    };
    (body, end(0.0), end(length))
}

fn aluminium() -> Value {
    json!({"name": "5052-H32", "youngs_modulus": 70300, "poisson_ratio": 0.33, "yield_strength": 193, "source": "test values"})
}

#[test]
fn a_setup_without_a_support_is_refused_before_anything_runs() {
    let mut s = Session::default();
    let (_, _, tip) = strip(&mut s, 100.0);
    for supports in [json!(null), json!([])] {
        let mut p = json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "material": aluminium()});
        if !supports.is_null() {
            p["supports"] = supports;
        }
        let e = s.execute(CMD, &p).unwrap_err().to_string();
        assert!(e.contains("support") && e.contains("free to move"), "{e}");
    }
    assert!(fea::shown(&s.document_id).is_none(), "nothing is painted");
}

#[test]
fn a_setup_without_a_load_a_sourced_material_or_a_known_face_is_refused() {
    let mut s = Session::default();
    let (_, root, tip) = strip(&mut s, 100.0);
    let base = || json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": aluminium()});
    let mut p = base();
    p["loads"] = json!([]);
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("load"));
    let mut p = base();
    p["material"]["source"] = json!("");
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("source"));
    let mut p = base();
    p.as_object_mut().map(|o| o.remove("material"));
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("where the numbers come from"));
    let mut p = base();
    p["supports"] = json!([{"face": "no such face"}]);
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("no such face"));
    let mut p = base();
    p["supports"] = json!([{"face": root, "type": "pinned"}]);
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("`fixed` (held still) or `roller`"));
    let mut p = base();
    p["loads"] = json!([{"face": root, "force": [0, 0, -10]}]);
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("also held by a support"));
    let mut p = base();
    p["loads"] = json!([{"face": tip, "force": [0, 0, 0]}]);
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("above 0 N"));
    let mut p = base();
    p["material"]["poisson_ratio"] = json!(0.7);
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("poisson_ratio"));
    assert!(fea::shown(&s.document_id).is_none());
}

/// U9: a material by grade name.
/// Input: the 100 mm strip; `material` as a grade's name (`"6061-T6"`, `{"name": "S235"}`, an
/// alias `"1.4301"`), as an unknown name, as `"6061"` without its temper, and left out on a body
/// assigned the generic `Aluminum`. Action: a dry run. Observable: the setup's material line, or
/// the error. Pass: a known grade runs with the table's numbers and source; an unknown or
/// temperless name is refused listing the grades; left out, the error names the aluminium
/// grades. Nothing is faked.
#[test]
fn a_material_by_grade_name_takes_the_tables_sourced_numbers() {
    let mut s = Session::default();
    let (body, root, tip) = strip(&mut s, 100.0);
    let p = |m: Value| json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": m, "dry_run": true});
    for (given, expect) in
        [(json!("6061-T6"), "6061-T6 aluminium"), (json!({"name": "S235"}), "S235 structural steel"), (json!("1.4301"), "304 stainless steel")]
    {
        let r = run(&mut s, CMD, p(given));
        let setup = r["setup"].to_string();
        assert!(setup.contains(expect), "{setup}");
    }
    for bad_name in ["unobtainium", "6061"] {
        let e = s.execute(CMD, &p(json!(bad_name))).unwrap_err().to_string();
        assert!(e.contains("6061-T6 aluminium") && e.contains("S355") && e.contains("source"), "{e}");
    }
    run(&mut s, "material.assign", json!({"bodies": [body], "material": "Aluminum"}));
    let mut q = p(json!(null));
    q.as_object_mut().map(|o| o.remove("material"));
    let e = s.execute(CMD, &q).unwrap_err().to_string();
    assert!(e.contains("this body is Aluminum") && e.contains("6082-T6") && !e.contains("S235"), "{e}");
}

/// Every grade of the table: numbers in range, a source that names a standard or a code, and
/// every name and alias finding its own grade (no two grades share one).
#[test]
fn every_grade_in_the_table_is_sourced_and_found_by_its_names() {
    for g in fea::materials::GRADES {
        assert!((1.0e3..=3.0e5).contains(&g.e_mpa) && g.nu > 0.0 && g.nu < 0.5 && g.yield_mpa > 50.0, "{}", g.name);
        assert!(["ASTM", "EN "].iter().any(|k| g.source.contains(k)), "{}: {}", g.name, g.source);
        for n in std::iter::once(g.name).chain(g.aliases.iter().copied()) {
            assert_eq!(fea::materials::grade(n).map(|x| x.name), Some(g.name), "{n}");
        }
    }
    assert!(fea::materials::grade("").is_none() && fea::materials::grade("steel").is_none());
}

/// The one setup preview is the whole app's (`fea::show_preview` replaces any other), and tests run
/// side by side: every dry run through [`run`] holds this while it draws, and a test that reads the
/// preview back holds it across its dry run and its reads ([`hold_preview`]). A dry run refused
/// before it draws (the tests that call `execute` themselves) needs none.
static PREVIEW_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

thread_local! {
    /// Whether this test's thread holds [`PREVIEW_LOCK`] already.
    static HOLDING_PREVIEW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// [`PREVIEW_LOCK`] until dropped, or nothing on a thread that holds it already.
struct PreviewHold(Option<std::sync::MutexGuard<'static, ()>>);

impl Drop for PreviewHold {
    fn drop(&mut self) {
        if self.0.is_some() {
            HOLDING_PREVIEW.with(|h| h.set(false));
        }
    }
}

fn hold_preview() -> PreviewHold {
    if HOLDING_PREVIEW.with(std::cell::Cell::get) {
        return PreviewHold(None);
    }
    let guard = PREVIEW_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    HOLDING_PREVIEW.with(|h| h.set(true));
    PreviewHold(Some(guard))
}

/// U10: the setup preview.
/// Input: the 100 mm strip held at x = 0, 10 N down at x = 100, as a roller-free setup. Action: a
/// dry run, then `inspect.stress_estimate.show {show: clear}`. Observable: `fea::preview` for the
/// design. Pass: the dry run leaves a preview with one load arrow (10 N, pointing down) at the tip
/// and one support labelled `fixed` at the root, and nothing painted; clear drops it. Nothing is
/// faked.
#[test]
fn a_dry_run_draws_its_supports_and_loads_until_cleared() {
    let _one = hold_preview();
    let mut s = Session::default();
    let (body, root, tip) = strip(&mut s, 100.0);
    assert!(fea::preview(&s.document_id).is_none());
    run(&mut s, CMD, json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": "6061-T6", "dry_run": true}));
    let pv = fea::preview(&s.document_id).unwrap_or_else(|| panic!("no preview"));
    assert_eq!(pv.body, body);
    assert_eq!(pv.loads.len(), 1);
    assert!((pv.loads[0].at[0] - 100.0).abs() < 1e-6 && pv.loads[0].dir == Some([0.0, 0.0, -1.0]) && pv.loads[0].text == "10.0 N", "{:?}", pv.loads);
    assert_eq!(pv.supports.len(), 1);
    assert!(pv.supports[0].at[0].abs() < 1e-6 && pv.supports[0].text == "fixed", "{:?}", pv.supports);
    assert!(pv.size > 100.0);
    assert!(fea::shown(&s.document_id).is_none(), "a dry run paints no result");
    run(&mut s, SHOW, json!({"show": "clear"}));
    assert!(fea::preview(&s.document_id).is_none());
}

/// The skill's first worked setup (`skills/solidcraft-analysis/reference.md`) is what agents copy:
/// its JSON parses and its faces, picked by point, resolve on the strip it describes.
#[test]
fn the_analysis_skills_worked_strip_setup_resolves() {
    let doc = include_str!("../../../../skills/solidcraft-analysis/reference.md");
    let block = doc.split("```json").nth(1).and_then(|b| b.split("```").next()).unwrap_or_else(|| panic!("no json block"));
    let call: Value = serde_json::from_str(block).unwrap_or_else(|e| panic!("{e}: {block}"));
    assert_eq!(call["command"], CMD);
    let mut s = Session::default();
    strip(&mut s, 100.0);
    let r = run(&mut s, CMD, call["params"].clone());
    let setup = r["setup"].to_string();
    assert!(setup.contains("6061-T6") && setup.contains("10.0 N"), "{setup}");
}

#[test]
fn showing_with_no_estimate_says_so() {
    let mut s = Session::default();
    let e = s.execute(SHOW, &json!({"show": "deflection"})).unwrap_err().to_string();
    assert!(e.contains("no stress estimate"), "{e}");
    assert!(s.execute(SHOW, &json!({"show": "colour"})).is_err());
    assert!(s.execute(SHOW, &json!({"show": "hide"})).is_err(), "nothing to hide");
    assert_eq!(run(&mut s, SHOW, json!({"show": "clear"}))["shown"], "clear");
}

#[test]
fn a_dry_run_describes_the_setup_in_plain_words_and_solves_nothing() {
    let mut s = Session::default();
    let (_, root, tip) = strip(&mut s, 100.0);
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": aluminium(), "dry_run": true}),
    );
    assert!(r["runs"].as_array().is_some_and(Vec::is_empty), "{r}");
    let setup: Vec<String> = r["setup"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
    let all = setup.join("\n");
    for needed in ["Load: 10.0 N down (−Z)", "Held still:", "Material: 5052-H32", "thin shells"] {
        assert!(all.contains(needed), "`{needed}` missing: {all}");
    }
    assert!(!all.contains("F2:face") && !all.contains("[0.000"), "no internal ids or raw vectors: {all}");
    assert!(fea::shown(&s.document_id).is_none(), "a dry run paints nothing");
}

#[test]
fn every_reply_tells_the_agent_how_to_report_without_verdicts_or_made_up_numbers() {
    let mut s = Session::default();
    let (_, root, tip) = strip(&mut s, 100.0);
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": aluminium(), "dry_run": true}),
    );
    let rules = r["how_to_report"].as_array().cloned().unwrap_or_default();
    let rules: Vec<&str> = rules.iter().filter_map(Value::as_str).collect();
    let all = rules.join(" ");
    for needed in [
        "do not talk about verdicts at all",
        "do not give a safety factor",
        "Only quote numbers this estimate computed",
        "run the estimate again",
        "where they came from",
    ] {
        assert!(all.contains(needed), "`{needed}` missing from how_to_report: {all}");
    }
    let doc = crate::find_command(CMD).map(|c| c.params).unwrap_or_default().to_lowercase();
    assert!(!doc.contains("can check") && !doc.contains("checks the part"), "the command never promises a check: {doc}");
}

/// An L bracket, 1.5 mm, with two Ø6.6 holes in its base: the faces of the first hole.
fn angle(s: &mut Session) -> Vec<u64> {
    run(s, "sheet.manage_rules", json!({"name": "R1", "thickness": "1.5 mm", "bend_radius": "1.5 mm", "active": true}));
    run(s, "sketch.create", json!({"plane": "XY", "name": "Base"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [60, 40]}));
    run(s, "sketch.finish", json!({}));
    run(s, "sheet.flange", json!({"sketch": "Base"}));
    run(s, "sheet.flange", json!({"edges": [[30, 40, 1.5]], "height": 40}));
    run(s, "solid.hole", json!({"position": [15, 15, 1.5], "diameter": 6.6}));
    run(s, "solid.hole", json!({"position": [45, 15, 1.5], "diameter": 6.6}));
    let body = s.model.state().bodies.first().map(|b| b.name.clone()).unwrap_or_default();
    let faces = run(s, "model.faces", json!({"body": body}));
    faces["faces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["cylinder"]["radius"].as_f64().is_some_and(|r| (r - 3.3).abs() < 1e-6) && f["centroid"][0].as_f64().is_some_and(|x| x < 30.0))
        .filter_map(|f| f["index"].as_u64())
        .collect()
}

#[test]
fn half_a_hole_takes_the_whole_hole_and_a_point_names_a_face() {
    let mut s = Session::default();
    let hole = angle(&mut s);
    assert!(!hole.is_empty(), "the first hole's faces");
    let base_top = [5.0, 5.0, 1.5];
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": base_top, "force": [0, 0, -5]}], "supports": [{"face": hole[0]}], "material": aluminium(), "dry_run": true}),
    );
    let setup = r["setup"].to_string();
    assert!(setup.contains("Held still: a hole Ø6.60"), "{setup}");
    if hole.len() > 1 {
        assert!(setup.contains("took the whole of each hole"), "{setup}");
    }
    assert!(setup.contains("the base, top side"), "the point on the base names its face: {setup}");
    let e = s
        .execute(CMD, &json!({"loads": [{"face": [500, 500, 500], "force": [0, 0, -5]}], "supports": [{"face": hole[0]}], "material": aluminium(), "dry_run": true}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("within 1 mm"), "{e}");
}

#[test]
fn a_solid_part_defaults_to_solid_elements_and_shells_on_it_need_a_thickness() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 60, "width": 20, "height": 10, "body_name": "Block"}));
    let faces = run(&mut s, "model.faces", json!({"body": "Block"}));
    let end = |x: f64| {
        faces["faces"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| (f["centroid"][0].as_f64().unwrap_or(f64::NAN) - x).abs() < 1e-6)
            .and_then(|f| f["index"].as_u64())
            .unwrap_or(0)
    };
    let p =
        json!({"loads": [{"face": end(60.0), "force": [0, 0, -100]}], "supports": [{"face": end(0.0)}], "material": aluminium(), "dry_run": true});
    let r = run(&mut s, CMD, p.clone());
    assert!(r["setup"].to_string().contains("solid elements"), "{r}");
    assert!(r["setup"].to_string().contains("the face toward +X") && r["setup"].to_string().contains("the face toward −X"), "plain face words: {r}");
    let mut shells = p.clone();
    shells["elements"] = json!("shell");
    let e = s.execute(CMD, &shells).unwrap_err().to_string();
    assert!(e.contains("thickness") && e.contains("solid is the default"), "{e}");
    shells["thickness"] = json!(10);
    let r = run(&mut s, CMD, shells);
    assert!(r["warnings"].to_string().contains("not a thin sheet"), "{r}");
}

#[test]
fn directions_and_sheet_faces_read_as_words() {
    assert_eq!(direction_words([0.0, -50.0, 0.0]), "along −Y");
    assert_eq!(direction_words([0.0, 0.0, -1.0]), "down (−Z)");
    assert_eq!(direction_words([3.0, 4.0, 0.0]), "along (0.600, 0.800, 0.000)");
    assert_eq!(sheet_words("sheet:Body1:base:top"), "the base, top side");
    assert_eq!(sheet_words("sheet:Body1:F3.0:rim.far"), "flange 1, far edge");
    assert_eq!(sheet_words("sheet:Body1:base:rim.l2"), "the base, edge");
}

fn fake_run(elements: Elements, stress: f64, disp: f64) -> fea::Run {
    fea::Run {
        elements,
        mesh_size: 2.0,
        nodes: 100,
        element_count: 40,
        max_stress: stress,
        max_stress_at: [0.0; 3],
        max_disp: disp,
        max_disp_at: [100.0, 0.0, 0.0],
        seconds: 1.0,
        points: Vec::new(),
        tensors: Vec::new(),
        cells: Vec::new(),
        isolated: true,
        equilibrium: None,
        reactions: Vec::new(),
        peaks: fea::Peaks { at: None, away: Some((stress, [0.0; 3])), distance: 3.0 },
        threads: 1,
        mesher_version: None,
        solver_version: None,
    }
}

/// A solve whose highest stress `max` sits at `at` (a support's label) or not, with `away` the
/// highest away from supports and loads.
fn peaked_run(max: f64, at: Option<&str>, away: Option<f64>, disp: f64) -> fea::Run {
    let mut r = fake_run(Elements::Solid, max, disp);
    r.peaks = fea::Peaks { at: at.map(str::to_string), away: away.map(|a| (a, [50.0, 0.0, 0.0])), distance: 3.0 };
    r
}

/// Words that would read as a verdict. The estimate says how far from yield, never these.
const VERDICTS: &[&str] = &["pass", "fail", "safe", "approved", "acceptable", "verdict", "compliant", "factor of safety", "safety factor"];

#[test]
fn the_words_say_estimate_and_how_far_from_yield_never_pass_or_fail() {
    let material = Material { name: "5052-H32".into(), e_mpa: 70300.0, nu: 0.33, yield_mpa: 193.0, source: "a handbook".into() };
    let loads = [Load { label: "tip".into(), faces: vec![1], kind: LoadKind::Force([0.0, 0.0, -10.0]) }];
    let supports = [Support { label: "root".into(), faces: vec![0], kind: SupportKind::Fixed }];
    let near = |p: P| if p[0] > 50.0 { "the tip".to_string() } else { "the root".to_string() };
    // Below, near and well over the yield strength: the same kind of words each time.
    for stress in [20.0, 190.0, 900.0] {
        let done = [fake_run(Elements::Shell, stress, 8.0), fake_run(Elements::Solid, stress * 0.9, 7.5)];
        let outcome = Outcome { done: &done, shown: &done[0], check: &Check::Off };
        let (summary, assumptions, both) = wording(&material, &loads, &supports, &outcome, 1.5, &near);
        let all = [summary.clone(), assumptions.join("\n"), both.clone().unwrap_or_default()].join("\n").to_lowercase();
        for w in VERDICTS {
            assert!(!all.contains(w), "`{w}` in: {all}");
        }
        assert!(summary.starts_with("Estimate"), "{summary}");
        assert!(summary.contains("% of the yield strength given"), "{summary}");
        assert!(both.is_some_and(|b| b.contains("Shell vs solid")));
        for needed in [
            "5052-H32",
            "a handbook",
            "given with the request",
            "mm, N, MPa",
            "thin shells",
            "solid elements",
            "mesh about",
            "Load: 10",
            "Held still: root",
        ] {
            assert!(assumptions.iter().any(|a| a.contains(needed)), "`{needed}` missing: {assumptions:?}");
        }
    }
}

/// U1 (part): the equilibrium line and its warning.
/// Input: a reaction of 10 N up against 10 N down; 9.95 N (0.5 % short); 9.9 N (1 %); 9 N (10 %).
/// Action: `equilibrium_line`, `equilibrium_warning`. Observable: the line's words; a warning or
/// none. Pass: "Supports carry 10.0 N; loads applied 10.0 N"; no warning up to 1 % apart; at 10 %
/// a warning that says the result is suspect and why. Nothing runs.
#[test]
fn the_reaction_is_quoted_against_the_loads_and_a_gap_over_one_percent_is_a_warning() {
    let e = |up: f64| fea::Equilibrium { reaction: [0.0, 0.0, up], applied: [0.0, 0.0, -10.0], scale: 10.0, moment: None };
    assert_eq!(equilibrium_line(&e(10.0)), "Supports carry 10.0 N; loads applied 10.0 N");
    for balanced in [10.0, 9.95, 9.9] {
        assert_eq!(equilibrium_warning(&e(balanced)), None, "{balanced} N");
    }
    let w = equilibrium_warning(&e(9.0)).unwrap_or_default();
    for needed in ["9.00 N", "10.0 N", "10.0% apart", "more than 1.00%", "suspect", "do not balance"] {
        assert!(w.contains(needed), "`{needed}` missing: {w}");
    }
}

/// A5: the moments in the equilibrium line and its warning, and one support's line.
/// Input: 10 N against 10 N with moments about the part's middle that cancel, then 20 % apart; a
/// clamp carrying 10 N up (with 1e-11 N of round-off across) and 1000 N·mm about −Y, its nodes'
/// middle at (0, 10, 0.75) mm. Action: `equilibrium_line`, `equilibrium_warning`, `support_json`.
/// Pass: "…; their moments balance too" and no warning; then "…; their moments are 20.0% apart"
/// and a warning naming both moments, 20 %, "suspect"; the support's line gives its force, moment,
/// their components (round-off and −0 read as 0) and where; its numbers rounded.
#[test]
fn moments_join_the_equilibrium_line_and_each_support_says_what_it_carries() {
    let m = |miss: f64| fea::MomentBalance {
        about: [50.0, 10.0, 0.75],
        reaction: [0.0, -500.0 * (1.0 - miss), 0.0],
        applied: [0.0, 500.0, 0.0],
        scale: 1000.0,
    };
    let e = |miss: f64| fea::Equilibrium { reaction: [0.0, 0.0, 10.0], applied: [0.0, 0.0, -10.0], scale: 10.0, moment: Some(m(miss)) };
    assert_eq!(equilibrium_line(&e(0.0)), "Supports carry 10.0 N; loads applied 10.0 N; their moments balance too");
    assert_eq!(equilibrium_warning(&e(0.0)), None);
    assert_eq!(equilibrium_line(&e(0.4)), "Supports carry 10.0 N; loads applied 10.0 N; their moments are 20.0% apart");
    let w = equilibrium_warning(&e(0.4)).unwrap_or_default();
    for needed in ["moment", "300 N·mm", "500 N·mm", "20.0% of the moments apart", "more than 1.00%", "suspect"] {
        assert!(w.contains(needed), "`{needed}` missing: {w}");
    }
    let r = fea::Reaction {
        label: "face 2 (flat, 20 × 1.5 mm)".into(),
        force: [1e-11, 0.0, 10.0],
        moment: [0.0, -1000.0, -0.0],
        about: [0.0, 10.0, 0.75],
    };
    let j = serde_json::to_value(support_json(&r, 100.0)).unwrap_or_default();
    assert_eq!(j["line"], "face 2 (flat, 20 × 1.5 mm) carries 10.0 N (0, 0, 10) and a moment of 1000 N·mm (0, -1000, 0) about (0, 10, 0.75) mm");
    assert_eq!(j["force_n"], json!([0.0, 0.0, 10.0]));
    assert_eq!(j["moment_nmm"], json!([0.0, -1000.0, 0.0]));
    assert_eq!((j["force_size_n"].as_f64(), j["moment_size_nmm"].as_f64()), (Some(10.0), Some(1000.0)));
    assert_eq!(j["about_mm"], json!([0.0, 10.0, 0.75]));
}

/// A number in a sentence keeps about three figures when it rounds up into the next decade: a
/// solid clamp's 9.99999 N read "10.00 N" beside the loads' "10.0 N".
#[test]
fn numbers_in_sentences_keep_three_figures_across_a_decade() {
    for (x, words) in [(9.99999, "10.0"), (99.96, "100"), (0.99996, "1.00"), (-9.99999, "-10.0"), (9.994, "9.99"), (123.4, "123"), (0.5, "0.500")] {
        assert_eq!(sig(x), words, "{x}");
    }
    assert_eq!(sig(f64::NAN), "?");
}

/// U3: the mesh check's statuses and bands, and what they mean (PIL-294 A6).
/// Input: fake runs (no programs): the overall peak 100 MPa at "the clamp", the peak away from it
/// 50 MPa, deflection 8 mm; then finer runs where the away peak moves 1 % (two meshes), 2 % (two
/// meshes, the third not solved), converges at order 2 to 52 MPa over three meshes, or keeps
/// rising (50, 55, 62 MPa). Action: `settling`, `third_wanted`, `check_warnings`, `check_line`,
/// `wording`, `lead_quoted`. Observable: each quantity's status, quote and band; the warnings; the
/// summary's lead number and share of yield.
/// Pass: two meshes and 1 %: settled within ±2.0 % (twice the change), no third mesh wanted; 1.2 %
/// wants one; a peak at the clamp never settles and wants none; 2 % on two meshes is still changing
/// within ±3.9 % with the sharp-corner warning; the order-2 series is settled about 52.0 MPa
/// within ±0.9 %, and the summary leads with it and shares 21.7 % of yield; the rising series has
/// no band and is called "keeps rising"; the clamp's peak gets the depends-on-the-mesh warning
/// only when it moved and the away peak settled; skipped or off says the bands are unknown.
#[test]
fn the_mesh_check_gives_each_number_a_band_and_flags_a_peak_held_up_by_a_support() {
    let near = |_: P| "the top".to_string();
    let first = peaked_run(100.0, Some("the clamp"), Some(50.0), 8.0);
    let two = |finer: &fea::Run| Check::Ran { meshes: vec![1.5, 1.0], items: settling(&[&first, finer]), third: Third::Ran };
    let warnings = |finer: &fea::Run| check_warnings(&two(finer), finer, &near).join("\n");
    // Everything 1 % on two meshes: settled within twice the change; no warnings; no third mesh.
    let calm = peaked_run(101.0, Some("the clamp"), Some(50.5), 8.08);
    let items = settling(&[&first, &calm]);
    assert_eq!(items.iter().map(|s| s.quantity).collect::<Vec<_>>(), [OVERALL, AWAY, DEFLECTION]);
    for s in &items {
        // The overall peak sits at the clamp, so it is never settled and has no band.
        if s.quantity == OVERALL {
            assert!(s.status() == "at a support or load: depends on the mesh" && s.band().is_none() && !s.settled(), "{s:?}");
        } else {
            assert!((s.change() - 1.0).abs() < 1e-9 && s.settled() && s.status() == "settled", "{s:?}");
            assert!(s.band().is_some_and(|b| (b - 0.02 / 1.01).abs() < 1e-12) && s.quote().ends_with("within ±2.0 %"), "{}", s.quote());
        }
    }
    assert_eq!(warnings(&calm), "");
    assert_eq!(third_wanted(&items), Err(Third::NotNeeded { quantity: AWAY, moved: items[1].change() }));
    assert_eq!(third_wanted(&settling(&[&first, &peaked_run(101.0, Some("the clamp"), Some(50.6), 8.0)])), Ok(()), "1.2 % wants a third");
    let at_clamp = |max: f64| peaked_run(max, Some("the clamp"), None, 8.0);
    assert_eq!(third_wanted(&settling(&[&at_clamp(100.0), &at_clamp(130.0)])), Err(Third::AtSupport));
    // The peak at the clamp moves 12 %, the away peak 1 %: the singular-support warning.
    let singular = peaked_run(112.0, Some("the clamp"), Some(50.5), 8.08);
    let w = warnings(&singular);
    for needed in
        ["The value at the clamp depends on the mesh", "100 → 112 MPa", "Lead with the stress away from it: about 50.5 MPa, within ±2.0 %", "settled"]
    {
        assert!(w.contains(needed), "`{needed}` missing: {w}");
    }
    // The same peak, not at a support: no singular-support warning.
    assert!(!warnings(&peaked_run(112.0, None, Some(50.5), 8.08)).contains("depends on the mesh"));
    // At the clamp, the away peak 2 % on two meshes (its third not solved): still changing within
    // ±3.9 %, the sharp-corner warning, no singular-support one.
    let w = warnings(&peaked_run(112.0, Some("the clamp"), Some(51.0), 8.08));
    for needed in ["has not settled over two meshes (50.0 → 51.0 MPa)", "about 51.0 MPa, within ±3.9 %", "sharp inside corner", "fillet"] {
        assert!(w.contains(needed), "`{needed}` missing: {w}");
    }
    assert!(!w.contains("depends on the mesh"), "{w}");
    // A deflection still changing says so.
    assert!(
        warnings(&peaked_run(101.0, Some("the clamp"), Some(50.5), 9.0))
            .contains("The largest deflection has not settled over two meshes (8.00 → 9.00 mm)")
    );

    // Three meshes, the away peak converging at order 2 to 52 MPa: extrapolated, settled.
    let quad = |h: f64| 52.0 - 2.0 * h * h;
    let (b, c) = (1.0 / 1.5, 1.0 / 2.25);
    let second = peaked_run(110.0, Some("the clamp"), Some(quad(b)), 8.01);
    let third = peaked_run(118.0, Some("the clamp"), Some(quad(c)), 8.012);
    let items = settling(&[&first, &second, &third]);
    let three = Check::Ran { meshes: vec![1.5, 1.0, 1.0 / 1.5], items: items.clone(), third: Third::Ran };
    let away = items.iter().find(|s| s.quantity == AWAY).unwrap_or_else(|| panic!("{items:?}"));
    let e = away.estimate().unwrap_or_else(|| panic!("{away:?}"));
    assert!(e.extrapolated() && (e.value - 52.0).abs() < 1e-9 && e.order.is_some_and(|p| (p - 2.0).abs() < 1e-9), "{e:?}");
    assert!(away.settled() && away.quote() == "about 52.0 MPa, within ±0.9 %", "{}", away.quote());
    let deflection = items.iter().find(|s| s.quantity == DEFLECTION).unwrap_or_else(|| panic!("{items:?}"));
    assert!(deflection.settled() && deflection.status() == "settled", "{deflection:?}");
    assert!(check_warnings(&three, &third, &near).join("\n").contains("The value at the clamp depends on the mesh"));
    let line = check_line(&three);
    assert!(
        line.starts_with("meshes 1.50 → 1.00 → 0.667 mm: highest stress 100 → 110 → 118 MPa (at a support or load: depends on the mesh); "),
        "{line}"
    );
    assert!(line.contains("highest stress away from supports and loads 50.0 → 51.1 → 51.6 MPa: about 52.0 MPa, within ±0.9 % (settled)"), "{line}");
    let (lead, item) = lead_quoted(&third, &three);
    assert!((lead - 52.0).abs() < 1e-9 && item.is_some_and(|s| s.quantity == AWAY), "{lead}");
    let material = Material { name: "6061-T6".into(), e_mpa: 69000.0, nu: 0.33, yield_mpa: 240.0, source: "a handbook".into() };
    let done = [first.clone_run()];
    let (summary, assumptions, _) = wording(&material, &[], &[], &Outcome { done: &done, shown: &third, check: &three }, 1.5, &near);
    for needed in [
        "away from the supports and loads, the highest von Mises stress is about 52.0 MPa (within ±0.9 %)",
        "that is about 21.7% of the yield strength given",
        "The highest overall, 118 MPa on the finest mesh, sits at the clamp",
        "Mesh check: meshes 1.50 → 1.00 → 0.667 mm",
    ] {
        assert!(summary.contains(needed), "`{needed}` missing: {summary}");
    }
    assert!(summary.find("52.0").unwrap_or(usize::MAX) < summary.find("118").unwrap_or(0), "the away peak comes first: {summary}");
    assert!(assumptions.iter().any(|a| a.starts_with("Shown: the mesh check's finest mesh") && a.contains("extrapolated")), "{assumptions:?}");

    // Three meshes, the away peak rising 50, 55, 62: no band, keeps rising, the finest quoted.
    let rising = Check::Ran {
        meshes: vec![1.5, 1.0, 1.0 / 1.5],
        items: settling(&[&first, &peaked_run(110.0, Some("the clamp"), Some(55.0), 8.0), &peaked_run(118.0, Some("the clamp"), Some(62.0), 8.0)]),
        third: Third::Ran,
    };
    let shown = peaked_run(118.0, Some("the clamp"), Some(62.0), 8.0);
    let w = check_warnings(&rising, &shown, &near).join("\n");
    assert!(w.contains("keeps rising as the mesh is refined (50.0 → 55.0 → 62.0 MPa)") && w.contains("sharp inside corner"), "{w}");
    assert!(check_line(&rising).contains("50.0 → 55.0 → 62.0 MPa (keeps rising: likely a sharp inside corner or an idealised support)"));
    let (summary, _, _) = wording(&material, &[], &[], &Outcome { done: &done, shown: &shown, check: &rising }, 1.5, &near);
    assert!(summary.contains("62.0 MPa on the finest mesh, and it keeps rising: likely a sharp inside corner"), "{summary}");
    assert_eq!(lead_quoted(&shown, &rising).0, 62.0);

    // Why there was no third mesh, in the line.
    let not_needed = Check::Ran { meshes: vec![1.5, 1.0], items: settling(&[&first, &calm]), third: Third::NotNeeded { quantity: AWAY, moved: 1.0 } };
    assert!(check_line(&not_needed).ends_with("no third mesh was needed: the highest stress away from supports and loads moved +1.0%, within 1%"));
    let over = Third::Skipped { nodes: 600_000.0, bytes: 1.4e10, limit: Limit::Memory(6.0e9) };
    let over_line = check_line(&Check::Ran { meshes: vec![1.5, 1.0], items: settling(&[&first, &calm]), third: over });
    assert!(
        over_line.contains("the third mesh (0.667 mm) was not solved, so the bands rest on two meshes: it would have about 600,000 nodes"),
        "{over_line}"
    );
    // Skipped or off: unknown, no warnings.
    let skipped = Check::Skipped { nodes: 673_000.0, bytes: 1.08e10, limit: Limit::Memory(6.0e9) };
    for check in [skipped.clone(), Check::Off] {
        assert!(check_warnings(&check, &singular, &near).is_empty());
        assert!(check_line(&check).contains("unknown"), "{}", check_line(&check));
        assert_eq!(lead_quoted(&singular, &check), (50.5, None));
    }
    assert!(check_line(&skipped).contains("about 670,000 nodes (about 11 GB)"), "{}", check_line(&skipped));
    // A peak at the clamp that moved only 1 % is still never called settled in the words: one step
    // of refinement misses its slow climb (R1's clamp: +0.4 % from 1.5 to 1.0 mm, yet 151 MPa at
    // 0.35 mm against 145).
    let (summary, _, _) = wording(&material, &[], &[], &Outcome { done: &done, shown: &calm, check: &two(&calm) }, 1.5, &near);
    assert!(summary.contains("it went 100 → 101 MPa with the mesh, but a value at an idealised support or load can keep creeping up"), "{summary}");
    assert!(!summary.contains("it settled"), "{summary}");
}

/// A fake run copied (fea::Run is not Clone: it can hold a million nodes).
trait CloneRun {
    fn clone_run(&self) -> fea::Run;
}
impl CloneRun for fea::Run {
    fn clone_run(&self) -> fea::Run {
        let mut r = fake_run(self.elements, self.max_stress, self.max_disp);
        r.peaks = self.peaks.clone();
        r.equilibrium = self.equilibrium;
        r.reactions = self.reactions.clone();
        r
    }
}

/// U4: setups the estimate refuses before anything runs.
/// Input: a strip (sheet metal), an L bracket with a hole, and a block with a slanted face, built
/// with the app's own commands; supports of type roller on a curved face (the hole) and on a flat
/// face that faces along no axis; pressures given as text without a unit, as a word, as a list,
/// as zero and together with a force; `mesh_check` as text. Action: `inspect.stress_estimate`
/// with `dry_run: true` (nothing would run anyway: each is refused while the setup is read).
/// Observable: the error text; nothing painted. Pass: each refusal names what is wrong; a roller
/// on the strip's end face (facing X) and a pressure in "250 kPa" or -5 (MPa) are accepted and
/// described in words.
#[test]
fn rollers_on_tilted_or_curved_faces_and_pressures_without_units_are_refused_before_anything_runs() {
    let mut s = Session::default();
    let (_, root, tip) = strip(&mut s, 100.0);
    let base = || json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": aluminium(), "dry_run": true});
    // A roller on the end face (facing X): held along X only.
    let mut p = base();
    // With a third roller on the side face (facing Y), so the three hold the strip still.
    p["supports"] = json!([{"face": root, "type": "roller"}, {"face": [50, 10, 0], "type": "symmetry"}, {"face": [50, 0, 0.75], "type": "roller"}]);
    let r = run(&mut s, CMD, p);
    let setup = r["setup"].to_string();
    assert!(setup.contains("Held along X only (a roller") && setup.contains("Held along Z only") && setup.contains("Held along Y only"), "{setup}");
    // Pressures: with a unit or as a number of MPa, described; otherwise refused.
    let mut p = base();
    p["loads"] = json!([{"face": tip, "pressure": "250 kPa"}, {"face": [50, 10, 1.5], "type": "pressure", "pressure": -5}]);
    let setup = run(&mut s, CMD, p)["setup"].to_string();
    assert!(setup.contains("Pressure: 0.250 MPa on") && setup.contains("pushing into the part"), "{setup}");
    assert!(setup.contains("Pressure: -5.00 MPa on") && setup.contains("pulling out of the part"), "{setup}");
    for (pressure, needed) in [
        (json!("10"), "has no unit"),
        (json!("lots"), "not a number with a unit"),
        (json!("10 furlongs"), "not known"),
        (json!([1, 2]), "must be a number in MPa"),
        (json!(0), "above 0"),
        (json!(1e9), "at most 10000 MPa"),
        (json!(null), "must be a number in MPa"),
    ] {
        let mut p = base();
        p["loads"] = json!([{"face": tip, "pressure": pressure}]);
        let e = s.execute(CMD, &p).unwrap_err().to_string();
        assert!(e.contains(needed), "{pressure}: `{needed}` missing from {e}");
    }
    let mut p = base();
    p["loads"] = json!([{"face": tip, "type": "pressure"}]);
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("give `pressure`"));
    let mut p = base();
    p["loads"] = json!([{"face": tip, "pressure": 1, "force": [0, 0, 1]}]);
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("takes `pressure` alone"));
    let mut p = base();
    p["mesh_check"] = json!("yes");
    assert!(s.execute(CMD, &p).unwrap_err().to_string().contains("`mesh_check` must be true or false"));
    let mut p = base();
    p["mesh_check"] = json!(false);
    assert!(run(&mut s, CMD, p)["setup"].to_string().contains("Mesh check: off"));
    assert!(fea::shown(&s.document_id).is_none());

    // A roller on a hole (curved): refused.
    let mut s = Session::default();
    let hole = angle(&mut s);
    let e = s
        .execute(CMD, &json!({"loads": [{"face": [5, 5, 1.5], "force": [0, 0, -5]}], "supports": [{"face": hole[0], "type": "roller"}], "material": aluminium(), "dry_run": true}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("a roller holds a flat face across itself") && e.contains("not flat"), "{e}");

    // A roller on a slanted flat face: refused (no tilted rollers yet).
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Wedge"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [40, 0], [0, 30]], "closed": true}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"sketch": "Wedge", "distance": 10}));
    let body = s.model.state().bodies.first().map(|b| b.name.clone()).unwrap_or_default();
    let faces = run(&mut s, "model.faces", json!({"body": body}));
    let face = |pick: &dyn Fn(f64, f64) -> bool| {
        faces["faces"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| pick(f["plane_normal"][0].as_f64().unwrap_or(0.0), f["plane_normal"][1].as_f64().unwrap_or(0.0)))
            .and_then(|f| f["index"].as_u64())
            .unwrap_or_else(|| panic!("no such face: {faces}"))
    };
    let slanted = face(&|x, _| x.abs() > 0.1 && x.abs() < 0.99);
    let back = face(&|x, _| x < -0.99);
    let e = s
        .execute(CMD, &json!({"loads": [{"face": slanted, "force": [-3, -4, 0]}], "supports": [{"face": back}, {"face": slanted, "type": "slider"}], "material": aluminium(), "dry_run": true}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("only a face that faces along X, Y or Z") && e.contains("tilted roller"), "{e}");
    assert!(fea::shown(&s.document_id).is_none(), "nothing ran");
}

/// U5: the words never pass or fail the part, whatever the mesh check and equilibrium say.
/// Input: fake runs at 20, 190 and 900 MPa against a 193 MPa yield; a peak at a support, still
/// changing; a reaction 10 % short; every kind of mesh check. Action: `wording`,
/// `check_warnings`, `check_line`, `equilibrium_line`, `equilibrium_warning`, the setup's words.
/// Pass: none of pass, fail, safe, approved, acceptable, verdict, compliant, factor of safety or
/// safety factor appears in any of them.
#[test]
fn the_new_words_never_pass_or_fail_the_part_or_give_a_factor_of_safety() {
    let material = Material { name: "5052-H32".into(), e_mpa: 70300.0, nu: 0.33, yield_mpa: 193.0, source: "a handbook".into() };
    let loads = [
        Load { label: "tip".into(), faces: vec![1], kind: LoadKind::Force([0.0, 0.0, -10.0]) },
        Load { label: "rim".into(), faces: vec![2], kind: LoadKind::Pressure(-10.0) },
    ];
    let supports = [
        Support { label: "root".into(), faces: vec![0], kind: SupportKind::Fixed },
        Support { label: "side".into(), faces: vec![3], kind: SupportKind::Roller(1) },
    ];
    let near = |_: P| "the root".to_string();
    for stress in [20.0, 190.0, 900.0] {
        let first = peaked_run(stress, Some("root"), Some(stress / 2.0), 8.0);
        let mut finer = peaked_run(stress * 1.2, Some("root"), Some(stress / 1.9), 9.0);
        finer.equilibrium = Some(fea::Equilibrium { reaction: [0.0, 0.0, 9.0], applied: [0.0, 0.0, -10.0], scale: 10.0, moment: None });
        for check in [
            Check::Ran { meshes: vec![2.0, 1.33], items: settling(&[&first, &finer]), third: Third::NotNeeded { quantity: AWAY, moved: 0.5 } },
            Check::Ran {
                meshes: vec![2.0, 1.33],
                items: settling(&[&first, &finer]),
                third: Third::Skipped { nodes: 9.0e5, bytes: 1.4e10, limit: Limit::Memory(6.0e9) },
            },
            Check::Ran {
                meshes: vec![2.0, 1.33, 0.89],
                items: settling(&[&first, &finer, &peaked_run(stress * 1.5, Some("root"), Some(stress / 1.5), 9.5)]),
                third: Third::Ran,
            },
            Check::Ran { meshes: vec![2.0, 1.33], items: settling(&[&first, &finer]), third: Third::AtSupport },
            Check::Skipped { nodes: 9.0e5, bytes: 1.4e10, limit: Limit::Nodes(2.0e5) },
            Check::Off,
        ] {
            let done = [first.clone_run()];
            let outcome = Outcome { done: &done, shown: &finer, check: &check };
            let (summary, assumptions, _) = wording(&material, &loads, &supports, &outcome, 1.5, &near);
            let mut all = vec![summary, assumptions.join("\n"), check_line(&check), check_warnings(&check, &finer, &near).join("\n")];
            if let Some(e) = &finer.equilibrium {
                all.push(equilibrium_line(e));
                all.push(equilibrium_warning(e).unwrap_or_default());
            }
            all.extend(loads.iter().map(load_words));
            all.extend(supports.iter().map(support_words));
            let all = all.join("\n").to_lowercase();
            for w in VERDICTS {
                assert!(!all.contains(w), "`{w}` in: {all}");
            }
        }
    }
}

#[test]
fn the_distance_away_from_supports_is_at_least_two_thicknesses_however_fine_the_mesh() {
    // A coarse mesh: its 1.5 elements are farther than two thicknesses.
    assert_eq!(away_distance(4.0, 1.5), 6.0);
    // A fine one (R5's 1 mm shells, 1.5 mm thick): two thicknesses, not 1.5 mm.
    assert_eq!(away_distance(1.0, 1.5), AWAY_SECTIONS * 1.5);
    // An unknown section leaves the elements' distance alone.
    for section in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(away_distance(1.0, section), 1.5, "{section}");
    }
    // Shells and sheets: the thickness given; other solids: 2 V / S, a plate's thickness give or
    // take its edges (R1's 100 x 20 x 1.5 strip: 1.38 mm), and 0 when not measured.
    assert_eq!(section_size(Elements::Shell, false, 1.5, Some(3000.0), 4360.0), 1.5);
    assert_eq!(section_size(Elements::Solid, true, 2.0, None, 0.0), 2.0);
    let strip = section_size(Elements::Solid, false, 1.0, Some(100.0 * 20.0 * 1.5), 2.0 * (2000.0 + 150.0 + 30.0));
    assert!((strip - 1.376).abs() < 1e-3, "{strip}");
    assert_eq!(section_size(Elements::Solid, false, 1.0, None, 4360.0), 0.0);
    assert_eq!(section_size(Elements::Solid, false, 1.0, Some(f64::NAN), 4360.0), 0.0);
    assert_eq!(section_size(Elements::Solid, false, 1.0, Some(3000.0), 0.0), 0.0);
}

/// A quantity's mesh check status and its first step's change (%), as the reply reports them.
fn mesh_check_status(r: &Value, quantity: &str) -> (String, f64) {
    let q = r["mesh_check"]["quantities"].as_array().into_iter().flatten().find(|q| q["quantity"] == quantity);
    q.map_or((String::new(), f64::NAN), |q| (q["status"].as_str().unwrap_or("").to_string(), q["changes_percent"][0].as_f64().unwrap_or(f64::NAN)))
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

/// R1, the real thing: a flat strip, 100 x 20 x 1.5 mm (5052-H32, E 70.3 GPa), clamped at one end
/// with 10 N down at the other, solid elements (C3D10), meshed and solved by gmsh and CalculiX on
/// this computer; the mesh check on. Needs both installed, so it runs only when asked:
/// `cargo test -p solidcraft-engine --release stress_tests::r1 -- --ignored --nocapture`.
/// Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// By hand (Euler-Bernoulli beam): I = b t³ / 12 = 5.625 mm⁴; tip deflection F L³ / (3 E I) =
/// 8.43 mm; bending stress σ(x) = F (L − x) c / I at the surface (c = 0.75 mm).
/// Pass: the mesh check ran; the highest stress is flagged at the clamp and is still changing or
/// above the peak away from it; the away peak is within 3 % of σ at its own x; the tip deflection
/// is within 4 % of 8.43 mm; the reaction is within 1 % of the 10 N applied.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn r1_a_cantilever_matches_beam_theory_away_from_the_clamp() {
    let mut s = Session::default();
    let (_, root, tip) = strip(&mut s, 100.0);
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": aluminium(), "elements": "solid"}),
    );
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
    assert_eq!(r["mesh_check"]["status"], "ran", "{r}");
    let clamp = r["setup"].as_array().into_iter().flatten().filter_map(Value::as_str).find_map(|l| l.strip_prefix("Held still: ")).unwrap_or("?");
    assert_eq!(r["peak_at_support"].as_str(), Some(clamp), "the highest stress sits at the clamp");
    let shown = &r["runs"][0];
    let (overall, away) = (number(&shown["max_von_mises_mpa"]), number(&r["peak_away"]["von_mises_mpa"]));
    let (status, change) = mesh_check_status(&r, "highest stress");
    eprintln!("R1 overall {overall:.1} MPa ({status}, {change:+.1}%), away {away:.1} MPa");
    assert_eq!(status, "at a support or load: depends on the mesh", "{r}");
    assert!(overall > away, "{r}");
    let at = &r["peak_away"]["at_mm"];
    let (x, z) = (number(&at[0]), number(&at[2]));
    assert!(z.abs() < 1e-3 || (z.abs() - 1.5).abs() < 1e-3, "the away peak is on a surface (c = 0.75 mm): z = {z}");
    let beam = 10.0 * (100.0 - x) * 0.75 / 5.625;
    eprintln!("R1 away peak at x = {x:.2} mm: {away:.2} MPa against beam theory {beam:.2} MPa ({:+.2}%)", 100.0 * (away / beam - 1.0));
    assert!((away / beam - 1.0).abs() <= 0.03, "away peak {away} vs beam {beam} at x = {x}");
    let tip_deflection = number(&shown["max_deflection_mm"]);
    let hand = 10.0 * 100f64.powi(3) / (3.0 * 70300.0 * 5.625);
    eprintln!("R1 tip deflection {tip_deflection:.3} mm against {hand:.3} mm ({:+.2}%)", 100.0 * (tip_deflection / hand - 1.0));
    assert!((tip_deflection / hand - 1.0).abs() <= 0.04, "{tip_deflection} vs {hand}");
    let eq = &r["equilibrium"];
    eprintln!(
        "R1 {} ({}% apart); solve times {:?} s with {} threads",
        eq["line"],
        eq["mismatch_percent"],
        r["runs"].as_array().map(|a| a.iter().map(|x| number(&x["seconds"])).collect::<Vec<_>>()),
        shown["solver_threads"]
    );
    assert!(number(&eq["mismatch_percent"]) < 1.0 && (number(&eq["loads_applied_n"]) - 10.0).abs() < 1e-6, "{eq}");
    assert!(
        fea::shown(&s.document_id).is_some_and(|f| f.away_stress.is_some() && f.mesh_check.is_some() && f.equilibrium.is_some()),
        "painted, with both markers and the lines"
    );
}

/// A plate 200 x 50 x 2 mm with a Ø10 hole through its middle, made with the app's commands: its
/// end faces at x = 0 and x = 200.
fn plate_with_hole(s: &mut Session) -> (u64, u64) {
    run(s, "solid.box", json!({"length": 200, "width": 50, "height": 2, "body_name": "Plate"}));
    run(s, "solid.hole", json!({"position": [100, 25, 2], "diameter": 10}));
    let faces = run(s, "model.faces", json!({"body": "Plate"}));
    let end = |x: f64| {
        faces["faces"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| {
                (f["centroid"][0].as_f64().unwrap_or(f64::NAN) - x).abs() < 1e-6 && f["plane_normal"][0].as_f64().is_some_and(|n| n.abs() > 0.99)
            })
            .and_then(|f| f["index"].as_u64())
            .unwrap_or_else(|| panic!("no end face at x = {x}: {faces}"))
    };
    (end(0.0), end(200.0))
}

/// R2, real: a plate 200 x 50 x 2 mm with a central Ø10 hole, one end held, 800 N pulling the
/// other end along X; solid elements, the mesh check on. A smooth stress concentration, far from
/// the support and the load.
/// By hand: the net section is (50 − 10) x 2 = 80 mm², so σ_nom = 800 / 80 = 10 MPa; Heywood's
/// finite-width formula for a central circular hole in tension, on the net section,
/// Kt = 2 + (1 − d/W)³ = 2 + 0.8³ = 2.512 (R. B. Heywood, Designing by Photoelasticity, Chapman &
/// Hall 1952; as given in W. D. Pilkey and D. F. Pilkey, Peterson's Stress Concentration Factors,
/// 3rd ed., Wiley 2008, ch. 4), so the peak at the hole's edge is Kt σ_nom = 25.1 MPa.
/// Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// Mesh: 1.5 mm (the default was 1.0 mm before `default_solid_size` capped its nodes: 199 504
/// nodes, whose mesh check's finer run, 595 186 nodes and 14 GB measured, is over the 6 GB limit
/// and skipped). At 1.5 mm the finer run
/// is predicted at about 4 GB, so the check runs on a computer with at least 8 GB available.
/// Run with SOLIDCRAFT_FEA_MAX_NODES set below its finer run (100000), the check is skipped and
/// says why, and everything but "settled" is still checked.
/// Pass: the away peak sits at the hole's edge (x ≈ 100, y ≈ 20 or 30), its mesh check status
/// is settled, and it is within 6 % of 25.1 MPa; the forces balance within 1 %.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn r2_a_plate_with_a_hole_in_tension_settles_at_heywoods_peak() {
    let mut s = Session::default();
    let (held, pulled) = plate_with_hole(&mut s);
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": pulled, "force": [800, 0, 0]}], "supports": [{"face": held}], "material": aluminium(), "elements": "solid", "mesh_size": 1.5}),
    );
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
    let capped = std::env::var(MAX_NODES_VAR).is_ok_and(|v| !v.trim().is_empty()) && r["mesh_check"]["status"] == "skipped";
    if capped {
        let line = r["mesh_check"]["line"].as_str().unwrap_or("");
        eprintln!("R2 with {MAX_NODES_VAR} set: mesh check {line}");
        assert!(line.contains(MAX_NODES_VAR) && line.contains("unknown"), "{line}");
    } else {
        assert_eq!(r["mesh_check"]["status"], "ran", "{r}");
    }
    let kt = 2.0 + (1.0 - 10.0 / 50.0f64).powi(3);
    let hand = kt * 800.0 / ((50.0 - 10.0) * 2.0);
    let away = number(&r["peak_away"]["von_mises_mpa"]);
    let at = &r["peak_away"]["at_mm"];
    let (x, y) = (number(&at[0]), number(&at[1]));
    let (status, change) = mesh_check_status(&r, "highest stress away from supports and loads");
    eprintln!(
        "R2 away peak {away:.2} MPa at ({x:.2}, {y:.2}) against Kt σnom = {hand:.2} MPa ({:+.2}%), mesh check {status} ({change:+.2}%); overall {} MPa at {:?}; {}; times {:?} s",
        100.0 * (away / hand - 1.0),
        r["runs"][0]["max_von_mises_mpa"],
        r["peak_at_support"],
        r["equilibrium"]["line"],
        r["runs"].as_array().map(|a| a.iter().map(|x| number(&x["seconds"])).collect::<Vec<_>>())
    );
    assert!((x - 100.0).abs() < 1.0 && ((y - 20.0).abs() < 1.0 || (y - 30.0).abs() < 1.0), "at the hole's edge across the pull: ({x}, {y})");
    if !capped {
        assert_eq!(status, "settled", "{r}");
    }
    assert!((away / hand - 1.0).abs() <= 0.06, "{away} vs {hand}");
    assert!(number(&r["equilibrium"]["mismatch_percent"]) < 1.0, "{}", r["equilibrium"]);
}

/// The NAFEMS LE1 membrane as the app models it (mm): a quarter of the elliptic ring between
/// x²/2000² + y²/1000² = 1 and x²/3250² + y²/2750² = 1, 100 mm thick, from two ellipses extruded
/// and intersected with the first quadrant.
fn le1(s: &mut Session) -> String {
    run(s, "sketch.create", json!({"plane": "XY", "name": "Ring"}));
    run(s, "sketch.ellipse", json!({"center": [0, 0], "major": [3250, 0], "minor_radius": 2750, "axes": false}));
    run(s, "sketch.ellipse", json!({"center": [0, 0], "major": [2000, 0], "minor_radius": 1000, "axes": false}));
    run(s, "sketch.finish", json!({}));
    run(s, "solid.extrude", json!({"sketch": "Ring", "distance": 100, "body_name": "LE1"}));
    run(s, "sketch.create", json!({"plane": "XY", "name": "Quadrant"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [4000, 4000]}));
    run(s, "sketch.finish", json!({}));
    run(s, "solid.extrude", json!({"sketch": "Quadrant", "distance": 100, "operation": "intersect", "targets": ["LE1"]}));
    "LE1".to_string()
}

/// R3, real: the NAFEMS LE1 benchmark (NAFEMS, The Standard NAFEMS Benchmarks, TNSB Rev. 3, 1990:
/// linear elastic test LE1, elliptic membrane). Quarter model: the ring above, E 210 GPa,
/// ν 0.3, an outward pressure of 10 MPa on the outer curved edge (−10 MPa here, where positive
/// pushes in), rollers on the y = 0 face (v held) and the x = 0 face (u held). The benchmark is
/// plane stress and free in z; a solid needs one more hold against sliding along z, so the
/// bottom face (z = 0) is a z roller: it is the mid-plane of a plate twice as thick, which in
/// plane stress leaves the in-plane stresses as they are.
/// Target: σyy at D = (2000, 0) on the inner edge = 92.7 MPa; there σxx and σzz are about 0, so
/// von Mises ≈ σyy. Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// Pass: the von Mises stress of the solver node nearest D at mid-thickness, (2000, 0, 50), is
/// within 3 % of 92.7 MPa on the shown (mesh-checked) mesh; the forces balance within 1 %.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn r3_nafems_le1_elliptic_membrane_matches_the_benchmark_at_d() {
    let mut s = Session::default();
    let body = le1(&mut s);
    let steel = json!({"name": "LE1 steel", "youngs_modulus": 210000, "poisson_ratio": 0.3, "yield_strength": 250, "source": "NAFEMS LE1 (E, ν); the yield strength is a nominal steel value for the colour scale only"});
    let r = run(
        &mut s,
        CMD,
        json!({
            "body": body,
            "loads": [{"face": [2298.1, 1944.5, 50], "pressure": "-10 MPa"}],
            "supports": [
                {"face": [2600, 0, 50], "type": "roller"},
                {"face": [0, 1800, 50], "type": "roller"},
                {"face": [2500, 500, 0], "type": "roller"},
            ],
            "material": steel,
        }),
    );
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
    let f = fea::shown(&s.document_id).unwrap_or_else(|| panic!("painted: {r}"));
    // The painted field takes each point's value from the solver node nearest it; a point at D's
    // place lies on a node there (the mesher puts nodes on the model's edges).
    let at = |p: P| {
        let i = (0..f.positions.len())
            .min_by(|a, b| fea::model::len(fea::model::sub(f.positions[*a], p)).total_cmp(&fea::model::len(fea::model::sub(f.positions[*b], p))));
        i.map_or((f64::NAN, f64::NAN), |i| (f64::from(f.stress[i]), fea::model::len(fea::model::sub(f.positions[i], p))))
    };
    for z in [0.0, 50.0, 100.0] {
        let (v, off) = at([2000.0, 0.0, z]);
        eprintln!("R3 von Mises at D, z = {z}: {v:.2} MPa ({:+.2}% against 92.7; point {off:.2} mm from D)", 100.0 * (v / 92.7 - 1.0));
    }
    let (v, _) = at([2000.0, 0.0, 50.0]);
    eprintln!(
        "R3 overall {} MPa at {:?}; away {}; mesh check {}; {}; times {:?} s",
        r["runs"][0]["max_von_mises_mpa"],
        r["runs"][0]["max_stress_near"],
        r["peak_away"]["von_mises_mpa"],
        r["mesh_check"]["line"],
        r["equilibrium"]["line"],
        r["runs"].as_array().map(|a| a.iter().map(|x| number(&x["seconds"])).collect::<Vec<_>>())
    );
    assert!(number(&r["equilibrium"]["mismatch_percent"]) < 1.0, "{}", r["equilibrium"]);
    assert!((v / 92.7 - 1.0).abs() <= 0.03, "von Mises at D {v} vs 92.7");
}

/// The painted von Mises stress at the display point nearest `p`, and how far that point is.
fn painted_at(s: &Session, p: P) -> (f64, f64) {
    let Some(f) = fea::shown(&s.document_id) else { return (f64::NAN, f64::NAN) };
    let d = |i: usize| fea::model::len(fea::model::sub(f.positions[i], p));
    (0..f.positions.len()).min_by(|a, b| d(*a).total_cmp(&d(*b))).map_or((f64::NAN, f64::NAN), |i| (f64::from(f.stress[i]), d(i)))
}

/// The run times of a reply, s.
fn times(r: &Value) -> Vec<f64> {
    r["runs"].as_array().map(|a| a.iter().map(|x| number(&x["seconds"])).collect()).unwrap_or_default()
}

/// R4, shells with a pressure: R1's strip (100 x 20 x 1.5 mm, 5052-H32) as six-node shells on its
/// mid-surface, clamped at x = 0, with 0.005 MPa pushing on its top face (10 N in all, down);
/// the default shell mesh and the mesh check on. Real: the app's modelling, gmsh, CalculiX.
/// Faked: nothing. Run with `--ignored`.
/// By hand (Euler-Bernoulli, a uniform load w = p b = 0.1 N/mm): σ(x) = w (L − x)² c / (2 I) =
/// (100 − x)² / 150 MPa at the surface (66.7 MPa at the root, 16.7 MPa at mid-span); tip
/// deflection w L⁴ / (8 E I) = 3.161 mm.
/// Pass: the loads applied are 10 N and the forces balance within 1 %; the away peak is within
/// 3 % of σ at its own x; the painted stress at the top surface's mid-span (50, 10, 1.5) is within
/// 5 % of 16.7 MPa; the tip deflection is within 6 % of 3.161 mm (R1's strip as shells with a tip
/// force deflected 5 % less than the beam: a 20 mm wide clamp also holds the strip's sideways
/// curl, which a beam ignores); the mesh check ran and reports every quantity.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn r4_shells_with_a_pressure_match_beam_theory() {
    let mut s = Session::default();
    let (_, root, _) = strip(&mut s, 100.0);
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": [50, 10, 1.5], "pressure": 0.005}], "supports": [{"face": root}], "material": aluminium(), "elements": "shell"}),
    );
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
    let eq = &r["equilibrium"];
    assert!((number(&eq["loads_applied_n"]) - 10.0).abs() < 1e-6 && number(&eq["mismatch_percent"]) < 1.0, "{eq}");
    let away = number(&r["peak_away"]["von_mises_mpa"]);
    let x = number(&r["peak_away"]["at_mm"][0]);
    let beam = (100.0 - x).powi(2) / 150.0;
    let (mid, off) = painted_at(&s, [50.0, 10.0, 1.5]);
    let tip = number(&r["runs"][0]["max_deflection_mm"]);
    let hand = 0.1 * 100f64.powi(4) / (8.0 * 70300.0 * 5.625);
    eprintln!(
        "R4 away peak at x = {x:.2} mm: {away:.2} MPa against {beam:.2} ({:+.2}%); mid-span {mid:.2} MPa against 16.67 ({:+.2}%, point {off:.2} mm off); \
         tip {tip:.3} mm against {hand:.3} ({:+.2}%); {}; mesh check {}; times {:?} s",
        100.0 * (away / beam - 1.0),
        100.0 * (mid / (50f64.powi(2) / 150.0) - 1.0),
        100.0 * (tip / hand - 1.0),
        eq["line"],
        r["mesh_check"]["line"],
        times(&r)
    );
    assert!((away / beam - 1.0).abs() <= 0.03, "away {away} vs {beam} at x = {x}");
    assert!((mid / (50f64.powi(2) / 150.0) - 1.0).abs() <= 0.05, "mid-span {mid}");
    assert!((tip / hand - 1.0).abs() <= 0.06, "tip {tip} vs {hand}");
    assert_eq!(r["mesh_check"]["status"], "ran", "{r}");
    assert_eq!(r["mesh_check"]["quantities"].as_array().map(Vec::len), Some(3), "{r}");
}

/// R5, shells with a symmetry plane: half of R1's strip, 100 x 10 x 1.5 mm, as shells, clamped
/// at x = 0, 5 N down at x = 100, and a `symmetry` support on its side at y = 10 (held along Y,
/// and as shells kept from tilting out of the plane); against the whole strip, 100 x 20 mm with
/// 10 N, at the same 1 mm mesh. Both with the mesh check off (this test is about the support, not
/// the mesh). Real: the app's modelling, gmsh, CalculiX. Faked: nothing. Run with `--ignored`.
/// Pass: both hold still (the setup is accepted, its words say held along Y) and balance within
/// 1 %; the half's tip deflection is within 1 % of the whole strip's and its away peak within 3 %
/// of the whole's and of beam theory at its own x. History: with the side held along Y only, the
/// half bent 1.2 % more than the whole (its edge could tilt); with the edge's tilt held too, the
/// tips match to 0.01 %. Before the away distance had its floor of two thicknesses
/// (`AWAY_SECTIONS`), the away peak at 1 mm sat 1.3 mm from the clamp, inside its disturbance, 4.7 %
/// over beam theory; with it, 5.5 mm away and 1.1 % over.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn r5_a_symmetry_plane_on_shells_gives_the_whole_strips_answer() {
    let solve = |width: f64, force: f64, symmetry: bool| {
        let mut s = Session::default();
        let (_, root, tip) = strip_of(&mut s, 100.0, width);
        let mut supports = vec![json!({"face": root})];
        if symmetry {
            supports.push(json!({"face": [50, width, 0.75], "type": "symmetry"}));
        }
        let r = run(
            &mut s,
            CMD,
            json!({"loads": [{"face": tip, "force": [0, 0, -force]}], "supports": supports, "material": aluminium(), "elements": "shell",
                   "mesh_size": 1.0, "mesh_check": false}),
        );
        eprintln!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
        assert!(number(&r["equilibrium"]["mismatch_percent"]) < 1.0, "{}", r["equilibrium"]);
        r
    };
    let half = solve(10.0, 5.0, true);
    let whole = solve(20.0, 10.0, false);
    let setup = half["setup"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
    assert!(setup.contains("Held along Y only"), "{setup}");
    let tip = |r: &Value| number(&r["runs"][0]["max_deflection_mm"]);
    let away = |r: &Value| number(&r["peak_away"]["von_mises_mpa"]);
    let x = number(&half["peak_away"]["at_mm"][0]);
    let beam = 10.0 * (100.0 - x) * 0.75 / 5.625;
    eprintln!(
        "R5 half: tip {:.3} mm, away {:.2} MPa at x = {x:.2} (beam {beam:.2}, {:+.2}%); whole: tip {:.3} mm, away {:.2} MPa; tip {:+.2}%, away {:+.2}%; \
         times {:?} / {:?} s",
        tip(&half),
        away(&half),
        100.0 * (away(&half) / beam - 1.0),
        tip(&whole),
        away(&whole),
        100.0 * (tip(&half) / tip(&whole) - 1.0),
        100.0 * (away(&half) / away(&whole) - 1.0),
        times(&half),
        times(&whole)
    );
    assert!((tip(&half) / tip(&whole) - 1.0).abs() <= 0.01, "tips");
    assert!((away(&half) / away(&whole) - 1.0).abs() <= 0.03, "away peaks");
    assert!((away(&half) / beam - 1.0).abs() <= 0.03, "away peak {} vs beam {beam} at x = {x}", away(&half));
}

/// R6, `elements: both` with the mesh check: R1's strip, clamped, 10 N at its tip, solved as
/// shells and as solids at the default meshes, the mesh check on. Real: the app's modelling,
/// gmsh, CalculiX. Faked: nothing. Run with `--ignored`.
/// Pass: three runs, the shown one first: the shells' finer run (mesh_check_run), the shells,
/// then the solids; the mesh check ran on the shells (its first mesh the shells' mesh) and
/// reports three quantities, the highest stress at the clamp as "at a support or load: depends on
/// the mesh"; the shell and solid results are compared side by side; both balance within 1 %; the
/// shells' and solids' tip deflections are within 6 % of each other.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn r6_both_elements_run_the_mesh_check_on_the_shells_and_compare_with_the_solids() {
    let mut s = Session::default();
    let (_, root, tip) = strip(&mut s, 100.0);
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": aluminium(), "elements": "both"}),
    );
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
    let runs = r["runs"].as_array().cloned().unwrap_or_default();
    let shape: Vec<(String, bool)> = runs.iter().map(|x| (x["elements"].as_str().unwrap_or("").to_string(), x["mesh_check_run"] == true)).collect();
    eprintln!(
        "R6 runs {shape:?}; mesh check {}; shell vs solid {}; times {:?} s; warnings {}",
        r["mesh_check"]["line"],
        r["shell_vs_solid"],
        times(&r),
        r["warnings"]
    );
    // The mesh check's finer shell runs (one, or two with a third mesh), then the first meshes.
    let checks = shape.iter().take_while(|(_, check)| *check).count();
    assert!((1..=2).contains(&checks) && shape.iter().all(|(e, check)| e == "shell" || !check), "{r}");
    assert_eq!(shape[checks..], [("shell".to_string(), false), ("solid".to_string(), false)], "{r}");
    assert_eq!(r["mesh_check"]["status"], "ran", "{r}");
    assert_eq!(number(&r["mesh_check"]["meshes_mm"][0]), number(&runs[checks]["mesh_size_mm"]), "{r}");
    let (status, _) = mesh_check_status(&r, "highest stress");
    assert_eq!(status, "at a support or load: depends on the mesh", "{r}");
    assert_eq!(r["mesh_check"]["quantities"].as_array().map(Vec::len), Some(3), "{r}");
    assert!(r["shell_vs_solid"].as_str().is_some_and(|t| !t.is_empty()), "{r}");
    for x in &runs {
        let (carry, applied) = (number(&x["supports_carry_n"]), number(&x["loads_applied_n"]));
        assert!((carry / applied - 1.0).abs() < 0.01, "{x}");
    }
    let (shell, solid) = (number(&runs[checks]["max_deflection_mm"]), number(&runs[checks + 1]["max_deflection_mm"]));
    assert!((shell / solid - 1.0).abs() <= 0.06, "shell {shell} vs solid {solid}");
}

/// P2, the memory limit on a computer short of memory: R2's plate (1.5 mm mesh: 66 300 nodes,
/// its finer run predicted at about 4 GB), run while something else holds most of the memory
/// (run it by hand that way, on Linux). Real: the app's modelling, gmsh, CalculiX,
/// /proc/meminfo. Faked: nothing. Run with `--ignored`.
/// Observable: the memory available before the run (read here); the mesh check's status and line.
/// Pass: with half the memory available under the finer run's prediction, the check is skipped,
/// its line says "over this computer's limit of about" a size within 25 % of half the memory
/// available, and the first run's numbers still come back balanced; with more memory, it ran.
#[test]
#[ignore = "runs gmsh and CalculiX, and wants most of the memory taken"]
fn p2_short_of_memory_the_mesh_check_is_skipped_at_half_the_memory_available() {
    let available = std::fs::read_to_string("/proc/meminfo").ok().and_then(|t| mem_available(&t)).unwrap_or(f64::NAN);
    let mut s = Session::default();
    let (held, pulled) = plate_with_hole(&mut s);
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": pulled, "force": [800, 0, 0]}], "supports": [{"face": held}], "material": aluminium(), "elements": "solid", "mesh_size": 1.5}),
    );
    let line = r["mesh_check"]["line"].as_str().unwrap_or("").to_string();
    let first = number(&r["runs"].as_array().and_then(|a| a.last()).map_or(Value::Null, |x| x["nodes"].clone()));
    let (_, predicted) = predict_finer(first as usize, Elements::Solid, forecast::layers(2.0 * 19858.6 / 20919.8, 1.0));
    eprintln!(
        "P2 available {:.2} GB (half {:.2} GB); first run {first} nodes, finer predicted {:.2} GB; mesh check {}: {line}; {}",
        available / 1e9,
        available / 2e9,
        predicted / 1e9,
        r["mesh_check"]["status"],
        r["equilibrium"]["line"]
    );
    assert!(number(&r["equilibrium"]["mismatch_percent"]) < 1.0, "{}", r["equilibrium"]);
    if available / 2.0 < predicted {
        assert_eq!(r["mesh_check"]["status"], "skipped", "{r}");
        let size = line.split("over this computer's limit of about ").nth(1).unwrap_or("");
        let gb = size.split(' ').next().and_then(|n| n.parse::<f64>().ok()).unwrap_or(f64::NAN) * if size.contains(" MB") { 1e-3 } else { 1.0 };
        assert!((gb * 1e9 / (available / 2.0) - 1.0).abs() <= 0.25, "limit {gb} GB against half of {available}: {line}");
    } else {
        assert_eq!(r["mesh_check"]["status"], "ran", "{r}");
    }
}

/// P1, how long the step between the mesh and the solver takes for a large shell model: R1's
/// strip as shells at a 0.3 mm mesh (about 100 000 nodes), clamped, 10 N at its tip, the mesh
/// check off. Real: the app's modelling, gmsh, CalculiX. Faked: nothing. Run with `--ignored`.
/// Observable: the run's seconds (mesh, model, solve, read), printed with its nodes and numbers.
/// Pass: it solves and balances within 1 %. The time is reported, not asserted (it depends on
/// the computer); before the face matching used a grid, this run spent about 300 s outside the
/// solver.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn p1_a_large_shell_model_is_prepared_quickly() {
    let mut s = Session::default();
    let (_, root, tip) = strip(&mut s, 100.0);
    let started = std::time::Instant::now();
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": aluminium(), "elements": "shell",
               "mesh_size": 0.3, "mesh_check": false}),
    );
    let x = &r["runs"][0];
    eprintln!(
        "P1 {} nodes, run {:.1} s (call {:.1} s); max {} MPa, away {} MPa, tip {} mm; {}",
        x["nodes"],
        number(&x["seconds"]),
        started.elapsed().as_secs_f64(),
        x["max_von_mises_mpa"],
        x["max_von_mises_away_mpa"],
        x["max_deflection_mm"],
        r["equilibrium"]["line"]
    );
    assert!(number(&r["equilibrium"]["mismatch_percent"]) < 1.0, "{}", r["equilibrium"]);
}

/// G1: the mesh check runs only when its finer run's predicted size fits the limit.
/// Input: fake first runs (node counts only, nothing runs): 20 000, 50 000 and R2's 199 504 solid
/// nodes, as deep as R2's plate at its finer size (1.9 mm at 0.67 mm: 2.8 layers), 50 000 and
/// 100 000 shell nodes; a 100 000-node cap given as SOLIDCRAFT_FEA_MAX_NODES's text, and a 6 GB
/// memory limit; junk cap texts.
/// Action: `finer_limit`, `mesh_check_gate`, `check_line`, `mesh_check_json`.
/// Observable: whether the check runs; the skipped line and the JSON reason.
/// Pass: under the node cap 20 000 solid nodes (about 60 000 predicted) runs and 50 000 (about
/// 150 000) is skipped, its line naming the predicted nodes, their memory, the cap and its variable
/// and saying whether the numbers settled is unknown; under 6 GB, R2's first run is skipped with
/// "over this computer's limit of about 6 GB", 50 000 solid nodes run, 50 000 shell nodes run and
/// 100 000 shell nodes are skipped; a cap text that is not a whole number above 0 is ignored.
#[test]
fn g1_the_mesh_check_is_skipped_with_its_reason_when_the_finer_run_would_be_over_the_limit() {
    let cap = finer_limit(Some("100000"), None, MEMORY_CEILING);
    assert_eq!(cap, Limit::Nodes(100_000.0));
    let deep = forecast::layers(2.0 * 19858.6 / 20919.8, 1.0 / 1.5);
    assert_eq!(mesh_check_gate(20_000, Elements::Solid, deep, cap), None, "about 60 000 nodes: under the cap, it runs");
    let skip = mesh_check_gate(50_000, Elements::Solid, deep, cap).unwrap_or_else(|| panic!("about 150 000 nodes: skipped"));
    let line = check_line(&skip);
    for needed in [
        "skipped: the finer mesh would have about 150,000 nodes (about ",
        "over the limit of 100,000 nodes set by SOLIDCRAFT_FEA_MAX_NODES",
        "so how far the numbers are from settling is unknown",
    ] {
        assert!(line.contains(needed), "`{needed}` missing: {line}");
    }
    let json = mesh_check_json(&skip);
    assert_eq!(json.status, "skipped");
    assert!(json.reason.as_deref().is_some_and(|r| r.starts_with("the finer mesh would have about 150,000 nodes")), "{:?}", json.reason);
    assert_eq!(json.line, line);

    let memory = Limit::Memory(6.0e9);
    let skip = mesh_check_gate(199_504, Elements::Solid, deep, memory).unwrap_or_else(|| panic!("R2's finer run is over 6 GB"));
    let line = check_line(&skip);
    eprintln!("G1 R2 under 6 GB: {line}");
    assert!(line.starts_with("skipped: the finer mesh would have about ") && line.contains("over this computer's limit of about 6 GB"), "{line}");
    assert_eq!(mesh_check_gate(50_000, Elements::Solid, deep, memory), None);
    assert_eq!(mesh_check_gate(50_000, Elements::Shell, 1.0, memory), None);
    assert!(mesh_check_gate(100_000, Elements::Shell, 1.0, memory).is_some());
    for junk in ["", "  ", "abc", "0", "-5", "1e5", "12.5"] {
        assert_eq!(finer_limit(Some(junk), None, 6.0e9), Limit::Memory(6.0e9), "`{junk}` is ignored");
    }
    assert!(!check_line(&skip).to_lowercase().contains("fail"));
}

/// G2: the memory limit is the smaller of the ceiling and half the memory available.
/// Input: sample /proc/meminfo texts with MemAvailable 8 000 000 kB (8.19 GB), 30 000 000 kB
/// (30.7 GB), none, and junk; no text at all (not Linux). Action: `mem_available`, `finer_limit`
/// with a 6 GB ceiling. Observable: the bytes read and the limit. Pass: 8 192 000 000 bytes and a
/// limit of half that (4.096 GB); 30.7 GB available keeps the 6 GB ceiling; a text without
/// MemAvailable, a junk number or no text gives the ceiling.
#[test]
fn g2_the_memory_limit_is_the_smaller_of_the_ceiling_and_half_the_memory_available() {
    let sample =
        |kb: &str| format!("MemTotal:       32617284 kB\nMemFree:        15938128 kB\nMemAvailable:   {kb} kB\nBuffers:          402312 kB\n");
    assert_eq!(mem_available(&sample("8000000")), Some(8_192_000_000.0));
    assert_eq!(finer_limit(None, mem_available(&sample("8000000")), 6.0e9), Limit::Memory(4_096_000_000.0));
    assert_eq!(finer_limit(None, mem_available(&sample("30000000")), 6.0e9), Limit::Memory(6.0e9));
    assert_eq!(finer_limit(None, mem_available("MemTotal: 32617284 kB\n"), 6.0e9), Limit::Memory(6.0e9));
    assert_eq!(finer_limit(None, mem_available(&sample("lots")), 6.0e9), Limit::Memory(6.0e9));
    assert_eq!(finer_limit(None, None, 6.0e9), Limit::Memory(6.0e9));
    // The cap, when set, replaces the memory limit.
    assert_eq!(finer_limit(Some("250000"), mem_available(&sample("8000000")), 6.0e9), Limit::Nodes(250_000.0));
}

/// Holds at `points`, fixed (X, Y and Z) or along one axis.
fn holds_at(points: &[P], along: Option<usize>) -> Vec<fea::model::Hold> {
    let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    points
        .iter()
        .flat_map(|p| {
            axes.iter().enumerate().filter(move |(k, _)| along.is_none_or(|a| a == *k)).map(move |(_, e)| fea::model::Hold { at: *p, along: *e })
        })
        .collect()
}

/// A strip's end face as points: x = 0, y 0 to 20, z 0 to 1.5.
fn end_face_points() -> Vec<P> {
    (0..=4).flat_map(|i| [0.0, 1.5].map(|z| [0.0, 5.0 * f64::from(i), z])).collect()
}

fn close(a: P, b: P) -> bool {
    fea::model::len(fea::model::sub(a, b)) < 1e-6
}

/// B1: a fixed flat face holds the part still; a fixed line leaves the turn about it.
/// Input (points built directly, nothing runs): the end face of a 100 x 20 x 1.5 mm strip as ten
/// points (x = 0, y 0 to 20, z 0 and 1.5), held fixed, part size 102 mm; eleven points on the line
/// y = 5, z = 2 from x = 0 to 10, held fixed. Action: `free_motions`, `freedom_words`.
/// Observable: the free slides and turns, the ratio, the words.
/// Pass: the face: nothing free, the ratio above 1e-6 (a million times the tolerance), no words;
/// the line: no slide, exactly one turn, about X through (5, 5, 2), and the words name it.
#[test]
fn b1_a_fixed_face_holds_still_and_a_fixed_line_leaves_the_turn_about_it() {
    let face = fea::model::free_motions(&holds_at(&end_face_points(), None), 102.0);
    eprintln!("B1 fixed end face: ratio {:e}", face.ratio);
    assert!(face.held() && face.ratio > 1e-6 && face.slides.is_empty() && face.turns.is_empty(), "{face:?}");
    assert_eq!(freedom_words(&face), None);
    let line: Vec<P> = (0..=10).map(|i| [f64::from(i), 5.0, 2.0]).collect();
    let free = fea::model::free_motions(&holds_at(&line, None), 102.0);
    eprintln!("B1 fixed line: ratio {:e}, {free:?}", free.ratio);
    assert!(!free.held() && free.slides.is_empty() && free.turns.len() == 1, "{free:?}");
    let (axis, through) = free.turns[0];
    assert!(close(axis, [1.0, 0.0, 0.0]) && close(through, [5.0, 5.0, 2.0]), "{free:?}");
    let words = freedom_words(&free).unwrap_or_default();
    assert!(words.contains("it can turn about an axis along X through (5, 5, 2) mm.") && !words.contains("slide"), "{words}");
    assert!(words.contains("stops the turn") && words.contains("Nothing was meshed or solved"), "{words}");
}

/// B2 (points): a single roller leaves three motions free, not five.
/// Input: the strip's end face points (x = 0) held by a roller along X only. Action: `free_motions`,
/// `freedom_words`. Observable: the free motions and the words.
/// Pass: slides along Y and Z and a turn about an axis along X through the face's middle
/// (0, 10, 0.75): three. (Not five: the face's points spread over Y and Z, so holding each one
/// along X stops the slide along X and the turns about Y and Z; only a single point would leave
/// five.) The words list both slides and the turn, and say a roller facing Y and one facing Z stop
/// the slides.
#[test]
fn b2_a_single_roller_leaves_two_slides_and_a_turn_free() {
    let free = fea::model::free_motions(&holds_at(&end_face_points(), Some(0)), 102.0);
    eprintln!("B2 one X roller: {free:?}");
    assert_eq!(free.slides.len(), 2, "{free:?}");
    assert!(close(free.slides[0], [0.0, 1.0, 0.0]) && close(free.slides[1], [0.0, 0.0, 1.0]), "{free:?}");
    assert_eq!(free.turns.len(), 1, "{free:?}");
    assert!(close(free.turns[0].0, [1.0, 0.0, 0.0]) && close(free.turns[0].1, [0.0, 10.0, 0.75]), "{free:?}");
    let words = freedom_words(&free).unwrap_or_default();
    assert!(words.contains("it can slide along Y, slide along Z and turn about an axis along X through (0, 10, 0.75) mm"), "{words}");
    assert!(words.contains("a roller on a face facing along Y stops the slide along Y"), "{words}");
    // A single point held along X leaves five.
    let point = fea::model::free_motions(&holds_at(&[[0.0, 10.0, 0.75]], Some(0)), 102.0);
    assert_eq!(point.slides.len() + point.turns.len(), 5, "{point:?}");
    assert_eq!(along_words([std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2, 0.0]), "along a direction (0.71, 0.71, 0)");
}

/// The free motions of `supports` (face index, kind) on `body`, from its display triangles.
fn freedom_of(s: &Session, body: &str, supports: &[(u64, SupportKind)]) -> fea::model::Freedom {
    let st = s.world_state().unwrap_or_else(|e| panic!("{e}"));
    let b = st.bodies.iter().find(|b| b.name == body).unwrap_or_else(|| panic!("no body {body}"));
    let mesh = b.mesh();
    let positions: Vec<P> = mesh.positions.iter().map(|v| [v.x, v.y, v.z]).collect();
    let display = Faces { positions: &positions, triangles: &mesh.triangles, tri_face: &mesh.tri_face };
    let supports: Vec<Support> =
        supports.iter().map(|(f, kind)| Support { label: String::new(), faces: vec![u32::try_from(*f).unwrap_or(u32::MAX)], kind: *kind }).collect();
    let (lo, hi) = positions.iter().fold(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), |(lo, hi), p| {
        (std::array::from_fn(|k| lo[k].min(p[k])), std::array::from_fn(|k| hi[k].max(p[k])))
    });
    fea::model::free_motions(&support_holds(&display, &supports), fea::model::len(fea::model::sub(hi, lo)))
}

/// The index of `body`'s flat face facing `normal`.
fn face_facing(s: &mut Session, body: &str, normal: P) -> u64 {
    let faces = run(s, "model.faces", json!({"body": body}));
    faces["faces"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| (0..3).all(|k| (f["plane_normal"][k].as_f64().unwrap_or(f64::NAN) - normal[k]).abs() < 1e-6))
        .and_then(|f| f["index"].as_u64())
        .unwrap_or_else(|| panic!("no face facing {normal:?}: {faces}"))
}

/// B2 (LE1) and the real tests' setups: which hold the part still, measured on the bodies the app
/// builds. Input: R1's strip clamped at its end face; R2's plate held at one end; NAFEMS LE1 (the
/// R3 ring, 100 mm thick) with rollers on its y = 0, x = 0 and z = 0 faces, and without the z = 0
/// roller. Action: `free_motions` on each support's display triangles; for LE1 also
/// `inspect.stress_estimate` with `dry_run`. Nothing is meshed or solved.
/// Observable: the free motions, the ratio; the dry run's reply or refusal.
/// Pass: R1, R2 and LE1's three rollers hold still with a ratio at least 1000 times the tolerance
/// (the dry run accepts LE1); without the z roller exactly one motion is free, the slide along Z,
/// and the dry run is refused naming it. The turns about X and Y are not free: each roller face
/// is 100 mm thick, so turning about X moves the y = 0 face's points along Y by different amounts
/// through the thickness (the y roller stops it) and turning about Y does the same to the x = 0
/// face along X. That matches the drift seen without the z roller: every node moved +0.04 mm, a
/// pure slide.
#[test]
fn b2_the_real_tests_setups_hold_still_and_le1_without_its_z_roller_slides_along_z() {
    let mut s = Session::default();
    let (body, root, _) = strip(&mut s, 100.0);
    let r1 = freedom_of(&s, &body, &[(root, SupportKind::Fixed)]);
    let mut s2 = Session::default();
    let (held, _) = plate_with_hole(&mut s2);
    let r2 = freedom_of(&s2, "Plate", &[(held, SupportKind::Fixed)]);
    let mut s3 = Session::default();
    let le1_body = le1(&mut s3);
    let (y0, x0, z0) = (
        face_facing(&mut s3, &le1_body, [0.0, -1.0, 0.0]),
        face_facing(&mut s3, &le1_body, [-1.0, 0.0, 0.0]),
        face_facing(&mut s3, &le1_body, [0.0, 0.0, -1.0]),
    );
    let rollers = [(y0, SupportKind::Roller(1)), (x0, SupportKind::Roller(0)), (z0, SupportKind::Roller(2))];
    let r3 = freedom_of(&s3, &le1_body, &rollers);
    let no_z = freedom_of(&s3, &le1_body, &rollers[..2]);
    eprintln!(
        "B2 ratios: R1 clamp {:e}, R2 end {:e}, R3 LE1 three rollers {:e}, LE1 without z {:e} ({no_z:?}); tolerance {:e}",
        r1.ratio,
        r2.ratio,
        r3.ratio,
        no_z.ratio,
        fea::model::RIGID_TOLERANCE
    );
    for (name, f) in [("R1", &r1), ("R2", &r2), ("R3", &r3)] {
        assert!(f.held() && f.ratio > 1e3 * fea::model::RIGID_TOLERANCE, "{name}: {f:?}");
    }
    assert!(no_z.slides.len() == 1 && close(no_z.slides[0], [0.0, 0.0, 1.0]) && no_z.turns.is_empty(), "{no_z:?}");

    let steel = json!({"name": "LE1 steel", "youngs_modulus": 210000, "poisson_ratio": 0.3, "yield_strength": 250, "source": "NAFEMS LE1"});
    let supports = json!([
        {"face": [2600, 0, 50], "type": "roller"},
        {"face": [0, 1800, 50], "type": "roller"},
        {"face": [2500, 500, 0], "type": "roller"},
    ]);
    let p = |supports: Value| json!({"body": le1_body, "loads": [{"face": [2298.1, 1944.5, 50], "pressure": "-10 MPa"}], "supports": supports, "material": steel, "dry_run": true});
    run(&mut s3, CMD, p(supports.clone()));
    let two = json!([supports[0], supports[1]]);
    let e = s3.execute(CMD, &p(two)).unwrap_err().to_string();
    assert!(e.contains("it can slide along Z.") && !e.contains("turn about"), "{e}");
    assert!(e.contains("a roller on a face facing along Z stops the slide along Z"), "{e}");
}

/// B3: through the command on a real part, a setup that lets the part move is refused before
/// anything runs. Input: the strip (100 x 20 x 1.5 mm sheet metal) with 10 N down on its far end;
/// held only by a roller on its end face (x = 0, which faces X, so the roller holds along X), and
/// only by a roller on its underside (which holds along Z). Action: `inspect.stress_estimate`, a
/// real request (not a dry run) and a dry run. Observable: the refusal's words; nothing painted.
/// Pass: the end-face roller is refused naming the slides along Y and Z and the turn about an axis
/// along X through (0, 10, 0.75) mm, saying what stops each and that nothing was meshed or
/// solved; the dry run gives the same refusal; the underside roller is refused naming the slides
/// along X and Y and a turn about an axis along Z. (Both come back at once, without the mesher or
/// the solver: the check runs before them.)
#[test]
fn b3_a_strip_held_only_by_a_roller_is_refused_before_anything_runs() {
    let mut s = Session::default();
    let (_, root, tip) = strip(&mut s, 100.0);
    let p = |supports: Value, dry: bool| json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": supports, "material": aluminium(), "elements": "solid", "dry_run": dry});
    let e = s.execute(CMD, &p(json!([{"face": root, "type": "roller"}]), false)).unwrap_err().to_string();
    eprintln!("B3 end-face roller: {e}");
    for needed in [
        "the supports do not hold the part still",
        "it can slide along Y, slide along Z and turn about an axis along X through (0, 10, 0.75) mm",
        "a roller on a face facing along Y stops the slide along Y",
        "a roller on a face facing along Z stops the slide along Z",
        "stops the turn",
        "Nothing was meshed or solved",
    ] {
        assert!(e.contains(needed), "`{needed}` missing: {e}");
    }
    let dry = s.execute(CMD, &p(json!([{"face": root, "type": "roller"}]), true)).unwrap_err().to_string();
    assert_eq!(dry, e, "a dry run refuses the same way");
    let e = s.execute(CMD, &p(json!([{"face": [50, 10, 0], "type": "roller"}]), false)).unwrap_err().to_string();
    eprintln!("B3 underside roller: {e}");
    assert!(e.contains("it can slide along X, slide along Y and turn about an axis along Z through"), "{e}");
    assert!(fea::shown(&s.document_id).is_none(), "nothing ran or was painted");
}

/// G3: the prediction against what was measured (zhikun, 2026-11-16: the R tests' runs and a
/// shell run of R1's strip, the solver's peak resident memory from /usr/bin/time -v).
/// Input: each run's node count and measured peak memory (bytes); each mesh check's first and
/// finer node counts. Action: `solver_bytes`, `predict_finer`. Nothing runs.
/// Observable: predicted bytes and finer nodes against the measured.
/// Pass: every memory prediction is at or above the measurement and at most 35 % above it (the
/// small runs are overestimated most, far under any limit); the finer node count is within 3 %
/// for R2 (at 1.0 and 1.5 mm), R3 and the shells, and above the measurement for R1 (a 1.5 mm
/// sheet, one or two elements thick, refines as a surface: 2.09 times, not 3).
#[test]
fn g3_the_memory_and_node_predictions_cover_what_was_measured() {
    let kib = |k: f64| k * 1024.0;
    for (elements, nodes, measured) in [
        (Elements::Solid, 14_555.0, kib(151_124.0)),
        (Elements::Solid, 30_447.0, kib(327_696.0)),
        (Elements::Solid, 44_748.0, kib(575_760.0)),
        (Elements::Solid, 66_300.0, kib(803_136.0)),
        (Elements::Solid, 132_915.0, kib(2_384_340.0)),
        (Elements::Solid, 199_504.0, kib(3_391_588.0)),
        (Elements::Solid, 595_186.0, kib(13_659_328.0)),
        (Elements::Shell, 26_607.0, kib(1_054_020.0)),
        (Elements::Shell, 47_053.0, kib(1_946_912.0)),
        (Elements::Shell, 58_673.0, kib(2_462_856.0)),
        (Elements::Shell, 104_175.0, kib(4_555_344.0)),
    ] {
        let predicted = solver_bytes(nodes, elements);
        eprintln!(
            "G3 {} {nodes} nodes: measured {:.3} GB, predicted {:.3} GB ({:+.1}%)",
            elements.word(),
            measured / 1e9,
            predicted / 1e9,
            100.0 * (predicted / measured - 1.0)
        );
        assert!(predicted >= measured && predicted <= 1.35 * measured, "{nodes}: {predicted} vs {measured}");
    }
    for (name, elements, first, finer) in [
        ("R2", Elements::Solid, 199_504, 595_186.0),
        ("R2 at 1.5 mm", Elements::Solid, 66_300, 199_504.0),
        ("R3", Elements::Solid, 44_748, 132_915.0),
        ("shells", Elements::Shell, 26_607, 58_673.0),
    ] {
        let (nodes, _) = predict_finer(first, elements, 1.0);
        eprintln!("G3 {name}: finer nodes predicted {nodes:.0}, measured {finer} ({:+.1}%)", 100.0 * (nodes / finer - 1.0));
        assert!((nodes / finer - 1.0).abs() <= 0.03, "{name}: {nodes} vs {finer}");
    }
    let (r1, _) = predict_finer(14_555, Elements::Solid, 1.0);
    eprintln!("G3 R1: finer nodes predicted {r1:.0}, measured 30447 ({:+.1}%)", 100.0 * (r1 / 30_447.0 - 1.0));
    assert!((30_447.0..=1.5 * 30_447.0).contains(&r1), "R1: {r1}");
}

/// A2: a solid part's default element size is about a 25th of it and half its thinnest side,
/// but no finer than a mesh of about 80 000 nodes for its volume, so that its mesh check's finer
/// run fits even at the densest mesh measured (D9). R2's plate (200 x 50 x 2 mm, a
/// Ø10 hole) was meshed at 1.0 mm, too fine for its mesh check to fit; a compact part keeps its
/// detail size; with no volume known the detail size stands.
#[test]
fn a_solid_parts_default_mesh_is_no_finer_than_about_eighty_thousand_nodes() {
    let plate_volume = 200.0 * 50.0 * 2.0 - std::f64::consts::PI * 25.0 * 2.0;
    let plate_diag = (200.0f64 * 200.0 + 50.0 * 50.0 + 4.0).sqrt();
    let h = default_solid_size(plate_diag, 2.0, Some(plate_volume));
    assert!((h - (SOLID_NODES_PER_MM3 * plate_volume / DEFAULT_SOLID_NODES).cbrt()).abs() < 1e-12 && h > 1.3 && h < 1.4, "{h}");
    // The finer run fits at the densest mesh measured, for the plate and for D9's thin disc
    // (Ø100 x 2 mm), whose mesh check the ship review found skipped.
    // The most nodes per mm³ measured of a curvature-sized mesh (see `SOLID_NODES_PER_MM3`).
    const SOLID_NODES_PER_MM3_MOST: f64 = 11.3;
    let disc_volume = std::f64::consts::PI * 50.0 * 50.0 * 2.0;
    let disc_diag = (100.0f64 * 100.0 + 100.0 * 100.0 + 4.0).sqrt();
    // Their surfaces, for the layers through them (decision 22).
    let pi = std::f64::consts::PI;
    let plate_area = 2.0 * (200.0 * 50.0 - pi * 25.0) + 2.0 * (200.0 + 50.0) * 2.0 + pi * 10.0 * 2.0;
    let disc_area = 2.0 * pi * 50.0 * 50.0 + pi * 100.0 * 2.0;
    for (what, v, area, diag) in [("plate", plate_volume, plate_area, plate_diag), ("disc", disc_volume, disc_area, disc_diag)] {
        let h = default_solid_size(diag, 2.0, Some(v));
        let nodes = SOLID_NODES_PER_MM3_MOST * v / h.powi(3);
        let deep = forecast::layers(section_size(Elements::Solid, false, 0.0, Some(v), area), h / MESH_CHECK_RATIO);
        let (finer_nodes, finer_bytes) = predict_finer(nodes as usize, Elements::Solid, deep);
        eprintln!("A2 {what}: finer run {finer_nodes:.0} nodes, {deep:.2} layers, {:.2} GB", finer_bytes / 1e9);
        assert!(finer_bytes <= MEMORY_CEILING, "{what}: the finer run fits: {finer_nodes} nodes, {finer_bytes} bytes");
    }
    let bar = default_solid_size((140.0f64 * 140.0 + 28.0 * 28.0 + 16.0).sqrt(), 4.0, Some(13_120.0));
    assert_eq!(bar, 2.0, "the Peterson bar keeps two elements through its 4 mm");
    assert_eq!(default_solid_size(plate_diag, 2.0, None), 1.0);
    assert_eq!(default_solid_size(1.0, 0.1, Some(0.01)), 0.3, "never under 0.3 mm by default");
    for v in [Some(f64::NAN), Some(-1.0), Some(f64::INFINITY), Some(0.0)] {
        assert_eq!(default_solid_size(plate_diag, 2.0, v), 1.0, "{v:?}");
    }
}

/// A bar 100 x 20 x `t` mm along X with a Ø`d` hole through its middle (along Z), made with the
/// app's commands: its end faces at x = 0 and x = 100.
fn bar_with_hole(s: &mut Session, t: f64, d: f64) -> (u64, u64) {
    run(s, "solid.box", json!({"length": 100, "width": 20, "height": t, "body_name": "Bar"}));
    run(s, "solid.cylinder", json!({"diameter": d, "height": t, "base": [50, 10, 0], "operation": "cut", "targets": ["Bar"]}));
    let faces = run(s, "model.faces", json!({"body": "Bar"}));
    let end = |x: f64| {
        faces["faces"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| {
                (f["centroid"][0].as_f64().unwrap_or(f64::NAN) - x).abs() < 1e-6 && f["plane_normal"][0].as_f64().is_some_and(|n| n.abs() > 0.99)
            })
            .and_then(|f| f["index"].as_u64())
            .unwrap_or_else(|| panic!("no end face at x = {x}: {faces}"))
    };
    (end(0.0), end(100.0))
}

/// V-A2, real: bars 100 x 20 x T mm (T 5 and 10) with one hole of Ø1, 2, 3, 4 or 6 mm through
/// the middle, clamped at one end with 10 N down at the other, 6061-T6, at the default settings
/// (the mesh check off, to keep it short). Before curvature sizing 5 of the 10 failed: gmsh's
/// "overlapping facets" or CalculiX's "nonpositive jacobian" (PIL-294 ship review).
/// Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// Pass: all 10 solve, and the forces balance within 1 %.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn va2_bars_with_small_holes_all_mesh_and_solve_at_the_default_settings() {
    let mut failed = Vec::new();
    for t in [5.0, 10.0] {
        for d in [1.0, 2.0, 3.0, 4.0, 6.0] {
            let mut s = Session::default();
            let (held, end) = bar_with_hole(&mut s, t, d);
            let p = json!({"loads": [{"face": end, "force": [0, 0, -10]}], "supports": [{"face": held}], "material": "6061-T6", "mesh_check": false});
            match s.execute(CMD, &p) {
                Ok(r) => {
                    eprintln!(
                        "VA2 T{t} Ø{d}: {} nodes at {} mm, away peak {} MPa, {}",
                        r["runs"][0]["nodes"], r["runs"][0]["mesh_size_mm"], r["peak_away"]["von_mises_mpa"], r["equilibrium"]["line"]
                    );
                    if number(&r["equilibrium"]["mismatch_percent"]) >= 1.0 {
                        failed.push(format!("T{t} Ø{d}: {}", r["equilibrium"]));
                    }
                }
                Err(e) => {
                    eprintln!("VA2 T{t} Ø{d}: {e}");
                    failed.push(format!("T{t} Ø{d}: {e}"));
                }
            }
        }
    }
    assert!(failed.is_empty(), "{} of 10 failed:\n{}", failed.len(), failed.join("\n"));
}

/// V-A3, real: a hole too small for even the smallest element (a Ø0.02 mm hole in a 100 x 20 x
/// 10 mm bar meshed at 4 mm, whose smallest element is 0.08 mm). A solver the system kills is
/// `fea::fea_tests::a_program_the_system_kills_reads_as_out_of_memory`.
/// Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// Pass: it fails with plain advice that names a `mesh_size` to try, without the programs' own
/// text (no "gpl.htm", "*ERROR", "Error   :", "signal"); or it meshes after all, which is said.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn va3_failures_read_as_plain_advice_without_the_programs_text() {
    let clean = |e: &str| {
        for raw in ["gpl.htm", "*ERROR", "Error   :", "signal", "exit status", "WARRANTY"] {
            assert!(!e.contains(raw), "`{raw}` in: {e}");
        }
    };
    let mut s = Session::default();
    let (held, end) = bar_with_hole(&mut s, 10.0, 0.02);
    let p = json!({"loads": [{"face": end, "force": [0, 0, -10]}], "supports": [{"face": held}], "material": "6061-T6", "mesh_check": false, "mesh_size": 4});
    match s.execute(CMD, &p) {
        Ok(r) => eprintln!("VA3 the Ø0.02 mm hole meshed after all: {} nodes", r["runs"][0]["nodes"]),
        Err(e) => {
            let e = e.to_string();
            eprintln!("VA3 tiny hole: {e}");
            clean(&e);
            assert!(e.contains("`mesh_size`, such as"), "{e}");
        }
    }
}

/// D1: the memory a run may use is half the memory free, or 8 GB where that is unknown; a
/// control group's limit (a container, `systemd-run -p MemoryMax=8G`) counts as what is free.
#[test]
fn d1_a_runs_memory_limit_is_half_the_memory_free_within_its_control_group() {
    let files = |path: &str, file: &str| -> Option<String> {
        match (path, file) {
            ("/user.slice/run-1.service", "memory.max") => Some("8000000000\n".into()),
            ("/user.slice/run-1.service", "memory.current") => Some("1000000000\n".into()),
            ("/user.slice", "memory.max") => Some("max\n".into()),
            ("/user.slice", "memory.current") => Some("3000000000\n".into()),
            _ => None,
        }
    };
    assert_eq!(cgroup_room("0::/user.slice/run-1.service\n", files), Some(7.0e9));
    assert_eq!(cgroup_room("0::/user.slice\n", files), None, "max: no limit");
    assert_eq!(cgroup_room("0::/\n", files), None);
    assert_eq!(cgroup_room("12:memory:/x\n", files), None, "cgroup v1 is not read");
    assert_eq!(first_run_limit(Some(8.0e9)), 4.0e9);
    for unknown in [None, Some(f64::NAN), Some(-1.0), Some(0.0)] {
        assert_eq!(first_run_limit(unknown), FIRST_RUN_FALLBACK, "{unknown:?}");
    }
    for e in [Elements::Solid, Elements::Shell] {
        for b in [1.0e9, 4.0e9, 12.0e9] {
            for deep in [1.0, 3.7] {
                let back = forecast::memory(nodes_for_bytes(b, e, deep), e, deep);
                assert!((back / b - 1.0).abs() < 1e-9, "{e:?} {b} {deep}: {back}");
            }
        }
    }
}

/// D1 and V-A1: a run too big for the memory is refused before the solver starts, naming the
/// memory it would need and a `mesh_size` that fits. R2's plate (200 x 50 x 2 mm, a Ø10 hole) at
/// 0.7 mm under an 8 GB cap (4 GB a run) is refused from its volume, before meshing; its mesh
/// (493 468 nodes with curvature sizing) is refused too; at the suggested size it fits.
#[test]
fn d1_a_run_too_big_for_the_memory_is_refused_with_a_mesh_size_that_fits() {
    let pi = std::f64::consts::PI;
    let v = 200.0 * 50.0 * 2.0 - pi * 25.0 * 2.0;
    // Twice the volume over the surface: 1.9 mm, 2.7 layers at 0.7 mm.
    let section = 2.0 * v / (2.0 * (200.0 * 50.0 - pi * 25.0) + 2.0 * (200.0 + 50.0) * 2.0 + pi * 10.0 * 2.0);
    let cap = 4.0e9;
    let predicted = SOLID_NODES_PER_MM3 * v / 0.7f64.powi(3);
    let words = memory_refusal(predicted, Elements::Solid, 0.7, section, cap, false).unwrap_or_else(|| panic!("0.7 mm under 4 GB is refused"));
    eprintln!("D1 R2 at 0.7 mm: {words}");
    assert!(words.starts_with("not solved: the solid mesh at about 0.7 mm would have about "), "{words}");
    assert!(words.contains("GB of memory; this computer has about 4 GB to spare") && words.contains("`mesh_size`, such as "), "{words}");
    let size: f64 = words
        .split("such as ")
        .nth(1)
        .and_then(|t| t.split(" mm").next())
        .and_then(|t| t.parse().ok())
        .unwrap_or_else(|| panic!("no size in: {words}"));
    let fits = forecast::memory(SOLID_NODES_PER_MM3 * v / size.powi(3), Elements::Solid, forecast::layers(section, size));
    assert!(fits <= cap, "{size} mm fits: {fits}");
    assert!(size < 1.3, "not far coarser than needed: {size}");
    let meshed = memory_refusal(493_468.0, Elements::Solid, 0.7, section, cap, true).unwrap_or_else(|| panic!("the mesh is refused"));
    assert!(meshed.contains(" has about 490,000 nodes"), "{meshed}");
    assert_eq!(memory_refusal(493_468.0, Elements::Solid, 0.7, section, 13.0e9, true), None, "half of 26 GB free: it fits");
    assert_eq!(memory_refusal(predicted, Elements::Solid, 0.7, section, 13.0e9, false), None, "a prediction is refused only when twice over");
    let shell = memory_refusal(150_000.0, Elements::Shell, 1.0, 1.5, 2.0e9, true).unwrap_or_else(|| panic!("150 000 shell nodes under 2 GB"));
    assert!(shell.starts_with("not solved: the shell mesh"), "{shell}");
    for raw in ["gpl.htm", "signal", "*ERROR"] {
        assert!(!words.contains(raw) && !shell.contains(raw));
    }
    for junk in [f64::NAN, 0.0, -5.0] {
        assert_eq!(memory_refusal(junk, Elements::Solid, 1.0, section, cap, true), None);
        assert_eq!(memory_refusal(1.0e7, Elements::Solid, junk, section, cap, true), None);
        // An unknown section counts one layer: ten million nodes are still refused.
        assert!(memory_refusal(1.0e7, Elements::Solid, 1.0, junk, cap, true).is_some(), "{junk}");
    }
}

/// Decision 22: A1's memory gate and the mesh check's gate predict memory with A13's model, the
/// card's, which counts the element layers through the part, not `solver_bytes`, which was up to
/// 48 % low on deep solids.
/// Input, measured: V-6's filleted bar (`stepped_bar`, its section 2V/S 3.3 mm) at its third mesh,
/// 2/2.25 mm, counted 196,605 nodes (3.7 layers); the 50 mm cube with a Ø20 hole at 1.7 mm, 156,016
/// nodes (7.3 layers), whose solver peaked at 5.78 GB ([`A13_MEASURED`]). Limits between the two
/// models' figures: 4.4 GB a run for V-6, 3.5 GB for the cube (7 GB free). Nothing runs.
/// Action: `memory_refusal` (A1's gate, on the count and on the prediction from the volume),
/// `finer_gate` (the mesh check's gate on a counted mesh), `mesh_check_gate` (on one predicted from
/// the mesh before, a third as many), `forecast::after_meshing` (the card), `reach_after_first`.
/// Observable: refused or let through, the memory each gate names, the layers each gate is asked at.
/// Pass: `solver_bytes` under each limit (the old gates let both through) and A13's model over it;
/// every gate refuses both, naming the card's memory to the byte; the cube's measured peak is over
/// its limit; the size A1's refusal suggests fits; the forecast asks the gate at the layers of the
/// mesh it would let through.
#[test]
fn d22_the_gates_refuse_a_deep_solid_that_solver_bytes_let_through() {
    // V-6's bar: 60 x 28 then 80 x 20 mm, 4 mm thick; its 2 mm fillets change the section by under 0.1 %.
    let (top, rim) = (60.0 * 28.0 + 80.0 * 20.0, 28.0 + 2.0 * 60.0 + 8.0 + 2.0 * 80.0 + 20.0);
    let v6_section = section_size(Elements::Solid, false, 0.0, Some(top * 4.0), 2.0 * top + rim * 4.0);
    let v6_h = 2.0 / (MESH_CHECK_RATIO * MESH_CHECK_RATIO);
    let (cube_volume, _, cube_section) = a13_part("block");
    for (name, nodes, h, section, cap, measured) in
        [("V-6's third mesh", 196_605.0, v6_h, v6_section, 4.4e9, None), ("the cube at 1.7 mm", 156_016.0, 1.7, cube_section, 3.5e9, Some(5.784e9))]
    {
        let deep = forecast::layers(section, h);
        let (old, card) = (solver_bytes(nodes, Elements::Solid), forecast::memory(nodes, Elements::Solid, deep));
        eprintln!(
            "D22 {name}: {nodes} nodes, {deep:.2} layers; solver_bytes {:.2} GB, A13 {:.2} GB, limit {:.1} GB",
            old / 1e9,
            card / 1e9,
            cap / 1e9
        );
        assert!(old < cap && card > cap, "{name}: the old model lets it through and A13's does not");
        assert!(measured.is_none_or(|m| m > cap), "{name}: it needed more than the limit");
        let words = memory_refusal(nodes, Elements::Solid, h, section, cap, true).unwrap_or_else(|| panic!("{name}: A1's gate refuses the count"));
        assert!(words.contains(&format!("would need about {} of memory", about_bytes(card))), "{words}");
        let size: f64 = words
            .split("such as ")
            .nth(1)
            .and_then(|t| t.split(" mm").next())
            .and_then(|t| t.parse().ok())
            .unwrap_or_else(|| panic!("no size in: {words}"));
        let at_size = forecast::memory(nodes * (h / size).powi(3), Elements::Solid, forecast::layers(section, size));
        eprintln!("D22 {name}: refused, {size} mm suggested ({:.2} GB there)", at_size / 1e9);
        assert!(at_size <= cap, "{name}: {size} mm fits");
        let limit = Limit::Memory(cap);
        assert_eq!(finer_gate(nodes, Elements::Solid, deep, limit), Some(Check::Skipped { nodes, bytes: card, limit }), "{name}");
        let before = (nodes / SOLID_GROWTH).floor() as usize;
        let (predicted, bytes) = predict_finer(before, Elements::Solid, deep);
        assert!(bytes > cap && solver_bytes(predicted, Elements::Solid) < cap, "{name}: {predicted} predicted");
        assert_eq!(mesh_check_gate(before, Elements::Solid, deep, limit), Some(Check::Skipped { nodes: predicted, bytes, limit }), "{name}");
    }
    // Before meshing, from the cube's volume: refused when twice over, which the old model was not.
    let predicted = SOLID_NODES_PER_MM3 * cube_volume.unwrap_or(f64::NAN) / 1.7f64.powi(3);
    assert!(solver_bytes(predicted, Elements::Solid) < 2.0 * 3.5e9, "the old model let it mesh");
    assert!(memory_refusal(predicted, Elements::Solid, 1.7, cube_section, 3.5e9, false).is_some(), "refused before meshing");

    // The card shows what the gates refuse by, and the forecast asks the gate at the next mesh's layers.
    let planned: Vec<forecast::Planned> = (0..3u32)
        .map(|level| forecast::Planned {
            elements: Elements::Solid,
            h: 2.0 / MESH_CHECK_RATIO.powi(level as i32),
            section: v6_section,
            nodes: 20_000.0 * 3f64.powi(level as i32),
            level,
        })
        .collect();
    let card = forecast::after_meshing(&planned, 2, 196_605, forecast::Reach { likely: 2, most: 2 }, "doc-d22").map(|f| f.bytes);
    let deep = forecast::layers(v6_section, v6_h);
    let gate = finer_gate(196_605.0, Elements::Solid, deep, Limit::Memory(4.4e9));
    assert_eq!(gate.and_then(|g| if let Check::Skipped { bytes, .. } = g { Some(bytes) } else { None }), card, "one model");
    let asked = std::cell::RefCell::new(Vec::new());
    let ask = |_: f64, _: Elements, deep: f64| {
        asked.borrow_mut().push(deep);
        true
    };
    assert_eq!(forecast::reach_after_first(&planned, 20_000.0, &ask), forecast::Reach { likely: 1, most: 2 });
    assert_eq!(*asked.borrow(), [planned[1].layers(), planned[2].layers()]);
}

/// Decision 22: where Linux's memory cannot be read, the gates take the host's reading (the desktop
/// app's on Windows and macOS, `fea::set_memory_reader`), and assume 8 GB a run only without one.
/// Input: a Linux reading of 8 GB or none; a host reader giving 9.6 GB, none, NaN, 0, or no reader
/// at all (the CLI on Windows and macOS). Nothing is registered: `memory_available` takes both.
/// Action: `memory_available`, `first_run_limit`, `finer_limit`; then `memory_available_here` on
/// the computer the test runs on.
/// Observable: the bytes available and the limits from them.
/// Pass: Linux's reading first; without it the host's 9.6 GB, so a run and the mesh check may use
/// 4.8 GB; with no usable reading, the 8 GB fallback and the mesh check's 6 GB ceiling; on Linux
/// this computer's memory is read.
#[test]
fn d22_the_hosts_memory_reading_stands_in_where_linux_cannot_be_read() {
    let host: fn() -> Option<f64> = || Some(9.6e9);
    let (nothing, nan, zero): (fn() -> Option<f64>, fn() -> Option<f64>, fn() -> Option<f64>) = (|| None, || Some(f64::NAN), || Some(0.0));
    assert_eq!(memory_available(|| Some(8.0e9), Some(host)), Some(8.0e9), "Linux's own reading first");
    let read = memory_available(|| None, Some(host));
    assert_eq!(read, Some(9.6e9));
    assert_eq!(first_run_limit(read), 4.8e9);
    assert_eq!(finer_limit(None, read, MEMORY_CEILING), Limit::Memory(4.8e9));
    for reader in [Some(nothing), Some(nan), Some(zero), None] {
        let read = memory_available(|| None, reader);
        assert_eq!(read, None);
        assert_eq!(first_run_limit(read), FIRST_RUN_FALLBACK);
        assert_eq!(finer_limit(None, read, MEMORY_CEILING), Limit::Memory(MEMORY_CEILING));
    }
    let here = memory_available_here();
    eprintln!("D22 available here: {:?} GB", here.map(|b| (b / 1e8).round() / 10.0));
    if cfg!(target_os = "linux") {
        assert!(here.is_some_and(|b| b > 0.0), "{here:?}");
    }
}

/// V-A1, real: R2's plate at 0.7 mm with a run limit of 4 GB (an 8 GB computer) is refused before
/// anything is meshed. Run it under `systemd-run --user -p MemoryMax=8G -p MemorySwapMax=0` to see
/// the control group read; without it the limit is half the memory free here, and the case is
/// refused only on a computer with less than about 29 GB free.
/// Real: the app's modelling and memory reading. Faked: nothing.
/// Pass: refused naming a `mesh_size`, within 15 s when the volume alone shows the mesh too big
/// (and within the meshing time when the mesh must be counted first), or (with much memory free)
/// it says so.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn va1_a_mesh_too_big_for_the_memory_is_refused_before_solving() {
    let cap = first_run_limit(memory_available_here());
    eprintln!("VA1 run limit here: {:.1} GB", cap / 1.0e9);
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 200, "width": 50, "height": 2, "body_name": "Plate"}));
    run(&mut s, "solid.cylinder", json!({"diameter": 10, "height": 2, "base": [100, 25, 0], "operation": "cut", "targets": ["Plate"]}));
    let p = json!({"supports": [{"face": [0, 25, 1]}], "loads": [{"face": [200, 25, 1], "force": [800, 0, 0]}], "material": "6061-T6", "mesh_size": 0.7, "mesh_check": false});
    let started = std::time::Instant::now();
    let r = s.execute(CMD, &p);
    let took = started.elapsed().as_secs_f64();
    match r {
        Err(e) => {
            let e = e.to_string();
            eprintln!("VA1 refused in {took:.1} s: {e}");
            assert!(e.contains("not solved: the solid mesh") && e.contains("`mesh_size`, such as"), "{e}");
            // Refused from the volume alone when the prediction is over twice the limit; between
            // one and two times it, the mesh is made and counted first, which takes longer.
            let volume = 200.0 * 50.0 * 2.0 - std::f64::consts::PI * 25.0 * 2.0;
            let section = 2.0 * 19858.6 / 20919.8;
            let before = memory_refusal(SOLID_NODES_PER_MM3 * volume / 0.7f64.powi(3), Elements::Solid, 0.7, section, cap, false).is_some();
            assert!(took < if before { 15.0 } else { 180.0 }, "{took} s (refused before meshing: {before})");
        }
        Ok(r) => {
            eprintln!("VA1 solved in {took:.0} s ({} nodes): this computer has the memory", r["runs"][0]["nodes"]);
            assert!(cap > 8.0e9, "solved under a {cap} limit");
        }
    }
}

/// D5: 304L is its own grade at the ASTM A240 minimum for 304L (170 MPa), not 304's 205.
/// Input: the grade names "304L", "1.4307", "304" and "1.4301". Action: `fea::materials::grade`.
/// Pass: the L names find 304L at 170 MPa with an ASTM source; the plain names find 304 at 205.
#[test]
fn d5_304l_is_its_own_grade_at_its_own_minimum_yield() {
    for name in ["304L", "304 L", "1.4307", "AISI 304L"] {
        let g = fea::materials::grade(name).unwrap_or_else(|| panic!("{name} is a grade"));
        assert!(g.name == "304L stainless steel" && g.yield_mpa == 170.0 && g.source.contains("ASTM A240"), "{name}: {}", g.name);
    }
    for name in ["304", "1.4301"] {
        assert_eq!(fea::materials::grade(name).map(|g| (g.name, g.yield_mpa)), Some(("304 stainless steel", 205.0)), "{name}");
    }
}

/// D6: a setting the estimate does not take is refused, not dropped.
/// Input: a valid setup with one extra key at a time: a remote load's `at`, top-level
/// `load_cases`, a support's `pin`, a material's `density`. Action: `check_keys`, and the
/// command as a dry run. Observable: the error. Pass: each is refused, naming the key and where it
/// was, with what it means where that says more (a remote load, load cases); the setup without
/// the extra key passes.
#[test]
fn d6_a_setting_the_estimate_does_not_take_is_refused_not_dropped() {
    let base = json!({"loads": [{"face": 1, "force": [0, 0, -5]}], "supports": [{"face": 0, "type": "fixed"}],
        "material": {"name": "x", "youngs_modulus": 70000, "poisson_ratio": 0.33, "yield_strength": 200, "source": "s"},
        "mesh_size": 1.0, "dry_run": true});
    assert!(check_keys(&base).is_ok());
    let with = |path: &[&str], key: &str| {
        let mut v = base.clone();
        let mut at = &mut v;
        for p in path {
            at = match p.parse::<usize>() {
                Ok(i) => &mut at[i],
                Err(_) => &mut at[*p],
            };
        }
        at[key] = json!(1);
        check_keys(&v).map(|_| ()).unwrap_err().to_string()
    };
    let e = with(&["loads", "0"], "at");
    assert!(e.contains("loads[0]: `at`") && e.contains("remote force"), "{e}");
    let e = with(&[], "load_cases");
    assert!(e.contains("`load_cases`") && e.contains("once for each load case"), "{e}");
    let e = with(&["supports", "0"], "pin");
    assert!(e.contains("supports[0]: `pin`") && e.contains("face, type"), "{e}");
    let e = with(&["material"], "density");
    assert!(e.contains("material: `density`"), "{e}");
    // Through the command too, before anything else is looked at.
    let mut s = Session::default();
    let (_, held, tip) = strip(&mut s, 100.0);
    let e = s
        .execute(
            CMD,
            &json!({"loads": [{"face": tip, "force": [0, 0, -10], "at": [200, 0, 0]}], "supports": [{"face": held}], "material": aluminium(), "dry_run": true}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("`at`"), "{e}");
}

/// The keys `check_keys` takes are the contracts' own: none missing, none extra.
#[test]
fn d6_the_known_keys_are_the_contracts_fields() {
    let schema = schemars::schema_for!(crate::schema::stress::StressEstimateParams);
    let v = schema.as_value();
    let props = |o: &Value| -> Vec<String> {
        let mut k: Vec<String> = o["properties"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
        k.sort();
        k
    };
    let sorted = |k: &[&str]| -> Vec<String> {
        let mut k: Vec<String> = k.iter().map(|x| x.to_string()).collect();
        k.sort();
        k
    };
    let defs = if v["$defs"].is_object() { &v["$defs"] } else { &v["definitions"] };
    assert_eq!(props(v), sorted(ESTIMATE_KEYS));
    assert_eq!(props(&defs["LoadParam"]), sorted(LOAD_KEYS));
    assert_eq!(props(&defs["SupportParam"]), sorted(SUPPORT_KEYS));
    assert_eq!(props(&defs["GivenMaterial"]), sorted(MATERIAL_KEYS));
    assert_eq!(props(&defs["ProbeParam"]), sorted(PROBE_KEYS));
}

/// D7: points on a cylinder (radius 2 about Z) with their outward normals, over `span` degrees.
fn round_points(span: f64, part_outside: bool) -> Vec<(P, P)> {
    (0..=40)
        .map(|i| {
            let t = (span * f64::from(i) / 40.0).to_radians();
            let (c, s) = (t.cos(), t.sin());
            let n = if part_outside { [-c, -s, 0.0] } else { [c, s, 0.0] };
            ([2.0 * c, 2.0 * s, f64::from(i % 3)], n)
        })
        .collect()
}

/// D7: a round face is named for what it is, not always "a hole".
/// Input: points on a cylinder over 360°, 90° and 180°, with normals toward the axis (the part
/// outside it) or away (the part inside). Action: `round_kind`, `Round::words`. Pass: a full turn
/// toward the axis is a hole, away a round outside face; a quarter turn is a fillet or a rounded
/// edge; half a turn a curved inside or outside face; holes keep Ø and the rest R.
#[test]
fn d7_a_round_face_is_named_for_what_it_is() {
    let kind = |span: f64, inside: bool| round_kind(&round_points(span, inside), [0.0, 0.0, 1.0], [0.0; 3]);
    assert_eq!(kind(360.0, true), Round::Hole);
    assert_eq!(kind(360.0, false), Round::Outside);
    assert_eq!(kind(90.0, true), Round::Fillet);
    assert_eq!(kind(90.0, false), Round::Rounded);
    assert_eq!(kind(180.0, true), Round::InsideCurve);
    assert_eq!(kind(180.0, false), Round::OutsideCurve);
    assert_eq!(Round::Hole.words(2, 3.3), "2 holes Ø6.60");
    assert_eq!(Round::Outside.words(1, 50.0), "a round outside face Ø100");
    assert_eq!(Round::Fillet.words(1, 2.0), "a fillet R2.00");
    assert_eq!(Round::Rounded.words(3, 2.0), "3 rounded edges R2.00");
}

/// D7 in the app: a sheet's bend is not called a hole, and a hole still is.
/// Input: the angle bracket (a base, a flange bent up, two Ø6.6 holes). Action: dry runs held at
/// a hole, and at the bend's round faces. Observable: `setup`. Pass: the hole reads "a hole
/// Ø6.60"; the bend never reads as a hole.
#[test]
fn d7_a_bend_is_not_called_a_hole() {
    let mut s = Session::default();
    let hole = angle(&mut s);
    let body = s.model.state().bodies.first().map(|b| b.name.clone()).unwrap_or_default();
    let faces = run(&mut s, "model.faces", json!({"body": body}));
    let bend: Vec<u64> = faces["faces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["cylinder"]["radius"].as_f64().is_some_and(|r| (r - 3.3).abs() > 1e-6))
        .filter_map(|f| f["index"].as_u64())
        .collect();
    assert!(!hole.is_empty() && !bend.is_empty(), "a hole and a bend: {faces}");
    let mut setup = |held: Value| {
        run(
            &mut s,
            CMD,
            json!({"loads": [{"face": [5.0, 5.0, 1.5], "force": [0, 0, -5]}], "supports": [{"face": held}], "material": aluminium(), "dry_run": true}),
        )["setup"]
            .to_string()
    };
    let at_hole = setup(json!(hole[0]));
    assert!(at_hole.contains("Held still: a hole Ø6.60"), "{at_hole}");
    let at_bend = setup(json!(bend));
    assert!(!at_bend.contains("hole"), "{at_bend}");
}

/// D8: one share of yield, the one the summary leads with, and a past-yield warning that names the
/// peak that passed it.
/// Input: peaks at 300 MPa overall against a 240 MPa yield, with the overall peak at a support or
/// not, and the away peak under or over yield, or none. Action: `lead_stress`, `yield_warning`.
/// Pass: the lead is the away peak only when the overall sits at a support; the warning says
/// "only at" the support when the away peak is under yield, quotes the away peak when it is over,
/// says nothing can be read with no away node, and keeps the plain words without a support peak;
/// no warning under yield.
#[test]
fn d8_one_share_of_yield_and_the_warning_names_the_peak_that_passed_it() {
    let peaks = |at: bool, away: Option<f64>| fea::Peaks {
        at: at.then(|| "the fixed face".to_string()),
        away: away.map(|v| (v, [1.0, 2.0, 3.0])),
        distance: 3.0,
    };
    let near = |_: P| "the hole".to_string();
    assert_eq!(lead_stress(300.0, &peaks(true, Some(120.0))), 120.0);
    assert_eq!(lead_stress(300.0, &peaks(false, Some(120.0))), 300.0);
    assert_eq!(lead_stress(300.0, &peaks(true, None)), 300.0);
    let w = |at, away| yield_warning(300.0, &peaks(at, away), 240.0, &near).unwrap_or_default();
    let only = w(true, Some(120.0));
    assert!(only.contains("only at the fixed face") && only.contains("120 MPa, 50.0% of yield") && !only.contains("would yield and"), "{only}");
    let over = w(true, Some(250.0));
    assert!(over.starts_with("Away from the supports and loads, the highest stress (250 MPa, near the hole)"), "{over}");
    assert!(w(true, None).contains("cannot be read from this mesh"));
    let plain = w(false, Some(250.0));
    assert!(plain.starts_with("The highest stress (300 MPa) is above") && plain.contains("would yield"), "{plain}");
    assert_eq!(yield_warning(200.0, &peaks(true, Some(120.0)), 240.0, &near), None);
}

/// D9, real: a thin solid part gets its mesh check at the default settings.
/// Input: a Ø100 x 2 mm disc, held at its rim, 100 N down on its top face, 6061-T6, no mesh size
/// given. Action: the estimate. Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// Pass: the mesh check ran (where this computer allows a 6 GB finer run and no node cap is set);
/// `fraction_of_yield` is the share of the number the summary leads with and
/// `fraction_of_yield_overall` the overall peak's.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn d9_a_thin_disc_gets_its_mesh_check_at_the_default_settings() {
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"diameter": 100, "height": 2, "body_name": "Disc"}));
    let p = json!({"supports": [{"face": [50, 0, 1]}], "loads": [{"face": [0, 0, 2], "force": [0, 0, -100]}], "material": "6061-T6"});
    let r = run(&mut s, CMD, p);
    let line = r["mesh_check"]["line"].as_str().unwrap_or("");
    eprintln!(
        "D9 disc: {} nodes at {} mm, mesh check {line}; fraction {} (overall {}); times {:?} s",
        r["runs"][r["runs"].as_array().map_or(0, |a| a.len().saturating_sub(1))]["nodes"],
        r["runs"][r["runs"].as_array().map_or(0, |a| a.len().saturating_sub(1))]["mesh_size_mm"],
        r["fraction_of_yield"],
        r["fraction_of_yield_overall"],
        r["runs"].as_array().map(|a| a.iter().map(|x| number(&x["seconds"])).collect::<Vec<_>>())
    );
    let roomy = matches!(finer_limit_here(), Limit::Memory(cap) if cap >= MEMORY_CEILING);
    if roomy {
        assert_eq!(r["mesh_check"]["status"], "ran", "{line}");
    }
    let yield_mpa = 240.0;
    let overall = number(&r["runs"][0]["max_von_mises_mpa"]);
    // The away peak leads only when there is one: a load over the whole top face leaves no node
    // away from supports and loads.
    let lead = if r["peak_at_support"].is_string() && r["peak_away"].is_object() { number(&r["peak_away"]["von_mises_mpa"]) } else { overall };
    assert!((number(&r["fraction_of_yield"]) - lead / yield_mpa).abs() < 2e-3, "{r}");
    assert!((number(&r["fraction_of_yield_overall"]) - overall / yield_mpa).abs() < 2e-3, "{r}");
}

/// D10: a grade's yield is warned about past the thickness its standard specifies it for.
/// Input: 6061-T6 (to 6.3 mm), S235 (to 16 mm), 304 (no limit) and a material given with the
/// request, at thicknesses inside and outside; then a 100 x 50 x 10 mm 6061-T6 block as a dry run.
/// Action: `grade_thickness_warning`, the command. Pass: a warning naming the limit only for a
/// table grade past its limit; the block's dry run carries it.
#[test]
fn d10_a_grade_past_the_thickness_its_yield_holds_for_is_warned() {
    let g = |n: &str| fea::materials::grade(n).map(|g| g.material()).unwrap_or_else(|| panic!("{n}"));
    let w = grade_thickness_warning(&g("6061-T6"), 10.0, false).unwrap_or_default();
    assert!(w.contains("10.0 mm across its thinnest side") && w.contains("specified only up to 6.3 mm") && w.contains("certificate"), "{w}");
    assert_eq!(grade_thickness_warning(&g("6061-T6"), 6.3, true), None);
    assert!(grade_thickness_warning(&g("S235"), 20.0, true).is_some_and(|w| w.contains("20.0 mm thick") && w.contains("up to 16 mm")));
    assert_eq!(grade_thickness_warning(&g("S355"), 12.0, false), None);
    assert_eq!(grade_thickness_warning(&g("304"), 80.0, false), None);
    let given = Material { name: "x".into(), e_mpa: 70_000.0, nu: 0.33, yield_mpa: 240.0, source: "s".into() };
    assert_eq!(grade_thickness_warning(&given, 80.0, false), None);
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 50, "height": 10, "body_name": "Block"}));
    let r = run(
        &mut s,
        CMD,
        json!({"supports": [{"face": [0, 25, 5]}], "loads": [{"face": [100, 25, 5], "force": [0, 0, -10]}], "material": "6061-T6", "dry_run": true}),
    );
    assert!(r["warnings"].to_string().contains("specified only up to 6.3 mm"), "{}", r["warnings"]);
}

/// D11: a `thickness` that is not the sheet's own is warned about.
/// Input: the angle bracket (a 1.5 mm sheet) as dry runs with `thickness` 3, 1.5 and 1.55, and
/// 3 with both shells and solids. Action: the command, `thickness_mismatch`. Pass: 3 mm warns,
/// naming both thicknesses, and with a solid run says the two runs are of different parts; 1.5
/// and 1.55 (within 5 %) do not.
#[test]
fn d11_a_thickness_that_is_not_the_sheets_own_is_warned() {
    let mut s = Session::default();
    let hole = angle(&mut s);
    let mut warnings = |t: f64, elements: &str| {
        run(
            &mut s,
            CMD,
            json!({"loads": [{"face": [5.0, 5.0, 1.5], "force": [0, 0, -5]}], "supports": [{"face": hole[0]}], "material": "6061-T6",
                   "thickness": t, "elements": elements, "dry_run": true}),
        )["warnings"]
            .to_string()
    };
    let w = warnings(3.0, "shell");
    assert!(w.contains("The thickness given (3.00 mm) is not this sheet's own (1.50 mm)") && !w.contains("two runs"), "{w}");
    assert!(warnings(3.0, "both").contains("the two runs are of different parts"));
    for t in [1.5, 1.55] {
        let w = warnings(t, "shell");
        assert!(!w.contains("not this sheet's own"), "{t}: {w}");
    }
    assert_eq!(thickness_mismatch(1.0, 0.0, false), None);
}

/// C1: the probes' settings are checked before anything runs.
/// Input: a 100 x 50 x 10 mm block; dry runs with a probe named `gauge` along X and an unnamed one,
/// then `probes` as text, 51 probes, a point of two numbers, a zero direction, an empty name, a
/// name of 61 characters, a point off the part and an unknown key. Action: the command with
/// `dry_run: true`. Pass: the setup reads each probe in words (its name or `probe 2`, the point,
/// the face near it, "along X"); each bad one is refused with words naming what is wrong.
#[test]
fn c1_probes_are_checked_and_described_before_anything_runs() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 50, "height": 10, "body_name": "Block"}));
    let setup = |probes: Value| {
        json!({"supports": [{"face": [0, 25, 5]}], "loads": [{"face": [100, 25, 5], "force": [0, 0, -10]}], "material": "6061-T6",
               "dry_run": true, "probes": probes})
    };
    let r = run(&mut s, CMD, setup(json!([{"at": [50, 25, 10], "direction": [2, 0, 0], "name": "gauge"}, {"at": [10, 10, 5]}])));
    let words = r["setup"].to_string();
    assert!(words.contains("Read at gauge: the stress and strain at [50, 25, 10] mm, near ") && words.contains("along X"), "{words}");
    assert!(words.contains("Read at probe 2: the stress and strain at [10, 10, 5] mm"), "{words}");
    let refused = |s: &mut Session, probes: Value| s.execute(CMD, &setup(probes)).map(|_| ()).err().map(|e| e.to_string()).unwrap_or_default();
    let many: Vec<Value> = (0..51).map(|_| json!({"at": [50, 25, 5]})).collect();
    for (probes, says) in [
        (json!("everywhere"), "must be a list"),
        (Value::Array(many), "at most 50 probes"),
        (json!([{"at": [50, 25]}]), "probes[0]: `at` must be a point"),
        (json!([{"at": [50, 25, 5], "direction": [0, 0, 0]}]), "`direction` must be [x, y, z], not zero"),
        (json!([{"at": [50, 25, 5], "name": " "}]), "`name` must be text of 1 to 60 characters"),
        (json!([{"at": [50, 25, 5], "name": "x".repeat(61)}]), "`name` must be text of 1 to 60 characters"),
        (json!([{"at": [50, 25, 5]}, {"at": [500, 25, 5], "name": "far"}]), "far: [500, 25, 5] is outside the part"),
        (json!([{"at": [50, 25, 5], "offset": 1}]), "probes[0]: `offset` is not a setting"),
    ] {
        let e = refused(&mut s, probes.clone());
        assert!(e.contains(says), "{probes}: {e}");
    }
}

/// C1, C2, C8 (V-C6), real: strain in a bar pulled along its axis.
/// Input: a 100 x 10 x 10 mm bar of 6061-T6 (E 68 900 MPa, ν 0.33), held only by symmetry
/// supports on its faces at x = 0, y = 0 and z = 0 (free to thin), 1000 N along X on its end at
/// x = 100; probes at the middle (50, 5, 5) along X and on a side face (30, 10, 5) along Y; the
/// default mesh and the mesh check. Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// By hand: σ = F / A = 10 MPa along X; ε = σ / E = 145.1 µε along X, −ν ε = −47.9 µε across.
/// Pass: at both probes the stress along X is within 2 % of 10 MPa, the principal stresses are
/// 10, 0, 0 within 0.2 MPa, the strains along X and across within 2 % of the hand values
/// (engineering shears under 1 µε); the middle's `along` reads 10 MPa and 145.1 µε within 2 %, the
/// side's −47.9 µε within 2 %; each probe's mesh check ran and is settled.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn c6_a_bar_in_tension_reads_hookes_strain_at_its_probes() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 10, "height": 10, "body_name": "Bar"}));
    let r = run(
        &mut s,
        CMD,
        json!({"supports": [{"face": [0, 5, 5], "type": "symmetry"}, {"face": [50, 0, 5], "type": "symmetry"}, {"face": [50, 5, 0], "type": "symmetry"}],
               "loads": [{"face": [100, 5, 5], "force": [1000, 0, 0]}], "material": "6061-T6",
               "probes": [{"at": [50, 5, 5], "direction": [1, 0, 0], "name": "middle"}, {"at": [30, 10, 5], "direction": [0, 1, 0], "name": "side"}]}),
    );
    let probes = r["probes"].as_array().cloned().unwrap_or_default();
    eprintln!("C6 bar: {}", serde_json::to_string_pretty(&probes).unwrap_or_default());
    assert_eq!(probes.len(), 2, "{r}");
    let (sigma, eps, nu) = (10.0, 10.0 / 68_900.0 * 1e6, 0.33);
    let near = |a: f64, b: f64, tol: f64| (a / b - 1.0).abs() <= tol;
    for pr in &probes {
        let name = pr["name"].as_str().unwrap_or("?");
        assert!(near(number(&pr["stress_mpa"]["xx"]), sigma, 0.02), "{name}: {pr}");
        let p = &pr["principal_stress_mpa"];
        assert!((number(&p[0]) - sigma).abs() < 0.2 && number(&p[1]).abs() < 0.2 && number(&p[2]).abs() < 0.2, "{name}: {p}");
        let e = &pr["strain_microstrain"];
        assert!(
            near(number(&e["xx"]), eps, 0.02) && near(number(&e["yy"]), -nu * eps, 0.02) && near(number(&e["zz"]), -nu * eps, 0.02),
            "{name}: {e}"
        );
        for g in ["gamma_xy", "gamma_yz", "gamma_zx"] {
            assert!(number(&e[g]).abs() < 1.0, "{name} {g}: {e}");
        }
        assert_eq!(pr["mesh_check"]["status"], "settled", "{name}: {pr}");
    }
    assert!(near(number(&probes[0]["along"]["stress_mpa"]), sigma, 0.02) && near(number(&probes[0]["along"]["strain_microstrain"]), eps, 0.02));
    assert!(near(number(&probes[1]["along"]["strain_microstrain"]), -nu * eps, 0.02), "{}", probes[1]);
}

/// C1, C2 (V-C1, V-C2), real: Lamé's thick cylinder read through its wall.
/// Input: the review's quarter ring, a = 20, b = 40 mm, 10 mm long, free ends, 50 MPa inside, S355
/// (E 210 GPa, ν 0.3), symmetry on its cut faces and a roller under it; probes at r = 20, 25, 30,
/// 35 and 40 mm at 45°, mid-length, along the hoop; the default mesh and the mesh check. Real: the
/// app's modelling, gmsh, CalculiX. Faked: nothing.
/// By hand (Lamé): σθ(r) = p a² / (b² − a²) (1 + b² / r²), 83.3 MPa at the bore and 33.3 at the
/// outside; at the bore σr = −p = −50 MPa and σz = 0, so the principal stresses are 83.3, 0 and
/// −50 MPa, and the hoop strain (σθ − ν σr) / E = 468.3 µε.
/// Pass: every probe's hoop stress (`along`) within 1.5 % of σθ(r); at the bore the principal
/// stresses within 2 % (the middle one within 1 MPa of zero) and the largest principal strain
/// within 2 % of 468.3 µε; a probe on the curved bore or outside face may be read up to 0.3 mm off
/// the straight-sided elements, no more; each probe's mesh check ran and is settled.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn c1_lames_cylinder_read_through_its_wall_at_probes() {
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"diameter": 80, "height": 10, "body_name": "Ring"}));
    run(&mut s, "solid.cylinder", json!({"diameter": 40, "height": 10, "operation": "cut", "targets": ["Ring"]}));
    run(&mut s, "solid.box", json!({"length": 50, "width": 50, "height": 10, "corner": [0, 0, 0], "operation": "intersect", "targets": ["Ring"]}));
    let (a, b, p, e, nu) = (20.0, 40.0, 50.0, 210_000.0, 0.3);
    let c = std::f64::consts::FRAC_1_SQRT_2;
    let radii = [20.0, 25.0, 30.0, 35.0, 40.0];
    let probes: Vec<Value> =
        radii.iter().map(|r| json!({"at": [r * c, r * c, 5.0], "direction": [-c, c, 0.0], "name": format!("r = {r}")})).collect();
    let r = run(
        &mut s,
        CMD,
        json!({"body": "Ring", "supports": [{"face": [0, 30, 5], "type": "symmetry"}, {"face": [30, 0, 5], "type": "symmetry"},
               {"face": [21.2, 21.2, 0], "type": "roller"}], "loads": [{"face": [14.142136, 14.142136, 5], "pressure": "50 MPa"}],
               "material": "S355", "probes": probes}),
    );
    let got = r["probes"].as_array().cloned().unwrap_or_default();
    assert_eq!(got.len(), radii.len(), "{r}");
    for (pr, rad) in got.iter().zip(radii) {
        let lame = p * a * a / (b * b - a * a) * (1.0 + b * b / (rad * rad));
        let hoop = number(&pr["along"]["stress_mpa"]);
        eprintln!("C1 r = {rad}: hoop {hoop:.2} MPa against Lamé {lame:.2} ({:+.2}%); {}", 100.0 * (hoop / lame - 1.0), pr["line"]);
        assert!((hoop / lame - 1.0).abs() <= 0.015, "r = {rad}: {pr}");
        assert!(pr["outside_part_mm"].as_f64().unwrap_or(0.0) <= 0.3, "{pr}");
        assert_eq!(pr["mesh_check"]["status"], "settled", "r = {rad}: {pr}");
    }
    let bore = &got[0];
    let sp = &bore["principal_stress_mpa"];
    let hoop_a = p * (b * b + a * a) / (b * b - a * a);
    assert!((number(&sp[0]) / hoop_a - 1.0).abs() <= 0.02 && number(&sp[1]).abs() <= 1.0 && (number(&sp[2]) / -p - 1.0).abs() <= 0.02, "{sp}");
    let strain = (hoop_a + nu * p) / e * 1e6;
    assert!((number(&bore["principal_strain_microstrain"][0]) / strain - 1.0).abs() <= 0.02, "{bore}");
}

/// C1 on shells, real: a probe on a sheet's top face reads the top, one on its bottom the bottom.
/// Input: R4's strip (100 x 20 x 1.5 mm, 5052-H32) as shells, clamped at x = 0, 0.005 MPa down on
/// its top; probes at mid-span on the top face (50, 10, 1.5) and the bottom face (50, 10, 0), along
/// X. Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// By hand (beam): σ(50) = (100 − 50)² / 150 = 16.7 MPa at the surface, tension on top (the strip
/// bends down), compression below.
/// Pass: the top reads +16.7 MPa along X within 5 % (R4's tolerance for the painted value), the
/// bottom −16.7 within 5 %, and the two are equal and opposite within 2 %.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn c1_a_probe_on_a_sheet_reads_the_face_it_is_on() {
    let mut s = Session::default();
    let (_, root, _) = strip(&mut s, 100.0);
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": [50, 10, 1.5], "pressure": 0.005}], "supports": [{"face": root}], "material": aluminium(), "elements": "shell",
               "probes": [{"at": [50, 10, 1.5], "direction": [1, 0, 0], "name": "top"}, {"at": [50, 10, 0], "direction": [1, 0, 0], "name": "bottom"}]}),
    );
    let along = |k: usize| number(&r["probes"][k]["along"]["stress_mpa"]);
    let (top, bottom, beam) = (along(0), along(1), 50f64.powi(2) / 150.0);
    eprintln!("C1 shells: top {top:.2} MPa, bottom {bottom:.2} MPa against ±{beam:.2}; {} | {}", r["probes"][0]["line"], r["probes"][1]["line"]);
    assert!((top / beam - 1.0).abs() <= 0.05, "{r}");
    assert!((bottom / -beam - 1.0).abs() <= 0.05, "{r}");
    assert!((top / -bottom - 1.0).abs() <= 0.02, "{top} vs {bottom}");
}

/// The angle bracket's base underside and the top end of its upright: the faces `angle` builds,
/// found by where they are (the underside faces down at z = 0; the upright's top end is the face
/// highest up).
fn angle_ends(s: &mut Session) -> (u64, u64) {
    angle(s);
    let body = s.model.state().bodies.first().map(|b| b.name.clone()).unwrap_or_default();
    let faces = run(s, "model.faces", json!({"body": body}));
    let all: Vec<Value> = faces["faces"].as_array().cloned().unwrap_or_default();
    let z = |f: &Value| number(&f["centroid"][2]);
    let base = all
        .iter()
        .find(|f| z(f).abs() < 1e-6 && number(&f["plane_normal"][2]) < -0.99)
        .and_then(|f| f["index"].as_u64())
        .unwrap_or_else(|| panic!("no underside: {faces}"));
    let top = all.iter().max_by(|a, b| z(a).total_cmp(&z(b))).and_then(|f| f["index"].as_u64()).unwrap_or_else(|| panic!("no top: {faces}"));
    (base, top)
}

/// A5 (V-A5), real: a clamp's moment is the force times its lever arm.
/// Input: R1's strip (100 x 20 x 1.5 mm, 5052-H32) clamped at x = 0, 10 N down on its end at
/// x = 100; as solids and as shells (where the clamp's moment is a couple across the expanded
/// shell's thickness), mesh check off. Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// By hand: the clamp carries 10 N up and F × e = 10 × 100 = 1000 N·mm about −Y, about its own
/// middle at x = 0.
/// Pass, for each: the one support's force within 0.5 % of 10 N up, its moment within 0.5 % of
/// 1000 N·mm and along −Y (its other components under 0.5 % of it); the moments balance within
/// 0.1 %; the reply names both programs' versions.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn a5_a_clamps_moment_is_the_force_times_its_lever_arm() {
    for elements in ["solid", "shell"] {
        let mut s = Session::default();
        let (_, root, tip) = strip(&mut s, 100.0);
        let r = run(
            &mut s,
            CMD,
            json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": aluminium(), "elements": elements,
                   "mesh_check": false}),
        );
        let eq = &r["equilibrium"];
        let support = &eq["supports"][0];
        eprintln!(
            "A5 {elements}: {} | {}; moments {}% apart; programs {}",
            support["line"], eq["line"], eq["moment_mismatch_percent"], r["programs"]
        );
        assert_eq!(eq["supports"].as_array().map(Vec::len), Some(1), "{eq}");
        let (f, m) = (&support["force_n"], &support["moment_nmm"]);
        assert!((number(&f[2]) / 10.0 - 1.0).abs() <= 0.005 && number(&f[0]).abs() < 0.05 && number(&f[1]).abs() < 0.05, "{elements}: {support}");
        assert!((number(&m[1]) / -1000.0 - 1.0).abs() <= 0.005, "{elements}: {support}");
        assert!(number(&m[0]).abs() <= 5.0 && number(&m[2]).abs() <= 5.0, "{elements}: {support}");
        assert!(number(&support["about_mm"][0]).abs() < 1e-6, "the clamp's middle is at x = 0: {support}");
        assert!(number(&eq["moment_mismatch_percent"]) <= 0.1, "{eq}");
        assert!(r["programs"]["mesher"].as_str().is_some_and(|v| v.starts_with("gmsh ")), "{}", r["programs"]);
        assert!(r["programs"]["solver"].as_str().is_some_and(|v| v.starts_with("CalculiX ")), "{}", r["programs"]);
    }
}

/// A4/A5 (V-A4), real: the L bracket's supports balance at every mesh.
/// Input: the angle bracket (a 60 x 40 base, a 40 mm upright, 1.5 mm 5052-H32) as shells, its
/// base's underside held (an area: translations only), 20 N along +Y on the upright's top end; at
/// meshes 2.75, 1.83, 1.0 and 0.6 mm, mesh check off. Real: the app's modelling, gmsh, CalculiX.
/// Faked: nothing.
/// Pass, at every mesh: the support's force within 0.1 % of 20 N along −Y; forces and moments
/// balance within 0.1 %; no warning says the result is suspect.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn a4_the_l_brackets_support_balances_at_every_mesh() {
    for mesh in [2.75, 1.83, 1.0, 0.6] {
        let mut s = Session::default();
        let (base, top) = angle_ends(&mut s);
        let r = run(
            &mut s,
            CMD,
            json!({"loads": [{"face": top, "force": [0, 20, 0]}], "supports": [{"face": base}], "material": aluminium(), "elements": "shell",
                   "mesh_size": mesh, "mesh_check": false}),
        );
        let eq = &r["equilibrium"];
        let support = &eq["supports"][0];
        eprintln!(
            "A4 at {mesh} mm ({} nodes): {} | forces {}%, moments {}% apart",
            r["runs"][0]["nodes"], support["line"], eq["mismatch_percent"], eq["moment_mismatch_percent"]
        );
        assert!((number(&support["force_n"][1]) / -20.0 - 1.0).abs() <= 0.001, "{mesh}: {support}");
        assert!(number(&eq["mismatch_percent"]) <= 0.1 && number(&eq["moment_mismatch_percent"]) <= 0.1, "{mesh}: {eq}");
        assert!(!r["warnings"].to_string().contains("suspect"), "{mesh}: {}", r["warnings"]);
    }
}

/// Peterson's stepped flat bar (V-6 and V-7), made with the app's commands: a wide part 60 x 28 mm
/// (D) and a narrow part 80 x 20 mm (d), 4 mm thick, along X, with a fillet of radius `fillet` mm
/// at each inside corner of the step (none: a sharp corner). The setup: the wide end at x = 0
/// held, 4000 N along +X on the narrow end at x = 140, S355.
fn stepped_bar(s: &mut Session, fillet: f64) -> Value {
    run(s, "solid.box", json!({"length": 60, "width": 28, "height": 4, "corner": [0, -14, 0], "body_name": "Bar"}));
    run(s, "solid.box", json!({"length": 80, "width": 20, "height": 4, "corner": [60, -10, 0], "operation": "join", "targets": ["Bar"]}));
    if fillet > 0.0 {
        run(s, "solid.fillet", json!({"edges": [[60, 10, 2], [60, -10, 2]], "radius": fillet, "body": "Bar"}));
    }
    json!({"body": "Bar", "supports": [{"face": [0, 0, 2]}], "loads": [{"face": [140, 0, 2], "force": [4000, 0, 0]}], "material": "S355"})
}

/// The converged von Mises peak at V-6's fillet, MPa: the finite-element model's own limit, from
/// meshes finer than the estimate's default. A quarter of the bar on two symmetry planes (across
/// its width and through its thickness), the curvature sizing refined with the element size, gave
/// 108.44, 109.96, 109.75 and 109.63 MPa at 2.0, 1.33, 0.889 and 0.593 mm (8,735 to 155,191 nodes;
/// Richardson on the last three: 109.47, order 1.4). Peterson's 107.3 MPa is a two-dimensional
/// chart's; the model sits 2 % over it, within V-6's 3 %, so the band is checked against the model's
/// own limit (PIL-294 A6, agreed 2026-11-20).
const V6_CONVERGED_MPA: f64 = 109.6;
/// Peterson's Chart 3.1 curve fit for V-6 (Kt 2.146 on 50 MPa), MPa.
const V6_PETERSON_MPA: f64 = 107.3;

/// V-6 and V-A6, real: Peterson's stepped flat bar in tension, at the default settings.
/// Input: `stepped_bar` with 2 mm fillets: D 28, d 20, r 2 mm (D/d 1.4, r/d 0.1), 4 mm thick, the
/// wide end held, 4000 N along the bar on the narrow end, S355; no mesh size, the mesh check on.
/// By hand: the nominal stress on the narrow section is 4000 / (20 × 4) = 50 MPa. Peterson's Chart
/// 3.1 curve fit (W. D. Pilkey and D. F. Pilkey, Peterson's Stress Concentration Factors, 3rd ed.,
/// Wiley 2008) at t/r = 2 and 2t/D = 0.286 gives Kt = 2.146, so 107.3 MPa at the fillet.
/// Real: the app's modelling, gmsh, CalculiX. Faked: nothing.
/// Pass: three meshes solved (the away peak moved more than 1 % on the first step); the away peak at
/// a fillet, settled, its band at most 3 % and holding the model's converged 109.6 MPa; the quoted
/// value within 3 % of 107.3; `fraction_of_yield` its share of 355 MPa, with the same band; the
/// summary gives the band.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn v6_a_shoulder_fillets_peak_is_settled_within_its_band() {
    let mut s = Session::default();
    let p = stepped_bar(&mut s, 2.0);
    let started = std::time::Instant::now();
    let r = run(&mut s, CMD, p);
    let check = &r["mesh_check"];
    let away = check["quantities"].as_array().into_iter().flatten().find(|q| q["quantity"] == AWAY).cloned().unwrap_or_default();
    let (estimate, band) = (number(&away["estimate"]), number(&away["band_percent"]));
    eprintln!(
        "V-6 {} | away {} (values {}, order {}) | {} s in all, runs {:?} s, nodes {:?}; peak near {}",
        check["line"],
        away["quote"],
        away["values"],
        away["observed_order"],
        started.elapsed().as_secs(),
        times(&r),
        r["runs"].as_array().map(|a| a.iter().map(|x| x["nodes"].clone()).collect::<Vec<_>>()),
        r["peak_away"]["near"]
    );
    assert_eq!(check["meshes_mm"].as_array().map(Vec::len), Some(3), "{check}");
    assert!(r["peak_away"]["near"].as_str().is_some_and(|n| n.contains("fillet")), "{}", r["peak_away"]);
    assert_eq!(away["status"], "settled", "{away}");
    assert!(band <= 3.0, "{away}");
    assert!((estimate - V6_CONVERGED_MPA).abs() <= estimate * band / 100.0, "{estimate} ±{band}% misses {V6_CONVERGED_MPA}: {away}");
    assert!((estimate / V6_PETERSON_MPA - 1.0).abs() <= 0.03, "{estimate} against Peterson's {V6_PETERSON_MPA}");
    assert!((number(&r["fraction_of_yield"]) - estimate / 355.0).abs() < 2e-3 && number(&r["band_percent"]) == band, "{r}");
    assert!(r["summary"].as_str().is_some_and(|t| t.contains(&format!("(within ±{band:.1} %)"))), "{}", r["summary"]);
}

/// V-7 and V-A6, real: the same bar with a sharp inside corner at the step (no fillet), at the
/// default settings. The stress at a re-entrant corner has no finite value: the peak grows as the
/// mesh is refined (95.6, 103.4, 121.6 MPa measured). Real: the app's modelling, gmsh, CalculiX.
/// Faked: nothing. Pass: three meshes; the away peak keeps rising, never settled, with no band and
/// no estimate; a warning says so; the reply's `band_percent` is absent.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn v7_a_sharp_corners_peak_keeps_rising_and_is_never_settled() {
    let mut s = Session::default();
    let p = stepped_bar(&mut s, 0.0);
    let r = run(&mut s, CMD, p);
    let check = &r["mesh_check"];
    let away = check["quantities"].as_array().into_iter().flatten().find(|q| q["quantity"] == AWAY).cloned().unwrap_or_default();
    eprintln!("V-7 {} | runs {:?} s; warnings {}", check["line"], times(&r), r["warnings"]);
    assert_eq!(check["meshes_mm"].as_array().map(Vec::len), Some(3), "{check}");
    assert_eq!(away["status"], RISING, "{away}");
    assert!(away["band_percent"].is_null() && away["estimate"].is_null(), "{away}");
    assert!(r["band_percent"].is_null(), "{r}");
    assert!(r["warnings"].to_string().contains("keeps rising as the mesh is refined"), "{}", r["warnings"]);
    assert!(r["summary"].as_str().is_some_and(|t| t.contains("MPa on the finest mesh, and it keeps rising")), "{}", r["summary"]);
}

/// A7 (V-A7), real: the same request gives the same mesh, numbers and words.
/// Input: R2's plate with a hole (solids, 1.5 mm), the angle bracket as shells and as solids (its
/// default meshes), mesh check off; and V-6's filleted bar at the default settings with the mesh
/// check on (three meshes, the extrapolated band); each set up afresh and run twice. Real: the
/// app's modelling, gmsh, CalculiX. Faked: nothing.
/// Observable: the whole reply but the seconds each solve took and the setup's line naming this
/// computer's memory limit (the memory free when it ran).
/// Pass: the two replies of each are identical: node and element counts, every number and band,
/// every line and warning.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn a7_the_same_request_gives_the_same_numbers_and_words() {
    let without_times = |mut r: Value| {
        for x in r["runs"].as_array_mut().into_iter().flatten() {
            if let Some(o) = x.as_object_mut() {
                o.remove("seconds");
            }
        }
        if let Some(setup) = r["setup"].as_array_mut() {
            setup.retain(|l| !l.as_str().is_some_and(|t| t.starts_with("Mesh check:")));
        }
        r
    };
    let plate = |s: &mut Session| {
        let (held, pulled) = plate_with_hole(s);
        json!({"loads": [{"face": pulled, "force": [800, 0, 0]}], "supports": [{"face": held}], "material": aluminium(), "elements": "solid",
               "mesh_size": 1.5, "mesh_check": false})
    };
    let bracket = |elements: &'static str| {
        move |s: &mut Session| {
            let (base, top) = angle_ends(s);
            json!({"loads": [{"face": top, "force": [0, 20, 0]}], "supports": [{"face": base}], "material": aluminium(), "elements": elements,
                   "mesh_check": false})
        }
    };
    let cases: [(&str, Box<dyn Fn(&mut Session) -> Value>); 4] = [
        ("plate with a hole", Box::new(plate)),
        ("bracket as shells", Box::new(bracket("shell"))),
        ("bracket as solids", Box::new(bracket("solid"))),
        ("filleted bar with its mesh check", Box::new(|s: &mut Session| stepped_bar(s, 2.0))),
    ];
    for (name, setup) in cases {
        let once = || {
            let mut s = Session::default();
            let p = setup(&mut s);
            without_times(run(&mut s, CMD, p))
        };
        let (a, b) = (once(), once());
        eprintln!(
            "A7 {name}: {} nodes, {} MPa, {} mm; {}; mesh check {}; identical: {}",
            a["runs"][0]["nodes"],
            a["runs"][0]["max_von_mises_mpa"],
            a["runs"][0]["max_deflection_mm"],
            a["equilibrium"]["line"],
            a["mesh_check"]["line"],
            a == b
        );
        assert_eq!(a, b, "{name}");
    }
}

/// The runs measured on zhikun for A13's fit (2026-11-20; see `stress_forecast.rs`): part, elements,
/// element size (mm), nodes counted, the solver's peak memory (GB, `/usr/bin/time`; NaN where not
/// measured) and the seconds meshing and solving took together. Round 1 at a load of 2 to 16 on
/// 24 cores, round 2 at 5 to 44.
const A13_MEASURED: &[(&str, Elements, f64, usize, f64, f64)] = &[
    ("plate", Elements::Solid, 3.0, 21855, f64::NAN, 2.2),    // round 1
    ("plate", Elements::Solid, 2.0, 42012, f64::NAN, 4.5),    // round 1
    ("plate", Elements::Solid, 1.5, 68745, f64::NAN, 7.9),    // round 1
    ("plate", Elements::Solid, 1.2, 114027, f64::NAN, 17.9),  // round 1
    ("plate", Elements::Solid, 1.0, 199504, f64::NAN, 49.1),  // round 1
    ("plate", Elements::Solid, 0.85, 288564, f64::NAN, 83.5), // round 1
    ("block", Elements::Solid, 5.0, 15802, f64::NAN, 3.6),    // round 1
    ("block", Elements::Solid, 4.0, 22834, f64::NAN, 6.5),    // round 1
    ("block", Elements::Solid, 3.0, 35902, f64::NAN, 13.2),   // round 1
    ("block", Elements::Solid, 2.5, 51410, f64::NAN, 22.3),   // round 1
    ("block", Elements::Solid, 2.0, 95384, f64::NAN, 81.1),   // round 1
    ("block", Elements::Solid, 1.7, 156016, f64::NAN, 222.5), // round 1
    ("sheet", Elements::Shell, 6.0, 2839, f64::NAN, 0.5),     // round 1
    ("sheet", Elements::Shell, 4.0, 6007, f64::NAN, 0.9),     // round 1
    ("sheet", Elements::Shell, 3.0, 10779, f64::NAN, 1.9),    // round 1
    ("sheet", Elements::Shell, 2.0, 23553, f64::NAN, 4.7),    // round 1
    ("sheet", Elements::Shell, 1.5, 42263, f64::NAN, 10.4),   // round 1
    ("sheet", Elements::Shell, 1.2, 65579, f64::NAN, 18.7),   // round 1
    ("sheet", Elements::Shell, 1.0, 93273, f64::NAN, 30.4),   // round 1
    ("angle", Elements::Shell, 2.0, 16467, f64::NAN, 2.5),    // round 1
    ("angle", Elements::Shell, 1.2, 39723, f64::NAN, 7.5),    // round 1
    ("angle", Elements::Shell, 0.8, 56061, f64::NAN, 11.7),   // round 1
    ("angle", Elements::Shell, 0.6, 80843, f64::NAN, 18.0),   // round 1
    ("plate", Elements::Solid, 1.5, 68745, 0.842, 10.4),      // round 2
    ("plate", Elements::Solid, 1.0, 199504, 3.474, 65.1),     // round 2
    ("plate", Elements::Solid, 0.85, 288564, 5.371, 109.4),   // round 2
    ("block", Elements::Solid, 4.0, 22834, 0.453, 8.6),       // round 2
    ("block", Elements::Solid, 3.0, 35902, 0.798, 16.1),      // round 2
    ("block", Elements::Solid, 2.5, 51410, 1.237, 30.5),      // round 2
    ("block", Elements::Solid, 2.0, 95384, 2.970, 101.7),     // round 2
    ("block", Elements::Solid, 1.7, 156016, 5.784, 265.5),    // round 2
    ("thick", Elements::Solid, 6.0, 16870, 0.283, 4.7),       // round 2
    ("thick", Elements::Solid, 4.0, 33200, 0.680, 14.6),      // round 2
    ("thick", Elements::Solid, 3.0, 59617, 1.388, 28.5),      // round 2
    ("thick", Elements::Solid, 2.5, 90339, 2.404, 55.7),      // round 2
    ("thick", Elements::Solid, 2.2, 131228, 4.081, 116.2),    // round 2
    ("bar", Elements::Solid, 4.0, 11346, 0.144, 1.6),         // round 2
    ("bar", Elements::Solid, 3.0, 24322, 0.384, 5.0),         // round 2
    ("bar", Elements::Solid, 2.2, 57344, 1.205, 19.5),        // round 2
    ("bar", Elements::Solid, 1.8, 100384, 2.563, 54.3),       // round 2
    ("sheet", Elements::Solid, 1.5, 133229, 1.812, 27.0),     // round 2
    ("sheet", Elements::Solid, 1.2, 199864, 2.839, 47.6),     // round 2
    ("angle", Elements::Solid, 1.5, 102752, 1.406, 25.3),     // round 2
    ("angle", Elements::Solid, 1.0, 143441, 2.094, 41.2),     // round 2
    ("sheet", Elements::Shell, 3.0, 10779, 0.434, 2.7),       // round 2
    ("sheet", Elements::Shell, 2.0, 23553, 0.991, 7.2),       // round 2
    ("sheet", Elements::Shell, 1.5, 42263, 1.846, 13.8),      // round 2
    ("sheet", Elements::Shell, 1.2, 65579, 2.943, 25.0),      // round 2
    ("sheet", Elements::Shell, 1.0, 93273, 4.277, 35.1),      // round 2
    ("angle", Elements::Shell, 1.2, 39723, 1.294, 8.2),       // round 2
    ("angle", Elements::Shell, 0.8, 56061, 1.894, 13.2),      // round 2
    ("angle", Elements::Shell, 0.6, 80843, 2.722, 19.6),      // round 2
];

/// The parts of [`A13_MEASURED`]: volume (mm³, None for a sheet), surface area (mm²) and section
/// ([`section_size`]: twice the volume over the area, or a sheet's 1.5 mm).
fn a13_part(name: &str) -> (Option<f64>, f64, f64) {
    let solid = |v: f64, s: f64| (Some(v), s, 2.0 * v / s);
    match name {
        "plate" => solid(19858.6, 20919.8),
        "block" => solid(109393.4, 17512.3),
        "thick" => solid(193877.8, 28636.3),
        "bar" => solid(80000.0, 16800.0),
        "sheet" => (None, 40900.0, 1.5),
        _ => (None, 9639.7, 1.5),
    }
}

/// A13: the forecast's models hold for every run they were fitted on, within the errors their
/// comments state. Input: [`A13_MEASURED`]. Action: `forecast::memory`, `forecast::seconds`,
/// `forecast::nodes_before_meshing`. Nothing runs.
/// Pass: the memory within 15 % of each solid run's measured peak, and from 10 % under to 40 % over
/// each shell run's (the shells keep `SHELL_MEMORY`, high on the bracket); meshing and solving
/// together within 45 %
/// of each measured time (solids and shells); nodes from the part's size within 12 % from 50,000
/// nodes up on the parts without small curved features (the bracket's bend and holes are
/// excluded: curvature sizing adds nodes the size does not show). And the reason for the layers:
/// the old `solver_bytes` is more than 40 % under the 50 mm cube's measured peak.
#[test]
fn a13_the_forecast_holds_for_the_runs_it_was_fitted_on() {
    let (mut worst_bytes, mut worst_time, mut worst_nodes) = (0.0f64, 0.0f64, 0.0f64);
    for &(part, elements, h, nodes, gb, seconds) in A13_MEASURED {
        let (volume, area, section) = a13_part(part);
        let deep = forecast::layers(section, h);
        let n = nodes as f64;
        if gb.is_finite() {
            let off = forecast::memory(n, elements, deep) / (gb * 1.0e9) - 1.0;
            worst_bytes = worst_bytes.max(off.abs());
            let (under, over) = if elements == Elements::Solid { (-0.15, 0.15) } else { (-0.10, 0.40) };
            assert!((under..=over).contains(&off), "{part} {elements:?} {h} mm: memory {:+.0}%", 100.0 * off);
        }
        let (mesh, solve) = forecast::seconds(n, elements, deep);
        let off = (mesh + solve) / seconds - 1.0;
        worst_time = worst_time.max(off.abs());
        assert!(off.abs() <= 0.45, "{part} {elements:?} {h} mm: {:.1} s predicted, {seconds} s measured", mesh + solve);
        let flat = part != "angle";
        if flat && nodes >= 50_000 && gb.is_finite() {
            let thickness = 1.5;
            let before = forecast::nodes_before_meshing(elements, h, volume, area, thickness).unwrap_or(f64::NAN);
            let off = before / n - 1.0;
            worst_nodes = worst_nodes.max(off.abs());
            assert!(off.abs() <= 0.12, "{part} {elements:?} {h} mm: {before:.0} nodes predicted, {nodes} counted");
        }
    }
    eprintln!("A13 worst: memory {:.0}%, time {:.0}%, nodes before meshing {:.0}%", 100.0 * worst_bytes, 100.0 * worst_time, 100.0 * worst_nodes);
    let cube = solver_bytes(156_016.0, Elements::Solid) / 5.784e9 - 1.0;
    assert!(cube < -0.4, "the old model on the cube: {:+.0}%", 100.0 * cube);
}

/// A13 (V-A13): a dry run starts no program and asks no approval (`agent_chat::gate`), and it says
/// what the run will take, from the part's size, for the approval card.
/// Input: R2's plate (200 x 50 x 2 mm, a Ø10 hole) at 1.5 mm as solids, with the mesh check off and
/// on; the 200 x 100 x 1.5 mm sheet as shells at 1.2 mm. Real: the app's modelling. Faked: nothing;
/// no program runs.
/// Measured on zhikun: the plate meshed with 68,745 nodes and its solver peaked at 0.84 GB; the
/// sheet with 65,579 nodes and 2.94 GB.
/// Pass: no run and no program versions in the reply; `predicted` within 10 % of the nodes and
/// 25 % of the memory measured (the memory model's value for the nodes predicted), one run with the
/// mesh check off; with it on, two when this computer has room for the second mesh, one when not;
/// at 4 mm, where the second and third fit the 6 GB ceiling, two runs and three at the most, with
/// more nodes, memory and time, in words too; seconds above 0; its line in words.
#[test]
fn a13_a_dry_run_says_what_the_run_will_take_from_the_parts_size() {
    let _one = hold_preview();
    let mut s = Session::default();
    let (held, pulled) = plate_with_hole(&mut s);
    let setup = |check: bool| {
        json!({"loads": [{"face": pulled, "force": [800, 0, 0]}], "supports": [{"face": held}], "material": aluminium(), "elements": "solid",
               "mesh_size": 1.5, "mesh_check": check, "dry_run": true})
    };
    let r = run(&mut s, CMD, setup(false));
    assert!(r["runs"].as_array().is_some_and(Vec::is_empty) && r.get("programs").is_none(), "{r}");
    let p = &r["predicted"];
    eprintln!("A13 plate, dry run: {p}");
    assert!((number(&p["nodes"]) / 68_745.0 - 1.0).abs() <= 0.10, "{p}");
    assert!((number(&p["memory_gb"]) / 0.842 - 1.0).abs() <= 0.25, "{p}");
    assert_eq!(p["runs"], 1, "{p}");
    assert!(number(&p["seconds"]) > 0.0, "{p}");
    let line = p["line"].as_str().unwrap_or("");
    assert!(line.contains("nodes") && line.contains("of memory") && line.contains("one run") && line.contains("before meshing"), "{line}");
    let checked = run(&mut s, CMD, setup(true));
    // The second mesh at 1.0 mm, through the plate's 1.9 mm section.
    let section = 2.0 * 19858.6 / 20919.8;
    let fits = forecast::finer_fits(number(&p["nodes"]), Elements::Solid, forecast::layers(section, 1.0));
    assert_eq!(checked["predicted"]["runs"], if fits { 2 } else { 1 }, "{}", checked["predicted"]);
    if fits {
        assert!(number(&checked["predicted"]["seconds"]) > number(&p["seconds"]), "{}", checked["predicted"]);
    }
    let mut coarse = setup(true);
    coarse["mesh_size"] = json!(4.0);
    let coarse = run(&mut s, CMD, coarse)["predicted"].clone();
    if forecast::finer_fits(number(&coarse["nodes"]), Elements::Solid, forecast::layers(section, 4.0 / 1.5)) {
        let most = &coarse["most"];
        assert_eq!((&coarse["runs"], &most["runs"]), (&json!(2), &json!(3)), "{coarse}");
        for key in ["nodes", "memory_gb", "seconds"] {
            assert!(number(&most[key]) > number(&coarse[key]), "{key}: {coarse}");
        }
        let line = coarse["line"].as_str().unwrap_or("");
        assert!(line.contains("for 2 runs") && line.contains("a third, finer mesh") && line.contains("for 3 runs"), "{line}");
    }
    assert!(p["most"].is_null(), "no mesh check, no third mesh: {p}");

    let mut s = Session::default();
    let (_, root, tip) = strip_of(&mut s, 200.0, 100.0);
    let r = run(
        &mut s,
        CMD,
        json!({"loads": [{"face": tip, "force": [0, 0, -10]}], "supports": [{"face": root}], "material": aluminium(), "elements": "shell",
               "mesh_size": 1.2, "mesh_check": false, "dry_run": true}),
    );
    let p = &r["predicted"];
    eprintln!("A13 sheet as shells, dry run: {p}");
    assert!((number(&p["nodes"]) / 65_579.0 - 1.0).abs() <= 0.10, "{p}");
    assert!((number(&p["memory_gb"]) / 2.943 - 1.0).abs() <= 0.25, "{p}");
}

/// A13: once a run has meshed the part, the forecast counts its nodes, the runs made and the time
/// left, likely and at the most. Input: a solid run planned at 1.5 mm and its mesh check's second
/// and third meshes at 1.0 and 0.67 mm (60,000, 180,000 and 540,000 nodes from the part's size), and
/// shells after them; the first counts 90,000 (half as many again), the second 250,000. The mesh
/// check's gate: one that lets every mesh through, and some that stop at a count of nodes. Action:
/// `forecast::which`, `after_meshing`, `before_meshing`, `Tracker`. Nothing runs.
/// Pass: run 1 of 2 (3 at the most) with the nodes counted and their memory, its time left its own
/// solve and the second's mesh and solve at 270,000 nodes (180,000 scaled by 1.5), and at the most
/// the third's at 810,000 too; run 2's its solve alone, and at the most the third's. A gate closed
/// to the second: run 1 of 1; to the third: 2 runs at the most. Counted, the third is run 3 of 3;
/// the shells after it run 4 of 4, and run 3 of 3 when no third was counted. The published forecast
/// carries the design and a newer number.
#[test]
fn a13_once_meshed_the_forecast_counts_the_nodes_the_runs_and_the_time_left() {
    let planned = vec![
        forecast::Planned { elements: Elements::Solid, h: 1.5, section: 3.0, nodes: 60_000.0, level: 0 },
        forecast::Planned { elements: Elements::Solid, h: 1.0, section: 3.0, nodes: 180_000.0, level: 1 },
        forecast::Planned { elements: Elements::Solid, h: 1.0 / 1.5, section: 3.0, nodes: 540_000.0, level: 2 },
    ];
    let two = forecast::Reach { likely: 1, most: 1 };
    let three = forecast::Reach { likely: 1, most: 2 };
    assert_eq!(forecast::which(&planned, Elements::Solid, 1.5), Some(0));
    assert_eq!(forecast::which(&planned, Elements::Solid, 1.0), Some(1));
    assert_eq!(forecast::which(&planned, Elements::Shell, 1.0), None);
    assert_eq!(forecast::which(&planned, Elements::Solid, 1.2), None);
    let first = forecast::after_meshing(&planned, 0, 90_000, two, "doc-a13").unwrap_or_else(|| panic!("a forecast"));
    assert_eq!((first.run, first.runs, first.runs_most, first.nodes, first.elements), (1, 2, 2, 90_000, Elements::Solid));
    assert_eq!(first.seconds_most, first.seconds_left, "no third mesh");
    assert_eq!(first.bytes, forecast::memory(90_000.0, Elements::Solid, 2.0));
    let (_, solve) = forecast::seconds(90_000.0, Elements::Solid, 2.0);
    let (m2, s2) = forecast::seconds(270_000.0, Elements::Solid, 3.0);
    assert!((first.seconds_left - (solve + m2 + s2)).abs() < 1e-9, "{first:?}");
    let first = forecast::after_meshing(&planned, 0, 90_000, three, "doc-a13").unwrap_or_else(|| panic!("a forecast"));
    let (m3, s3) = forecast::seconds(810_000.0, Elements::Solid, 4.5);
    assert_eq!((first.run, first.runs, first.runs_most), (1, 2, 3));
    assert!((first.seconds_left - (solve + m2 + s2)).abs() < 1e-9 && (first.seconds_most - (solve + m2 + s2 + m3 + s3)).abs() < 1e-6, "{first:?}");
    let second = forecast::after_meshing(&planned, 1, 250_000, three, "doc-a13").unwrap_or_else(|| panic!("a forecast"));
    assert_eq!((second.run, second.runs, second.runs_most), (2, 2, 3));
    let (m3, s3) = forecast::seconds(750_000.0, Elements::Solid, 4.5);
    assert!((second.seconds_most - second.seconds_left - (m3 + s3)).abs() < 1e-6, "{second:?}");
    assert!((second.seconds_left - forecast::seconds(250_000.0, Elements::Solid, 3.0).1).abs() < 1e-9, "{second:?}");

    let up_to_80k = |nodes: f64, _: Elements, _: f64| nodes <= 80_000.0;
    let all = |_: f64, _: Elements, _: f64| true;
    let runs =
        |fits: &dyn Fn(f64, Elements, f64) -> bool| forecast::before_meshing(&planned, fits).map(|(sure, most)| (sure.runs, most.map(|m| m.runs)));
    assert_eq!(runs(&up_to_80k), Some((2, None)), "60,000 predicted: the second mesh is planned, and no third after 180,000");
    assert_eq!(runs(&all), Some((2, Some(3))));
    assert_eq!(runs(&|_, _, _| false), Some((1, None)));
    // The second's nodes are scaled by the first's count (180,000 x 1.5), and 0.7 of them decide whether
    // a third may follow; once the second is counted, its own nodes do.
    let up_to = |most: f64| move |nodes: f64, _: Elements, _: f64| nodes <= most;
    let reach = |likely, most| forecast::Reach { likely, most };
    assert_eq!(forecast::reach_after_first(&planned, 90_000.0, &up_to(150_000.0)), reach(1, 1), "270,000 x 0.7 is over");
    assert_eq!(forecast::reach_after_first(&planned, 60_000.0, &up_to(150_000.0)), reach(1, 2), "180,000 x 0.7 is not");
    assert_eq!(forecast::reach_after_first(&planned, 90_000.0, &up_to(200_000.0)), reach(1, 2), "270,000 x 0.7 is not");
    assert_eq!(forecast::reach_after_first(&planned, 90_000.0, &up_to(80_000.0)), reach(0, 0));
    assert_eq!(forecast::reach_after_first(&planned[..1], 90_000.0, &all), reach(0, 0), "no mesh check");
    assert_eq!(forecast::reach_after_second(&planned, 250_000.0, &up_to(200_000.0)), reach(1, 1));
    assert_eq!(forecast::reach_after_second(&planned, 250_000.0, &up_to(260_000.0)), reach(1, 2));
    let tracker = forecast::Tracker::new(&planned, "doc-a13-tracker".into(), &up_to_80k);
    let before = fea::forecast_seq();
    tracker.counted(Elements::Solid, 1.5, 90_000);
    let f = fea::forecast().unwrap_or_else(|| panic!("published"));
    assert!(f.seq > before, "{f:?}");
    // Only this test of those that run by default publishes forecasts.
    assert_eq!(f.document_id, "doc-a13-tracker");
    assert_eq!((f.run, f.runs, f.runs_most, f.nodes), (1, 1, 1, 90_000), "90,000 counted: the second mesh is not made");
    assert!((f.seconds_left - solve).abs() < 1e-9, "{f:?}");
    let tracker = forecast::Tracker::new(&planned, "doc-a13-tracker-2".into(), &all);
    tracker.counted(Elements::Solid, 1.5, 90_000);
    tracker.counted(Elements::Solid, 1.0, 250_000);
    let f = fea::forecast().unwrap_or_else(|| panic!("published"));
    assert_eq!(f.document_id, "doc-a13-tracker-2");
    assert_eq!((f.run, f.runs, f.runs_most, f.nodes), (2, 2, 3, 250_000));
    assert!(f.seconds_most > f.seconds_left, "{f:?}");
    // The third was made or not: it and the shells after it are counted among the runs made.
    let mut both = planned.clone();
    both.push(forecast::Planned { elements: Elements::Shell, h: 1.5, section: 3.0, nodes: 20_000.0, level: 0 });
    for (third, shells) in [(Some(700_000), (4, 4)), (None, (3, 3))] {
        let doc = format!("doc-a13-tracker-third-{}", third.is_some());
        let tracker = forecast::Tracker::new(&both, doc.clone(), &all);
        tracker.counted(Elements::Solid, 1.5, 90_000);
        tracker.counted(Elements::Solid, 1.0, 250_000);
        if let Some(n) = third {
            tracker.counted(Elements::Solid, 1.0 / 1.5, n);
            let f = fea::forecast().unwrap_or_else(|| panic!("published"));
            assert_eq!(f.document_id, doc);
            assert_eq!((f.run, f.runs, f.runs_most, f.nodes), (3, 4, 4, n));
            assert_eq!(f.seconds_most, f.seconds_left);
        }
        tracker.counted(Elements::Shell, 1.5, 20_000);
        let f = fea::forecast().unwrap_or_else(|| panic!("published"));
        assert_eq!(f.document_id, doc);
        assert_eq!(((f.run, f.runs), f.runs_most), (shells, shells.1), "{third:?}");
    }
}

/// A time in the dry run's words.
#[test]
fn a13_times_read_as_roughly_as_they_are_known() {
    assert_eq!(about_time(0.4), "under a second");
    assert_eq!(about_time(7.6), "8 s");
    assert_eq!(about_time(43.0), "45 s");
    assert_eq!(about_time(150.0), "2.5 min");
    assert_eq!(about_time(1500.0), "25 min");
    assert_eq!(about_time(5000.0), "83 min");
    assert_eq!(about_time(6000.0), "1.7 h");
    assert_eq!(about_time(f64::NAN), "?");
}

/// A13: the dry run's sentence words the figures it gives, which the card words again in the user's
/// language, so the two say the same.
/// Input: a solid run of 100 to 2,000,000 nodes, 2 % apart, alone and with the mesh check's second
/// mesh. Before this the sentence worded the figures before they were rounded: V-6's bar said
/// 1.5 min for 3 runs (104.6 s) where the card said 2 min (105 s).
/// Pass: the sentence has the words of `nodes`, `memory_gb` and `seconds`, and of `most`'s.
#[test]
fn a13_the_dry_runs_sentence_words_the_figures_it_gives() {
    let mut nodes = 100.0;
    while nodes < 2.0e6 {
        let first = forecast::Planned { elements: Elements::Solid, h: 1.5, section: 6.0, nodes, level: 0 };
        let second = forecast::Planned { h: 1.0, nodes: nodes * 3.0, level: 1, ..first.clone() };
        let third = forecast::Planned { h: 1.0 / 1.5, nodes: nodes * 9.0, level: 2, ..first.clone() };
        for planned in [vec![first.clone()], vec![first, second, third]] {
            let p = predicted(&planned).unwrap_or_else(|| panic!("{nodes} nodes: no prediction"));
            let most = p.most.as_ref().map(|m| (m.nodes, m.memory_gb, m.seconds));
            for (n, gb, s) in [Some((p.nodes, p.memory_gb, p.seconds)), most].into_iter().flatten() {
                for words in [about_count(n as f64), about_bytes(gb * 1.0e9), about_time(s)] {
                    assert!(p.line.contains(&format!("about {words} ")), "{words}: {}", p.line);
                }
            }
        }
        nodes *= 1.02;
    }
}

/// A13 (V-A13), real: a run publishes the nodes it counted after meshing, for the card the user
/// allowed it from, and the count is near the dry run's prediction.
/// Input: R2's plate at 1.5 mm as solids, mesh check off: a dry run, then the run. Real: the app's
/// modelling, gmsh, CalculiX. Faked: nothing.
/// Pass: the last forecast for this design is run 1 of 1 with the reply's node count and the
/// memory model's value for it; the dry run's nodes are within 10 % of that count.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn a13_a_run_publishes_its_counted_nodes_for_the_card() {
    let mut s = Session::default();
    let (held, pulled) = plate_with_hole(&mut s);
    let setup = |dry: bool| {
        json!({"loads": [{"face": pulled, "force": [800, 0, 0]}], "supports": [{"face": held}], "material": aluminium(), "elements": "solid",
               "mesh_size": 1.5, "mesh_check": false, "dry_run": dry})
    };
    let dry = run(&mut s, CMD, setup(true));
    let r = run(&mut s, CMD, setup(false));
    let nodes = r["runs"][0]["nodes"].as_u64().unwrap_or(0) as usize;
    let f = fea::forecast().filter(|f| f.document_id == s.document_id).unwrap_or_else(|| panic!("no forecast for this design"));
    eprintln!(
        "A13 real: predicted {} nodes, {} GB, {} s; counted {nodes} nodes, forecast {:.2} GB and {:.1} s to solve; took {} s",
        dry["predicted"]["nodes"],
        dry["predicted"]["memory_gb"],
        dry["predicted"]["seconds"],
        f.bytes / 1e9,
        f.seconds_left,
        r["runs"][0]["seconds"]
    );
    assert_eq!((f.run, f.runs, f.runs_most, f.nodes, f.elements), (1, 1, 1, nodes, Elements::Solid));
    let section = 2.0 * 19858.6 / 20919.8;
    assert!((f.bytes / forecast::memory(nodes as f64, Elements::Solid, forecast::layers(section, 1.5)) - 1.0).abs() < 0.01, "{f:?}");
    assert!((number(&dry["predicted"]["nodes"]) / nodes as f64 - 1.0).abs() <= 0.10, "{} vs {nodes}", dry["predicted"]);
}

/// A13 with A6, real: an estimate that solves a third mesh said so, before meshing and at each
/// count.
/// Input: V-6's filleted bar and V-7's sharp one at the default settings, which solve three meshes:
/// a dry run, then the run, with every forecast published for the design noted as it comes. Real:
/// the app's modelling, gmsh, CalculiX. Faked: nothing.
/// Before the third mesh was planned, the card said 17 s and 0.91 GB for 2 runs and V-6's bar took
/// 127 s in 3; its second count said 16 s left with 116 s to go.
/// Pass: the dry run says 2 runs, 3 at the most, with more nodes, memory and time; the forecasts are
/// run 1 and 2 of 2 (3 at the most), then run 3 of 3, with the reply's node counts. The time really
/// left after each is printed beside `seconds_most`, not asserted: tests do not time the wall clock,
/// and the fit's times are one computer's.
#[test]
#[ignore = "runs gmsh and CalculiX"]
fn a13_the_forecasts_know_the_third_mesh() {
    for (name, fillet) in [("V-6", 2.0), ("V-7", 0.0)] {
        let mut s = Session::default();
        let mut p = stepped_bar(&mut s, fillet);
        p["dry_run"] = json!(true);
        let dry = run(&mut s, CMD, p.clone());
        p["dry_run"] = json!(false);
        let predicted = &dry["predicted"];
        let doc = s.document_id.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (ended, started) = (stop.clone(), std::time::Instant::now());
        let watch = std::thread::spawn(move || {
            let mut seen: Vec<(f64, fea::Forecast)> = Vec::new();
            while !ended.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(f) = fea::forecast().filter(|f| f.document_id == doc)
                    && seen.last().is_none_or(|(_, x)| x.seq != f.seq)
                {
                    seen.push((started.elapsed().as_secs_f64(), f));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            seen
        });
        let r = run(&mut s, CMD, p);
        let took = started.elapsed().as_secs_f64();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let seen = watch.join().unwrap_or_default();
        eprintln!("A13 third mesh {name}: predicted {predicted}; took {took:.1} s, runs {:?} s", times(&r));
        for (at, f) in &seen {
            eprintln!(
                "A13 third mesh {name} at {at:.1} s: run {} of {} ({}), {} nodes, {:.2} GB, {:.1} to {:.1} s left, really {:.1}",
                f.run,
                f.runs,
                f.runs_most,
                f.nodes,
                f.bytes / 1e9,
                f.seconds_left,
                f.seconds_most,
                took - at
            );
        }
        assert_eq!((&predicted["runs"], &predicted["most"]["runs"]), (&json!(2), &json!(3)), "{predicted}");
        for key in ["nodes", "memory_gb", "seconds"] {
            assert!(number(&predicted["most"][key]) > number(&predicted[key]), "{key}: {predicted}");
        }
        assert_eq!(r["mesh_check"]["meshes_mm"].as_array().map(Vec::len), Some(3), "{}", r["mesh_check"]);
        let places: Vec<(usize, usize, usize)> = seen.iter().map(|(_, f)| (f.run, f.runs, f.runs_most)).collect();
        assert_eq!(places, [(1, 2, 3), (2, 2, 3), (3, 3, 3)], "{name}");
        // The reply lists the finest first.
        let mut nodes: Vec<u64> = r["runs"].as_array().into_iter().flatten().filter_map(|x| x["nodes"].as_u64()).collect();
        nodes.reverse();
        assert_eq!(seen.iter().map(|(_, f)| f.nodes as u64).collect::<Vec<_>>(), nodes, "{name}");
    }
}
