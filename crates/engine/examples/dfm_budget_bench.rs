//! Measure the Review (`dfm.check`) run path on three model sizes.
//!
//! This is the evidence harness for a possible `(Capability::Review,
//! EntryCategory::ReviewPanel)` safe-stop bound. It does not decide a bound and it does not touch
//! the budget table; it only times the same in-process path a controlled Review press uses:
//! project freeze, snapshot capture, the background worker, candidate verification, the native
//! rebuild, every `dfm.sheet` check, the App's flat-overlap check, and the findings write.
//!
//! The measured sizes are:
//!
//! - `sample_bracket`: the bundled "Sheet Metal Bracket" sample, built from its shipped script;
//! - `small`: a purpose-built 120 x 60 mm plate with eight holes and two edge flanges;
//! - `large`: four 400 x 300 mm perforated panels, each with one edge flange (a single
//!   `sheet.perforate` feature per body gives each panel about 500 holes);
//! - `rebuild`: one 200 x 120 mm plate with 32 `solid.hole` features, which stresses the native
//!   rebuild's B-rep booleans rather than the sheet facts.
//!
//! Each size runs one warm-up pass (the first pass also opens the validation port and pays for
//! cold file caches) and then `--repeats` measured passes. The output is a Markdown table of the
//! size metric and the median, worst and cold wall time. Wall time is taken around
//! [`service::start_reserved`] up to the point the worker's status stops running, so it includes
//! everything between a Review press and a settled run.
//!
//! Run it with:
//!
//! ```text
//! CARGO_TARGET_DIR=.targets/dfm-budget cargo run --release \
//!     --example dfm_budget_bench -- --repeats 5
//! ```
//!
//! Add `--out FILE` to also write the table and the machine facts to a file. This is a
//! measurement harness, not a test: it needs no assertions about rule outcomes, and a shared,
//! noisy machine can change the numbers, so it is not part of CI.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;
use solidcraft_engine::Session;
use solidcraft_engine::dfm::run::Reservation;
use solidcraft_engine::dfm::service;
use solidcraft_engine::validation::{generate_run_id, now_ms};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The longest a single measured run may take before the harness gives up.
const RUN_DEADLINE: Duration = Duration::from_secs(600);

/// The reservation stop used while measuring. It is a harness guard, not a proposed bound: it
/// only keeps a runaway run from waiting forever.
const RESERVATION_GUARD_MS: u64 = 3_600_000;

/// One built model and the facts a reader needs to place it on the size axis.
struct Model {
    label: &'static str,
    session: Session,
    bodies: usize,
    holes: usize,
    flanges: usize,
    features: usize,
    design_bytes: u64,
}

/// One measured pass.
struct Timed {
    millis: f64,
    outcome: String,
    run: String,
}

fn main() -> Result<()> {
    let mut repeats = 5usize;
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(key) = args.next() {
        match key.as_str() {
            "--help" | "-h" => {
                println!("dfm_budget_bench [--repeats 1..50] [--out FILE]");
                return Ok(());
            }
            "--repeats" => {
                let value = args.next().ok_or("missing value for --repeats")?;
                repeats = value.parse()?;
            }
            "--out" => out = Some(args.next().ok_or("missing value for --out")?.into()),
            other => return Err(format!("unknown option `{other}` (see --help)").into()),
        }
    }
    if !(1..=50).contains(&repeats) {
        return Err("--repeats must be 1..50".into());
    }

    let root = scratch()?;
    let candidates = root.join("candidates");
    let validation = root.join("validation");
    std::fs::create_dir_all(&candidates)?;
    std::fs::create_dir_all(&validation)?;
    // The DFM service keeps process-wide roots and one worker.
    solidcraft_engine::cmd::set_candidate_store(Some(candidates.clone()));
    service::set_store_root(Some(validation));

    let mut models = [
        model("sample_bracket", sample_bracket()?, &root)?,
        model("small", small_part()?, &root)?,
        model("large", large_part()?, &root)?,
        model("rebuild", rebuild_part()?, &root)?,
    ];

    let mut report = format!(
        "# DFM Review run-path measurement\n\n\
         Machine: {} / {}, {} logical CPUs, {}. This is a shared development host, not a CI runner; \
         other work on the machine moves the numbers.\n\n\
         Each size: one warm-up pass, then {repeats} measured passes. Wall time is measured from \
         `service::start_reserved` until the background worker stops running, so it covers project \
         freeze, snapshot capture, candidate verification, the native rebuild, every `dfm.sheet` \
         check, the flat-overlap check and the findings write. `rebuild ms*` and `facts+overlap ms*` \
         are one separate out-of-band pass over the saved design, taken after the timed runs to \
         attribute the cost; they are not part of the median.\n\n\
         | size | bodies | holes | flanges | features | design KiB | cold ms | median ms | worst ms | max incl. cold ms | rebuild ms* | facts+overlap ms* | runs (ms) | outcome |\n\
         |---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        cpu_model(),
    );

    for entry in models.iter_mut() {
        let cold = timed_run(&mut entry.session, entry.bodies)?;
        if cold.outcome != "current" {
            return Err(format!("{}: the warm-up run ended `{}` ({})", entry.label, cold.outcome, cold.run).into());
        }
        let mut passes = Vec::with_capacity(repeats);
        for _ in 0..repeats {
            let pass = timed_run(&mut entry.session, entry.bodies)?;
            if pass.outcome != "current" {
                return Err(format!("{}: a measured run ended `{}` ({})", entry.label, pass.outcome, pass.run).into());
            }
            passes.push(pass.millis);
        }
        let mut all = Vec::with_capacity(passes.len() + 1);
        all.push(cold.millis);
        all.extend_from_slice(&passes);
        let median = median(&passes)?;
        let worst = passes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let listed = passes.iter().map(|ms| format!("{ms:.1}")).collect::<Vec<_>>().join(", ");
        let (rebuild_ms, facts_ms) = out_of_band(entry)?;
        report.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {} | {} |\n",
            entry.label,
            entry.bodies,
            entry.holes,
            entry.flanges,
            entry.features,
            entry.design_bytes as f64 / 1024.0,
            cold.millis,
            median,
            worst,
            all.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            rebuild_ms,
            facts_ms,
            listed,
            "current",
        ));
    }

    report.push_str(
        "\nWall time covers one Review run in the same process that built the design. It does not \
         include opening the project or the window, and it does not include the paid admission, \
         ledger or settlement calls around the run.\n",
    );
    print!("{report}");
    if let Some(path) = out {
        std::fs::write(path, &report)?;
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// Create a project for the model and measure its size facts. The project is saved by
/// `project.create`, so `dfm.check` has a saved design and a frozen configuration to use.
fn model(label: &'static str, mut session: Session, root: &Path) -> Result<Model> {
    if session.model.state().sheets.is_empty() {
        return Err(format!("{label}: the design has no native sheet body").into());
    }
    let folder = root.join(label);
    std::fs::create_dir_all(&folder)?;
    session.execute("project.create", &json!({"name": label, "folder": folder.to_string_lossy()}))?;
    session.execute("project.settings_set", &json!({"process": "press brake", "material": "5052-H32"}))?;
    let shop = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/project/test_data/critical_shop.json");
    session.execute("project.data_set", &json!({"kind": "factory", "path": shop.to_string_lossy()}))?;

    let state = session.model.state();
    let mut holes = 0usize;
    let mut flanges = 0usize;
    for sheet in &state.sheets {
        holes = holes.saturating_add(sheet.holes.len());
        for perforation in &sheet.perforations {
            holes = holes.saturating_add(sheet.perforation_holes(perforation)?.len());
        }
        flanges = flanges.saturating_add(sheet.flanges.len());
    }
    let design = session.path.as_deref().ok_or("the project did not save the design")?;
    let design_bytes = std::fs::metadata(design)?.len();
    Ok(Model { label, bodies: state.sheets.len(), holes, flanges, features: session.model.results.len(), design_bytes, session })
}

/// The bundled sample: the shipped "Sheet Metal Bracket" script, run through the normal commands.
fn sample_bracket() -> Result<Session> {
    let mut session = Session::default();
    let script = solidcraft_engine::sample::samples()
        .into_iter()
        .find(|sample| sample.name == "Sheet Metal Bracket")
        .map(|sample| (sample.script)())
        .ok_or("the bundled sample list has no Sheet Metal Bracket")?;
    session.run_script(&script)?;
    Ok(session)
}

/// A 120 x 60 mm plate with eight Ø4 holes and two 20 mm edge flanges.
fn small_part() -> Result<Session> {
    let mut session = Session::default();
    sheet_rule(&mut session, 2.0)?;
    base(&mut session, "Base", 0.0, 120.0, 60.0)?;
    for i in 0..8 {
        let x = 15.0 + i as f64 * 12.0;
        session.execute("solid.hole", &json!({"position": [x, 30.0, 2.0], "diameter": 4.0}))?;
    }
    session.execute("sheet.flange", &json!({"edges": [[0.0, 30.0, 2.0], [120.0, 30.0, 2.0]], "height": 20.0}))?;
    Ok(session)
}

/// One 200 x 120 mm plate with 32 Ø4 holes made as separate `solid.hole` features. This is the
/// rebuild axis: every hole is a B-rep boolean the native rebuild evaluates again, unlike a
/// cosmetic perforation field.
fn rebuild_part() -> Result<Session> {
    let mut session = Session::default();
    sheet_rule(&mut session, 2.0)?;
    base(&mut session, "Base", 0.0, 200.0, 120.0)?;
    for i in 0..32 {
        let x = 15.0 + (i % 8) as f64 * 24.0;
        let y = 20.0 + (i / 8) as f64 * 25.0;
        session.execute("solid.hole", &json!({"position": [x, y, 2.0], "diameter": 4.0}))?;
    }
    Ok(session)
}

/// Four perforated panels with an edge flange each: many sheet bodies, and about two thousand
/// holes, all from a small number of features.
fn large_part() -> Result<Session> {
    let mut session = Session::default();
    sheet_rule(&mut session, 1.0)?;
    for i in 0..4 {
        let x = i as f64 * 450.0;
        base(&mut session, &format!("Base{i}"), x, 400.0, 300.0)?;
        // One square perforation field per body: Ø5 holes on a 15 mm pitch, 6 mm margin.
        session.execute("sheet.perforate", &json!({"diameter": "5 mm", "pitch": "15 mm", "margin": "6 mm", "pattern": "square"}))?;
        session.execute("sheet.flange", &json!({"edges": [[x + 200.0, 0.0, 1.0]], "height": 20.0}))?;
    }
    Ok(session)
}

fn sheet_rule(session: &mut Session, thickness_mm: f64) -> Result<()> {
    session.execute(
        "sheet.manage_rules",
        &json!({
            "name": "Budget",
            "thickness": format!("{thickness_mm} mm"),
            "k_factor": "0.5",
            "bend_radius": format!("{} mm", thickness_mm.max(1.0)),
            "relief_width": "2 mm",
            "relief_depth": "1 mm",
            "active": true,
        }),
    )?;
    Ok(())
}

fn base(session: &mut Session, name: &str, x: f64, width: f64, depth: f64) -> Result<()> {
    session.execute("sketch.create", &json!({"plane": "XY", "name": name}))?;
    session.execute("sketch.rectangle.two_point", &json!({"p0": [x, 0.0], "p1": [x + width, depth]}))?;
    session.execute("sketch.finish", &json!({}))?;
    session.execute("sheet.flange", &json!({"sketch": name}))?;
    Ok(())
}

/// Time the native rebuild and the pure DFM facts separately from the service run, over the same
/// saved design. This attributes the run's cost; it is indicative, not a second measurement.
fn out_of_band(model: &Model) -> Result<(f64, f64)> {
    let path = model.session.path.as_deref().ok_or("the project did not save the design")?;
    let bytes = std::fs::read(path)?;
    let start = Instant::now();
    let document = solidcraft_io::read_design(&bytes)?;
    let rebuilt = Session::opened_at(document, None, Some(path));
    let state = rebuilt.world_state()?;
    let rebuild_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    for sheet in &state.sheets {
        let _ = solidcraft_doc::sheet_dfm::facts(sheet)?;
        let _ = solidcraft_doc::sheet_dfm::flat_overlap(sheet)?;
    }
    let facts_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok((rebuild_ms, facts_ms))
}

/// Queue one controlled Review run and wait for its terminal status.
fn timed_run(session: &mut Session, expected_sheets: usize) -> Result<Timed> {
    let now = now_ms();
    let reservation = Reservation {
        run_id: generate_run_id(now),
        operation_id: format!("op_dfm_budget_{}_{}", std::process::id(), now),
        stop_at_ms: now.saturating_add(RESERVATION_GUARD_MS),
    };
    let before = service::status().last_run;
    let start = Instant::now();
    let ticket = service::start_reserved(session, reservation)?;
    if ticket.sheets != expected_sheets {
        return Err(format!("the check ticket saw {} sheets, not {expected_sheets}", ticket.sheets).into());
    }
    let deadline = Instant::now() + RUN_DEADLINE;
    loop {
        let status = service::status();
        if let Some(error) = status.error {
            return Err(format!("the DFM worker failed: {error}").into());
        }
        if !status.running
            && let Some(run) = status.last_run.as_ref()
            && before.as_deref() != Some(run.as_str())
        {
            return Ok(Timed {
                millis: start.elapsed().as_secs_f64() * 1000.0,
                outcome: status.last_outcome.unwrap_or_else(|| "unknown".into()),
                run: run.clone(),
            });
        }
        if Instant::now() >= deadline {
            return Err("the measured run did not finish within the harness deadline".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn median(values: &[f64]) -> Result<f64> {
    if values.is_empty() {
        return Err("no measured passes".into());
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted.get(middle).copied().ok_or_else(|| "median index out of range".into())
    } else {
        let lower = sorted.get(middle.saturating_sub(1)).copied().ok_or("median index out of range")?;
        let upper = sorted.get(middle).copied().ok_or("median index out of range")?;
        Ok((lower + upper) / 2.0)
    }
}

/// The first `model name` in `/proc/cpuinfo`, on hosts that have one.
fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("model name").and_then(|rest| rest.split_once(':')).map(|(_, name)| name.trim().to_string()))
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unknown CPU".into())
}

fn scratch() -> Result<PathBuf> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("intcad-dfm-budget-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
