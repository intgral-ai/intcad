# Assistant Visual Release Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the oversized composer and collapsed clipping/backing defects before IntCAD 0.1.3.

**Architecture:** Keep one bottom-centre Assistant. Test shared sizing at the bundled-page boundary. Diagnose clipped text and exposed backing using the real Linux native host; change the layer that produces each measured defect rather than masking it with global zoom.

**Tech Stack:** Rust, egui, GTK/WebKitGTK, WKWebView, WebView2, React, Playwright, IntCAD desktop E2E.

**Spec:** `docs/superpowers/specs/2026-11-23-assistant-release-visual-design.md`

## Global Constraints

- Keep the bottom-centre Assistant from #221.
- Do not change overall desktop or CAD zoom, non-centred chat, or camera behaviour.
- Accept installed bytes from the same merged-develop-based candidate SHA.
- Do not publish without explicit CTO approval.

## Review Focus

- Fractional DPI and narrow-window resize must not crop the status line or reveal backing (Task 1 and Task 3).
- Collapsed error vs ordinary status must both fit; hidden content must not capture clicks (Task 1 and Task 3).
- A connection change must not resize a draft composer; the side-panel sizing must be unchanged (Task 2).
- Open dialog or menu and return focus: dismissal must not consume the next composer or model click (Task 3).
- A different WebView engine or earlier package run does not prove visual acceptance (Task 4).

---

### Task 1: Pin a native Linux reproduction

**Files:** `crates/ui-egui/src/agent_chat/panel.rs`, `crates/ui-egui/src/agent_chat/web.rs`, `apps/solidcraft/src/assistant_web.rs` (diagnostic trace points only); evidence in the dedicated VM task directory.

**Interfaces:** Consumes page `{type: 'size', height}`, host `web_height`, `Placed` bounds, and GTK widget allocation. Produces the before snapshot and the exact boundary that loses a row or exposes backing.

- [ ] Use `intcad-e2e --source /home/hyjing/Code/intgral/intcad/worktrees/int1503-preview --task ui221-repro` for the source preview `f4a7cf179b`. Record VM, binary SHA-256, and source identity. If an Ubuntu-compatible binary is needed, build the same commit in the VM or dispatch the affected build; do not substitute a pre-#221 package.
- [ ] With a clean profile and Sample Plate, open Assistant with an external agent and a synthetic status/error without uploading any credentials or user data. Collapse by clicking the viewport. Capture the whole app after collapse and after narrow resize. Save separate evidence at 100% and 150% for normal and error status.
- [ ] If the screenshot shows a cut row, compare the page's requested height, host rectangle, device-pixel rounding, GTK allocation and visible page bounds on the same transition. Measure status glyph clipping and backing outside the intended outline independently. Stop native implementation until these distinguish the failing layer; record attempts if the cut row cannot be reproduced.

### Task 2: Normalise centred-composer sizing

**Files:** `.agents/skills/verify-intcad/browser/test_assistant_ui.py`, `packages/assistant-web/src/assistant.css`, `packages/assistant-web/src/main.tsx` only if necessary to expose the scoped state, and generated `packages/assistant-web/dist/` plus inventory.

**Interfaces:** Consumes root `html.centre` and existing `data-sizes="mastra"` flag. Produces centred sizing independent of agent selection without changing the docked mode.

- [ ] Add `test_centred_composer_uses_same_scale_across_agent_types` to the existing suite. Use the same centred fixture and viewport for both sizing modes; assert root size `16px`, input size `14px`, and no more than `1px` change in composer height. Contrast a non-centred external session retaining `20px` root size.
- [ ] Run `python -m pytest -v .agents/skills/verify-intcad/browser/test_assistant_ui.py -k centred_composer_uses_same_scale` against the current dist and verify the centred external case gets `20px` rather than `16px`.
- [ ] Scope the `100%` root sizing rule to `html.centre` as well as `html[data-sizes='mastra']`; rebuild dist and its inventory using the pinned build documented in `packages/assistant-web/README.md`.
- [ ] Re-run the targeted test and the Assistant browser suite. Verify draft text and focus survive agent selection; commit the size fix and regression separately from the clipping fix.

### Task 3: Fix native collapse/backing where Task 1 proves it

**Files:** `apps/solidcraft/src/assistant_web.rs`, `crates/ui-egui/src/agent_chat/tests.rs`, `crates/ui-egui/src/agent_chat/panel.rs`, `packages/assistant-web/src/App.tsx` only for the measured losing boundary. Keep macOS `apps/solidcraft/src/assistant_clip.rs` unchanged unless macOS evidence calls for a fix.

**Interfaces:** Consumes Task 1's failing transition, `Placed` native bounds and the page's size message. Produces a native surface with a fully shown or fully hidden status row and no backing outside the intended visible region.

- [ ] Turn the Task 1 failure into a red test in the owning existing test module. Assert a visible status ink rectangle lies entirely within the native visible rect, or no status ink is visible; assert the exposed backing region is empty after collapse settles. Use an isolated desktop capture for these if the GTK compositor seam cannot be tested at the geometry layer. Do not substitute a DOM-only check for the backing defect.
- [ ] Make the smallest fix at the failing layer. If the request is undersized, fix the height reported by the page; if allocation or background differs from the agreed rect, fix the native host. Keep hit-testing and keyboard focus intact. Do not choose an implementation of either path without the Task 1 measurements.
- [ ] Re-run the failing test and the Linux VM scenario with status/error, menu dismissal, resize and model click. Run focused Assistant native tests and `cargo xtask ui-containers`. Record before/after screenshots and exact SHAs, and request separate visual review.

### Task 4: Return to the release candidate

**Files:** `docs/release-readiness-0.1.3.md` §15.1, `docs/releasing.md` (procedure), existing release/packaging scripts.

- [ ] Review the fix and run #221's required checks on the reviewed SHA. Keep the paused watchers paused until the pre-merge visual fix is accepted and integration is approved. Verify required merge commits are in `develop` before cutting the candidate.
- [ ] Dispatch nonpublishing `release.yml` using one candidate commit from merged `develop`; record SHA, run ID, package hashes, `build.json` and `contents.json`. Check aliases and both checksum filenames with `--require`.
- [ ] Install that run's exact Linux, Windows and macOS assets for separate acceptance. Check visual matrix and status/resize, core CAD, built-in agent/Skills, login, slash commands and updates. Mark genuine blockers or NOT TESTED explicitly, including unavailable real Mac hardware. Do not reuse earlier rehearsal results or publish from this plan.
