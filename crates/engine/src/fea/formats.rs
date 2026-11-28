//! The files the stress estimate trades with the external mesher and solver: the mesh gmsh
//! writes (`.msh`, format 2.2, ASCII), the job CalculiX reads (`.inp`) and the results it writes
//! (`.frd`, ASCII). Everything read here comes from another program, so it is parsed as hostile
//! input: a malformed line is skipped or ends the read with an error, never a panic.

use std::collections::HashMap;
use std::fmt::Write as _;

/// A point in mm.
pub type P = [f64; 3];

/// A mesh as gmsh wrote it: nodes by their ids, and the elements by kind.
#[derive(Debug, Default)]
pub struct Msh {
    pub nodes: HashMap<u64, P>,
    /// Second-order triangles on surfaces: (surface tag, six node ids, corners first).
    pub tri6: Vec<(i64, [u64; 6])>,
    /// Second-order tetrahedra: ten node ids, corners first.
    pub tet10: Vec<[u64; 10]>,
}

/// Read a gmsh `.msh` file of format 2.2 (ASCII).
pub fn read_msh(text: &str) -> Result<Msh, String> {
    let mut out = Msh::default();
    let mut lines = text.lines();
    let mut saw_nodes = false;
    while let Some(line) = lines.next() {
        match line.trim() {
            "$MeshFormat" => {
                let v = lines.next().unwrap_or("");
                if !v.trim_start().starts_with('2') {
                    return Err(format!("the mesher wrote mesh format `{}`, not 2.2", v.trim()));
                }
            }
            "$Nodes" => {
                let n: usize = lines.next().and_then(|l| l.trim().parse().ok()).ok_or("the mesh's node count is not a number")?;
                for _ in 0..n.min(50_000_000) {
                    let l = lines.next().ok_or("the mesh file ends inside its nodes")?;
                    let f: Vec<&str> = l.split_whitespace().collect();
                    let (Some(id), Some(x), Some(y), Some(z)) = (f.first(), f.get(1), f.get(2), f.get(3)) else { continue };
                    let (Ok(id), Ok(x), Ok(y), Ok(z)) = (id.parse::<u64>(), x.parse::<f64>(), y.parse::<f64>(), z.parse::<f64>()) else { continue };
                    if x.is_finite() && y.is_finite() && z.is_finite() {
                        out.nodes.insert(id, [x, y, z]);
                    }
                }
                saw_nodes = true;
            }
            "$Elements" => {
                let n: usize = lines.next().and_then(|l| l.trim().parse().ok()).ok_or("the mesh's element count is not a number")?;
                for _ in 0..n.min(50_000_000) {
                    let l = lines.next().ok_or("the mesh file ends inside its elements")?;
                    let f: Vec<i64> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    let (Some(&kind), Some(&ntags)) = (f.get(1), f.get(2)) else { continue };
                    let Ok(ntags) = usize::try_from(ntags) else { continue };
                    // Tags: the physical group, then the elementary entity (the surface or volume).
                    let entity = if ntags >= 2 { f.get(4).copied() } else { f.get(3).copied() }.unwrap_or(0);
                    let nodes: Vec<u64> = f.iter().skip(3 + ntags).filter_map(|&x| u64::try_from(x).ok()).collect();
                    match kind {
                        9 => {
                            if let Ok(t) = <[u64; 6]>::try_from(nodes.as_slice()) {
                                out.tri6.push((entity, t));
                            }
                        }
                        11 => {
                            if let Ok(t) = <[u64; 10]>::try_from(nodes.as_slice()) {
                                out.tet10.push(t);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if !saw_nodes || out.nodes.is_empty() {
        return Err("the mesher wrote no nodes".into());
    }
    if out.tri6.is_empty() {
        return Err("the mesher wrote no second-order surface triangles".into());
    }
    Ok(out)
}

/// The material as the solver takes it.
pub struct InpMaterial {
    pub e_mpa: f64,
    pub nu: f64,
}

/// What the job's elements are.
pub enum InpElements<'a> {
    /// Six-node shells of this thickness (mm), node indices into the job's nodes.
    Shell { tris: &'a [[usize; 6]], thickness: f64 },
    /// Ten-node tetrahedra.
    Solid { tets: &'a [[usize; 10]] },
}

/// The held nodes: `fixed` lose every translation (and a shell's rotations, a clamp); `pinned`
/// lose every translation and keep a shell's rotations (a support over an area of a sheet: its
/// rows of held nodes stop it turning, and a clamp there gave the solver's reaction total a
/// force that is not there, PIL-294 D3); a `rollers` node loses only the translation along its
/// axis (0 x, 1 y, 2 z); an `untilted` shell node on a symmetry plane across that axis also loses
/// its turns about the other two (a solid's nodes have none, so there it is ignored).
#[derive(Default)]
pub struct InpHolds<'a> {
    pub fixed: &'a [usize],
    pub pinned: &'a [usize],
    pub rollers: &'a [(usize, usize)],
    pub untilted: &'a [(usize, usize)],
}

/// The node set whose reaction totals the job prints (`*NODE PRINT, TOTALS=ONLY`): every held node.
pub const HELD_SET: &str = "HELD";

/// The node sets of nodes held along x, y and z only.
const ROLLER_SETS: [&str; 3] = ["ROLLX", "ROLLY", "ROLLZ"];

/// The node sets of shell nodes on symmetry planes across x, y and z.
const SYMMETRY_SETS: [&str; 3] = ["SYMX", "SYMY", "SYMZ"];

/// A CalculiX job: linear static, mm, N, MPa. Nodes are numbered from 1 in the order given. The
/// job also prints the total reaction force on all held nodes to its `.dat` file (equilibrium).
pub fn write_inp(nodes: &[P], elements: &InpElements, material: &InpMaterial, holds: &InpHolds, forces: &[(usize, P)]) -> String {
    let mut s = String::with_capacity(64 * nodes.len());
    let _ = writeln!(s, "*HEADING\nIntcad stress estimate (linear static; mm, N, MPa)");
    let _ = writeln!(s, "*NODE, NSET=NALL");
    for (i, p) in nodes.iter().enumerate() {
        let _ = writeln!(s, "{}, {:.9e}, {:.9e}, {:.9e}", i + 1, p[0], p[1], p[2]);
    }
    match elements {
        InpElements::Shell { tris, .. } => {
            let _ = writeln!(s, "*ELEMENT, TYPE=S6, ELSET=EALL");
            for (i, t) in tris.iter().enumerate() {
                let ids: Vec<String> = t.iter().map(|n| (n + 1).to_string()).collect();
                let _ = writeln!(s, "{}, {}", i + 1, ids.join(", "));
            }
        }
        InpElements::Solid { tets } => {
            let _ = writeln!(s, "*ELEMENT, TYPE=C3D10, ELSET=EALL");
            for (i, t) in tets.iter().enumerate() {
                let ids: Vec<String> = t.iter().map(|n| (n + 1).to_string()).collect();
                // At most 16 entries a line: the id and the first nine nodes, then the tenth.
                let (head, tail) = ids.split_at(9.min(ids.len()));
                let _ = writeln!(s, "{}, {},\n{}", i + 1, head.join(", "), tail.join(", "));
            }
        }
    }
    let nset = |s: &mut String, name: &str, ids: &[usize]| {
        // The solver refuses an empty set.
        if ids.is_empty() {
            return;
        }
        let _ = writeln!(s, "*NSET, NSET={name}");
        for chunk in ids.chunks(12) {
            let ids: Vec<String> = chunk.iter().map(|n| (n + 1).to_string()).collect();
            let _ = writeln!(s, "{},", ids.join(", "));
        }
    };
    nset(&mut s, "FIXED", holds.fixed);
    // A node both clamped and pinned is clamped.
    let mut pinned: Vec<usize> = holds.pinned.iter().copied().filter(|n| !holds.fixed.contains(n)).collect();
    pinned.sort_unstable();
    pinned.dedup();
    nset(&mut s, "PINNED", &pinned);
    let mut by_axis: [Vec<usize>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (n, axis) in holds.rollers {
        if let Some(v) = by_axis.get_mut(*axis) {
            v.push(*n);
        }
    }
    for (v, name) in by_axis.iter_mut().zip(ROLLER_SETS) {
        v.sort_unstable();
        v.dedup();
        nset(&mut s, name, v);
    }
    let shell = matches!(elements, InpElements::Shell { .. });
    let mut untilted: [Vec<usize>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    if shell {
        for (n, axis) in holds.untilted {
            if let Some(v) = untilted.get_mut(*axis) {
                v.push(*n);
            }
        }
    }
    for (v, name) in untilted.iter_mut().zip(SYMMETRY_SETS) {
        v.sort_unstable();
        v.dedup();
        nset(&mut s, name, v);
    }
    let mut held: Vec<usize> = holds.fixed.iter().copied().chain(pinned.iter().copied()).chain(by_axis.iter().flatten().copied()).collect();
    held.sort_unstable();
    held.dedup();
    nset(&mut s, HELD_SET, &held);
    let _ = writeln!(s, "*MATERIAL, NAME=M1\n*ELASTIC\n{:.6e}, {:.6}", material.e_mpa, material.nu);
    match elements {
        InpElements::Shell { thickness, .. } => {
            let _ = writeln!(s, "*SHELL SECTION, ELSET=EALL, MATERIAL=M1\n{thickness:.9e}");
        }
        InpElements::Solid { .. } => {
            let _ = writeln!(s, "*SOLID SECTION, ELSET=EALL, MATERIAL=M1");
        }
    }
    // A held shell keeps its rotations too (a clamp); a solid's nodes have none.
    let last = if shell { 6 } else { 3 };
    let _ = writeln!(s, "*BOUNDARY");
    if !holds.fixed.is_empty() {
        let _ = writeln!(s, "FIXED, 1, {last}");
    }
    if !pinned.is_empty() {
        let _ = writeln!(s, "PINNED, 1, 3");
    }
    for (axis, (v, name)) in by_axis.iter().zip(ROLLER_SETS).enumerate() {
        if !v.is_empty() {
            let _ = writeln!(s, "{name}, {}, {}", axis + 1, axis + 1);
        }
    }
    // Turns are degrees of freedom 4, 5, 6 (about x, y, z): a plane across `axis` keeps the turns
    // about the other two axes.
    for (axis, (v, name)) in untilted.iter().zip(SYMMETRY_SETS).enumerate() {
        if !v.is_empty() {
            for other in (0..3).filter(|k| *k != axis) {
                let _ = writeln!(s, "{name}, {}, {}", other + 4, other + 4);
            }
        }
    }
    let _ = writeln!(s, "*STEP\n*STATIC\n*CLOAD");
    for (n, f) in forces {
        for (dof, v) in f.iter().enumerate() {
            if v.abs() > 0.0 {
                let _ = writeln!(s, "{}, {}, {:.9e}", n + 1, dof + 1, v);
            }
        }
    }
    if !held.is_empty() {
        let _ = writeln!(s, "*NODE PRINT, NSET={HELD_SET}, TOTALS=ONLY\nRF");
    }
    match elements {
        InpElements::Shell { .. } => {
            let _ = writeln!(s, "*NODE FILE, OUTPUT=3D\nU\n*EL FILE, OUTPUT=3D\nS");
        }
        InpElements::Solid { .. } => {
            let _ = writeln!(s, "*NODE FILE\nU\n*EL FILE\nS");
        }
    }
    let _ = writeln!(s, "*END STEP");
    s
}

/// The total reaction force (N) on node set `set` that the solver printed to its `.dat` file
/// (`*NODE PRINT, TOTALS=ONLY` with RF): a line `total force (fx,fy,fz) for set <SET> and time …`,
/// then, on the next line that is not empty, three numbers. The last such block counts (the end of
/// the step). `None` when there is none, or it does not read as three finite numbers.
pub fn read_dat_total(text: &str, set: &str) -> Option<P> {
    let wanted = format!("FOR SET {} ", set.trim().to_ascii_uppercase());
    let mut lines = text.lines();
    let mut found = None;
    while let Some(line) = lines.next() {
        let upper = line.to_ascii_uppercase();
        if !(upper.contains("TOTAL FORCE") && upper.contains(&wanted)) {
            continue;
        }
        let Some(values) = lines.by_ref().map(str::trim).find(|l| !l.is_empty()) else { break };
        let v: Vec<f64> = values.split_whitespace().map_while(|x| x.parse::<f64>().ok()).collect();
        found = match v[..] {
            [x, y, z] if [x, y, z].iter().all(|c| c.is_finite()) => Some([x, y, z]),
            _ => None,
        };
    }
    found
}

/// What the solver's results file holds: the nodes it reports on, with their displacement (mm)
/// and their stress tensor (MPa: xx, yy, zz, xy, yz, zx) where it gave them.
#[derive(Debug, Default)]
pub struct Frd {
    pub nodes: HashMap<u64, P>,
    pub disp: HashMap<u64, P>,
    pub stress: HashMap<u64, [f64; 6]>,
}

/// One data line of an `.frd` block: ` -1`, the node id, then values of 12 characters each. The
/// id's width is what is left once the values are counted (5 or 10 characters).
fn frd_values(line: &str, count: usize) -> Option<(u64, Vec<f64>)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let rest = line.strip_prefix(" -1")?;
    let width = rest.len().checked_sub(12 * count)?;
    if width == 0 || !rest.is_char_boundary(width) {
        return None;
    }
    let (id, values) = rest.split_at(width);
    let id = id.trim().parse().ok()?;
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let v: f64 = values.get(12 * k..12 * (k + 1))?.trim().parse().ok()?;
        if !v.is_finite() {
            return None;
        }
        out.push(v);
    }
    Some((id, out))
}

/// Read an `.frd` results file (ASCII): its nodes, the `DISP` block and the `STRESS` block.
pub fn read_frd(text: &str) -> Result<Frd, String> {
    let mut out = Frd::default();
    #[derive(Clone, Copy, PartialEq)]
    enum Block {
        None,
        Nodes,
        Disp,
        Stress,
        Other,
    }
    let mut block = Block::None;
    for line in text.lines() {
        if line.starts_with("    2C") {
            block = Block::Nodes;
            continue;
        }
        if line.starts_with(" -4") {
            let name = line.get(3..).unwrap_or("").split_whitespace().next().unwrap_or("");
            block = match name {
                "DISP" => Block::Disp,
                "STRESS" => Block::Stress,
                _ => Block::Other,
            };
            continue;
        }
        if line.starts_with(" -3") {
            block = Block::None;
            continue;
        }
        if !line.starts_with(" -1") {
            continue;
        }
        match block {
            Block::Nodes => {
                if let Some((id, v)) = frd_values(line, 3)
                    && let [x, y, z] = v[..]
                {
                    out.nodes.insert(id, [x, y, z]);
                }
            }
            Block::Disp => {
                if let Some((id, v)) = frd_values(line, 3)
                    && let [x, y, z] = v[..]
                {
                    out.disp.insert(id, [x, y, z]);
                }
            }
            Block::Stress => {
                if let Some((id, v)) = frd_values(line, 6)
                    && let [a, b, c, d, e, f] = v[..]
                {
                    out.stress.insert(id, [a, b, c, d, e, f]);
                }
            }
            Block::None | Block::Other => {}
        }
    }
    if out.nodes.is_empty() || out.disp.is_empty() || out.stress.is_empty() {
        return Err("the solver's results file has no nodes, displacements or stresses".into());
    }
    Ok(out)
}

/// The von Mises stress of a stress tensor (xx, yy, zz, xy, yz, zx).
pub fn von_mises(s: &[f64; 6]) -> f64 {
    let [xx, yy, zz, xy, yz, zx] = *s;
    (0.5 * ((xx - yy).powi(2) + (yy - zz).powi(2) + (zz - xx).powi(2)) + 3.0 * (xy * xy + yz * yz + zx * zx)).max(0.0).sqrt()
}

#[cfg(test)]
#[path = "formats_tests.rs"]
mod tests;
