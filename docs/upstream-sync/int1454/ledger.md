# INT-1454 upstream-sync ledger

## Validation terminal transaction (owner-approved, 2026-11-19)

The owner approved the bounded cross-process store transaction and background
cancellation repair after Sol diagnosis and Astra architecture audit. This fixes
an inherited cancel/completion race; it does not change protocol, authorization,
terminal states, acceptance or verified-report-before-marker recovery precedence.

Implementation: `docs/superpowers/plans/2026-11-19-validation-terminal-transaction.md`.
Base: `c5f2f42fe`. Task branch: `fix/int1454-validation-transaction`.
Status: test-first implementation in progress; no passing validation claimed.
Local baseline/builds NOT_RUN under the disk-growth gate. Use remote CI.
Stop all validation participants before upgrade or rollback. Keep the stable
transaction lock file and run records; mixed old/new concurrent writers are not supported.

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

Coverage is computed by `tools/coverage.py`, which assigns **every** non-merge commit in
`06749ac4..v0.44.1` to at least one path area and exits non-zero if one is unassigned. Run:

```sh
python3 docs/upstream-sync/int1454/tools/coverage.py        # 4939 commits, "all commits assigned", exit 0
```

The same pathspecs also cover **every distinct path** in the range (15,357 paths, 0 unmatched),
so nothing hides in a commit that happens to touch another area too.

Result (a commit touching several areas is counted in each, so counts overlap):

| Area | Commits | Ledger units |
| --- | --- | --- |
| kernel | 884 | K0–K15 |
| doc-engine | 753 | D1–D6 |
| assistant | 80 | A1–A7 |
| ui | 700 | U1–U22 |
| build | 298 | B1–B9 |
| apps | 88 | B7 |
| docs | 494 | B8 |
| test-policy | 24 | B9 |
| retired / moved | 2,700 | X1 |
| empty (no tree change) | 29 | X2 |

Two further facts, measured separately:

- Commits whose **every** path is absent at v0.44.1: **2,654**. The larger 2,700 above is the
  path-based "retired" area, which also counts commits that touch a retired path *and* a live one.
- Of the 29 empty commits, **7** have a `port:` subject; the rest are `blend:`, `wire:`, `fix:`,
  `difftest:`, `solidcore:` or `Revert` markers with no tree change.

The unit tables below group paths into functional units, so their counts differ from this table.
The areas are the coverage check; the units are the work plan. `generated/commits.tsv` is a
per-commit inventory with area labels; it is a navigation aid, not the coverage proof.

## 1. Kernel / geometry (INT-1490)

| ID | Upstream | Function | IntCAD today | Licence | Deps | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| K0 | 701 commits touching `solidcore/`; snapshot = `524be9e1…` (tree `8b5c0144…`) | Vendored kernel snapshot: B-rep, booleans, blends, tessellation, STEP, difftest/goldens harness | No `solidcore/` tree; git pins `8ddce901…` (solidcore) + `f5e8e9a2…` (record), 282 commits apart; the pin predates `solidcore-ops/-tess/-step` | Apache-2.0 (USD BRep port) + fixtures ODC-By-1.0 | toolchain 1.99.1 | **Adopted in progress**: `feat/int1454-kernel-snapshot` vendors the tree and ports kernel/smlib/geom/dynamics; workspace compiles; kernel rig pending |
| K1 | 34 commits, kernel STEP/IGES I/O | Topology-first STEP reader + AP242 writer now in `solidcore-step` | `crates/step` removed with K0; the reader is now `solidcore-step` in the vendored tree | MIT OR Apache-2.0 | K0 | **Adopted**; SEC-2 linear decoder confirmed present |
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

Kernel is a **snapshot replacement**, not a bump: in the solidcore repository the pinned
`solidcore` revision `8ddce901` and the vendored snapshot `524be9e1` have **no common ancestor**
(`git merge-base` exits 1), and the pin does not contain the crates the new kernel imports. The
distance is `git rev-list --count 8ddce901..524be9e1` = **3,501** commits. Measured with:

```sh
git clone --filter=blob:none --no-checkout https://github.com/tangency-labs/solidcore /tmp/opencode/solidcore
cd /tmp/opencode/solidcore
git merge-base 8ddce901671f1cebd9214a31d21895d725e886c8 524be9e16707f726586a7ab8696d87fd633e2096; echo $?   # 1
git rev-list --count 8ddce901671f1cebd9214a31d21895d725e886c8..524be9e16707f726586a7ab8696d87fd633e2096   # 3501
```

**No kernel unit can land before K0**, which makes kernel one atomic PR (K0 + the code units it
unlocks), guarded by `cargo xtask kernel-cases --known-failing` and the oracle rather than by
textual merging. K0 also vendors the kernel's **own** test suite and CI workflows; the kernel PR
must state which of those gates IntCAD runs in its CI and which it deliberately does not, rather
than dropping them silently.

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
| A3 | 26 commits, `crates/craft-plugin*` + engine plugin surface | Wasmtime plugin host: no WASI/network, fuel/memory/deadline caps, staged writes, folder grants, approval hash, safe archive install | **No plugin runtime** (`plugins.rs` is a metadata list) | MIT OR Apache-2.0; pulls `wasmtime` (Apache-2.0 WITH LLVM-exception) | D3, D5 | **Blocked — CD-5 approved deferral.** Unblock: a product decision to ship plugins, a licence/packaging review of `wasmtime`/`cranelift`, and a security review of the capability model. Not Not Applicable |
| A4 | 44 commits, `ui-egui/src/agent_chat` incl. `7b22bb3e2` | Gate decides on the registered id, reads `command` or `id`, unknown ids refused, `plugin.run` asks | **Vulnerable**: case-sensitive gate vs case-insensitive engine, `id` ignored | MIT OR Apache-2.0 | — | **Adopted now** (security; see `security-findings.md` SEC-1) |
| A5 | `54b4bb3f4` (STEP decode) | Linear-time Part 21 escape decode | **Absent**: `crates/step/src/p21.rs` still searches the tail per unterminated run (O(N²)); the fixed decoder is in the v0.44.1 snapshot (`solidcore-record/src/step/p21.rs`) | MIT OR Apache-2.0 | K0, D5 | **Adopted** via the snapshot, or backported earlier (SEC-2) |
| A6 | `18d82f584` (control) | Drain unread input past the connection limit so the refusal is received, not reset | Refusal **already sent** (`control_server.rs` sends `too many connections`), but the stream is dropped without a drain | MIT OR Apache-2.0 | — | **Adopted**: the message exists; the drain/shared-counter refinement is the work (SEC-3) |
| A7 | `12a9a7f0f`, `4d2691e63` | Default-off control channel; opt-in insecure HTTP | IntCAD defaults on; keys only to https/loopback | MIT OR Apache-2.0 | — | **CD-3: keep default on** (default change Not Applicable; applicable fixes still adopted). **CD-4: rejected** (plaintext opt-in Not Applicable) |

## 4. UI (INT-1493)

The UI cannot be migrated as code: upstream's front end is split into crates IntCAD does not have
and must not take, and 200 of its 626 `ui-egui` files import them. What moves is **functional
logic**, adapted into IntCAD's fork (537 upstream-changed files; 63 also changed by IntCAD).

| ID | Upstream | Function | IntCAD today | Licence | Deps | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| U1 | 30 commits, `viewport*` split (`5c99792a8`) | Render math, headless picture, CPU raster, GPU pipelines split out of `render`/`gpu` | Divergent `render`/`gpu` (same content, older); no `viewport` crate | MIT OR Apache-2.0 | `solidcore-tess` | **Adopted** — `doc` depends on `solidcraft-viewport` (6 modules) and the grid/pick logic lives there; the crate split is a prerequisite, not an optional refactor |
| U2 | 145 commits, camera/navigation fixes | Upright orbit from a rolled view, perspective zoom clamp, orthographic depth drift, polyline picking | Tests absent by name | MIT OR Apache-2.0 | K5 (mesher) for some | **Adopted** (adapt into `render::camera`) |
| U3 | 30 commits, viewport-gpu draw/perf | Indirect instanced draws, while-loops in `tess.wgsl`, capture OOM reporting, hover edges, drag layers | `tess.wgsl` 318 vs 378 lines; capture 668 vs 881 | MIT OR Apache-2.0 | K5 | **Adopted** (adapt) |
| U4 | `7b90a309a` + `82660d4ee` | Grid spacing moved to viewport; `color` crate (sRGB, analysis colours) | `AnalysisColors` exists in `render/src/analysis.rs`; the grid test `the_grid_steps_by_one_two_and_five_with_the_view` exists **only** upstream (`viewport/src/grid.rs:37`) | MIT OR Apache-2.0 | U1 | **Adopted** (grid) / **Already Covered** (analysis colours) / **Not Applicable** (colour dedup refactor) |
| U5 | `a833316f8` | `display-units` crate (152 lines, 6 tests) | `doc/src/display_units.rs` is a **932-line superset** with the same API | MIT OR Apache-2.0 | — | **Already Covered** — do not import |
| U6 | `82660d4ee` + 301 locale commits | `ui-text` crate (catalog lookup, substitution, RTL), message ids, locale growth (5,702→7,020 keys, +3 languages) | `ui-egui/src/i18n.rs` + `bidi.rs` are at **fork parity**; upstream moved both into `ui-text` (`ui-egui/src/bidi.rs` is absent at the target). Catalogs are 1,318 keys behind | MIT OR Apache-2.0 | — | **Already Covered** (machinery at fork parity) / **Adopted** (message ids used by migrated UI only) |
| U7 | `82660d4ee` | `ui-registry` handle table (233 lines) | `ui-egui/src/scenario.rs` (1,219 lines) has the same functions; `ui.*` control methods present | MIT OR Apache-2.0 | — | **Already Covered** — do not import |
| U8 | 39 commits, `control`/`control-egui` split | Transport split, typed params, runtime on/off switch | In-crate `control.rs` (3,768 lines) | MIT OR Apache-2.0 | — | **Not Applicable** (refactor) / **Adopted** (`control_switch` behaviour, typed params) |
| U9 | 64/13/6 commits, `ui-kit`/`look`/`theme` (`--full-history --no-merges`) | Ribbon, chrome, Preferences, start page; icons; tokens | In-crate `ui_kit` module (66 files, 34,203 lines, +15 IntCAD lines) | **LICENSE-BRAND — Blocked** | — | **Blocked (CD-6)** |
| U10 | 414 icon files, `crates/look/icons` | Every post-brand icon lives only in the brand crate | `assets/icons/ui` 400 files (pre-brand, unchanged but 2) | **LICENSE-BRAND — Blocked** | — | **Blocked (CD-6)** — draw in-house or reuse pre-brand |
| U11 | ~101 commits, panel foundation | Dock/partition engine, drag paint, production descriptors | `panel_runtime` 6 files vs 16 upstream | MIT/Apache code, kit-coupled | U9 | **Blocked (CD-6)** — unusable without brand crates |
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
| B4 | 15 commits, `packaging/**` | AppImage/dock icon, Flatpak hosted-AI, macOS stapler retry, Windows Server Core CLI, debug symbols, MSI shortcut icon, GPO templates | IntCAD ships Linux/macOS/Windows packaging with `intcad` identity | MIT OR Apache-2.0 | — | **Adopted**: the platform fixes in areas IntCAD ships; **Not Applicable**: GPO templates and brand-named MSI/registry ids only |
| B5 | 4 commits, `vendor/**` | egui-wgpu 0.37 re-base, winit fixes, accesskit_macos removal | IntCAD vendors egui-wgpu/winit/accesskit_macos at 0.36 | MIT/Apache | B6 | **Adopted with the egui bump** |
| B6 | 117 commits: manifests, lock, toolchain, deny, nextest, flake, `.gitignore`/`.gitattributes`, `.coderabbit.yaml`, `THIRD-PARTY-LICENSES.txt` | egui 0.36→0.37, toolchain pin 1.99.1, new deps (pgp, coset, ed25519, sysinfo, wasmtime), advisory ignores, flake git-dep fix | No `rust-toolchain.toml`; lock 716→815 packages; third-party list 16,162→21,374 lines | mixed, all permissive (0 copyleft expressions in either generated list) | — | **Adopted selectively**: toolchain pin, egui bump, flake fix; pgp stack only with D5 provenance; wasmtime only with A3 |
| B7 | 88 commits, `apps/**` | App glue: stack size, single instance, web features | IntCAD apps with own identity | MIT OR Apache-2.0 | — | **Partial** |
| B8 | 494 commits, `docs/`, `skills/`, `attribution/` plus root `README.md`, `CONTRIBUTING.md`, `AGENTS.md`, `CLAUDE.md`, `NOTICE`, `ATTRIBUTION.md`, `ROADMAP.md`, `SECURITY.md`, `DISCLAIMER.md`, `LICENSE-*` | Documentation, command guides, screenshots, skill text | IntCAD docs | MIT OR Apache-2.0 (screenshots excluded) | — | **Portable subset**; screenshots **Not Applicable** |
| B9 | 24 commits, `tests/KNOWN_FAILING_TESTS`, `tests/QUARANTINED_TESTS` | Upstream's known-failing and quarantine lists, incl. "11 tests regressed by solidcore c0d3cc0124" | IntCAD has its own 33-entry list and gate | MIT OR Apache-2.0 | — | **Not Applicable** as content (names upstream tests only) — but every entry is a **signal** to check in K12 |
| X1 | 2,700 commits touching paths retired at v0.44.1 (2,654 whose every path is absent) | `plan/`, `reports/`, `harness/`, `site/`, `fixtures/`, `crates/{difftest,goldens,solidcore*,step-import,step-brep,ssi,nurbs-grid-mesh,diagnostics,xtask}` | IntCAD never had most of these; `difftest`/`goldens` were upstream-only tooling | MIT OR Apache-2.0 | — | **Not Applicable** — tree absent at the target; content moved into `solidcore/` (K0) or retired |
| X2 | 29 empty commits (7 `port:`, the rest `blend:`/`wire:`/`fix:`/`difftest:`/`solidcore:`/`Revert`) | Process markers with no tree change | — | — | — | **Not Applicable** — no content to migrate |

Reproduce any row with `git log --no-merges --full-history --format='%H' 06749ac4..v0.44.1 -- <paths>`.
B6's full pathspec is `Cargo.toml Cargo.lock rust-toolchain.toml deny.toml clippy.toml rustfmt.toml
.cargo .config flake.nix flake.lock package.json package-lock.json tools .gitignore .gitattributes
.coderabbit.yaml THIRD-PARTY-LICENSES.txt` (117); dropping `package.json package-lock.json` and
`tools` gives 115. B8 additionally names the root documents listed in its row.

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

## 7. Compatibility Decisions — **approved 2026-11-17**

| ID | Question | Decision | Consequence in this ledger |
| --- | --- | --- | --- |
| CD-1 | Format 58 and the newer-file policy | **Approved**: adopt upstream format 58, Doc and the single persistence implementation; keep upstream's existing compatibility/save mechanisms; write no IntCAD-specific compatibility, backup or conflict layer. Verify opening an old design, save and reopen, and data integrity on real files. | D2, D6, §6 ≡ Adopted; the two promise changes are accepted (older builds cannot open new files; newer files open and are preserved) |
| CD-2 | MCP tool surface | **Approved**: first publish a map of old name → new name, input/output schema, behaviour, side effects, permissions, callers. Pure renames are replaced outright (registry, callers, skills, tests, docs), with no legacy-name compatibility layer. Schema changes that are semantically equivalent migrate together. Any functional, permission or side-effect change goes to a separate Compatibility Decision. New tools are audited individually. **Case and `id`/`command` aliases must take the same resolve, authorise and execute path.** | A1; SEC-1 is the first instance of the alias rule |
| CD-3 | Control channel default | **Approved**: keep IntCAD's current default **on**; absorb applicable security fixes; do not widen permissions. | A7 ≡ Already Covered for the default; upstream `12a9a7f0f`'s default change is Not Applicable |
| CD-4 | Opt-in plaintext HTTP | **Approved: rejected**. Keep the HTTPS/loopback-only key rule. | A7 ≡ Not Applicable for `4d2691e63` |
| CD-5 | Plugin runtime | **Approved: not enabled this round.** Record as Blocked/deferred with unblock conditions; not Not Applicable. | A3 ≡ Blocked (owner-approved deferral); U18 Blocked |
| CD-6 | Post-brand UI material | **Approved: do not introduce** brand-era `ui-kit`/`look`/`theme`/chrome/assets. Migrate only provably compliant functional logic, adapted to the existing egui + WebView. | U9/U10/U11 and the licence-uncertain chrome files ≡ **Blocked** by decision, permanently for this sync |
| CD-7 | Kernel delivery model | **Approved**: adopt upstream's vendored `solidcore` closure in full (ops/tess/step), remove the stale rev pins, update Cargo, lockfile, `docs/compatibility.md`. No two kernels may coexist. | K0 ≡ Adopted as a subtree; CD-7 closes open question 2 |

These are owner decisions, not implementer preferences. Changing one requires a new decision, not an
edit to this row.

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
2. ~~Vendored `solidcore` vs rev pin~~ — **decided by CD-7**: vendored closure, pins removed,
   `docs/compatibility.md` updated, one kernel.
3. Which UI features are declared "backend migrated, UI not exposed"? The list is in §4; the UI
   PR confirms it feature by feature.
4. Independent review: two rounds completed; defects fixed; the Phase 0 gate passed.
