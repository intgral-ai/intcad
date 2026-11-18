# INT-1454 Phase 0 — frozen baseline

Status: Phase 0 investigation. Nothing here is a claim that the sync is implemented, tested or
accepted. Every number below is reproducible with the command shown.

## 1. Frozen references

| Ref | Value | How to re-check |
| --- | --- | --- |
| Upstream target | SolidCraft **v0.44.1** | `git rev-parse v0.44.1` |
| v0.44.1 object | `e151d5ea6866fb055e011379153fb1cae0566f22` (lightweight tag → commit) | `git cat-file -t v0.44.1` → `commit` |
| IntCAD baseline | `origin/develop` = `0fd92bea266e7dcea5ce09676ed9ff9ae6c66d7f` | `git rev-parse origin/develop` |
| Fork point / merge base | `06749ac464e16357d0375b78e6b3cac214066493` | `git merge-base origin/develop v0.44.1` |
| Previous sync attempt | PR #145, head `f45d977481614fad850c7f9ca67e48e6e7956e5f`, base `develop`, **draft**, `mergeable=false`, `mergeable_state=dirty` | GitHub API `pulls/145` |
| v0.41.0 (previous research) | `3c8efdb51952ed9a5f3ebc722c2bad1454fe6d80` | `git rev-parse v0.41.0` |

The target is a **real tag SHA**, not a guess. v0.44.1 is a lightweight tag, so the SHA is the
commit itself and can be pinned directly.

## 2. Repository relationship

- IntCAD forked from SolidCraft at `06749ac4`; the fork point is still the merge base. Develop has
  **219** own commits since then (`git rev-list --count 06749ac4..origin/develop`), upstream has
  **4994** commits (`git rev-list --count 06749ac4..v0.44.1`), of which **4939** are non-merge.
- Nothing from v0.41.0+ is merged into develop: PR #145 is a WIP merge of v0.41.0 into a side
  branch (`sync/upstream-v0.41.0`), untouched here and not to be merged.
- IntCAD carries its own product crates that upstream does not have (`dfm-rules`, `agent-task`,
  `agent-task-engine`) and keeps `step`; upstream moved `step` into the vendored `solidcore/`
  (`solidcore-step`) and added `format`, `viewport*`, `control*`, `craft-plugin*`, `mcp-server`,
  `ui-*`, `color`, `display-units`.

```
git ls-tree -d --name-only origin/develop:crates   # 21 crates
git ls-tree -d --name-only v0.44.1:crates          # 33 crates
```

Upstream-only: `color control control-egui craft-plugin craft-plugin-wasm display-units format
look mcp-server theme ui-kit ui-registry ui-text viewport viewport-egui viewport-gpu`.
IntCAD-only: `agent-task agent-task-engine dfm-rules step`.

## 3. Release windows inside the delta

| Window | Commits |
| --- | --- |
| `v0.40.1..v0.41.0` | 203 |
| `v0.41.0..v0.42.0` | 351 |
| `v0.42.0..v0.43.0` | 269 |
| `v0.43.0..v0.44.0` | 362 |
| `v0.44.0..v0.44.1` | 3 |
| `06749ac4..v0.44.1` | 4994 (4939 non-merge) |

The ledger covers the whole `06749ac4..v0.44.1` delta because no released upstream content has
been merged into develop. Release windows are used for grouping and audit, not as the scope
boundary.

## 4. Diff scale

| Measure | Value |
| --- | --- |
| `git diff --shortstat origin/develop v0.44.1` | 12,899 files, +9,952,774 / −755,300 |
| Same, excluding `solidcore/`, `vendor/`, `plan/`, `reports/`, locales, action matrix, screenshots, examples | 4,313 files, +563,245 / −729,433 |
| Rust sources only, `crates/**/*.rs` | 1,810 files, +484,621 / −170,614 |

The raw total is dominated by the vendored `solidcore/` subtree (+2.5M lines), upstream
fixtures/corpora (+6.5M), and generated `plan/`, `reports/`, `docs/book`. **Commit count and raw
line count are not the success measure.** Note: Git skipped exhaustive rename detection on the
full diff (`renameLimit`); counts are approximate and must be regenerated with a raised limit
when they become evidence.

## 5. Architecture facts that drive the DAG

1. **Kernel dependency form changed.** Develop pins the kernel by git revision:
   `solidcore = { git = "https://github.com/tangency-labs/solidcore", rev = "8ddce901671f1cebd9214a31d21895d725e886c8" }`
   and `solidcore-record` at a **different** rev `f5e8e9a29e0c9b7cf6c4040a5bc8878382b3c62f`.
   v0.44.1 vendors `solidcore/` in-tree with path dependencies and adds `solidcore-ops` and
   `solidcore-tess` to `crates/kernel`.
2. **The UI split is upstream's, not IntCAD's.** Upstream's egui front end is split across
   `ui-egui`, `ui-kit`, `look`, `theme`, `color`, `ui-text`, `ui-registry`, `display-units`,
   `viewport*`. IntCAD keeps a single forked `crates/ui-egui`.
3. **Brand boundary is licence-defined, not path-guessed**: `LICENSE-BRAND` §1 names
   `crates/ui-kit`, `crates/look`, `crates/theme`, `assets/branding`, `assets/icons/app`.
4. **Doc persistence**: IntCAD must adopt upstream's Doc/Format/save implementation as the single
   implementation. Upstream gained a `crates/format` crate (absent in develop); the exact format
   version delta and compatibility risk are tracked in the ledger, not assumed here.
5. **Develop's `NOTICE` is stale**: it says the kernel "builds on the truck crates", while
   `crates/kernel/Cargo.toml` depends on `solidcore-record` and `solidcraft-smlib`. This is an
   existing attribution/accuracy defect to fix during the licence audit, not a sync result.

## 6. CI and test baseline (develop `0fd92bea2`)

Evidence: GitHub Actions runs at `0fd92bea2` (2026-11-16T21:00Z), all `completed/success`:
CI `43955685642`, Lists `43955685446`, Kernel rig `43955685477`, Fillet rig `43955685536`,
Packaging lint `43955685558`.

- Local full gate: `cargo xtask ci` (fmt, assets, icons, licences, runtime-artifact policy, lists,
  skills, layers, UI containers, mesh decisions, workspace clippy, wasm check, doc tests,
  web-offline, selected nextest). It does **not** run action matrix, kernel rig, packaging matrix
  or the full native suite.
- Known failures are pinned in `tests/KNOWN_FAILING_TESTS` (**42** entries) and checked by
  `cargo xtask known-failing`; a listed test that starts passing also fails the gate. The sync may
  not add entries, skip tests or loosen assertions.
- Validation policy: `docs/validation-policy.md`.
- E2E entry lives only in the main checkout: `/home/hyjing/Code/intgral/intcad/.dev/e2e/bin/`
  (`intcad-e2e`, `intcad-e2e-mcp`, `intcad-e2e-win`, …). Worktrees have no `.dev/e2e` copy.

## 7. Machine and resource constraints (this workstation)

- IntCAD total on disk ≈ **188 GiB** (main checkout 105.3 GiB + worktrees ≈ 83 GiB; the two
  largest are `worktrees/int1484` 53.7 GiB and `worktrees/int1487` 25.8 GiB). Filesystem free
  ≈ 708 GiB.
- Local rule `.opencode/AGENTS.md`: at 180–200 GiB no new heavyweight local build may start;
  prefer CI. Phase 0 therefore uses git reads and small checks only. This is a **hard gate for
  Phase 1–2 planning**: at most one heavyweight local build at a time, and packaging /
  cross-platform validation goes to CI.

## 8. Reproduction

```sh
cd /home/hyjing/Code/intgral/intcad
git fetch origin --prune && git fetch upstream --tags
git rev-parse origin/develop v0.44.1 v0.41.0
git merge-base origin/develop v0.44.1
git rev-list --count 06749ac4..origin/develop     # 219
git rev-list --count --no-merges 06749ac4..v0.44.1 # 4939
git diff --shortstat origin/develop v0.44.1
```
