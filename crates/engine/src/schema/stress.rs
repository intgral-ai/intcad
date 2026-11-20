//! Contracts and result types for the stress estimate, the commands in `cmd/stress.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A face of the body: its index, its name as `model.faces` lists it, its sheet name, or a point
/// `[x, y, z]` on it (mm). One half of a hole takes the whole hole.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FaceRef {
    Index(u64),
    Name(String),
    Point([f64; 3]),
}

/// One face or several.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FaceRefs {
    One(FaceRef),
    Many(Vec<FaceRef>),
}

/// A pressure: a number in MPa, or text with its unit ("10 MPa", "250 kPa", "2 bar", "10 N/mm²").
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PressureValue {
    Mpa(f64),
    WithUnit(String),
}

/// A load on faces of the body: a force, or a pressure.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct LoadParam {
    /// The face or faces the load is on.
    pub face: FaceRefs,
    /// `force` (the default) or `pressure` (a load given `pressure` is a pressure anyway).
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// A pressure, MPa: a number, or text with its unit. Positive pushes into the part along each
    /// point's inward normal (the usual convention); negative pulls out. Each surface triangle
    /// takes the pressure times its area, shared over its nodes as a force over an area is.
    #[serde(default)]
    pub pressure: Option<PressureValue>,
    /// The total force, `[x, y, z]` in N. Give this, or `direction` and `magnitude`.
    #[serde(default)]
    pub force: Option<[f64; 3]>,
    /// The force's direction, `[x, y, z]` (normalised).
    #[serde(default)]
    pub direction: Option<[f64; 3]>,
    /// The force's size, in N, along `direction`.
    #[serde(default)]
    pub magnitude: Option<f64>,
}

/// Faces held in place.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SupportParam {
    /// The face or faces held.
    pub face: FaceRefs,
    /// How they are held: `fixed` (no translation; a sheet as shells keeps its rotations too), or
    /// `roller` (also `symmetry`, `slider`): a flat face facing along X, Y or Z held only along
    /// that axis, free to slide in its own plane; another face is refused. Default: fixed.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

/// The material: a grade by name, whose numbers and their source come from the estimate's own
/// table (`6061-T6`, `6082-T6`, `5052-H32` aluminium; `S235`, `S355` structural steel; `304`
/// and `304L` stainless; each at its standard's minimum yield), or any material with all its numbers and
/// where they come from.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MaterialParam {
    /// A grade by name, such as `6061-T6` or `S235`.
    Grade(String),
    /// `{name}` alone for a grade of the table, or a material with its own numbers.
    Given(GivenMaterial),
}

/// A material by name, or with its own numbers: then all four of them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GivenMaterial {
    /// Its name and temper, such as `5052-H32`.
    pub name: String,
    /// Young's modulus, MPa (100 … 1 000 000).
    #[serde(default)]
    pub youngs_modulus: Option<f64>,
    /// Poisson's ratio (0 … 0.5).
    #[serde(default)]
    pub poisson_ratio: Option<f64>,
    /// Yield strength, MPa: the estimate says how far from it the stress is.
    #[serde(default)]
    pub yield_strength: Option<f64>,
    /// Where the numbers come from (a datasheet, a handbook and its edition).
    #[serde(default)]
    pub source: Option<String>,
}

/// Estimate the stress and deflection of a body under loads on its faces: the body is meshed
/// and solved by gmsh and CalculiX, run as separate programs on this computer (the part is not
/// uploaded). Shells on the mid-surface suit 1–2 mm sheet metal; solids suit other parts; `both`
/// runs the two and compares them. The first model is solved again with a finer mesh to say
/// whether its numbers settle, the reaction totals are checked against the loads, and the
/// highest stress away from supports and loads is reported beside the highest overall. The
/// result is painted on the body with a legend, the loads,
/// supports and the highest value. It is an estimate: report it as the reply's `how_to_report`
/// says.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct StressEstimateParams {
    /// The body, by name. Default: the only body.
    #[serde(default)]
    pub body: Option<String>,
    /// The forces. At least one.
    #[serde(default)]
    pub loads: Option<Vec<LoadParam>>,
    /// The faces held. At least one: without a support the part would just move.
    #[serde(default)]
    pub supports: Option<Vec<SupportParam>>,
    /// The material: a grade by name (`6061-T6`), or its numbers with their source. Needed: left
    /// out, the error names the grades of the body's assigned material.
    #[serde(default)]
    pub material: Option<MaterialParam>,
    /// The sheet's thickness, mm. Default: the sheet metal rule the body was made with.
    #[serde(default)]
    pub thickness: Option<f64>,
    /// The largest element size, mm; curved faces get smaller elements (24 per turn, down to a
    /// fiftieth of this). Default: about a thirtieth of the body for shells; for solids the
    /// thickness of sheet metal, otherwise about a 25th of the body and half its thinnest side,
    /// but no finer than a mesh of about 80 000 nodes.
    #[serde(default)]
    pub mesh_size: Option<f64>,
    /// `shell`, `solid`, or `both` (the shell result is shown). Default: shell for sheet metal,
    /// solid for any other body.
    #[serde(default)]
    pub elements: Option<String>,
    /// What the colours show: `stress` (von Mises, the default) or `deflection`.
    #[serde(default)]
    pub show: Option<String>,
    /// What the stress colours run between: `yield` (zero to the yield strength given, the
    /// default, so runs compare) or `auto` (zero to this run's highest stress).
    #[serde(default)]
    pub scale: Option<String>,
    /// Solve the first model again with a mesh 1.5 times finer and say, for the highest stress,
    /// the highest stress away from supports and loads and the largest deflection, whether each
    /// settled (moved 2.5 % or less) or is still changing, and how far off it may still be; the
    /// finer result is shown. Skipped when the finer mesh would need more memory than this computer can spare (the smaller of 6 GB
    /// and half the memory available; `SOLIDCRAFT_FEA_MAX_NODES` caps its nodes instead).
    /// Default: true.
    #[serde(default)]
    pub mesh_check: Option<bool>,
    /// Keep the mesher's and solver's files (default: they are removed when the run ends).
    #[serde(default)]
    pub keep_files: Option<bool>,
    /// Check the setup and describe it, and solve nothing (what the approval card shows).
    #[serde(default)]
    pub dry_run: Option<bool>,
    /// Points to read the stress and strain at (at most 50): each gives the stress tensor, its
    /// principal values and directions, the elastic strain and the displacement there, read inside
    /// the solver's element that holds the point, and how the value moved in the mesh check.
    #[serde(default)]
    pub probes: Option<Vec<ProbeParam>>,
}

/// A point to read the stress and strain at.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ProbeParam {
    /// The point `[x, y, z]`, mm: inside the part or on its surface (a point up to 1 mm outside is
    /// read at the part's nearest point). On sheet metal solved as shells, a point on the top or
    /// bottom face reads that face.
    pub at: [f64; 3],
    /// A direction `[x, y, z]` (any length) to also give the stress and strain along, as a strain
    /// gauge laid that way reads them.
    #[serde(default)]
    pub direction: Option<[f64; 3]>,
    /// A name for it in the reply (`fillet`, `gauge 1`). Default: `probe 1`, `probe 2`, ...
    #[serde(default)]
    pub name: Option<String>,
}

/// One solve.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressRun {
    /// `shell` or `solid`.
    pub elements: String,
    /// The element type, in words.
    pub element_type: String,
    /// The largest element size, mm.
    pub mesh_size_mm: f64,
    pub nodes: usize,
    pub element_count: usize,
    /// The highest von Mises stress, MPa (at the solver's nodes).
    pub max_von_mises_mpa: f64,
    /// The face nearest it.
    pub max_stress_near: String,
    /// The largest displacement, mm.
    pub max_deflection_mm: f64,
    /// The face nearest it.
    pub max_deflection_near: String,
    /// How long meshing and solving took, s.
    pub seconds: f64,
    /// The solver's threads (OpenMP; its equation solver runs on one, where it is reliable).
    pub solver_threads: usize,
    /// This is the mesh check's finer solve (the one shown).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mesh_check_run: bool,
    /// The fixed support or load the highest stress sits at (within 1.5 elements of its nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_stress_at_support: Option<String>,
    /// The highest von Mises stress away from every fixed support and load, MPa.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_von_mises_away_mpa: Option<f64>,
    /// The face nearest it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_away_near: Option<String>,
    /// The total reaction on every held node, N (the solver's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_carry_n: Option<f64>,
    /// The total of the loads applied, N.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loads_applied_n: Option<f64>,
}

/// The highest stress away from every fixed support and load.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressPeak {
    pub von_mises_mpa: f64,
    /// The face nearest it.
    pub near: String,
    /// Where it is (the solver's node), mm.
    pub at_mm: [f64; 3],
    /// As a fraction of the yield strength given.
    pub fraction_of_yield: f64,
    /// Looked for at nodes farther than this from every fixed support's and force's node (not a
    /// roller's or a pressure's), mm: 1.5 elements of the first mesh, and at least twice the
    /// part's thickness (a shell's or sheet's; for other solids twice the volume over the surface
    /// area), over which a clamp's or a force's local stress fades (Saint-Venant).
    pub away_from_supports_mm: f64,
    /// The principal stresses there, MPa, largest first: positive is tension, negative
    /// compression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_stress_mpa: Option<[f64; 3]>,
    /// The principal elastic strains there, microstrain (µε, 1e-6), largest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_strain_microstrain: Option<[f64; 3]>,
}

/// A stress tensor's components, MPa, in the design's X, Y and Z.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressComponents {
    pub xx: f64,
    pub yy: f64,
    pub zz: f64,
    pub xy: f64,
    pub yz: f64,
    pub zx: f64,
}

/// An elastic strain's components, microstrain (µε, 1e-6), in the design's X, Y and Z. The shears
/// are engineering shear strains (γ, twice the tensor component), as strain gauges and most
/// finite-element programs report them.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StrainComponents {
    pub xx: f64,
    pub yy: f64,
    pub zz: f64,
    pub gamma_xy: f64,
    pub gamma_yz: f64,
    pub gamma_zx: f64,
}

/// The stress and strain along a direction, as a strain gauge laid that way reads them.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressAlong {
    /// The direction, as a unit vector.
    pub direction: [f64; 3],
    /// The normal stress along it, MPa.
    pub stress_mpa: f64,
    /// The normal strain along it, microstrain.
    pub strain_microstrain: f64,
}

/// A probe's von Mises stress in one solve.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressProbeRun {
    /// `shell` or `solid`.
    pub elements: String,
    pub mesh_size_mm: f64,
    /// The mesh check's finer solve.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mesh_check_run: bool,
    pub von_mises_mpa: f64,
}

/// The stress and strain at a probe's point (shown run), read inside the solver's element that
/// holds it with the element's shape functions.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressProbe {
    /// Its name: the one given, or `probe 1`, `probe 2`, ...
    pub name: String,
    /// The point given, mm.
    pub at_mm: [f64; 3],
    /// How far the point was outside the part, mm, when it was read at the part's nearest point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside_part_mm: Option<f64>,
    /// The face nearest it.
    pub near: String,
    pub von_mises_mpa: f64,
    /// Von Mises as a share of the yield strength given.
    pub fraction_of_yield: f64,
    /// The stress tensor, MPa.
    pub stress_mpa: StressComponents,
    /// The principal stresses, MPa, largest first: positive is tension, negative compression.
    pub principal_stress_mpa: [f64; 3],
    /// Their directions (unit vectors), in the same order.
    pub principal_directions: [[f64; 3]; 3],
    /// The elastic strain, microstrain.
    pub strain_microstrain: StrainComponents,
    /// The principal strains, microstrain, largest first (along the principal stresses'
    /// directions, in a material that is the same in every direction).
    pub principal_strain_microstrain: [f64; 3],
    /// Along the direction given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub along: Option<StressAlong>,
    /// The displacement, mm.
    pub displacement_mm: [f64; 3],
    /// Its von Mises stress on the first mesh and the finer one, when the mesh check ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_check: Option<StressSettling>,
    /// Its von Mises stress in every solve.
    pub runs: Vec<StressProbeRun>,
    /// The probe in one line, for the user.
    pub line: String,
}

/// What the supports carry against the loads applied: they should cancel.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressEquilibrium {
    /// The size of the total reaction on the held nodes, N.
    pub supports_carry_n: f64,
    /// The size of the total load applied, N.
    pub loads_applied_n: f64,
    /// How far they are from cancelling, % of the loads (over 1 % makes the result suspect).
    pub mismatch_percent: f64,
    /// How far the supports' moment and the loads' moment, both about `moments_about_mm`, are from
    /// cancelling, % of the moments that make them up (over 1 % makes the result suspect). Absent
    /// when the solver gave no node's force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moment_mismatch_percent: Option<f64>,
    /// The point the moment balance is taken about, mm: the middle of the part's bounding box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moments_about_mm: Option<[f64; 3]>,
    /// What each support carries, in the order the supports were given.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supports: Vec<StressSupportReaction>,
    /// "Supports carry 10.0 N; loads applied 10.0 N; their moments balance too".
    pub line: String,
}

/// What one support carries: the solver's reaction on the nodes it holds (a node two supports
/// hold counts for the first). On a clamp or a bolted face, what it must hold.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressSupportReaction {
    /// The support, as the setup names it.
    pub support: String,
    /// The force on the part, N (x, y, z): the opposite pushes on whatever holds it.
    pub force_n: [f64; 3],
    /// Its size, N.
    pub force_size_n: f64,
    /// Its moment about `about_mm`, N·mm (x, y, z): what the support resists turning by.
    pub moment_nmm: [f64; 3],
    /// Its size, N·mm.
    pub moment_size_nmm: f64,
    /// The middle of the nodes it holds, mm.
    pub about_mm: [f64; 3],
    /// The support in one line, for the user.
    pub line: String,
}

/// The programs that meshed and solved, as they named themselves; each ran on one thread, so the
/// same request gives the same numbers with the same versions.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressPrograms {
    /// "gmsh 4.12.1".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesher: Option<String>,
    /// "CalculiX 2.21".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solver: Option<String>,
}

/// One number on the first mesh and the finer one.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressSettling {
    /// `highest stress`, `highest stress away from supports and loads` or `largest deflection`.
    pub quantity: String,
    /// `MPa` or `mm`.
    pub unit: String,
    pub first: f64,
    pub finer: f64,
    /// (finer − first) / first, %.
    pub change_percent: f64,
    /// `settled` (moved 2.5 % or less), `still changing`, or, at a support or load, that it
    /// depends on the mesh.
    pub status: String,
    /// How far off the finer value may still be, %: twice its change (one 1.5× step leaves up to
    /// that for the slowest a smooth stress converges). Absent at a support or load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub may_be_off_percent: Option<f64>,
}

/// Whether the numbers settle when the mesh is refined.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressMeshCheck {
    /// `ran`, `skipped` (the finer mesh would need more memory than this computer's limit) or
    /// `off` (mesh_check: false).
    pub status: String,
    /// Why it was skipped (the finer mesh's predicted nodes and memory against the limit) or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_mesh_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finer_mesh_mm: Option<f64>,
    pub quantities: Vec<StressSettling>,
    /// The mesh check in one line.
    pub line: String,
}

/// What a run of this setup is predicted to take, from the part's size before anything is meshed
/// (a dry run starts no program). The run counts its nodes after meshing and refuses a mesh this
/// computer cannot solve.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressPredicted {
    /// The largest run's nodes (the mid-surface's for shells).
    pub nodes: u64,
    /// The most memory one solve needs, GB.
    pub memory_gb: f64,
    /// Every run meshed and solved, seconds (measured on one computer; others differ).
    pub seconds: f64,
    /// How many runs: each kind of elements, and the mesh check's finer run.
    pub runs: usize,
    /// The same in words.
    pub line: String,
}

/// What `inspect.stress_estimate` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressEstimateResult {
    /// Always true: these are estimates, not checks.
    pub estimate: bool,
    pub body: String,
    /// The setup in plain words: loads, supports, material, model.
    pub setup: Vec<String>,
    /// One or two sentences for the user.
    pub summary: String,
    /// The solves, the shown one first (the mesh check's finer one when it ran).
    pub runs: Vec<StressRun>,
    /// The share of the yield strength given (0.5 is half of it) of the number the summary leads
    /// with: the highest stress away from supports and loads when the highest overall sits at one
    /// (`peak_at_support`), otherwise the shown run's highest stress. How far from yield, in one
    /// number.
    pub fraction_of_yield: f64,
    /// The shown run's highest stress overall as a share of the yield strength given, including a
    /// peak at an idealised support or load that depends on the mesh. Equal to
    /// `fraction_of_yield` when no peak sits at a support.
    pub fraction_of_yield_overall: f64,
    /// The shell and solid results side by side, with `elements: both`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_vs_solid: Option<String>,
    /// The fixed support or load the highest stress sits at, if it does: lead with `peak_away`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_at_support: Option<String>,
    /// The highest stress away from every fixed support and load (shown run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_away: Option<StressPeak>,
    /// What the supports carry against the loads applied (shown run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equilibrium: Option<StressEquilibrium>,
    /// Whether the numbers settle with a finer mesh. Absent on a dry run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_check: Option<StressMeshCheck>,
    /// The stress and strain at each point of `probes` that could be read (a point that could not
    /// is a warning).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<StressProbe>,
    /// Where the estimate says little (above yield, large deflection, a thick part as shells):
    /// tell the user before the numbers.
    pub warnings: Vec<String>,
    /// Every assumption the numbers rest on.
    pub assumptions: Vec<String>,
    /// How to report this estimate to the user. Follow it.
    pub how_to_report: Vec<String>,
    /// Where it ran.
    pub network: String,
    /// The mesher's and solver's versions. Absent on a dry run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub programs: Option<StressPrograms>,
    /// On a dry run: the nodes, memory and time the run is predicted to take, from the part's size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted: Option<StressPredicted>,
    /// The folder with the mesh, the solver's job and its results, with `keep_files`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// Show the stress estimate's stress or its deflection, hide it (to show again), or clear it.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct StressShowParams {
    /// `stress`, `deflection`, `hide` (keep it, draw nothing; `off` is the same) or `clear` (drop it).
    #[serde(default)]
    pub show: Option<String>,
    /// What the stress colours run between: `yield` (zero to the yield strength given) or `auto`
    /// (zero to the highest stress).
    #[serde(default)]
    pub scale: Option<String>,
}

/// What `inspect.stress_estimate.show` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StressShowResult {
    /// What is shown now: `stress`, `deflection`, `hide` or `clear`.
    pub shown: String,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}
