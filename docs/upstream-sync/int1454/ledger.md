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

## 1. Kernel / geometry (INT-1490)

| ID | Upstream | Function | IntCAD today | Licence | Deps | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| K1 | 701 commits touching `solidcore/`; `524be9e1…` tree `8b5c0144…` | Vendored kernel snapshot: B-rep, booleans, blends, tessellation, STEP, difftest/goldens harness, plan/notes | Pins `solidcore` `8ddce901…` + `solidcore-record` `f5e8e9a2…` by git rev; **3,501 commits behind**, disjoint history | Apache-2.0 with its own LICENSE/NOTICE/ATTRIBUTION | none | **Adopted** (snapshot replacement) |
| K2 | 158 commits, `crates/kernel` | Kernel boundary: booleans, blends, chamfers, shell, offset, body/record, measures | 6 IntCAD files touched, 3 overlap (`ops.rs`, boolean property tests) | MIT OR Apache-2.0 | K1 | **Adopted** |
| K3 | 185 commits, `smlib`/`geom`/`dynamics` | SMLib adapter, geometry primitives, multibody dynamics | kernel union-by-lump fix (IntCAD, `5ec3a4897`) must be re-checked | MIT OR Apache-2.0 | K1 | **Adopted**, keep IntCAD fix |
| K4 | 65 commits, `tests/kernel-cases`, `tests/oracle`, action matrix, hole/taper/ssi corpora, `examples/regressions` | Kernel acceptance corpora and oracles | IntCAD has the same rigs; corpus content diverged | MIT OR Apache-2.0 | K1, K2 | **Adopted** (corpora and expected values re-recorded only with evidence) |

Kernel is a **snapshot replacement**: develop's pinned revisions and v0.44.1's vendored tree have
no common ancestor (`git merge-base` exits 1), so this is not a fast-forward. The guard is the
kernel rig (`cargo xtask kernel-cases --known-failing`), not a textual merge.

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
| A3 | 5 commits, `crates/craft-plugin*` + engine plugin surface | Wasmtime plugin host: no WASI/network, fuel/memory/deadline caps, staged writes, folder grants, approval hash, safe archive install | **No plugin runtime** (`plugins.rs` is a metadata list) | MIT OR Apache-2.0; pulls `wasmtime` (Apache-2.0 WITH LLVM-exception) | D3, D5 | **Not Applicable** for this sync; adopting plugins is a new subsystem, not a sync |
| A4 | 44 commits, `ui-egui/src/agent_chat` incl. `7b22bb3e2` | Gate decides on the registered id, reads `command` or `id`, unknown ids refused, `plugin.run` asks | **Vulnerable**: case-sensitive gate vs case-insensitive engine, `id` ignored | MIT OR Apache-2.0 | — | **Adopted now** (security; see `security-findings.md` SEC-1) |
| A5 | `54b4bb3f4` (STEP decode), `18d82f584` (control) | Hostile-input and availability fixes | Both present in develop | MIT OR Apache-2.0 | K1/D5 for STEP | **Adopted** (SEC-2, SEC-3) |
| A6 | `12a9a7f0f`, `4d2691e63` | Default-off control channel; opt-in insecure HTTP | IntCAD defaults on; keys only to https/loopback | MIT OR Apache-2.0 | — | **Blocked** pending a Compatibility Decision — see §7 |

## 4. UI (INT-1493)

| ID | Upstream | Function | IntCAD today | Licence | Deps | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| U1 | 559 commits, `crates/ui-egui` (+155,110 lines) | Upstream egui front end: panels, ribbons, dialogs, assistant, viewport wiring | IntCAD fork: 648 files, 106 changed since fork, 63 overlapping upstream files | MIT OR Apache-2.0 | D3, U2, U3 | **Adopted as adaptation**: feature logic ported into IntCAD's fork; upstream files that import `ui-kit`/`look`/`theme` (200 of 626) cannot be taken verbatim |
| U2 | 50 commits, `viewport*`, `render`, `gpu`, `space-mouse` | Viewport crate split, renderer and GPU changes | IntCAD has `render`, `gpu`, `space-mouse`; no `viewport*` | MIT OR Apache-2.0 | — | **Adopted** (non-brand plumbing; required by `doc`/`engine`) |
| U3 | 73 commits, `ui-kit`, `look`, `theme`, `color`, `ui-text`, `ui-registry`, `display-units` | Brand UI kit (blocked) + open plumbing (adoptable) | IntCAD has an in-crate `ui_kit` module from the pre-brand snapshot | **Brand-restricted** for `ui-kit`/`look`/`theme`; MIT/Apache for the rest | — | **Blocked** (brand) / **Adopted** (plumbing) |

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

## 8. Open questions

1. Is `crates/step` kept locally (patched) until K1 lands, or is the STEP move part of K1 itself?
2. Does the vendored `solidcore/` model conflict with IntCAD's rev-pinning rule in
   `docs/compatibility.md`? The snapshot has disjoint history, so a subtree or a pinned mirrormust
   be chosen deliberately.
3. Which UI features are declared "backend migrated, UI not exposed"? Filled in by the UI unit
   report.
4. Independent review of `Already Covered` / `Not Applicable` rows: not yet assigned.
