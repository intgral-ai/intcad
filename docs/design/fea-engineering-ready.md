# Stress estimate: ready for real engineering work (spec)

Status: decisions agreed 2026-11-17; the definition of ready narrowed to linear-elastic stress and strain on 2026-11-19 (section 7, decisions 11 to 13). Requirements and suite open to change as Phase A lands. Linear: PIL-294. Builds on the PRD (a Linear document, v0.6, not public) and on the ship review, [docs/reviews/fea-ship-review.md](../reviews/fea-ship-review.md).
Scope: linear-elastic static analysis of one part, plus bolted and pinned parts: stress and strain. Nonlinear, dynamic and thermal analysis stay out of scope.

## 1. What "ready" means

Four bars, each on top of the one before (decisions 31 to 34, 2026-11-28). Tianwei accepts each one, with its evidence linked; no agent marks one met.

1. **Ready for review**, for each PR: it builds and its tests pass against the `develop` merged into it; its own review is done; its description is checked and true, and its blockers are stated. Met by #169 to #208 on 2026-11-27.
2. **Merged:** Tianwei approves and merges, and `ci` passes on `develop`. A switch hides the Stress button until #270 is in, and the agent route until bar 3b (decision 32), so a merge does not ship either.
3. **Shipped:** a release shows the Stress button, and the estimate works for a user in the installed app, run by hand (decision 35). This is "done" for the stress estimate. It needs all of Phase A merged; the free run by hand (#270); decision 22's memory model; one manual run on Windows; runs by hand on R2's plate and the L bracket; a warning for gmsh and CalculiX versions the suite has not passed; and their packaging and dependency audit for the release (`docs/release-readiness-0.1.3.md`, section 13.1).
   - **3b. Agents:** an agent's run admitted and charged (decision 30); runs longer than the paid-call bound handled; one run with a real model in the chat with the whole set. Until then an agent's estimate is refused, as it is today.
4. **Ready for engineers:** the rest of this section.

A working mechanical engineer brings the parts they size every week:
- a machined or sheet-metal bracket bolted to a frame;
- a lid or housing under pressure;
- a lug or link on a pin, under a bearing load.

They say the supports, loads and material in words. They get back:
- stresses and strains they can put in a design note: the peaks, and the value at any point they name;
- how far to trust each number;
- the reactions at each support and bolt.

The feature is **ready for engineers** (bar 4) when all of the following hold:
1. Every suite case that gates ready (section 5) passes in CI, on Linux and Windows (macOS follows, decision 13).
2. The parts in section 2's evidence mesh and solve at default settings, or are refused with a plain reason before anything heavy runs.
3. Every number comes with a stated discretisation error, and that error is true on the suite.
4. Bolts, bearing loads on holes and pinned supports (B1 to B3) are available.
5. Stress (von Mises, principal, components) and elastic strain can be read at a point and per support (C1, C2, C8).

Left out of ready, for later (decision 11): remote forces and moments, gravity, several load cases, elastic mounts, tilted rollers, imperial units in and out, more grades (B4 to B10); linearisation, local refinement, report and results export (C3 to C6).

It stays an estimate. Section 6 says how the wording keeps that.

## 2. Evidence (measured 2026-11-17 on a 24-core Linux workstation: gmsh 4.12.1, CalculiX 2.21)

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
| D4 | "Settled" (5 % over one 1.5× step) passed a fillet peak that was 5–6 % low | Peterson: 97.8 → 102.1, "settled"; converged FE value about 109.6 (A6's study; the ship review's 108.2 came from meshes too coarse at the fillet) | With D2's sizing the same check gives 1.2–1.5 % and is true. Section 4.1 replaces the criterion anyway |
| D5–D12 | 304L yield 21 % high; unknown keys dropped; rims and fillets called "holes"; two shares of yield; no thickness bands; analysis thickness unchecked; two approvals per estimate | see the review | section 4.1 |

Three more findings shape the requirements:
- gmsh gave two different meshes for the same input and size, so runs are not yet reproducible (seen in D3). Not reproduced on 2026-11-19: gmsh 4.12.1 already meshes on one thread, and the same STEP gave byte-identical meshes five times running for each of three parts. What did change between computers was CalculiX's thread count (its last digits), and Intcad read the results in a hash map's order. Both are fixed on `fea/8-reactions-repro` (A7).
- The finer mesh-check run of a 68,863-node solid took 313 s (24 cores, under contention).
- Solve time and memory depend on how deep the mesh is, not only on its nodes: a 50 mm cube with 95,384 nodes solved in 70–86 s and needed 3.0 GB; R2's 2 mm plate with 114,027 nodes solved in 13 s. `solver_bytes` is up to 48 % low on deep solids (measured 2026-11-20, 21 runs, for A13).

## 3. Phases

| Phase | Name | Gate |
|---|---|---|
| A | Trust what it already does | All of it merged before any release shows the Stress button (bar 3) |
| B | The setups engineers bring | B1 to B3 before telling engineers it is ready (bar 4); B4 to B10 after |
| C | Reading, refining and reporting | C1, C2, C7 and C8 before telling engineers it is ready (bar 4); C3 to C6 after |
| D | Scale and assemblies | Later; not part of "ready" |

A comes first and is short. B and C can run in parallel once A has landed: they touch different files (setup and job writing, versus results and reporting).

**Where Phase A stands (2026-11-20; draft PRs 175 to 195, none merged):**

| ID | State |
|---|---|
| A1 | Done: the first run is refused before meshing or before solving when it will not fit (V-A1) |
| A2, A3 | Done: curvature sizing and high-order optimisation; failures read as advice (V-A2, V-A3) |
| A4 | Done: area supports on shells hold translations only (unit tests); V-A4's four-mesh bracket passes on `fea/8-reactions-repro` (forces and moments within 0.002 % at 2.75, 1.83, 1.0 and 0.6 mm) |
| A5 | Done on `fea/8-reactions-repro` (2026-11-19, the eighth PR of the stack): each support's force and moment, and the moment balance; V-A5 passes (the clamp's F × e = 1000 N·mm to three figures, solids and shells; the test allows 0.5 %) |
| A6 | Done in draft #195 (`tianwei/fea-9-error-band-8e0f9c`, 2026-11-20): h, h/1.5 and, when the away peak moved more than 1 % and it fits A1's limit, h/2.25; curvature sizing refined with each step; Richardson at the observed order; a band on every quoted number, at least ±0.5 %; "keeps rising" under order 0.5. V-A6 passes: V-6 about 110 MPa within ±0.5 %, holding the FE limit 109.6; V-7 keeps rising |
| A7 | Done on `fea/8-reactions-repro`: gmsh and CalculiX on one thread, results read in node order, both versions in the reply; V-A7 passes for V-2's plate and V-8's bracket as shells and as solids, twice each: identical replies. V-6's filleted bar is not in that test yet (no test builds it) |
| A8, A9, A10 | Done (D6, D7, D8) |
| A11 | Done in draft #193: 304L at 170 MPa; every grade carries its standard's thickness bands (EN 10025-2, EN 485-2, ASTM B209M, ASTM A240), the yield is the band's for the part's thickness, and a warning fires outside every band (V-A11) |
| A12 | Done for the shipping path in draft #193: sheets are checked against their own thickness; on a solid the reply says the thickness was not checked (decision 15). Ray pairing on solids stays before "ready" |
| A13 | Done in draft #196: the card shows the prediction from the part's size before Allow, and each run's counted nodes, memory and time left after meshing; it plans #195's third mesh, giving a range until the second mesh is solved; V-A13 passes (dry run without approval; the card's text; a real run's count; three meshes on V-6 and V-7). #196 is based on #195 (decision 28) |
| A14 | Done on Linux in draft #194: `fea-suite.yml` runs the real-solve suite nightly against `develop` and in the release gate at the candidate's commit, with gmsh 4.12.1 and CalculiX 2.21 pinned; a failing case, an empty run or another version blocks a stable or Beta release. 18 of 18 passed in 35 min on a standard 4-vCPU, 16 GB runner. Windows follows before ready (decision 13) |

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
| A6 | **Discretisation error.** Replace "settled ≤ 5 %" with: solve on h and h/1.5; if the away peak moved more than 1 %, solve h/2.25 when it fits the memory limit. Use Richardson extrapolation with the observed order. Report the extrapolated value and an error band, e.g. "106 MPa, within ±2 % of the converged value". "Settled" requires the band to be ≤ 3 % and the observed order to lie in [0.5, 4]. A peak that grows without converging (observed order < 0.5) is reported as "keeps rising: likely a sharp inside corner or an idealised support". With only two meshes, say so and use the conservative bound \|Δ\| / (1.5 − 1). Each step also refines the curvature sizing by 1.5 (24, 36, 54 elements per turn), so every element shrinks by the same ratio. An observed order over 4 means the first mesh was too coarse to count: the two finer meshes decide, with the two-mesh bound, and the number is settled when that bound is ≤ 3 %. No band is narrower than ±0.5 %: meshes of the same size made another way give peaks up to that much apart. | V-A6 |
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

Each case runs through the command as an agent would. The away peak, or the stated point, must be within the tolerance of the reference, and the reported error band must contain the case's converged finite-element value. A handbook formula or chart has errors of its own that a discretisation band does not measure (R2: Heywood's 25.12 MPa sits 0.5 % under the band of 25.25 MPa). Cases marked † are already in the code (R1–R6) or were run in the review; cases marked ‡ need their reference confirmed against the cited edition before they become tests.

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
| V-A6 | V-6 and V-7 at default settings | V-6: band ≤ 3 % and contains the FE model's converged value, 109.6 MPa (a quarter of the bar on two symmetry planes, meshes to h/3.4); the quoted value within V-6's 3 % of 107.3. V-7: "keeps rising", never "settled" | verdict |
| V-A7 | V-2, V-6 and V-8, each run twice | identical node counts and numbers | exact |
| V-A8 to V-A13 | One test per requirement: unknown keys refused; rim, fillet and boss wording; one share of yield; 304L at 170 MPa, and a thickness band warning on S355; a thickness mismatch warned; a dry run without approval | — | behaviour |
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
| V-C4 | V-6 from the default mesh with refinement on | within 3 % of 109.6, the FE model's converged value (decision 19), with fewer nodes than a uniform mesh of the same accuracy | 3 % |
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
18. **A grade's yield is its standard's minimum for the part's thickness band (agreed 2026-11-20).** A sheet or shell takes the band of its thickness; a solid takes the band of its bounding box's thinnest side, the thinnest stock it can come from, and its material line says when thicker stock is specified lower. A warning fires only when the part is outside every band the standard covers. The alternative, one yield per grade with a warning naming the band's yield, keeps a yield the standard does not give for that thickness. In draft #193.
19. **Error bands measure the mesh, not the handbook (agreed 2026-11-20).** A suite case's band must contain the FE model's own converged value; the quoted value must be within the case's tolerance of the reference. V-6's model converges to 109.6 MPa, 2.1 % over Peterson's 107.3. An observed order over 4 is judged on the two finer meshes, and no band is under ±0.5 %. In draft #195.
20. **A Fieldcraft solver-leg spike runs against A14's suite (agreed 2026-11-20).** It starts from #194's branch, without waiting for #194 to merge. Tangency Labs' Fieldcraft 0.8.1 passed all five pass bars of a one-computer trial on Windows: V-A2's ten bars, V-2's plate within 3 % on quadratic meshes, V-A7's repeat runs, a refusal instead of a kill at fine sizes, and Intcad's `.sc` parts opened without editing. Its default mesh (tet4) is not usable for stress. The spike tries it as a replacement for the gmsh-and-CalculiX leg only, judged by A14's suite. Phase A is unchanged, and any swap is its own later decision. *(Stopped by Tianwei on 2026-11-21.)*
21. **The approval card shows the run's size twice (agreed 2026-11-20).** Before Allow, a prediction from the part's size, since the dry run starts no program (decision 4). After Allow, each run's counted nodes, its memory and the time left. The alternatives were the prediction alone, or the count alone. In draft #196.
22. **One memory model for the card and the gates (agreed 2026-11-20).** A1's memory gate and the mesh check's gate move to A13's model, which counts the element layers (±14 % on 21 runs), from `solver_bytes`, which is up to 48 % low on deep solids, so A1's gate can let through a run that needs more memory than it predicts. Its own PR, after #195 merges, because the mesh check's gate is A6's code.

Agreed by Tianwei on 2026-11-21, on how the chat uses Mastra (the PRD's items M1 to M5). The notes on drafts are of 2026-11-25:

23. **The built-in agent loads a skill set called `modelling-skills` (M1).** It goes in `.agents/skills` in the agent's own home folder (`MASTRA_APP_DATA_DIR/home`), which Mastra Code 1.19.1 already searches. The set holds the four modelling skills, and `solidcraft-analysis` under its own name once this stack merges. It is built from the repository's `skills/` when the agent is packaged. Other agents keep `skill_reference`. The four are on `develop` since #204 (2026-11-23); `solidcraft-analysis` is #199, after this stack.
24. **The Stress button opens a "Stress estimate" chat mode (M2)** in `agent_chat/skill.rs`, like Text-to-CAD. Its note carries `solidcraft-analysis`'s SKILL.md, so every agent but the retired Mastra workflow connection has the workflow in its first message; today `skill_reference` without `file` returns `reference.md`, and the workflow comes only on request. The worked setups in `reference.md` are still read on demand, so this need not save a call. Mastra's `/skill/` command was not used: it works only at the very start of a message, and the app puts its own notes first. In draft #206.
25. **The app checks a stress answer's wording for every agent (M3):** verdict words, and the share of yield. A miss shows a warning under the answer; the answer is not rewritten. A Mastra output processor was not used: its step and result hooks run after the text has streamed (read from its source, not run), one that holds the text back would hide the answer until the end, and either covers the built-in agent only. In draft #207.
26. **Long runs: first test when the 300 s wait starts (M4).** If it starts at the call, it moves to Allow. For the built-in agent only, the call waits for #196's predicted time plus a margin; Mastra Code waits up to 7 days on an MCP call. Other agents keep "still running" and `pending`. In draft #205. Its test found that a chat agent's 300 s was the MCP bridge's read timeout, counted from the send: the time on the approval card counted too, and the estimate was sent a second time. The app's 60 s plus the bridge's 240 s, which this decision was first put in terms of, never applied to chat agents. Since `develop`'s #250 (2026-11-26) the forecast wait reaches only agents on the MCP bridge: decision 29.
27. **No Mastra workflow for now (M5).** Studies such as load cases or a thickness sweep become Rust commands first. The workflow runtime cannot run beside the built-in agent in a release today.

Agreed by Tianwei on 2026-11-27, at the set's final review after `develop` e401f8a18 and 83d02730b were merged in:

28. **#196 is based on #195.** The approval card's forecast plans #195's third mesh, which #196 alone cannot know, so #195 merges first. #205 stays on #196.
29. **#205 keeps its forecast wait, for agents on the app's MCP server.** Since `develop`'s #250 the built-in agent in a package reaches the window over its own stdio, not the MCP bridge. Its call waits up to an hour from the send (`CALL_TIMEOUT` in `agent_chat/host_bridge.rs`), approval included, so a long run returns its result without the forecast. #205's forecast wait reaches the built-in agent only when it is started from `SOLIDCRAFT_MASTRA_AGENT`. Its other two changes, the wait starting at Allow and a late reply no longer sending the request a second time (the bridge still retries once on a closed connection), are for every agent on the bridge.
30. **An agent's stress estimate counts toward the user's credits; the user's own use by hand is free.** docs/account/entry-matrix.md charges an agent's MCP call one unit per successful logical operation, read-only ones included, so an agent's estimate costs one unit. Not decided yet: whether an agent's dry run (sizing only, no program started) counts too, and the admission and bound a run of several minutes needs (section 8, 2026-11-27). A free run by hand needs its own request from the user's own gesture (the free GUI entry), never the agent's call after Allow. Before #270 the set had none: the Stress button, Inspect > Estimate and the command palette opened the chat (`stress_overlay::ask_in_chat`), and the agent made the call. #270 is that run by hand (decision 36), which bar 3 needs; an agent's charged run is bar 3b (decision 35).

Agreed by Tianwei on 2026-11-28, on what done means:

31. **Four bars: ready for review, merged, shipped, ready for engineers** (section 1). Done for the stress estimate is bar 3, the first release that shows the Stress button; bar 4 stays the later milestone that section 1 describes. For one PR, done is bar 1, then Tianwei's merge.
32. **A switch keeps the Stress button off in releases** until bar 3 holds, so the reviewed set can merge before the estimate works end to end. In draft #272. *(Settled by Tianwei on 2026-11-28, with decision 35: the switch hides the Stress button until #270 is in, so the button opens the panel by hand when it appears; and it hides the agent route, the chat's Stress estimate mode and `inspect.stress_estimate` for agents, until bar 3b holds. A release that has #270 meets bar 3's other items first.)* *(Its form, agreed by Tianwei on 2026-11-28: a preference, `stress_estimate`, not a build flag, so a tester can turn it on in an installed app. Its values are `auto` (the default), `off`, `by_hand` and `agents`. `auto` is `off` in a release build and in the browser, and `agents` in a development build or a test. It is not a plain on/off, because every preference is saved whole: a saved `off` would keep a later release's default from reaching people who already have the app. It has no Settings row. A tester sets it in the settings file or with `INTCAD_STRESS_ESTIMATE`, and `ui.prefs` from a controlled entry may not set it. A person's own run by hand is never refused. It sits in its own PR on #192, between the chain and the drafts, with one commit each on #270 and #206, and merges top-down so `develop` never has the button without it.)*
33. **Bar 3 needs one manual run on Windows; the suite in Windows CI belongs to bar 4.** Decision 13 (Linux and Windows for ready) is bar 4's.
34. **Tianwei accepts each bar**, with its evidence linked; no agent marks one met.
35. **Bar 3 ships by hand first.** The first release that shows the Stress button needs the run by hand (#270), not the agent route. The agent route is bar 3b (section 1): an agent's run admitted and charged, runs longer than the paid-call bound handled, and one run with a real model in the chat with the whole set.

Agreed by Tianwei on 2026-11-27, for the run by hand, and numbered after the bars:

36. **The run by hand is the right rail's Simulation panel (#270, agreed 2026-11-27).** It is bar 3's run by hand (decision 35). The Stress button, Inspect > Estimate and the command palette open it (`stress_by_hand.rs`); without the right rail they open the chat as before. The user chooses one of the seven sourced grades (none is chosen at first, and Run waits for one), adds the selected faces of one body as supports (fixed, roller, symmetry, slider) and as loads (a force in N along ±X, ±Y or ±Z, or a pressure), and sees the dry run's setup in words and drawn on the model, as the approval card shows it. Run is the only place the free request is built: `ControlRequest::trusted_gui` (`HumanGuiFree`), on the app's own queue, answered on a copy off the window thread. The chat's Allow and every agent route keep their own entries, and are refused before they start today (bar 3b). "Ask the Assistant instead" opens the chat, in decision 24's Stress estimate mode once #206 is in; decision 32's switch is to hide that agent route until bar 3b. Not in this version: custom material values, a force in any direction, probes, mesh size and elements (the command's defaults apply). This changes decision 24: the Stress button no longer opens the chat itself. Tried in the desktop app on 2026-11-28, set up and run by hand with no sign-in (#270's description).

## 8. Risks
- **CalculiX keyword support in 2.21** (couplings, pre-tension, transforms) is taken from its manual and not yet run. The Phase B spike settles it first.
- **gmsh determinism** (A7): settled on Linux, across computers too: on CI's runner, V-A7's three cases gave the same node counts and numbers as on the Linux workstation. Still to see on Windows and with other gmsh versions; A14's Windows job runs V-A7 there.
- **Windows and macOS:** gmsh and CalculiX builds vary in version and solver. Pin and test them; refuse versions the suite has not passed.
- **Mesh size:** curvature sizing raises node counts on parts with many small features: ×1.3 to ×3.3 on the test bars (`-clcurv` 12 and 24). A1's gate catches the extreme cases; C4's local refinement is the long-term answer.

Found on 2026-11-21, checking the drafts against "a release may show the Stress button", and checked again on 2026-11-25. The recommendations are not yet decided:
- **On Windows and macOS the memory gate can let through a run that does not fit (decision 22).** A1's gate predicts with `solver_bytes`, up to 48 % low on deep solids. Linux reads the free memory (`memory_available_here`) and allows half of it, which the worst case measured would just fit. On Windows and macOS a run may use a fixed 8 GB (`FIRST_RUN_FALLBACK`, half a 16 GB laptop's), all of an 8 GB computer's memory, and a run predicted at 8 GB could need about 15 GB. Recommended before a release shows the Stress button.
- **Not run on Windows or macOS yet.** Every gmsh and CalculiX solve, the suite's runs and every screenshot so far ran on Linux. Decision 14 lets the Stress button ship with Linux CI only. Recommended before a release: one manual run on Windows, from the Stress button to the colour map, with gmsh and CalculiX from their own installers.
- **One real model has run the chat's workflow, on one sheet-metal part.** On 2026-11-25 Claude Code, in the chat, read the guide, did the dry run, asked once on the approval card, solved, gave the share of yield and gave no verdict (`develop` 97236b5dd with the stack up to #192 and #208). That run predates `develop`'s #240; on e401f8a18 the same request is refused before it starts (2026-11-27 below). It did not ask the user to confirm the faces; the request had named the holes and asked for one run. The screenshots in #196, #206 and #207 used a scripted ACP agent, and #205 lists a real agent as not run. Not run with a real model: the built-in agent, #195, #196 and #205 to #207, R2's plate and the L bracket. Recommended before a release: one run with a real model in the chat, with the whole set, on R2's plate and the L bracket.
- **No check of the programs' versions (decisions 9 and 13).** The reply names the gmsh and CalculiX versions that ran (A7), and the suite pins gmsh 4.12.1 and CalculiX 2.21 (A14). Nothing in the app warns about or refuses another version; #194's release gate does. Recommended before a release: at least a warning.
- **Agents outside the chat get no Allow card.** The card is the chat's (`agent_chat/gate.rs`). An agent in a terminal using `intcad-cli mcp` reaches the app through the control port, which "Let AI agents connect" opens by default, with a per-launch token (docs/control-protocol.md). Since `develop`'s #240 (v0.1.3's paid-authority chain), that port's requests are a controlled entry like the chat's, and an estimate there is refused before it starts as well (2026-11-27 below). A1's memory refusal still applies to a run that is admitted.

Found on 2026-11-24, merging the whole set onto `develop` f18a9aec2 in a scratch merge. On 2026-11-27 `develop` e401f8a18, then 83d02730b, was merged into #169 to #196 and #205 to #208. Each item says where it stands now:
- **Release scope.** `develop`'s docs/release-readiness-0.1.3.md (section 13.1, 2026-11-23) puts the stress-estimate series in 0.1.4: its merges start only after the 0.1.3 candidate is dispatched, and the gmsh and CalculiX programs need a packaging and dependency audit. The tag v0.1.3 was pushed on 2026-11-25; its release gate passed on 2026-11-26, and no 0.1.3 release was published by 2026-11-27.
- **`ci` runs only on #169,** whose base is `develop`: on 2026-11-17, and again on 2026-11-27 after `develop` was merged in, with the checks it selects for the change: clippy, the selected regression gate, format and repository checks, and the WebAssembly build. The other drafts are based on feature branches, so `ci` (build, tests, clippy) has not run on them; fmt, `cargo xtask skills`, `cargo xtask lists`, clippy and the wasm check were run on each branch on the Linux workstation. #194's branch was built once in CI, for the real-solve suite (2026-11-20, 18 of 18 passed).
- **The approval forecast did not know the third mesh (A6 with A13).** #195's estimate solves up to three meshes and #196's forecast planned two; they merge without a conflict (#196's body said the third mesh needs one more planned run). Measured on the Linux workstation before the fix: V-6's bar took 127 s in 3 runs where the forecast was 17 s, 0.91 GB and 2 runs; V-7's took 70 s for the same forecast. Fixed in #196: the card gives a range (about 15 s – 2 min, 2 – 3 runs for V-6, by the code; not seen on screen), each count says the most the rest may take, and the built-in agent's wait uses that. After each count the time really left was under the upper figure on both bars (V-6: 124, 115 and 79 s against 177, 150 and 97). Before meshing the upper figure is still short on a part with fillets: V-6 took 126 s where the forecast was 105 s (20 % more), and its third mesh counted 196,605 nodes (4.9 GB by the model) where the forecast was 155,255 (3.8 GB), because curvature sizing adds nodes the part's size does not show. V-7, with no fillet, took 68 s under its 104 s. The dry run's sentence worded its figures before they were rounded and the card after (V-6: 1.5 min against 2 min); #196 alone does it too. Fixed in #196 on 2026-11-25: both word the figures given, and `memory_gb` has three decimals. The gates' own memory model is decision 22's, not touched here.
- **Four more things the merge needed, two of which git did not report.** All four are in the PRs since 2026-11-27: #173's merge shortens four lines of `AGENTS.md` (6,499 of 6,500 characters); #196's `packages/assistant-web` takes `develop`'s #178 and #221; #205's test calls `session_new_params` with `develop`'s arguments; and #196 shows the card's setup lines under the request. Below #196 (#172 to #192), the bottom-centre Assistant still cuts the card to two lines.
- **Dimension checks and the wording check (decision 25).** `develop`'s `intcad-context` skill asks the built-in agent whether each requirement passes, which #207 warns about in a turn with an estimate. Letting measured dimensions through was tried and dropped: it also let real verdicts on strength through. Saying "meets" and "does not meet" in the skill instead is in no PR; it is Tianwei's decision.
- **A solve in a batch, in the chat.** Its card cannot show the setup. Refusing it, and telling the agent to use `execute`, is in no PR yet; it is Tianwei's decision. Recommended.
- **Simulation panel.** `develop`'s right rail has a Simulation panel that holds nothing. Listing the estimate there, with a command that reads the last result back as `inspect.dfm` does for DFM, is not built. Recommended after the set has merged.

Found on 2026-11-27, merging `develop` e401f8a18 into #169 to #192 (checked again on 83d02730b). Not yet decided, except the charge (decision 30):
- **No agent can run the estimate in the desktop app.** `develop`'s #240 refuses every controlled entry (the chat's agents, the packaged Agent, the control port) before a request answered on a copy of the design starts: "this controlled entry needs a durable commit path that is not available yet". #172 answers the estimate on a copy, so the dry run and the solve are refused after Allow. A probe on the Linux workstation showed it for the MCP bridge's entry and the control port's, dry run and solve alike; the user's own entry is not refused. `develop`'s own `measure` and `inspect_design` take the same path. On the packaged Agent's route (#250) the execution guard also refuses a stamped estimate as unsupported (`execution_guard.rs`, `check_step`; `crates/mcp/src/document_guard.rs` has the same list). Decision 30 says an agent's estimate counts toward credits, so what is missing is a charged admission for a read on a copy that commits nothing: the grant carried through the copy's worker to settlement (`control_job.rs` `admit_copy_path` and `answered`, `control.rs` `commit_decision` and `settle_result`). That is the account code's work (its comments call it Task 8/9), with a bound for long runs (next item).
- **Runs longer than the paid-call bound lose their result.** A signed-in desktop admits a chat agent's call for 280 s (docs/account/control-safe-stop-bound.md). A result after that never reaches the design and is discarded, while the run goes on. An estimate may take up to an hour (#205). Recommended: refuse a run whose forecast (#196) is over the bound, or measure a bound of its own as the Review panel has.
- **The packaged built-in Agent does not get #205's wait.** Its CAD calls go over ACP, one at a time, and wait up to an hour in the app, so the agent cannot ask `pending` meanwhile. Decision 29.
- **The bottom-centre Assistant covers the running estimate's status and Stop** (`develop`'s #221, on by default). The chat's own Stop still stops the run.
- **Bundled or installed by the user: gmsh and CalculiX (bar 3's packaging item).** Left to Huaiyuan (Tianwei, 2026-11-28). Today the user installs both and the app checks their versions. Bundling them in the installers needs an exception to `AGENTS.md`'s rule against copying GPL code, like its Mastra exception, and the GPL's licence-text and source obligations for the versions shipped (gmsh 4.12.1, CalculiX 2.21).

## 9. Next steps

Phase A's first three steps (A1 to A4, A8 to A10, and parts of A6 and A11 to A13) landed in draft PR 175 as the fixes for the review's D1 to D12. What is left, each as one PR stacked on PR 175, with its acceptance tests:
1. C8 + C1 + C2: strain, the point probe, and stress components and principal stresses. In draft #190 (`fea/7-strain-probe`): `probes` on the estimate (points, an optional gauge direction), read inside the solver's element; principal stresses and strains at the away peak; V-C1, V-C2 and V-C6 pass. Still to do for C1 and C2: probes on a face, an edge or a hole, the click in the viewport, and colour maps of principal stress and strain (with C7).
2. A7 + A5 (the first step of INT-1498, the shortest path to showing the Stress button): repeatable runs with the versions in the reply, and reactions per support with the moment balance. In draft #192 (`fea/8-reactions-repro`), stacked on #190; V-A4, V-A5 and V-A7 pass.
3. A6: the three-mesh error band. Done in draft #195 (2026-11-20), based on `fea/8-reactions-repro`.
4. A11 and A13's remainders (thickness bands per grade; the approval card's predicted nodes, memory and time), and A14: the suite in CI, on Linux first (decision 14). A11 and A12 in draft #193 and A14 in draft #194 (2026-11-20), and A13 in draft #196, each based on `fea/8-reactions-repro`.
5. Decision 22: A1's gate and the mesh check's gate on A13's memory model, after #195 merges. Recommended before a release shows the Stress button (section 8).
6. B1 to B3: bolts, bearing loads and pins, after the one-day CalculiX keyword spike (section 4.2).
7. Decisions 23 to 27, the chat's use of Mastra: 23's four modelling skills landed in #204 (2026-11-23), and its analysis guide is #199, after this stack; 26 is in draft #205, 24 in #206 and 25 in #207.
8. Bar 3 (section 1, decisions 31 and 35), besides Phase A's merged PRs and step 5: the free run by hand (#270); one manual run on Windows; runs by hand on R2's plate and the L bracket; a warning for gmsh and CalculiX versions the suite has not passed; and the gmsh and CalculiX packaging and dependency audit for the release. Bar 3b, after it: an agent route that the typed admission of `develop`'s #240 accepts, charged as decision 30 says (#269), a bound or a refusal for runs longer than the paid-call bound (section 8, 2026-11-27), and one run with a real model in the chat with the whole set.
9. **Bring the set up to `develop`:** done on 2026-11-27 with e401f8a18 and 83d02730b, for #169 to #196 and #205 to #208. Left out, as Tianwei's decisions: `solidcraft-analysis` in the built-in agent's skill set (#199), the "meets" wording in `develop`'s `intcad-context` skill, and refusing a solve in a batch. `ci` runs on a draft only once its base is `develop`.

10. The switch (decision 32): the Stress button hidden until #270 is in, and the agent route until bar 3b. In draft #272, on #192; #270 and #206 each take one commit for it.

The rest of A11 to A13 goes with whichever PR touches the same code.
