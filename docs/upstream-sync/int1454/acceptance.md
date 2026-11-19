# INT-1454 acceptance package (draft — incomplete until sign-off)

Status: PR #185 is **open** at remote head `099e4360c3c1a9e70582731801289d4b4a5a4570`
(`feat/int1454-ui`); GitHub reports `mergeStateStatus: BLOCKED`. The local integration worktree
has one unpushed commit on top of that head: `61835bfafb151c7d6803aeab195e824b6d8458d3`
(parent `099e4360c`), the seven-line engine known-failing deletion from `7ceea11d5` (22 -> 15).
It has not been reviewed, pushed or run through pinned CI here. At the 2026-11-19T08:49Z CI
snapshot, run 44472128741 attempt 2 at the old head had `Build full-baseline archive` success
(48m22), `Native Linux selected regression gate` failure with exit 143 during `cargo nextest list`
(no test receipt), Action matrix shards 1 and 15 success, other shards/properties/determinism in
flight, and no aggregate. This package is a draft, not a completion claim: it records what is
verified, what is local, what is historic, and what remains open. Sources: `gh pr view 185`,
`/tmp/opencode/int1454-ci-current/SNAPSHOT-2026-11-19T0849Z.md`,
`/tmp/opencode/int1454-engine-allowances/FIX-REPORT.md`, `git rev-parse` in the integration
worktree, 2026-11-19.

## 1. Fixed references

| Item | Value | Source / revision |
| --- | --- | --- |
| Upstream target | SolidCraft v0.44.1 = `e151d5ea6866fb055e011379153fb1cae0566f22` | `git rev-parse v0.44.1` |
| Vendored kernel | solidcore `524be9e16707f726586a7ab8696d87fd633e2096`, tree `8b5c01445619305dcfc5072db3fe76b8cfea67be` | `git rev-parse HEAD:solidcore` and `git rev-parse v0.44.1:solidcore` |
| Pre-sync develop baseline | `0fd92bea266e7dcea5ce09676ed9ff9ae6c66d7f` (historical baseline, **not** the revert target) | `git show -s 0fd92bea2` |
| Fork point | `06749ac464e16357d0375b78e6b3cac214066493` | `git merge-base v0.44.1 origin/develop` |
| Remote PR head under review | `099e4360c3c1a9e70582731801289d4b4a5a4570` (short `099e4360c`, PR #185) | `git rev-parse 099e4360c`; `gh pr view 185` |
| Local integration head | `61835bfafb151c7d6803aeab195e824b6d8458d3` (parent `099e4360c`; `feat/int1454-ui`, ahead 1, **not pushed**) | `git -C worktrees/int1454-ui rev-parse HEAD HEAD^`; `/tmp/opencode/int1454-engine-allowances/FIX-REPORT.md` |
| Landing method and commit | **TBD** (merge commit or squash; the revert target follows the chosen method) | to be recorded after landing |
| Final develop SHA | **TBD after landing** | to be recorded after landing |

## 2. Merged work

| PR | Scope | Merge commit |
| --- | --- | --- |
| #177 | Phase 0 archive: baseline, ledger, DAG, coverage tool, CD-1…7 | `2c46e8fba` |
| #180 | Rust 1.99.1 pin + 29 CI install sites | `3cb29651e` |
| #182 | `cargo xtask fmt` (Windows-safe batched rustfmt) | `aca5b768e` |
| #179 | SEC-1: one command id for judging, remembering and running | `96a247ccb` |
| #185 | Kernel, doc/format/IO, engine, MCP, GPU/viewport and UI sync | **TBD after landing** |

`develop` also contains independent work merged after the pre-sync baseline: #167 (Black theme /
0.1.2, via #186), #176 (layout-rect control query), #181 (Windows UIA E2E), #186 (main
reconciliation) and #188 (CJK font discovery). Any rollback of #185 must preserve those changes;
see §8. Source: `git log --first-parent 0fd92bea2..origin/develop`.

## 2a. Commit index (verified reachable commits; exact head index pending)

The earlier "55 commits above `develop` at `f82b56372`" count described `f82b56372`, not the
reviewed head. Do not use it as the head's count or as a PR-only count. The exact head index is
**pending**; the rows below are verified ancestors/endpoints at `099e4360c` (source:
`git merge-base --is-ancestor` and `git rev-list` on 2026-11-19). Ranges are branch spans and
include base commits; they are not counts of commits added by the PR.

| Layer | Verified reachable range / commit | Notes |
| --- | --- | --- |
| Kernel (K0–K15) | `06749ac4..d12c2fa5f` on `feat/int1454-kernel-snapshot` | vendored solidcore, kernel/smlib/geom/dynamics, STEP tree, rig and action-matrix re-records |
| Doc/format/IO | `ad0262a49..06e7aa667` | format/colour/display-units/viewport/sketch/render added, doc+io ported, attribution, format-58 note. The earlier `30fe35c92` endpoint is not reachable from the head and is not used. |
| Engine | `2ca1b7fd8..3e7a7aa46` | engine port, DFM/project/validation re-applied, plugin surface removed. The engine branch tip `7ceea11d5` (seven known-failing removals) is not in the remote head; the local integration head `61835bfaf` applies exactly that seven-line deletion; see §1, §4 and §7. |
| MCP | `6e1bcf53a..701d274ca` | server split, guarded envelopes, `version`, `--compact`, CLI stdio fix |
| GPU/viewport | `9bc73566d` | viewport-gpu/egui added, `crates/gpu` compute-only |
| UI | `8c1fc4f6c..f82b56372` | egui adaptation, labels/help/dialogs, viewport API, new-file preference, wasm fix. Later review fixes `19396bb4e`, `7aa8e008e`, the develop merge `5b02edb95` and `099e4360c` are outside this row. |
| Integration | `1a18423ed`, `b06d531e5` | develop merge and the two rule fixes (defaults line, advisories) |
| Local follow-up | `61835bfaf` (parent `099e4360c`, not pushed) | seven-line `tests/KNOWN_FAILING_TESTS` deletion from `7ceea11d5`; 22 -> 15 |

No exact head count is claimed until the landing commit is fixed and the index is regenerated.

## 2b. Licence audit (Cargo inventory and rights provenance are separate)

The ledger's licence audit records the vendored kernel as Apache-2.0 (not dual MIT/Apache), and the
SolidCraft-derived crates as MIT OR Apache-2.0; Cargo metadata is the inventory source, not a rights
conclusion. The kernel's own `LICENSE`,
`NOTICE` and `ATTRIBUTION.md` are carried, with the USD BRep/SMLib credits intact (verified:
`git rev-parse HEAD:solidcore` = `8b5c0144…`; `git diff v0.44.1 -- solidcore` empty). The moved
Liberation Sans font keeps its OFL row, the seven STEP regression fixtures have rows in
`attribution/pending/step-regressions-v0441.md`, and the FreeCAD fixtures/font moved with the doc
port have rows in `attribution/pending/doc-assets-v0441.md` (rows inspected at `099e4360c`).

`cargo xtask licences --check` is a Cargo-inventory check only: it reports that the generated
`THIRD-PARTY-LICENSES.txt` will be rewritten on `develop` by the Generated files workflow. It is
not npm/Node coverage and not a source-rights or provenance clearance. The absence of
`crates/ui-kit`, `crates/look`, `crates/theme` and brand assets is consistent with CD-6, but it
does not by itself prove that no post-brand code was adapted into the existing `ui-egui` chrome
files. The ledger flags those chrome files as licence-uncertain; owner/legal provenance review
remains open (§7). No blanket "all code is compliant" claim is made here.

## 3. Ledger coverage (mapping, not adoption)

`tools/coverage.py` assigns every non-merge commit in `06749ac4..v0.44.1` and every distinct path
to a path area, and exits non-zero on an orphan. Reproduced on 2026-11-19 from this worktree
(`python3 docs/upstream-sync/int1454/tools/coverage.py`):

```
range 06749ac4..v0.44.1: 4939 non-merge commits
  kernel         884
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

The range is 4,994 commits in total (4,939 non-merge), 15,357 distinct paths, 0 unmatched. This
proves path-area mapping only. `retired 2700` and `(empty) 29` are area labels; coverage does not
review, adopt, implement or license any row. The disposition table is `ledger.md`, and the ledger
states that classification is a planned disposition, not completed work (`ledger.md:12–13`).

The owner approved CD-1…CD-7 in `ledger.md` §7. The Phase 0 review cited at `ledger.md` §9 item 4
(there is no "Method" section) records two rounds and a passed Phase 0 gate; its brief scopes it to
the ledger, not the implementation, and it predates the current Already Covered rows. It is not
evidence of implementation acceptance. Blocked rows are multiple and each needs owner disposition:
A3 (CD-5), D5 (STEP move; the ledger row says blocked until K1 — recheck against the landed K1),
U9/U10/U11 (CD-6), U14 airfoil, U17 tickets/Windows policy, and U18. Only CD-5 has an explicit
approved deferral; CD-6 is a separate owner decision.


## 3a. Compatibility Decisions and how each is implemented

| Decision | Implementation | Evidence |
| --- | --- | --- |
| CD-1: one persistence implementation, format 58 | `solidcraft_format`'s registry/container is the only writer; `doc`/`io` use it; the five Intcad `format_pending` names are the same bytes as v0.44.1's `v51` files | `design::tests::a_design_saved_at_format_51_with_its_pending_changes_saves_and_reopens` and the snapshot-budget test exist in source. The GUI round trip is transcript-only (§5); it has no durable report and must be recaptured at the final SHA. |
| CD-2: MCP map, no legacy alias layer | 21 → 22 tools; `version` adopted; `cancel` and batch's `path` script input **not** adopted (no app `control.cancel`; a file-reading input is a behaviour change); schema deltas are claimed as classified, not re-audited here | `docs/mcp.md`, `crates/mcp/src/tests.rs`; the §5a "MCP slice" is a claimed review, not an artifact in the repo |
| CD-3: control channel stays on by default | no default change adopted; `crates/control` ported with its auth/loopback rules | `crates/control/src/server.rs` (byte-identical to v0.44.1); 33 control tests were reported passed in an earlier run, not re-run at the audited head |
| CD-4: no opt-in plaintext HTTP | no plaintext transport was added; the loopback token rules are unchanged | `crates/control/src/auth.rs`, `crates/engine/src/control_auth.rs` shim |
| CD-5: plugin runtime Blocked, not "Not Applicable" | runtime and plugin **command** surface removed: no `plugin.*`, `file.import`, `model.mesh`; no `craft-plugin`/`wasmtime`/`cranelift` in manifests or lock. A metadata-only `crates/ui-egui/src/plugins.rs` page remains. | `Cargo.lock`, engine command registry, §7 |
| CD-6: no brand-era UI kit/look/theme/assets | No brand-era `ui-kit`/`look`/`theme` crates or brand assets were found. The `ui-egui` diff is not a provenance proof; the ledger's licence-uncertain chrome files remain for owner/legal review. The only moved assets claimed are the Liberation Sans font (OFL) and the STEP/FreeCAD fixtures (attribution rows). | `git diff v0.44.1 -- crates/ui-egui` (scope only), ledger §4 and §7, §2b |
| CD-7: vendored solidcore closure, no rev pins | `solidcore/` byte-identical to `v0.44.1:solidcore`; `crates/step` deleted; `deny.toml` `[sources]` has no git sources | `git diff v0.44.1 -- solidcore` is empty and both trees hash to `8b5c01445619305dcfc5072db3fe76b8cfea67be`; `deny.toml` denies unknown git sources |

## 4. Gate evidence: local fixed checks, historic logs, commands (not final-SHA evidence)

Rows marked `local 61835bfaf` are new deterministic checks at the unpushed local head; they are not
pinned-CI evidence. The other counts are historic local logs recorded on 2026-11-18, before the
layer tips and the reviewed head, and are **historic and unverified for `099e4360c`**. Rows citing
`gh` or the CI snapshot are CI evidence at the old head only. This table is not a final-SHA gate
record; each row gives the command to rerun.

| Gate | Evidence (source and revision) |
| --- | --- |
| Build | No local head-bound `cargo check` log recorded in this package. At the 08:49Z CI snapshot, `Build full-baseline archive` at `099e4360c` was success in 48m22. Rerun: `cargo check --workspace --all-targets --profile ci`. |
| Lints | `Native Linux Clippy` check passed at `099e4360c` (gh, 2026-11-19); local `clippy` logs are older. Rerun: `cargo clippy --workspace --all-targets --profile ci -- -D warnings`. |
| Kernel | 932 passed / 6 failed, log 2026-11-18 02:22 (`/tmp/opencode/final-kernel-tests.log`), before kernel tip `d12c2fa5f` (11:58). Head list has 6 kernel entries. Historic/unverified at head. Rerun: `cargo test --profile ci -p solidcraft-kernel`. |
| Doc/format/IO | Historic logs are not a `--profile ci` triple: a debug run at 2026-11-18 01:59 has 873 passed (doc), 10 passed (format), and 722 passed + 1 failed (io, `gerber::tests::hostile_input_never_panics`); a separate `--profile ci` io run has 213 passed + 2 failed (`design::tests::the_cache_key_covers_every_crate_the_model_is_computed_from`, `tests::obj_export_is_welded_and_keeps_names`). None of these failures is on `KNOWN_FAILING_TESTS`. Unverified at head. Rerun: `cargo test --profile ci -p solidcraft-doc -p solidcraft-format -p solidcraft-io`. |
| Engine | Historic: 5309 passed / 4 failed, log 2026-11-18 05:18 (`/tmp/opencode/engine-test6.log`), before engine tip `7ceea11d5` (05:51); the remaining four engine allowances are still historic pending the current gate, not an automatic full pass. New `local 61835bfaf` (parent `099e4360c`, not pushed): the seven-line deletion from `7ceea11d5` gives 22 -> 15 entries. Each of the seven tests ran separately under `cargo test --profile ci` and `cargo-nextest 0.9.146`: 7/7 PASS (`/tmp/opencode/int1454-engine-allowances/results-all.tsv`). The known-failing grader exits 0 after deletion: `7 ran, 7 pass, 0 known failing, 15 listed did not run here; no regression` (`grader-after-commit.log`). Local compiler 1.98.1 (repo pin/CI 1.99.1); no pinned CI pass yet. Rerun: `cargo test --profile ci -p solidcraft-engine`; `cargo xtask known-failing --junit ...`. |
| MCP | No local log/report found for `163 / 14 / 33`; the three-number form is ambiguous, so no pass/fail inference is made. Rerun: `cargo test --profile ci -p solidcraft-mcp`. |
| UI | No local log/report found for `4997 passed / 2 environment-only failures`; the failures are unnamed and unlisted. Unverified; do not treat as environment-only without names and evidence. Rerun: `cargo test --profile ci -p solidcraft-ui-egui`. |
| Kernel rig | Historic local rig record (not re-run in this audit): 17 of 19 areas green at `136223609` (import, inspect, history, frames, booleans, crashes, plastic, construction, continuity, fillets, sheet-metal, solids, surfaces, shell, tessellation, sweeps, threads). The two that differed were re-recorded at `e95afde4d` with the branch's own release CLI (`target/release/intcad-cli`, 864,557,376 bytes, software/vulkan): `--area assemblies` -> `96 pass, 0 known failing, no regression`; `--ladder --record` then `--ladder --gate` -> `170/170 pass, every case as the golden records it`; `--coverage` -> `229 commands have a case`. The ladder change is count-only for threads (v0.44.1 keeps one exact helical face: `06-threads/002-external-m10x1-fine` 6/12/8 where the golden had 126/372/248, volume unchanged at 2081.901104145 mm3 against the analytic 2081.901104148) plus two corner-fillet refusals whose reason moved to a tangent seam; one assembly case now succeeds because v0.44.1's engine carries a moved-in body's history (#12142). CI `Kernel rig` run `44454875225` at `e95afde4d` and current run `44472128721` at `099e4360c` completed **success** (gh, 2026-11-19). |
| Rules | CI `Format and repository checks` passed at `099e4360c` (gh, 2026-11-19); local `xtask` rule logs are older. Rerun: `cargo xtask layers`, `mesh-decisions`, `lists`, `assets`, `ui-containers`, `fmt`. |
| CI | Snapshot 2026-11-19T08:49Z for run `44472128741` attempt 2 at old head `099e4360c` (test-merge `ceeb57bd…`): `Build full-baseline archive` success 48m22; `Native Linux selected regression gate` failure with exit 143 (runner shutdown during `cargo nextest list`, no test receipt); Action matrix 1/16 and 15/16 success; other shards, properties and determinism in flight; aggregate not created. This is a snapshot, not a current-status or pass claim; no test result was produced by the failing gate. Source: `/tmp/opencode/int1454-ci-current/SNAPSHOT-2026-11-19T0849Z.md`. |

## 5. Persistence (CD-1)

The single implementation is `solidcraft_format`'s registry/container used by `doc`/`io`. Test
`design::tests::a_design_saved_at_format_51_with_its_pending_changes_saves_and_reopens` builds a
file exactly as Intcad 0.1.1 wrote it, saves it as `solidcraft/58` and reopens it with identical
data. It and `a_snapshot_budget_bounds_the_design_container_it_writes` were reported passing on the
branch, but this audit did not rerun them; treat both as historic until the final-SHA run. The
`open_newer_files` preference is implemented, and refusal tests exist at
`crates/engine/src/cmd/file.rs:3417` and `:3459`.

GUI-level round trip: the controller's session transcript recorded Linux opening
`modern-sheet-metal.solidcraft` and saving a 16,508-byte file, and Windows saving
`C:/Users/Public/cd1-win.sc` (44,940 bytes) and reopening it. **No durable local `REPORT.md` /
`report.json`, log, saved file, or hash for those round trips was found** in the E2E state. Treat
them as transcript-only, not acceptance evidence; recapture raw evidence at the final SHA.

## 5a. Independent review record (claims; no review artifact in the repo)

These verdicts are recorded from the controller's session claims for source slices. No reviewer
identity, date, reviewed revision or report file is in the repo. They are not an audit of all 4,994
upstream commits and do not cover the implementation as a whole.

| Review | Scope | Verdict | Fixes it forced |
| --- | --- | --- | --- |
| Kernel slice | vendored kernel, rig, ladder, action matrix | approved after 2 rounds | rig path names, empty renamed recipe, action-matrix baseline recorded on this stack, the exact-blend `splines` check reading a `Surface::Blend`'s net |
| Doc/format/IO slice | format 58, one persistence implementation | approved | attribution rows for the moved font and FreeCAD fixtures |
| Engine slice | registry, DFM/project/validation preservation, CD-5 surface | approved after 2 rounds | `control_auth` compatibility shim, plugin surface removed wholesale (not just the runtime), duplicate mesh-decision rows, `.solvecraft` doc comment |
| MCP slice | guarded envelopes, CD-2 map | approved after 2 rounds | CLI `!Send` stdio, `--compact` wired, CD-2 table corrections, ledger counts |
| Final branch review | post-approval deltas only | **changes requested** at `e95afde4d`, then re-reviewed twice | (1) staged/wasm opens ignored `open_newer_files`; (2) the preference applied only on the next frame, and Cancel/Defaults restored it without syncing the session; (3) runtime GPU render requests dropped directional/punctual lights; (4) `EXEMPT` put non-geometry commands in the geometry backlog. Fixed in `19396bb4e` and `7aa8e008e`; F1/F3/F4 and F2 were then confirmed pass by the same reviewer. The worktree then advanced through `5b02edb95` (develop merge) to `099e4360c`; the F2 source fix was approved, but there is no whole-head final approval. The develop merge and lockfile reconciliation were not covered by the final review. |

The reviewer also corrected an overstated claim in an earlier draft of the pull request: `KNOWN_FAILING_TESTS`
has **33 entries on `develop` and 22 at remote head `099e4360c`** (11 removed because their test now passes,
2 reworded to name the vendored kernel instead of the old pin, 0 added), not the "42 to 15" an earlier draft
said. The 33/22 counts are verified. At local head `61835bfaf` (parent `099e4360c`, not pushed), the engine
branch's `7ceea11d5` deletion is applied exactly: 22 -> 15. The seven tests pass 7/7 under local
`cargo test --profile ci` and `cargo-nextest 0.9.146`; the grader exits 0 with
`7 ran, 7 pass, 0 known failing, 15 listed did not run here; no regression`. Local compiler 1.98.1; no
pinned CI pass yet. Await independent task review, push and pinned CI. The remaining four engine allowances
stay historic pending the current gate; this is not a full engine pass.

## 6. Platform acceptance

| Platform | Status |
| --- | --- |
| Linux native CI | Snapshot 2026-11-19T08:49Z for run `44472128741` attempt 2 at old head `099e4360c`: `Build full-baseline archive` success 48m22; `Native Linux selected regression gate` failure with exit 143 (runner shutdown during `cargo nextest list`, no test receipt); Action matrix 1/16 and 15/16 success; other shards/properties/determinism in flight; aggregate not created. `Native Linux Clippy` passed. This is not a current-status or final-green claim. Source: `SNAPSHOT-2026-11-19T0849Z.md`. |
| Linux GUI E2E | **PASS 8/8 at old SHA `42f4d670a`** (task `int1454`): sample open, orbit drag, face select, Assistant open/resize/control. Report `~/.local/state/intcad-e2e/runs/intcad-e2e-int1454-ui-c28767d6c085-int1454/20261118T212022Z-681395/REPORT.md` and artifact sha256 `e6da03a57c438530d1640a0a171b0f1f220768aa95e3a1d5a54cef246b5a83cc` verified. The SHA predates `19396bb4e`/`7aa8e008e` and is not `099e4360c` or a final SHA; repeat at the final SHA. |
| Windows smoke | **Smoke only at `f82b56372`** (task `int1454w`): MSI `b1d55038d01c9452f4b534bfe93e8d365c0f08c2875518395aaccaf84890be4a`, app sha256 `ca1c34de2167f4b2ec028afe6403430d43a7a83a09ae4f7c6992dbcf036ba6c1`; `about.json`/`first-inspect.json` show installed, Sample Plate loaded, control channel listening. No `REPORT.md`/`report.json`; this is not a completed VM GUI E2E. CI run `44402787343` at that SHA failed overall (Windows startup/FXC jobs passed), so it is not all-native-CI PASS. Recapture a full `accept` at the final SHA. |
| macOS native automation | not run |
| macOS/desktop UI (PP) | not run |

## 6a. Security findings

Source: source inspection at `099e4360c`; PR #179 merge `96a247ccb`.

| Finding | Status | Evidence |
| --- | --- | --- |
| SEC-1: the agent gate could be bypassed by command-id case or an id/command alias | **Fixed and merged** | PR #179 = `96a247ccb`; one `resolve_command_id` decides for the gate, the control channel and execution, with allow/ask/refuse and hostile-input regressions |
| SEC-2: STEP import decoded `\\X2\\`/`\\X4\\` strings in quadratic time (`54b4bb3f4`) | **Adopted with the kernel** | `crates/step` (the old per-run tail search) is deleted; the fixed decoder is `solidcore/crates/solidcore-record/src/step/p21.rs` (`decode_string_with`, disjoint stretches, linear however many unterminated runs). Its hostile-input test `run_terminators_are_searched_once` decodes 100,000 unterminated tags; no other Part 21 decoder remains in `crates/` |
| SEC-3: control channel refused a limited connection without draining (`18d82f584`) | **Landed, not yet wired** | `crates/control/src/server.rs` is byte-identical to v0.44.1 (`git diff v0.44.1 -- crates/control/src/server.rs` is empty) but the app still serves through `apps/solidcraft/src/control_server.rs`; moving the app onto the ported server is UI/platform-slice work and is listed as an open item |
| Plugins (Wasmtime) | **Blocked per CD-5** | Runtime and plugin command surface removed; a metadata-only `crates/ui-egui/src/plugins.rs` page remains. See §7. |

## 7. Open items and exceptions

- CD-5 plugin runtime: Blocked/deferred with unblocking conditions (ledger A3).
- CD-6: no brand-era `ui-kit`/`look`/`theme` crates or brand assets were found, but provenance
  clearance is not complete. The licence-uncertain chrome files identified in ledger §4 remain for
  owner/legal review.
- Engine allowance deletion: local at `61835bfaf` (parent `099e4360c`, ahead 1, not pushed),
  exactly the seven-line deletion from `7ceea11d5`, 22 -> 15. Seven/7 local `cargo test --profile ci`
  and `cargo-nextest 0.9.146` PASS; grader exit 0 (`7 ran, 7 pass, 0 known failing, 15 listed did
  not run here`); local compiler 1.98.1 (repo pin/CI 1.99.1). Await independent task review, push
  and pinned CI. The remaining four engine allowances stay historic pending the current gate, not
  an automatic full pass.
- Ledger §4's recorded "UI not exposed" list: snap points, silhouettes, timeline groups/folders,
  assembly-constraint dialogs, frame end caps, render-job panes, import report. Statuses:
  U12/U13/U15 adopted-but-not-exposed; U14 airfoil Blocked; U9/U10/U11 CD-6 Blocked; U17
  tickets/Windows policy Blocked; U18 plugin UI Blocked.
- A separate worker-reported list (multi-plane sections, point-cloud segments, spline handles,
  joint plane/axis origins, design view tabs, airfoil import, several feature options, CPU-preview
  options, sparse panel bodies) is **not in ledger §4** and has no source in this package. Owner
  review pending; do not treat it as ledger status.
- SEC-3 drain: present in `solidcraft_control` but the app still runs its own control server.
- `--compact` wired for the CLI; the app does not answer `mcp.options` yet.
- `THIRD-PARTY-LICENSES.txt` is regenerated on `develop` by the Generated files workflow.
- Landing method, landing commit and final develop SHA: TBD.

## 8. Rollback (landing method and target TBD)

The landing method and commit hash are not fixed. If #185 lands as a merge commit, rollback is a
revert PR against that merge commit (or against the squash commit if it lands squashed). The revert
must preserve unrelated develop changes merged after the pre-sync baseline, including #167, #176,
#181, #186/main reconciliation and #188, plus any later work; do not promise a return to
`0fd92bea2`. That SHA is a historical baseline, not the current develop parent.

Per-layer commits remain on their branches for partial re-application. The kernel snapshot has no
second kernel; partial re-pinning without reverting the whole sync is not supported without a new
compatibility decision.

## 9. Sign-off (outstanding gates)

No sign-off yet. Outstanding:

1. Land #185 by the chosen method; record the landing commit and final develop SHA.
2. Green CI at that SHA. At audit, CI `44472128741` was still in progress with two checks pending.
3. Complete independent task review and push of local `61835bfaf` (parent `099e4360c`), then obtain
   a pinned-CI result for the seven-line deletion. The remaining four engine allowances still need
   the current gate; do not treat the local 7/7 as a full engine pass.
4. Re-run the per-crate gates at the final SHA; replace historic counts and resolve the unlisted
   IO/UI/MCP failures.
5. Run Linux GUI E2E and a full Windows `accept` at the final SHA; recapture CD-1 raw save/reopen
   evidence (currently transcript-only).
6. Record macOS automation and PP UI acceptance.
7. Obtain a scoped whole-head final review; the F2 source fix was approved but the worktree advanced
   to `099e4360c`, and no whole-head approval exists.
8. Obtain owner disposition for every Blocked row and review the UI lists in §7.
9. Refresh this package with the final SHA, commands, reports and hashes.

This remains a draft; none of the above is a completion claim.

## 10. Verification sources used here

- `git rev-parse`, `git merge-base --is-ancestor`, `git log --first-parent` and `git diff` in the
  acceptance and integration worktrees, 2026-11-19. Local integration head:
  `61835bfaf` (parent `099e4360c`), `git -C worktrees/int1454-ui rev-parse HEAD HEAD^`.
- `gh pr view 185`, `gh pr checks 185`, `gh run view 44472128741`, `gh run view 44472128721`,
  `gh run view 44454875225` and `gh run list --commit f82b56372` (2026-11-19).
- CI snapshot: `/tmp/opencode/int1454-ci-current/SNAPSHOT-2026-11-19T0849Z.md` (attempt 2 at
  `099e4360c`; snapshot only, not current status).
- Engine allowances: `/tmp/opencode/int1454-engine-allowances/FIX-REPORT.md`, `SUMMARY.md`,
  `results-all.tsv`, `grader-after-commit.log` and `post-commit.txt` (local `61835bfaf`; local
  compiler 1.98.1 from `target/ci/.rustc_info.json`; no pinned-CI pass).
- `python3 docs/upstream-sync/int1454/tools/coverage.py` from the acceptance worktree.
- Historic local logs: `/tmp/opencode/final-kernel-tests.log`, `/tmp/opencode/final4.log`,
  `/tmp/opencode/final-io-tests.log` and `/tmp/opencode/engine-test6.log` (2026-11-18; not
  head-bound).
- E2E: the Linux report path in §6 (8/8, verified hash); the Windows run directory in §6 (smoke
  files only).
