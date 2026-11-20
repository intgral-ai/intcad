# INT-1454 Union Doc Naming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore the FrontTriangle join and press-pull without accepting unverified persisted face identities.

**Architecture:** Keep the original kernel operation first. Treat the existing exact-error retry and any accompanying Doc history/naming adjustments as one temporary IntCAD patch set, not an upstream design. Doc may recover identity only from verifiable topology provenance.

**Tech Stack:** Rust 1.99.1, IntCAD Doc and Kernel adapter, existing Engine and action-matrix regressions.

**Spec:** [Original fallback constraints](2026-11-19-int1454-union-fallback.md), extended by owner approval on 2026-11-20 to allow the minimum required `crates/doc` face-history and naming work. This is not approval to merge.

## Global Constraints

- Keep vendored `solidcore` byte-identical to tree `8b5c01445619305dcfc5072db3fe76b8cfea67be`; do not change its version or introduce another implementation.
- Keep every result face exact, not merely previously exact faces. Preserve format 58, existing persistence and reference semantics, and test assertions. No baseline blessing, skips or exception-list changes.
- Keep original-order Union first and the existing exact fingerprint, operation and operand guards. Inconclusive history must not produce an accepted model.
- No ordinal based on traversal, sampled geometry or ambiguous siblings. Existing stable references must not silently retarget. Stop and report if the exact evidence is insufficient or a format or contract change would be required.

## Review Focus

- Identify the two originally unverified target faces independently from the four new split faces.
- Prove distinct topology provenance, not just different printed names, for split siblings that share parent and bounds.
- Verify unchanged exact references away from the join, deterministic recomputation after a parameter edit, and stable references after undo/redo, script replay and save/reopen.
- Preserve original-order and other-operation behavior; use the existing guard controls.
- Ensure the local patch can be removed together without removing product regressions or stranding saved references.

### Task 1: Identify the earliest lost identity

**Files:** Read `crates/doc/src/history_names.rs` and its callers; place any focused Doc regression in `crates/doc/src/history_names_tests.rs`. Reuse `crates/engine/src/boolean_regression_tests.rs` and the retained test artifacts.

- [ ] Trace `F11:side:e1` and `F6:side:e1` backwards from the pre-join body to the first call that returns an unverified identity. Record distinct input topology and the exact refusal or ambiguity.
- [ ] Trace both pairs of `F15:start` descendants from strict run `44675835996` through history and the Doc discriminator. Identify a unique, stable exact discriminator for each pair or record the missing evidence.
- [ ] Attach a source-pinned evidence note and use the existing strict RED. Do not repeat the already recorded CI unless a new diagnostic is needed to distinguish a concrete hypothesis.

### Task 2: Repair proven Doc lineage, then validate the product path

**Files:** `crates/doc/src/history_names.rs` and the specific Doc caller proven by Task 1; Doc tests in `crates/doc/src/history_names_tests.rs`; existing Engine checks in `crates/engine/src/boolean_regression_tests.rs`. No vendored changes.

- [ ] Add a focused RED that asserts the two inherited faces have exact identities before the join. Add a second focused RED for distinct identities for each of the two split sibling pairs; permute incidental ordering and assert the mapping to the same topology is unchanged.
- [ ] Repair only the proven identity gap, fail closed on unresolved duplicates, and run the focused Doc suite. No guessed discriminator or broader naming redesign if the evidence cannot distinguish a sibling.
- [ ] Run the existing four `front_triangle_` Engine tests without changing the strict assertion. Require all four to pass, including ordinary-order audit, join, press-pull, edit, undo/redo, replay and save/reopen. Run focused Doc/Engine checks, Clippy and fmt, then all matrix shards on the reviewed integration candidate. Shards 2 and 8 must actually execute the original cases and pass.
- [ ] Obtain fresh task and L3 review. Record exact source, vendor tree, commands, test results and the local patch commit/files. Keep any temporary diagnostic workflow out of delivery. No PR update or merge on partial evidence.

## Local patch retirement: prefer the upstream fix

This patch is an IntCAD workaround for v0.44.1, not a requirement on how upstream fixes the defect. Mark the production call sites with an INT-1454 reference to this plan. Keep retry, history remapping and any Doc adjustments individually identifiable; keep product regressions independent of the workaround.

1. At the next relevant upstream update, record the upstream commit and vendored tree. Test the upstream implementation with **both the local retry and its Doc naming adjustments removed or disabled**. Do not judge it by whether it produces the same internal topology ordering or uses our naming algorithm.
2. Require ordinary-order join and press-pull, all exact names, edit/undo/replay and save/reopen, including a design saved by the local-patch version if that version shipped. Confirm references still identify the intended faces. Use the same strict tests and original matrix actions.
3. If upstream passes, adopt its implementation and **delete** the local retry/remapping and Doc-only workarounds as one retirement change. Retain the product regressions and document the upstream commit and results in the ledger.
4. If upstream corrects the boolean but saved references do not resolve safely, do not preserve both implementations silently or remove compatibility blindly. Report the evidence and seek the smallest separately reviewed compatibility decision. If the upstream fix still fails, update the evidence and re-review the workaround; never carry it forward by default.
