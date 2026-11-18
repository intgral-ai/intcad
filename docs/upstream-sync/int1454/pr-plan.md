# INT-1454 Phase 0 — PR plan, ownership and integration order

Provisional until the ledger is reviewed; the dependency order is what matters, PR numbering is
not. Every PR: one worktree, one owner, upstream SHA in the body, ledger rows updated, affected
checks run, no direct push to `develop`.

## 1. Streams

| Stream | PR | Contents | Ticket | Depends on |
| --- | --- | --- | --- | --- |
| S0 security | P0 | agent-gate canonicalisation + unknown-id refusal (`7b22bb3e2` shape); STEP decode bound | INT-1492 | none (can ship immediately) |
| S5 platform | P1 | `rust-toolchain.toml` (1.99.1), `cargo xtask fmt` | INT-1494 | none |
| S1 kernel | P2 | `solidcore/` snapshot + pins, `kernel`, `smlib`, `geom`, `dynamics`, kernel corpora; mesher API decision (see DAG §5) | INT-1490 | P1 |
| S2 foundation | P3 | `format`, `color`, `viewport`, `display-units`, `ui-text`, `ui-registry` | INT-1493 (plumbing) | P1 |
| S2 foundation | P4 | `doc`, `io`, STEP through `solidcore-step`, `sketch` | INT-1491 | P2 (STEP, mesher), P3 |
| S3 commands | P5 | `engine` + schema campaign + IntCAD DFM/project/validation preserved; `gpu`, `render` consumers | INT-1491 | P2, P3, P4 |
| S3 commands | P6 | `mcp`, `control`, `mcp-server` split decision | INT-1492 | P5 |
| S4 UI | P7 | `ui-egui` adaptation: non-brand logic (snap, silhouettes, timeline, dialogs, render panes, navigation, export/mesh units) | INT-1493 | P5, P6 |
| S5 platform | P8 | CI jobs, nextest config, vendor re-base (egui 0.37) | INT-1494 | P7 for vendor |
| S5 platform | P9 | `assets/locales`, generated lists, docs/book, apps glue, packaging | INT-1494 | last |

**Not planned as PRs** (reason in the ledger): `ui-kit`/`look`/`theme` and app brand assets
(Blocked, licence), `craft-plugin*` and plugin UI (Blocked, CD-5), upstream `mcp-server` split
(Not Applicable now), solidcore mirror/release CI jobs (Not Applicable), upstream screenshots
(Not Applicable).

Not in any PR: `crates/ui-kit`, `crates/look`, `crates/theme`, `assets/branding`,
`assets/icons/app` (brand licence). Any icon or theme a feature needs is drawn in-house or
recorded as a UI gap.

## 2. Ordering rationale

- **P0 first** because it closes a live bypass; it is tiny and independent of everything else.
- **P1 is a cheap unblocker**: the upstream snapshot assumes Rust 1.99.1, and IntCAD has no
  toolchain pin at all.
- **P2 is atomic and cannot be split by "independent" modules**: the current pins do not contain
  `solidcore-ops/-tess/-step`, so v0.44.1's kernel, smlib and geom do not compile until the
  snapshot lands. It is one PR with internal commits per unit.
- **P3 is independent of P2** and may run in parallel; only the root manifest is shared, which the
  integrator serialises.
- **P4 needs P2 and P3**: `doc` gains `format`/`viewport` edges and the mesher types; STEP needs
  `solidcore-step`.
- **P5 is the bottleneck**: `engine` consumes `doc`, `io`, `render`, `viewport`, and carries
  IntCAD's own ~16k lines (DFM, project, validation). Single owner, after its inputs.
- **P6 depends on P5** (schemas and command surface) and changes the MCP tool contract.
- **P7 last among product code**: the egui fork compiles against the final `doc`/`engine`/`render`
  API. It ports logic into IntCAD's own widgets; it does not import upstream files that use
  `ui-kit`/`look`/`theme`.
- **P8/P9 are cleanup**: vendor re-base follows the egui bump; locales/lists/docs are regenerated
  at the end.

## 3. File ownership

| Owner region | Files | Rule |
| --- | --- | --- |
| integrator | `Cargo.toml`, `Cargo.lock`, `xtask/*.txt`, `.github/**`, `deny.toml`, `rust-toolchain.toml` | one change at a time; every other agent requests |
| kernel owner | `solidcore/**`, `crates/kernel`, `crates/smlib`, `crates/geom`, `crates/dynamics`, `tests/kernel-cases`, `tests/oracle` | serial |
| foundation owner | `crates/{format,doc,io,sketch,viewport*,render,gpu,color,display-units,ui-*}` | serial across P2–P4 |
| command owner | `crates/engine/**`, `crates/engine/src/schema/**` | serial, after foundation |
| assistant owner | `crates/{mcp,mcp-server,control*,craft-plugin*}`, `crates/ui-egui/src/agent_chat/**` | serial |
| UI owner | `crates/ui-egui/**` except `agent_chat` | serial, after command owner |
| platform owner | `.github/**`, `xtask/**`, `scripts/**`, `packaging/**`, `vendor/**` | serial |

Two owners never edit the same file in flight. The integrator rebases each PR on the latest
`develop` before merge and re-runs the affected checks.

## 4. Per-PR acceptance

| PR | Must show |
| --- | --- |
| P0 | a test that sends a non-canonical id and is refused/asked; the existing gate tests still pass |
| P1 | `cargo xtask ci` on the pinned toolchain; every workflow uses the same version |
| P2 | `cargo test -p solidcraft-kernel`/`-smlib`/`-geom`, `cargo xtask kernel-cases --known-failing`, oracle delta explained; mesher decision recorded |
| P3 | per-crate tests, `cargo xtask layers`, `cargo xtask mesh-decisions` |
| P4 | doc/format/io/sketch tests, save→reopen→data-integrity test, `scripts/test-format-pending.sh`, STEP round trip |
| P5 | engine tests, oracle, action matrix, DFM acceptance, schema coverage (`unconverted.txt` 339→7 only shrinks) |
| P6 | MCP tool list before/after, gating/refusal tests, HostDispatch/Undo tests |
| P7 | `cargo xtask ui-containers`, UI E2E on the flows in `test-matrix.md`, icons drawn in-house |
| P8 | `cargo xtask ci`, vendor re-base diff reviewed against upstream's |
| P9 | `cargo xtask lists`, `cargo xtask assets`, `cargo xtask i18n --check`, packaging lint |

## 5. Integration and rollback

- Temporary integration branch allowed for cross-PR verification; never a second mainline.
- Merge order: P0, P1, P3, P2, P4, P5, P6, P7, P8, P9. P2 and P3 may land in either order but not
  concurrently on the root manifest. Adjust only with a recorded reason in INT-1489.
- A failing PR pauses its dependants; independent PRs stay. Revert unit-by-unit using the ledger's
  upstream range; never revert a verified independent unit to unblock another.
- Rollback of the kernel unit is the largest blast radius: keep the old pins in the PR description
  so the snapshot swap can be reverted in one commit.
