//! A stress estimate of a part (PIL-294, the first-round demo): the body goes out as STEP to a
//! mesher (gmsh) and the mesh to a finite-element solver (CalculiX), both separate programs on
//! this computer, run in a folder of their own; nothing is linked, nothing leaves the machine.
//! The results come back as a field on the body's display triangles, which the viewport paints.
//!
//! The numbers are estimates: a mesh, a linear material and idealised loads and supports. They
//! never pass or fail anything; verdicts come only from exact deterministic checks.

pub mod formats;
pub mod materials;
pub mod model;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use formats::{InpElements, InpMaterial, P};
use model::{Faces, Grid};

/// A material as the user entered it, with where its numbers come from.
#[derive(Clone, Debug)]
pub struct Material {
    pub name: String,
    /// Young's modulus, MPa.
    pub e_mpa: f64,
    pub nu: f64,
    /// Yield strength, MPa: only to say how far the estimate is from it.
    pub yield_mpa: f64,
    pub source: String,
}

/// How the part is modelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elements {
    /// Six-node shells on the sheet's mid-surface.
    Shell,
    /// Ten-node tetrahedra through the whole solid.
    Solid,
}

impl Elements {
    pub fn word(self) -> &'static str {
        match self {
            Elements::Shell => "shell",
            Elements::Solid => "solid",
        }
    }
    /// The model in plain words, for the legend and the summary.
    pub fn plain(self, t: f64) -> String {
        match self {
            Elements::Shell => format!("thin shells on the sheet's mid-surface, {t} mm thick"),
            Elements::Solid => "solid elements through the part".into(),
        }
    }
    pub fn described(self, t: f64) -> String {
        match self {
            Elements::Shell => format!("six-node shells (CalculiX S6) on the mid-surface, {t} mm thick"),
            Elements::Solid => "ten-node tetrahedra (CalculiX C3D10) through the solid".into(),
        }
    }
}

/// What a load on faces is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoadKind {
    /// A total force, N, spread over the faces' area.
    Force(P),
    /// A pressure, MPa, along each point's inward normal: positive pushes into the part.
    Pressure(f64),
}

/// A load on faces.
#[derive(Clone, Debug)]
pub struct Load {
    pub label: String,
    pub faces: Vec<u32>,
    pub kind: LoadKind,
}

/// How a support holds its faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportKind {
    /// No translation (a held shell keeps its rotations too: a clamp).
    Fixed,
    /// Only the translation along this axis (0 x, 1 y, 2 z) is held: a flat face sliding on a
    /// plane.
    Roller(usize),
    /// A symmetry plane across this axis: the translation along it is held, and a shell on it
    /// also keeps from tilting out of it (its turns about the plane's two other axes are held:
    /// without them a half strip on a symmetry edge bent 1.2 % more than the whole strip; with
    /// them it matched to 0.01 %). A solid's nodes have no turns, so on solids it is a roller.
    Symmetry(usize),
}

/// Faces held.
#[derive(Clone, Debug)]
pub struct Support {
    pub label: String,
    pub faces: Vec<u32>,
    pub kind: SupportKind,
}

/// One analysis as the command set it up.
pub struct Setup {
    pub thickness: f64,
    pub mesh_size: f64,
    pub material: Material,
    pub loads: Vec<Load>,
    pub supports: Vec<Support>,
    /// How far from a fixed support's or a load's nodes the peak "away" from them is looked for,
    /// mm ([`peaks`]). The same for every run of an estimate, so the mesh check compares like
    /// with like.
    pub away_distance: f64,
}

/// The support or load the highest stress sits at, and the highest stress away from them all.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Peaks {
    /// The label of the fixed support or load with a node within `distance` of the highest
    /// stress (the nearest such node's), if any.
    pub at: Option<String>,
    /// The highest stress (MPa) at a node farther than `distance` from every fixed support's and
    /// load's node, and that node. `None` when every node is that close.
    pub away: Option<(f64, P)>,
    /// mm.
    pub distance: f64,
}

/// Where the highest stress is relative to the idealised supports and loads, and the highest
/// stress away from them. `points` are the solver's nodes (position, von Mises, displacement);
/// `anchors` the nodes of fixed supports and loads with an index into `labels`. A clamp or a
/// load on a face edge concentrates stress without limit as the mesh is refined, but only in the
/// elements that touch it: a node farther than `distance` from every anchor shares no element
/// with one (see `cmd/stress.rs` for the choice of 1.5 element sizes).
pub fn peaks(points: &[(P, f64, P)], anchors: &[(P, usize)], labels: &[String], distance: f64) -> Peaks {
    let distance = if distance.is_finite() && distance > 0.0 { distance } else { 0.0 };
    let grid = Grid::new(anchors.iter().map(|a| a.0).collect(), distance.max(1e-6));
    // The nearest anchor within `distance` of `p`: a node that close lies in this cell or the next.
    let near = |p: P| -> Option<usize> {
        let i = grid.nearest(p, 1)?;
        let (q, label) = anchors.get(i)?;
        (model::len(model::sub(*q, p)) <= distance).then_some(*label)
    };
    let mut out = Peaks { at: None, away: None, distance };
    let Some(top) = points.iter().filter(|x| x.1.is_finite()).max_by(|a, b| a.1.total_cmp(&b.1)) else { return out };
    out.at = near(top.0).and_then(|l| labels.get(l).cloned());
    for (p, s, _) in points {
        if !s.is_finite() || out.away.is_some_and(|(best, _)| *s <= best) {
            continue;
        }
        if near(*p).is_none() {
            out.away = Some((*s, *p));
        }
    }
    out
}

/// Whether the forces balance: the solver's total reaction on the held nodes against the loads
/// applied. They should cancel; when they do not, the supports or loads did not reach the model
/// as set up, and the result is suspect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Equilibrium {
    /// The total reaction force on every held node, N: the solver's total there (its RF at a
    /// node is the node's internal force) less the loads applied on held nodes themselves.
    pub reaction: P,
    /// The sum of the nodal forces applied, N.
    pub applied: P,
    /// The sum of the nodal forces' sizes, N: the scale when the loads cancel each other.
    pub scale: f64,
}

/// The most the reaction may miss the applied loads by, as a fraction of them, before the result
/// is called suspect.
pub const EQUILIBRIUM_TOLERANCE: f64 = 0.01;

impl Equilibrium {
    /// What the mismatch is measured against, N: the applied total, or a thousandth of the sum of
    /// the forces' sizes when the loads (nearly) cancel each other.
    pub fn reference(&self) -> f64 {
        model::len(self.applied).max(1e-3 * self.scale).max(1e-12)
    }
    /// How far the reaction misses the applied total, as a fraction of it (0.01 is 1 %).
    pub fn mismatch(&self) -> f64 {
        model::len(model::add(self.reaction, self.applied)) / self.reference()
    }
    /// The forces balance within [`EQUILIBRIUM_TOLERANCE`].
    pub fn balances(&self) -> bool {
        self.mismatch() <= EQUILIBRIUM_TOLERANCE
    }
}

/// One solve's outcome.
pub struct Run {
    pub elements: Elements,
    pub mesh_size: f64,
    pub nodes: usize,
    pub element_count: usize,
    pub max_stress: f64,
    pub max_stress_at: P,
    pub max_disp: f64,
    pub max_disp_at: P,
    pub seconds: f64,
    /// The solver's nodes with their von Mises stress (MPa) and displacement (mm).
    pub points: Vec<(P, f64, P)>,
    /// The solver ran with networking switched off (a network namespace of its own).
    pub isolated: bool,
    /// The reaction against the loads, when the solver printed its reaction totals.
    pub equilibrium: Option<Equilibrium>,
    /// The highest stress's place against the supports and loads, and the peak away from them.
    pub peaks: Peaks,
    /// The solver's threads (`OMP_NUM_THREADS`; see [`solver_threads`]).
    pub threads: usize,
}

/// What the result shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantity {
    Stress,
    Deflection,
}

/// What the stress colours run between: zero to the yield strength given (the default, so runs
/// compare), or zero to this run's highest stress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    Yield,
    Auto,
}

/// A thing drawn on the result: a load (with its direction), a support or the highest value.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub at: P,
    /// A load's direction (unit), pointing the way the force pushes.
    pub dir: Option<P>,
    pub text: String,
}

/// A result painted on a body: its display triangles (refined), a value at each corner.
#[derive(Clone)]
pub struct Field {
    pub document_id: String,
    pub body: String,
    /// The body's shape when it was solved ([`fingerprint`]): another shape means the result is
    /// out of date.
    pub fingerprint: u64,
    pub positions: Vec<P>,
    pub normals: Vec<P>,
    pub triangles: Vec<[u32; 3]>,
    /// Von Mises stress, MPa.
    pub stress: Vec<f32>,
    /// Displacement, mm.
    pub disp: Vec<P>,
    pub max_stress: f64,
    pub max_stress_at: P,
    pub max_disp: f64,
    pub max_disp_at: P,
    /// The yield strength given, MPa.
    pub yield_mpa: f64,
    pub scale: ScaleMode,
    pub quantity: Quantity,
    /// Kept but not drawn (Hide); Show draws it again.
    pub hidden: bool,
    /// How much the shape is exaggerated when deflection is shown.
    pub deform_scale: f64,
    /// The heading: what was run.
    pub heading: String,
    /// Every assumption, one a line.
    pub assumptions: Vec<String>,
    /// Where the estimate says little (past yield, large deflection, a thick part as shells).
    pub warnings: Vec<String>,
    pub loads: Vec<Marker>,
    pub supports: Vec<Marker>,
    /// The highest stress away from the fixed supports and loads (MPa) and where, when the
    /// highest overall sits at one of them: a second marker.
    pub away_stress: Option<(f64, P)>,
    /// "Supports carry 10.0 N; loads applied 10.0 N", when the solver gave its reactions.
    pub equilibrium: Option<String>,
    /// What the mesh check found (or why it did not run), for the legend's Mesh check line.
    pub mesh_check: Option<String>,
}

/// What an estimate would apply, drawn on the model before anything is solved: a dry run's (the
/// agent's, or the one behind its approval card), so the user sees which faces are held and loaded
/// before saying yes.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub document_id: String,
    pub body: String,
    /// The body's shape when it was checked ([`fingerprint`]): another shape means the preview is
    /// out of date and is not drawn.
    pub fingerprint: u64,
    /// The body's size (its box's diagonal), mm: how long the load arrows are drawn.
    pub size: f64,
    pub loads: Vec<Marker>,
    pub supports: Vec<Marker>,
}

static SHOWN: Mutex<Option<Arc<Field>>> = Mutex::new(None);
static PREVIEW: Mutex<Option<Arc<Preview>>> = Mutex::new(None);
static REVISION: AtomicU64 = AtomicU64::new(1);

/// The setup preview for design `document_id`, if any.
pub fn preview(document_id: &str) -> Option<Arc<Preview>> {
    PREVIEW.lock().ok().and_then(|g| g.clone()).filter(|p| p.document_id == document_id)
}

/// Show `preview` (replacing any other).
pub fn show_preview(preview: Preview) {
    if let Ok(mut g) = PREVIEW.lock() {
        *g = Some(Arc::new(preview));
        REVISION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Drop the setup preview for `document_id`: the estimate was declined, or it runs now.
pub fn clear_preview(document_id: &str) -> bool {
    let Ok(mut g) = PREVIEW.lock() else { return false };
    if g.as_ref().is_some_and(|p| p.document_id == document_id) {
        *g = None;
        REVISION.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    false
}

/// The result shown for design `document_id`, if any.
pub fn shown(document_id: &str) -> Option<Arc<Field>> {
    SHOWN.lock().ok().and_then(|g| g.clone()).filter(|f| f.document_id == document_id)
}

/// Bumped whenever what is shown changes, for the viewport's caches.
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

/// Show `field` (replacing what was shown).
pub fn show(field: Field) {
    if let Ok(mut g) = SHOWN.lock() {
        *g = Some(Arc::new(field));
        REVISION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Change the result shown for `document_id`. `false` when there is none.
pub fn update(document_id: &str, change: impl FnOnce(&mut Field)) -> bool {
    let Ok(mut g) = SHOWN.lock() else { return false };
    let Some(f) = g.as_ref().filter(|f| f.document_id == document_id) else { return false };
    let mut next = Field::clone(f);
    change(&mut next);
    *g = Some(Arc::new(next));
    REVISION.fetch_add(1, Ordering::Relaxed);
    true
}

/// Show stress or deflection on the result for `document_id` (and draw it if it was hidden).
/// `false` when there is none.
pub fn set_quantity(document_id: &str, q: Quantity) -> bool {
    update(document_id, |f| {
        f.quantity = q;
        f.hidden = false;
    })
}

/// Drop the result for `document_id`. `false` when there was none.
pub fn clear(document_id: &str) -> bool {
    let Ok(mut g) = SHOWN.lock() else { return false };
    if g.as_ref().is_some_and(|f| f.document_id == document_id) {
        *g = None;
        REVISION.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    false
}

/// A body's shape as far as a result cares: its faces, their total area and its exact extent. A
/// hole, a new thickness (even one that grows inward and keeps the outside size) or a moved face
/// changes it; showing, hiding or recolouring does not.
pub fn fingerprint(body: &solidcraft_doc::ModelBody) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.name.hash(&mut h);
    body.body.face_count().hash(&mut h);
    if let Ok(b) = body.body.exact_bounds() {
        for v in [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z] {
            // To a micrometre: the same shape evaluated again lands on the same value.
            ((v * 1000.0).round() as i64).hash(&mut h);
        }
    }
    let tol = (body.body.size() * 2e-3).max(1e-3);
    if let Ok(faces) = body.body.faces(tol) {
        let area: f64 = faces.iter().map(|f| f.area).sum();
        // To a hundredth of a square millimetre.
        ((area * 100.0).round() as i64).hash(&mut h);
    }
    h.finish()
}

/// What a running estimate is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Meshing,
    Solving,
}

/// A running estimate: what it does now, for the status line.
#[derive(Clone, Debug)]
pub struct Progress {
    pub document_id: String,
    pub stage: Stage,
    pub elements: Elements,
    /// This run (1-based) of `runs`.
    pub run: usize,
    pub runs: usize,
    /// Seconds since the estimate started, when the stage began.
    pub since: f64,
}

static PROGRESS: Mutex<Option<Progress>> = Mutex::new(None);
static CANCEL: AtomicU64 = AtomicU64::new(0);

/// The estimate running now, if any.
pub fn progress() -> Option<Progress> {
    PROGRESS.lock().ok().and_then(|g| g.clone())
}

pub fn set_progress(p: Option<Progress>) {
    if let Ok(mut g) = PROGRESS.lock() {
        *g = p;
    }
}

/// Stop the estimate running now: its mesher or solver is killed at once and the command answers
/// that it was stopped.
pub fn cancel() {
    CANCEL.fetch_add(1, Ordering::Relaxed);
}

/// The stop counter when an estimate starts; [`cancel`] moves it on.
pub fn cancel_generation() -> u64 {
    CANCEL.load(Ordering::Relaxed)
}

/// The error a stopped estimate ends with.
pub const STOPPED: &str = "stopped: the stress estimate was stopped; nothing was shown";

/// The programs: `SOLIDCRAFT_GMSH` and `SOLIDCRAFT_CCX`, or `gmsh` and `ccx` on the PATH.
pub fn programs() -> (String, String) {
    let var = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| d.to_string());
    (var("SOLIDCRAFT_GMSH", "gmsh"), var("SOLIDCRAFT_CCX", "ccx"))
}

/// A fresh folder for one analysis, under the system's temporary folder.
pub fn work_folder() -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let d = std::env::temp_dir().join(format!("solidcraft-fea-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    Ok(d)
}

/// Whether programs can be started without network access here: Linux with `unshare` and
/// unprivileged user namespaces.
#[cfg(not(target_arch = "wasm32"))]
fn can_isolate() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        cfg!(target_os = "linux")
            && std::process::Command::new("unshare")
                .args(["-r", "-n", "true"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    })
}

/// The solver's thread settings, each only where it is not set already: `OMP_NUM_THREADS` (the
/// element and result loops) to this computer's available parallelism, and
/// `CCX_NPROC_EQUATION_SOLVER` (the SPOOLES equation solver) to 1. On CalculiX 2.21 the
/// multithreaded SPOOLES solve returned a wrong displacement field now and then: the same 14 555
/// node job came back wrong 4 times in 60 with both at 24 threads and 8 in 60 with the solver
/// alone at 24, never in 60 with only OMP_NUM_THREADS at 24 or in 20 on one thread (PIL-294,
/// measured on a 24-core Linux host). The variables to set, and the thread count used.
pub fn solver_threads() -> (Vec<(&'static str, String)>, usize) {
    let available = std::thread::available_parallelism().map_or(1, |n| n.get());
    let set = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let used = set("OMP_NUM_THREADS").and_then(|v| v.trim().parse::<usize>().ok()).filter(|n| *n > 0).unwrap_or(available);
    let env = [("OMP_NUM_THREADS", available), ("CCX_NPROC_EQUATION_SOLVER", 1)]
        .into_iter()
        .filter(|(k, _)| set(k).is_none())
        .map(|(k, n)| (k, n.to_string()))
        .collect();
    (env, used)
}

/// Run `program` with `args` (and `env` set) in `dir`, its output to `<log>.log`, at most
/// `limit` seconds; with networking switched off where this computer allows it. Returns its
/// output and whether it ran isolated.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn run_program(
    program: &str,
    args: &[String],
    env: &[(&str, String)],
    dir: &Path,
    log: &str,
    limit: u64,
    what: &str,
    generation: u64,
) -> Result<(String, bool), String> {
    use std::process::{Command, Stdio};
    let isolated = can_isolate();
    let mut cmd = if isolated {
        let mut c = Command::new("unshare");
        c.args(["-r", "-n", program]);
        c
    } else {
        Command::new(program)
    };
    let log_path = dir.join(format!("{log}.log"));
    let out = std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    cmd.args(args).envs(env.iter().map(|(k, v)| (*k, v.as_str()))).current_dir(dir).stdin(Stdio::null()).stdout(out).stderr(err);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "stress estimates need two free programs on this computer, the mesher gmsh and the solver CalculiX, and `{program}` was \
                 not found. Install them (on Ubuntu or Debian: sudo apt install gmsh calculix-ccx; elsewhere from gmsh.info and \
                 calculix.de), then run the estimate again. They run beside Intcad and are not part of it. If they are installed \
                 somewhere unusual, SOLIDCRAFT_GMSH and SOLIDCRAFT_CCX can name them"
            )
        } else {
            format!("could not start the {what} `{program}`: {e}")
        }
    })?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let text = std::fs::read_to_string(&log_path).unwrap_or_default();
                if !status.success() {
                    let tail: Vec<&str> = text.lines().rev().take(6).collect();
                    let tail: Vec<&str> = tail.into_iter().rev().collect();
                    return Err(format!("the {what} `{program}` stopped with {status}: {}", tail.join(" | ")));
                }
                return Ok((text, isolated));
            }
            Ok(None) if CANCEL.load(Ordering::Relaxed) != generation => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(STOPPED.into());
            }
            Ok(None) if start.elapsed().as_secs() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("the {what} `{program}` took longer than {limit} s and was stopped"));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return Err(format!("the {what} `{program}`: {e}")),
        }
    }
}

/// Where one solve stands in an estimate: for the status line and Stop.
pub struct RunCtx<'a> {
    pub document_id: &'a str,
    pub run: usize,
    pub runs: usize,
    pub generation: u64,
    pub started: f64,
}

/// Mesh `step` (the body, mm) and solve it with `elements`, in `dir`.
#[cfg(target_arch = "wasm32")]
pub fn solve(_setup: &Setup, _step: &[u8], _faces: &Faces, _dir: &Path, _elements: Elements, _ctx: &RunCtx) -> Result<Run, String> {
    Err("the stress estimate needs the desktop app: it runs a mesher and a solver on this computer".into())
}

/// Mesh `step` (the body, mm) and solve it with `elements`, in `dir`.
#[cfg(not(target_arch = "wasm32"))]
pub fn solve(setup: &Setup, step: &[u8], faces: &Faces, dir: &Path, elements: Elements, ctx: &RunCtx) -> Result<Run, String> {
    let started = std::time::Instant::now();
    let stage = |stage: Stage| {
        set_progress(Some(Progress { document_id: ctx.document_id.to_string(), stage, elements, run: ctx.run, runs: ctx.runs, since: ctx.started }))
    };
    stage(Stage::Meshing);
    let (gmsh, ccx) = programs();
    let word = elements.word();
    // Each run of an estimate has files of its own (the mesh check solves the same model again).
    let tag = format!("{word}-{}", ctx.run);
    let step_path = dir.join("part.step");
    std::fs::write(&step_path, step).map_err(|e| format!("{}: {e}", step_path.display()))?;
    let h = setup.mesh_size;
    let msh_name = format!("{tag}.msh");
    let args: Vec<String> = [
        "part.step",
        if elements == Elements::Solid { "-3" } else { "-2" },
        "-order",
        "2",
        "-clmax",
        &format!("{h}"),
        "-format",
        "msh22",
        "-save_all",
        "-o",
        &msh_name,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let (_, mesher_isolated) = run_program(&gmsh, &args, &[], dir, &format!("{tag}-mesher"), 600, "mesher", ctx.generation)?;
    let text = std::fs::read_to_string(dir.join(&msh_name)).map_err(|e| format!("the mesher wrote no mesh ({e})"))?;
    let mut msh = formats::read_msh(&text)?;
    // The mesh in mm, as the body: a STEP reader that converted to metres is scaled back.
    let diag = |pts: &mut dyn Iterator<Item = P>| {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in pts {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        model::len(model::sub(hi, lo))
    };
    let body_diag = diag(&mut faces.positions.iter().copied());
    let mesh_diag = diag(&mut msh.nodes.values().copied());
    let ratio = body_diag / mesh_diag;
    if !(ratio.is_finite() && ratio > 0.0) {
        return Err("the mesh has no extent".into());
    }
    if (ratio / 1000.0 - 1.0).abs() < 0.05 || (ratio * 1000.0 - 1.0).abs() < 0.05 {
        for p in msh.nodes.values_mut() {
            *p = model::mul(*p, ratio);
        }
    } else if (ratio - 1.0).abs() > 0.05 {
        return Err(format!("the mesh is {mesh_diag:.3} across where the body is {body_diag:.3} mm: its units are not mm"));
    }
    let surface_face = model::surface_faces(&msh, faces);
    let fem = match elements {
        Elements::Shell => model::shell_model(&msh, &surface_face, setup.thickness)?,
        Elements::Solid => model::solid_model(&msh, &surface_face)?,
    };
    let mut fixed: Vec<usize> = Vec::new();
    let mut rollers: Vec<(usize, usize)> = Vec::new();
    let mut untilted: Vec<(usize, usize)> = Vec::new();
    // The nodes of fixed supports and loads, each with its label: where stress concentrates
    // without limit as the mesh is refined (a roller on a flat face holds only what the face's
    // own plane does, so it concentrates nothing).
    let mut anchors: Vec<(P, usize)> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for s in &setup.supports {
        let n = model::held(&fem, &s.faces);
        if n.is_empty() {
            return Err(format!("the support on {} holds no node of the {word} mesh", s.label));
        }
        match s.kind {
            SupportKind::Fixed => {
                labels.push(s.label.clone());
                anchors.extend(n.iter().filter_map(|i| fem.nodes.get(*i).map(|p| (*p, labels.len() - 1))));
                fixed.extend(n);
            }
            SupportKind::Roller(axis) => rollers.extend(n.into_iter().map(|i| (i, axis))),
            SupportKind::Symmetry(axis) => {
                untilted.extend(n.iter().map(|i| (*i, axis)));
                rollers.extend(n.into_iter().map(|i| (i, axis)));
            }
        }
    }
    fixed.sort_unstable();
    fixed.dedup();
    rollers.sort_unstable();
    rollers.dedup();
    let mut forces: Vec<(usize, P)> = Vec::new();
    for l in &setup.loads {
        let f = match l.kind {
            LoadKind::Force(force) => {
                model::spread(&fem, &l.faces, force).ok_or_else(|| format!("the load on {} reaches no node of the {word} mesh", l.label))?
            }
            LoadKind::Pressure(p) => model::pressure(&fem, &l.faces, p).ok_or_else(|| match elements {
                Elements::Shell => format!(
                    "the pressure on {} has no area in the shell model (it is an edge of the sheet, which shells have no area for); use solid \
                     elements (`elements: solid`) to press on it",
                    l.label
                ),
                Elements::Solid => format!("the pressure on {} reaches no element face of the {word} mesh", l.label),
            })?,
        };
        // A pressure is not an anchor: it presses evenly on its whole face and gathers at no point
        // or edge, and it often covers most of the part (R4: a strip's whole top), which would
        // leave nothing "away". A force stands in for a bolt or a hand on a small face, so it
        // counts.
        if matches!(l.kind, LoadKind::Force(_)) {
            labels.push(l.label.clone());
            anchors.extend(f.iter().filter(|(_, x)| model::len(*x) > 0.0).filter_map(|(i, _)| fem.nodes.get(*i).map(|p| (*p, labels.len() - 1))));
        }
        forces.extend(f);
    }
    let applied = forces.iter().fold([0.0; 3], |a, (_, f)| model::add(a, *f));
    let scale: f64 = forces.iter().map(|(_, f)| model::len(*f)).sum();
    // The solver's RF at a held node is the node's internal force: where a load is applied on a
    // held node too (a pressured face's rim on a roller face), the support carries RF less it.
    let held_nodes: std::collections::BTreeSet<usize> = fixed.iter().copied().chain(rollers.iter().map(|(n, _)| *n)).collect();
    let on_held = forces.iter().filter(|(n, _)| held_nodes.contains(n)).fold([0.0; 3], |a, (_, f)| model::add(a, *f));
    let material = InpMaterial { e_mpa: setup.material.e_mpa, nu: setup.material.nu };
    untilted.sort_unstable();
    untilted.dedup();
    let holds = formats::InpHolds { fixed: &fixed, rollers: &rollers, untilted: &untilted };
    let (inp, element_count) = match elements {
        Elements::Shell => (
            formats::write_inp(&fem.nodes, &InpElements::Shell { tris: &fem.shells, thickness: setup.thickness }, &material, &holds, &forces),
            fem.shells.len(),
        ),
        Elements::Solid => (formats::write_inp(&fem.nodes, &InpElements::Solid { tets: &fem.tets }, &material, &holds, &forces), fem.tets.len()),
    };
    let job = format!("{tag}-job");
    std::fs::write(dir.join(format!("{job}.inp")), inp).map_err(|e| e.to_string())?;
    stage(Stage::Solving);
    let (env, threads) = solver_threads();
    let (out, solver_isolated) =
        run_program(&ccx, &["-i".to_string(), job.clone()], &env, dir, &format!("{tag}-solver"), 1800, "solver", ctx.generation)?;
    if let Some(line) = out.lines().find(|l| l.contains("*ERROR")) {
        return Err(format!("the solver refused the job: {}", line.trim()));
    }
    let frd = std::fs::read_to_string(dir.join(format!("{job}.frd"))).map_err(|e| format!("the solver wrote no results ({e})"))?;
    let frd = formats::read_frd(&frd)?;
    let mut points = Vec::with_capacity(frd.stress.len());
    for (id, s) in &frd.stress {
        let (Some(p), Some(u)) = (frd.nodes.get(id), frd.disp.get(id)) else { continue };
        points.push((*p, formats::von_mises(s), *u));
    }
    let top = points.iter().max_by(|a, b| a.1.total_cmp(&b.1)).ok_or("the results have no stresses")?;
    let far = points.iter().max_by(|a, b| model::len(a.2).total_cmp(&model::len(b.2))).ok_or("the results have no displacements")?;
    let reaction = std::fs::read_to_string(dir.join(format!("{job}.dat"))).ok().and_then(|t| formats::read_dat_total(&t, formats::HELD_SET));
    let found = peaks(&points, &anchors, &labels, setup.away_distance);
    Ok(Run {
        equilibrium: reaction.map(|rf| Equilibrium { reaction: model::sub(rf, on_held), applied, scale }),
        peaks: found,
        threads,
        elements,
        mesh_size: h,
        nodes: fem.nodes.len(),
        element_count,
        max_stress: top.1,
        max_stress_at: top.0,
        max_disp: model::len(far.2),
        max_disp_at: model::add(far.0, far.2),
        seconds: started.elapsed().as_secs_f64(),
        isolated: mesher_isolated && solver_isolated,
        points,
    })
}

/// The run's results on the body's display triangles, split until no edge is longer than
/// `edge` so the colours follow the solver's nodes.
pub fn field(run: &Run, positions: &[P], normals: &[P], triangles: &[[u32; 3]], edge: f64) -> (Vec<P>, Vec<P>, Vec<[u32; 3]>, Vec<f32>, Vec<P>) {
    let mut pos: Vec<P> = Vec::new();
    let mut nor: Vec<P> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    const MAX_TRIANGLES: usize = 160_000;
    let mut stack: Vec<([P; 3], [P; 3], u32)> = Vec::new();
    for t in triangles {
        let get = |i: u32, v: &[P]| v.get(i as usize).copied().unwrap_or([0.0; 3]);
        stack.push((
            [get(t[0], positions), get(t[1], positions), get(t[2], positions)],
            [get(t[0], normals), get(t[1], normals), get(t[2], normals)],
            0,
        ));
        while let Some((p, n, depth)) = stack.pop() {
            let longest = (0..3).map(|k| model::len(model::sub(p[(k + 1) % 3], p[k]))).fold(0.0, f64::max);
            if longest > edge && depth < 4 && tris.len() + stack.len() * 4 < MAX_TRIANGLES {
                let m = |a: usize, b: usize, v: &[P; 3]| model::mid(v[a], v[b]);
                let (pm, nm) = ([m(0, 1, &p), m(1, 2, &p), m(2, 0, &p)], [m(0, 1, &n), m(1, 2, &n), m(2, 0, &n)]);
                stack.push(([p[0], pm[0], pm[2]], [n[0], nm[0], nm[2]], depth + 1));
                stack.push(([pm[0], p[1], pm[1]], [nm[0], n[1], nm[1]], depth + 1));
                stack.push(([pm[2], pm[1], p[2]], [nm[2], nm[1], n[2]], depth + 1));
                stack.push(([pm[0], pm[1], pm[2]], [nm[0], nm[1], nm[2]], depth + 1));
                continue;
            }
            let base = pos.len() as u32;
            pos.extend(p);
            nor.extend(n.map(|x| model::unit(x).unwrap_or([0.0, 0.0, 1.0])));
            tris.push([base, base + 1, base + 2]);
        }
    }
    let grid = Grid::new(run.points.iter().map(|x| x.0).collect(), run.mesh_size.max(1e-3));
    let mut stress = Vec::with_capacity(pos.len());
    let mut disp = Vec::with_capacity(pos.len());
    for p in &pos {
        match grid.nearest(*p, 6).and_then(|i| run.points.get(i)) {
            Some((_, s, u)) => {
                stress.push(*s as f32);
                disp.push(*u);
            }
            None => {
                stress.push(0.0);
                disp.push([0.0; 3]);
            }
        }
    }
    (pos, nor, tris, stress, disp)
}

#[cfg(test)]
#[path = "fea_tests.rs"]
mod tests;
