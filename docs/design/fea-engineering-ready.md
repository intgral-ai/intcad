# Stress estimate: ready for real engineering work (spec)

Status: decisions agreed 2026-11-17; the definition of ready narrowed to linear-elastic stress and strain on 2026-11-19 (section 7, decisions 11 to 13). Requirements and suite open to change as Phase A lands. Linear: PIL-294. Builds on the PRD (Linear document f4f98b3a4d54, v0.6) and on the ship review, [docs/reviews/fea-ship-review.md](../reviews/fea-ship-review.md).
Scope: linear-elastic static analysis of one part, plus bolted and pinned parts: stress and strain. Nonlinear, dynamic and thermal analysis stay out of scope.

## 1. What "ready" means

A working mechanical engineer brings the parts they size every week:
- a machined or sheet-metal bracket bolted to a frame;
- a lid or housing under pressure;
- a lug or link on a pin, under a bearing load.

They say the supports, loads and material in words. They get back:
- stresses and strains they can put in a design note: the peaks, and the value at any point they name;
- how far to trust each number;
- the reactions at each support and bolt.

The feature is **ready** when all of the following hold:
1. Every suite case that gates ready (section 5) passes in CI, on Linux and Windows (macOS follows, decision 13).
2. The parts in section 2's evidence mesh and solve at default settings, or are refused with a plain reason before anything heavy runs.
3. Every number comes with a stated discretisation error, and that error is true on the suite.
4. Bolts, bearing loads on holes and pinned supports (B1 to B3) are available.
5. Stress (von Mises, principal, components) and elastic strain can be read at a point and per support (C1, C2, C8).

Left out of ready, for later (decision 11): remote forces and moments, gravity, several load cases, elastic mounts, tilted rollers, imperial units in and out, more grades (B4 to B10); linearisation, local refinement, report and results export (C3 to C6).

It stays an estimate. Section 6 says how the wording keeps that.

## 2. Evidence (measured 2026-11-17 on zhikun: gmsh 4.12.1, CalculiX 2.21)

**Accuracy where a part meshes:**
- Within 2 % of the reference away from supports, on: the cantilever, Heywood's hole, NAFEMS LE1, Lamé, Roark's clamped plate, the Peterson fillet on a fine mesh, the inch design and the STEP import.
- Deflections within 3.3 % everywhere.
- Refusals and wording: both right throughout the review.

**What blocks real use:**

| # | Problem | Measured | Fix proven today |
|---|---|---|---|
| D1 | The first solve has no memory check | R2's plate at 0.7 mm in 8 GB: 115 s, then "ccx stopped with signal 9 … see gpl.htm" | Mesh first, count nodes, predict, then refuse or solve. gmsh alone took 9.8 s and 0.29 GB for 493,468 nodes (predicted solve: 11.9 GB). At 1.0 mm: 3.0 s, 199,504 nodes, 4.0 GB |
| D2 | The default mesh fails at small holes | 5 of 6 bars with a Ø1–4 hole (the review's 10-bar table) failed: gmsh "overlapping facets" or ccx "nonpositive jacobian" | `-clcurv 12`: 6 of 6 mesh. `-clcurv 24 -optimize_ho`: 6 of 6, and Peterson +2.1 % (was −4.8 %). R2 unchanged (+0.28 %). `-optimize_ho` alone: 3 of 6 still fail |
| D3 | Shell clamps over an area report a spurious reaction, and the reply calls correct numbers "suspect" | L bracket, base clamped: mismatch 4.5 % to 492 % depending on the mesh | Hold only translations on area supports: the reaction balances exactly (Fz 32.4 N → 3e-10 N), and the tip deflection 0.355 mm comes closer to the solid model's 0.351. Edge clamps must keep rotations: without them R1's strip is a mechanism (tip 2e10 mm) |
| D4 | "Settled" (5 % over one 1.5× step) passed a fillet peak that was 5–6 % low | Peterson: 97.8 → 102.1, "settled"; converged value 108.2 | With D2's sizing the same check gives 1.2–1.5 % and is true. Section 4.1 replaces the criterion anyway |
| D5–D12 | 304L yield 21 % high; unknown keys dropped; rims and fillets called "holes"; two shares of yield; no thickness bands; analysis thickness unchecked; two approvals per estimate | see the review | section 4.1 |

Two more findings shape the requirements:
- gmsh gave two different meshes for the same input and size, so runs are not yet reproducible (seen in D3). Not reproduced on 2026-11-19: gmsh 4.12.1 already meshes on one thread, and the same STEP gave byte-identical meshes five times running for each of three parts. What did change between computers was CalculiX's thread count (its last digits), and Intcad read the results in a hash map's order. Both are fixed on `fea/8-reactions-repro` (A7).
- The finer mesh-check run of a 68,863-node solid took 313 s (24 cores, under contention).

## 3. Phases

| Phase | Name | Gate |
|---|---|---|
| A | Trust what it already does | All of it before any release shows the Stress button |
| B | The setups engineers bring | B1 to B3 before telling engineers it is ready; B4 to B10 after |
| C | Reading, refining and reporting | C1, C2, C7 and C8 before telling engineers it is ready; C3 to C6 after |
| D | Scale and assemblies | Later; not part of "ready" |

A comes first and is short. B and C can run in parallel once A has landed: they touch different files (setup and job writing, versus results and reporting).

**Where Phase A stands (2026-11-19, draft PR 175, not merged):**

| ID | State |
|---|---|
| A1 | Done: the first run is refused before meshing or before solving when it will not fit (V-A1) |
| A2, A3 | Done: curvature sizing and high-order optimisation; failures read as advice (V-A2, V-A3) |
| A4 | Done: area supports on shells hold translations only (unit tests); V-A4's four-mesh bracket passes on `fea/8-reactions-repro` (forces and moments within 0.002 % at 2.75, 1.83, 1.0 and 0.6 mm) |
| A5 | Done on `fea/8-reactions-repro` (2026-11-19, the eighth PR of the stack): each support's force and moment, and the moment balance; V-A5 passes (the clamp's F × e = 1000 N·mm to three figures, solids and shells; the test allows 0.5 %) |
| A6 | Partly: a two-mesh check at 2.5 % with a first-order "may be off" bound; no three-mesh extrapolation or error band |
| A7 | Done on `fea/8-reactions-repro`: gmsh and CalculiX on one thread, results read in node order, both versions in the reply; V-A7 passes for V-2's plate and V-8's bracket as shells and as solids, twice each: identical replies. V-6's filleted bar is not in that test yet (no test builds it) |
| A8, A9, A10 | Done (D6, D7, D8) |
| A11 | Partly: 304L at 170 MPa; one upper thickness limit per grade, not its full bands |
| A12 | Partly: sheets only; no ray-pairing check on solids |
| A13 | Partly: a dry run asks no approval; the card does not yet show predicted nodes, memory and time |
| A14 | Not started: the real-solve tests are `#[ignore]` and run by hand on Linux; no CI job runs them |

## 4. Requirements

IDs carry over into PR titles. Each requirement names its acceptance test from section 5.

### 4.1 Phase A: trust what it already does

| ID | Requirement | Acceptance |
|---|---|---|
| A1 | **Memory gate before solving.** Mesh, count nodes, predict the solver's memory (the existing `solver_bytes`), and refuse above the first-run limit (decision 3). The refusal names the predicted memory and a `mesh_size` that fits. A solver killed by the system becomes a plain message ("the solver ran out of memory …"), never the program's banner. | V-A1 |
| A2 | **Meshing that survives real parts.** gmsh gets sizing from curvature (24 elements per turn), a minimum size of 1/50 of the maximum, and high-order optimisation. The default solid size takes the part's size into account as well as its thinnest side, so a 200 × 2 mm plate does not get 600,000 nodes. | V-A2, V-1…V-8 |
| A3 | **Readable failures.** Map the known mesher and solver failures to advice, naming the face or feature near the failure when the element can be located. The raw text goes to the kept log only. Known failures: overlapping facets, no elements in volume, nonpositive jacobian, out of memory, timeout. | V-A3 |
| A4 | **Shell supports.** An area support holds translations only. An edge support (a sheet's end, rim or hole wall) holds translations and rotations, as now. | V-A4, V-1, V-4 |
| A5 | **Equilibrium per support, and moments.** Print RF per support set, not only the total. Report each support's force and moment, and check force and moment balance about the middle of the part's bounding box (decision 17). The warning fires only when the balance fails. | V-A4, V-A5 |
| A6 | **Discretisation error.** Replace "settled ≤ 5 %" with: solve on h and h/1.5; if the away peak moved more than 1 %, solve h/2.25 when it fits the memory limit. Use Richardson extrapolation with the observed order. Report the extrapolated value and an error band, e.g. "106 MPa, within ±2 % of the converged value". "Settled" requires the band to be ≤ 3 % and the observed order to lie in [0.5, 4]. A peak that grows without converging (observed order < 0.5) is reported as "keeps rising: likely a sharp inside corner or an idealised support". With only two meshes, say so and use the conservative bound \|Δ\| / (1.5 − 1). | V-A6 |
| A7 | **Reproducible runs.** The same design and request give the same mesh, the same numbers and the same words. Pin gmsh's threads for meshing (`-nt 1`, or whatever proves deterministic) and record both programs' versions in the reply. | V-A7 |
| A8 | **Strict parameters.** Unknown keys, at top level and inside loads and supports, are refused in the dry run, with the nearest known key. This covers `at`, `load_cases` and `moment`, until Phase B makes them real. | V-A8 |
| A9 | **Faces in words.** A cylindrical face is "a hole Ø…" only when it faces inward (a concave cylinder bounding a through or blind hole). Otherwise it is "the outer rim Ø…", "a fillet R…", "a boss Ø…" or "a curved face". | V-A9 |
| A10 | **One share of yield.** The top-level `fraction_of_yield` is the share of the number the summary leads with: the away peak when the overall peak is at a support. The overall share becomes `fraction_of_yield_overall`. The past-yield warning names which peak passed yield, and says when it is only the idealised support's. | V-A10 |
| A11 | **Materials.** Remove `304l` from 304's aliases and add 304L at 170 MPa (ASTM A240). Each grade gets its standard's thickness bands, and a warning fires when the part's thickness is outside the band its yield comes from. | V-A11 |
| A12 | **Thickness check.** An analysis `thickness` that differs from the sheet's own thickness, or from the thickness measured by ray pairing on a solid, by more than 5 % is a warning in the dry run. | V-A12 |
| A13 | **One approval.** A dry run starts no program and asks no approval. The real run asks once, and its card shows the predicted nodes, memory and time, once known. | V-A13 |
| A14 | **Verification in CI.** Section 5's suite runs nightly on Linux and Windows (macOS after ready, decision 13) with pinned gmsh and CalculiX versions. A failing case blocks the next release. | the suite itself |

### 4.2 Phase B: the setups engineers bring

CalculiX 2.21 keywords named below are taken from its manual. Phase B starts with a one-day spike that runs each keyword on a one-element model and records what works.

| ID | Requirement | Sketch of the model | Acceptance |
|---|---|---|---|
| B1 | **Bolt connector:** "bolted through holes 1 and 2 to the frame", with an optional preload in N. Reports each bolt's axial and shear force. | A beam (B31) bolt between two distributing couplings on the washer faces or hole walls; preload by `*PRE-TENSION SECTION`; "to the frame" fixes the far end. Replaces fixed hole walls as the advised bolted support. | V-B1 |
| B2 | **Bearing load on a hole:** a pin or shaft pressing on a hole wall. | A cosine pressure over the loaded half, with its resultant equal to the force. | V-B2 |
| B3 | **Pin and cylindrical supports:** free to turn about the hole's axis (a hinge), with or without axial sliding. | Hole nodes in a cylindrical `*TRANSFORM`: radial and axial held; tangential free. | V-B3 |
| B4 | **Remote force and moment:** a force at a point off the part, or a torque on a face. | `*COUPLING` + `*DISTRIBUTING` to a reference node carrying the load. | V-B4 |
| B5 | **Gravity and acceleration:** the part's own weight, or "3 g along −Z". Needs the material's density, sourced. | `*DLOAD … GRAV`. | V-B5 |
| B6 | **Several load cases** in one run, with named results and a worst case per quantity. | One `*STEP` per case, loads replaced each step (`OP=NEW`). Combinations are linear sums done in Intcad. | V-B6 |
| B7 | **Elastic support:** a mount of given stiffness, N/mm per axis. | Springs to ground on a coupling node. | V-B7 |
| B8 | **Rollers on any flat face** (not only X, Y, Z), and a frictionless support on a cylinder. | `*TRANSFORM`, rectangular or cylindrical. | V-B8 |
| B9 | **Imperial units in and out:** psi, ksi, lbf, kip, in, ft·lbf, N·m, kN. The reply gives every number in the design's units and in SI. Points for faces use the design's units, the same as `model.faces` reports them (decision 7). | — | V-B9 |
| B10 | **More grades,** each with sourced minimum values, thickness bands and density: 7075-T6, 2024-T3, 5083-H111, 316/316L, S275, 1.0503/C45 normalised, 42CrMo4 QT, 17-4PH H1150. | — | V-A11 |

### 4.3 Phase C: reading, refining, reporting

| ID | Requirement | Acceptance |
|---|---|---|
| C1 | **Probe:** the stress (von Mises, principal, components) and displacement at a point, a face, an edge or a hole. Each value is interpolated from the element, not the nearest node, and comes with its error band from A6. A command, and a click in the viewport. | V-C1 |
| C2 | **Principal stresses and components** in world or a chosen axis set (cylindrical about a hole or shaft), as a colour map and in the probe. | V-C2 |
| C3 | **Stress linearisation** along a line through a wall: membrane, bending and peak, in the manner of ASME VIII-2 Annex 5-A. | V-C3 |
| C4 | **Local refinement:** refine where the away peak or a probed point sits until A6's band is ≤ 3 %, within the memory limit, using a gmsh size field around the point. | V-C4 |
| C5 | **Report export (HTML and PDF).** Contents: the setup with pictures of the supports and loads; the material and its source; the mesh, the convergence table and error bands; peaks, probes, reactions per support and per bolt; deflection; warnings; what the estimate leaves out; both programs' versions; Intcad's version and the design's hash. | V-C5 |
| C6 | **Results file export** (VTU and CSV) for checking in other tools. | V-C5 |
| C7 | **Smooth colours:** the stress painted from the element field, not the nearest node, so the map matches the probe. | V-C1 |
| C8 | **Elastic strain.** Computed from the stress by Hooke's law: for a linear isotropic material it is the same linear map of the same field as the solver's own `E`, without doubling the results file. The reply gives the principal strains at the away peak; the probe gives the components (shears as engineering γ), the principal strains and the strain along a gauge direction, in µε. No "equivalent strain": its definitions differ between programs. | V-C6 |

### 4.4 Phase D (later, not part of "ready")

D-1 bonded and contact assemblies, and bolted joints with contact. D-2 a faster solver (PaStiX or PARDISO builds), adopted only after it passes the whole suite five times running with no wrong answer. D-3 time estimates and models of a million nodes. D-4 modal and buckling analysis.

## 5. Verification suite

Each case runs through the command as an agent would. The away peak, or the stated point, must be within the tolerance of the reference, and the reported error band must contain the reference. Cases marked † are already in the code (R1–R6) or were run in the review; cases marked ‡ need their reference confirmed against the cited edition before they become tests.

Every case gates "ready" except those for requirements left out of it (decision 11): V-B4 to V-B9, V-C3, V-C4 and V-C5. Those stay in the suite and gate the release that adds their requirement.

| ID | Case | Reference | Tolerance |
|---|---|---|---|
| V-1† | Cantilever strip: solid, shell, half on a symmetry plane | Euler-Bernoulli σ(x), tip F L³/3EI | 3 % stress, 4 % tip |
| V-2† | Plate with a central hole in tension | Heywood Kt = 2 + (1 − d/W)³ | 3 % |
| V-3† | NAFEMS LE1 elliptic membrane | σyy at D = 92.7 MPa | 3 % |
| V-4† | Lamé thick cylinder, internal pressure | σθ(a) = p(b² + a²)/(b² − a²); u(a) | 2 % |
| V-5† | Clamped circular plate, uniform pressure, solid and shell | Roark (7th ed.) Table 11.2 case 10b: w, σ(r) | 3 % |
| V-6† | Stepped flat bar with shoulder fillets, tension | Peterson (Pilkey 3rd ed.) Chart 3.1 curve fit, Kt 2.146 at D/d 1.4, r/d 0.1 | 3 % |
| V-7† | The same bar with a sharp corner | singular: must never be "settled" | verdict |
| V-8† | Sheet-metal L bracket, base clamped, load on the wall: shell vs solid | wall moment by statics; shell equals solid | 5 %, equilibrium 1 % |
| V-9‡ | NAFEMS LE10 thick plate | σyy at D = −5.38 MPa | 3 % |
| V-10‡ | NAFEMS LE5 Z-section cantilever (torque) | σxx at A = −108 MPa | 5 % |
| V-11 | Thin cylinder with end caps, shells, symmetry | pr/t and pr/2t | 2 % |
| V-12 | Curved beam: the 90° bend of a sheet bracket | Winkler-Bach at the inner fibre | 5 % |
| V-A1 | R2's plate at 0.7 mm under an 8 GB cap | refused before solving, in ≤ 15 s, naming a mesh size that fits | behaviour |
| V-A2 | 100 × 20 × T bars, T 5 and 10, one hole Ø 1, 2, 3, 4, 6 | all 10 solve at default settings | 10 of 10 |
| V-A3 | A hole too small for even the minimum size; a solver killed for memory | plain advice, no program banner | behaviour |
| V-A4 | V-8 at meshes 2.75, 1.83, 1.0, 0.6 | per-support forces balance within 0.1 %; no "suspect" warning | 4 of 4 |
| V-A5 | A force with a lever arm, clamped | moment reaction = F × e | 0.5 % |
| V-A6 | V-6 and V-7 at default settings | V-6: band ≤ 3 % and contains 107.3 to 108.2; V-7: "keeps rising" | verdict |
| V-A7 | V-2, V-6 and V-8, each run twice | identical node counts and numbers | exact |
| V-A8 to V-A13 | One test per requirement: unknown keys refused; rim, fillet and boss wording; one share of yield; 304L at 170 MPa with a thickness band warning; a thickness mismatch warned; a dry run without approval | — | behaviour |
| V-B1 | Plate bolted through two holes to ground, preload P, then a shear load | bolt axial force = P; shear shared by statics | 2 % |
| V-B2 | Pin-loaded lug | a frictionless CalculiX contact model of the same lug and pin, in the test (decision 5) | 10 % at the hole edge; resultant exact |
| V-B3 | A bar pinned at one hole, loaded at the other end | free to turn: zero moment at the pin; simply supported beam σ | 3 % |
| V-B4 | A round shaft with a torque by remote moment | τ = 16T/πd³; twist TL/GJ | 3 % |
| V-B5 | A simply supported plate strip under its own weight | σ = 3ρgL²/4t | 3 % |
| V-B6 | Two load cases and their sum | superposition: case 1 + case 2 = the combined run | 0.1 % |
| V-B7 | A bar pulled along its axis through an elastic mount of k N/mm | end displacement = F/k + F L/(E A) | 2 % |
| V-B8 | LE1 with its rollers turned 30° | the same σyy at D | 3 % |
| V-B9 | V-1 in an inch design with lbf, and V-4 in psi | the same physics in both unit systems | 0.1 % |
| V-C1 | V-4: probes at 15 points through the wall | Lamé σθ(r) | 1.5 % (met by a hand probe in the review) |
| V-C2 | V-4: principal stresses at the bore | σ1 = σθ, σ3 = σr = −p | 2 % |
| V-C3 | V-4: linearisation through the wall | membrane hoop = p a/(b − a) = 50 MPa; bending from Lamé's closed form | 2 % |
| V-C4 | V-6 from the default mesh with refinement on | within 3 % of 108.2, with fewer nodes than a uniform mesh of the same accuracy | 3 % |
| V-C5 | The report for V-B1 | every number equals the reply's; the versions and the design hash match | exact |
| V-C6 | A bar in tension; V-1's cantilever strip | axial ε = F/(E A), lateral −ν ε; surface ε(x) = σ(x)/E away from the clamp | 2 % |

## 6. Wording and agent contract

This part stays as it is: an estimate, never a verdict; no safe, unsafe, pass, fail, OK, margin or safety factor; the share of the yield strength given; tests on every reply.

It changes in three ways:
- **Error bands:** every quoted stress carries its error band from A6: "about 106 MPa (within ±2 %), 30 % of the yield strength given".
- **Bolts:** bolt forces are quoted as forces, never as a "bolt OK".
- **Skill guide:** `solidcraft-analysis` gains the bolt, bearing, remote-load and load-case setups. Its "Not available yet" list shrinks as Phase B lands.

## 7. Decisions

All ten agreed by Tianwei on 2026-11-17, as recommended below.

1. *(Replaced by decision 11 on 2026-11-19.)* **Definition of ready = Phases A + B + C** with the suite green on all three operating systems. Recommended. The alternative, A + B with the probe only, ships sooner without reports or linearisation, which engineers need for design notes.
2. **Convergence criterion:** a three-mesh Richardson estimate with a 3 % band (A6). Recommended. Keeping the two-mesh 5 % rule leaves D4.
3. **First-run memory limit:** half the memory available. The 6 GB ceiling stays for the mesh check's extra runs; a preference can raise both. Recommended.
4. **Dry run stays program-free and asks no approval;** the memory gate sits inside the run, after meshing, which costs ≤ 10 s. Recommended. The alternative, meshing in the dry run, gives node counts earlier but starts gmsh before the user has approved anything.
5. **Bearing-load reference:** a CalculiX contact model of the same lug, built in the test, plus exact statics. Recommended over a chart value we cannot confirm.
6. **Bolts:** beam-and-coupling connectors with optional preload (B1) become the advised bolted support; a fixed hole wall stays available, flagged as rigid. Recommended.
7. **Units:** faces and points in the design's units, the reply in both. Recommended. Today a design in inches reports mm, which works but invites a 25.4× error from an agent that reads the user's sketch.
8. **Safety factor display** (PRD decision 2): still no factor. Once A6's bands exist, show "share of yield, within ±x %". Recommended.
9. *(Narrowed by decision 13 on 2026-11-19.)* **Platforms:** Windows and macOS are in "ready". The app checks for gmsh and CalculiX and their versions, and points to an installer. Recommended. Linux-only is faster to ship but leaves most engineers out.
10. **Where this spec lives:** this file, linked from PIL-294, with the PRD's section 4 pointing here. Recommended.

Agreed by Tianwei on 2026-11-19:

11. **Ready means linear-elastic stress and strain** on one part, with bolts, bearing loads and pins: all of Phase A, B1 to B3, and C1, C2, C7 and C8. This replaces decision 1. The rest of Phases B and C comes after ready, in the order of the review's ranking.
12. **Bolts, bearing loads and pins stay in ready.** They are elastic setups, and nearly every bracket has one; without them a bolted part can only be clamped rigidly at its hole walls, which is singular there.
13. **Platforms: Linux and Windows for ready, macOS after.** This narrows decision 9; the app's check for gmsh and CalculiX and their versions is unchanged.
14. **Shipping the Stress button needs Linux CI only (agreed 2026-11-19).** A14's nightly job starts on Linux; Windows follows before "ready" (decision 13 still holds for ready). Tracked in INT-1498.
15. **A12 on solids is deferred (agreed 2026-11-19).** The analysis-thickness check covers sheets only; on a solid, the reply says the thickness was not checked. Ray pairing on solids returns before "ready". Tracked in INT-1498.

Agreed by Tianwei on 2026-11-20:

16. **CalculiX always runs on one thread,** whatever `OMP_NUM_THREADS` says, so the same request gives the same numbers on every computer. Twenty-four threads changed the last digits (about 2 × 10⁻⁹ apart) and saved 3 % of the time on one 89,786-node job. No release shows the Stress button yet, so no user loses a default.
17. **Moments are balanced about the middle of the part's bounding box,** not its centroid. When the forces balance, the moment balance is the same about any point; the point only changes the moments printed and the 1 % scale, and the middle needs no volume integration. The reply states the point it used. Each support's own moment is given about the middle of the nodes it holds.

## 8. Risks
- **CalculiX keyword support in 2.21** (couplings, pre-tension, transforms) is taken from its manual and not yet run. The Phase B spike settles it first.
- **gmsh determinism** (A7): settled on Linux, where one thread repeats byte for byte. Still to see on Windows and with other gmsh versions; A14's CI runs V-A7 there.
- **Windows and macOS:** gmsh and CalculiX builds vary in version and solver. Pin and test them; refuse versions the suite has not passed.
- **Mesh size:** curvature sizing raises node counts on parts with many small features: ×1.3 to ×3.3 on the test bars (`-clcurv` 12 and 24). A1's gate catches the extreme cases; C4's local refinement is the long-term answer.

## 9. Next steps

Phase A's first three steps (A1 to A4, A8 to A10, and parts of A6 and A11 to A13) landed in draft PR 175 as the fixes for the review's D1 to D12. What is left, each as one PR stacked on PR 175, with its acceptance tests:
1. C8 + C1 + C2: strain, the point probe, and stress components and principal stresses. In progress on `fea/7-strain-probe` (2026-11-19): `probes` on the estimate (points, an optional gauge direction), read inside the solver's element; principal stresses and strains at the away peak; V-C1, V-C2 and V-C6 pass. Still to do for C1 and C2: probes on a face, an edge or a hole, the click in the viewport, and colour maps of principal stress and strain (with C7).
2. A7 + A5 (the first step of INT-1498, the shortest path to showing the Stress button): repeatable runs with the versions in the reply, and reactions per support with the moment balance. Done on `fea/8-reactions-repro` (2026-11-19), stacked on `fea/7-strain-probe`; V-A4, V-A5 and V-A7 pass.
3. A6: the three-mesh error band.
4. A11 and A13's remainders (thickness bands per grade; the approval card's predicted nodes, memory and time), and A14: the suite in CI, on Linux first (decision 14).
5. B1 to B3: bolts, bearing loads and pins, after the one-day CalculiX keyword spike (section 4.2).

The rest of A11 to A13 goes with whichever PR touches the same code.
