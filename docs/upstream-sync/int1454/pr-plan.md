# INT-1454 Phase 0 — PR plan, ownership and integration order

Provisional until the ledger is reviewed; the dependency order is what matters, PR numbering is
not. Every PR: one worktree, one owner, upstream SHA in the body, ledger rows updated, affected
checks run, no direct push to `develop`.

## 1. Streams

| Stream | PR | Contents | Ticket | Depends on |
| --- | --- | --- | --- | --- |
| S0 security | P0 | agent-gate canonicalisation + unknown-id refusal (`7b22bb3e2` shape), STEP decode bound if not deferred | INT-1492 | none (can ship immediately) |
| S1 kernel | P1 | `solidcore/` snapshot, workspace pins, `crates/kernel`, `smlib`, `geom`, `dynamics`, kernel corpora | INT-1490 | none |
| S2 foundation | P2 | `color`, `display-units`, `ui-text`, `ui-registry`, `viewport`, `viewport-gpu`, `render`, `gpu`, `space-mouse` | INT-1493 (plumbing) | none |
| S2 foundation | P3 | `format`, `doc`, `io`, STEP through `solidcore-step` | INT-1491 | P1 (STEP), P2 |
| S2 foundation | P4 | `sketch` (3D sketching, constraints) | INT-1491 | P1 |
| S3 commands | P5 | `engine` + schema campaign + IntCAD DFM/project/validation preserved | INT-1491 | P2, P3, P4 |
| S3 commands | P6 | `mcp`, `mcp-server`, `control`, `control-egui`, `craft-plugin*` | INT-1492 | P5 |
| S4 UI | P7 | `ui-egui` adaptation to the new doc/engine/render APIs | INT-1493 | P5, P6 |
| S5 platform | P8 | `rust-toolchain.toml`, `cargo xtask fmt`, CI jobs, nextest config, vendor re-base | INT-1494 | may run early for toolchain/fmt; vendor follows egui bump |
| S5 platform | P9 | `assets/locales`, generated lists, docs/book, apps glue, packaging | INT-1494 | last |

Not in any PR: `crates/ui-kit`, `crates/look`, `crates/theme`, `assets/branding`,
`assets/icons/app` (brand licence). Any icon or theme a feature needs is drawn in-house or
recorded as a UI gap.

## 2. Ordering rationale

- **P0 first** because it closes a live bypass; it is tiny and independent of everything else.
- **P1 and P2 are independent** and may run in parallel (different crates, different owners,
  root manifest touched once by P1 after P2 lands, or vice versa — the integrator serialises that
  single file).
- **P3/P4 need P1 and P2**: `doc` gains `format`/`viewport` edges, STEP needs the vendored
  `solidcore-step`.
- **P5 is the bottleneck**: `engine` consumes `doc`, `io`, `render`, `viewport`, `control`,
  `craft-plugin`. It also carries IntCAD's own ~16k lines (DFM, project, validation), so it is
  single-owner and lands after its inputs, not concurrently with them.
- **P6 depends on P5** (schemas and command surface) and changes the MCP tool contract.
- **P7 last**: the egui fork compiles against the final `doc`/`engine`/`render` API.
- **P8/P9 are cleanup**: toolchain and `fmt` can land early; vendor re-base must follow the egui
  bump; locales/lists/docs are regenerated at the end.

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
| P1 | `cargo test -p solidcraft-kernel`, `cargo xtask kernel-cases --known-failing`, oracle delta explained |
| P2 | `cargo test` for each adopted crate, `cargo xtask layers`, `cargo xtask mesh-decisions` |
| P3 | doc/format/io tests, save→reopen→reopen-data test, `scripts/test-format-pending.sh`, STEP round trip |
| P4 | sketch tests, constraint regressions |
| P5 | engine tests, oracle, action matrix, DFM acceptance, schema coverage (`unconverted.txt` only shrinks) |
| P6 | MCP tool list before/after, gating/refusal tests, HostDispatch/Undo tests |
| P7 | `cargo xtask ui-containers`, UI E2E on the flows in `test-matrix.md` |
| P8 | `cargo xtask ci`, toolchain pin honoured by every job |
| P9 | `cargo xtask lists`, `cargo xtask assets`, `cargo xtask i18n --check`, packaging lint |

## 5. Integration and rollback

- Temporary integration branch allowed for cross-PR verification; never a second mainline.
- Merge order: P0, P8(toolchain), P2, P1, P3, P4, P5, P6, P7, P9. Adjust only with a recorded
  reason in INT-1489.
- A failing PR pauses its dependants; independent PRs stay. Revert unit-by-unit using the ledger's
  upstream range; never revert a verified independent unit to unblock another.
- Rollback of the kernel unit is the largest blast radius: keep the old pins in the PR description
  so the snapshot swap can be reverted in one commit.
