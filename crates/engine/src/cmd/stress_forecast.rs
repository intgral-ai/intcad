//! What a stress estimate will take, for its approval card (PIL-294 A13): the nodes, the solver's
//! peak memory and the seconds. Nothing is meshed before the user allows the run (decision 4), so
//! the card's first figures come from the part's size ([`before_meshing`]); once a run has meshed
//! the part, it counts the nodes and says again ([`after_meshing`], published as
//! [`fea::Forecast`]).
//!
//! Measured on zhikun (gmsh 4.12.1, CalculiX 2.21 on one thread, an Intel Core Ultra 9 275HX, a
//! laptop processor; 2026-11-20): 33 solid runs of six parts (R2's 200 x 50 x 2 plate with a Ø10
//! hole, a 50 mm cube with a Ø20 hole, a 100 x 100 x 20 plate with a Ø20 hole, a 20 x 20 x 200
//! bar, a 200 x 100 x 1.5 sheet and the 1.5 mm L bracket, both as solids) from 11,000 to 289,000
//! nodes, and 19 shell runs of the sheet and the bracket from 2,800 to 93,000 nodes; the solver's
//! peak memory from `/usr/bin/time` on 21 solid and 8 shell runs. The computer was shared, at a
//! load of 2 to 44 on 24 cores: the same run took up to 35 % longer at the higher loads.

use super::{Elements, MESH_CHECK_RATIO, mesh_check_gate};
use crate::fea::{self, Forecast};

/// Element layers through the part at size `h` mm: its section ([`super::section_size`]: a
/// sheet's or a shell's thickness, twice the volume over the surface area for any other solid,
/// which is a plate's thickness and a round bar's radius) over `h`, at least one. The solver's
/// memory and time grow with it at the same node count: its factorisation fills in more where the
/// mesh is many elements deep. At 95,000 nodes the 50 mm cube (6 layers) solved in 70 to 86 s and
/// needed 3.0 GB; R2's plate (about 2 layers) at 114,000 nodes solved in 13 s.
pub(crate) fn layers(section: f64, h: f64) -> f64 {
    if !(section.is_finite() && section > 0.0 && h.is_finite() && h > 0.0) {
        return 1.0;
    }
    (section / h).max(1.0)
}

/// Solid nodes per mm³ of part at 1 mm elements, and per mm² of its surface: a mesh of `h` mm has
/// about `SOLID_PER_MM3 V / h³ + SOLID_PER_MM2 S / h²` nodes (the surface's own layer of nodes
/// counts for more on a thin part). Fitted on the six parts' 21 runs of the second round without
/// the bracket: within ±11 % from 50,000 nodes up, and up to 48 % low on coarse meshes under
/// 35,000 nodes. Curvature sizing (A2: 24 elements a turn) adds nodes around small holes,
/// fillets and bends that the part's size does not show: the bracket meshed with 3.5 times as many
/// at 1.5 mm and 1.8 times at 1.0 mm.
pub(crate) const SOLID_PER_MM3: f64 = 6.16;
pub(crate) const SOLID_PER_MM2: f64 = 4.01;
/// Shell nodes per mm² of the sheet's mid-surface (half the solid body's surface) at 1 mm
/// elements: within 2 % on the flat sheet from 3 to 1 mm. The bracket's bend and holes made it 2.6
/// times as many at 1.2 mm and 1.3 times at 0.6 mm.
pub(crate) const SHELL_PER_MM2: f64 = 4.64;

/// A run's nodes before meshing, from the part's size: `volume` (mm³, a solid that is not a sheet),
/// `area` (the body's whole surface, mm²) and `thickness` (a sheet's), at `h` mm. `None` when the
/// sizes say nothing.
pub(crate) fn nodes_before_meshing(elements: Elements, h: f64, volume: Option<f64>, area: f64, thickness: f64) -> Option<f64> {
    if !(h.is_finite() && h > 0.0 && area.is_finite() && area > 0.0) {
        return None;
    }
    let n = match elements {
        Elements::Shell => SHELL_PER_MM2 * (area / 2.0) / (h * h),
        Elements::Solid => {
            // A sheet as solids: its mid-surface times its thickness.
            let v = volume.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(area / 2.0 * thickness);
            SOLID_PER_MM3 * v / h.powi(3) + SOLID_PER_MM2 * area / (h * h)
        }
    };
    (n.is_finite() && n > 0.0).then_some(n)
}

/// The solver's peak memory for solids, bytes: `per_node × n × (n / 100,000)^extra × layers^deep`.
/// Within ±14 % of all 21 solid runs measured (0.14 to 5.8 GB). [`super::solver_bytes`], fitted on
/// thin parts only, gave up to 48 % less on the deep ones (the cube at 156,016 nodes: 5.78 GB
/// measured, 2.99 GB from it) and up to 42 % more on thin ones.
pub(crate) const SOLID_BYTES: (f64, f64, f64) = (12_190.0, 0.116, 0.481);

/// The solver's peak memory for a run of `nodes` nodes `layers` elements deep, bytes. Shells keep
/// [`super::SHELL_MEMORY`]: within −7 % and +36 % of the 8 shell runs measured.
pub(crate) fn memory(nodes: f64, elements: Elements, layers: f64) -> f64 {
    let n = nodes.max(0.0);
    match elements {
        Elements::Solid => {
            let (per_node, extra, deep) = SOLID_BYTES;
            per_node * n * (n / 100_000.0).powf(extra) * layers.max(1.0).powf(deep)
        }
        Elements::Shell => super::solver_bytes(n, Elements::Shell),
    }
}

/// Seconds for solids, `a × (n / 100,000)^b × layers^c`: meshing (gmsh, and reading its mesh)
/// and solving (CalculiX, from its job to its results). Together within −42 % and +44 % of the 33
/// solid runs' 1.6 to 266 s; nodes alone, without the layers, were 4.5 times out (the cube took
/// 4.5 times as long as nodes alone said, the thin plate a third).
pub(crate) const SOLID_MESH_SECONDS: (f64, f64, f64) = (4.43, 1.073, 0.645);
pub(crate) const SOLID_SOLVE_SECONDS: (f64, f64, f64) = (9.83, 1.350, 1.047);
/// Seconds for shells, `a × (n / 100,000)^b` (mid-surface nodes): together within −33 % and +30 %
/// of the 19 shell runs' 0.5 to 35 s.
pub(crate) const SHELL_MESH_SECONDS: (f64, f64) = (3.95, 1.064);
pub(crate) const SHELL_SOLVE_SECONDS: (f64, f64) = (23.9, 1.216);

/// Seconds to mesh a run of `nodes` nodes `layers` elements deep, and to solve it.
pub(crate) fn seconds(nodes: f64, elements: Elements, layers: f64) -> (f64, f64) {
    let x = nodes.max(0.0) / 100_000.0;
    match elements {
        Elements::Solid => {
            let deep = layers.max(1.0);
            let ((a, b, c), (d, e, f)) = (SOLID_MESH_SECONDS, SOLID_SOLVE_SECONDS);
            (a * x.powf(b) * deep.powf(c), d * x.powf(e) * deep.powf(f))
        }
        Elements::Shell => {
            let ((a, b), (d, e)) = (SHELL_MESH_SECONDS, SHELL_SOLVE_SECONDS);
            (a * x.powf(b), d * x.powf(e))
        }
    }
}

/// One run an estimate may make.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Planned {
    pub elements: Elements,
    /// Its element size, mm.
    pub h: f64,
    /// The part's section ([`layers`]).
    pub section: f64,
    /// Its nodes, from the part's size.
    pub nodes: f64,
    /// The mesh check's finer run: made only when its gate lets it, on the first run's nodes.
    pub finer: bool,
}

impl Planned {
    pub fn layers(&self) -> f64 {
        layers(self.section, self.h)
    }
}

/// The runs an estimate may make, in order, with their nodes from the part's size: each kind of
/// elements at its size `h`, and after the first, when `mesh_check` is on, the mesh check's finer
/// run at `h` / [`MESH_CHECK_RATIO`]. Empty when the part's size says nothing.
pub(crate) fn planned(runs: &[(Elements, f64, f64)], mesh_check: bool, volume: Option<f64>, area: f64, thickness: f64) -> Vec<Planned> {
    let mut out = Vec::new();
    for (i, &(elements, h, section)) in runs.iter().enumerate() {
        let Some(nodes) = nodes_before_meshing(elements, h, volume, area, thickness) else { return Vec::new() };
        out.push(Planned { elements, h, section, nodes, finer: false });
        if i == 0 && mesh_check {
            let fine_h = h / MESH_CHECK_RATIO;
            let Some(fine) = nodes_before_meshing(elements, fine_h, volume, area, thickness) else { return Vec::new() };
            out.push(Planned { elements, h: fine_h, section, nodes: fine, finer: true });
        }
    }
    out
}

/// Whether the mesh check's finer run, after a first run of `nodes`, fits this computer's limit:
/// the run's own gate ([`mesh_check_gate`]).
pub(crate) fn finer_fits(nodes: f64, elements: Elements) -> bool {
    mesh_check_gate(nodes.max(0.0).round() as usize, elements, super::finer_limit_here()).is_none()
}

/// The runs made, with the finer run when `finer` (its gate's answer) says so.
fn made(runs: &[Planned], finer: bool) -> impl Iterator<Item = (usize, &Planned)> {
    runs.iter().enumerate().filter(move |(_, r)| finer || !r.finer)
}

/// The estimate before meshing: the largest run's nodes, the most memory a run needs, the seconds
/// of every run together, and how many runs; the finer run counts when `fits` lets it on the first
/// run's predicted nodes. `None` without runs.
pub(crate) fn before_meshing(runs: &[Planned], fits: &dyn Fn(f64, Elements) -> bool) -> Option<(f64, f64, f64, usize)> {
    let first = runs.first()?;
    let finer = fits(first.nodes, first.elements);
    let (mut nodes, mut bytes, mut total, mut count) = (0.0f64, 0.0f64, 0.0, 0);
    for (_, r) in made(runs, finer) {
        nodes = nodes.max(r.nodes);
        bytes = bytes.max(memory(r.nodes, r.elements, r.layers()));
        let (m, s) = seconds(r.nodes, r.elements, r.layers());
        total += m + s;
        count += 1;
    }
    Some((nodes, bytes, total, count))
}

/// The estimate once run `k` (from 0, in `runs`) has meshed with `counted` nodes, `finer` saying
/// whether the mesh check's finer run is made: this run's place among those made, its memory, and
/// the seconds left (its solve, then each later run made, meshed and solved, their nodes from the
/// part's size scaled by how far this run's count was from its own).
pub(crate) fn after_meshing(runs: &[Planned], k: usize, counted: usize, finer: bool, document_id: &str) -> Option<Forecast> {
    let run = runs.get(k)?;
    let n = counted as f64;
    let scale = if run.nodes > 0.0 { n / run.nodes } else { 1.0 };
    let (_, solve) = seconds(n, run.elements, run.layers());
    let made: Vec<(usize, &Planned)> = made(runs, finer || run.finer).collect();
    let place = made.iter().position(|(i, _)| *i == k)?;
    let later: f64 = made
        .iter()
        .skip(place + 1)
        .map(|(_, r)| {
            let (m, s) = seconds(r.nodes * scale, r.elements, r.layers());
            m + s
        })
        .sum();
    Some(Forecast {
        document_id: document_id.to_string(),
        seq: 0,
        run: place + 1,
        runs: made.len(),
        elements: run.elements,
        nodes: counted,
        bytes: memory(n, run.elements, run.layers()),
        seconds_left: solve + later,
    })
}

/// Which planned run a solve at `h` mm with `elements` is, from 0.
pub(crate) fn which(runs: &[Planned], elements: Elements, h: f64) -> Option<usize> {
    runs.iter().position(|r| r.elements == elements && (r.h - h).abs() <= 1e-9 * h.abs().max(1.0))
}

/// Follows an estimate's runs as each counts its nodes after meshing, and publishes the forecast
/// ([`fea::set_forecast`]). The first run's count decides the mesh check's finer run with the same
/// gate the run uses (`fits`), so the card counts the runs that are made.
pub(crate) struct Tracker<'a> {
    pub runs: &'a [Planned],
    pub document_id: String,
    pub fits: &'a dyn Fn(f64, Elements) -> bool,
    finer: std::cell::Cell<Option<bool>>,
}

impl<'a> Tracker<'a> {
    pub fn new(runs: &'a [Planned], document_id: String, fits: &'a dyn Fn(f64, Elements) -> bool) -> Self {
        Tracker { runs, document_id, fits, finer: std::cell::Cell::new(None) }
    }

    /// A run of `elements` at `h` mm has meshed with `nodes` nodes.
    pub fn counted(&self, elements: Elements, h: f64, nodes: usize) {
        let Some(k) = which(self.runs, elements, h) else { return };
        if k == 0 {
            self.finer.set(Some((self.fits)(nodes as f64, elements)));
        }
        let finer = self.finer.get().unwrap_or_else(|| self.runs.first().is_some_and(|r| (self.fits)(r.nodes, r.elements)));
        if let Some(f) = after_meshing(self.runs, k, nodes, finer, &self.document_id) {
            fea::set_forecast(f);
        }
    }
}
