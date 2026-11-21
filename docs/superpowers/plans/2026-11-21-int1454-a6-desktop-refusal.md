# INT-1454 A6 Desktop Refusal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Track steps with checkboxes.

**Goal:** Let the desktop deliver its existing connection-limit refusal without holding up admissions.

**Architecture:** Keep the desktop server, its two listener entry points and their shared admission counter. Use one fixed disposal worker per listener, with bounded, nonblocking admission to that worker. Only disposal handles refused connections; normal authenticated dispatch is unchanged.

**Tech Stack:** Rust 1.99.1, desktop Rust test module, existing CI.

**Spec:** INT-1454 SEC-3/A6 and CD-3/CD-4. This is refusal-delivery parity, not A2 shared-server adoption.

## Global Constraints

- Preserve loopback, token-first admission, wire response, default-on preference, shared 16-connection cap, and both existing endpoints. No new commands, permissions, defaults, or dependencies.
- Refusal handling may not run or parse commands. Bound the queued work, byte count, and lifetime. One absolute deadline covers write and cleanup; it starts before handoff and retries cannot extend it. On overload or disposal failure, fail closed without waiting on the listener.
- Tests use bounded synthetic in-process I/O and synchronization. Do not reproduce a live vulnerability, generate traffic floods, or add offensive tooling.
- No direct `develop` push, PR #185 update, skip, assertion weakening, or exception changes. Check disk and applicable budget before heavy work. Keep evidence; only reclaim unused regenerable artifacts owned by this task.

## Review Focus

- None of the refused connections is dispatched or counted as a served slot after rejection.
- Deadline still holds after queuing, partial operations and I/O failure.
- Full or disconnected handoff does not block acceptance or create additional workers.
- Existing successful admission and unauthorized handling are unchanged.
- An orderly close is expected on the normal refusal path, not promised after overload or I/O failure.

### Task 1: Harden desktop limit-refusal delivery

**Files:** `apps/solidcraft/src/control_server.rs`, existing test module in the same file; `crates/engine/src/control_auth.rs` only if needed to reuse the existing shared refusal constant.

- [ ] Add an in-process behavioral RED at the disposal/admission seam with scripted bounded I/O: exact existing reply and write-side close before cleanup, no dispatch, and immediate release of the served-slot claim. Independently pin deadline/byte limit, I/O failure, and nonblocking full handoff using synthetic fixtures. Record exact source, selected tests and exits.
- [ ] Implement the smallest refusal-only change. Keep one worker per listener and a one-item handoff, one 500 ms absolute budget and the existing `MAX_LINE + 1` byte ceiling. Reuse existing refusal I/O where practical without altering unauthorized behavior. Stop if preserving the contract requires an A2 migration or a new public behavior.
- [ ] Run the new and existing relevant desktop tests, targeted Clippy and format. Prefer remote CI if compiling the app exceeds local budget. Use ordinary checks; do not add an unreviewed workflow or claim unexecuted tests passed.
- [ ] Commit and push the owned task branch checkpoint, preserve command output, and request independent Spec and Quality review before integration. Keep A2 residuals and final candidate SHA evidence separate.
