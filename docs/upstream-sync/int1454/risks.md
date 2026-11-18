# INT-1454 Phase 0 — risks, blockers and decision candidates

Severity is about the sync, not about IntCAD's current health. Each risk names the evidence that
created it and the guard that would catch it.

## 1. Risk register

| # | Risk | Evidence | Severity | Guard / mitigation |
| --- | --- | --- | --- | --- |
| R1 | Engine integration is the largest conflict surface | upstream `crates/engine` +144,001 lines; IntCAD added ~16k lines (validation subsystem, DFM commands); 35 files changed on both sides | **High** | single owner for `crates/engine`; adopt upstream file-by-file; keep IntCAD modules as separate files; oracle + action matrix |
| R2 | UI cannot be taken from upstream and must be adapted by hand | upstream `crates/ui-egui` +155,110 lines across 537 files, 63 overlapping IntCAD files; brand crates forbidden | **High** | treat UI as an adaptation unit, not a migration unit; record "backend migrated, UI not exposed" per feature; GUI E2E on the flows IntCAD actually has |
| R3 | Kernel is a snapshot replacement with disjoint history | v0.44.1 vendors `524be9e1…`; IntCAD pins `8ddce901…`, 3,501 commits behind, `merge-base` exits 1 | **High** | replace the payload, run `cargo xtask kernel-cases --known-failing` and the oracle; any expected-value change needs before/after evidence |
| R4 | File persistence behaviour changes for real user files | upstream added `crates/format`; IntCAD has its own `crates/doc/src/format/pending` flow; `doc/src/format.rs` changed on both sides | **High** | adopt the upstream implementation only; test save→reopen→data integrity; raise a Compatibility Decision if the written format becomes unreadable by older builds |
| R5 | Known-failing list will move under a new kernel | 42 entries, many state "as on SMLib"/"solidcore refuses…" | Medium | refresh with `cargo xtask known-failing --junit`; a line is removed only when its test passes; never add a line to hide a regression |
| R6 | MCP contract collision | 7 of 9 upstream-touched `crates/mcp` files are also IntCAD-modified; IntCAD rewrote the server | Medium | single owner; keep IntCAD HostDispatch/gating; compare tool names and schemas before/after |
| R7 | Two pins of the same solidcore repository | `solidcore` at `8ddce901…`, `solidcore-record` at `f5e8e9a2…` | Medium | converge on the vendored snapshot's single tree; check for duplicate crate sources in `Cargo.lock` |
| R8 | Build feedback is CI-bound | disk ≈188 GiB of the 200 GiB budget → heavy local builds forbidden | Medium | area checks locally, full lanes on CI; one heavy build at a time after cleanup |
| R9 | Brand-blocked UI assets create feature gaps | `crates/look` (455 files), `crates/ui-kit` (115), `crates/theme`, app icons | Medium | record gaps in the ledger; draw icons in-house; never copy brand material |
| R10 | Ledger can silently miss commits | 4,939 non-merge commits; 2,654 touch paths retired at v0.44.1 | Medium | path/commit coverage script plus a reviewer independent of the implementer; "Already Covered"/"Not Applicable" need cited evidence |
| R11 | PR #145 is dirty and tempting to reuse | draft, `mergeable_state=dirty`, 4,017 commits, 10,399 files | Low | keep it draft; take only verified findings, never code |
| R12 | Scope creep beyond `develop` | release/tag/website are out of scope | Low | stop at `develop`; no tag, no package release |

## 2. Blockers

| Blocker | Unit(s) | Why | Unblock |
| --- | --- | --- | --- |
| Brand-restricted UI kit, look, theme, app icons | INT-1493 UI work | `LICENSE-BRAND` §1 forbids use outside SolidCraft | cannot be lifted; design in-house equivalents or record a gap |
| Upstream UI screenshots and docs that show SolidCraft chrome | docs/evidence | not licence-blocked but identity-confusing | capture IntCAD's own evidence |

## 3. Compatibility Decision candidates (not decided here)

The Spec lets ordinary, compatible new behaviour ship after tests pass. These change what a user
or an external agent observes, so they go to the owner before merge:

1. **CD-1 — design file format.** If adopting the v0.44.1 Doc/Format implementation writes a
   format that older IntCAD builds cannot open, or silently upgrades an opened file: choose
   between (a) upstream-only, new files need the new build, (b) keep an older write path —
   (b) is forbidden by the single-implementation rule, so the real question is user messaging and
   a minimum-version note in release notes.
2. **CD-2 — MCP/agent tool surface.** If upstream renamed or reshaped tools that IntCAD's agents
   and HostDispatch expose, external automations break. Decide whether to alias old names.
3. **CD-3 — command/default behaviour.** Any command whose default changes (e.g. extrude distance
   origin, view switcher) must keep old behaviour behind a preference per repo rules.

## 4. What would stop the sync from starting

Phase 0 gate fails only if: the SHAs cannot be reproduced; the ledger cannot be verified; licence
status of a unit is unclear and it is load-bearing; or the baseline CI is not green. Current
status: SHAs frozen and reproducible, baseline green, licence boundary defined. The ledger and its
independent review remain the gate.
