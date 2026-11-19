# INT-1454 acceptance package (draft — incomplete until sign-off)

Status: the v0.44.1 sync is integrated in PR #185, head `099e4360c`. Its pull-request workflows
(`CI` 44472128741, `Kernel rig` 44472128721, `Fillet rig` 44472128632, `PR checks`, `Packaging lint`)
were running when this line was written. This package is not a completion claim; it records what
exists, what is verified, and what is still open.

## 1. Fixed references

| Item | Value |
| --- | --- |
| Upstream target | SolidCraft v0.44.1 = `e151d5ea6866fb055e011379153fb1cae0566f22` |
| Vendored kernel | solidcore `524be9e16707f726586a7ab8696d87fd633e2096`, tree `8b5c01445619305dcfc5072db3fe76b8cfea67be` |
| Pre-sync develop baseline | `0fd92bea266e7dcea5ce09676ed9ff9ae6c66d7f` |
| Fork point | `06749ac464e16357d0375b78e6b3cac214066493` |
| Sync head under review | `099e4360c` (PR #185) |
| Final develop SHA | **TBD after merge** |

## 2. Merged work

| PR | Scope | Merge commit |
| --- | --- | --- |
| #177 | Phase 0 archive: baseline, ledger, DAG, coverage tool, CD-1…7 | `2c46e8fba` |
| #180 | Rust 1.99.1 pin + 29 CI install sites | `3cb29651e` |
| #182 | `cargo xtask fmt` (Windows-safe batched rustfmt) | `aca5b768e` |
| #179 | SEC-1: one command id for judging, remembering and running | `96a247ccb` |
| #185 | Kernel, doc/format/IO, engine, MCP, GPU/viewport and UI sync | **TBD after merge** |

## 2a. Commit index by layer (55 commits above `develop` at `f82b56372`)

| Layer | Range (first..last) | Notes |
| --- | --- | --- |
| Kernel (K0–K15) | `06749ac4..d12c2fa5f` on `feat/int1454-kernel-snapshot` | vendored solidcore, kernel/smlib/geom/dynamics, STEP tree, rig and action-matrix re-records |
| Doc/format/IO | `30fe35c92..06e7aa667` | format/colour/display-units/viewport/sketch/render added, doc+io ported, attribution, format-58 note |
| Engine | `2ca1b7fd8..3e7a7aa46` | engine port, DFM/project/validation re-applied, plugin surface removed, known-failing shrink |
| MCP | `6e1bcf53a..701d274ca` | server split, guarded envelopes, `version`, `--compact`, CLI stdio fix |
| GPU/viewport | `9bc73566d` | viewport-gpu/egui added, `crates/gpu` compute-only |
| UI | `8c1fc4f6c..f82b56372` | egui adaptation, labels/help/dialogs, viewport API, new-file preference, wasm fix |
| Integration | `1a18423ed`, `b06d531e5`… | develop merge and the two rule fixes (defaults line, advisories) |

## 2b. Licence audit (summary)

All code adopted from SolidCraft is MIT OR Apache-2.0. The kernel is the vendored solidcore tree
(Apache-2.0; its own `LICENSE`, `NOTICE` and `ATTRIBUTION.md` are carried; USD BRep/SMLib credits
intact). The moved Liberation Sans font keeps its OFL row, the seven STEP regression fixtures have
rows in `attribution/pending/step-regressions-v0441.md`, and the FreeCAD fixtures/font moved with
the doc port have rows in `attribution/pending/doc-assets-v0441.md`. `cargo xtask licences --check`
reports the generated list will be rewritten on `develop` by the Generated files workflow; no
brand-era UI kit, look, theme or asset was imported (CD-6).

## 3. Ledger coverage

`tools/coverage.py` assigns every non-merge commit in `06749ac4..v0.44.1` and every distinct path
to an area, and exits non-zero on an orphan (4,939 commits, 15,357 paths, 0 unmatched at Phase 0).
The disposition table is `ledger.md`; the owner approved CD-1…CD-7 (§7 there).

Re-run on this branch (read-only, from the branch itself):

```
$ python3 docs/upstream-sync/int1454/tools/coverage.py
  doc-engine     753
  assistant       80
  ui             700
  build          298
  apps            88
  docs           494
  test-policy     24
  retired       2700
  (empty)         29
distinct paths: 15357 unmatched: 0

all commits assigned, every path matched
```

The ledger's `Already Covered`, `Not Applicable` and `Blocked` dispositions were reviewed
independently in Phase 0 (ledger §"Method", closing line: "Independent review: two rounds completed;
defects fixed; the Phase 0 gate passed"), and the one `Blocked` area carries CD-5's approved deferral.


## 3a. Compatibility Decisions and how each is implemented

| Decision | Implementation | Evidence |
| --- | --- | --- |
| CD-1: one persistence implementation, format 58 | `solidcraft_format`'s registry/container is the only writer; `doc`/`io` use it; the five Intcad `format_pending` names are the same bytes as v0.44.1's `v51` files | `design::tests::a_design_saved_at_format_51_with_its_pending_changes_saves_and_reopens`, the snapshot-budget test, and the GUI round trip in §5 |
| CD-2: MCP map, no legacy alias layer | 21 → 22 tools; `version` adopted; `cancel` and batch's `path` script input **not** adopted (no app `control.cancel`; a file-reading input is a behaviour change); every schema delta classified as equivalent or behaviour-changing | `docs/mcp.md`, the MCP slice review record in §5a, `crates/mcp/src/tests.rs` |
| CD-3: control channel stays on by default | no default change adopted; `crates/control` ported with its auth/loopback rules | `crates/control/src/server.rs` (byte-identical to v0.44.1), control tests 33 passed |
| CD-4: no opt-in plaintext HTTP | no plaintext transport was added; the loopback token rules are unchanged | `crates/control/src/auth.rs`, `crates/engine/src/control_auth.rs` shim |
| CD-5: plugin runtime Blocked, not "Not Applicable" | runtime **and** surface removed: no `plugin.*`, `file.import`, `model.mesh`, no `craft-plugin`/`wasmtime`/`cranelift` in manifests or lock | `Cargo.lock`, engine command registry, §7 |
| CD-6: no brand-era UI kit/look/theme/assets | Intcad's egui UI kept and adapted; no upstream UI or brand asset imported; the only moved assets are the Liberation Sans font (OFL) and the STEP/FreeCAD fixtures (attribution rows) | `git diff v0.44.1 -- crates/ui-egui` shows no wholesale import; §2b |
| CD-7: vendored solidcore closure, no rev pins | `solidcore/` byte-identical to `v0.44.1:solidcore`; `crates/step` deleted; `deny.toml` `[sources]` has no git sources | `git diff v0.44.1 -- solidcore` is empty and both trees hash to `8b5c01445619305dcfc5072db3fe76b8cfea67be`; `deny.toml` denies unknown git sources |

## 4. Gates and evidence (to be refreshed at the final SHA)

| Gate | Evidence |
| --- | --- |
| Build | `cargo check --workspace --all-targets --profile ci` clean |
| Lints | `cargo clippy --workspace --all-targets --profile ci -- -D warnings` clean |
| Kernel | 932 passed / 6 failed (all on `KNOWN_FAILING_TESTS`), rig areas and ladder as recorded |
| Doc/format/IO | 873 / 10 / 723 via `--profile ci`; format-51 file saves as 58 and reopens with identical data |
| Engine | 5309 passed / 4 failed (all listed); DFM acceptance 3/3; `schema/unconverted.txt` 339 → 7 |
| MCP | 163 / 14 / 33; guarded envelopes preserved; `version` adopted, `cancel` omitted |
| UI | 4997 passed / 2 environment-only failures; workspace compiles |
| Kernel rig | 17 of 19 areas green at `136223609` (import, inspect, history, frames, booleans, crashes, plastic, construction, continuity, fillets, sheet-metal, solids, surfaces, shell, tessellation, sweeps, threads). The two that differed were re-recorded at `e95afde4d` with the branch's own release CLI (`target/release/intcad-cli`, 864,557,376 bytes, software/vulkan): `--area assemblies` -> `96 pass, 0 known failing, no regression`; `--ladder --record` then `--ladder --gate` -> `170/170 pass, every case as the golden records it`; `--coverage` -> `229 commands have a case`. The ladder change is count-only for threads (v0.44.1 keeps one exact helical face: `06-threads/002-external-m10x1-fine` 6/12/8 where the golden had 126/372/248, volume unchanged at 2081.901104145 mm3 against the analytic 2081.901104148) plus two corner-fillet refusals whose reason moved to a tangent seam; one assembly case now succeeds because v0.44.1's engine carries a moved-in body's history (#12142). CI `Kernel rig` run `44454875225` on that head then completed **success**, all 19 areas including the re-recorded ladder and assemblies |
| Rules | `xtask layers`, `mesh-decisions`, `lists`, `assets`, `ui-containers`, `fmt` clean |
| CI | run IDs to be recorded from PR #185's final green run |

## 5. Persistence (CD-1)

The single implementation is `solidcraft_format`'s registry/container used by `doc`/`io`. Test
`design::tests::a_design_saved_at_format_51_with_its_pending_changes_saves_and_reopens` builds a
file exactly as Intcad 0.1.1 wrote it, saves it as `solidcraft/58` and reopens it with identical
data (1 passed on the branch; `a_snapshot_budget_bounds_the_design_container_it_writes` also 1
passed). GUI-level verification on the branch: Linux opened the real `solidcraft/1` corpus design
`modern-sheet-metal.solidcraft` (errors 0, features 2), saved it as a new file (16,508 bytes) and
reopened it with the same bodies, sketch (Contour, 3 curves, dof 8, solved) and timeline; Windows
opened the sample, saved it (`C:/Users/Public/cd1-win.sc`, 44,940 bytes) and reopened it (errors 0,
features 9, parameters preserved). The `open_newer_files` preference is implemented and its
refusal path is tested. All of this is on branch commits, to be repeated at the final SHA.

## 5a. Independent review record

| Review | Scope | Verdict | Fixes it forced |
| --- | --- | --- | --- |
| Kernel slice | vendored kernel, rig, ladder, action matrix | approved after 2 rounds | rig path names, empty renamed recipe, action-matrix baseline recorded on this stack, the exact-blend `splines` check reading a `Surface::Blend`'s net |
| Doc/format/IO slice | format 58, one persistence implementation | approved | attribution rows for the moved font and FreeCAD fixtures |
| Engine slice | registry, DFM/project/validation preservation, CD-5 surface | approved after 2 rounds | `control_auth` compatibility shim, plugin surface removed wholesale (not just the runtime), duplicate mesh-decision rows, `.solvecraft` doc comment |
| MCP slice | guarded envelopes, CD-2 map | approved after 2 rounds | CLI `!Send` stdio, `--compact` wired, CD-2 table corrections, ledger counts |
| Final branch review | post-approval deltas only | **changes requested** at `e95afde4d`, then re-reviewed twice | (1) staged/wasm opens ignored `open_newer_files`; (2) the preference applied only on the next frame, and Cancel/Defaults restored it without syncing the session; (3) runtime GPU render requests dropped directional/punctual lights; (4) `EXEMPT` put non-geometry commands in the geometry backlog. Fixed in `19396bb4e` and `7aa8e008e`; F1/F3/F4 and F2 were then confirmed pass by the same reviewer, with the F2 fix verified again after the develop merge |

The reviewer also corrected an overstated claim in an earlier draft of the pull request: `KNOWN_FAILING_TESTS`
has **33 entries on `develop` and 22 here** (11 removed because their test now passes, 2 reworded to name the
vendored kernel instead of the old pin, 0 added), not the "42 to 15" an earlier draft said.

## 6. Platform acceptance

| Platform | Status |
| --- | --- |
| Linux native CI | PR #185 pull-request runs at `099e4360c` (CI 44472128741, `Native Linux Clippy`, `Native Linux selected regression gate` in full scope; pending final) |
| Linux GUI E2E | **PASS 8/8** at `42f4d670a` (task `int1454`): sample open, orbit drag, face select, Assistant open/resize/control. Evidence `~/.local/state/intcad-e2e/runs/intcad-e2e-int1454-ui-c28767d6c085-int1454/20261118T212022Z-681395/REPORT.md`; artifact sha256 `e6da03a57c438530d1640a0a171b0f1f220768aa95e3a1d5a54cef246b5a83cc` |
| Windows native CI + VM GUI E2E | **PASS** at `f82b56372` (task `int1454w`): MSI `b1d55038d01c9452f4b534bfe93e8d365c0f08c2875518395aaccaf84890be4a`, app sha256 `ca1c34de2167f4b2ec028afe6403430d43a7a83a09ae4f7c6992dbcf036ba6c1`, installed and running, Sample Plate loaded, control channel ok. Evidence `~/.local/state/intcad-e2e/windows/runs/…/20261118T213755Z-734039` |
| macOS native automation | not run |
| macOS/desktop UI (PP) | not run |

## 6a. Security findings

| Finding | Status | Evidence |
| --- | --- | --- |
| SEC-1: the agent gate could be bypassed by command-id case or an id/command alias | **Fixed and merged** | PR #179 = `96a247ccb`; one `resolve_command_id` decides for the gate, the control channel and execution, with allow/ask/refuse and hostile-input regressions |
| SEC-2: STEP import decoded `\\X2\\`/`\\X4\\` strings in quadratic time (`54b4bb3f4`) | **Adopted with the kernel** | `crates/step` (the old per-run tail search) is deleted; the fixed decoder is `solidcore/crates/solidcore-record/src/step/p21.rs` (`decode_string_with`, disjoint stretches, linear however many unterminated runs). Its hostile-input test `run_terminators_are_searched_once` decodes 100,000 unterminated tags; no other Part 21 decoder remains in `crates/` |
| SEC-3: control channel refused a limited connection without draining (`18d82f584`) | **Landed, not yet wired** | `crates/control/src/server.rs` is byte-identical to v0.44.1 (`git diff v0.44.1 -- crates/control/src/server.rs` is empty) but the app still serves through `apps/solidcraft/src/control_server.rs`; moving the app onto the ported server is UI/platform-slice work and is listed as an open item |
| Plugins (Wasmtime) | **Blocked per CD-5** | Runtime and every plugin command removed; see §7 |

## 7. Open items and exceptions

- CD-5 plugin runtime: Blocked/deferred with unblocking conditions (ledger A3).
- CD-6 brand-era UI material: not imported.
- UI entry points still missing for: multi-plane sections, point-cloud segments, spline handles,
  joint plane/axis origins, design view tabs, airfoil import, several feature options, CPU-preview
  options, sparse panel bodies (ledger §4).
- SEC-3 drain: present in `solidcraft_control` but the app still runs its own control server.
- `--compact` wired for the CLI; the app does not answer `mcp.options` yet.
- `THIRD-PARTY-LICENSES.txt` is regenerated on `develop` by the Generated files workflow.

## 8. Rollback

The sync is one PR; reverting its merge commit returns `develop` to `0fd92bea2` plus the
independent PRs (#177, #179, #180, #182). Per-layer commits remain on their branches for partial
re-application. The kernel snapshot can be re-pinned only by reverting the whole PR: no second
kernel exists.

## 9. Sign-off

Required from the owner after: final develop SHA fixed, Linux GUI and Windows VM E2E recorded,
macOS automation and PP UI acceptance recorded, and this package refreshed with those artifacts.
