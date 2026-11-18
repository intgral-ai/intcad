# INT-1454 acceptance package (draft — incomplete until sign-off)

Status: the v0.44.1 sync is integrated in PR #185 and CI is running at `b06d531e5`. This package is
not a completion claim; it records what exists, what is verified, and what is still open.

## 1. Fixed references

| Item | Value |
| --- | --- |
| Upstream target | SolidCraft v0.44.1 = `e151d5ea6866fb055e011379153fb1cae0566f22` |
| Vendored kernel | solidcore `524be9e16707f726586a7ab8696d87fd633e2096`, tree `8b5c01445619305dcfc5072db3fe76b8cfea67be` |
| Pre-sync develop baseline | `0fd92bea266e7dcea5ce09676ed9ff9ae6c66d7f` |
| Fork point | `06749ac464e16357d0375b78e6b3cac214066493` |
| Sync head under review | `b06d531e5` (PR #185) |
| Final develop SHA | **TBD after merge** |

## 2. Merged work

| PR | Scope | Merge commit |
| --- | --- | --- |
| #177 | Phase 0 archive: baseline, ledger, DAG, coverage tool, CD-1…7 | `2c46e8fba` |
| #180 | Rust 1.99.1 pin + 29 CI install sites | `3cb29651e` |
| #182 | `cargo xtask fmt` (Windows-safe batched rustfmt) | `aca5b768e` |
| #179 | SEC-1: one command id for judging, remembering and running | `96a247ccb` |
| #185 | Kernel, doc/format/IO, engine, MCP, GPU/viewport and UI sync | **TBD after merge** |

## 3. Ledger coverage

`tools/coverage.py` assigns every non-merge commit in `06749ac4..v0.44.1` and every distinct path
to an area, and exits non-zero on an orphan (4,939 commits, 15,357 paths, 0 unmatched at Phase 0).
The disposition table is `ledger.md`; the owner approved CD-1…CD-7 (§7 there).

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
| Rules | `xtask layers`, `mesh-decisions`, `lists`, `assets`, `ui-containers`, `fmt` clean |
| CI | run IDs to be recorded from PR #185's final green run |

## 5. Persistence (CD-1)

The single implementation is `solidcraft_format`'s registry/container used by `doc`/`io`. Test
`design::tests::a_design_saved_at_format_51_with_its_pending_changes_saves_and_reopens` builds a
file exactly as Intcad 0.1.1 wrote it, saves it as `solidcraft/58` and reopens it with identical
data. GUI-level verification (open a real project, save, reopen) and the new
`open_newer_files` preference are to be recorded at the final SHA.

## 6. Platform acceptance

| Platform | Status |
| --- | --- |
| Linux native CI | in PR #185's run (to record) |
| Linux GUI E2E | not run |
| Windows native CI + VM GUI E2E | not run |
| macOS native automation | not run |
| macOS/desktop UI (PP) | not run |

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
