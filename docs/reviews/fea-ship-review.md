# Stress estimate (PIL-294): ship review

Reviewed: `fea/5-agent-skill` at b805339c8 (the tip of PRs 169–173). Date: 2026-11-17. Reviewer: an agent session with no part in building it.
Method: a fresh copy built on zhikun (Linux, 24 cores, gmsh 4.12.1, CalculiX 2.21). Every case was driven through `intcad-cli mcp`, as an agent drives it. Results were checked against closed-form or published values, and against the solver's own `.frd` files read independently (`keep_files`).
Raw replies and scripts: the shared notes folder, `fea-ship-review-2026-11-17/fea-review-results.tgz`.

## 1. Verdict

| Question | Verdict |
|---|---|
| (a) Merge the stack into `develop` as an experimental feature | **Ship with caveats.** Merge, but fix D1–D3 (or hide the Stress button) before a release puts it in front of users. |
| (b) Engineers relying on it for real linear-elastic work | **Not yet.** |

**Where it works, the numbers are right.** Every case that meshed was within 2 % of its reference away from supports, and the deflections were within 3.3 %:
- Lamé thick cylinder: +0.3 %;
- Roark clamped plate: +0.6 % stress, −0.5 % deflection;
- NAFEMS LE1: +0.06 %;
- a Heywood hole: +0.3 %;
- a Peterson fillet: −1.2 % on a fine mesh.

**The guard rails work.** All 26 plainly bad setups I tried were refused, with plain messages that say how to fix the setup; the near misses that were accepted are D5, D6 and D11. The wording never slid into a verdict: no "safe", "pass", "fail", "OK", "margin" or "factor" appeared in any reply saved in this review (`how_to_report`, which names those words to forbid them, excluded).

**What blocks real use is robustness, not accuracy:**
- **Small holes:** ordinary parts with small holes fail to mesh at the default settings (5 of 10 plain test bars), and the user gets the solver's raw error text.
- **Memory:** nothing stops a first solve that needs more memory than the computer has.
- **Clamped sheet metal:** the most common setup, a bracket clamped flat on its base, comes back labelled "suspect, do not read the numbers" when the numbers are in fact correct.

**What engineers need next:** a bolt or bearing load on a hole, a probe for the stress at a point, and local mesh refinement. Section 4 ranks the full list.

## 2. Validation cases

Units: mm, N, MPa. "Away" is the reply's `peak_away`, the highest stress away from supports and loads. The mesh check solves again with a mesh 1.5× finer and calls a number settled when it moved 5 % or less. The last column says whether that verdict told the truth.

| # | Case (elements, mesh) | Reference | Intcad | Error | Mesh verdict | Verdict right? |
|---|---|---|---|---|---|---|
| R1 | Cantilever strip 100×20×1.5, solid, 1.5→1.0 | beam σ at x=5.37: 126.17; tip 8.429 | 126.71; 8.236 | +0.43 %; −2.30 % | away settled; clamp "depends on mesh" | yes |
| R2 | Plate 200×50×2, Ø10 hole, 800 N, solid 1.5 | Heywood Kt σnom 25.12 | 25.19 | +0.28 % | settled (−0.10 %) | yes |
| R3 | NAFEMS LE1, solid | σyy at D: 92.7 | 92.75 | +0.06 % | settled | yes |
| R4 | Strip as shells + 0.005 MPa | beam 59.46 at x=5.56; tip 3.161 | 59.49; 3.058 | +0.04 %; −3.24 % | settled | yes |
| R5 | Half strip, symmetry plane, shells | beam 126.01; whole strip | 127.42; tips equal | +1.12 %; 0.00 % | off | n/a |
| R6 | Strip, shell and solid | shell vs solid | 8.20 vs 8.23 mm | 0.4 % | settled | yes |
| N1 | **Lamé** quarter ring a=20, b=40, p=50, S355, solid (default 2.3 mm) | vM at bore 116.67; u(a) 9.365e-3 | 117.06; 9.40e-3 | +0.34 %; +0.4 % | settled (−0.04 %) | yes |
| N1f | Lamé at 1.0 mm | 116.67 | 116.83 | +0.14 % | settled | yes |
| N1p | Lamé hoop stress, 15 probes r = 20/30/40 mm (from `.frd`) | Lamé σθ(r) | — | all within ±1.5 % | — | — |
| N2 | **Peterson** stepped bar D/d 1.4, r/d 0.1, 4000 N, solid, default (2.0→1.33) | Kt 2.146 → 107.3 | 102.1 (first 97.8) | −4.8 % | **settled (+4.4 %)** | **misleading**: the shown value is 5–6 % low |
| N2f | Peterson at 0.8 mm (mesh check off) | 107.3 | 106.0 | −1.2 % | — | Richardson from 1.33/0.8: 108.2, which confirms the reference to <1 % |
| N3 | Same bar, **sharp** inside corner (singular) | none (no finite value) | 95.6 → 102.7 → 121.4 (2.0/1.33/0.8 mm) | — | **still changing (+7.4 %)**, with the warning "may be a sharp inside corner" | yes |
| N4 | **Roark** clamped circular plate a=50, t=2, q=0.1, solid default (1.0 mm, 157,900 nodes) | w 0.1909 (Mindlin); vM at r=46.2: 31.93 | 0.190; 32.12 | −0.5 %; +0.6 % | **skipped** (finer run predicted 11 GB) | unknown, by design; probes at r = 0/25/40 within 1.5 % |
| N4s | Same plate as shells, t=2 | w 0.1909; centre 31.17 | 0.1877; 30.93 | −1.7 %; −0.8 % | settled | yes |
| N5 | **Sheet-metal L bracket** 1.5 mm, base underside fixed, 20 N on wall top, shells | σ = 6F(z_top−z)/(bt²) at the away point: 31.32 | 30.24 | −3.5 % | settled (+2.0 %) | yes, but **equilibrium false alarm** (D3) |
| N5p | Same, mid-width wall probes at z = 9/19/29 | beam: 27.9/18.9/9.9; plate (×√(1−ν+ν²)=0.88): 24.7/16.7/8.8 | 25.9/17.0/9.1 | between beam and plate theory, as expected | — | — |
| N5s | Same bracket as solids (1.5 mm) | shells' tip uy 0.334–0.344 | 0.351; wall at z=20 17.08 vs shells 16.4–17.1 | ≤ 5 % | — | the shell solve itself is right |
| N6 | **Bolted** L bracket (two Ø6.6 hole walls fixed), shells | no closed form for the base | away 37.5 in the base; hole 58.8→65.7 | — | away settled; hole "depends on the mesh" (+11.7 %) | yes |
| N7 | **Inch design** (`document.units in`), strip 4×0.8×0.06 in, 10 N | beam 121.90 at its x; tip 8.465 | 123.83; 8.277 | +1.6 %; −2.2 % | settled | yes (faces and reply in mm throughout) |
| N8 | **STEP import**: N1 exported, reopened as a new design | N1 | 117.06 (identical) | +0.33 % | settled | yes |
| N9 | **Near free-floating**: 100×20×5 bar held only by a Ø1 or Ø0.2 hole wall | statics | — | — | — | could not run: the mesh failed (D2) |
| N10 | **Fine-mesh memory**: R2's plate at mesh 0.7 inside an 8 GB cap | should refuse or warn first | ran 115 s, then "ccx stopped with signal 9 … see gpl.htm" | — | — | D1 |

What the mesh verdict did:
- **Correct calls:** "settled" and "still changing" told the truth in 13 of the 14 checks that ran.
- **Misleading call:** one, at a fillet meshed at about the fillet radius. There the 5 % threshold on a single 1.5× step let a value that was 5–6 % low (non-conservative) be called settled.
- **Sharp corner:** the singular case was caught.
- **Clamp peaks:** a peak at a clamp was never called settled.

## 3. Blocking defects

**D1. Nothing checks the first solve's memory.** The memory gate covers only the mesh check's finer run.
- **Where:** `crates/engine/src/cmd/stress.rs:976` solves at once; the gate is at `stress.rs:984` (`mesh_check_gate`) and applies only to the finer run. The dry run (`stress.rs:885`) says nothing about size, and `mesh_size` down to 0.05 mm is accepted (`stress.rs:681`).
- **Repro:**
  1. Make R2's plate (200×50×2, Ø10 hole).
  2. Run `{"supports":[{"face":[0,25,1]}],"loads":[{"face":[200,25,1],"force":[800,0,0]}],"material":"6061-T6","mesh_size":0.7}`, about half a million nodes (gmsh alone, with curvature sizing: 493,468 in 9.8 s; ≈12 GB by the code's own formula), inside an 8 GB memory cap (`systemd-run --user -p MemoryMax=8G -p MemorySwapMax=0`).
  3. Result: after 115 s, `the solver ccx stopped with signal: 9 (SIGKILL): software, and you are welcome to redistribute it under | certain conditions, see gpl.htm`.
- **Default settings hit it too:** the default solid mesh is half the thinnest side, whatever the part's size (`stress.rs:963`).
  - A 100 mm × 2 mm disc gets 157,900 nodes, about 3 GB.
  - A 200 mm × 2 mm plate modelled as a plain solid would predict about 630,000 nodes and about 16 GB with no `mesh_size` given. This is a prediction from the code's formula; it was not run.
  - On a 16 GB laptop with no memory cap, that means swapping or the OOM killer.
- **Fix:** run `predict` on the first run before meshing, or read gmsh's node count before the solve. Refuse, or coarsen, above the same limit. Turn a killed solver into a plain message.

**D2. The default mesh fails on ordinary small holes, and the user gets raw errors.**
- **Where:** gmsh is called with only `-clmax` (`crates/engine/src/fea/mod.rs:597-609`): no sizing from curvature, no minimum size, no high-order optimisation. The default size ignores feature size (`stress.rs:963`). Errors are passed through as the programs' own text (`mod.rs:546`, `mod.rs:723-724`).
- **Repro:** a bar 100×20×T, one through-hole Ø D at x=50, clamped at x=0, 10 N at x=100, default mesh:

| T | D=1 | D=2 | D=3 | D=4 | D=6 |
|---|---|---|---|---|---|
| 5 | gmsh: "Invalid boundary mesh (overlapping facets)" | ok | ok | ok | ok |
| 10 | ccx: "nonpositive jacobian" | gmsh error | ccx: "nonpositive jacobian" | ccx: "nonpositive jacobian" | ok |

  - A Ø3.4 clearance hole for an M3 bolt in a 10 mm part is a normal part, and it fails.
  - The near-free case N9 failed the same way, at the default mesh and at 1.0 mm.
- **Fix:** add `-clcurv` / `Mesh.MeshSizeFromCurvature`, a minimum size tied to the smallest edge, and `-optimize_ho` (or `Mesh.HighOrderOptimize`). Map "nonpositive jacobian" and gmsh's mesh errors to "the mesh is too coarse for a small feature near …; try mesh_size …".

**D3. Shells with a clamp over an area report a reaction that is not there, and the reply tells the user not to read correct numbers.**
- **Where:** shells are fixed on DOF 1–6 (`crates/engine/src/fea/formats.rs:197-200`). CalculiX carries the rotation constraint through its own extra nodes, so the `TOTALS=ONLY` RF sum over `HELD` (read at `mod.rs:735-738`) gains a spurious vertical force. The field itself is right: wall stress and deflection match the solid model within 5 %.
- **Repro:**
  1. Make a sheet-metal L bracket, 1.5 mm: base 60×40, 40 mm flange on the y=40 edge.
  2. Fix the base underside: `[30,20,0]`.
  3. Load the wall's top face with `[0,-20,0]`, material 5052-H32.
  4. Result: the equilibrium mismatch is 4.5 % (2.75 mm, eqcheck run), 162 % (2.75 mm, a later run), 41 % (1.83), 22 % (1.0) and 492 % (0.6). Each job file gives the same answer when rerun with 1 or 24 threads; the first two 2.75 mm runs differ only because gmsh gave two different meshes. The default run shows the warning "the forces do not balance, so this result is suspect … do not read the numbers".
- **Not affected:** the same part as solids balances (0.0 %); so do edge clamps (R1, R4–R6) and hole-wall clamps (N6).
- **Fix:** hold only translations where a support covers an area of the shell (its rotations follow). Or check equilibrium from the applied loads and the solver's internal forces, not the RF total.

### Lesser defects
- **D4 (high: a non-conservative "settled").** `SETTLED_PERCENT = 5` on one 1.5× step (`stress.rs:517`) let a fillet peak 5–6 % low be called settled (N2). For p ≈ 1–2, the remaining error is about change / (1.5^p − 1), so 3.5–9 % at a 4.4 % change. Report a Richardson estimate or an error band, tighten to about 2 %, or refine locally.
- **D5 (medium: an over-stated yield).** `"304l"` is an alias of 304 at 205 MPa (`crates/engine/src/fea/materials.rs:79`). The ASTM A240 minimum yield for 304L is 170 MPa, so the yield is 21 % too high. Repro: `material: "304L"` is accepted as "304 stainless steel, yield 205 MPa".
- **D6 (medium: setup silently dropped).** Unknown keys are ignored without a `param_warnings` entry. `loads[].at` (a remote load) and a top-level `load_cases` were accepted, and the solve used only the face load. Repro: `unknown.py` in the results archive.
- **D7 (medium: wrong words in what the user approves).** Every cylindrical face is described as a hole (`stress.rs:207-233`):
  - the clamped rim of a disc is "Held still: a hole Ø100";
  - a 2 mm fillet is "near a hole Ø4.00".
  These are the words the user approves in the dry run and the approval card.
- **D8 (low: two shares of yield).** The top-level `fraction_of_yield` is the overall peak over yield, including a singular clamp peak (`stress.rs:1053`, `stress.rs:1122`), while the summary leads with the away peak. On the inch strip they are 0.61 and 51.6 %. The past-yield warning (`stress.rs:1011`) also fires on the singular value.
- **D9 (low: no mesh check on thin solids).** The mesh check is skipped by default on plain thin solids. A 100 mm × 2 mm disc's finer run is predicted at 11 GB, because the default size is half the thickness at any part size (`stress.rs:963`).
- **D10 (low: grade limits not checked).** The grade yields hold only up to a thickness: 6061-T6 to 6.3 mm, S235 and S355 to 16 mm. The source text says so, but nothing warns when the part is thicker.
- **D11 (low: thickness mismatch not flagged).** A `thickness` that differs from the sheet's own thickness (3 mm on a 1.5 mm sheet) is accepted without a warning.
- **D12 (low, read in the code, not run).** `gate.rs:138` asks for every `inspect.stress_estimate`, including `dry_run: true`, which starts no program. The skill's dry-run-first workflow therefore costs the user two approvals.

### Not defects (checked)
- The refusals fired, with plain fixes, for:
  - **setup:** no supports; no loads; an unknown face; a point more than 1 mm from a face; a load on a held face; zero force or force over 10 MN; two bodies with none named;
  - **material:** none given; "steel"; "6061" with no temper; a material without a source; E in Pa or GPa; yield in psi;
  - **pressure:** a pressure without a unit; "psi";
  - **supports and elements:** a lone roller; rollers that leave a slide free (the reply names the axis and the fix); a roller on a hole; `pin`; `bolt`; shells on a solid without a thickness; `mesh_size` 0.01.
- Bare-number pressure is MPa, and the setup says so.

## 4. Missing capabilities, ranked by how often they block real work

1. **Bolts and holes.** There is no bearing (pin) load on a hole, no pinned support, and no bolt preload. A fixed hole wall is singular, and a force on a hole wall is spread over the whole cylinder. Nearly every bracket needs this.
2. **Reading the field.** There is no probe for the stress at a point, no path or section, and no stress components. Here the field could only be read by parsing `.frd` files by hand.
3. **Local mesh refinement and sizing from curvature** at fillets and holes. This is also the fix for D2 and D4.
4. **Several load cases** and their combinations.
5. **Remote forces and moments, torque, gravity and body loads.**
6. **Principal stresses**, signed, for brittle materials and fatigue, and **stress linearisation** for pressure-vessel codes.
7. **Report export:** setup, materials and sources, mesh, convergence, pictures.
8. **Contact and bonded assemblies.**
9. **Imperial units** in and out: psi, ksi, lbf, in.
10. **More materials:** grades by thickness and temper, and plastics with stated caveats.
11. **More support types:** elastic supports, rollers on tilted faces, frictionless cylindrical supports.
12. Modal and buckling analysis (out of scope here, listed for completeness).

## 5. What I could not check
- **The UI flow** (Stress button, approval card, legend, preview): the desktop app does not build on zhikun without the glib/GTK development packages, and installing system packages is outside this review's limits. It was judged from the code only (`stress_overlay.rs`, `agent_chat/gate.rs`, `acp.rs`) and the earlier session's screenshots. I did not see D3's false warning or D7's "hole" label on screen.
- **A real chat agent's wording:** only the command's own replies and the system note were audited.
- **Windows and macOS.** Network isolation was also not exercised: `unshare` is blocked on zhikun, so the runs fell back to running without isolation, as before.
- **A real 16 GB laptop.** It was emulated with a cgroup memory cap. Timings ran on 24 cores, partly under contention from parallel runs: the Lamé 1.0 mm mesh check took 313 s.
- **NAFEMS LE10,** and bent sheet metal under pressure or a symmetry plane.
- **The Peterson curve fit** (Pilkey and Pilkey, 3rd ed., Chart 3.1) was used from its published coefficients, not checked against the printed chart. Richardson extrapolation of my own two fine meshes, 108.2 MPa, agrees with its 107.3 MPa to under 1 %.
- **gmsh's run-to-run repeatability:** it varies (seen in D3), and was not studied.

## 6. Recommended order
1. D2: mesh sizing from curvature, high-order optimisation and readable errors. This unblocks most real parts.
2. D1: a memory gate on the first run.
3. D3: the shell clamp holds and the equilibrium check.
4. D4–D7.
5. Then the capabilities in section 4: items 1 to 3 first.
