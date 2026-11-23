# INT-1454 Blocked disposition worksheet

Scope: the ledger at integration candidate `db0b8dd350bbdffed8927845c86c5028d280cb0e`, the prior independent disposition review, and the acceptance receipts. This worksheet does not reclassify rows, grant a waiver, or sign off on the migration.

| Item | Current evidence and disposition | What remains |
| --- | --- | --- |
| A3 — Wasmtime plugin runtime | Blocked; CD-5 explicitly approves deferral. | A future product decision to ship plugins, dependency licence and packaging review, and capability security review. |
| U18 — plugin UI | Blocked by A3. CD-5 excludes the runtime from this sync; the ledger records U18 as a dependent exclusion. | Keep out of this sync. Record U18 separately in owner sign-off; a later implementation needs an approved runtime and UI acceptance. |
| U9 — `ui-kit`, `look`, `theme` | Blocked under approved CD-6. | Keep brand-era code out. A later proposal would need proven rights or an independently implemented equivalent. |
| U10 — post-brand icons and assets | Blocked under approved CD-6. | Keep excluded; use original or proven pre-brand assets for future work. |
| U11 — kit-coupled panel foundation | Blocked under approved CD-6; the upstream implementation depends on the excluded UI kit. | Any later adaptation must establish compliant provenance and run against IntCAD's UI without the excluded kit. |
| CD-6 — unnumbered chrome | Approved exclusion includes licence-uncertain chrome and `prefs*`. The prior review indexed paths and blob identities, not licence clearance. | Require a provenance decision before relying on uncertain code, including code already present in the candidate. |
| U14 — airfoil desktop entry | **Blocked — owner-approved deferral for this sync.** Retain `sketch.insert_airfoil` and existing Agent access. The selected case `a_naca_0012_section_extrudes_to_its_area_times_the_depth` passed in the otherwise red historical c5 native run. At `db0b8dd35`, the `AGENT_ONLY` entry is attributable to IntCAD commit `7755e4c47`, but its containing `workspace.rs` remains on the CD-6 exclusion list. The alternative drawing-import route is not independently cleared: proposed dialog helpers include U9 material. See the [owner disposition record](https://linear.app/intgral/issue/INT-1454#comment-ba9b9a09-bed6-40e4-829f-460b1f1ac241). | Before resuming, verify a compliant routing path under CD-6 and complete desktop acceptance. Backend evidence is not UI acceptance. |
| U17 — problem tickets | **Blocked — owner-approved deferral for this sync.** Do not connect to the upstream ticket service. See the [owner disposition record](https://linear.app/intgral/issue/INT-1454#comment-5b68d6cf-6587-49b1-bc37-a359986312c2). | Before resuming, define the IntCAD-owned intake channel and how reports will be handled. |
| U17 — Windows policy | **Blocked — owner-approved deferral for this sync.** Do not adopt managed update checks, organization-name display, or ADMX/ADML templates. Preserve current update behavior. See the [owner disposition record](https://linear.app/intgral/issue/INT-1454#comment-5b68d6cf-6587-49b1-bc37-a359986312c2). | Before resuming, define which user choices an administrator may override and which deployment templates IntCAD will support. |
| D5 — STEP move | Independent technical review passed at c5; see [acceptance §11.15](acceptance.md). The ledger records K1 as Adopted and the SEC-2 linear decoder as present, but the D5 row still contains the old K1 condition. | Reconcile that wording and complete final-SHA kernel/STEP checks, `cargo xtask layers`, `mesh-decisions`, `lists`, and related rule checks. This is an engineering gate, not a request to waive it. |

## Owner decisions recorded 2026-11-23 (later round)

**KVL — kernel vendoring.** The owner chose to land the vendored `solidcore` closure together with
the kernel adaptation it requires, not the subtree alone. Measured reason: the sync candidate
replaces the pinned kernel with a closure whose API differs, so `crates/smlib` (44 files),
`crates/kernel` (109) and `crates/geom` (29) must change with it, and the candidate also removes
`crates/step`. A subtree-only commit would leave `develop` uncompilable and CI red. Agreed shape:
one PR carrying the vendored `solidcore/` tree, the `smlib`/`kernel`/`geom` adaptation, the
`crates/step` removal, and the manifest/lockfile/`deny.toml` changes, split into reviewable commits.
`solidcore/` must arrive as a `git subtree` per `docs/book/src/dev/kernel.md`, keeping upstream
history and its Apache-2.0 licence with the NVIDIA copyright line. Generated files
(`THIRD-PARTY-LICENSES.txt`, `ATTRIBUTION.md` rows) are updated by tooling, never by hand.
Nothing is landed or enabled by this record.

**K12 Q:10–12.** Q:10 is recorded as not applicable on the current code, where the native
render-preparation retirement path does not exist; if the landed head gains `render_preparation`,
Q:10 returns to required evidence. Q:11 (workspace shared-menu fixture) and Q:12 (shell scenario
fixture) are deferred separately under U11/CD-6. None of the three counts as a pass and none grants
an allowance.

**KVL outcome 2026-11-23 (owner decision).** The vendored-kernel slice was built and verified, then
deliberately **left unlanded and without a PR**. Branch `prep/int1454-kvl` (pushed) holds three
commits on `develop` `be9cb2c16`: the `git subtree` import `65fc039277` (tree `8b5c0144…`, 8457
files), the kernel wiring `e9dca37fda` (five `solidcore*` deps moved from pinned git revisions to
path dependencies, `crates/kernel`/`smlib`/`geom` adopting the closure API, `crates/step` removed,
`deny.toml` losing its allowed git source), and the consumer alignment `090ed5fd9d` (engine, doc,
io, render, gpu, ui-egui, sketch, dynamics, viewport*, CLI, apps, tests, examples plus the new
`format`/`color`/`display-units`/`control`/`mcp-server` crates). Head
`090ed5fd9d5ef6fbe922c4504203d557fb669750` (tree `31bf1ee3…`) passes
`cargo check --locked --workspace --all-targets --profile ci` with exit `0` and no errors, and the
lock file is byte-identical before and after that run (840 packages; `pgp 0.21.1`, `rsa 0.9.10`,
`crypto-bigint 0.5.5`, `solidcore-step` and `solidcraft-format` present; `solidcraft-step` gone).
No two kernels coexist: zero `tangency-labs/solidcore` references remain in `Cargo.toml` or
`Cargo.lock`. Develop's later work was preserved by per-file three-way merges against the fork point
`9714e5b3`: the right rail and Files work (#174, #210), the agent connection pages (#177, #178), the
rail task list (#213), the FreeBSD paste fix (#219), the dependency bumps (#211, #218) and the A6
refusal delivery (#209).

**Superseded same day: the owner rescoped `#185` and approved carrying its kernel part here.** The
slice is now draft [#220](https://github.com/intgral-ai/intcad/pull/220) (base `develop`, head
`090ed5fd9d`, three commits). The reason the earlier record withheld it still stands and is stated in
the PR: `#185` already carries the identical tree (`git ls-tree c5f2f42fe -- solidcore` is
`040000 tree 8b5c0144…`), so the two cannot diverge in content, and a second PR for content an open
lower-numbered PR covers is what the contribution rules forbid — hence the owner rescope rather than
a competing PR. The slice carries no Union swapped-operand retry: the string
`the proto edge is not listed` does not appear anywhere in its code, so that decision is untouched.

Opening the PR does not land anything: B3 stays closed, no integration SHA exists, and every
final-acceptance gate listed elsewhere in this file is still unverified.

## Separate unresolved gates

- **Union naming:** retain the strict persisted-reference contract. Run `44675835996` did not pass; do not integrate the fallback. Reconsider only with a repair that passes the existing contract or a separately approved change to scope.
- **A2 / U8 — shared control crate versus desktop wiring:** At `db0b8dd35`, the engine uses shared token/auth helpers through `crates/engine/src/control_auth.rs:4,16`, and MCP uses the shared client via `crates/mcp/src/backend/remote.rs:8`. `apps/solidcraft/src/main.rs:1327–1334` reads `agent_control` when planning the main listener at startup, then registers a separate app-local conversation entry point. Neither is the upstream live switch. The shared server and request/watch types are not wired into the desktop path; track the live-switch residual here once, not as a separate U8 adoption or waiver.
- **U8 — typed method bodies:** At frozen upstream `e151d5ea`, the UI-local decoder in `crates/ui-egui/src/control_params.rs:13` handles method bodies through `control.rs:641–776`, including `engine.execute`, `engine.batch`, and several UI methods. At `db0b8dd35` that module is absent. The app imports the UI-local `ControlRequest` (`apps/solidcraft/src/control_server.rs:15`), whose body remains `params: Value` (`crates/ui-egui/src/control.rs:64–76`). Route-specific checks and narrower typed decoding still exist: the nested `ui.menu` target (`control.rs:1346`), the merged camera (`control.rs:1484–1501`), and inner panel/layout command inputs (`crates/engine/src/panels/commands.rs:354–414`). Those do not establish parity for the outer method bodies. This is source evidence only; no candidate runtime parity receipt was verified.
- **U8 — `control-egui` split:** At `e151d5ea`, `crates/control-egui/src/{window,screenshot}.rs` provide the resize helper, screenshot state and encoding; `crates/ui-egui/src/control.rs` calls those helpers. `db0b8dd35` has no `crates/control-egui`; `ui.resize` is still implemented in `crates/ui-egui/src/control.rs`. The split is not adopted, independently of whether the local UI methods behave the same. Shared auth/client wiring under A2 does not close this residual. This records source scope only, not approval to change desktop wiring.
- **A6 / SEC-3 — refusal delivery:** At the same candidate, `crates/control/src/server.rs` implements a bounded refusal drain, while the desktop connection-limit branch in `apps/solidcraft/src/control_server.rs` writes the refusal and closes without that drain. The historical 33/33 shared-crate result (§11.13 of [acceptance](acceptance.md)) does not exercise desktop refusal delivery. Track the desktop gap independently of A2; do not change the disposition based on shared-crate tests.
- **A6 isolated repair:** Task branch `fix/int1454-a6-desktop` at `581e606f0e30f3404b1bc9cd786a44bc015537ea` adds one process-wide refusal worker and one waiting slot, without changing A2 wiring. The scoped L3 follow-up closed the worker-lifetime finding; the pinned local desktop control suite passed 32/32, with Clippy and format checks passing. The [isolated Windows diagnostic](acceptance.md#1116-a6-isolated-windows-diagnostic) passed 23/23 selected tests plus startup and resize smoke. [Real desktop TCP refusal passed in the isolated Linux VM](acceptance.md#1117-a6-isolated-linux-desktop-tcp-acceptance). Windows TCP refusal, full Windows VM E2E, and combined-candidate CI remain `NOT_RUN` for this delta. The repair is not in the integration candidate.
- **A6 independent patch preflight:** The product-only diff `db0b8dd35..581e606f` for `apps/solidcraft/src/control_server.rs` passed `git apply --cached --check` in a disposable index seeded from `1b7255797`. The GitHub contents API confirmed that the file at `develop` `79bf25d89` has the same Git blob (`caaec8b9c33a5e4af36d8a1eb32ce2e028a34675`). No product changes were applied. This establishes textual applicability only, not independent build, behavior, CI, or mergeability.
- **A1:** the candidate uses `crates/mcp-server` in the CLI and desktop chat bridge. The [candidate CD-2 record](mcp-tool-map.md#candidate-a1-disposition) distinguishes this wiring from omitted `cancel` and `batch.path` behavior. [Acceptance §5a](acceptance.md) records the historical review claim and its evidence limits; remaining tool-by-tool classifications and candidate runtime evidence are not closed here.

The issue's tracker state is not owner acceptance. Final CI, persistence, platform evidence and final owner sign-off remain separate.
