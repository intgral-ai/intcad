# INT-1497 Desktop Boundary Follow-up Implementation Plan

> **For agentic workers:** Use subagent-driven-development and test-driven-development.

**Goal:** Close the two load-bearing findings remaining at `f4861147e`.

**Architecture:** Derive browser opening from the specific trusted control activation, not a frame-wide input flag. Require a usable secure store at the desktop account port's write boundary.

**Tech stack:** Rust, egui and the existing account port and native credential adapter.

**Spec:** INT-1497 §3; final scoped review of `e1b3ebc0e..f4861147e`.

## Constraints

Keep the frozen contract and installation authority unchanged. No live authentication, push, merge or deployment. Keep ordinary CAD and noninteractive entry behavior unchanged. Do not claim that platform input provenance proves a physical human against OS-level automation; the boundary covers Intcad-injected input.

### Task 1: Close both desktop boundaries

- [ ] Add focused regressions in the existing UI account and desktop account suites. Record RED.
- [ ] In `crates/ui-egui/src/lib.rs`, `account/panel.rs` and the input routing as needed, bind browser dispatch to the account button's trusted gesture, accounting for buffered input, mixed input and cross-frame gesture state. Fail closed when provenance is ambiguous. Preserve an accessible explicit trusted activation path.
- [ ] In `apps/solidcraft/src/account.rs`, reject sign-out and switch before mutation or worker spawn when the secure-store probe failed, even if the logout marker was readable.
- [ ] Run focused test suites and diff/pin checks, record GREEN, commit locally and request independent review. Preserve the test logs and active target.
