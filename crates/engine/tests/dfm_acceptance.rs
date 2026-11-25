//! The full-catalogue DFM acceptance harness: sample parts built through the normal modelling
//! commands, checked through `dfm.check`, the validation worker and the write to
//! `.intcad/checks/runs/<run>/`, then read back through `inspect.dfm`.
//!
//! See `docs/dfm-catalogue-acceptance.md` for the sample table, the exact commands, and what to
//! change when a rule's fact family lands. This file is the one place a later slice edits: add a
//! sample to [`SAMPLE_IDS`], build it in [`build`], and add its expectation in [`expectations`].
//!
//! Select one sample with `SOLIDCRAFT_DFM_SAMPLE=l_bracket`; change the shared geometry with
//! `SOLIDCRAFT_DFM_FLANGE_LEG=4` and the declared tolerance with
//! `SOLIDCRAFT_DFM_TOLERANCE_MM=0.3`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use solidcraft_doc::sheet_dfm;
use solidcraft_engine::Session;
use solidcraft_engine::dfm::service;

/// The samples this harness drives. A later slice that lands a fact family keeps the sample and
/// changes its expectation in [`expectations`], never the rule's number.
const SAMPLE_IDS: &[&str] = &[
    "l_bracket",
    "u_channel",
    "no_relief",
    "relief_small",
    "plate",
    "plate_mixed",
    "spacing",
    "countersink",
    "mixed_radii",
    "long_bend",
    "hole_bend",
];

/// The sheet rule every sample uses: 2 mm, K 0.5, 1 mm inside radius. With it a 10 mm flange has
/// a 7 mm leg past the bend and a 4 mm flange only 1 mm, so one number changes a flange result.
fn sheet_rule(s: &mut Session, relief_width_mm: f64) {
    sheet_rule_sizes(s, relief_width_mm, relief_width_mm);
}

fn sheet_rule_sizes(s: &mut Session, relief_width_mm: f64, relief_depth_mm: f64) {
    exec(
        s,
        "sheet.manage_rules",
        json!({
            "name": "Acceptance",
            "thickness": "2 mm",
            "k_factor": "0.5",
            "bend_radius": format!("{} mm", env_number("SOLIDCRAFT_DFM_BEND_RADIUS", 1.0)),
            "relief_width": format!("{relief_width_mm} mm"),
            "relief_depth": format!("{relief_depth_mm} mm"),
            "active": true,
        }),
    );
}

/// The rule used by the corner-relief sample: a rectangular 1 mm corner cut, so the sheet model
/// records both the shape and the size the fact layer needs.
fn sheet_rule_corner(s: &mut Session) {
    let size = env_number("SOLIDCRAFT_DFM_RELIEF_SIZE", 1.0);
    exec(
        s,
        "sheet.manage_rules",
        json!({
            "name": "AcceptanceCorner",
            "thickness": "2 mm",
            "k_factor": "0.5",
            "bend_radius": "1 mm",
            "relief_width": format!("{} mm", size / 2.0),
            "relief_depth": format!("{} mm", size / 2.0),
            "corner_relief": format!("{size} mm"),
            "corner_relief_shape": "rectangular",
            "active": true,
        }),
    );
}

fn base(s: &mut Session, width: f64, depth: f64) {
    exec(s, "sketch.create", json!({"plane": "XY", "name": "Base"}));
    exec(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [width, depth]}));
    exec(s, "sketch.finish", json!({}));
    exec(s, "sheet.flange", json!({"sketch": "Base"}));
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    match s.execute(id, &params) {
        Ok(value) => value,
        Err(error) => panic!("{id} {params}: {error}"),
    }
}

fn exec(s: &mut Session, id: &str, params: Value) -> Value {
    run(s, id, params)
}

/// The shared dimensions a later slice can change from the shell.
#[derive(Clone, Copy, Debug)]
struct Config {
    /// The edge flange's height: 10 mm gives a 7 mm leg, 4 mm gives 1 mm.
    flange_leg_mm: f64,
    /// The declared plus-or-minus across one bend: 0.2 mm is the test shop's one-bend value.
    tolerance_mm: f64,
    /// The U-channel's base width, tangent to tangent.
    base_width_mm: f64,
    /// Centre distance between the two holes in the spacing sample.
    hole_gap_mm: f64,
    /// The countersink's rim diameter over the Ø4 hole.
    cs_diameter_mm: f64,
    /// The second bend's inside radius in the mixed-radii sample.
    second_radius_mm: f64,
}

impl Config {
    fn from_env() -> Config {
        Config {
            flange_leg_mm: env_number("SOLIDCRAFT_DFM_FLANGE_LEG", 10.0),
            tolerance_mm: env_number("SOLIDCRAFT_DFM_TOLERANCE_MM", 0.1),
            base_width_mm: env_number("SOLIDCRAFT_DFM_BASE_WIDTH", 60.0),
            hole_gap_mm: env_number("SOLIDCRAFT_DFM_HOLE_GAP", 4.0),
            cs_diameter_mm: env_number("SOLIDCRAFT_DFM_CS_DIAMETER", 6.0),
            second_radius_mm: env_number("SOLIDCRAFT_DFM_SECOND_RADIUS", 3.0),
        }
    }
}

fn env_number(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|value| value.trim().parse::<f64>().ok()).filter(|value| value.is_finite() && *value > 0.0).unwrap_or(default)
}

/// One built sample: the session and any declaration to make.
struct Built {
    session: Session,
    declaration: Option<Value>,
}

fn build(id: &str, cfg: Config) -> Built {
    let mut s = Session::default();
    let declaration = match id {
        "l_bracket" => {
            // One 90° bend: the base's long side, with a hole on each flat.
            sheet_rule(&mut s, 4.0);
            base(&mut s, 60.0, 40.0);
            exec(&mut s, "sheet.flange", json!({"edges": [[30, 0, 2]], "height": cfg.flange_leg_mm}));
            exec(&mut s, "solid.hole", json!({"position": [30, 20, 2], "diameter": 3}));
            let on_flange = (cfg.flange_leg_mm * 0.7).clamp(3.5, cfg.flange_leg_mm - 1.0);
            exec(&mut s, "solid.hole", json!({"position": [30, 2, on_flange], "diameter": 3}));
            Some(across_one_bend(&s, cfg.tolerance_mm))
        }
        "u_channel" => {
            // Two parallel same-way bends: the left and right edges both turn up.
            sheet_rule(&mut s, 4.0);
            base(&mut s, cfg.base_width_mm, 40.0);
            exec(&mut s, "sheet.flange", json!({"edges": [[0, 20, 2], [cfg.base_width_mm, 20, 2]], "height": cfg.flange_leg_mm}));
            None
        }
        "no_relief" => {
            // Two flanges meeting at a base corner: the second flat starts on the first bend's
            // line with no margin, and the zero relief width leaves the corner unrelieved.
            sheet_rule(&mut s, 0.0);
            base(&mut s, 60.0, 40.0);
            exec(&mut s, "sheet.flange", json!({"edges": [[30, 0, 2], [60, 20, 2]], "height": cfg.flange_leg_mm}));
            None
        }
        "plate" => {
            // A 2 mm hole (below the shop's 3 mm) and a hole whose wall is 1 mm from the outline.
            sheet_rule(&mut s, 4.0);
            base(&mut s, 60.0, 40.0);
            exec(&mut s, "solid.hole", json!({"position": [30, 20, 2], "diameter": 2}));
            exec(&mut s, "solid.hole", json!({"position": [2, 20, 2], "diameter": 2}));
            None
        }
        "relief_small" => {
            // A corner relief 1 mm across the corner, 0.5 mm into each bend end, below the test
            // shop's 2 mm width and 1 mm depth.
            sheet_rule_corner(&mut s);
            base(&mut s, 60.0, 40.0);
            exec(
                &mut s,
                "sheet.flange",
                json!({
                    "edges": [[30, 0, 2], [60, 20, 2]],
                    "height": cfg.flange_leg_mm,
                    "corner_relief_shape": "rectangular",
                    "corner_relief": format!("{} mm", env_number("SOLIDCRAFT_DFM_RELIEF_SIZE", 1.0)),
                }),
            );
            None
        }
        "long_bend" => {
            // A 600 mm bend line against the TEST-ONLY 500 mm upper row.
            sheet_rule(&mut s, 4.0);
            base(&mut s, 600.0, 40.0);
            exec(&mut s, "sheet.flange", json!({"edges": [[300, 0, 2]], "height": 10}));
            None
        }
        "hole_bend" => {
            // A hole 1.5 mm from the bend tangent, below the TEST-ONLY 2.5 mm row.
            sheet_rule(&mut s, 4.0);
            base(&mut s, 60.0, 40.0);
            exec(&mut s, "sheet.flange", json!({"edges": [[30, 0, 2]], "height": 10}));
            exec(&mut s, "solid.hole", json!({"position": [30, 6, 2], "diameter": 3}));
            None
        }
        "plate_mixed" => {
            // A measurable round hole beside a non-circular opening: the hole rules can answer
            // for the hole, while the report must still say the opening was not checked.
            sheet_rule(&mut s, 4.0);
            base(&mut s, 60.0, 40.0);
            exec(&mut s, "solid.hole", json!({"position": [50, 20, 2], "diameter": 5}));
            exec(&mut s, "sketch.create", json!({"plane": "XY", "name": "Slot"}));
            exec(&mut s, "sketch.rectangle.two_point", json!({"p0": [14, 14], "p1": [26, 26]}));
            exec(&mut s, "sketch.finish", json!({}));
            exec(&mut s, "solid.extrude", json!({"sketch": "Slot", "distance": 2, "direction": "positive", "operation": "cut"}));
            None
        }
        "spacing" => {
            // Two Ø3 holes; the gap changes the wall between them without changing the outline.
            sheet_rule(&mut s, 4.0);
            base(&mut s, 60.0, 40.0);
            exec(&mut s, "solid.hole", json!({"position": [20, 20, 2], "diameter": 3}));
            exec(&mut s, "solid.hole", json!({"position": [20.0 + cfg.hole_gap_mm, 20, 2], "diameter": 3}));
            None
        }
        "countersink" => {
            // A countersunk round through hole on a native sheet. Ø6 over Ø4 at 90° is 1 mm deep
            // in a 2 mm sheet and leaves a 1 mm land.
            sheet_rule(&mut s, 4.0);
            base(&mut s, 60.0, 40.0);
            let (sx, sy, sa) =
                (env_number("SOLIDCRAFT_DFM_SINK_X", 50.0), env_number("SOLIDCRAFT_DFM_SINK_Y", 30.0), env_number("SOLIDCRAFT_DFM_SINK_ANGLE", 90.0));
            exec(
                &mut s,
                "solid.hole",
                json!({"position": [sx, sy, 2], "diameter": 4, "type": "countersink", "cs_diameter": cfg.cs_diameter_mm, "cs_angle": sa}),
            );
            None
        }
        "mixed_radii" => {
            // Two edge flanges with their own radii: the part carries two inside radii and the
            // consistency rule has something to compare.
            sheet_rule(&mut s, 4.0);
            base(&mut s, 60.0, 40.0);
            exec(&mut s, "sheet.flange", json!({"edges": [[0, 20, 2]], "height": cfg.flange_leg_mm, "radius": "1 mm"}));
            exec(
                &mut s,
                "sheet.flange",
                json!({"edges": [[60, 20, 2]], "height": cfg.flange_leg_mm, "radius": format!("{} mm", cfg.second_radius_mm)}),
            );
            None
        }
        other => panic!("unknown sample `{other}`; known: {SAMPLE_IDS:?}"),
    };
    if s.model.state().sheets.is_empty() {
        panic!("{id}: no native sheet body was built");
    }
    Built { session: s, declaration }
}

/// A declaration from the two holes on different flats of the L-bracket. The points are the
/// measured folded hole centres, which is what a person reads from the report's `part.holes`.
fn across_one_bend(s: &Session, tolerance_mm: f64) -> Value {
    let facts = sheet_dfm::facts(&s.model.state().sheets[0]).expect("sheet facts");
    let base = facts.holes.iter().find(|hole| hole.flat.is_none() && hole.flat_assigned).expect("a hole on the base");
    let flange = facts.holes.iter().find(|hole| hole.flat.is_some() && hole.flat_assigned).expect("a hole on the flange");
    assert_ne!(base.flat, flange.flat, "the declaration must span two flats");
    json!({
        "critical": [{
            "name": "base to flange",
            "fromMm": [base.x_mm, base.y_mm, base.z_mm],
            "toMm": [flange.x_mm, flange.y_mm, flange.z_mm],
            "toleranceMm": tolerance_mm,
        }]
    })
}

/// What one target rule must say for one sample today.
#[derive(Debug)]
struct Expectation {
    rule: &'static str,
    /// The worst state in `findings[].rules[].state`.
    state: &'static str,
    /// The state of the one finding this sample is about.
    finding_state: &'static str,
    /// The measured value, when the rule measured one.
    measured_mm: Option<f64>,
    /// The finding's `code`, when it has one.
    code: Option<&'static str>,
}

/// The target rows per sample. These are the assertions a later slice changes when the missing
/// fact families arrive; the rule thresholds themselves are never relaxed.
fn expectations(id: &str, cfg: Config) -> Vec<Expectation> {
    let leg = cfg.flange_leg_mm - 3.0;
    let radius = env_number("SOLIDCRAFT_DFM_BEND_RADIUS", 1.0);
    let relief_size = env_number("SOLIDCRAFT_DFM_RELIEF_SIZE", 1.0);
    let (sink_x, sink_y, sink_angle) =
        (env_number("SOLIDCRAFT_DFM_SINK_X", 50.0), env_number("SOLIDCRAFT_DFM_SINK_Y", 30.0), env_number("SOLIDCRAFT_DFM_SINK_ANGLE", 90.0));
    let rim = cfg.cs_diameter_mm / 2.0;
    // The two bend zones take a 3 mm allowance each at these test-rule values.
    let web = cfg.base_width_mm - 6.0;
    let wall = cfg.hole_gap_mm - 3.0;
    match id {
        "l_bracket" => vec![
            Expectation {
                rule: "sm.tolerance_across_bends",
                // 0.1 mm is tighter than the test shop's 0.2 mm across one bend; 0.3 mm is looser.
                state: if cfg.tolerance_mm < 0.2 { "failed" } else { "passed" },
                finding_state: if cfg.tolerance_mm < 0.2 { "failed" } else { "passed" },
                measured_mm: Some(cfg.tolerance_mm),
                code: (cfg.tolerance_mm < 0.2).then_some("TOLERANCE_TIGHTER_THAN_SHOP"),
            },
            Expectation {
                rule: "sm.inside_radius",
                state: if radius < 0.5 { "failed" } else { "passed" },
                finding_state: if radius < 0.5 { "failed" } else { "passed" },
                measured_mm: (radius < 0.5).then_some(radius),
                code: None,
            },
            Expectation { rule: "sm.max_bend_length", state: "passed", finding_state: "passed", measured_mm: None, code: None },
            Expectation { rule: "sm.hole_to_bend", state: "passed", finding_state: "passed", measured_mm: None, code: None },
            // The 8 mm shop floor: a 10 mm height gives a 7 mm leg and fails; 14 mm gives 11 mm.
            Expectation {
                rule: "sm.min_flange",
                state: if leg < 8.0 { "failed" } else { "passed" },
                finding_state: if leg < 8.0 { "failed" } else { "passed" },
                measured_mm: (leg < 8.0 && (radius - 1.0).abs() < 1e-9).then_some(leg),
                code: (leg < 8.0).then_some("FLANGE_BELOW_LIMIT"),
            },
        ],
        "u_channel" => vec![
            Expectation {
                rule: "sm.min_web_parallel_bends",
                state: if web < 5.0 { "failed" } else { "passed" },
                finding_state: if web < 5.0 { "failed" } else { "passed" },
                measured_mm: (web < 5.0).then_some(web),
                code: (web < 5.0).then_some("WEB_BELOW_LIMIT"),
            },
            Expectation {
                rule: "sm.u_channel_ratio",
                state: if leg / web > 0.2 { "failed" } else { "passed" },
                finding_state: if leg / web > 0.2 { "failed" } else { "passed" },
                measured_mm: (leg / web > 0.2).then_some(leg),
                code: (leg / web > 0.2).then_some("U_FLANGE_ABOVE_LIMIT"),
            },
        ],
        "no_relief" => vec![Expectation {
            rule: "sm.bend_relief_present",
            state: "failed",
            finding_state: "failed",
            measured_mm: None,
            code: Some("RELIEF_NO_MARGIN"),
        }],
        "plate" => vec![
            Expectation { rule: "sm.hole_min_diameter", state: "failed", finding_state: "failed", measured_mm: Some(2.0), code: None },
            // The far hole passes at 19 mm; the near hole's 1 mm wall is the failing one.
            Expectation { rule: "sm.hole_to_edge", state: "failed", finding_state: "failed", measured_mm: Some(1.0), code: None },
            Expectation { rule: "sm.hole_spacing", state: "passed", finding_state: "passed", measured_mm: None, code: None },
        ],
        "relief_small" => vec![
            Expectation { rule: "sm.bend_relief_present", state: "passed", finding_state: "passed", measured_mm: None, code: None },
            // A rectangular 1 mm corner relief is 1 mm across the bend and 0.5 mm deep at each
            // end; the TEST-ONLY shop rows are 2 mm and 1 mm, so both fail.
            Expectation {
                rule: "sm.bend_relief_width",
                state: if relief_size < 2.0 { "failed" } else { "passed" },
                finding_state: if relief_size < 2.0 { "failed" } else { "passed" },
                measured_mm: (relief_size < 2.0).then_some(relief_size),
                code: (relief_size < 2.0).then_some("RELIEF_WIDTH_BELOW_LIMIT"),
            },
            Expectation {
                rule: "sm.bend_relief_depth",
                state: if relief_size / 2.0 < 1.0 { "failed" } else { "passed" },
                finding_state: if relief_size / 2.0 < 1.0 { "failed" } else { "passed" },
                measured_mm: (relief_size / 2.0 < 1.0).then_some(relief_size / 2.0),
                code: (relief_size / 2.0 < 1.0).then_some("RELIEF_DEPTH_BELOW_LIMIT"),
            },
        ],
        "long_bend" => {
            vec![Expectation { rule: "sm.max_bend_length", state: "failed", finding_state: "failed", measured_mm: Some(600.0), code: None }]
        }
        "hole_bend" => vec![Expectation { rule: "sm.hole_to_bend", state: "failed", finding_state: "failed", measured_mm: None, code: None }],
        "plate_mixed" => vec![Expectation {
            rule: "sm.hole_min_diameter",
            // A non-circular opening is present, so the hole rule says it cannot answer rather
            // than guessing from the round hole alone.
            state: "unsupported",
            finding_state: "unsupported",
            measured_mm: None,
            code: None,
        }],
        "spacing" => vec![Expectation {
            rule: "sm.hole_spacing",
            state: if wall < 4.0 { "failed" } else { "passed" },
            finding_state: if wall < 4.0 { "failed" } else { "passed" },
            measured_mm: (wall < 4.0).then_some(wall),
            code: (wall < 4.0).then_some("HOLE_SPACING_BELOW_LIMIT"),
        }],
        "countersink" => {
            // Ø6 over Ø4 at 90° is 1 mm deep; Ø4.5 is 0.25 mm, below the 0.75 mm row. The rim is
            // 3 mm across, 17 mm from the nearest long edge, and leaves a 1 mm land.
            let depth = cfg.cs_diameter_mm / 2.0 - 2.0;
            vec![
                Expectation {
                    rule: "sm.countersink_depth",
                    state: if depth > 0.75 { "failed" } else { "passed" },
                    finding_state: if depth > 0.75 { "failed" } else { "passed" },
                    measured_mm: Some(depth),
                    code: (depth > 0.75).then_some("COUNTERSINK_DEPTH_ABOVE_LIMIT"),
                },
                {
                    let edge = sink_x.min(60.0 - sink_x).min(sink_y).min(40.0 - sink_y) - rim;
                    Expectation {
                        rule: "sm.countersink_to_edge",
                        state: if edge < 3.0 { "failed" } else { "passed" },
                        finding_state: if edge < 3.0 { "failed" } else { "passed" },
                        measured_mm: Some(edge),
                        code: None,
                    }
                },
                Expectation {
                    rule: "sm.countersink_land",
                    state: if 4.0 - rim < 0.5 { "failed" } else { "passed" },
                    finding_state: if 4.0 - rim < 0.5 { "failed" } else { "passed" },
                    measured_mm: Some(4.0 - rim),
                    code: None,
                },
                Expectation {
                    rule: "sm.countersink_angle",
                    state: if (sink_angle - 90.0).abs() > 1.0 { "unsupported" } else { "passed" },
                    finding_state: if (sink_angle - 90.0).abs() > 1.0 { "unsupported" } else { "passed" },
                    measured_mm: None,
                    code: None,
                },
                // This native sample's countersink report has no bend gaps (the rule is
                // `checked`); the adapter-level tests carry the pass/fail bend evidence.
                Expectation { rule: "sm.countersink_to_bend", state: "checked", finding_state: "checked", measured_mm: None, code: None },
            ]
        }
        "mixed_radii" => vec![Expectation {
            rule: "sm.bend_radius_consistent",
            state: if (cfg.second_radius_mm - 1.0).abs() > 0.05 { "failed" } else { "passed" },
            finding_state: if (cfg.second_radius_mm - 1.0).abs() > 0.05 { "failed" } else { "passed" },
            measured_mm: ((cfg.second_radius_mm - 1.0).abs() > 0.05).then_some(2.0),
            code: ((cfg.second_radius_mm - 1.0).abs() > 0.05).then_some("BEND_RADII_MIXED"),
        }],
        other => panic!("unknown sample `{other}`; known: {SAMPLE_IDS:?}"),
    }
}

/// The project folder and the two process-wide stores, made once per test binary.
struct Roots {
    scratch: PathBuf,
    shop: PathBuf,
}

static ROOTS: OnceLock<Roots> = OnceLock::new();
/// The DFM service keeps process-wide roots and one worker, so the runs in this binary are serial.
static GUARD: Mutex<()> = Mutex::new(());

fn roots() -> &'static Roots {
    ROOTS.get_or_init(|| {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let scratch = std::env::temp_dir().join(format!("intcad-dfm-acceptance-{}-{nanos}", std::process::id()));
        let candidates = scratch.join("candidates");
        let validation = scratch.join("validation");
        std::fs::create_dir_all(&scratch).expect("scratch");
        std::fs::create_dir_all(&candidates).expect("candidates");
        std::fs::create_dir_all(&validation).expect("validation");
        solidcraft_engine::cmd::set_candidate_store(Some(candidates));
        service::set_store_root(Some(validation));
        Roots { scratch, shop: Path::new(env!("CARGO_MANIFEST_DIR")).join("src/project/test_data/critical_shop.json") }
    })
}

fn selected_samples() -> Vec<&'static str> {
    match std::env::var("SOLIDCRAFT_DFM_SAMPLE") {
        Ok(one) => {
            let id = SAMPLE_IDS.iter().copied().find(|known| *known == one.trim()).unwrap_or_else(|| {
                panic!("SOLIDCRAFT_DFM_SAMPLE=`{one}` is not one of {SAMPLE_IDS:?}");
            });
            vec![id]
        }
        Err(_) => SAMPLE_IDS.to_vec(),
    }
}

/// Put the built design in a new project with the test shop and (when given) its declaration.
fn project_for(built: &mut Built, id: &str) -> Value {
    let roots = roots();
    let created = exec(&mut built.session, "project.create", json!({"name": id, "folder": roots.scratch.to_string_lossy()}));
    exec(&mut built.session, "project.settings_set", json!({"process": "press brake", "material": "5052-H32"}));
    exec(&mut built.session, "project.data_set", json!({"kind": "factory", "path": roots.shop.to_string_lossy()}));
    if let Some(declaration) = &built.declaration {
        exec(&mut built.session, "project.critical_set", declaration.clone());
    }
    created
}

/// Queue one `dfm.check` and wait for its worker record, returning the run id.
fn check_and_wait(s: &mut Session, id: &str) -> String {
    let before = service::status().last_run;
    let ticket = exec(s, "dfm.check", json!({}));
    assert!(ticket["sheets"].as_u64().unwrap_or(0) >= 1, "{id}: dfm.check found no sheet: {ticket}");
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let status = service::status();
        if let Some(error) = status.error {
            panic!("{id}: the DFM worker failed: {error}");
        }
        if !status.running
            && let Some(run) = status.last_run.clone()
            && before.as_deref() != Some(run.as_str())
        {
            assert_eq!(status.last_outcome.as_deref(), Some("current"), "{id}: {status:?}");
            return run;
        }
        assert!(Instant::now() < deadline, "{id}: the check did not finish: {status:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn inspect(s: &mut Session, run: &str) -> Value {
    exec(s, "inspect.dfm", json!({"run": run}))
}

fn rule_row<'a>(findings: &'a Value, rule: &str) -> &'a Value {
    findings["rules"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["ruleId"].as_str() == Some(rule)))
        .unwrap_or_else(|| panic!("the report has no `{rule}` row: {}", findings["rules"]))
}

fn target_findings<'a>(findings: &'a Value, rule: &str) -> Vec<&'a Value> {
    findings["findings"]
        .as_array()
        .map(|rows| rows.iter().filter(|row| row["rules"].as_array().is_some_and(|rules| rules.iter().any(|r| r.as_str() == Some(rule)))).collect())
        .unwrap_or_default()
}

/// The full catalogue each report must speak for: every non-gate `RULE_ORDER` entry plus the
/// App-owned flat self-overlap rule. A rule with no applicable object is still a row, not a
/// missing one; the four gates are findings, not rows.
fn assert_catalogue_rows(id: &str, findings: &Value) {
    let rows = findings["rules"].as_array().unwrap_or_else(|| panic!("{id}: no rule rows"));
    for rule in solidcraft_dfm_rules::RULE_ORDER.iter().copied().filter(|rule| !rule.contains("gate_")).chain(std::iter::once("app.flat_no_overlap"))
    {
        assert!(rows.iter().any(|row| row["ruleId"].as_str() == Some(rule)), "{id}: the report has no `{rule}` row");
    }
}

fn assert_sample(id: &str, report: &Value, expected: &[Expectation]) {
    assert_eq!(report["available"], true, "{id}: no report: {report}");
    assert_eq!(report["outcome"], "current", "{id}: {report}");
    let findings = &report["findings"];
    assert!(findings["runId"].as_str().is_some_and(|run| !run.is_empty()), "{id}: no run identity");
    assert!(findings["candidateDigest"].as_str().is_some_and(|digest| !digest.is_empty()), "{id}: no model identity");
    assert!(findings["configDigest"].as_str().is_some_and(|digest| !digest.is_empty()), "{id}: no configuration identity");
    assert_catalogue_rows(id, findings);
    if std::env::var_os("SOLIDCRAFT_DFM_PRINT").is_some() {
        eprintln!("{id} all rows: {}", findings["rules"]);
    }
    let not_checked = findings["notChecked"].as_str().unwrap_or_default();
    assert!(!not_checked.trim().is_empty(), "{id}: no not-checked line");
    assert!(not_checked.contains("were not checked"), "{id}: {not_checked}");
    if id == "plate_mixed" {
        assert!(not_checked.contains("non-circular opening"), "{id}: the unmeasured opening is not named: {not_checked}");
    }

    for want in expected {
        let row = rule_row(findings, want.rule);
        assert_eq!(row["state"], want.state, "{id} {}: {row}", want.rule);
        assert!(row["count"].as_u64().is_some(), "{id} {}: no finding count: {row}", want.rule);
        let rows = target_findings(findings, want.rule);
        assert!(!rows.is_empty(), "{id} {}: the rule row has no finding", want.rule);
        for finding in &rows {
            assert!(!finding["id"].as_str().unwrap_or_default().is_empty(), "{id} {}: finding without an id: {finding}", want.rule);
            assert!(!finding["body"].as_str().unwrap_or_default().is_empty(), "{id} {}: finding without a body: {finding}", want.rule);
        }
        let finding = rows
            .iter()
            .find(|finding| {
                finding["state"].as_str() == Some(want.finding_state)
                    && match want.measured_mm {
                        Some(measured) => (finding["measured"]["value"].as_f64().unwrap_or(f64::NAN) - measured).abs() < 1e-9,
                        None => true,
                    }
            })
            .unwrap_or_else(|| panic!("{id} {}: no `{}` finding at {want:?}: {rows:?}", want.rule, want.finding_state));
        if let Some(measured) = want.measured_mm {
            let got = finding["measured"]["value"].as_f64().unwrap_or(f64::NAN);
            assert!((got - measured).abs() < 1e-9, "{id} {}: measured {got}, expected {measured}: {finding}", want.rule);
            // A failing or advisory finding that measured a number located it: the anchor, flat
            // point or flat outline is what the viewport selects. A passing summary line need not
            // name one object. `sm.bend_radius_consistent` is a whole-part result (several bends
            // with different radii), so it has no single object to select.
            let needs_location = matches!(want.finding_state, "failed" | "advisory") && want.rule != "sm.bend_radius_consistent";
            let located = finding["anchor"].as_array().is_some_and(|at| at.len() == 3)
                || finding["flat"].as_array().is_some_and(|at| at.len() == 2)
                || finding["outline"].as_array().is_some_and(|at| !at.is_empty());
            assert!(!needs_location || located, "{id} {}: a measured finding has no location: {finding}", want.rule);
        }
        if let Some(code) = want.code {
            assert_eq!(finding["code"], code, "{id} {}: {finding}", want.rule);
        }
        if std::env::var_os("SOLIDCRAFT_DFM_PRINT").is_some() {
            eprintln!("{id} {}: row {row}", want.rule);
            eprintln!("{id} {}: finding {finding}", want.rule);
        }
        if want.state == "unsupported" {
            assert!(not_checked.contains(want.rule), "{id} {}: the not-checked text does not name the rule: {not_checked}", want.rule);
        }
    }
}

fn assert_tolerance_state(report: &Value, expected_state: &str, measured: f64) {
    let findings = &report["findings"];
    let row = rule_row(findings, "sm.tolerance_across_bends");
    assert_eq!(row["state"], expected_state, "{row}");
    let rows = target_findings(findings, "sm.tolerance_across_bends");
    let finding = rows.first().expect("the tolerance finding");
    assert!((finding["measured"]["value"].as_f64().unwrap_or(f64::NAN) - measured).abs() < 1e-9, "{finding}");
    assert!(finding["anchor"].as_array().is_some_and(|at| at.len() == 3), "the tolerance finding has no midpoint: {finding}");
}

/// The full-catalogue sample run, one `dfm.check` per sample through the real worker.
#[test]
fn catalogue_samples_reach_the_written_report() {
    let _guard = GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let cfg = Config::from_env();
    roots();
    let selected = selected_samples();

    for id in selected {
        let mut built = build(id, cfg);
        project_for(&mut built, id);
        let run = check_and_wait(&mut built.session, id);
        let report = inspect(&mut built.session, &run);
        assert_sample(id, &report, &expectations(id, cfg));
        let _ = std::fs::remove_dir_all(roots().scratch.join(id));
    }
}

/// A declaration is frozen into the run, changing it makes the old report stale, and checking
/// again with a looser plus-or-minus turns the same rule into a pass.
#[test]
fn a_changed_declaration_makes_the_old_report_stale_and_the_next_check_decides() {
    let _guard = GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let cfg = Config::from_env();
    roots();
    let mut built = build("l_bracket", cfg);
    project_for(&mut built, "l_bracket_declaration");
    let first = check_and_wait(&mut built.session, "l_bracket_declaration");
    let report = inspect(&mut built.session, &first);
    assert_eq!(report["freshness"], "current", "{report}");
    assert_eq!(report["configDigest"], report["findings"]["configDigest"], "the run froze the live configuration");
    let config_before = report["configDigest"].clone();
    assert_tolerance_state(&report, if cfg.tolerance_mm < 0.2 { "failed" } else { "passed" }, cfg.tolerance_mm);

    // The same two holes, a looser plus-or-minus: the old report no longer describes the project.
    let looser = if cfg.tolerance_mm < 0.2 { 0.3 } else { 0.1 };
    let Some(declaration) = &built.declaration else { panic!("the L-bracket declares a dimension") };
    let mut changed = declaration.clone();
    changed["critical"][0]["toleranceMm"] = json!(looser);
    exec(&mut built.session, "project.critical_set", changed);
    let stale = inspect(&mut built.session, &first);
    assert_eq!(stale["freshness"], "staleConfig", "a changed declaration must stale the old report: {stale}");
    assert_eq!(stale["findings"]["runId"], first, "the old report keeps its own identity");

    let second = check_and_wait(&mut built.session, "l_bracket_declaration");
    assert_ne!(second, first);
    let report = inspect(&mut built.session, &second);
    assert_eq!(report["freshness"], "current", "{report}");
    assert_ne!(report["configDigest"], config_before, "the declaration is part of the frozen configuration digest");
    assert_tolerance_state(&report, if looser < 0.2 { "failed" } else { "passed" }, looser);
    let _ = std::fs::remove_dir_all(roots().scratch.join("l_bracket_declaration"));
}

/// A missing number, a generic reference and a declared thickness are three different
/// "not a pass" states; none of them may be reported as a pass or as measured geometry.
#[test]
fn gates_and_generic_values_never_fake_a_pass() {
    let _guard = GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    roots();
    let mut built = build("plate", Config::from_env());
    let created = exec(&mut built.session, "project.create", json!({"name": "gates", "folder": roots().scratch.to_string_lossy()}));
    exec(&mut built.session, "project.settings_set", json!({"process": "press brake", "material": "5052-H32"}));
    let _ = created;

    // No selected data: the numeric rule has no number, so it cannot decide.
    let run = check_and_wait(&mut built.session, "gates-none");
    let report = inspect(&mut built.session, &run);
    let hole = rule_row(&report["findings"], "sm.hole_min_diameter");
    assert_eq!(hole["state"], "unsupported", "{hole}");
    let no_number = target_findings(&report["findings"], "sm.hole_min_diameter");
    assert!(no_number.iter().any(|finding| finding["code"] == "NO_SHOP_NUMBER"), "{no_number:?}");

    // A project override is a team-reported number: it may advise, never fail. The layer, not
    // the value, decides whether a number can conclude.
    exec(
        &mut built.session,
        "dfm.override",
        json!({
            "rule": "sm.hole_min_diameter",
            "limit": "min",
            "old": {"mm": 4.0},
            "value": {"mm": 3.0},
            "reason": "TEST-ONLY acceptance row",
            "by": "DFM acceptance",
        }),
    );
    let run = check_and_wait(&mut built.session, "gates-generic");
    let report = inspect(&mut built.session, &run);
    let hole = rule_row(&report["findings"], "sm.hole_min_diameter");
    assert_eq!(hole["state"], "advisory", "{hole}");
    let advisory = target_findings(&report["findings"], "sm.hole_min_diameter");
    assert!(advisory.iter().any(|finding| finding["state"] == "advisory"), "{advisory:?}");

    // A declared thickness that conflicts with the model blocks the dependent rules, but it is
    // one body's problem: the report still says which checks did not run.
    exec(&mut built.session, "project.settings_set", json!({"thickness_mm": 3.0}));
    let run = check_and_wait(&mut built.session, "gates-thickness");
    let report = inspect(&mut built.session, &run);
    let gate = rule_row(&report["findings"], "sm.gate_sheet_thickness");
    assert_eq!(gate["state"], "failed", "{gate}");
    let hole = rule_row(&report["findings"], "sm.hole_min_diameter");
    assert_eq!(hole["state"], "unsupported", "{hole}");
    let not_checked = report["findings"]["notChecked"].as_str().unwrap_or_default();
    assert!(not_checked.contains("sm.hole_min_diameter"), "the blocked rule is not named: {not_checked}");
    let _ = std::fs::remove_dir_all(roots().scratch.join("gates"));
}
