# INT-1454 acceptance package (draft — incomplete until sign-off)

Status: PR #185 is **open** at remote head `099e4360c3c1a9e70582731801289d4b4a5a4570`
(`feat/int1454-ui`); GitHub reports `mergeStateStatus: BLOCKED`. At the time of the engine-allowance
evidence, the local integration worktree had one commit on top of that head:
`61835bfafb151c7d6803aeab195e824b6d8458d3` (parent `099e4360c`), the seven-line engine
known-failing deletion from `7ceea11d5` (22 -> 15). It had not been reviewed or run through pinned
CI then. A later read-only observation on 2026-11-19 found the worktree at
`c5f2f42fe6f06ed5c2c4ec1df04f7e9c6aae50ad` (pushed; unrelated sheet-hole work), so `61835bfaf` is
no longer the local head; the engine evidence remains at `61835bfaf`. At the 2026-11-19T08:49Z CI
snapshot, run 44472128741 attempt 2 at the old head had `Build full-baseline archive` success
(48m22), `Native Linux selected regression gate` failure with exit 143 during `cargo nextest list`
(no test receipt), Action matrix shards 1 and 15 success, other shards/properties/determinism in
flight, and no aggregate. This package is a draft, not a completion claim: it records what is
verified, what is local, what is historic, and what remains open. Owner authorization for the
follow-on validation-lock/Union design is recorded in §3b; it is design approval only. Sources:
`gh pr view 185`, `/tmp/opencode/int1454-ci-current/SNAPSHOT-2026-11-19T0849Z.md`,
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
| Engine-evidence local head | `61835bfafb151c7d6803aeab195e824b6d8458d3` (parent `099e4360c`; fixed for the seven-line evidence). Later observed head: `c5f2f42fe` (pushed; not used here). | `git -C worktrees/int1454-ui rev-parse HEAD HEAD^`; `/tmp/opencode/int1454-engine-allowances/FIX-REPORT.md` |
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
| Engine | `2ca1b7fd8..3e7a7aa46` | engine port, DFM/project/validation re-applied, plugin surface removed. The engine branch tip `7ceea11d5` (seven known-failing removals) is not in the remote head; the local engine-evidence head `61835bfaf` applies exactly that seven-line deletion; see §1, §4 and §7. |
| MCP | `6e1bcf53a..701d274ca` | server split, guarded envelopes, `version`, `--compact`, CLI stdio fix |
| GPU/viewport | `9bc73566d` | viewport-gpu/egui added, `crates/gpu` compute-only |
| UI | `8c1fc4f6c..f82b56372` | egui adaptation, labels/help/dialogs, viewport API, new-file preference, wasm fix. Later review fixes `19396bb4e`, `7aa8e008e`, the develop merge `5b02edb95` and `099e4360c` are outside this row. |
| Integration | `1a18423ed`, `b06d531e5` | develop merge and the two rule fixes (defaults line, advisories) |
| Local follow-up | `61835bfaf` (parent `099e4360c`; engine-evidence head, later observed `c5f2f42fe`) | seven-line `tests/KNOWN_FAILING_TESTS` deletion from `7ceea11d5`; 22 -> 15 |

No exact head count is claimed until the landing commit is fixed and the index is regenerated.

## 2b. Licence audit (Cargo inventory and rights provenance are separate)

The [historical package notice inventory](package-notice-inventory.md)
separates examined payloads from build receipts. It is not the final
landed-SHA package or licence gate.

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

## 3b. Owner authorization (2026-11-19) — design only

**Scope.** The owner approved the two designs below on 2026-11-19. This is **design approval
only**: it is **not** code complete, not reviewed, not CI green and not merge approval.

### (a) Validation lock and bounded UI cancellation

Approved: a stable cross-process lock for validating terminal transactions, plus bounded
background UI cancellation. The design must preserve command schemas, permission checks and
terminal precedence. Tests must cover concurrent processes, bounded cancellation, and unchanged
schema, permission and precedence behaviour.

### (b) Temporary isolated Union swapped-operand retry

Approved: a temporary, isolated Union adapter that retries the original path first and falls back
only for a specific confirmed internal error. The vendored solidcore tree is **not** changed.
Strict geometry, naming and history behaviour must be proven, or the work stops.

### Removal lifecycle

On the next upstream upgrade, test the upstream original path with the fallback disabled:

- fully fixed: delete the adaptation and keep the product regression tests;
- not fully fixed: revalidate it; do not blindly carry it forward;
- changed error, algorithm or naming: suspend the adapter.

### Provenance and task ownership

- **Provenance:** owner conversation, 2026-11-19. Existing design reports/controller transcript
  have uncertain provenance and are not treated as reviewed evidence; record them under this
  owner-conversation provenance only.
- **Sol:** implementation in an independent worktree, plus validation.
- **GOAT:** implementation of the Union fallback.
- **Heavy local builds:** paused under the disk budget; this authorization does not start one.
- **Kernel source:** remains CD-7 frozen at vendored solidcore tree
  `8b5c01445619305dcfc5072db3fe76b8cfea67be`; this authorization does not change it.

### Actionable gates (same head, final tests)

1. Name one head SHA. Run every gate and final test at that same SHA; do not combine results from
   different heads.
2. Lock gate: concurrent-process test, stable acquisition, bounded cancellation, and unchanged
   schemas, permissions and terminal precedence.
3. Union gate: original path first; fallback only on the specific confirmed internal error;
   geometry, naming and history proofs; fallback-disabled run; no solidcore change. Stop if the
   strict proofs fail.
4. Lifecycle gate: simulate the next upgrade with the fallback disabled and record the delete,
   revalidate or suspend disposition.
5. Record command, head, result and evidence for each gate. No CI-green or merge claim until a
   pinned CI run and review exist.

## 4. Gate evidence: local fixed checks, historic logs, commands (not final-SHA evidence)

Rows marked `local 61835bfaf` are new deterministic checks at the engine-evidence local head; they
are not pinned-CI evidence. The other counts are historic local logs recorded on 2026-11-18, before the
layer tips and the reviewed head, and are **historic and unverified for `099e4360c`**. Rows citing
`gh` or the CI snapshot are CI evidence at the old head only. This table is not a final-SHA gate
record; each row gives the command to rerun.

| Gate | Evidence (source and revision) |
| --- | --- |
| Build | No local head-bound `cargo check` log recorded in this package. At the 08:49Z CI snapshot, `Build full-baseline archive` at `099e4360c` was success in 48m22. Rerun: `cargo check --workspace --all-targets --profile ci`. |
| Lints | `Native Linux Clippy` check passed at `099e4360c` (gh, 2026-11-19); local `clippy` logs are older. Rerun: `cargo clippy --workspace --all-targets --profile ci -- -D warnings`. |
| Kernel | **Verified at `9c24061c` on 2026-11-23**: `cargo test --locked --profile ci -p solidcraft-kernel --lib` → exit `101`, 808 passed, 13 failed, 4 ignored. All 13 raw failures match entries in that revision's `tests/KNOWN_FAILING_TESTS` and no listed test passed: `check-known-failing.py` exits `0` (`gates-9c24061c/kernel-known-failing.txt`). The raw suite is red only through its recorded allowances; this is not a green run and it clears none of them. The historic `932 / 6` figures, recorded before kernel tip `d12c2fa5f`, are superseded. |
| Doc/format/IO | Historic logs are not a `--profile ci` triple: a debug run at 2026-11-18 01:59 has 873 passed (doc), 10 passed (format), and 722 passed + 1 failed (io, `gerber::tests::hostile_input_never_panics`); a separate `--profile ci` io run has 213 passed + 2 failed (`design::tests::the_cache_key_covers_every_crate_the_model_is_computed_from`, `tests::obj_export_is_welded_and_keeps_names`). None of these failures is on `KNOWN_FAILING_TESTS`. Unverified at head. Rerun: `cargo test --profile ci -p solidcraft-doc -p solidcraft-format -p solidcraft-io`. |
| Engine | **Verified at `9c24061c` on 2026-11-23**: `cargo test --locked --profile ci -p solidcraft-engine --lib` → exit `101`, 3627 passed, 11 failed, 10 ignored. All 11 raw failures match entries in that revision's `tests/KNOWN_FAILING_TESTS` (33 entries) and no listed test passed: `check-known-failing.py` exits `0`. The raw suite is red only through its recorded allowances; this is not a green run and it does not clear them. The historic `5309 / 4` figures are superseded. |
| MCP | **Verified at `9c24061c` on 2026-11-23**: `cargo test --locked --profile ci -p solidcraft-mcp --lib` → exit `0`, 140 passed, 0 failed, 0 ignored. The earlier ambiguous `163 / 14 / 33` numbers are superseded; nothing was inferred from them. |
| UI | No local log/report found for `4997 passed / 2 environment-only failures`; the failures are unnamed and unlisted. Unverified; do not treat as environment-only without names and evidence. Rerun: `cargo test --profile ci -p solidcraft-ui-egui`. |
| Kernel rig | **Measured case by case at `9c24061c` on 2026-11-23** with the built `intcad-cli` `kernel-cases <case> --json` (one process a case, the rig's own invocation) over all 1044 cases of `tests/kernel-cases`. Excluding the 12 `ladder` cases, which are graded against the ladder golden and fail on standalone replay on both builds: **1010 pass, 17 fail, 5 unsupported of 1032**. The 22 non-pass cases all appear in `tests/kernel-cases/*/KNOWN_FAILING` at this revision, with no unlisted failure and no listed case passing. This is not the CI `Kernel rig` gate (`cargo xtask kernel-cases --gate` on the selected areas); it is an equivalent per-case measurement and it does not run the ladder gate. The same inventory on `db0b8dd35` gives 1015/8/9, and the 12 case differences between the builds trace to their different kernel sources. Receipts: `cd1/kernel/*-9c24061c.json`. |
| Rules | **Verified at `9c24061c` on 2026-11-23** with Rust 1.99.1 (`cargo 1.99.1 (5f94df478 2026-08-27)`, `rustc 1.99.1 (58cf8f9c3 2026-10-13)`) in a clean worktree at that exact SHA. All six checks exited `0`: `cargo xtask layers` (25 crates, no layering violations), `cargo xtask mesh-decisions` (181 known decision sites use a mesh, no new ones), `cargo xtask lists` (growing arrays checked; catalogs, known-failing list and rig cases sorted; pending locale and attribution files fold cleanly), `cargo xtask assets` (1049 files, all attributed; 264 encoded render resources match their source receipts), `cargo xtask ui-containers` (all outer hosts use the shared API), `cargo xtask fmt --check` (rustfmt on 119 files in 8 runs). The worktree was clean before and after. Receipts: `rules-9c24061c/results.tsv` SHA-256 `99c0fb12…`, with per-check logs. This is rule-check evidence at `9c24061c`, not at a landed integration SHA. |
| CI | Snapshot 2026-11-19T08:49Z for run `44472128741` attempt 2 at old head `099e4360c` (test-merge `ceeb57bd…`): `Build full-baseline archive` success 48m22; `Native Linux selected regression gate` failure with exit 143 (runner shutdown during `cargo nextest list`, no test receipt); Action matrix 1/16 and 15/16 success; other shards, properties and determinism in flight; aggregate not created. This is a snapshot, not a current-status or pass claim; no test result was produced by the failing gate. Source: `/tmp/opencode/int1454-ci-current/SNAPSHOT-2026-11-19T0849Z.md`. |

### 4a. Two-build kernel rig baseline and the kernel-source divergence

A case-by-case rig baseline now exists for the whole `tests/kernel-cases` tree on two builds:
post-M16 `develop` `9c24061c` and historical candidate `db0b8dd35`. Each case ran in its own process
through the built CLI, the same invocation the rig uses.

| Areas | Cases | `9c24061c` | `db0b8dd35` |
| --- | --- | --- | --- |
| frames | 6 | 6 pass | 6 pass |
| solids, continuity, crashes, tessellation, inspect, plastic, history, threads | 170 | 170 pass | 170 pass |
| booleans, sweeps, surfaces | 287 | 287 pass | 287 pass |
| assemblies, sheet-metal, construction | 227 | 227 pass | 227 pass |
| import | 35 | 35 pass | 35 pass |
| shell | 37 | 36 pass, 1 fail | 37 pass |
| fillets | 271 | 250 pass, 16 fail, 5 unsupported | 254 pass, 8 fail, 9 unsupported |
| **Total (excluding ladder)** | **1032** | **1010 pass, 17 fail, 5 unsupported** | **1015 pass, 8 fail, 9 unsupported** |

**Coverage is complete:** all 1044 cases of the tree ran on both builds, with identical case sets;
`ladder` (12) is excluded from the totals because those cases are graded against the ladder's golden
and report `fail` on standalone replay on both builds.

**Differences: 12 cases — 11 in `fillets` and 1 in `shell`. 1032 cases are identical in outcome.**
Every differing case directory is byte-identical at both SHAs.

**The two builds do not run the same kernel source.** `db0b8dd35` builds the vendored `solidcore/`
tree `8b5c0144…` (solidcore `524be9e1…`); `9c24061c` pins git revision `8ddce901…`. Comparing
`crates/solidcore/src`: **451 files differ, 153 exist only in the vendored copy, 4 only in the pinned
copy**. The vendored copy's newest `plan/findings` date is 2026-11-16 against 2026-11-02 for the
pinned one, so the vendored kernel is the later source. The two crates both declare version `1.0.1`,
so the version string does not distinguish them.

**What this blocks, precisely:**

- The rig differences above are **not** IntCAD before/after evidence. They compare two different
  kernels, so they cannot show a regression, a fix, or justify changing any `KNOWN_FAILING` entry.
  Under the same-kernel requirement, a fillets or shell difference may only be judged after the
  pinned and vendored kernels are the same revision.
- K12's before/after gate therefore has two separate prerequisites: the same kernel source on both
  sides, and one named head SHA carrying every result.
- The 17 mapped unit-test IDs and the three deferred UI IDs are unaffected by this, because they are
  `cargo test` cases rather than rig cases; they still need their own same-head run.

Each build's list is self-consistent: `9c24061c` has 21 non-pass fillets cases and lists 21;
`db0b8dd35` has 17 and lists 17; neither has an unlisted failure or a listed case that passes. So the
divergence is between kernels, not a broken allowance list on either side.

Receipts: `cd1/kernel/{areas,rest,heavy,last}-{9c24061c,db0}.json` and their `*-compare.json`, all
produced by `kernel-collect.py` and retained locally.

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

### 5.1 Headless cross-version check (candidate evidence, not final)

A reusable headless check now covers the CD-1 round trip without a window, driving `intcad-cli mcp`
over stdio. It opens a real `solidcraft/13` corpus design, saves it, reopens it, compares design
content excluding evaluation timings and the derived name, asserts the ordinary save reply is exactly
`path`, `bytes`, `name`, and confirms a truncated copy is refused without replacing the active design.

- **`9c24061c`** (current `develop`, CLI SHA-256 `d098a1547791d4285c19cf4f1d54a2785851d3c56ca7a8eb9f51ecc45934824c`): pass. It writes `solidcraft/51` plus five `format_pending` names and reads formats up to 51.
- **`db0b8dd35`** (historical candidate, CLI SHA-256 `b3a856e2e99886f2b1786d8561fea5df964060797bed86de1ae572c098d779f9`): pass. It writes `solidcraft/58`.
- **Cross-version:** `db0b8dd35` opens a `51` file, saves it as `58`, and reopens it with the same content digest. `9c24061c` refuses a `58` file with `this design was saved by a newer Intcad (file format 58; this version reads formats up to 51)` and leaves the active design unchanged.

So the format gap is one-way and explicit: the migration moves persistence from 51 to 58, and a
pre-migration build cannot read a post-migration file. That is the existing newer-file policy, not
silent coercion; it also means the migration cannot be rolled back by a build that only understands
51 without losing access to files written in between.

### 5.2 Real desktop round trip on `develop`

The same checks ran through the running desktop app, not a headless session, on `9c24061c`.

- Builds: `intcad` SHA-256 `d09dc11bfa756ca21d5896008fedc427a1ba28a3e9db01889a182502de024d11`, `intcad-cli` `d098a1547791d4285c19cf4f1d54a2785851d3c56ca7a8eb9f51ecc45934824c`, both Rust 1.99.1 `ci` profile from the pinned lockfile.
- Harness: `cd1-verify.py` SHA-256 `aa2f2c5193a979a0ae743e60ec41a3bf219e7910f82ec3c5c65400b08c701c4a`, fixture SHA-256 `1a3b36bb…`. Binary, CLI, harness and fixture hashes were identical before and after the run.
- Result: exit `0`; the server reported `backend: desktop app`. The `solidcraft/13` design opened, saved as `solidcraft/51`, and reopened with the same content digest `6922cc60…` as the headless run; the second save reported 7663 bytes with the same file digest `eb75cd2a…`; a truncated copy was refused (`HDR resource admission: EOF while parsing an object`) without replacing the active design. The record has no headless-session note. VM shut off afterwards.
- Log SHA-256 `f5cfdade3b0f0344d46187ed2c73f46d0fe95757364029dacddb40c7bd4512f7`.

The run was repeated with a newer-file step added (harness SHA-256 `d89a363aba08724a410e2a02d595fa2daf343c029c1943b6d1f09816cf1383f6`):
a copy of the saved file tagged `solidcraft/52` was refused with `this design was saved by a newer
Intcad (file format 52; this version reads formats up to 51)`, the active design stayed unchanged,
hashes again matched before and after, and the log SHA-256 is
`1897b77905245369df382f6c9d9c6e1ff5f2cb13199e11194c7c195aa4c4c0cb`.

`open_newer_files` does not exist on `develop`; the preference belongs to the sync candidate, so the
preference path cannot be exercised on this build. The unconditional newer-file refusal above is
the whole policy here. Preference on/off verification belongs to the landed SHA.

This closes the desktop CD-1 round trip for the pre-migration `develop` build only. The equivalent
run on the landed migration SHA, with the format and preference checks repeated there, remains open.

This is tool validation on existing revisions. Final-SHA persistence acceptance still needs the same
check run on the CLI built from the landed SHA, with the GUI round trip recaptured.

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
said. The 33/22 counts are verified. At local engine-evidence head `61835bfaf` (parent `099e4360c`),
the engine branch's `7ceea11d5` deletion is applied exactly: 22 -> 15. The seven tests pass 7/7 under local
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
- Engine allowance deletion: local at engine-evidence head `61835bfaf` (parent `099e4360c`; later
  observed `c5f2f42fe`), exactly the seven-line deletion from `7ceea11d5`, 22 -> 15. Seven/7 local
  `cargo test --profile ci` and `cargo-nextest 0.9.146` PASS; grader exit 0 (`7 ran, 7 pass, 0 known
  failing, 15 listed did not run here`); local compiler 1.98.1 (repo pin/CI 1.99.1). Await
  independent task review and pinned CI; the deletion is now in history behind `c5f2f42fe`, not at
  that head. The remaining four engine allowances stay historic pending the current gate, not an
  automatic full pass.
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
2. Green CI at that SHA. At the 08:49Z snapshot the native gate had no test receipt and the
   aggregate did not exist; no final green run.
3. Complete independent task review of the seven-line deletion (now in history behind `c5f2f42fe`)
   and obtain a pinned-CI result at the actual head. The remaining four engine allowances still need
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
10. Complete the owner-authorized validation-lock/Union work and run the §3b same-head gates;
    design approval is not code, review, CI or merge approval.

This remains a draft; none of the above is a completion claim.

## 10. Verification sources used here

- `git rev-parse`, `git merge-base --is-ancestor`, `git log --first-parent` and `git diff` in the
  acceptance and integration worktrees, 2026-11-19. Engine-evidence local head:
  `61835bfaf` (parent `099e4360c`), later observed integration head `c5f2f42fe`; source
  `git -C worktrees/int1454-ui rev-parse HEAD HEAD^`.
- `gh pr view 185`, `gh pr checks 185`, `gh run view 44472128741`, `gh run view 44472128721`,
  `gh run view 44454875225` and `gh run list --commit f82b56372` (2026-11-19).
- CI snapshot: `/tmp/opencode/int1454-ci-current/SNAPSHOT-2026-11-19T0849Z.md` (attempt 2 at
  `099e4360c`; snapshot only, not current status).
- Engine allowances: `/tmp/opencode/int1454-engine-allowances/FIX-REPORT.md`, `SUMMARY.md`,
  `results-all.tsv`, `grader-after-commit.log` and `post-commit.txt` (local `61835bfaf`; local
  compiler 1.98.1 from `target/ci/.rustc_info.json`; no pinned-CI pass).
- Owner authorization: owner conversation, 2026-11-19; design-only. Existing design
  reports/controller transcript have uncertain provenance and are recorded under that
  conversation, not as reviewed evidence.
- `python3 docs/upstream-sync/int1454/tools/coverage.py` from the acceptance worktree.
- Historic local logs: `/tmp/opencode/final-kernel-tests.log`, `/tmp/opencode/final4.log`,
  `/tmp/opencode/final-io-tests.log` and `/tmp/opencode/engine-test6.log` (2026-11-18; not
  head-bound).
- E2E: the Linux report path in §6 (8/8, verified hash); the Windows run directory in §6 (smoke
  files only).

## 11. Checkpoint revision 2026-11-19T23:30Z (documentation-only, append-only)

Scope: this section records later verified observations. It adds no source change, no build, no CI
dispatch, rerun or cancel, and no merge or release. It does not rewrite earlier sections; where §1
or §6 names an older head, this section records the newer observation and keeps the old text as
history. No sign-off, closure or completion claim is made. This checkpoint used the official
DeepSeek API. No prohibited external source was read for it; the rights incident in §11.7 is
reported from its existing record only.

### 11.1 Fixed references observed at this checkpoint

| Item | Value | Verification |
| --- | --- | --- |
| Base of this checkpoint | `docs/int1454-acceptance` = `182d485e2c6b31b324ff45794324f3454b63c67e`, worktree clean | `git status --porcelain` empty; `git log -1` |
| PR #185 source head, last verified | `c5f2f42fe6f06ed5c2c4ec1df04f7e9c6aae50ad` (`feat/int1454-ui`; PR OPEN, `mergeStateStatus: BLOCKED`) | local and `origin/feat/int1454-ui` both at this SHA; CI snapshot |
| PR #185 test-merge, last verified | `2a20b4b3974d11b68388db8f72facbd44aadedc6`; parents `9714e5b…` + `c5f2f42…` | `git rev-parse 2a20b4b^1 2a20b4b^2` |
| Develop observed at checkpoint | `9714e5b300030f23bc775fa180b1ba7c80047f80` = `origin/develop` then | `git rev-parse origin/develop`; not guaranteed current after this checkpoint |
| Landing commit / final develop SHA | **TBD** | no landing or release has happened |

The earlier engine-evidence head `61835bfaf` (parent `099e4360c`) remains a local evidence revision,
not the current PR head. The seven-line engine allowance deletion is included in `c5f2f42`’s
ancestry. At `c5f2f42`, `tests/KNOWN_FAILING_TESTS` has 15 entries: 13 allowed known failures and
the 2 `FIXED` entries noted in §11.2. That is a flat list count, not a per-case c5 PASS proof. The
exact seven-case PASS result remains the local `61835bfaf` evidence. The independent task review and
pinned-CI gate for the deletion remain open (§9); this checkpoint does not claim the seven tests
individually pass at `c5f2f42`.

### 11.2 Whole-CI state at `c5f2f42` (snapshot 2026-11-19T20:39–20:45Z)

Source: `/tmp/opencode/int1454-ci-current/SNAPSHOT-2026-11-19T2045Z.md`. The five runs listed there
all have `head_sha` `c5f2f42…`, attempt 1. This is a read-only snapshot, not a current-status claim.

| Run | Workflow | Result | Scope / note |
| --- | --- | --- | --- |
| 44505747549 | CI | **failure** | native gate + Action matrix 2/16 and 8/16 + aggregate |
| 44505747558 | Kernel rig | success | 21 rig jobs + `kernel-rig` (22 job entries) |
| 44505747529 | Fillet rig | success | 1 job |
| 44505747527 | PR checks | success | 4 jobs |
| 44505747682 | Packaging lint | success | lint + assistant page |

The actual matrix job labels are `Action matrix (2/16)` and `Action matrix (8/16)`; the shorthand
“matrix 2/8” is not used here. The `Action matrix (16/16)` **job** passed; shards `(2/16)` and
`(8/16)` failed, and the other shards passed. All other lanes in CI 44505747549 passed: clippy,
wasm, doc tests, reference,
both Mastra jobs, Windows startup/FXC, properties, determinism and matrix 1, 3–7, 9–16.

Native Linux gate: the uploaded `evidence-rust` shows nextest list exit 0, nextest run exit **100**,
and `xtask known-failing` exit **1**. JUnit reports 16,839 tests and **24 failures**; no compile
error. The known-failing grader reports 13 listed failures allowed, **11 off-list regressions**,
and **2 listed-but-now-passing** entries (`FIXED`) that must be removed from
`tests/KNOWN_FAILING_TESTS`. The aggregate job fails downstream at its verify step; it is not an
independent test failure. Whole CI is **not green**, and PR #185 stays BLOCKED.

The two `FIXED` doc-eval tests are the tap-marking cost tests. They pass at `c5f2f42`; removing their
allowance lines is a follow-up, not a new allowance. This checkpoint does not change
`KNOWN_FAILING_TESTS`.

### 11.3 Focused fixes with remote evidence

Both rows below are focused remote runs, not whole-CI green. Their task-review verdicts are recorded
as controller provenance; no separate reviewer report file is in the cited evidence directories.

| Work | Reviewed source / workflow head | Run and result | Evidence |
| --- | --- | --- | --- |
| MCP connect regression | source `562973ade451a5903ba23adfd97802242d8d120a`; workflow head `7aeeab3fc32f838985dfe95567ba3d9cdb7ec953`; test blob `46f1ee81c60292e5a3e289a55407504a7ea3d3b4` identical at both | run `44612760207` success; raw nextest exit **0**; **11/11 passed, 0 skipped**; Rust/cargo **1.99.1**, nextest 0.9.148 | `worktrees/int1454-mcp-connect/target/int1454-mcp-review-evidence/` (`README.md`, `nextest.exit`, `nextest.log`, `summary.txt`, `identity.txt`, `junit-status.txt`); ledger `/tmp/opencode/int1454-mcp-connect-ledger.md` |
| Joint reconciliation | source `50e605faa5b81cfccb591182b0329445061f2933` | run `44629972383` success; exact **10 matched, 0 missing, 0 extra**; **10/10 passed**; **2,403 tests and 21 binaries skipped/not executed**; Rust **1.99.1** | `worktrees/int1454-recon/target/int1454-recon-review-evidence/run-44629972383/` (`per-test-results.txt`, `matched-identities.txt`, `selection.json`, `targeted.log`, `run-meta.json`) |

MCP: the change is test-stub fidelity only. The app-options probe is intended; the two stubs now
serve every request line on one connection until EOF. No product logic, permission, auth or parser
semantics changed. The focused workflow did **not** archive JUnit (`junit-status.txt`); the raw
per-test PASS lines are in `nextest.log`, and the run was not restarted just to add JUnit. Task
review: Spec and Quality PASS (controller provenance).

Recon: the earlier task review returned `CHANGES_REQUIRED`; `REVIEW-RESPONSE.md` and
`SCOPED-REVIEW-FIX.md` record the fixes. The final targeted run is the 10/10 above. Task review:
Spec and Quality PASS (controller provenance). The 10-test selection is exact; the 2,403 other tests
were filtered out and were **not executed**, so this is not a full-suite pass.

### 11.4 Joint-normalization scope actually proven

- Actual corpus: a **single joint**, with no origins or links. The allocation is a local id.
- Normalization applies **only** to source-absent derived `x`/`face`/`end`. Stored references are
  compared in full. The comparison table has **14 cases in total: 10 negative (`ne`) and 4 equal
  (`eq`)**, not 7 and 7. The `old_ok=7/14` line is not a 7-negative split: the old helper classified
  7 negative cases as equal. The new model reports 14/14.
- The earlier `2a354996a` failure was left = source absent, right = replay derived. That corrects the
  earlier reverse claim.
- `red-green-model.py` is a **script model** of serialized operations, not a Rust test run. The
  **14-case** table is model output; the Rust evidence is the separate 10/10 targeted run, not the
  14-case model.
- No general multi-joint reallocation-reference proof exists yet. Do not generalize the single-joint
  result to multi-joint reallocation.

### 11.5 Prepared combination branch

`prep/int1454-mcp-connect` = `1eeb7560aaed93d9e8f400f9a6882dc3e4f61325`. Its tree is
`9ea627e37ff5ec531efaace3c1e3410377a693c0`; `git rev-parse 1eeb7560^{tree}` matches that tree.
It combines the reviewed MCP and reconciliation source. The branch-only diagnostic workflows are
**not** in that tree (`mcp-connect-diagnostic.yml` and `int1454-recon.yml` are absent). The branch is
clean and pushed to `origin/prep/int1454-mcp-connect`, then frozen. It is **not** PR #185, and no
CI has run on this combined head.

### 11.6 Integration review and active packaging work

Controller-recorded L2 Sol integration review: **CHANGES_REQUIRED**. Findings include a foreign
Flatpak end-of-life rebase that may move data into the new sandbox, and bypassed sibling grants or
empty ids. The core CLI combination has no blocker in that review. No artifact path for this review
is recorded here.

The active reconciliation worker is fixing packaging. Verified commits on `fix/int1454-recon`:
`fc0d9a0adde2761c6afa2311e05147ee6ec741f8` (publish only Intcad's app id), `7a6abbcc0`
(packaging-lint SC2251 fix) and `dfc6bd789` (remove the temporary targeted workflow). The first
packaging CI, run `44637129117`, failed on SC2251 `! grep`; the fix uses an explicit `if grep`.
Run `44637527571` was the controller’s last-known active owned watch at this checkpoint
(2026-11-19T23:30Z), not a current state. **Do not query, poll or cancel it here.** Task review at
that snapshot said the core behaviour was correct but returned `CHANGES_REQUIRED` for a stale
manifest line and evidence exactness; the owner was then fixing it and investigating mirrored old
end-of-life metadata.

Later controller snapshot (2026-11-19T23:37Z, not polled here): run `44637527571` succeeded, and run
`44638313864` was the last reported pending run, with task review still `CHANGES_REQUIRED` for new
docs/prune fidelity. This is a later snapshot only; the worker’s current state is not chased in this
checkpoint.

Packaging status at this checkpoint: installed Flatpak acceptance **NOT_RUN**. Local packaging tests
were 17 tests with 1 RED, then 0 RED. The nightly execution recorded 12/9 with a missing `jq` and
exit 1; it is classified **BLOCKED (environment)**, not NOT_RUN. None of this is release approval.

### 11.7 Rights incident — explicit owner disposition still blocked

Record: `worktrees/int1454-recon/target/int1454-recon-review-evidence/flatpak-isolation/RIGHTS-INCIDENT.md`.

The incident record says two Flatpak LGPL-2.1 source files were inspected and additional files were
fetched during the same investigation; the linked record is the full account, not a two-file limit.
This happened while investigating the packaging behaviour, although the root rule prohibits reading
GPL/LGPL/AGPL source. The record states this plainly and is not erased. An independent review
found **no copied source, template or implementation** in the patch; the patch is an original
straight deletion plus original tests and documentation. That finding is **not** a rights clearance.

Owner disposition remains **explicitly blocked**. The owner was asked to accept the recorded incident
for continuation of the original patch; no reply is recorded as of this revision. **No waiver,
clearance or licence pass exists.** Do not invent one, and do not read the prohibited external code
again.

Later disposition: see §11.12 (2026-11-20T00:14Z). The blocked status above remains the historical
state at this checkpoint; it is not erased.

### 11.8 Other active work — no completion claim

- Union fallback worktree `int1454-union-fallback`: active per the controller snapshot at
  2026-11-19T23:30Z (not a fresh status). Last-known plan/test-only reference `1c8f7213b`; no GREEN
  or current production approval. Not re-surveyed for completion.
- Validation-transaction worktree `int1454-valtx`: active per the same controller snapshot (not a
  fresh status). Last-known `fa12cfe47` had L3 `CHANGES_REQUIRED` with five findings (retention
  replay, conversation routing, lost wakeup, duplicate overwrite, clock mismatch). No approved
  integrated fix.
- Both are uncompleted. Their work was handed to the official DeepSeek path because Go and GOAT were
  exhausted; that is a provider handoff, not a quota BLOCKED state. Do not poll or restate their
  heads as current beyond the last-known references above.

DAG note: no new dependency edge is recorded by this checkpoint. It adds evidence and status, not a
change to the migration graph.

### 11.9 Remaining final milestones

1. Final landing method, landing commit and final `develop` SHA: **TBD**.
2. Green whole CI at the final SHA, including the native gate and all matrix shards.
3. Final-SHA CD-1 persistence evidence: real save/reopen files, hashes and a durable report. The
   current GUI round trips remain transcript-only.
4. Linux GUI acceptance at the final SHA. The historic `42f4d670a` 8/8 report is not final-SHA
   acceptance.
5. Full Windows `accept` at the final SHA. The `f82b56372` result is smoke only, not a full VM GUI
   E2E.
6. macOS native automation and macOS/desktop PP UI acceptance: **not run**.
7. Owner/legal disposition for the rights incident and every Blocked row, including CD-6 chrome
   provenance.
8. Scoped whole-head final review and sign-off at the final SHA.

No INT-1454 closure, no merge to `develop` and no main release is claimed.

### 11.10 Evidence and commands for this checkpoint

Evidence paths read (read-only):

- `/tmp/opencode/int1454-ci-current/SNAPSHOT-2026-11-19T2045Z.md` and
  `evidence-rust-c5f2f42/rust/{junit.xml,commands.json}`;
  `native-c5f2f42-failures-2026-11-19T2039Z.txt`.
- `worktrees/int1454-mcp-connect/target/int1454-mcp-review-evidence/` (README, identity, summary,
  raw exit, nextest log, JUnit status).
- `worktrees/int1454-recon/target/int1454-recon-review-evidence/run-44629972383/` (per-test results,
  matched identities, selection, targeted log, run metadata), plus `REVIEW-RESPONSE.md`,
  `SCOPED-REVIEW-FIX.md` and `flatpak-isolation/RIGHTS-INCIDENT.md`.

Commands used: `git status`, `git log`, `git rev-parse`, `git cat-file`, `git worktree list`,
`git merge-base --is-ancestor`, `git ls-tree`, `git ls-remote` and `sed`/`grep` on the paths above.
No `gh` query, no CI dispatch/rerun/cancel, no local build and no source edit were performed for
this checkpoint.

Revision provenance: appended by the official DeepSeek worker on 2026-11-19; base
`182d485e2c6b31b324ff45794324f3454b63c67e`; own branch and worktree recorded in the handoff, not
in this document. Earlier sections remain unchanged.

### 11.11 Revision record

- 2026-11-19T23:37Z — fresh L1 doc review fix, wording only. §11.1 no longer claims the seven-line
  engine deletion is validated at c5; it records the c5 ancestry and the 15-entry list (13 allowed
  known failures + 2 `FIXED`), keeps the exact seven-case result local to `61835bfaf`, and leaves
  the independent review/pinned-CI gate open. §11.2 states that the `Action matrix (16/16)` job
  passed while shards (2/16) and (8/16) failed. §11.4 corrects the table to 14 cases in total:
  **10 negative and 4 equal**, and labels it as the script model. §11.6 labels the packaging watch
  as a checkpoint snapshot and adds a later dated controller snapshot. §11.7 records that no owner
  reply or waiver exists. No new CI, build, metadata query or source change was made.
- 2026-11-19T23:44Z — final fresh L1 broad review correction. Verified the actual
  `red-green-model.py` lines 42–59 and `red-green-model.txt` lines 1–16 in the read-only
  reconciliation worktree/evidence: 14 cases = **10 `ne` + 4 `eq`**. `old_ok=7/14` is the old
  helper misclassifying 7 negative cases; the new model is 14/14. The Rust `script_roundtrip.rs`
  lines 120–189 at `50e605faa` carries the stored-reference assertions; the separate targeted run
  is 10 test identities (10/10), not the 14-case model. The enumerated case evidence is in the
  ignored review package. §11.2 now distinguishes the passed
  (16/16) job from the failed (2/16)/(8/16) shards, and §11.7 no longer implies only two files were
  fetched. No state poll, build, CI or source edit was made.

### 11.12 Owner disposition update (2026-11-20T00:14Z)

Superseding entry for §11.7 only. The owner replied **“接受”** (accept) to the explicit proposal to
record the LGPL source-consultation breach and allow this original no-copied-code Flatpak patch to
continue acceptance.

This closes only the pending owner disposition for this incident. It is **not** a waiver for future
reading of prohibited source, **not** legal or cleanroom clearance, and **not** merge, release or
sign-off approval. The §11.7 blocked status remains accurate as the historical state at that
checkpoint, and the original incident evidence is preserved.

Technical status recorded with this disposition: source `eee67717f5c79248f00a2232ae3a6f41dfc0a605`
has a scoped task PASS (controller record); matched CI run `44640487050` succeeded with **23 tests
OK**, and the controller directly checked raw log lines 378/380. Installed Flatpak acceptance is
**NOT_RUN**; the combined candidate review and CI are pending. No source consultation, build, CI or
other worktree change was made for this entry.

### 11.13 Security evidence checkpoint (2026-11-20T00:25Z)

SEC-2 (STEP hostile decode): at `c5f2f42fe6f06ed5c2c4ec1df04f7e9c6aae50ad`, the app STEP import
path routes through `solidcraft_io` and the kernel `step_in` wrapper to the vendored
`solidcore-record` `decode_string_with`; `crates/step` is absent. The read-only audit
`/tmp/opencode/int1454-security-current-audit/` records the source/call-chain check and two exact
captured tests: `step::p21::tests::run_terminators_are_searched_once` and
`step::p21::tests::hostile_text_is_an_error`, each **1 passed, exit 0** under Rust/Cargo **1.99.1**,
`--locked`. The extracted `solidcore` tree `8b5c01445619305dcfc5072db3fe76b8cfea67be` has **8,457
entries, 0 mismatches before/after**; the controller checked both raw logs and the Git tree. This
supplements the c5 JUnit, where these two tests were **NOT_RUN**. It is not final-SHA or full-CI
evidence, and not security acceptance.

SEC-3 (connection-limit refusal drain): **low availability, not fully adopted**. The `crates/control`
scoped limit-refusal drain has a c5 JUnit pass
(`solidcraft-control :: server::tests::a_connection_past_the_limit_reads_why`; all 33 control tests
passed). The app still uses the separate `apps/solidcraft/src/control_server.rs`; its limit-refusal
branch sends the refusal and drops the stream without the drain. The **app path is unverified**.
This is not a permission change and gives no access gain.

Scope: no code, reproduction, build, rerun, CI metadata poll, source consultation or new policy
decision was made. Provider: official DeepSeek; Go and GOAT recorded unavailable; no quota probes.
No Flatpak evidence was duplicated.

### 11.14 U5 owner decision (2026-11-20T04:55Z)

Owner reply: **“可以接受这个方案”** to the narrow U5 proposal. The accepted scope is to retain only
the verbatim `display-units` crate extraction, preserve the functional **Already Covered** status,
and add no new length-display behaviour. The tolerance/dimension-text API remains a separate Doc
acceptance item.

Evidence: upstream extraction `a833316f8b2e84ae44ee789f872efdd78253ae46`; local verbatim import
`ad0262a49cb6ea81d05be2c50c9893833d0e23da`; MIT OR Apache-2.0; the four length-display functions
and six tests are unchanged; `doc` re-exports them at `crates/doc/src/display_units.rs:14`; and
`crates/io/build.rs:17` includes `display-units` in `EVALUATION_CRATES` for evaluation-cache-key
recomputation. The separate `02e6a492c` tolerance/dimension-text delta is not part of this
extraction. Fresh independent review `ses_e08e02d84ffeVX0IugS3JXn7aB` recorded technical
equivalence PASS. The existing c5 `solidcraft-display-units` suite 6/6 is targeted suite evidence
only.

Historical scope conflict: the prior ledger wording **“Already Covered — do not import”** and the
open scope conflict remain the historical record in the v1–v3 handoffs and
`target/int1454-u5-review/`. This entry answers the owner disposition only for this extraction; it
is not a waiver for other rows. No final-SHA, merge, release or sign-off approval is claimed, and no
other disposition is changed.

### 11.15 D5 technical source evidence checkpoint (2026-11-20T17:21Z)

Scope: documentation-only append. It records an independent **technical source** review of the D5
slice, the source/test-merge binding, and a controller-parsed subset of the c5 native JUnit. It is
not a new test run, whole CI, desktop GUI or final-SHA acceptance, and it changes no disposition.

**Independent review provenance.** OpenDesign Eco session `ses_e0618d3edffe5szAKHDvR107oJ` recorded
**Spec PASS and Quality PASS** for the D5 technical source at source
`c5f2f42fe6f06ed5c2c4ec1df04f7e9c6aae50ad` and vendored tree
`8b5c01445619305dcfc5072db3fe76b8cfea67be`. This is controller-recorded provenance; no reviewer
report file is in the repository. It is technical source review, not implementation acceptance of
the whole branch.

**Source/test-merge binding (reverified from retained receipts, not rerun).**

- Source head `c5f2f42fe6f06ed5c2c4ec1df04f7e9c6aae50ad`; test-merge
  `2a20b4b3974d11b68388db8f72facbd44aadedc6`, parents `9714e5b300030f23bc775fa180b1ba7c80047f80`
  (develop) and `c5f2f42…` (PR). `git show -s` in the main checkout.
- The source head `c5f2f42` and the test-merge `2a20b4b` have the exact same tree
  `6f289a06dc8635a3e9306e900b5a9682312069fd`; the merge adds no tree change beyond the source.
- Both revisions have `solidcore` tree `8b5c01445619305dcfc5072db3fe76b8cfea67be`; `git diff
  --quiet v0.44.1 <rev> -- solidcore` is empty for each. `crates/step` is absent at both
  `c5f2f42` and the test-merge.
- The D5 paths `crates/io` and `solidcore/crates/solidcore-step` are byte-identical between
  `c5f2f42` and the test-merge (`git diff --quiet`); the absent `crates/step` is not a
  byte-identical path. A native run at the test-merge therefore carries the same D5 source as the
  source head.
- The retained plan `selection-plan-c5f2f42/plan.json` records
  `sourceRevision: 2a20b4b…`; the receipt `evidence-rust-c5f2f42/rust/selection.json` build
  metadata records `SOLIDCRAFT_SOURCE_COMMIT: 2a20b4b3…`; `commands.json` records `nextest list`
  exit 0, `nextest run` exit **100**, and `xtask known-failing` exit 1. The c5 run was red.
  `junit.xml` is the matching report: **16,839 tests, 24 failures, 0 skipped, 0 errors**.
  This binds the source and test-merge; it does not make the run green.

**Selected results.** The controller's direct parse of
`/tmp/opencode/int1454-ci-current/evidence-rust-c5f2f42/rust/junit.xml` reported **13 selected
test cases passed**, including:

- `solidcraft-kernel::step_export_bytes::step_export_keeps_its_bytes_and_reads_back`
- `solidcraft-engine::tests::sample_plate_step_round_trip`
- `solidcraft-kernel::step_out::tests::assembly_names_colours_round_trip`
- `solidcraft-ui-egui::scenarios::s09_step_roundtrip_hole`

This entry reopened the same JUnit and confirmed all four named cases are present with no
`<failure>` and no `<skipped>` element. The file-level count is 16,839 cases and 24 failures. The
four names are examples, not a new selection; the retained receipts do not store the 13-name list,
so the count of 13 remains the controller's parse and is not regenerated here. This selected set is
separate from the 13 allowed known-failing entries, which failed. The run has 24 failures: 13
allowed known failures and 11 off-list regressions; the 2 `FIXED` doc-eval entries passed and
remain listed (§11.2). The prior snapshot recorded whole CI as not green and PR #185 as
OPEN/BLOCKED, and this entry makes no current status claim. No test was rerun.

**cadmpeg qualification.** At `c5f2f42`, `cadmpeg-core`, `cadmpeg-ir` and `cadmpeg-registry` are
linked (Apache-2.0; `Cargo.toml`, `ATTRIBUTION.md`). Only the `fcstd`, `rhino` and `step` features
are built, and the `step` feature is the **writer** used by `crates/io/src/native_cad.rs` to restate
FreeCAD/Rhino files into STEP text. No callable cadmpeg STEP **decoder** was observed through the
app/engine; `crates/kernel/src/step_in/p21.rs` re-exports the `Entity`, `Exchange` and `Param`
items from `solidcore_step::p21`, not the entire module. This qualifies the
observed call path; it is **not** a link exclusion and does not claim cadmpeg is absent. The
`solidcraft-io` tests `formats::tests::a_host_without_cadmpeg_leaves_its_rows_out` and
`tests::step_import_names_follow_the_choice` pass in the same JUnit; they add no STEP decoder.

**§11.13 boundary.** The two exact vendor tests recorded in §11.13
(`step::p21::tests::run_terminators_are_searched_once` and
`step::p21::tests::hostile_text_is_an_error`) are not duplicated or rerun here. They are `NOT_RUN`
in this c5 JUnit; §11.13's separate captures remain their only result. This entry adds no second
decoder and no SEC-2 result.

**Low advisories and final-SHA checks.**

- The retained CI plan `selection-plan-c5f2f42/plan.json` is bound to test-merge `2a20b4b` and
  records `jobs.packages.decision: not_applicable` and `jobs.packages.reason: no_relevant_change`;
  treat it as a historical plan artifact, not a package or release gate.
- The cadmpeg/FreeCAD rows in `ATTRIBUTION.md` are historical licence inventory, not a refreshed
  rights clearance; this entry adds no attribution evidence.
- Layering rules are candidate-dependent: the test-merge's tree changes `xtask/src/layers.rs`
  (+26/−4) and `xtask/src/mesh_decisions.rs` (105 changed lines) relative to its develop parent,
  including removal of the old `step` layer and its `step → kernel` ordering. This D5 review does
  not prove `cargo xtask layers`, `mesh-decisions`, `lists` or the other rule checks at the final
  SHA. Run them at the final SHA and record their results.
- No final-SHA CI, Linux/Windows GUI E2E or macOS acceptance is claimed.

**Disposition.** This is technical source review evidence only. It does **not** reclassify D5: the
ledger's D5 row (io Adopted; STEP move Blocked until K1) is unchanged, and no owner approval or
waiver is implied. It approves no merge, no `main` change and no release. The prior snapshot
recorded PR #185 as OPEN/BLOCKED; this entry makes no current status claim. No source, ledger, CI,
build, test, worktree or Linear change was made for this entry.

Evidence and commands: `git show -s`, `git rev-parse`, `git ls-tree`, `git diff --quiet`, the
retained JUnit/selection/commands files above, and a Python `xml.etree` count of the JUnit. No
`gh` query, CI dispatch/rerun, local build, test run or source edit was performed.

### 11.16 A6 isolated Windows diagnostic

[Run 44893965299, attempt 1, job 136815530268](https://github.com/intgral-ai/intcad/actions/runs/44893965299/job/136815530268) passed. Workflow commit `9641436fb7bf284fae32582f07af26d3102ccf58` checked out clean product source `581e606f0e30f3404b1bc9cd786a44bc015537ea`. The runner receipt records `win25-vs2026`, image `20261116.315.1`, and Rust/Cargo `1.99.1`.

The five `cargo test -p solidcraft --bin intcad --profile ci --locked` selections all exited zero: startup **18/18**, native crash **2/2**, native frame **1/1**, A6 shared refusal worker **1/1**, and A6 drain ceiling **1/1**. The native `cargo build -p solidcraft --profile ci --locked` and the existing Windows startup and resize smoke scripts also passed. The job log records six startup scenarios, a first frame at 1024×720, and an 80 px bottom-edge resize.

- Evidence ZIP `14704593233`: 14,054 bytes; verified SHA-256 `4a219a27ef7b129556aa317db7d377d95e8db199ddcc153128a999d00daccc69`.
- Job log SHA-256 `f7f55263ba5fb6634a613405a1a048a1b0e0dfb9b00f2b4d54e178f2ac70aa5e`.
- Runner-reported executable SHA-256 `78d2b9a05a6fa866853ebc2ed0f4044f77bb2d4e7dac321d175d3e4f3304742b`. The archive contains the hash receipt, not the executable or screenshots; no independent executable rehash is claimed.

This is isolated hosted Windows evidence, not a full Windows VM E2E run, a Windows TCP limit-refusal check, combined-candidate CI, or landed-`develop` evidence. Keep those gates open and do not integrate the diagnostic workflow.

### 11.17 A6 isolated Linux desktop TCP acceptance

- Product source `581e606f0e30f3404b1bc9cd786a44bc015537ea`, clean. Local Ubuntu 24.04 build produced a direct desktop binary; build and deployment each exited `0`. Build manifest, recomputed host binary, and recomputed deployed binary agree on SHA-256 `ad6f54323258ae93c17ddd9c0d973adf1e9a1f3e3a9d9122aa9b2f5fe1c6cef0`.
- In a dedicated Ubuntu 24.04.5 LTS VM, 16 authenticated simultaneous read-only clients succeeded; client 17 received the exact `too many connections` response and clean EOF; an admitted client remained usable; new admission resumed after disconnection. The guest-local check exited `0`. Log SHA-256 `9e56039cf8bc60379daa69595c756db8dd3ed051e974816af91a0cb8ad4a3c59`. Raw logs and script are retained locally under `/tmp/opencode/int1454-a6-real-tcp/`. The same directory's `vm-shutdown-status.txt` records the dedicated VM in `shutoff` state (SHA-256 `188f984d6d9358f5d4a1446a17340e41e151b7c03aab15f0ceae3f3339ef62df`).
- This covers normal-client Linux loopback refusal on the isolated product SHA. It is not Windows TCP parity, full platform E2E, combined CI, evidence on `develop`, or independent merge approval.
