//! The stress estimate (PIL-294, first-round demo): loads and supports on faces of a body, a
//! material given with its source, meshed and solved by separate programs on this computer
//! (`crate::fea`), the result painted on the body. Estimates only: the reply and the legend say
//! how far the stress is from the yield strength given; they never judge the part, and the reply
//! tells the agent so in as many words ([`HOW_TO_REPORT`]).

use serde_json::Value;
use solidcraft_io::Format;

use super::CommandSpec;
use crate::fea::{
    self, Elements, Field, Load, LoadKind, Marker, Material, Quantity, RunCtx, ScaleMode, Setup, Support, SupportKind,
    formats::P,
    model::{Faces, Hold},
};
use crate::params::bad;
use crate::schema::stress::{
    StrainComponents, StressAlong, StressComponents, StressEquilibrium, StressEstimateParams, StressEstimateResult, StressMeshCheck, StressPeak,
    StressPredicted, StressProbe, StressProbeRun, StressPrograms, StressRun, StressSettling, StressShowParams, StressShowResult,
    StressSupportReaction,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{EngineError, Result, Session};

const CMD: &str = "inspect.stress_estimate";
const SHOW: &str = "inspect.stress_estimate.show";

const ESTIMATE_CONTRACT: TypedContract<StressEstimateParams> = TypedContract::new();
const SHOW_CONTRACT: TypedContract<StressShowParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new(CMD, "Stress Estimate", crate::checked!("inspect.stress_estimate", estimate))
        .at("SOLID", "INSPECT")
        .icon("curvature_map")
        .noundo()
        .contract(&ESTIMATE_CONTRACT)
        .output(output_schema_for::<StressEstimateResult>)
        .params(
            "loads: [{face, force?: [x,y,z] N, direction?: [x,y,z], magnitude?: N} | {face, type?: pressure, pressure: MPa (a number, or \
             text with its unit: \"10 MPa\", \"250 kPa\", \"2 bar\"; positive pushes into the part along each point's inward normal, negative \
             pulls out)}]; supports: [{face, type?: fixed|roller|symmetry|slider}] (fixed: held still, the default; roller, symmetry, \
             slider: a flat face facing along X, Y or Z held only along that axis, free to slide in its plane); face: a name or index from \
             model.faces, a point [x,y,z] within 1 mm of it, or a list; direction: any length; picking one half of a hole takes the whole hole); material: a grade by name (6061-T6, \
             6082-T6, 5052-H32, S235, S355, 304, 304L: sourced, at the standard's minimum yield) or {name, youngs_modulus: MPa, poisson_ratio, \
             yield_strength: MPa, source}; body?; thickness?: mm (sheet metal: its rule's); mesh_size?: mm; elements?: \
             shell|solid|both (default: shell for sheet metal, solid otherwise); show?: stress|deflection; scale?: yield|auto; mesh_check?: \
             bool (default true: the first model is solved again with a 1.5x finer mesh, and with a 2.25x finer one when the peak away from \
             supports moved more than 1%; each number gets an error band, extrapolated from three meshes (Richardson) or from two, and is \
             settled (band within 3%), still changing, or keeps rising (a sharp inside corner or an idealised support); the finest result is \
             shown. A finer mesh that would need more memory than this computer can spare is not solved, with the reason: the smaller of 6 \
             GB and half the memory available; SOLIDCRAFT_FEA_MAX_NODES caps its nodes instead); \
             keep_files?; dry_run?: true (check the setup, describe it and draw its supports and loads on the model; solves nothing: run \
             it first so the user can see the faces; `predicted`: the nodes, memory and time a run will take); probes?: [{at: \
             [x,y,z] mm, direction?: [x,y,z], name?}] (at most 50: the stress \
             tensor, principal stresses and directions, elastic strain in microstrain (shears as engineering γ), displacement, and along \
             `direction` the stress and strain a gauge laid that way reads, at each point, read inside the solver's element that holds \
             it, with its own band; a point up to 1 mm outside the part is read at its nearest point; on shells a point on the top \
             or bottom face reads that face) → estimate: true, setup, summary, runs, fraction_of_yield (of the number the summary leads with), band_percent (its band), fraction_of_yield_overall, peak_at_support, peak_away (the highest stress \
             away from fixed supports and loads, with its principal stresses and strains), equilibrium (supports carry vs loads applied; each \
             support's force in N and moment in N·mm about its own middle; the moments' balance), mesh_check, probes, warnings, \
             assumptions, how_to_report, programs (the mesher's and solver's versions; each runs on one thread, so the same request \
             gives the same numbers). Meshes with gmsh and solves with CalculiX on this computer and paints the estimate on the body. Report it as \
             how_to_report says. skill_reference solidcraft-analysis has the workflow and worked setups",
        ),
    CommandSpec::new(SHOW, "Show Stress Estimate", crate::checked!("inspect.stress_estimate.show", show))
        .noundo()
        .icon("curvature_map")
        .doc_only()
        .contract(&SHOW_CONTRACT)
        .output(output_schema_for::<StressShowResult>)
        .params(
            "show?: stress|deflection|hide|clear (hide keeps the estimate to show again; clear drops it; off is hide); scale?: yield|auto → shown",
        ),
];

/// What the agent is told about reporting an estimate, in every reply (PRD principle 1: verdicts
/// come only from exact deterministic checks; a mesh-based stress is an estimate).
pub(crate) const HOW_TO_REPORT: &[&str] = &[
    "This is an estimate. Describe what it shows; do not judge the part, and do not talk about verdicts at all, not even to say you \
     are not giving one (no safe, unsafe, pass, fail, certify, or saying the part was checked).",
    "Say where the highest stress is, about how large it is, and what fraction of the yield strength given that is. Quote it as a share \
     of yield (\"about 60 % of the yield strength\"), not as a margin or a factor.",
    "Do not call the part or a design safe, fine, OK, strong enough, too weak or too low; do not give a safety factor; do not say it \
     passes, fails or is acceptable.",
    "Only quote numbers this estimate computed or the user gave (converting the user's kg to N is fine). For another thickness, load or \
     shape, change the design and run the estimate again; do not scale or guess numbers for designs that were not solved.",
    "Tell the user the material values used and where they came from; if you chose them, say so.",
    "If `warnings` is not empty, say what each means before the numbers.",
    "Quote every stress and deflection with its band from `mesh_check` (its `quote`): \"about 108 MPa (within ±1.2 %), 30 % of the \
     yield strength given\". The number to quote is its `estimate` (extrapolated from three meshes when `extrapolated`, else the finest \
     mesh's), and `fraction_of_yield` is the share of the summary's number, within ±`band_percent` %. Say whether it settled; say so when \
     the bands rest on two meshes (`third_mesh`). A number that keeps rising has no band and no converged value: say it is likely a sharp \
     inside corner or an idealised support, quote its finest value only as that, and give it no band. If the mesh check was skipped or \
     off, say that how far the numbers are from settling is unknown, and why (its `reason`). The mesh check is about the numbers, not the \
     part.",
    "If `peak_at_support` is set, the highest stress sits at that support or load, where an idealised hold concentrates stress: lead with \
     `peak_away` (the highest stress away from supports and loads) and its fraction of yield, and say the value at the support depends on the \
     mesh (even when one refinement step moved it little, it can keep creeping up; say so plainly when the mesh check calls it still \
     changing).",
    "Quote the `equilibrium` line (what the supports carry against the loads applied); if a warning says the forces or their moments do \
     not balance, say the result is suspect before any number. When the user asks what a support, clamp or bolted face holds, quote its \
     line in `equilibrium.supports`: its force in N and its moment in N·mm about the point given.",
    "For a point in `probes`, quote the values the user asked for (von Mises, a principal stress, a strain, the value `along` a direction) \
     with that probe's own mesh check; principal stresses are positive in tension and negative in compression; strains are microstrain \
     (µε) and shear strains are engineering (γ).",
];

fn json_of(cmd: &str, reply: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

fn finite3(v: &Value) -> Option<P> {
    let a = v.as_array()?;
    let [x, y, z] = a.as_slice() else { return None };
    let p = [x.as_f64()?, y.as_f64()?, z.as_f64()?];
    p.iter().all(|c| c.is_finite()).then_some(p)
}

/// What the command knows about the body's faces: names, sheet names, surfaces, centres.
struct BodyFaces<'a> {
    names: Vec<String>,
    sheet: Vec<Option<String>>,
    spots: Vec<Option<solidcraft_doc::naming::FaceSpot>>,
    centroids: Vec<P>,
    /// Each planar face's outward normal.
    normals: Vec<Option<P>>,
    count: usize,
    display: Faces<'a>,
    /// The display mesh's vertex normals (outward), beside `display.positions`.
    vertex_normals: &'a [P],
}

/// A cylinder face's (radius, axis, origin), if it is one.
fn cylinder(spot: Option<&solidcraft_doc::naming::FaceSpot>) -> Option<(f64, P, P)> {
    match spot?.surface? {
        solidcraft_kernel::FaceSurface::Cylinder { origin, axis, radius } => {
            let a = solidcore_record::Vec3::from(axis);
            Some((radius, [a.x, a.y, a.z], [origin.x, origin.y, origin.z]))
        }
        _ => None,
    }
}

impl BodyFaces<'_> {
    /// The faces of the same hole as cylinder face `f`: same radius, the same axis line.
    fn hole_of(&self, f: u32) -> Vec<u32> {
        let Some((r, axis, origin)) = cylinder(self.spots.get(f as usize).and_then(Option::as_ref)) else { return vec![f] };
        let tol = (r * 1e-3).max(1e-4);
        (0..self.count)
            .filter_map(|i| u32::try_from(i).ok())
            .filter(|i| {
                let Some((r2, a2, o2)) = cylinder(self.spots.get(*i as usize).and_then(Option::as_ref)) else { return false };
                let d = fea::model::sub(o2, origin);
                let off_axis = fea::model::len(fea::model::cross(d, axis));
                (r2 - r).abs() <= tol && fea::model::len(fea::model::cross(a2, axis)) < 1e-3 && off_axis <= tol
            })
            .collect()
    }

    /// The faces a `face` parameter names: an index, a name or sheet name, a point on it, or a
    /// list of those. A hole's half takes the whole hole (`notes` says so).
    fn named(&self, v: &Value, what: &str, notes: &mut Vec<String>) -> Result<Vec<u32>> {
        let one = |x: &Value| -> Result<Vec<u32>> {
            if let Some(i) = x.as_u64() {
                return u32::try_from(i)
                    .ok()
                    .filter(|i| (*i as usize) < self.count)
                    .map(|i| vec![i])
                    .ok_or_else(|| bad(CMD, format!("{what}: the body has no face {i} (it has {})", self.count)));
            }
            if let Some(at) = finite3(x) {
                return match self.display.nearest_face(at) {
                    Some((f, d)) if d <= 1.0 => Ok(vec![f]),
                    _ => Err(bad(CMD, format!("{what}: no face of the body lies within 1 mm of [{}, {}, {}]", at[0], at[1], at[2]))),
                };
            }
            let Some(name) = x.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                return Err(bad(CMD, format!("{what}: `face` must be a face's name, its index, a point [x, y, z] on it, or a list of them")));
            };
            let found: Vec<u32> = (0..self.count)
                .filter(|i| self.names.get(*i).is_some_and(|n| n == name) || self.sheet.get(*i).and_then(|n| n.as_deref()) == Some(name))
                .filter_map(|i| u32::try_from(i).ok())
                .collect();
            if found.is_empty() {
                let mut known: Vec<&str> = self.sheet.iter().flatten().map(String::as_str).chain(self.names.iter().map(String::as_str)).collect();
                known.dedup();
                let shown: Vec<&str> = known.into_iter().take(16).collect();
                return Err(bad(
                    CMD,
                    format!("{what}: no face of the body is named `{name}` (`model.faces` lists them; some: {})", shown.join(", ")),
                ));
            }
            Ok(found)
        };
        let mut faces = match v {
            Value::Array(list) if finite3(v).is_none() => {
                if list.is_empty() || list.len() > 200 {
                    return Err(bad(CMD, format!("{what}: give 1 to 200 faces")));
                }
                let mut faces = Vec::new();
                for x in list {
                    faces.extend(one(x)?);
                }
                faces
            }
            x => one(x)?,
        };
        faces.sort_unstable();
        faces.dedup();
        // A hole is two half-faces in the kernel: one half held is a half-held hole.
        let mut whole = faces.clone();
        for f in &faces {
            whole.extend(self.hole_of(*f));
        }
        whole.sort_unstable();
        whole.dedup();
        if whole.len() > faces.len() {
            notes.push(format!("{what}: took the whole of each hole picked (its other half was not in the list)"));
        }
        Ok(whole)
    }

    /// Plain words for `faces`: "2 holes Ø6.6", "the base's top side", "the far edge of flange 1".
    fn describe(&self, faces: &[u32]) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut holes: Vec<(f64, Vec<u32>)> = Vec::new();
        for f in faces {
            if let Some((r, ..)) = cylinder(self.spots.get(*f as usize).and_then(Option::as_ref)) {
                if holes.iter().any(|(_, h)| h.contains(f)) {
                    continue;
                }
                let group = self.hole_of(*f);
                // A sheet's bend is part of a turn with a name of its own ("flange 1, bend"):
                // that name says more than "a fillet". A hole or a round outside keeps its size.
                let named = self.sheet.get(*f as usize).is_some_and(Option::is_some);
                if !named || matches!(self.round_of(&group), Round::Hole | Round::Outside) {
                    holes.push((r, group));
                    continue;
                }
            }
            let words = self.sheet.get(*f as usize).and_then(|n| n.as_deref()).map(sheet_words).unwrap_or_else(|| {
                match self.normals.get(*f as usize).copied().flatten() {
                    Some(n) => format!("the face toward {}", axis_words(n)),
                    None => format!("face {f}"),
                }
            });
            if !parts.contains(&words) {
                parts.push(words);
            }
        }
        // Each round group in words by what it is (a hole, a boss's outside, a fillet, a rounded
        // edge), counted by kind and size: a fillet is not "a hole" (PIL-294 D7).
        let mut by_size: Vec<(Round, f64, usize)> = Vec::new();
        for (r, group) in &holes {
            let kind = self.round_of(group);
            match by_size.iter_mut().find(|(k, r2, _)| *k == kind && (r2 - r).abs() < 1e-6) {
                Some(x) => x.2 += 1,
                None => by_size.push((kind, *r, 1)),
            }
        }
        for (kind, r, n) in by_size.into_iter().rev() {
            parts.insert(0, kind.words(n, r));
        }
        if parts.is_empty() { "no face".into() } else { parts.join(", ") }
    }

    /// What the round faces `group` (one cylinder's) are, from their display points.
    fn round_of(&self, group: &[u32]) -> Round {
        let Some((_, axis, origin)) = group.first().and_then(|f| cylinder(self.spots.get(*f as usize).and_then(Option::as_ref))) else {
            return Round::Hole;
        };
        let mut points: Vec<(P, P)> = Vec::new();
        for (t, tri) in self.display.triangles.iter().enumerate() {
            if !self.display.tri_face.get(t).is_some_and(|f| group.contains(f)) {
                continue;
            }
            for v in tri {
                if let (Some(p), Some(n)) = (self.display.positions.get(*v as usize), self.vertex_normals.get(*v as usize)) {
                    points.push((*p, *n));
                }
            }
        }
        round_kind(&points, axis, origin)
    }
}

/// What a group of faces on one cylinder is, for words.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Round {
    /// A full turn, the part outside it: a hole.
    Hole,
    /// A full turn, the part inside it: the outside of a boss, a shaft or a disc's rim.
    Outside,
    /// Part of a turn, at most a quarter or so, the part outside it: a fillet in an inside corner.
    Fillet,
    /// Part of a turn, at most a quarter or so, the part inside it: a rounded outside edge.
    Rounded,
    /// More of a turn, the part outside it (a slot's end, a part-hole).
    InsideCurve,
    /// More of a turn, the part inside it.
    OutsideCurve,
}

impl Round {
    /// `n` of them of radius `r`, in words.
    pub(crate) fn words(self, n: usize, r: f64) -> String {
        let (one, many, size) = match self {
            Round::Hole => ("a hole", "holes", format!("Ø{}", sig(2.0 * r))),
            Round::Outside => ("a round outside face", "round outside faces", format!("Ø{}", sig(2.0 * r))),
            Round::Fillet => ("a fillet", "fillets", format!("R{}", sig(r))),
            Round::Rounded => ("a rounded edge", "rounded edges", format!("R{}", sig(r))),
            Round::InsideCurve => ("a curved inside face", "curved inside faces", format!("R{}", sig(r))),
            Round::OutsideCurve => ("a curved outside face", "curved outside faces", format!("R{}", sig(r))),
        };
        if n == 1 { format!("{one} {size}") } else { format!("{n} {many} {size}") }
    }
}

/// What the cylinder of `axis` through `origin` is, from points on its faces with their outward
/// normals: whether the faces go all the way round (no gap over 60° between their points around
/// the axis), and whether their normals face the axis (the part is outside them: a hole, a fillet)
/// or away from it. Part of a turn up to 100° is a fillet or a rounded edge.
pub(crate) fn round_kind(points: &[(P, P)], axis: P, origin: P) -> Round {
    let Some(a) = fea::model::unit(axis) else { return Round::Hole };
    let helper = if a[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let Some(e1) = fea::model::unit(fea::model::cross(a, helper)) else { return Round::Hole };
    let e2 = fea::model::cross(a, e1);
    let (mut angles, mut facing) = (Vec::new(), 0.0);
    for (p, n) in points {
        let d = fea::model::sub(*p, origin);
        let radial = fea::model::sub(d, fea::model::mul(a, fea::model::dot(d, a)));
        let Some(u) = fea::model::unit(radial) else { continue };
        facing += fea::model::dot(*n, u);
        angles.push(fea::model::dot(radial, e2).atan2(fea::model::dot(radial, e1)));
    }
    if angles.is_empty() {
        return Round::Hole;
    }
    angles.sort_by(f64::total_cmp);
    let wrap = angles.first().copied().unwrap_or(0.0) + std::f64::consts::TAU - angles.last().copied().unwrap_or(0.0);
    let gap = angles.windows(2).map(|w| w[1] - w[0]).fold(wrap, f64::max);
    let span = std::f64::consts::TAU - gap;
    let inside = facing < 0.0;
    if gap < 60f64.to_radians() {
        if inside { Round::Hole } else { Round::Outside }
    } else if span <= 100f64.to_radians() {
        if inside { Round::Fillet } else { Round::Rounded }
    } else if inside {
        Round::InsideCurve
    } else {
        Round::OutsideCurve
    }
}

impl BodyFaces<'_> {
    /// Where a marker for `faces` goes: the mean of their centres.
    fn centre(&self, faces: &[u32]) -> P {
        let pts: Vec<P> = faces.iter().filter_map(|f| self.centroids.get(*f as usize).copied()).collect();
        if pts.is_empty() {
            return [0.0; 3];
        }
        fea::model::mul(pts.iter().fold([0.0; 3], |a, p| fea::model::add(a, *p)), 1.0 / pts.len() as f64)
    }

    /// The axis a roller on `faces` holds (0 x, 1 y, 2 z): every face flat and facing along the
    /// same one of X, Y or Z. Anything else is refused before anything runs: a tilted roller needs
    /// a coordinate system of its own, which the estimate cannot set up yet.
    fn roller_axis(&self, faces: &[u32], label: &str, what: &str) -> Result<usize> {
        let mut axis: Option<usize> = None;
        for f in faces {
            let Some(n) = self.normals.get(*f as usize).copied().flatten().and_then(fea::model::unit) else {
                return Err(bad(
                    CMD,
                    format!("{what}: a roller holds a flat face across itself, and {label} is not flat; hold it `fixed`, or pick a flat face"),
                ));
            };
            let Some(k) = (0..3).find(|k| n[*k].abs() >= 1.0 - 1e-6) else {
                return Err(bad(
                    CMD,
                    format!(
                        "{what}: a roller can hold only a face that faces along X, Y or Z for now, and {label} faces {}; a tilted roller needs \
                         a coordinate system of its own, which the estimate does not set up yet. Hold it `fixed`, or model the part so the \
                         face lies along an axis",
                        axis_words(n)
                    ),
                ));
            };
            if axis.is_some_and(|a| a != k) {
                return Err(bad(
                    CMD,
                    format!("{what}: a roller's faces must all face along the same axis; give faces along another axis their own support"),
                ));
            }
            axis = Some(k);
        }
        axis.ok_or_else(|| bad(CMD, format!("{what}: give `face`")))
    }

    /// The outward normal `faces` share, when every one is flat and they all face the same way.
    fn common_normal(&self, faces: &[u32]) -> Option<P> {
        let mut n: Option<P> = None;
        for f in faces {
            let m = self.normals.get(*f as usize).copied().flatten().and_then(fea::model::unit)?;
            if n.is_some_and(|n| fea::model::dot(n, m) < 1.0 - 1e-6) {
                return None;
            }
            n = Some(m);
        }
        n
    }

    /// The face nearest a point, in plain words.
    fn near(&self, at: P) -> String {
        match self.display.nearest_face(at) {
            Some((f, _)) => self.describe(&[f]),
            None => "?".into(),
        }
    }
}

/// A sheet face name (`sheet:<body>:<panel>:<side>`) in words.
fn sheet_words(name: &str) -> String {
    let mut parts = name.split(':').skip(2);
    let panel = parts.next().unwrap_or("");
    let side = parts.next().unwrap_or("");
    let panel = if panel == "base" {
        "the base".to_string()
    } else if let Some(rest) = panel.strip_prefix('F') {
        let n = rest.split('.').nth(1).and_then(|k| k.parse::<usize>().ok()).map_or(String::new(), |k| format!(" {}", k + 1));
        format!("flange{n}")
    } else {
        panel.to_string()
    };
    match side {
        "top" => format!("{panel}, top side"),
        "bottom" => format!("{panel}, underside"),
        s if s.starts_with("rim") => {
            let which = s.strip_prefix("rim.").unwrap_or("");
            let which = match which {
                "far" => "far ",
                "near" => "near ",
                _ => "",
            };
            format!("{panel}, {which}edge")
        }
        s if s.starts_with("bend") => format!("{panel}, bend"),
        "" => panel,
        s => format!("{panel}, {s}"),
    }
}

/// A face's outward direction in words: "+X", "−Z (underside)", or its components.
fn axis_words(n: P) -> String {
    let Some(u) = fea::model::unit(n) else { return "?".into() };
    for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
        if u[k].abs() > 0.995 {
            let sign = if u[k] > 0.0 { "+" } else { "−" };
            return match (k, u[k] > 0.0) {
                (2, true) => "+Z (top)".into(),
                (2, false) => "−Z (underside)".into(),
                _ => format!("{sign}{axis}"),
            };
        }
    }
    format!("({}, {}, {})", sig(u[0]), sig(u[1]), sig(u[2]))
}

/// A direction in words: "−Y", "down (−Z)", or its components.
fn direction_words(f: P) -> String {
    let Some(u) = fea::model::unit(f) else { return "nowhere".into() };
    for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
        if u[k].abs() > 0.995 {
            let sign = if u[k] > 0.0 { "+" } else { "−" };
            return match (k, u[k] > 0.0) {
                (2, false) => "down (−Z)".into(),
                (2, true) => "up (+Z)".into(),
                _ => format!("along {sign}{axis}"),
            };
        }
    }
    format!("along ({}, {}, {})", sig(u[0]), sig(u[1]), sig(u[2]))
}

/// A pressure in MPa: a number (MPa), or text with its unit ("10 MPa", "10 N/mm²", "250 kPa",
/// "2 bar", "1e5 Pa"). Text without a unit is refused rather than guessed. Positive pushes in.
pub(crate) fn pressure_mpa(v: &Value) -> std::result::Result<f64, String> {
    let mpa = if let Some(x) = v.as_f64() {
        x
    } else if let Some(text) = v.as_str() {
        let text = text.trim();
        // The number runs to the first character no number has (no unit starts with e or E).
        let split = text.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(text.len());
        let (number, unit) = text.split_at(split);
        let number: f64 = number.trim().parse().map_err(|_| format!("`pressure` `{text}` is not a number with a unit (such as \"10 MPa\")"))?;
        let factor = match unit.trim().replace(' ', "").to_ascii_lowercase().as_str() {
            "" => return Err(format!("`pressure` `{text}` has no unit: give a number in MPa, or the number with its unit, such as \"{text} MPa\"")),
            "mpa" | "n/mm2" | "n/mm²" | "n/mm^2" => 1.0,
            "kpa" => 1e-3,
            "pa" | "n/m2" | "n/m²" | "n/m^2" => 1e-6,
            "bar" => 0.1,
            "gpa" => 1e3,
            other => return Err(format!("`pressure`: the unit `{other}` is not known; use MPa, N/mm², kPa, Pa or bar")),
        };
        number * factor
    } else {
        return Err("`pressure` must be a number in MPa, or text with its unit such as \"10 MPa\"".into());
    };
    if !(mpa.abs() > 0.0 && mpa.abs() <= 1.0e4) {
        return Err("`pressure` must be above 0 and at most 10000 MPa in size (positive pushes into the part, negative pulls out)".into());
    }
    Ok(mpa)
}

/// The material: a grade by name (`"6061-T6"`, or `{name}` alone) takes the built-in table's
/// sourced numbers ([`fea::materials`]); any other material comes with all its numbers and their
/// source. `body_material` is the body's assigned (generic) material, to suggest its grades.
fn material_of(p: &Value, body_material: &str) -> Result<Material> {
    let own_numbers = "or give {name, youngs_modulus (MPa), poisson_ratio, yield_strength (MPa), source} with where the numbers come from";
    let unknown =
        |name: &str| bad(CMD, format!("`{name}` is not a grade the estimate knows; give one of: {}; {own_numbers}", fea::materials::names()));
    let Some(m) = p.get("material") else {
        let grades = fea::materials::grades_of(body_material);
        return Err(bad(
            CMD,
            if grades.is_empty() {
                format!("give `material`: a grade by name ({}), {own_numbers}", fea::materials::names())
            } else {
                format!("give `material`: this body is {body_material}, so say which grade ({}), {own_numbers}", grades.join(", "))
            },
        ));
    };
    if let Some(name) = m.as_str() {
        return fea::materials::grade(name).map(|g| g.material()).ok_or_else(|| unknown(name));
    }
    if !m.is_object() {
        return Err(bad(CMD, format!("`material` must be a grade's name or an object; {own_numbers}")));
    }
    let text = |k: &str| m.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty() && s.len() <= 400).map(str::to_string);
    let number = |k: &str| m.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
    let name = text("name").ok_or_else(|| bad(CMD, "`material.name` is missing"))?;
    // A name alone: the table's grade.
    if ["youngs_modulus", "poisson_ratio", "yield_strength", "source"].iter().all(|k| m.get(*k).is_none()) {
        return fea::materials::grade(&name).map(|g| g.material()).ok_or_else(|| unknown(&name));
    }
    let source = text("source").ok_or_else(|| bad(CMD, "`material.source` is missing: say where the numbers come from (a datasheet, a handbook)"))?;
    let e_mpa = number("youngs_modulus")
        .filter(|e| (100.0..=1.0e6).contains(e))
        .ok_or_else(|| bad(CMD, "`material.youngs_modulus` must be in MPa, 100 to 1000000 (aluminium is about 70000)"))?;
    let nu =
        number("poisson_ratio").filter(|n| *n > 0.0 && *n < 0.5).ok_or_else(|| bad(CMD, "`material.poisson_ratio` must be above 0 and below 0.5"))?;
    let yield_mpa =
        number("yield_strength").filter(|y| *y > 0.0 && *y <= 1.0e4).ok_or_else(|| bad(CMD, "`material.yield_strength` must be in MPa, above 0"))?;
    Ok(Material { name, e_mpa, nu, yield_mpa, source })
}

fn round_to(x: f64, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (x * k).round() / k
}

/// A number to about three significant figures, for sentences. The decimals follow the number as
/// rounded, so 9.99999 reads "10.0", not "10.00".
fn sig(x: f64) -> String {
    if !x.is_finite() {
        return "?".into();
    }
    let decimals = |a: f64| {
        if a >= 100.0 {
            0
        } else if a >= 10.0 {
            1
        } else if a >= 1.0 {
            2
        } else {
            3
        }
    };
    let first = format!("{x:.*}", decimals(x.abs()));
    let rounded = first.parse::<f64>().map_or(x.abs(), f64::abs);
    if decimals(rounded) == decimals(x.abs()) {
        return first;
    }
    format!("{x:.*}", decimals(rounded))
}

/// Everything the estimate needs, checked: what a dry run reports and a solve uses.
struct Plan {
    body: String,
    loads: Vec<Load>,
    supports: Vec<Support>,
    material: Material,
    runs: Vec<Elements>,
    quantity: Quantity,
    scale: ScaleMode,
    thickness: f64,
    sheet: bool,
    mesh_size: Option<f64>,
    keep_files: bool,
    mesh_check: bool,
    probes: Vec<Probe>,
    notes: Vec<String>,
    setup: Vec<String>,
}

/// A point to read the stress and strain at, checked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Probe {
    pub name: String,
    pub at: P,
    /// A unit vector.
    pub direction: Option<P>,
}

/// At most this many probes in one estimate.
pub(crate) const MAX_PROBES: usize = 50;

/// The setup's probes, checked: a finite point, a direction that is not zero, a name of 1 to 60
/// characters (`probe 1`, `probe 2`, ... by default).
pub(crate) fn probes_of(p: &Value) -> Result<Vec<Probe>> {
    let list = match p.get("probes") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(v) => v.as_array().ok_or_else(|| bad(CMD, "`probes` must be a list: [{at: [x, y, z], direction?, name?}]"))?,
    };
    if list.len() > MAX_PROBES {
        return Err(bad(CMD, format!("give at most {MAX_PROBES} probes")));
    }
    let mut out = Vec::new();
    for (i, v) in list.iter().enumerate() {
        let what = format!("probes[{i}]");
        let at = v.get("at").and_then(finite3).ok_or_else(|| bad(CMD, format!("{what}: `at` must be a point [x, y, z] in mm")))?;
        let direction = match v.get("direction") {
            None | Some(Value::Null) => None,
            Some(d) => {
                Some(finite3(d).and_then(fea::model::unit).ok_or_else(|| bad(CMD, format!("{what}: `direction` must be [x, y, z], not zero")))?)
            }
        };
        let name = match v.get("name") {
            None | Some(Value::Null) => format!("probe {}", i + 1),
            Some(n) => n
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty() && n.chars().count() <= 60)
                .ok_or_else(|| bad(CMD, format!("{what}: `name` must be text of 1 to 60 characters")))?
                .to_string(),
        };
        out.push(Probe { name, at, direction });
    }
    Ok(out)
}

/// A point for a sentence: `[50, 10, 1.5]`.
fn point_words(p: P) -> String {
    format!("[{}, {}, {}]", coord(p[0], 1.0), coord(p[1], 1.0), coord(p[2], 1.0))
}

/// A probe in the setup's words.
fn probe_words(pr: &Probe, near: &str) -> String {
    let along = pr.direction.map_or_else(String::new, |d| format!(", and {}", along_words(d)));
    format!("Read at {}: the stress and strain at {} mm, near {near}{along}", pr.name, point_words(pr.at))
}

/// The mesh check solves again with the element size divided by this, and again for a third mesh.
pub(crate) const MESH_CHECK_RATIO: f64 = fea::REFINEMENT_RATIO;
/// The third mesh (h / 2.25) is solved when the lead peak moved by more than this (%) from the first
/// mesh to the second (PIL-294 A6): a number that moved less is settled on two meshes, its band
/// (twice its change) within 2 %.
pub(crate) const THIRD_MESH_PERCENT: f64 = 1.0;
/// The finer run's nodes per node of the first, for solids. 1.5³ ≈ 3.4 is the count in a part
/// many elements thick; measured (gmsh 4, PIL-294): R2's plate 199 504 → 595 186 nodes (2.98)
/// and 66 300 → 199 504 at 1.5 mm (3.01), R3's LE1 ring 44 748 → 132 915 (2.97), R1's 1.5 mm
/// strip 14 555 → 30 447 (2.09: one or two elements through its thickness, so it refines as a
/// surface). 3.0 fits the parts that matter for memory and overestimates thin ones, the safe side.
pub(crate) const SOLID_GROWTH: f64 = 3.0;
/// The same for shells, which mesh a surface: 1.5² = 2.25; measured 26 607 → 58 673 (2.21) on
/// R1's strip as shells.
pub(crate) const SHELL_GROWTH: f64 = 2.25;
/// The solver's peak memory (CalculiX 2.21, SPOOLES), bytes, for `n` nodes:
/// `per_node × n × (n / at)^extra`. The bytes a node grow with the model (the factorisation's
/// fill-in grows faster than the nodes), so no single figure fits small and large runs. Peak
/// resident memory of the solver, measured with /usr/bin/time -v on zhikun (PIL-294 follow-up,
/// 2026-11-16), against this formula:
///
/// | run | nodes | measured | predicted |
/// |---|---|---|---|
/// | R1 strip, solid | 14 555 / 30 447 | 155 / 336 MB | 174 / 421 MB |
/// | R3 LE1 ring | 44 748 / 132 915 | 590 MB / 2.44 GB | 669 MB / 2.47 GB |
/// | R2 plate | 66 300 / 199 504 / 595 186 | 0.82 / 3.47 / 13.99 GB | 1.07 / 4.02 / 14.9 GB |
/// | R1 strip, shells | 26 607 / 47 053 / 58 673 / 104 175 | 1.08 / 1.99 / 2.52 / 4.66 GB | 1.09 / 2.04 / 2.61 / 4.90 GB |
///
/// Every prediction is at or above the measurement. The mesher's peak is far below (0.33 GB for
/// R2's 595 186-node mesh).
pub(crate) const SOLID_MEMORY: (f64, f64, f64) = (18_500.0, 130_000.0, 0.2);
/// The same for shells, per node of the mid-surface (the solver expands each into three through
/// the thickness).
pub(crate) const SHELL_MEMORY: (f64, f64, f64) = (44_500.0, 60_000.0, 0.1);
/// Nodes of a second-order tetrahedral mesh per mm³ of part with 1 mm elements: a mesh of `h` mm
/// has about this × volume / h³ nodes. Measured with curvature sizing on R2's plate (200 x 50 x
/// 2 mm, a Ø10 hole): 11.3 at 1.5 mm, 10.1 at 1.0 mm, 8.5 at 0.7 mm.
pub(crate) const SOLID_NODES_PER_MM3: f64 = 10.0;
/// The most nodes a default solid mesh is sized for: a first run whose mesh check's finer run
/// still fits [`MEMORY_CEILING`] (a first run of about 93 000 nodes, see there). The mesh is sized
/// at [`SOLID_NODES_PER_MM3`], but a real mesh has up to 13 % more (11.3 per mm³ measured), so
/// 80 000 at the nominal density lands at or under about 90 000. At 90 000 there was no margin: at
/// 11.3 per mm³ a first run has about 102 000 nodes, its finer run is predicted over the ceiling and
/// the mesh check is skipped (PIL-294 D9, the review's Ø100 x 2 mm disc).
pub(crate) const DEFAULT_SOLID_NODES: f64 = 80_000.0;

/// The element size of a run of `elements`: `mesh_size` when given (a sheet's solids no coarser
/// than its thickness), else the default for the part (`diag` its bounding box's diagonal,
/// `smallest` its thinnest side, `volume` a solid's that is not a sheet).
pub(crate) fn run_size(
    mesh_size: Option<f64>,
    elements: Elements,
    sheet: bool,
    thickness: f64,
    diag: f64,
    smallest: f64,
    volume: Option<f64>,
) -> f64 {
    match (mesh_size, elements, sheet) {
        (Some(h), Elements::Shell, _) => h,
        (Some(h), Elements::Solid, true) => h.min(thickness.max(0.3)),
        (Some(h), Elements::Solid, false) => h,
        (None, Elements::Shell, _) => (diag / 30.0).clamp(1.5 * thickness, 8.0 * thickness),
        (None, Elements::Solid, true) => thickness.max(0.3),
        (None, Elements::Solid, false) => default_solid_size(diag, smallest, volume),
    }
}

/// The default element size for a solid part that is not sheet metal, mm: about 25 elements
/// across and two through its thinnest side, but no finer than a mesh of about
/// [`DEFAULT_SOLID_NODES`] for its `volume` (mm³). Without that, a large thin part got a mesh too
/// big for this computer: R2's 200 x 50 x 2 mm plate was meshed at 1.0 mm (199 504 nodes), so the
/// mesh check's finer run (595 186 nodes, 14 GB measured) never fitted; it is meshed at about
/// 1.35 mm now. Under 0.3 mm only when given.
pub(crate) fn default_solid_size(diag: f64, smallest: f64, volume: Option<f64>) -> f64 {
    let detail = (diag / 25.0).min(smallest / 2.0);
    let budget = volume.filter(|v| v.is_finite() && *v > 0.0).map_or(0.0, |v| (SOLID_NODES_PER_MM3 * v / DEFAULT_SOLID_NODES).cbrt());
    detail.max(budget).max(0.3)
}

/// The most memory the mesh check's finer run may need, bytes, wherever the computer has more to
/// spare. 6 GB: a finer run of about 278 000 solid nodes (a first run of about 93 000) or
/// 125 000 shell nodes (a first run of about 55 000), about a minute and a half of solving for
/// solids; it leaves a 16 GB laptop room for the app and the system. R2's plate at its default
/// mesh (a 595 186-node finer run, 14 GB measured) is over it.
pub(crate) const MEMORY_CEILING: f64 = 6.0e9;
/// A node cap for the finer run that replaces the memory limit (for tests and power users).
pub(crate) const MAX_NODES_VAR: &str = "SOLIDCRAFT_FEA_MAX_NODES";

/// The setup's keys, by where they go: one the estimate does not know is refused, not ignored,
/// because a dropped load or support changes the answer without a word (PIL-294 D6). Kept equal
/// to the contracts in `schema/stress.rs` by a test.
pub(crate) const ESTIMATE_KEYS: &[&str] = &[
    "body",
    "loads",
    "supports",
    "material",
    "thickness",
    "mesh_size",
    "elements",
    "show",
    "scale",
    "mesh_check",
    "keep_files",
    "dry_run",
    "probes",
];
pub(crate) const PROBE_KEYS: &[&str] = &["at", "direction", "name"];
pub(crate) const LOAD_KEYS: &[&str] = &["face", "type", "pressure", "force", "direction", "magnitude"];
pub(crate) const SUPPORT_KEYS: &[&str] = &["face", "type"];
pub(crate) const MATERIAL_KEYS: &[&str] = &["name", "youngs_modulus", "poisson_ratio", "yield_strength", "source"];

/// What a key the estimate does not take means, where it says more than "unknown".
fn unknown_key_hint(key: &str) -> &'static str {
    match key {
        "at" | "point" | "remote" | "moment" | "torque" | "couple" => {
            ": a remote force, a moment or a torque is not supported yet; put the force on the faces it goes in through"
        }
        "load_cases" | "cases" | "load_case" | "combinations" => ": one estimate takes one set of loads; run it once for each load case",
        "gravity" | "acceleration" | "self_weight" | "density" => ": gravity and other body loads are not supported yet",
        "units" | "unit" => ": forces are N, lengths mm and pressures MPa (or text with its unit, such as \"250 kPa\")",
        "preload" | "bolt" | "contact" | "bearing" => ": bolts, bearing loads and contact are not supported yet",
        _ => "",
    }
}

/// Refuses any key of the setup the estimate does not take: at the top, in a load, a support or a
/// material given as an object.
pub(crate) fn check_keys(p: &Value) -> Result<()> {
    fn check(v: &Value, allowed: &[&str], what: &str) -> Result<()> {
        let Some(obj) = v.as_object() else { return Ok(()) };
        if let Some(key) = obj.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(bad(
                CMD,
                format!("{what}: `{key}` is not a setting of the estimate{} (it takes: {})", unknown_key_hint(key), allowed.join(", ")),
            ));
        }
        Ok(())
    }
    check(p, ESTIMATE_KEYS, "the setup")?;
    for (list, allowed, name) in [("loads", LOAD_KEYS, "loads"), ("supports", SUPPORT_KEYS, "supports"), ("probes", PROBE_KEYS, "probes")] {
        for (i, v) in p.get(list).and_then(Value::as_array).into_iter().flatten().enumerate() {
            check(v, allowed, &format!("{name}[{i}]"))?;
        }
    }
    if let Some(m) = p.get("material") {
        check(m, MATERIAL_KEYS, "material")?;
    }
    Ok(())
}
/// The peak "away from supports and loads" is looked for at nodes farther than this many
/// element sizes (of the first mesh) from every fixed support's and force's node (a roller or a
/// pressure concentrates nothing, see `fea::solve`). A node's stress
/// is the average of the elements around it, and the elements that touch a clamped or loaded
/// node (where an idealised support or load concentrates stress without limit) reach about one
/// element size from it: 1.5 keeps every node of those elements out, with half an element to
/// spare for the mesher's elements that come out larger than asked. The same distance is used for
/// the finer run, so the mesh check compares the same region.
pub(crate) const AWAY_ELEMENTS: f64 = 1.5;
/// ... and at least this many section sizes (`section_size`), however fine the mesh: by
/// Saint-Venant's principle a clamp's or a force's local disturbance fades over about one section
/// size, and on a fine mesh 1.5 elements fall inside it. Measured on R5 (1 mm shells, 1.5 mm
/// thick): 1.5 elements put the away peak 1.34 mm from the clamp, 4.7 % over beam theory; two
/// thicknesses put it 5.5 mm away, 1.1 % over (three: the same). One thickness is no farther than
/// 1.5 elements on any mesh coarser than 2/3 of the thickness, so it changes too little.
pub(crate) const AWAY_SECTIONS: f64 = 2.0;

/// How far from every fixed support's and force's node the peak "away" from them is looked for,
/// mm: `AWAY_ELEMENTS` element sizes `h`, and at least `AWAY_SECTIONS` section sizes.
pub(crate) fn away_distance(h: f64, section: f64) -> f64 {
    let floor = if section.is_finite() && section > 0.0 { AWAY_SECTIONS * section } else { 0.0 };
    (AWAY_ELEMENTS * h).max(floor)
}

/// The part's section size, mm, for `away_distance`: a shell's or a sheet's thickness; for any
/// other solid twice its volume over its surface area, which is a plate's thickness (2 V / S =
/// t / (1 + t (1/a + 1/b)) for an a x b x t plate) and a round bar's radius. 0 when unknown.
pub(crate) fn section_size(elements: Elements, sheet: bool, thickness: f64, volume: Option<f64>, area: f64) -> f64 {
    if elements == Elements::Shell || sheet {
        return thickness;
    }
    match volume {
        Some(v) if v.is_finite() && v > 0.0 && area.is_finite() && area > 0.0 => 2.0 * v / area,
        _ => 0.0,
    }
}

fn plan(p: &Value, faces: &BodyFaces, body: &solidcraft_doc::ModelBody, sheet_t: Option<f64>, body_material: &str) -> Result<Plan> {
    let mut notes = Vec::new();
    // Supports and loads: refused before anything runs when missing.
    let supports_v = p.get("supports").and_then(Value::as_array).cloned().unwrap_or_default();
    if supports_v.is_empty() {
        return Err(bad(
            CMD,
            "give at least one support (`supports: [{face, type: fixed}]`): without one the part is free to move and no stress can be estimated",
        ));
    }
    let loads_v = p.get("loads").and_then(Value::as_array).cloned().unwrap_or_default();
    if loads_v.is_empty() {
        return Err(bad(CMD, "give at least one load (`loads: [{face, force: [x, y, z]}]`, N)"));
    }
    if supports_v.len() > 50 || loads_v.len() > 50 {
        return Err(bad(CMD, "give at most 50 loads and 50 supports"));
    }
    let mut supports = Vec::new();
    for (i, v) in supports_v.iter().enumerate() {
        let what = format!("supports[{i}]");
        let kind = match v.get("type") {
            None => "fixed",
            Some(t) => t.as_str().map(str::trim).ok_or_else(|| bad(CMD, format!("{what}: `type` must be fixed or roller")))?,
        };
        let roller = match kind {
            "fixed" => None,
            "roller" | "slider" => Some(false),
            "symmetry" => Some(true),
            other => {
                return Err(bad(
                    CMD,
                    format!(
                        "{what}: `type` `{other}` is not available yet; `fixed` (held still) or `roller` (a flat face held only across itself; also `symmetry`, `slider`)"
                    ),
                ));
            }
        };
        let face = v.get("face").ok_or_else(|| bad(CMD, format!("{what}: give `face`")))?;
        let list = faces.named(face, &what, &mut notes)?;
        let label = faces.describe(&list);
        let kind = match roller {
            None => SupportKind::Fixed,
            Some(false) => SupportKind::Roller(faces.roller_axis(&list, &label, &what)?),
            Some(true) => SupportKind::Symmetry(faces.roller_axis(&list, &label, &what)?),
        };
        supports.push(Support { label, faces: list, kind });
    }
    let mut loads = Vec::new();
    for (i, v) in loads_v.iter().enumerate() {
        let what = format!("loads[{i}]");
        let face = v.get("face").ok_or_else(|| bad(CMD, format!("{what}: give `face`")))?;
        let list = faces.named(face, &what, &mut notes)?;
        let is_pressure = match v.get("type") {
            None => v.get("pressure").is_some(),
            Some(t) => match t.as_str().map(str::trim) {
                Some("force") => false,
                Some("pressure") => true,
                _ => return Err(bad(CMD, format!("{what}: `type` must be force or pressure"))),
            },
        };
        let kind = if is_pressure {
            if v.get("force").is_some() || v.get("direction").is_some() || v.get("magnitude").is_some() {
                return Err(bad(CMD, format!("{what}: a pressure takes `pressure` alone (no force, direction or magnitude)")));
            }
            let p = v
                .get("pressure")
                .ok_or_else(|| bad(CMD, format!("{what}: give `pressure`, in MPa (a number, or text with its unit such as \"10 MPa\")")))?;
            LoadKind::Pressure(pressure_mpa(p).map_err(|e| bad(CMD, format!("{what}: {e}")))?)
        } else {
            let force = match (v.get("force"), v.get("direction"), v.get("magnitude")) {
                (Some(f), None, None) => finite3(f).ok_or_else(|| bad(CMD, format!("{what}: `force` must be [x, y, z] in N")))?,
                (None, Some(d), Some(m)) => {
                    let d =
                        finite3(d).and_then(fea::model::unit).ok_or_else(|| bad(CMD, format!("{what}: `direction` must be a non-zero [x, y, z]")))?;
                    let m = m.as_f64().filter(|m| m.is_finite()).ok_or_else(|| bad(CMD, format!("{what}: `magnitude` must be a number of N")))?;
                    fea::model::mul(d, m)
                }
                _ => return Err(bad(CMD, format!("{what}: give `force` [x, y, z] in N, or `direction` and `magnitude`, or `pressure` in MPa"))),
            };
            let size = fea::model::len(force);
            if !(size > 0.0) || size > 1.0e7 {
                return Err(bad(CMD, format!("{what}: the force must be above 0 N and at most 10 MN")));
            }
            LoadKind::Force(force)
        };
        let label = faces.describe(&list);
        if supports.iter().any(|s| s.faces.iter().any(|f| list.contains(f))) {
            return Err(bad(CMD, format!("{what}: {label} is also held by a support; a load on a held face does nothing")));
        }
        loads.push(Load { label, faces: list, kind });
    }
    let material = material_of(p, body_material)?;
    let sheet = sheet_t.is_some();
    let default_elements = if sheet { "shell" } else { "solid" };
    let runs: Vec<Elements> = match p.get("elements").and_then(Value::as_str).unwrap_or(default_elements) {
        "shell" => vec![Elements::Shell],
        "solid" => vec![Elements::Solid],
        "both" => vec![Elements::Shell, Elements::Solid],
        other => return Err(bad(CMD, format!("`elements` must be shell, solid or both (not `{other}`)"))),
    };
    let quantity = match p.get("show").and_then(Value::as_str).unwrap_or("stress") {
        "stress" => Quantity::Stress,
        "deflection" => Quantity::Deflection,
        other => return Err(bad(CMD, format!("`show` must be stress or deflection (not `{other}`)"))),
    };
    let scale = match p.get("scale").and_then(Value::as_str).unwrap_or("yield") {
        "yield" => ScaleMode::Yield,
        "auto" => ScaleMode::Auto,
        other => return Err(bad(CMD, format!("`scale` must be yield or auto (not `{other}`)"))),
    };
    let thickness = match p.get("thickness") {
        Some(v) => v.as_f64().filter(|t| t.is_finite() && *t > 0.0 && *t <= 50.0).ok_or_else(|| bad(CMD, "`thickness` must be in mm, above 0"))?,
        None => match sheet_t {
            Some(t) => t,
            None if runs.contains(&Elements::Shell) => {
                return Err(bad(
                    CMD,
                    "this body was not made as sheet metal, so shells need `thickness` (mm); for a solid part leave `elements` out (solid is \
                     the default) and no thickness is needed",
                ));
            }
            None => 1.0,
        },
    };
    let mesh_size = match p.get("mesh_size") {
        Some(v) => Some(
            v.as_f64().filter(|h| h.is_finite() && *h >= 0.05 && *h <= 1000.0).ok_or_else(|| bad(CMD, "`mesh_size` must be in mm, 0.05 to 1000"))?,
        ),
        None => None,
    };
    let keep_files = p.get("keep_files").and_then(Value::as_bool).unwrap_or(false);
    let mesh_check = match p.get("mesh_check") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(CMD, "`mesh_check` must be true or false")),
    };
    let mut setup = vec![format!("Body: {}", body.name)];
    for l in &loads {
        setup.push(load_words(l));
    }
    for sp in &supports {
        setup.push(support_words(sp));
    }
    setup.push(format!(
        "Material: {} — E {} GPa, Poisson {}, yield {} MPa (source: {})",
        material.name,
        sig(material.e_mpa / 1000.0),
        material.nu,
        sig(material.yield_mpa),
        material.source
    ));
    let model_words: Vec<String> = runs.iter().map(|e| e.plain(thickness)).collect();
    setup.push(format!("Model: {}", model_words.join(", then ")));
    setup.push(if mesh_check {
        let limit = match finer_limit_here() {
            Limit::Memory(cap) => format!("about {} of memory, this computer's limit", about_bytes(cap)),
            Limit::Nodes(cap) => format!("{} nodes, the limit set by {MAX_NODES_VAR}", about_count(cap)),
        };
        format!(
            "Mesh check: the first model is solved again with a mesh {MESH_CHECK_RATIO}× finer, and {}× finer when the peak away from \
             supports and loads moves more than {THIRD_MESH_PERCENT}%, to give each number an error band (unless a finer mesh would need \
             more than {limit})",
            MESH_CHECK_RATIO * MESH_CHECK_RATIO
        )
    } else {
        "Mesh check: off (mesh_check: false), so how far the numbers are from settling will be unknown".to_string()
    });
    let probes = probes_of(p)?;
    for pr in &probes {
        setup.push(probe_words(pr, &faces.near(pr.at)));
    }
    setup.extend(notes.iter().cloned());
    Ok(Plan {
        body: body.name.clone(),
        loads,
        supports,
        material,
        runs,
        quantity,
        scale,
        thickness,
        sheet,
        mesh_size,
        keep_files,
        mesh_check,
        probes,
        notes,
        setup,
    })
}

/// A load in words: "Load: 10.0 N down (−Z) on …", "Pressure: 10.0 MPa pushing into …".
fn load_words(l: &Load) -> String {
    match l.kind {
        LoadKind::Force(f) => format!("Load: {} N {} on {}", sig(fea::model::len(f)), direction_words(f), l.label),
        LoadKind::Pressure(p) if p > 0.0 => format!("Pressure: {} MPa on {}, pushing into the part", sig(p), l.label),
        LoadKind::Pressure(p) => format!("Pressure: {} MPa on {}, pulling out of the part (a negative pressure)", sig(p), l.label),
    }
}

/// What is drawn for the loads (an arrow and its size) and supports (a held mark and its kind),
/// on the setup preview and on the result.
fn markers_of(plan: &Plan, faces: &BodyFaces) -> (Vec<Marker>, Vec<Marker>) {
    let loads = plan
        .loads
        .iter()
        .map(|l| match l.kind {
            LoadKind::Force(f) => Marker { at: faces.centre(&l.faces), dir: fea::model::unit(f), text: format!("{} N", sig(fea::model::len(f))) },
            LoadKind::Pressure(p) => Marker {
                at: faces.centre(&l.faces),
                // Along the faces' inward normal when they are flat and face one way.
                dir: faces.common_normal(&l.faces).map(|n| fea::model::mul(n, -p.signum())),
                text: format!("{} MPa", sig(p)),
            },
        })
        .collect();
    let supports = plan
        .supports
        .iter()
        .flat_map(|sp| {
            let axis = |k: usize| ["X", "Y", "Z"].get(k).copied().unwrap_or("?");
            let text = match sp.kind {
                SupportKind::Fixed => "fixed".to_string(),
                SupportKind::Roller(k) => format!("roller, held along {}", axis(k)),
                SupportKind::Symmetry(k) => format!("symmetry, held along {}", axis(k)),
            };
            // A marker on each hole, else one on the faces' middle.
            let mut seen: Vec<u32> = Vec::new();
            let mut out = Vec::new();
            for f in &sp.faces {
                if seen.contains(f) {
                    continue;
                }
                let group = faces.hole_of(*f);
                seen.extend(group.iter().copied());
                out.push(Marker { at: faces.centre(&group), dir: None, text: text.clone() });
            }
            out
        })
        .collect();
    (loads, supports)
}

/// A support in words: "Held still: …", "Held along X only (a roller, free to slide across it): …".
fn support_words(s: &Support) -> String {
    match s.kind {
        SupportKind::Fixed => format!("Held still: {}", s.label),
        SupportKind::Roller(axis) => {
            let a = ["X", "Y", "Z"].get(axis).copied().unwrap_or("?");
            format!("Held along {a} only (a roller: free to slide in its own plane): {}", s.label)
        }
        SupportKind::Symmetry(axis) => {
            let a = ["X", "Y", "Z"].get(axis).copied().unwrap_or("?");
            format!(
                "Held along {a} only (a symmetry plane: free to slide in its own plane; as shells, its edge is also kept from tilting out of                  it): {}",
                s.label
            )
        }
    }
}

fn estimate(s: &mut Session, p: &Value) -> Result<Value> {
    check_keys(p)?;
    // What to analyse, and its faces.
    let st = s.world_state()?;
    let visible: Vec<&solidcraft_doc::ModelBody> = st.bodies.iter().filter(|b| s.body_visible(&b.name)).collect();
    let body = match p.get("body").and_then(Value::as_str) {
        Some(name) => st.bodies.iter().find(|b| b.name == name).ok_or_else(|| bad(CMD, format!("no body is named `{name}`")))?,
        None => match visible.as_slice() {
            [one] => *one,
            [] => return Err(bad(CMD, "there is no body to analyse")),
            _ => return Err(bad(CMD, "give `body`: the design has several bodies")),
        },
    };
    if body.body.is_mesh() {
        return Err(bad(CMD, format!("`{}` is a mesh body: the estimate needs a solid", body.name)));
    }
    let model = s.model.state();
    let own = model.body(&body.name);
    let sheet_names = own.map(|own| solidcraft_doc::sheet::body_sheet_face_names(&model, own)).unwrap_or_default();
    let sheet_t = model.sheets.iter().find(|x| x.body == body.name && !x.flat).map(|x| x.t);
    let mesh = body.mesh();
    let positions: Vec<P> = mesh.positions.iter().map(|v| [v.x, v.y, v.z]).collect();
    let normals: Vec<P> = mesh.normals.iter().map(|v| [v.x, v.y, v.z]).collect();
    let tol = (body.body.size() * 2e-3).max(1e-3);
    let mut centroids = vec![[0.0; 3]; body.body.face_count()];
    let mut face_normals = vec![None; body.body.face_count()];
    let mut area = 0.0;
    for f in body.body.faces(tol)? {
        area += f.area;
        if let Some(c) = centroids.get_mut(f.index) {
            *c = [f.centroid.x, f.centroid.y, f.centroid.z];
        }
        if let Some(n) = face_normals.get_mut(f.index) {
            *n = f.plane_normal.map(|v| [v.x, v.y, v.z]);
        }
    }
    let faces = BodyFaces {
        names: solidcraft_doc::naming::face_names(body).to_vec(),
        sheet: sheet_names,
        spots: solidcraft_doc::naming::face_spots(body),
        centroids,
        normals: face_normals,
        count: body.body.face_count(),
        display: Faces { positions: &positions, triangles: &mesh.triangles, tri_face: &mesh.tri_face },
        vertex_normals: &normals,
    };
    let plan = plan(p, &faces, body, sheet_t, &s.doc.body_material(&body.name))?;
    let (lo, hi) = positions.iter().fold(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), |(mut lo, mut hi), p| {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
        (lo, hi)
    });
    let extent = fea::model::sub(hi, lo);
    let diag = fea::model::len(extent);
    if !(diag.is_finite() && diag > 0.0) {
        return Err(bad(CMD, format!("`{}` has nothing to mesh", body.name)));
    }
    let smallest = extent.iter().copied().fold(f64::INFINITY, f64::min).max(1e-3);
    // A probe off the part is refused before anything runs; one inside its box is read in the
    // solve, or named in a warning when no element is within reach of it.
    let reach = fea::probe::REACH;
    if let Some(pr) = plan.probes.iter().find(|pr| (0..3).any(|k| pr.at[k] < lo[k] - reach || pr.at[k] > hi[k] + reach)) {
        return Err(bad(
            CMD,
            format!(
                "{}: {} is outside the part, which spans {} to {} mm",
                pr.name,
                point_words(pr.at),
                point_words(lo.map(|x| round_to(x, 3))),
                point_words(hi.map(|x| round_to(x, 3)))
            ),
        ));
    }
    // For the distance away from supports and loads (`section_size`); only solids that are not
    // sheets measure it.
    let volume = if plan.sheet || !plan.runs.contains(&Elements::Solid) { None } else { body.body.volume_at(tol) };
    // Supports that leave the part free to slide or turn: refused before anything is meshed. The
    // solver does not fail on them; the part drifts and the numbers mean nothing.
    if let Some(why) = freedom_words(&fea::model::free_motions(&support_holds(&faces.display, &plan.supports), diag)) {
        return Err(bad(CMD, why));
    }
    let mut warnings = Vec::new();
    // Shells model a thin sheet: a part as thick as a tenth of its size bends less than they say.
    if plan.runs.contains(&Elements::Shell) && !plan.sheet && plan.thickness > 0.1 * diag {
        warnings.push(format!(
            "The body is not a thin sheet ({} mm thick for a {} mm part): shells read it as thinner than it behaves, so their stress and \
             deflection run high or low; solid elements suit it better",
            sig(plan.thickness),
            sig(diag)
        ));
    }
    // The thickness asked for against the sheet's own (PIL-294 D11).
    if let Some(own) = sheet_t
        && p.get("thickness").is_some()
        && let Some(w) = thickness_mismatch(plan.thickness, own, plan.runs.contains(&Elements::Solid))
    {
        warnings.push(w);
    }
    // A grade's yield holds up to a thickness (PIL-294 D10). A solid's stock was at least as thick
    // as its bounding box's thinnest side.
    let stock = if plan.sheet || !plan.runs.contains(&Elements::Solid) { plan.thickness } else { smallest };
    warnings.extend(grade_thickness_warning(&plan.material, stock, plan.sheet || !plan.runs.contains(&Elements::Solid)));
    // What the runs will take, from the part's size (A13): on a dry run for the approval card,
    // and again in each run once it has counted its nodes.
    let sized: Vec<(Elements, f64, f64)> = plan
        .runs
        .iter()
        .map(|e| {
            (
                *e,
                run_size(plan.mesh_size, *e, plan.sheet, plan.thickness, diag, smallest, volume),
                section_size(*e, plan.sheet, plan.thickness, volume, area),
            )
        })
        .collect();
    let planned = forecast::planned(&sized, plan.mesh_check, volume, area, plan.thickness);
    if p.get("dry_run").and_then(Value::as_bool).unwrap_or(false) {
        // Drawn on the model until the estimate runs or is declined: the user sees the faces.
        let (loads, supports) = markers_of(&plan, &faces);
        fea::show_preview(fea::Preview {
            document_id: s.document_id.clone(),
            body: body.name.clone(),
            fingerprint: fea::fingerprint(body),
            size: diag,
            loads,
            supports,
        });
        return json_of(
            CMD,
            StressEstimateResult {
                estimate: true,
                body: plan.body.clone(),
                setup: plan.setup,
                summary: "Not solved: a dry run checks the setup and describes it.".into(),
                runs: Vec::new(),
                fraction_of_yield: 0.0,
                fraction_of_yield_overall: 0.0,
                band_percent: None,
                shell_vs_solid: None,
                peak_at_support: None,
                peak_away: None,
                equilibrium: None,
                mesh_check: None,
                probes: Vec::new(),
                warnings,
                assumptions: Vec::new(),
                how_to_report: HOW_TO_REPORT.iter().map(|x| x.to_string()).collect(),
                network: String::new(),
                programs: None,
                predicted: predicted(&planned),
                folder: None,
                param_warnings: Vec::new(),
            },
        );
    }

    // It runs now: the result's markers take the preview's place.
    fea::clear_preview(&s.document_id);
    let marks = markers_of(&plan, &faces);

    // The body as the mesher reads it.
    let step = solidcraft_io::export_with_parts(
        &s.doc.painted(&st),
        std::slice::from_ref(&body.name),
        &[],
        Format::Step,
        &body.name,
        &s.doc.metadata,
        solidcraft_io::MeshQuality::default(),
    )?;
    let dir = fea::work_folder().map_err(EngineError::Other)?;
    let generation = fea::cancel_generation();
    let failed = |e: String| -> EngineError {
        fea::set_progress(None);
        let stopped = e == fea::STOPPED;
        if stopped || !plan.keep_files {
            let _ = std::fs::remove_dir_all(&dir);
        }
        if stopped {
            EngineError::Other(e)
        } else if plan.keep_files {
            EngineError::Other(format!("{CMD}: {e} (its files are in {})", dir.display()))
        } else {
            EngineError::Other(format!("{CMD}: {e}"))
        }
    };
    // At most: the mesh check's second and third meshes.
    let total = plan.runs.len() + if plan.mesh_check { 2 } else { 0 };
    let mut done: Vec<fea::Run> = Vec::new();
    // The mesh check's finer runs, the second mesh and then the third.
    let mut finer: Vec<fea::Run> = Vec::new();
    let mut check = Check::Off;
    let mut k = 0usize;
    // Half the memory free when the estimate starts (D1).
    let run_cap = first_run_limit(memory_available_here());
    let tracker = forecast::Tracker::new(&planned, s.document_id.clone(), &forecast::finer_fits);
    let fits = |nodes: usize, elements: Elements, h: f64| {
        tracker.counted(elements, h, nodes);
        memory_refusal(nodes as f64, elements, h, run_cap, true)
    };
    for (i, elements) in plan.runs.iter().enumerate() {
        let h = run_size(plan.mesh_size, *elements, plan.sheet, plan.thickness, diag, smallest, volume);
        // The finer run looks at the same region away from supports and loads as this one.
        let setup = |size: f64, level: u32| Setup {
            thickness: plan.thickness,
            mesh_size: size,
            material: plan.material.clone(),
            loads: plan.loads.clone(),
            supports: plan.supports.clone(),
            away_distance: away_distance(h, section_size(*elements, plan.sheet, plan.thickness, volume, area)),
            level,
        };
        // A solid's mesh is predicted from its volume before meshing, which takes time and memory
        // of its own at a size far too fine; every mesh is counted again before the solver runs.
        if *elements == Elements::Solid
            && let Some(v) = volume.filter(|v| v.is_finite() && *v > 0.0)
            && let Some(refusal) = memory_refusal(SOLID_NODES_PER_MM3 * v / h.powi(3), Elements::Solid, h, run_cap, false)
        {
            return Err(failed(refusal));
        }
        k += 1;
        let ctx = RunCtx { document_id: &s.document_id, run: k, runs: total, generation, started: 0.0, fits: Some(&fits) };
        let run = fea::solve(&setup(h, 0), &step, &faces.display, &dir, *elements, &ctx).map_err(|f| failed(f.words(|p| faces.near(p))))?;
        if i == 0 && plan.mesh_check {
            // Gated on the finer run's predicted size, not the first run's time: memory is what
            // runs out (R2's finer run took 14 GB), and a solid run that fits solves in about a
            // minute and a half (the solver's time grows as nodes^1.8: 36 s at 199 504 nodes,
            // 258 s at 595 186). Shells at 104 175 nodes take about 34 s in all (25 s solving;
            // `model::shell_model`'s face matching took about 300 s before it used a grid, P1).
            // Stop ends either at once.
            let limit = finer_limit_here();
            if let Some(skip) = mesh_check_gate(run.nodes, *elements, limit) {
                check = skip;
            } else {
                k += 1;
                let fine_h = h / MESH_CHECK_RATIO;
                let ctx = RunCtx { document_id: &s.document_id, run: k, runs: total, generation, started: 0.0, fits: Some(&fits) };
                let fine =
                    fea::solve(&setup(fine_h, 1), &step, &faces.display, &dir, *elements, &ctx).map_err(|f| failed(f.words(|p| faces.near(p))))?;
                // A third mesh when the lead peak moved more than 1 % (PIL-294 A6), predicted from
                // the second's nodes and counted again once meshed: over the limit, the bands
                // rest on two meshes.
                let mut third = match third_wanted(&settling(&[&run, &fine])) {
                    Ok(()) => match mesh_check_gate(fine.nodes, *elements, limit) {
                        Some(Check::Skipped { nodes, bytes, limit }) => Third::Skipped { nodes, bytes, limit },
                        _ => Third::Ran,
                    },
                    Err(why) => why,
                };
                if third == Third::Ran {
                    k += 1;
                    let counted = std::cell::Cell::new(None);
                    let third_fits = |nodes: usize, elements: Elements, h: f64| {
                        let n = nodes as f64;
                        let bytes = solver_bytes(n, elements);
                        let over = match limit {
                            Limit::Memory(cap) => bytes > cap,
                            Limit::Nodes(cap) => n > cap,
                        };
                        if over {
                            counted.set(Some((n, bytes)));
                            return Some("the third mesh is over the mesh check's limit".to_string());
                        }
                        fits(nodes, elements, h)
                    };
                    let ctx = RunCtx { document_id: &s.document_id, run: k, runs: total, generation, started: 0.0, fits: Some(&third_fits) };
                    match fea::solve(&setup(fine_h / MESH_CHECK_RATIO, 2), &step, &faces.display, &dir, *elements, &ctx) {
                        Ok(r) => finer.extend([fine, r]),
                        Err(f) => match counted.get() {
                            Some((nodes, bytes)) => {
                                third = Third::Skipped { nodes, bytes, limit };
                                finer.push(fine);
                            }
                            None => return Err(failed(f.words(|p| faces.near(p)))),
                        },
                    }
                } else {
                    finer.push(fine);
                }
                let mut meshes = vec![h];
                meshes.extend(finer.iter().map(|r| r.mesh_size));
                let series: Vec<&fea::Run> = std::iter::once(&run).chain(finer.iter()).collect();
                check = Check::Ran { meshes, items: settling(&series), third };
            }
        }
        done.push(run);
    }
    fea::set_progress(None);
    let Some(shown) = finer.last().or(done.first()) else { return Err(bad(CMD, "nothing was solved")) };
    let near = |at: P| faces.near(at);

    // Where the estimate says little.
    for r in finer.iter().chain(done.iter()) {
        if let Some(e) = &r.equilibrium
            && let Some(w) = equilibrium_warning(e)
        {
            warnings.push(format!("{w} ({}, mesh about {} mm)", r.elements.plain(plan.thickness), sig(r.mesh_size)));
        }
    }
    warnings.extend(check_warnings(&check, shown, &near));
    warnings.extend(yield_warning(shown.max_stress, &shown.peaks, plan.material.yield_mpa, &near));
    let span = if plan.sheet { (5.0 * plan.thickness).min(0.1 * diag) } else { 0.05 * diag };
    if shown.max_disp > span {
        warnings.push(format!(
            "The largest deflection ({} mm) is large for this part: a linear (small-deflection) estimate does not hold well at that size",
            sig(shown.max_disp)
        ));
    }

    // The probes, read in every solve (the finest first); the mesh check reads them on its meshes,
    // first to finest.
    let all: Vec<(&fea::Run, bool)> = finer.iter().rev().map(|r| (r, true)).chain(done.iter().map(|r| (r, false))).collect();
    let series: Vec<&fea::Run> = match (&check, done.first()) {
        (Check::Ran { .. }, Some(first)) => std::iter::once(first).chain(finer.iter()).collect(),
        _ => Vec::new(),
    };
    let mut probes = Vec::new();
    for pr in &plan.probes {
        match probe_reading(pr, shown, &all, &series, &plan.material, &near) {
            Ok(r) => probes.push(r),
            Err(w) => warnings.push(w),
        }
    }

    let outcome = Outcome { done: &done, shown, check: &check };
    let (mut summary, assumptions, shell_vs_solid) = wording(&plan.material, &plan.loads, &plan.supports, &outcome, plan.thickness, &near);
    if !probes.is_empty() {
        if probes.len() <= 3 {
            let lines: Vec<&str> = probes.iter().map(|r| r.line.as_str()).collect();
            summary = format!("{summary} {}", lines.join(" "));
        } else {
            summary = format!("{summary} {} probes are read in `probes`.", probes.len());
        }
    }
    let run_json: Vec<StressRun> = finer
        .iter()
        .rev()
        .map(|r| (r, true))
        .chain(done.iter().map(|r| (r, false)))
        .map(|(r, fine)| StressRun {
            elements: r.elements.word().into(),
            element_type: r.elements.described(plan.thickness),
            mesh_check_run: fine,
            mesh_size_mm: round_to(r.mesh_size, 3),
            nodes: r.nodes,
            element_count: r.element_count,
            max_von_mises_mpa: round_to(r.max_stress, 2),
            max_stress_near: near(r.max_stress_at),
            max_stress_at_support: r.peaks.at.clone(),
            max_von_mises_away_mpa: r.peaks.away.map(|(v, _)| round_to(v, 2)),
            max_away_near: r.peaks.away.map(|(_, at)| near(at)),
            max_deflection_mm: round_to(r.max_disp, 4),
            max_deflection_near: near(r.max_disp_at),
            supports_carry_n: r.equilibrium.map(|e| round_to(fea::model::len(e.reaction), 4)),
            loads_applied_n: r.equilibrium.map(|e| round_to(fea::model::len(e.applied), 4)),
            seconds: round_to(r.seconds, 1),
            solver_threads: r.threads,
        })
        .collect();
    // The share of yield of the number the summary leads with: extrapolated, with its band, when
    // the mesh check gave one (PIL-294 A6, spec section 6 and decision 8).
    let (lead, lead_item) = lead_quoted(shown, &check);
    let fraction = lead / plan.material.yield_mpa;
    let band_percent = lead_item.and_then(|s| if s.at_support { None } else { s.band() }).map(|b| round_to(100.0 * b, 2));
    let isolated = done.iter().chain(finer.iter()).all(|r| r.isolated);
    let network = if isolated {
        "Meshed and solved on this computer, with the solver's networking switched off; the part was not uploaded".to_string()
    } else {
        "Meshed and solved on this computer; the part was not uploaded".to_string()
    };
    let folder = if plan.keep_files {
        Some(dir.to_string_lossy().to_string())
    } else {
        let _ = std::fs::remove_dir_all(&dir);
        None
    };
    let equilibrium = shown.equilibrium.map(|e| StressEquilibrium {
        supports_carry_n: round_to(fea::model::len(e.reaction), 4),
        loads_applied_n: round_to(fea::model::len(e.applied), 4),
        mismatch_percent: round_to(100.0 * e.mismatch(), 3),
        moment_mismatch_percent: e.moment.map(|m| round_to(100.0 * m.mismatch(), 3)),
        moments_about_mm: e.moment.map(|m| m.about.map(|c| round_to(c, 3) + 0.0)),
        supports: shown.reactions.iter().map(|r| support_json(r, diag)).collect(),
        line: equilibrium_line(&e),
    });
    let programs = Some(StressPrograms { mesher: shown.mesher_version.clone(), solver: shown.solver_version.clone() });
    let peak_away = shown.peaks.away.map(|(v, at)| {
        // The peak is a node of the run: its own tensor.
        let tensor = shown.points.iter().position(|x| x.0 == at).and_then(|i| shown.tensors.get(i));
        StressPeak {
            von_mises_mpa: round_to(v, 2),
            near: near(at),
            at_mm: at.map(|c| round_to(c, 3)),
            fraction_of_yield: round_to(v / plan.material.yield_mpa, 3),
            away_from_supports_mm: round_to(shown.peaks.distance, 3),
            principal_stress_mpa: tensor.map(|s| fea::probe::principal(s).0.map(|x| round_to(x, 2))),
            principal_strain_microstrain: tensor
                .map(|s| fea::probe::principal(&fea::probe::strain(s, plan.material.e_mpa, plan.material.nu)).0.map(|x| round_to(x * 1e6, 1))),
        }
    });
    let mesh_check = Some(mesh_check_json(&check));

    // Paint it, with what was applied on it.
    let (fpos, fnor, ftri, stress, disp) = fea::field(shown, &positions, &normals, &mesh.triangles, (shown.mesh_size * 0.75).max(0.3));
    let deform_scale = if shown.max_disp > 0.0 { (0.08 * diag / shown.max_disp).clamp(1.0, 1.0e6) } else { 1.0 };
    let heading = format!("{} — {}, {}", body.name, shown.elements.word(), plan.material.name);
    let mut notes_shown = assumptions.clone();
    notes_shown.extend(plan.notes.iter().cloned());
    fea::show(Field {
        document_id: s.document_id.clone(),
        body: body.name.clone(),
        fingerprint: fea::fingerprint(body),
        positions: fpos,
        normals: fnor,
        triangles: ftri,
        stress,
        disp,
        max_stress: shown.max_stress,
        max_stress_at: shown.max_stress_at,
        max_disp: shown.max_disp,
        max_disp_at: shown.max_disp_at,
        yield_mpa: plan.material.yield_mpa,
        scale: plan.scale,
        quantity: plan.quantity,
        hidden: false,
        deform_scale: round_to(deform_scale, 0).max(1.0),
        heading,
        assumptions: notes_shown,
        warnings: warnings.clone(),
        loads: marks.0,
        supports: marks.1,
        away_stress: if shown.peaks.at.is_some() { shown.peaks.away } else { None },
        equilibrium: shown.equilibrium.map(|e| equilibrium_line(&e)),
        mesh_check: Some(check_line(&check)),
    });
    json_of(
        CMD,
        StressEstimateResult {
            estimate: true,
            body: body.name.clone(),
            setup: plan.setup,
            summary,
            runs: run_json,
            fraction_of_yield: round_to(fraction, 3),
            fraction_of_yield_overall: round_to(shown.max_stress / plan.material.yield_mpa, 3),
            band_percent,
            shell_vs_solid,
            peak_at_support: shown.peaks.at.clone(),
            peak_away,
            equilibrium,
            mesh_check,
            probes,
            warnings,
            assumptions,
            how_to_report: HOW_TO_REPORT.iter().map(|x| x.to_string()).collect(),
            network,
            programs,
            predicted: None,
            folder,
            param_warnings: Vec::new(),
        },
    )
}

/// What the supports hold, from their faces' own geometry: every corner of the display triangles
/// on a support's faces, held along X, Y and Z (fixed) or along the roller's axis. The display
/// triangles are the body's, through its whole thickness, so a sheet held as shells is checked as
/// its solid is.
pub(crate) fn support_holds(display: &Faces, supports: &[Support]) -> Vec<Hold> {
    const AXES: [P; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut out = Vec::new();
    for s in supports {
        let mut seen = std::collections::BTreeSet::new();
        for (tri, face) in display.triangles.iter().zip(display.tri_face) {
            if !s.faces.contains(face) {
                continue;
            }
            for v in tri {
                let Some(at) = display.positions.get(*v as usize).copied() else { continue };
                if !seen.insert(*v) {
                    continue;
                }
                match s.kind {
                    SupportKind::Fixed => out.extend(AXES.iter().map(|along| Hold { at, along: *along })),
                    SupportKind::Roller(k) | SupportKind::Symmetry(k) => out.extend(AXES.get(k).map(|along| Hold { at, along: *along })),
                }
            }
        }
    }
    out
}

/// A direction in words: "along X", or "along a direction (0.71, 0.71, 0)".
fn along_words(d: P) -> String {
    for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
        if d[k].abs() > 0.995 {
            return format!("along {axis}");
        }
    }
    let short = |x: f64| {
        let s = format!("{x:.2}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" || s.is_empty() { "0".to_string() } else { s.to_string() }
    };
    format!("along a direction ({}, {}, {})", short(d[0]), short(d[1]), short(d[2]))
}

/// A coordinate for a sentence: about three significant figures, no trailing zeros, round-off
/// next to zero read as 0 (`size`: the part's size, mm).
fn coord(x: f64, size: f64) -> String {
    if x.abs() <= 1e-6 * size.max(1.0) {
        return "0".into();
    }
    let s = sig(x);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// Why the supports do not hold the part still, naming each free motion and what stops it;
/// `None` when they do.
pub(crate) fn freedom_words(f: &fea::model::Freedom) -> Option<String> {
    if f.held() {
        return None;
    }
    let size = f.turns.iter().flat_map(|(_, p)| p.iter().map(|x| x.abs())).fold(1.0, f64::max);
    let mut motions: Vec<String> = f.slides.iter().map(|d| format!("slide {}", along_words(*d))).collect();
    motions.extend(f.turns.iter().map(|(axis, at)| {
        format!("turn about an axis {} through ({}, {}, {}) mm", along_words(*axis), coord(at[0], size), coord(at[1], size), coord(at[2], size))
    }));
    if motions.is_empty() {
        // Round-off hid which motion is free; the check still refuses.
        motions.push("move".into());
    }
    let list = match motions.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    let mut fixes: Vec<String> = Vec::new();
    for d in &f.slides {
        if let Some(axis) = (0..3).find(|k| d[*k].abs() > 0.995).and_then(|k| ["X", "Y", "Z"].get(k)) {
            fixes.push(format!("a roller on a face facing along {axis} stops the slide along {axis}"));
        } else {
            fixes.push(format!("a support on a face facing {} stops that slide", along_words(*d).trim_start_matches("along ")));
        }
    }
    if !f.turns.is_empty() {
        fixes.push("a support on another face, away from the axis, stops the turn".into());
    }
    Some(format!(
        "the supports do not hold the part still: it can {list}. A part free to move has no stress to estimate (the solver would let it \
         drift rather than stop). Hold a face `fixed`, or add supports: {}. Nothing was meshed or solved",
        fixes.join("; ")
    ))
}

/// "Supports carry 10.0 N; loads applied 10.0 N", then whether their moments balance too, when
/// the solver gave them.
pub(crate) fn equilibrium_line(e: &fea::Equilibrium) -> String {
    let forces = format!("Supports carry {} N; loads applied {} N", sig(fea::model::len(e.reaction)), sig(fea::model::len(e.applied)));
    match e.moment {
        None => forces,
        Some(_) if e.moments_balance() => format!("{forces}; their moments balance too"),
        Some(m) => format!("{forces}; their moments are {}% apart", sig(100.0 * m.mismatch())),
    }
}

/// The warning when the reaction misses the loads by more than 1 % of them, or its moment misses
/// theirs by more than 1 % of the moments; `None` when both balance.
pub(crate) fn equilibrium_warning(e: &fea::Equilibrium) -> Option<String> {
    const WHY: &str = "A support or load may not have reached the mesh as set up, or the solve went wrong; do not read the numbers until \
                       that is found";
    if !e.forces_balance() {
        return Some(format!(
            "The supports carry {} N but the loads applied add up to {} N, {}% apart (more than {}%): the forces do not balance, so this \
             result is suspect. {WHY}",
            sig(fea::model::len(e.reaction)),
            sig(fea::model::len(e.applied)),
            sig(100.0 * e.mismatch()),
            sig(100.0 * fea::EQUILIBRIUM_TOLERANCE)
        ));
    }
    let m = e.moment.filter(|_| !e.moments_balance())?;
    Some(format!(
        "The supports' moment about the middle of the part ({} N·mm) does not cancel the loads' ({} N·mm): {}% of the moments apart (more \
         than {}%), so this result is suspect. {WHY}",
        sig(fea::model::len(m.reaction)),
        sig(fea::model::len(m.applied)),
        sig(100.0 * m.mismatch()),
        sig(100.0 * fea::EQUILIBRIUM_TOLERANCE)
    ))
}

/// A vector for a sentence: "(0, -720, 0)", a component under a ten-thousandth of the vector's
/// size read as 0.
fn vector_words(v: P) -> String {
    let size = fea::model::len(v);
    let c = |x: f64| if x.abs() <= 1e-4 * size { "0".to_string() } else { coord(x, 0.0) };
    format!("({}, {}, {})", c(v[0]), c(v[1]), c(v[2]))
}

/// One support's reaction in the reply. `size`: the part's size (mm), for reading its middle.
pub(crate) fn support_json(r: &fea::Reaction, size: f64) -> StressSupportReaction {
    let (f, m) = (fea::model::len(r.force), fea::model::len(r.moment));
    let at = format!("({}, {}, {})", coord(r.about[0], size), coord(r.about[1], size), coord(r.about[2], size));
    StressSupportReaction {
        support: r.label.clone(),
        force_n: r.force.map(|x| round_to(x, 4) + 0.0),
        force_size_n: round_to(f, 4),
        moment_nmm: r.moment.map(|x| round_to(x, 2) + 0.0),
        moment_size_nmm: round_to(m, 2),
        about_mm: r.about.map(|x| round_to(x, 3) + 0.0),
        line: format!(
            "{} carries {} N {} and a moment of {} N·mm {} about {at} mm",
            r.label,
            sig(f),
            vector_words(r.force),
            sig(m),
            vector_words(r.moment)
        ),
    }
}

/// The mesh check's quantities, as named in the reply.
pub(crate) const OVERALL: &str = "highest stress";
pub(crate) const AWAY: &str = "highest stress away from supports and loads";
pub(crate) const DEFLECTION: &str = "largest deflection";

/// One number on each mesh of the check (PIL-294 A6).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settling {
    pub quantity: &'static str,
    pub unit: &'static str,
    /// Its value on each mesh, first (coarsest) to finest: two or three.
    pub values: Vec<f64>,
    /// The overall peak sits at a support or load (on the finest mesh): an idealised hold
    /// concentrates stress without limit, so it is never reported as settled and has no band.
    pub at_support: bool,
}

/// A step's change, %, from `a` to `b`.
fn change_percent(a: f64, b: f64) -> f64 {
    if a.abs() > 1e-12 {
        100.0 * (b - a) / a.abs()
    } else if b.abs() > 1e-12 {
        100.0
    } else {
        0.0
    }
}

/// What a number that keeps rising or falling with the mesh is, in the reply's words.
pub(crate) const RISING: &str = "keeps rising: likely a sharp inside corner or an idealised support";
pub(crate) const FALLING: &str = "keeps falling with the mesh: it has not converged";

impl Settling {
    /// The finest mesh's value.
    pub(crate) fn finest(&self) -> f64 {
        self.values.last().copied().unwrap_or(f64::NAN)
    }
    /// Each step's change, % (first to second, then second to third).
    pub(crate) fn changes(&self) -> Vec<f64> {
        self.values.windows(2).map(|w| change_percent(w[0], w[1])).collect()
    }
    /// The first step's change, %: what decides whether a third mesh is solved.
    pub(crate) fn change(&self) -> f64 {
        self.changes().first().copied().unwrap_or(0.0)
    }
    /// What the meshes say about it, as if it were not at a support.
    pub(crate) fn raw_estimate(&self) -> Option<fea::convergence::Estimate> {
        fea::convergence::estimate(&self.values, MESH_CHECK_RATIO)
    }
    /// What the meshes say about it: `None` at a support or load, where it has no value to
    /// settle on.
    pub(crate) fn estimate(&self) -> Option<fea::convergence::Estimate> {
        if self.at_support { None } else { self.raw_estimate() }
    }
    /// It settled: within a 3 % band, at an order the extrapolation trusts.
    pub(crate) fn settled(&self) -> bool {
        self.estimate().is_some_and(|e| e.trend == fea::convergence::Trend::Settled)
    }
    /// It keeps rising (or falling) with the mesh: it has no band.
    pub(crate) fn unbounded(&self) -> bool {
        self.estimate().is_some_and(|e| matches!(e.trend, fea::convergence::Trend::Rising | fea::convergence::Trend::Falling))
    }
    /// The value to quote: the extrapolated one when three meshes allowed it, else the finest.
    pub(crate) fn quoted(&self) -> f64 {
        self.estimate().map_or_else(|| self.finest(), |e| e.value)
    }
    /// How far the quoted value may be from the converged one, as a share of it (0.02 is ±2 %).
    pub(crate) fn band(&self) -> Option<f64> {
        self.estimate().and_then(|e| e.band())
    }
    pub(crate) fn status(&self) -> &'static str {
        use fea::convergence::Trend;
        match self.estimate().map(|e| e.trend) {
            _ if self.at_support => "at a support or load: depends on the mesh",
            Some(Trend::Settled) => "settled",
            Some(Trend::Rising) => RISING,
            Some(Trend::Falling) => FALLING,
            Some(Trend::Changing) | None => "still changing",
        }
    }
    /// How the band was found, in words.
    pub(crate) fn basis(&self) -> String {
        use fea::convergence::Basis;
        let Some(e) = self.estimate() else { return "at a support or load: no band".into() };
        let order = e.order.map_or_else(String::new, |p| format!(", order {p:.1}"));
        let floor = format!("at least ±{} %", 100.0 * fea::convergence::MIN_BAND);
        match e.basis {
            Basis::TwoMeshes => {
                format!("two meshes: the change over (1.5 − 1), for an error that falls at least as fast as the element size; {floor}")
            }
            Basis::Extrapolated => {
                format!("three meshes, extrapolated (Richardson{order}): 1.25 times the distance from the finest mesh's value, {floor}")
            }
            Basis::Noise => format!("three meshes that moved under 0.25 % a step: twice the larger step, {floor}"),
            Basis::FastOrder => format!(
                "three meshes, the first too coarse to count (its step was over five times the next{order}): the two finer meshes' change \
                 over (1.5 − 1), {floor}"
            ),
            Basis::Oscillating => format!("three meshes that go up and down: the larger step, {floor}"),
            Basis::NoLimit => format!("three meshes: it keeps growing without settling{order}, so it has no band"),
        }
    }
    /// The number with its band, for sentences: "about 108 MPa, within ±1.2 %", or what it does
    /// when it has none.
    pub(crate) fn quote(&self) -> String {
        match (self.at_support, self.band()) {
            (true, _) => format!("{} {}, depending on the mesh", sig(self.finest()), self.unit),
            (false, Some(b)) => format!("about {} {}, within ±{}", sig(self.quoted()), self.unit, band_words(b)),
            (false, None) => format!("{} {} on the finest mesh, and it {}", sig(self.finest()), self.unit, self.status()),
        }
    }
}

/// A band in words: "1.2 %", at least "0.1 %".
pub(crate) fn band_words(band: f64) -> String {
    if !band.is_finite() {
        return "? %".into();
    }
    format!("{:.1} %", (100.0 * band).max(0.1))
}

/// Why the mesh check stopped at two meshes, or that it solved three.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Third {
    /// Solved.
    Ran,
    /// Not needed: the lead peak (`quantity`) moved by `moved` %, no more than
    /// [`THIRD_MESH_PERCENT`].
    NotNeeded { quantity: &'static str, moved: f64 },
    /// Not solved: every node lies near a support or load, whose peak a finer mesh never settles.
    AtSupport,
    /// Not solved: the third mesh would have (or has) about `nodes` nodes and need about `bytes`
    /// of memory, over `limit`.
    Skipped { nodes: f64, bytes: f64, limit: Limit },
}

/// What the mesh check did.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Check {
    /// Solved again with finer meshes: `meshes` are their element sizes, first to finest.
    Ran { meshes: Vec<f64>, items: Vec<Settling>, third: Third },
    /// Not run: the finer run would have about `nodes` nodes and need about `bytes` of memory,
    /// over `limit`.
    Skipped { nodes: f64, bytes: f64, limit: Limit },
    /// Not asked for (`mesh_check: false`).
    Off,
}

/// How large the mesh check's finer run may be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Limit {
    /// This computer's memory limit, bytes: the smaller of [`MEMORY_CEILING`] and half the memory
    /// available.
    Memory(f64),
    /// A node cap set by [`MAX_NODES_VAR`].
    Nodes(f64),
}

/// The memory available now (Linux `/proc/meminfo`'s MemAvailable), bytes.
pub(crate) fn mem_available(meminfo: &str) -> Option<f64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let mut words = line.split_whitespace().skip(1);
    let n: f64 = words.next()?.parse().ok()?;
    let unit = match words.next().map(str::to_ascii_lowercase).as_deref() {
        Some("kb") => 1024.0,
        Some("mb") => 1024.0 * 1024.0,
        Some("gb") => 1024.0 * 1024.0 * 1024.0,
        None | Some("b") => 1.0,
        Some(_) => return None,
    };
    let bytes = n * unit;
    (bytes.is_finite() && bytes > 0.0).then_some(bytes)
}

/// The finer run's limit: the node cap in `max_nodes` (the variable's text) when it is a whole
/// number above 0; otherwise the smaller of `ceiling` and half the memory `available`, bytes (the
/// ceiling alone where that is not known).
pub(crate) fn finer_limit(max_nodes: Option<&str>, available: Option<f64>, ceiling: f64) -> Limit {
    if let Some(n) = max_nodes.and_then(|v| v.trim().parse::<u64>().ok()).filter(|n| *n > 0) {
        return Limit::Nodes(n as f64);
    }
    Limit::Memory(available.map_or(ceiling, |a| ceiling.min(a / 2.0)))
}

/// The finer run's limit on this computer now.
pub(crate) fn finer_limit_here() -> Limit {
    let var = std::env::var(MAX_NODES_VAR).ok();
    finer_limit(var.as_deref(), memory_available_here(), MEMORY_CEILING)
}

/// What a control group (Linux cgroup v2: a container, or `systemd-run -p MemoryMax=`) leaves
/// this process, bytes: the least, over the group in `cgroup` (`/proc/self/cgroup`'s text) and
/// each group above it, of its `memory.max` less its `memory.current`, as `read` gives those
/// files' text for a group's path under `/sys/fs/cgroup`. `None` where no group sets a limit.
pub(crate) fn cgroup_room(cgroup: &str, read: impl Fn(&str, &str) -> Option<String>) -> Option<f64> {
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?.trim();
    let mut room: Option<f64> = None;
    let mut at = path.trim_end_matches('/').to_string();
    loop {
        let number = |file: &str| read(&at, file).and_then(|t| t.trim().parse::<f64>().ok());
        if let (Some(max), Some(used)) = (number("memory.max"), number("memory.current")) {
            let left = (max - used).max(0.0);
            room = Some(room.map_or(left, |r: f64| r.min(left)));
        }
        match at.rfind('/') {
            Some(i) if !at.is_empty() => at.truncate(i),
            _ => break,
        }
    }
    room
}

/// The memory this process may use now, bytes: Linux's MemAvailable, no more than its control
/// group leaves it ([`cgroup_room`]). `None` where it cannot be read (Windows, macOS).
pub(crate) fn memory_available_here() -> Option<f64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let free = std::fs::read_to_string("/proc/meminfo").ok().as_deref().and_then(mem_available);
    let group = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|c| cgroup_room(&c, |path, file| std::fs::read_to_string(format!("/sys/fs/cgroup{path}/{file}")).ok()));
    match (free, group) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// The memory a run may use where this computer's free memory cannot be read: half a 16 GB
/// laptop's.
pub(crate) const FIRST_RUN_FALLBACK: f64 = 8.0e9;

/// The most memory a run may need, bytes: half the memory available (`available`), or
/// [`FIRST_RUN_FALLBACK`] where that is not known. The mesh check's finer run has the smaller
/// [`finer_limit`] besides.
pub(crate) fn first_run_limit(available: Option<f64>) -> f64 {
    available.filter(|a| a.is_finite() && *a > 0.0).map_or(FIRST_RUN_FALLBACK, |a| a / 2.0)
}

/// The most nodes whose solve fits in `bytes` ([`solver_bytes`] turned round).
pub(crate) fn nodes_for_bytes(bytes: f64, elements: Elements) -> f64 {
    let (per_node, at, extra) = match elements {
        Elements::Solid => SOLID_MEMORY,
        Elements::Shell => SHELL_MEMORY,
    };
    (bytes.max(0.0) * at.powf(extra) / per_node).powf(1.0 / (1.0 + extra))
}

/// Whether a run of `nodes` nodes (the mid-surface's for shells) at `h` mm fits `cap` bytes: `None`
/// when it does, else the words that refuse it, with the predicted memory and a `mesh_size` that
/// fits. `meshed`: the count is the mesh's own; otherwise it is predicted from the part's volume
/// ([`SOLID_NODES_PER_MM3`], which can be 20 % high at fine sizes), and refused only when twice
/// over (PIL-294 D1: R2's plate at 0.7 mm was killed by the system after 115 s).
pub(crate) fn memory_refusal(nodes: f64, elements: Elements, h: f64, cap: f64, meshed: bool) -> Option<String> {
    let bytes = solver_bytes(nodes, elements);
    let over = if meshed { bytes > cap } else { bytes > 2.0 * cap };
    if !over || !(nodes.is_finite() && nodes > 0.0 && h.is_finite() && h > 0.0) {
        return None;
    }
    let fit = nodes_for_bytes(cap, elements);
    let dims = match elements {
        Elements::Solid => 3.0,
        Elements::Shell => 2.0,
    };
    // A tenth over, then up to two significant figures: a size that fits, not one at the edge.
    let size = h * (nodes / fit).powf(1.0 / dims) * 1.1;
    let k = 10f64.powf(size.log10().floor() - 1.0);
    let size = (size / k).ceil() * k;
    let has = if meshed { "has" } else { "would have" };
    let mm = |x: f64| format!("{}", (x * 1000.0).round() / 1000.0);
    Some(format!(
        "not solved: the {} mesh at about {} mm {has} about {} nodes, and the solver would need about {} of memory; this computer has about {} \
         to spare for one run (half the memory free now). Use a larger `mesh_size`, such as {} mm, or free some memory",
        elements.word(),
        mm(h),
        about_count(nodes),
        about_bytes(bytes),
        about_bytes(cap),
        mm(size)
    ))
}

/// The mesh check's finer run from the first's node count: its predicted nodes and memory, bytes.
pub(crate) fn predict_finer(first_nodes: usize, elements: Elements) -> (f64, f64) {
    let growth = match elements {
        Elements::Solid => SOLID_GROWTH,
        Elements::Shell => SHELL_GROWTH,
    };
    let nodes = first_nodes as f64 * growth;
    (nodes, solver_bytes(nodes, elements))
}

/// The solver's predicted peak memory for a run of `nodes` nodes, bytes ([`SOLID_MEMORY`]).
pub(crate) fn solver_bytes(nodes: f64, elements: Elements) -> f64 {
    let (per_node, at, extra) = match elements {
        Elements::Solid => SOLID_MEMORY,
        Elements::Shell => SHELL_MEMORY,
    };
    per_node * nodes * (nodes.max(0.0) / at).powf(extra)
}

/// Whether the mesh check's finer run fits `limit`: `None` when it does, else the skip.
pub(crate) fn mesh_check_gate(first_nodes: usize, elements: Elements, limit: Limit) -> Option<Check> {
    let (nodes, bytes) = predict_finer(first_nodes, elements);
    let over = match limit {
        Limit::Memory(cap) => bytes > cap,
        Limit::Nodes(cap) => nodes > cap,
    };
    over.then_some(Check::Skipped { nodes, bytes, limit })
}

/// A count to two significant figures, with thousands separated: "680,000".
fn about_count(n: f64) -> String {
    if !(n.is_finite() && n >= 0.0) {
        return "?".into();
    }
    let digits = if n >= 10.0 { n.log10().floor() as i32 - 1 } else { 0 };
    let k = 10f64.powi(digits.max(0));
    let rounded = ((n / k).round() * k) as u64;
    let text = rounded.to_string();
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        if i > 0 && (text.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Memory in words, two significant figures: "11 GB", "9.5 GB", "480 MB".
fn about_bytes(b: f64) -> String {
    let two = |x: f64| if x >= 9.95 { format!("{x:.0}") } else { format!("{x:.1}").trim_end_matches(".0").to_string() };
    if b >= 1.0e9 { format!("{} GB", two(b / 1.0e9)) } else { format!("{} MB", two((b / 1.0e6).max(0.1))) }
}

/// Why the mesh check was skipped, in plain words.
pub(crate) fn skip_reason(nodes: f64, bytes: f64, limit: Limit) -> String {
    match limit {
        Limit::Memory(cap) => format!(
            "the finer mesh would have about {} nodes (about {}), over this computer's limit of about {}",
            about_count(nodes),
            about_bytes(bytes),
            about_bytes(cap)
        ),
        Limit::Nodes(cap) => format!(
            "the finer mesh would have about {} nodes (about {}), over the limit of {} nodes set by {MAX_NODES_VAR}",
            about_count(nodes),
            about_bytes(bytes),
            about_count(cap)
        ),
    }
}

/// The overall peak, the peak away from supports and loads (when every run has one) and the
/// largest deflection, on each mesh of the check (`runs`, first to finest).
pub(crate) fn settling(runs: &[&fea::Run]) -> Vec<Settling> {
    let at_support = runs.last().is_some_and(|r| r.peaks.at.is_some());
    let mut out = vec![Settling { quantity: OVERALL, unit: "MPa", values: runs.iter().map(|r| r.max_stress).collect(), at_support }];
    if let Some(away) = runs.iter().map(|r| r.peaks.away.map(|(v, _)| v)).collect::<Option<Vec<f64>>>() {
        out.push(Settling { quantity: AWAY, unit: "MPa", values: away, at_support: false });
    }
    out.push(Settling { quantity: DEFLECTION, unit: "mm", values: runs.iter().map(|r| r.max_disp).collect(), at_support: false });
    out
}

/// The peak the third mesh is decided on, and the summary leads with: the one away from supports
/// and loads when there is one, else the overall peak.
pub(crate) fn lead_item(items: &[Settling]) -> Option<&Settling> {
    items.iter().find(|s| s.quantity == AWAY).or_else(|| items.iter().find(|s| s.quantity == OVERALL))
}

/// Whether the third mesh is wanted after the first two (`items` on two meshes): the lead peak
/// moved by more than [`THIRD_MESH_PERCENT`]. `Err` says why not. A peak at a support never
/// settles, so a third mesh would not help it: with no peak away from supports and loads, none is
/// solved.
pub(crate) fn third_wanted(items: &[Settling]) -> std::result::Result<(), Third> {
    match lead_item(items) {
        Some(s) if s.at_support => Err(Third::AtSupport),
        Some(s) if s.change().abs() > THIRD_MESH_PERCENT => Ok(()),
        Some(s) => Err(Third::NotNeeded { quantity: s.quantity, moved: s.change() }),
        None => Err(Third::AtSupport),
    }
}

/// One number of the mesh check in words: "highest stress away from supports and loads 97.8 →
/// 102 → 104 MPa: about 108 MPa, within ±1.2 % (settled)".
fn settling_words(s: &Settling) -> String {
    let values: Vec<String> = s.values.iter().map(|v| sig(*v)).collect();
    let trail = match (s.at_support, s.band()) {
        (true, _) => format!(" ({})", s.status()),
        (false, Some(_)) => format!(": {} ({})", s.quote(), s.status()),
        (false, None) => format!(" ({})", s.status()),
    };
    format!("{} {} {}{trail}", s.quantity, values.join(" → "), s.unit)
}

/// Why the third mesh was not solved, in words; `None` when it was.
pub(crate) fn third_words(third: &Third, finest_h: f64) -> Option<String> {
    match third {
        Third::Ran => None,
        Third::NotNeeded { quantity, moved } => {
            Some(format!("no third mesh was needed: the {quantity} moved {moved:+.1}%, within {THIRD_MESH_PERCENT}%"))
        }
        Third::AtSupport => Some(
            "no third mesh was solved: the highest stress sits at a support or load, which a finer mesh does not settle, and no node lies \
             away from them"
                .to_string(),
        ),
        Third::Skipped { nodes, bytes, limit } => Some(format!(
            "the third mesh ({} mm) was not solved, so the bands rest on two meshes: {}",
            sig(finest_h / MESH_CHECK_RATIO),
            skip_reason(*nodes, *bytes, *limit).replacen("the finer mesh", "it", 1)
        )),
    }
}

/// The mesh check in one line, for the legend and the reply.
pub(crate) fn check_line(check: &Check) -> String {
    match check {
        Check::Ran { meshes, items, third } => {
            let sizes: Vec<String> = meshes.iter().map(|h| sig(*h)).collect();
            let parts: Vec<String> = items.iter().map(settling_words).collect();
            let finest = meshes.last().copied().unwrap_or(f64::NAN);
            let why = third_words(third, finest).map_or_else(String::new, |w| format!("; {w}"));
            format!("meshes {} mm: {}{why}", sizes.join(" → "), parts.join("; "))
        }
        Check::Skipped { nodes, bytes, limit } => {
            format!("skipped: {}, so how far the numbers are from settling is unknown", skip_reason(*nodes, *bytes, *limit))
        }
        Check::Off => "off (mesh_check: false), so how far the numbers are from settling is unknown".to_string(),
    }
}

fn settling_json(s: &Settling) -> StressSettling {
    let e = s.estimate();
    StressSettling {
        quantity: s.quantity.into(),
        unit: s.unit.into(),
        values: s.values.iter().map(|v| round_to(*v, 4)).collect(),
        changes_percent: s.changes().iter().map(|c| round_to(*c, 2)).collect(),
        status: s.status().into(),
        estimate: e.as_ref().filter(|e| e.error.is_some()).map(|e| round_to(e.value, 4)),
        band_percent: s.band().map(|b| round_to(100.0 * b, 2)),
        observed_order: e.as_ref().and_then(|e| e.order).map(|p| round_to(p, 2)),
        extrapolated: e.as_ref().is_some_and(|e| e.extrapolated()),
        basis: s.basis(),
        quote: s.quote(),
    }
}

/// What a probe's mesh check compares.
pub(crate) const PROBE_QUANTITY: &str = "von Mises stress at the probe";

/// A probe read in the shown run, with its von Mises stress in every run (`all`: each run and
/// whether it is one of the mesh check's finer ones) and, when the mesh check ran (`series`: its
/// meshes' runs, first to finest), its band. `Err` is the warning when the shown run cannot read the
/// point.
pub(crate) fn probe_reading(
    pr: &Probe,
    shown: &fea::Run,
    all: &[(&fea::Run, bool)],
    series: &[&fea::Run],
    material: &Material,
    near: &dyn Fn(P) -> String,
) -> std::result::Result<StressProbe, String> {
    let read = |r: &fea::Run| fea::probe::sample(&r.cells, &r.points, &r.tensors, pr.at);
    let s = read(shown).map_err(|miss| {
        let why = match miss {
            fea::probe::Miss::Outside(d) => {
                format!("the part's nearest element is {} mm from it, and a point is read up to {} mm outside the part", sig(d), fea::probe::REACH)
            }
            fea::probe::Miss::NoElements => "the solver's results have no elements to read it in".to_string(),
        };
        format!("{} at {} mm was not read: {why}", pr.name, point_words(pr.at))
    })?;
    let vm = fea::formats::von_mises(&s.stress);
    let (principal, directions) = fea::probe::principal(&s.stress);
    let strain = fea::probe::strain(&s.stress, material.e_mpa, material.nu);
    let (principal_strain, _) = fea::probe::principal(&strain);
    // `+ 0.0` reads a rounded −0 as 0.
    let micro = |x: f64| round_to(x * 1e6, 1) + 0.0;
    let mpa = |x: f64| round_to(x, 2) + 0.0;
    let settling = (series.len() >= 2)
        .then(|| series.iter().map(|r| read(r).ok().map(|x| fea::formats::von_mises(&x.stress))).collect::<Option<Vec<f64>>>())
        .flatten()
        .map(|values| Settling { quantity: PROBE_QUANTITY, unit: "MPa", values, at_support: false });
    let runs = all
        .iter()
        .filter_map(|(r, fine)| {
            let x = read(r).ok()?;
            Some(StressProbeRun {
                elements: r.elements.word().into(),
                mesh_size_mm: round_to(r.mesh_size, 3),
                mesh_check_run: *fine,
                von_mises_mpa: mpa(fea::formats::von_mises(&x.stress)),
            })
        })
        .collect();
    let along = pr.direction.map(|d| StressAlong {
        direction: d.map(|c| round_to(c, 4)),
        stress_mpa: mpa(fea::probe::along(&s.stress, d)),
        strain_microstrain: micro(fea::probe::along(&strain, d)),
    });
    let near_words = near(pr.at);
    let check_words = settling.as_ref().map_or_else(
        || "mesh check: not run, so how far it is from settling is unknown".to_string(),
        |st| match st.band() {
            Some(_) => format!("mesh check: {}, {}", st.status(), st.quote()),
            None => format!("mesh check: it {}", st.status()),
        },
    );
    let gauge = match (&along, pr.direction) {
        (Some(a), Some(d)) => format!("; {}: {} MPa, {} µε", along_words(d), sig(a.stress_mpa), sig(a.strain_microstrain)),
        _ => String::new(),
    };
    let line = format!(
        "At {} ({} mm, near {near_words}): von Mises {} MPa ({:.0}% of the yield strength given); principal stresses {}, {}, {} MPa; \
         principal strains {}, {}, {} µε{gauge}; {check_words}.",
        pr.name,
        point_words(pr.at),
        sig(vm),
        100.0 * vm / material.yield_mpa,
        sig(principal[0]),
        sig(principal[1]),
        sig(principal[2]),
        sig(principal_strain[0] * 1e6),
        sig(principal_strain[1] * 1e6),
        sig(principal_strain[2] * 1e6),
    );
    Ok(StressProbe {
        name: pr.name.clone(),
        at_mm: pr.at,
        outside_part_mm: (s.outside > 1e-3).then(|| round_to(s.outside, 3)),
        near: near_words,
        von_mises_mpa: mpa(vm),
        fraction_of_yield: round_to(vm / material.yield_mpa, 3),
        stress_mpa: StressComponents {
            xx: mpa(s.stress[0]),
            yy: mpa(s.stress[1]),
            zz: mpa(s.stress[2]),
            xy: mpa(s.stress[3]),
            yz: mpa(s.stress[4]),
            zx: mpa(s.stress[5]),
        },
        principal_stress_mpa: principal.map(mpa),
        principal_directions: directions.map(|d| d.map(|c| round_to(c, 4))),
        strain_microstrain: StrainComponents {
            xx: micro(strain[0]),
            yy: micro(strain[1]),
            zz: micro(strain[2]),
            gamma_xy: micro(2.0 * strain[3]),
            gamma_yz: micro(2.0 * strain[4]),
            gamma_zx: micro(2.0 * strain[5]),
        },
        principal_strain_microstrain: principal_strain.map(micro),
        along,
        displacement_mm: s.disp.map(|c| round_to(c, 5)),
        mesh_check: settling.as_ref().map(settling_json),
        runs,
        line,
    })
}

fn mesh_check_json(check: &Check) -> StressMeshCheck {
    let line = check_line(check);
    match check {
        Check::Ran { meshes, items, third } => StressMeshCheck {
            status: "ran".into(),
            reason: None,
            meshes_mm: meshes.iter().map(|h| round_to(*h, 3)).collect(),
            third_mesh: third_words(third, meshes.last().copied().unwrap_or(f64::NAN)),
            quantities: items.iter().map(settling_json).collect(),
            line,
        },
        Check::Skipped { nodes, bytes, limit } => StressMeshCheck {
            status: "skipped".into(),
            reason: Some(skip_reason(*nodes, *bytes, *limit)),
            meshes_mm: Vec::new(),
            third_mesh: None,
            quantities: Vec::new(),
            line,
        },
        Check::Off => StressMeshCheck {
            status: "off".into(),
            reason: Some("mesh_check: false".into()),
            meshes_mm: Vec::new(),
            third_mesh: None,
            quantities: Vec::new(),
            line,
        },
    }
}

/// A number's values over the mesh check, in words: "97.8 → 102 → 104 MPa".
fn values_words(s: &Settling) -> String {
    let values: Vec<String> = s.values.iter().map(|v| sig(*v)).collect();
    format!("{} {}", values.join(" → "), s.unit)
}

/// How many meshes a number rests on, in words.
fn meshes_words(s: &Settling) -> &'static str {
    if s.values.len() >= 3 { "over three meshes" } else { "over two meshes" }
}

/// What the mesh check means for reading the numbers. `shown` is the finest run.
pub(crate) fn check_warnings(check: &Check, shown: &fea::Run, face_label: &dyn Fn(P) -> String) -> Vec<String> {
    let Check::Ran { items, .. } = check else { return Vec::new() };
    let item = |q: &str| items.iter().find(|s| s.quantity == q);
    let mut out = Vec::new();
    let away = item(AWAY);
    // The peak at a support moved too much to call it settled had it been anywhere else, and the
    // stress away from it settled: say which to lead with.
    if let (Some(o), Some(at), Some(a)) = (item(OVERALL), shown.peaks.at.as_deref(), away)
        && !o.raw_estimate().is_some_and(|e| e.trend == fea::convergence::Trend::Settled)
        && a.settled()
    {
        let away_at = shown.peaks.away.map_or_else(String::new, |(_, p)| format!(", near {}", face_label(p)));
        out.push(format!(
            "The value at {at} depends on the mesh: the highest stress there went {} as the mesh was refined. An idealised support or \
             point load concentrates stress without limit, so that number keeps growing as the mesh is refined and says nothing about the \
             part. Lead with the stress away from it: {}{away_at}, which settled",
            values_words(o),
            a.quote()
        ));
    }
    let at = |p: Option<(f64, P)>| p.map_or_else(String::new, |(_, p)| format!(" near {}", face_label(p)));
    if let Some(a) = away
        && !a.settled()
    {
        let where_ = at(shown.peaks.away);
        out.push(if a.unbounded() {
            format!(
                "The highest stress away from supports and loads{where_} keeps rising as the mesh is refined ({}), so it has no value to \
                 settle on: it is likely a sharp inside corner, where the stress grows without limit, or an idealised support. It cannot be \
                 read as the part's stress; a fillet there gives a value that settles",
                values_words(a)
            )
        } else {
            format!(
                "The highest stress away from supports and loads{where_} has not settled {} ({}): {}. It may be a sharp inside corner, where \
                 the stress keeps rising as the mesh is refined, or a fillet or hole the mesh is coarse for. It needs a finer mesh there (or, \
                 at a sharp corner, a fillet) before it can be read",
                meshes_words(a),
                values_words(a),
                a.quote()
            )
        });
    } else if away.is_none()
        && let Some(o) = item(OVERALL)
        && !o.at_support
        && !o.settled()
    {
        out.push(format!("The highest stress has not settled {} ({}): {}; read it as rough", meshes_words(o), values_words(o), o.quote()));
    }
    if let Some(d) = item(DEFLECTION)
        && !d.settled()
    {
        out.push(format!(
            "The largest deflection has not settled {} ({}): {}. The mesh is coarse for this part; read it as rough",
            meshes_words(d),
            values_words(d),
            d.quote()
        ));
    }
    out
}

/// What the reply's words are made from: the solves of the setup (first meshes), the one shown
/// (the finer mesh check run when it ran) and the mesh check.
pub(crate) struct Outcome<'a> {
    pub done: &'a [fea::Run],
    pub shown: &'a fea::Run,
    pub check: &'a Check,
}

/// The warning when the part is thicker than its grade's yield is specified for (PIL-294 D10):
/// thicker stock is specified lower, or not at all. `thickness`: a sheet's or shell's thickness
/// (`sheet`), or a solid's thinnest side across, the least its stock can have been. Only for a
/// grade from the table; numbers given with the request are the user's.
pub(crate) fn grade_thickness_warning(material: &Material, thickness: f64, sheet: bool) -> Option<String> {
    let g = fea::materials::GRADES.iter().find(|g| g.name == material.name && g.source == material.source)?;
    let limit = g.up_to_mm?;
    (thickness > limit * 1.001).then(|| {
        let part = if sheet { format!("The part is {} mm thick", sig(thickness)) } else { format!("The part is {} mm across its thinnest side", sig(thickness)) };
        format!(
            "{part}, but {}'s yield ({} MPa) is specified only up to {} mm: thicker material is specified lower or not at all, so the              yield used may be too high; take it from the material's certificate (`material` with `yield_strength` and its `source`)",
            g.name,
            sig(g.yield_mpa),
            limit
        )
    })
}

/// The warning when the `thickness` asked for is not the sheet's own (PIL-294 D11): more than 5 %
/// apart. `solid_run`: a solid run is also asked for, which meshes the part as drawn.
pub(crate) fn thickness_mismatch(given: f64, own: f64, solid_run: bool) -> Option<String> {
    if !(own > 0.0) || (given - own).abs() <= 0.05 * own {
        return None;
    }
    let solid = if solid_run {
        format!("; the solid run meshes the part as drawn, {} mm thick, so the two runs are of different parts", sig(own))
    } else {
        String::new()
    };
    Some(format!(
        "The thickness given ({} mm) is not this sheet's own ({} mm): the shells are of a {} mm sheet, not the part as drawn{solid}",
        sig(given),
        sig(own),
        sig(given)
    ))
}

/// The stress the summary leads with, and the one `fraction_of_yield` shares (PIL-294 D8): the
/// highest away from supports and loads when the highest overall sits at one of them (that peak
/// depends on the mesh, not the part), otherwise the highest overall.
pub(crate) fn lead_stress(max_stress: f64, peaks: &fea::Peaks) -> f64 {
    match (&peaks.at, peaks.away) {
        (Some(_), Some((away, _))) => away,
        _ => max_stress,
    }
}

/// The number the summary leads with and `fraction_of_yield` shares (PIL-294 D8 and A6): the peak
/// away from supports and loads when the highest sits at one, else the highest ([`lead_stress`]),
/// quoted as the mesh check gives it when it gives a band (extrapolated from three meshes, or the
/// finest mesh's with two), else the finest mesh's own value. With the mesh check's item for it.
pub(crate) fn lead_quoted<'a>(shown: &fea::Run, check: &'a Check) -> (f64, Option<&'a Settling>) {
    let finest = lead_stress(shown.max_stress, &shown.peaks);
    let Check::Ran { items, .. } = check else { return (finest, None) };
    let has_away = items.iter().any(|s| s.quantity == AWAY);
    let q = match (&shown.peaks.at, shown.peaks.away) {
        (Some(_), Some(_)) | (None, _) if has_away => AWAY,
        _ => OVERALL,
    };
    match items.iter().find(|s| s.quantity == q) {
        Some(s) if !s.at_support && s.band().is_some() => (s.quoted(), Some(s)),
        item => (finest, item),
    }
}

/// A number in a sentence with its band when the mesh check (`item`) gave one: "about 108 MPa
/// (within ±1.2 %)". One that keeps rising is given on the finest mesh (`value`) with what it does.
pub(crate) fn quoted_words(item: Option<&Settling>, value: f64, unit: &str) -> String {
    match item {
        Some(s) if !s.at_support && s.band().is_some() => {
            format!("about {} {unit} (within ±{})", sig(s.quoted()), band_words(s.band().unwrap_or(f64::NAN)))
        }
        Some(s) if s.unbounded() => format!("{} {unit} on the finest mesh, and it {}", sig(value), s.status()),
        _ => format!("about {} {unit}", sig(value)),
    }
}

/// The past-yield warning, naming which peak passed yield (PIL-294 D8). A peak at an idealised
/// support or load grows without limit as the mesh is refined, so on its own it is not read as
/// the part yielding.
pub(crate) fn yield_warning(max_stress: f64, peaks: &fea::Peaks, yield_mpa: f64, near: &dyn Fn(P) -> String) -> Option<String> {
    if max_stress <= yield_mpa {
        return None;
    }
    const PAST: &str = "past yield the material no longer behaves linearly, so this estimate does not hold there; the real part would \
                        yield and the stress would spread";
    Some(match (&peaks.at, peaks.away) {
        (Some(at), Some((away, _))) if away <= yield_mpa => format!(
            "The highest stress ({} MPa) is above the yield strength given ({} MPa) only at {at}, where an idealised support or load \
             concentrates stress without limit as the mesh is refined; away from it the highest is {} MPa, {}% of yield. A real clamp or \
             fixture spreads its load, though the material right at it may yield locally; this estimate cannot say how much",
            sig(max_stress),
            sig(yield_mpa),
            sig(away),
            sig(100.0 * away / yield_mpa)
        ),
        (Some(_), Some((away, away_at))) => format!(
            "Away from the supports and loads, the highest stress ({} MPa, near {}) is above the yield strength given ({} MPa): {PAST}",
            sig(away),
            near(away_at),
            sig(yield_mpa)
        ),
        (Some(at), None) => format!(
            "The highest stress ({} MPa) is above the yield strength given ({} MPa) at {at}, where an idealised support or load \
             concentrates stress; no node lies away from the supports and loads, so whether the part itself passes yield cannot be read \
             from this mesh",
            sig(max_stress),
            sig(yield_mpa)
        ),
        (None, _) => format!("The highest stress ({} MPa) is above the yield strength given ({} MPa): {PAST}", sig(max_stress), sig(yield_mpa)),
    })
}

/// The reply's words: a summary, every assumption, and the shell and solid side by side. They
/// say how far the stress is from the yield strength given; never pass or fail. When the highest
/// stress sits at a support or load, they lead with the highest stress away from them.
pub(crate) fn wording(
    material: &Material,
    loads: &[Load],
    supports: &[Support],
    outcome: &Outcome,
    thickness: f64,
    face_label: &dyn Fn(P) -> String,
) -> (String, Vec<String>, Option<String>) {
    let shown = outcome.shown;
    // A grade of the estimate's own table, or numbers that came with the request.
    let from_table = fea::materials::GRADES.iter().any(|g| g.name == material.name && g.source == material.source);
    let origin = if from_table { "the estimate's grade table (the standard's minimum yield)" } else { "values given with the request" };
    let mut assumptions = vec![
        format!(
            "Material: {} — E {} MPa, Poisson {}, yield {} MPa; {origin}, source: {} (not from the material library)",
            material.name, material.e_mpa, material.nu, material.yield_mpa, material.source
        ),
        "Units: mm, N, MPa".to_string(),
        "Linear elastic, small deflections, static load".to_string(),
    ];
    for r in outcome.done {
        assumptions.push(format!("Model: {}; mesh about {} mm ({} nodes)", r.elements.plain(thickness), sig(r.mesh_size), r.nodes));
    }
    if let Check::Ran { meshes, .. } = outcome.check {
        assumptions.push(format!(
            "Shown: the mesh check's finest mesh, about {} mm ({} nodes). The numbers quoted with a band are extrapolated from the meshes \
             (Richardson, at the rate they converge) where three meshes allow it, else the finest mesh's own; the colours are the finest \
             mesh's",
            sig(meshes.last().copied().unwrap_or(f64::NAN)),
            shown.nodes
        ));
    }
    for l in loads {
        assumptions.push(load_words(l));
    }
    for s in supports {
        assumptions.push(support_words(s));
    }
    assumptions.push(format!(
        "Away from supports and loads: at nodes farther than {} mm ({AWAY_ELEMENTS} elements of the first mesh, and at least \
         {AWAY_SECTIONS} times the part's thickness) from every fixed support's and force's node. A clamp or a force's edge concentrates \
         stress without limit in the elements touching it, and disturbs it for about one thickness around (Saint-Venant); a roller on a \
         flat face and a pressure (even over its whole face) do not, so they are not counted",
        sig(shown.peaks.distance)
    ));
    assumptions.push(format!("Mesh check: {}", check_line(outcome.check)));
    assumptions.push(match &shown.equilibrium {
        Some(e) => format!("Equilibrium: {}", equilibrium_line(e)),
        None => "Equilibrium: not checked (the solver gave no reaction totals)".to_string(),
    });
    if let (Some(mesher), Some(solver)) = (&shown.mesher_version, &shown.solver_version) {
        assumptions.push(format!(
            "Programs: {mesher} meshed and {solver} solved, each on one thread, so the same request gives the same numbers with these versions"
        ));
    }

    let check_items: &[Settling] = match outcome.check {
        Check::Ran { items, .. } => items,
        _ => &[],
    };
    let status_of = |q: &str| check_items.iter().find(|s| s.quantity == q);
    let deflection =
        format!("The largest deflection is {}, near {}.", quoted_words(status_of(DEFLECTION), shown.max_disp, "mm"), face_label(shown.max_disp_at));
    let (lead, lead_item) = lead_quoted(shown, outcome.check);
    let mut summary = match (&shown.peaks.at, shown.peaks.away) {
        (Some(at), Some((away, away_at))) => {
            // A value at an idealised support is never called settled: a clamped strip's crept up
            // 145 → 151 MPa from 1.5 to 0.35 mm, 0.4% over the first 1.5× step.
            let settled = match status_of(OVERALL) {
                Some(s) if s.raw_estimate().is_some_and(|e| e.trend == fea::convergence::Trend::Settled) => {
                    format!(
                        "; it went {} with the mesh, but a value at an idealised support or load can keep creeping up as the mesh is \
                         refined, so read it as depending on the mesh",
                        values_words(s)
                    )
                }
                Some(_) => "; it is still changing with the mesh, so it depends on the mesh and is not the part's".to_string(),
                None => String::new(),
            };
            format!(
                "Estimate ({}): away from the supports and loads, the highest von Mises stress is {}, near {}; that is about {}% of the \
                 yield strength given ({} MPa, {}). The highest overall, {} MPa on the finest mesh, sits at {at}, where an idealised support \
                 or load concentrates stress{settled}. {deflection}",
                shown.elements.plain(thickness),
                quoted_words(lead_item, away, "MPa"),
                face_label(away_at),
                sig(100.0 * lead / material.yield_mpa),
                sig(material.yield_mpa),
                material.name,
                sig(shown.max_stress),
            )
        }
        _ => format!(
            "Estimate ({}): the highest von Mises stress is {}, near {}; that is about {}% of the yield strength given ({} MPa, {}). \
             {deflection}",
            shown.elements.plain(thickness),
            quoted_words(lead_item, shown.max_stress, "MPa"),
            face_label(shown.max_stress_at),
            sig(100.0 * lead / material.yield_mpa),
            sig(material.yield_mpa),
            material.name,
        ),
    };
    summary.push_str(&format!(" Mesh check: {}.", check_line(outcome.check)));
    let shell_vs_solid = match outcome.done {
        [a, b] => Some(format!(
            "Shell vs solid: highest stress {} vs {} MPa (solid/shell {}), largest deflection {} vs {} mm (solid/shell {})",
            sig(a.max_stress),
            sig(b.max_stress),
            sig(b.max_stress / a.max_stress),
            sig(a.max_disp),
            sig(b.max_disp),
            sig(b.max_disp / a.max_disp),
        )),
        _ => None,
    };
    (summary, assumptions, shell_vs_solid)
}

fn show(s: &mut Session, p: &Value) -> Result<Value> {
    let id = s.document_id.clone();
    let scale = match p.get("scale").and_then(Value::as_str) {
        None => None,
        Some("yield") => Some(ScaleMode::Yield),
        Some("auto") => Some(ScaleMode::Auto),
        Some(other) => return Err(bad(SHOW, format!("`scale` must be yield or auto (not `{other}`)"))),
    };
    let Some(want) = p.get("show").and_then(Value::as_str) else {
        let Some(scale) = scale else { return Err(bad(SHOW, "give `show` (stress, deflection, hide or clear) or `scale` (yield or auto)")) };
        if !fea::update(&id, |f| f.scale = scale) {
            return Err(bad(SHOW, "there is no stress estimate for this design yet: run one (Inspect › Stress Estimate, or ask in the chat)"));
        }
        let shown = fea::shown(&id).map_or("stress", |f| if f.quantity == Quantity::Deflection { "deflection" } else { "stress" });
        return json_of(SHOW, StressShowResult { shown: shown.to_string(), param_warnings: Vec::new() });
    };
    if let Some(scale) = scale {
        fea::update(&id, |f| f.scale = scale);
    }
    let ok = match want {
        "stress" => fea::set_quantity(&id, Quantity::Stress),
        "deflection" => fea::set_quantity(&id, Quantity::Deflection),
        "hide" | "off" => fea::update(&id, |f| f.hidden = true),
        "clear" => {
            fea::clear(&id);
            fea::clear_preview(&id);
            true
        }
        other => return Err(bad(SHOW, format!("`show` must be stress, deflection, hide or clear (not `{other}`)"))),
    };
    if !ok {
        return Err(bad(SHOW, "there is no stress estimate for this design yet: run one (Inspect › Stress Estimate, or ask in the chat)"));
    }
    let shown = if want == "off" { "hide" } else { want };
    json_of(SHOW, StressShowResult { shown: shown.to_string(), param_warnings: Vec::new() })
}

/// The dry run's prediction, in numbers and words.
fn predicted(planned: &[forecast::Planned]) -> Option<StressPredicted> {
    let (nodes, bytes, seconds, runs) = forecast::before_meshing(planned, &forecast::finer_fits)?;
    let what = if runs == 1 { "one run".to_string() } else { format!("{runs} runs") };
    Some(StressPredicted {
        nodes: nodes.round() as u64,
        memory_gb: round_to(bytes / 1.0e9, 2),
        seconds: round_to(seconds, 0),
        runs,
        line: format!(
            "From the part's size, before meshing: about {} nodes, about {} of memory, and about {} for {what} to mesh and solve. Small \
             holes, fillets and bends can make the mesh several times larger; each run counts its nodes after meshing and refuses a \
             mesh this computer cannot solve",
            about_count(nodes),
            about_bytes(bytes),
            about_time(seconds)
        ),
    })
}

/// A time in words, as roughly as it is known: "8 s", "45 s", "2.5 min", "25 min", "1.7 h".
pub(crate) fn about_time(seconds: f64) -> String {
    if !(seconds.is_finite() && seconds >= 0.0) {
        return "?".into();
    }
    if seconds < 1.0 {
        return "under a second".into();
    }
    if seconds < 90.0 {
        let k = if seconds < 10.0 { 1.0 } else { 5.0 };
        return format!("{} s", ((seconds / k).round() * k).max(1.0));
    }
    let minutes = seconds / 60.0;
    if minutes < 90.0 {
        return if minutes < 10.0 { format!("{} min", (minutes * 2.0).round() / 2.0) } else { format!("{} min", minutes.round()) };
    }
    format!("{} h", (minutes / 6.0).round() / 10.0)
}

#[path = "stress_forecast.rs"]
mod forecast;

#[cfg(test)]
#[path = "stress_tests.rs"]
mod tests;
