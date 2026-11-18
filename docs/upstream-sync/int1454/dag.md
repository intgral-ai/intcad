# INT-1454 Phase 0 — migration dependency DAG and ownership

Derived from the real workspace manifests at `origin/develop` and `v0.44.1` (parsed
`crates/*/Cargo.toml`, `workspace = true` and `path =` edges), plus file-level overlap between
IntCAD's own 219 commits and upstream's 4939. Not derived from ticket names.

## 1. Upstream v0.44.1 crate graph (internal edges only)

```
color          (leaf)
format         (leaf)
control        (leaf)
mcp-server     (leaf)
craft-plugin   (leaf)
ui-text / ui-registry / display-units (leaf)
viewport       -> color
render         -> color, geom, viewport
viewport-gpu   -> color, viewport
viewport-egui  -> viewport-gpu
craft-plugin-wasm -> craft-plugin
theme          -> color
look           -> theme, ui-kit
ui-kit         -> color, theme, ui-registry, ui-text, viewport-egui, viewport-gpu
doc            -> color, derive, display-units, format, geom, kernel, profile, render, sketch, viewport
io             -> color, doc, format, geom, kernel, profile, sketch, testkit
engine         -> color, control, craft-plugin, craft-plugin-wasm, doc, geom, io, kernel,
                  profile, render, sketch, viewport
gpu            -> color, engine, geom, io, kernel, render, viewport, viewport-gpu
mcp            -> control, engine, gpu, mcp-server, viewport, viewport-gpu
ui-egui        -> color, control, control-egui, engine, gpu, look, space-mouse, theme, ui-kit,
                  ui-registry, ui-text, viewport, viewport-egui, viewport-gpu
```

`kernel` depends on `solidcore-record` (and, at v0.44.1, `solidcore-ops`, `solidcore-tess`,
`solidcore-step`), which are path crates of the vendored `solidcore/` subtree, not `crates/`.

## 2. Where IntCAD's graph differs

- IntCAD `engine` additionally depends on **`dfm-rules`** (IntCAD-only crate).
- IntCAD `ui-egui` additionally depends on **`agent-task`, `agent-task-engine`, `mcp`**; it does
  not use `ui-kit`, `look`, `theme`, `viewport*`.
- IntCAD keeps **`step`**; upstream removed it and moved STEP into `solidcore-step`.
- IntCAD has **no** `format`, `viewport*`, `control*`, `craft-plugin*`, `mcp-server`, `ui-*`,
  `color`, `display-units`.
- IntCAD `kernel` pins `solidcore` and `solidcore-record` by **git revision**
  (`8ddce901…` and `f5e8e9a2…`), not by path.

## 3. Kernel is a snapshot replacement, not a bump

The solidcore tree vendored at v0.44.1 is commit `524be9e16707f726586a7ab8696d87fd633e2096`
(root tree `8b5c0144…`), 2026-11-17. IntCAD's pinned `8ddce901…` (solidcore) and `f5e8e9a2…`
(solidcore-record) are 2026-11-02/03 and are **3,501 commits behind** that snapshot. The pinned
revs and the release snapshot have **disjoint history** (`git merge-base` exits 1), so the
kernel migration is not a fast-forward: it replaces the dependency payload and re-validates every
kernel behaviour. IntCAD holds no local solidcore patches (it consumes the crate), which keeps the
replacement tractable.

Also a governance defect to settle during the kernel unit: the two pins are different revisions of
the **same** repository, so `solidcore` and `solidcore-record` can resolve to two source trees.

## 4. File-level conflict matrix (changed on both sides since the fork)

| Area | IntCAD-changed files | Upstream-changed files | Overlap | Risk |
| --- | --- | --- | --- | --- |
| `crates/kernel` + `smlib`/`geom`/`dynamics` | 6 | 188 | 3 | low |
| `crates/sketch` | 0 | 43 | 0 | none — clean adoption |
| `crates/doc` | 11 | 190 | 9 | medium; `format.rs`, `eval.rs` are persistence/eval hot spots |
| `crates/engine` | 73 | 467 | 35 | high; IntCAD added a 6k-line validation subsystem + DFM commands |
| `crates/io` (+ `step`) | 11 | 79 | 10 | high proportion; `formats.rs`, `design.rs` |
| `crates/mcp` | 8 | 9 | 7 | very high; IntCAD rewrote most of what upstream also touched |
| `crates/ui-egui` | 106 | 537 | 63 | very high; independent forks of the same files |
| `assets` | 96 | 496 | 42 | medium; locales/icons are generated and sorted |
| `xtask` | 13 | 26 | 8 | medium; `layers`, `lists`, `licences`, `kernel_cases` |
| `.github` | 20 | 27 | 16 | medium; IntCAD CI is authoritative |

IntCAD-only modules layered on shared files (must survive the sync):
`crates/engine/src/validation/**` (~6k lines), `crates/doc/src/sheet_dfm.rs` (2,803 lines),
`crates/io/src/candidate.rs` (738 lines), `crates/engine/tests/dfm_acceptance.rs`,
`crates/dfm-rules`, `crates/agent-task*`.

## 5. Change order (waves)

Waves are ordered by the dependency edges above; a later wave may start only when its inputs are
merged and green.

| Wave | Units | Why here |
| --- | --- | --- |
| W0 | `solidcore/` snapshot + workspace pins; leaf crates `color`, `format`, `control`, `mcp-server`, `craft-plugin`, `ui-text`, `ui-registry`, `display-units`, `viewport` | leaves; everything else consumes them |
| W1 | `render`, `viewport-gpu`, `kernel` (source on the new snapshot), `geom`, `sketch`, `smlib` | consume W0 only |
| W2 | `doc` (model + `format` registry + persistence), `io` (+ STEP through `solidcore-step`) | consume W1 |
| W3 | `engine` (+ IntCAD DFM/agent commands), `gpu` | consume W0–W2; owns the command contract |
| W4 | `mcp` (+ `mcp-server`, `control`), `craft-plugin-wasm` | consume engine |
| W5 | `ui-egui` adaptation to the new engine/doc/render APIs | consumes the changed contracts |
| W6 | `apps/*`, `assets/locales`, sorting/generated lists, `xtask`, CI, packaging | consume everything; no product logic |

**Not in the waves:** `ui-kit`, `look`, `theme`, `assets/branding`, `assets/icons/app`
(`LICENSE-BRAND`). They are excluded on licence grounds, not on difficulty.

## 6. Ownership rules

1. **One owner per shared contract file.** Root `Cargo.toml`, `Cargo.lock`, workspace dependency
   table, `xtask/*.txt` seam lists, `.github/**`, `assets/locales/**`, `crates/engine/src/schema/**`
   are touched only by the integrator or by the one wave owner allowed to change that contract.
   Everyone else requests the change.
2. **Never two agents in one wave region.** `engine`, `io`, `mcp`, `ui-egui` are single-owner
   serial regions; they may not be worked in parallel.
3. **Parallel-safe pairs**: kernel vs doc/io (different crates), viewport/render vs craft-plugin/
   control, docs/tooling vs product code — provided they do not both edit root manifests.
4. **`assets/locales` and generated files** are regenerated by `cargo xtask lists` /
   `cargo xtask i18n`, never hand-merged.
5. **No agent pushes `develop`.** Each unit lands through its own PR from its own worktree.

## 7. Concurrency and machine limits

- Disk is ≈188 GiB of the 200 GiB budget → **no new heavyweight local build** may start; at most
  one heavy build at a time and only after reclaiming space or on CI. Phase 0 stays read-only.
- Each agent gets its own worktree and checks only its area (`cargo check -p …`,
  `cargo test --profile ci -p …`). Full workspace, packaging and cross-platform runs go to CI.
- A mutable shared Cargo target is not accepted as evidence for another agent's worktree.

## 8. Provisional PR slicing

Final slicing follows the ledger units; the provisional order is one PR per wave entry above,
with kernel split into (a) solidcore snapshot + pins and (b) kernel source changes if the
combined diff is reviewable. Affected tickets: INT-1490 (kernel), INT-1491 (doc/engine/assembly/
io), INT-1492 (assistant/MCP/security), INT-1493 (UI logic), INT-1494 (build/CI), INT-1495
(integration/acceptance). Where the real dependency closure forces a different split, INT-1489
records the change and why.
