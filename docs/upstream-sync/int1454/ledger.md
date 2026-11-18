# INT-1454 upstream-sync ledger

Upstream: SolidCraft **v0.44.1** = `e151d5ea6866fb055e011379153fb1cae0566f22`
IntCAD baseline: `origin/develop` = `0fd92bea266e7dcea5ce09676ed9ff9ae6c66d7f`
Range: `06749ac464e16357d0375b78e6b3cac214066493..v0.44.1` — **4,994 commits, 4,939 non-merge**.

Rules: every unit is `Adopted` / `Already Covered` / `Not Applicable` / `Blocked`. Aggregation is
allowed; the commit range and the query that reproduces its count are always given.
`Already Covered` and `Not Applicable` need evidence and an independent reviewer; `Blocked` needs
the owner's approval before it can count as settled.

Classification in this Phase 0 draft means **planned disposition**, not completed work. Nothing is
`Adopted` until its PR is merged and green.

## 0. Coverage

Every non-merge upstream commit is assigned to at least one unit by path. Counts overlap where a
commit touches several units, so they do not sum to 4,939.

| Unit group | Commits | Unit group | Commits |
| --- | --- | --- | --- |
| K1 solidcore subtree | 701 | A1 MCP | 31 |
| K2 kernel | 158 | A2 control channel | 12 |
| K3 geom/smlib/dynamics | 185 | A3 plugins | 5 |
| K4 kernel corpora | 65 | A4 agent gate | 44 |
| D1 format | 3 | U1 ui-egui | 559 |
| D2 doc | 296 | U2 viewport/render/gpu | 50 |
| D3 engine | 619 | U3 brand + UI plumbing | 73 |
| D4 sketch | 63 | B1 CI | 104 |
| D5 io/STEP | 80 | B2 xtask | 84 |
| X1 retired/moved paths | 2,690 | B3 scripts | 50 |
| X2 empty process commits | 29 | B4 packaging | 15 |
| | | B5 vendor | 4 |
| | | B6 manifests | 92 |
| | | B7 apps | 88 |
| | | B8 docs | 494 |

Reproduce:

```sh
git log --no-merges --format='%H' 06749ac4..v0.44.1 | wc -l            # 4939
git log --no-merges --oneline 06749ac4..v0.44.1 -- <unit paths> | wc -l
# X1 = commits whose every path is absent at v0.44.1 (retired or moved into solidcore/)
# X2 = 29 commits with an empty tree diff ("port: …" process markers)
```

The machine-readable per-commit inventory is `generated/commits.tsv`.

The unit tables that follow group paths into functional units, so their per-unit counts differ
from this path-based table. The path-based table is the coverage check; the unit tables are the
work plan. Both are reproducible with the commands above.

## 1. Kernel / geometry (INT-1490)

| ID | Upstream | Function | IntCAD today | Licence | Deps | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| K0 | 701 commits touching `solidcore/`; snapshot = `524be9e1…` (tree `8b5c0144…`) | Vendored kernel snapshot: B-rep, booleans, blends, tessellation, STEP, difftest/goldens harness | No `solidcore/` tree; git pins `8ddce901…` + `f5e8e9a2…`, **3,501 commits behind**, history disjoint; pin predates `solidcore-ops/-tess/-step` | Apache-2.0 (USD BRep port) + fixtures ODC-By-1.0 | toolchain 1.99.1 | **Adopted** — snapshot replacement, precondition for K1–K10 |
| K1 | 34 commits, kernel STEP/IGES I/O | Topology-first STEP reader + AP242 writer now in `solidcore-step` | Absent (fork state); `crates/step` still exists separately | MIT OR Apache-2.0 | K0 | **Adopted** |
| K2 | 24 commits, blend/fillet/chamfer | Blend refusals as data from ops, wider-than-face rounds, tangent-chain angle | Fork state | MIT OR Apache-2.0 | K0 (`solidcore-ops`) | **Adopted** |
| K3 | 35 commits, sweep/loft/boundary/surface | Sweep/loft correctness, planar loft walls via solidcore skin | Fork state | MIT OR Apache-2.0 | K0 | **Adopted** |
| K4 | 9 commits, boolean/split/sew/heal/delete-face | Booleans and splits move onto `solidcore-ops` | Fork state + IntCAD `union_by_lumps` (`ops.rs`, `5ec3a4897`) absent upstream → guaranteed conflict | MIT OR Apache-2.0 | K0 | **Adopted**, keep IntCAD fix |
| K5 | 9 commits (+16 file deletions), mesher → `solidcore-tess` | One mesher in `solidcore-tess`; kernel re-exports `MeshSpec/PlanStats` | 16 `kernel/src/tess/*` files, fork state | MIT OR Apache-2.0 | K0 | **Adopted** — public API break reaches `doc`, `io`, `gpu`, `render` |
| K6 | 8 commits, measure/mass properties | Exact volume/mass/centre, exact two-edge measures, cached boxes | Fork state | MIT OR Apache-2.0 | K0 | **Adopted** |
| K7 | 47 commits, kernel core/body/ops | Bodies become `solidcore_ops::Body`, exact edges/faces replace mesh reads | Fork state + `union_by_lumps` | MIT OR Apache-2.0 | K0 | **Adopted** |
| K8 | 25 commits, feature builders/perf/cancel | Emboss, helix, prism, offset, native bodies, parallel eval, cancel | Fork state | MIT OR Apache-2.0 | K0, K7 | **Adopted**, split per feature if review size demands |
| K9 | 167 commits, `crates/smlib` | SMLib wrapper re-targeted onto `solidcore_ops::Body` | One line changed since fork | MIT OR Apache-2.0; `offset.rs` adapted from ablasik/solidcraft (row already in ATTRIBUTION) | K0 | **Adopted**; seam list changes with it |
| K10 | 19 commits, `crates/geom` | Geometry data moves to `solidcore::eval`; exact B-spline math kept | Fork state | MIT OR Apache-2.0 | K0 | **Adopted** |
| K11 | 1 commit, `crates/dynamics` | Joint springs, motors, motion links | Fork state | MIT OR Apache-2.0 | — | **Adopted** (small, independent) |
| K12 | 56 commits, `tests/kernel-cases` | Expected values re-recorded after the kernel fixes; **no new cases** | Same 2,496 case files; IntCAD changed 2 (DFM exemptions) | MIT OR Apache-2.0; copied STEP corpus CC0-1.0 | K1–K8 | **Adopted** after the code, with before/after evidence per case |
| K13 | 7 commits, `examples/regressions` | 11 new regression designs | Fork state | MIT OR Apache-2.0 | data only | **Adopted** |
| K14 | 32 commits, `xtask/mesh-decisions.txt` | Removes doc-layer mesh decisions as exact geometry lands | Same as fork | MIT OR Apache-2.0 | doc exact-geometry work | **Adopted** with D2; the list may only shrink |
| K15 | 4 commits, layering/seam files | `xtask/solidcore-seam.txt`, `-paths`, `-public`, `docs/layers*` | Absent; the seam text in AGENTS.md is ahead of develop | MIT OR Apache-2.0 | K0, K5, K9 | **Adopted** after the deps exist |

Kernel is a **snapshot replacement**, not a bump: develop's pinned revisions and v0.44.1's vendored
tree have no common ancestor (`git merge-base` exits 1), and the pin does not even contain the
crates the new kernel imports. **No kernel unit can land before K0**, which makes kernel one
atomic PR (K0 + the code units it unlocks), guarded by `cargo xtask kernel-cases --known-failing`
and the oracle rather than by textual merging.

## 2. Document / format / engine / assembly / IO (INT-1491)

| ID | Upstream | Function | IntCAD today | Licence | Deps | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| D1 | 3 commits, `crates/format` | Format tag/registry/upgrade, ZIP container reader/writer, newer-file handling | **Crate absent**; container lives in `io/src/design.rs`, registry in `doc/src/format.rs` | MIT OR Apache-2.0 | — | **Adopted** |
| D2 | 296 commits, `crates/doc` | Document model, eval, drawings, joints/assembly, new features (cell grid, frame, notes, tolerance, render coats), format v52–v58 | 11 files touched by IntCAD, 9 overlap; IntCAD adds `sheet_dfm.rs` (2,803 lines) | MIT OR Apache-2.0 | D1, K2, U2 | **Adopted** + keep IntCAD modules |
| D3 | 619 commits, `crates/engine` | Command registry, 54 more commands (582→636 `CommandSpec`), schema campaign (`unconverted.txt` 339→7), undo/selection | 73 files touched by IntCAD, 35 overlap; IntCAD adds DFM, project, validation (~16k lines) | MIT OR Apache-2.0 | D1, D2, D4, D5, U2, A2, A3 | **Adopted** + keep IntCAD modules |
| D4 | 63 commits, `crates/sketch` | 3D sketching (wire solve, constraints, profiles, pierce), spline handles, linked rebuild | **Identical to fork** (no IntCAD change) | MIT OR Apache-2.0 | K1 | **Adopted** |
| D5 | 80 commits, `crates/io` + `crates/step` + STEP corpora | Design containers, import cache, DXF rewrite, new importers, STEP reader moved to `solidcore-step` | `crates/io` 39→73 files; IntCAD adds `candidate.rs` (738 lines), snapshot budget; `crates/step` removed upstream | MIT OR Apache-2.0 | D1, D2, K1 | **Adopted**; the STEP move is **Blocked** until K1 lands |
| D6 | 65 format change files in `v52..v58`; `v51` has 15 | Format steps: assembly constraints, 3D sketch, joints, sheet, sweep, placements, saved face points, split tool | develop writes `solidcraft/51` + 5 `format_pending` names | MIT OR Apache-2.0 | D1, D2 | **Adopted** — see §6 |

`D4` is the cleanest unit: upstream `crates/sketch` is new work on an untouched IntCAD file set.

## 3. Assistant / MCP / control / plugins / security (INT-1492)

| ID | Upstream | Function | IntCAD today | Licence | Deps | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| A1 | 31 commits, `crates/mcp` + new `mcp-server` | MCP machinery split, 25→27 tools (`version`, `cancel`), prompts/resources/completions, compact replies, batch from script | Flat `crates/mcp` + IntCAD `document_guard` (guarded envelopes); no `mcp-server` | MIT OR Apache-2.0 | D3, A2 | **Port-selective**: `version` (and `cancel` if the agent-task controller needs it); the crate split is deferred |
| A2 | 12 commits, `crates/control*` | Token transport/protocol split, shared client, `TOO_MANY_CONNECTIONS`, live enable/disable, extra methods | App-local `control_server.rs` + `engine/src/control_auth.rs`; IntCAD method `agent.task.revalidate` | MIT OR Apache-2.0 | — | **Port-selective**: refusal-with-drain; crate extraction **Not Applicable** now |
| A3 | 26 commits, `crates/craft-plugin*` + engine plugin surface | Wasmtime plugin host: no WASI/network, fuel/memory/deadline caps, staged writes, folder grants, approval hash, safe archive install | **No plugin runtime** (`plugins.rs` is a metadata list) | MIT OR Apache-2.0; pulls `wasmtime` (Apache-2.0 WITH LLVM-exception) | D3, D5 | **Blocked — product decision** (CD-5): new subsystem, new execution surface, new dependency and packaging story; not a migration of existing behaviour |
| A4 | 44 commits, `ui-egui/src/agent_chat` incl. `7b22bb3e2` | Gate decides on the registered id, reads `command` or `id`, unknown ids refused, `plugin.run` asks | **Vulnerable**: case-sensitive gate vs case-insensitive engine, `id` ignored | MIT OR Apache-2.0 | — | **Adopted now** (security; see `security-findings.md` SEC-1) |
| A5 | `54b4bb3f4` (STEP decode), `18d82f584` (control) | Hostile-input and availability fixes | Both present in develop | MIT OR Apache-2.0 | K1/D5 for STEP | **Adopted** (SEC-2, SEC-3) |
| A6 | `12a9a7f0f`, `4d2691e63` | Default-off control channel; opt-in insecure HTTP | IntCAD defaults on; keys only to https/loopback | MIT OR Apache-2.0 | — | **Blocked** pending a Compatibility Decision — see §7 |

## 4. UI (INT-1493)

The UI cannot be migrated as code: upstream's front end is split into crates IntCAD does not have
and must not take, and 200 of its 626 `ui-egui` files import them. What moves is **functional
logic**, adapted into IntCAD's fork (537 upstream-changed files; 63 also changed by IntCAD).

| ID | Upstream | Function | IntCAD today | Licence | Deps | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| U1 | 30 commits, `viewport*` split (`5c99792a8`) | Render math, headless picture, CPU raster, GPU pipelines split out of `render`/`gpu` | Divergent `render`/`gpu` (same content, older) | MIT OR Apache-2.0 | `solidcore-tess` | **Not Applicable as a refactor**; individual fixes portable |
| U2 | 145 commits, camera/navigation fixes | Upright orbit from a rolled view, perspective zoom clamp, orthographic depth drift, polyline picking | Tests absent by name | MIT OR Apache-2.0 | K5 (mesher) for some | **Adopted** (adapt into `render::camera`) |
| U3 | 30 commits, viewport-gpu draw/perf | Indirect instanced draws, while-loops in `tess.wgsl`, capture OOM reporting, hover edges, drag layers | `tess.wgsl` 318 vs 378 lines; capture 668 vs 881 | MIT OR Apache-2.0 | K5 | **Adopted** (adapt) |
| U4 | `7b90a309a` + `82660d4ee` | Grid spacing moved to viewport; `color` crate (sRGB, analysis colours) | `the_grid_steps_by_one_two_and_five_with_the_view` already present; `AnalysisColors` in `render/src/analysis.rs` | MIT OR Apache-2.0 | — | **Already Covered** (grid, analysis) / **Not Applicable** (colour dedup) |
| U5 | `a833316f8` | `display-units` crate (152 lines, 6 tests) | `doc/src/display_units.rs` is a **932-line superset** with the same API | MIT OR Apache-2.0 | — | **Already Covered** — do not import |
| U6 | `82660d4ee` + 301 locale commits | `ui-text` machinery, message ids, locale growth (5,702→7,020 keys, +3 languages) | `ui-egui/src/i18n.rs` + `bidi.rs` (identical apart from visibility) | MIT OR Apache-2.0 | — | **Already Covered** (machinery) / **Adopted** (message ids used by migrated UI only) |
| U7 | `82660d4ee` | `ui-registry` handle table (233 lines) | `ui-egui/src/scenario.rs` (1,219 lines) has the same functions; `ui.*` control methods present | MIT OR Apache-2.0 | — | **Already Covered** — do not import |
| U8 | 39 commits, `control`/`control-egui` split | Transport split, typed params, runtime on/off switch | In-crate `control.rs` (3,768 lines) | MIT OR Apache-2.0 | — | **Not Applicable** (refactor) / **Adopted** (`control_switch` behaviour, typed params) |
| U9 | 64/13/0 commits, `ui-kit`/`look`/`theme` | Ribbon, chrome, Preferences, start page; icons; tokens | In-crate `ui_kit` module (66 files, 34,203 lines, +15 IntCAD lines) | **LICENSE-BRAND — Blocked** | — | **Blocked** |
| U10 | 414 icon files, `crates/look/icons` | Every post-brand icon lives only in the brand crate | `assets/icons/ui` 400 files (pre-brand, unchanged but 2) | **LICENSE-BRAND — Blocked** | — | **Blocked** — draw in-house or reuse pre-brand |
| U11 | ~101 commits, panel foundation | Dock/partition engine, drag paint, production descriptors | `panel_runtime` 6 files vs 8 upstream | MIT/Apache code, kit-coupled | U9 | **Blocked** (unusable without brand crates) |
| U12 | 12 + 32 commits, viewport interaction | Snap-point glyphs, drag layer, pen, edge detail, silhouettes | Engine support present (`engine/src/snap.rs` identical); all 7 UI files absent | MIT OR Apache-2.0 | U3, D3 | **Adopted** — best value/lowest brand risk |
| U13 | 37 commits, timeline | Fold chips, folders, component scope, per-design fold memory | Timeline exists; fold/scope files absent | MIT/Apache (1 kit ref each) | D3 | **Adopted** (adapt) |
| U14 | 153 commits, dialogs | Constraint picker, end caps, airfoil, auto-dialog | Files absent; `sketch.insert_airfoil` absent | MIT/Apache | D3 | **Adopted** except airfoil (**Blocked** on engine command) |
| U15 | browser/navigation chrome | Orbit/roll controls, design views, new window, view slots | Partial; `tab.view.*` absent | MIT/Apache | D3 | **Adopted** (design views only) |
| U16 | library/start page/links | Folder library, thumbnails, start page, link registry | `home.rs` exists; library absent | MIT/Apache | D3 | **Adopted** (adapted, IntCAD URLs) |
| U17 | diagnostics/export/mesh units | Problem tickets, signature verify, export name templates, mesh-unit dialog, safe graphics | All absent | MIT/Apache | D3, host startup | **Adopted** for export/mesh units/view memory; **Blocked** for tickets (upstream service) and Windows policy |
| U18 | plugin UI | Install/approve/menus/slots | Metadata list only | MIT/Apache | A3 | **Blocked** (no plugin runtime) |
| U19 | 43 commits, render workspace | Live/prepare/quality/displayed/surface-import panes | 16 files, lacks 5 panes | MIT/Apache | D3 | **Adopted** (adapt; largest surface) |
| U20 | `16f3b939b` | egui/eframe 0.36 → 0.37 | 0.36 | MIT/Apache | vendor re-base | **Adopted** (prerequisite for wholesale backports) |
| U21 | 0 commits, `space-mouse` | — | Identical to fork | MIT/Apache | — | **Not Applicable** |
| U22 | font asset | Korean UI face (OFL) | Missing | OFL 1.1 | — | **Adopted** (asset, row first) |

**Backend migrated, UI not exposed** (recorded, not counted as complete): snap points, silhouettes,
timeline groups/folders, assembly-constraint dialogs, frame end caps, render-job panes, import
report. Each has engine support in develop and no UI entry today.

**Licence-uncertain, treat as unusable until legal says otherwise**: post-brand changes to
`ui-egui` chrome files that implement components `LICENSE-BRAND` §1(a) names — `titlebar.rs`,
`toolbar.rs`, `toolbar_custom.rs`, `shell.rs`, `prefs*`, `workspace.rs`, `documents.rs`,
`starting.rs`, `home.rs`, `navigation_bar_view.rs`. The files carry the workspace MIT/Apache
licence, but their content is the licensed interface; §5 grandfathers only versions published
before the licence took effect.

## 5. Build / platform / CI (INT-1494)

| ID | Upstream | Function | IntCAD today | Licence | Deps | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| B1 | 104 commits, `.github/**` | Job split (`quick-compile`, sharded GPU tests), nightly alert, test builds, solidcore CI | IntCAD CI with `scripts/ci/plan.py`, Linux CI + Windows smoke jobs | repo code | — | **Portable subset**: `quick-compile`, GPU sharding, nightly alert. **Not Applicable**: solidcore mirror/release jobs |
| B2 | 84 commits, `xtask` | `fmt` (Windows command-line limit), `shared`, parallel/params/rustfmt modules, layers `Shared` class | All commands except `fmt`/`shared` | MIT OR Apache-2.0 | — | **Adopted**: `fmt`; `shared` only with the vendored-solidcore model |
| B3 | 50 commits, `scripts/**` | pr-check kernel rule, pending-format retry, mirror/updater scripts | IntCAD superset of CI scripts | MIT OR Apache-2.0 | — | **Portable subset**; mirror/updater **Not Applicable** |
| B4 | 15 commits, `packaging/**` | Windows GPO templates, shortcut lint, CLI entitlements | IntCAD packaging (`intcad` identity) | MIT OR Apache-2.0 | — | **Portable**: shortcut lint; GPO/branding **Not Applicable**; CLI entitlements only with plugins |
| B5 | 4 commits, `vendor/**` | egui-wgpu 0.37 re-base, winit fixes, accesskit_macos removal | IntCAD vendors egui-wgpu/winit/accesskit_macos at 0.36 | MIT/Apache | B6 | **Adopted with the egui bump** |
| B6 | 92 commits, manifests/lock/toolchain/deny/nextest | egui 0.36→0.37, toolchain pin 1.99.1, new deps (pgp, coset, ed25519, sysinfo, wasmtime), advisory ignores | No `rust-toolchain.toml`; lock 716→815 packages; third-party list 16,162→21,374 lines | mixed, all permissive (verified: 0 copyleft expressions) | — | **Adopted selectively**: toolchain pin, egui bump; pgp stack only with D5 provenance; wasmtime only with A3 |
| B7 | 88 commits, `apps/**` | App glue: stack size, single instance, web features | IntCAD apps with own identity | MIT OR Apache-2.0 | — | **Partial** |
| B8 | 494 commits, `docs/**`, skills, attribution | Documentation, command guides, screenshots, skill text | IntCAD docs | MIT OR Apache-2.0 (screenshots excluded) | — | **Portable subset**; screenshots **Not Applicable** |
| X1 | 2,690 commits on paths retired at v0.44.1 | `plan/`, `reports/`, `harness/`, `site/`, `fixtures/`, `crates/{difftest,goldens,solidcore*,step-import,step-brep,ssi,nurbs-grid-mesh}` | IntCAD never had most of these; `difftest`/`goldens` were upstream-only tooling | MIT OR Apache-2.0 | — | **Not Applicable** — tree absent at the target, content moved into `solidcore/` (K1) or retired |
| X2 | 29 empty commits | `port: …` process markers with no tree change | — | — | — | **Not Applicable** — no content to migrate |

## 6. Persistence (verified)

- develop writes `{"format":"solidcraft/51","format_pending":[5 names]}`; v0.44.1 writes
  `{"format":"solidcraft/58"}`.
- The five IntCAD pending changes are **byte-identical blobs** to files upstream folded into
  `v51/` (`git rev-parse` on both sides: `5f960278`, `34c49ca9`, `481269c2`, `2b6ed033`,
  `0058834c`). IntCAD's "own" pending flow is upstream's mechanism at the fork state.
- Therefore an existing IntCAD file is a **prefix subset** of upstream `51`, and adopting upstream
  requires no data migration and no parallel storage path — which satisfies the single-
  implementation rule.
- Two promises change and need the owner's approval (§7): new files are format 58 (older IntCAD
  builds cannot open them), and a file from a newer build changes from "refused" to "opened, the
  unreadable parts kept, re-saved under its own number".

## 7. Compatibility Decision candidates

| ID | Question | Evidence | Status |
| --- | --- | --- | --- |
| CD-1 | Adopt format 58 and open-newer-and-preserve, accepting that older IntCAD builds cannot open new files? | §6; `doc/src/format.rs`, `format/src/newer.rs` | **Waiting for owner** |
| CD-2 | Rename/alias MCP tools if upstream changed the exposed surface? | A1 | Waiting for A1 detail |
| CD-3 | Control channel default on→off (upstream `12a9a7f0f`)? | SEC-4 | Waiting for owner |
| CD-4 | Adopt opt-in plaintext HTTP for private endpoints (`4d2691e63`)? | SEC-4; IntCAD today is stricter | Waiting for owner |
| CD-5 | Adopt upstream's plugin runtime (Wasmtime, folder grants, install approval)? | A3; new subsystem, new execution surface, +`wasmtime`/`cranelift`, packaging entitlements | Waiting for owner |
| CD-6 | May IntCAD reuse post-brand changes to `ui-egui` chrome files named in `LICENSE-BRAND` §1(a)? | U9/U10/U11 note; files carry MIT/Apache but implement licensed interface | Waiting for owner/legal — default answer is **no** |

## 8. Conflict watchlist

Files changed by both sides, with the IntCAD work that must survive. This is the merge checklist,
not a wish list.

| Area | File(s) | IntCAD work to preserve |
| --- | --- | --- |
| kernel | `crates/kernel/src/ops.rs`, `boolean_history_tests.rs`, `tests/boolean_props*` | by-lump union of a multi-lump body (`5ec3a4897`, #55/#60) |
| doc | `src/format.rs`, `src/eval.rs`, `src/drawing*.rs`, `src/appearance_pack.rs` | IntCAD naming, `sheet_dfm.rs` integration, format messages |
| engine | 35 files incl. `tests.rs`, `boolean_regression_tests.rs`, `capture.rs`, `cmd/bom.rs`, `schema/snapshot.rs` | DFM/project/validation modules, snapshot budget, agent-facing commands |
| io | `src/design.rs`, `src/formats.rs`, `src/lib.rs`, `src/legacy_dir.rs` | `candidate.rs`, container snapshot budget, IntCAD extension handling |
| mcp | 7 of 9 touched files | `document_guard.rs` guarded envelopes, HostDispatch gating |
| ui-egui | 63 files | IntCAD panels, Assistant, DFM/Review/Files, identity strings |
| xtask | `layers.rs`, `lists.rs`, `licences.rs`, `kernel_cases.rs` | IntCAD seam lists and sorted-list rules |
| CI | `ci.yml`, `pr-checks.yml`, workflows | IntCAD job model, Windows smoke jobs, naming |

## 9. Open questions

1. Is `crates/step` kept locally (patched) until K1 lands, or is the STEP move part of K1 itself?
2. Does the vendored `solidcore/` model conflict with IntCAD's rev-pinning rule in
   `docs/compatibility.md`? The snapshot has disjoint history, so a subtree or a pinned mirrormust
   be chosen deliberately.
3. Which UI features are declared "backend migrated, UI not exposed"? Filled in by the UI unit
   report.
4. Independent review of `Already Covered` / `Not Applicable` rows: not yet assigned.
