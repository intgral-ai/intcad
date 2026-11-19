# Validation terminal transaction implementation plan

> **For agentic workers:** Use superpowers:executing-plans inline. No delegation. Steps use checkbox tracking.

**Goal:** Serialize validation persistence across native processes without blocking Stop/reset or changing authorization or terminal outcomes.

**Architecture:** A stable store-level native file lock guards all persistence decisions; guard-required helpers prevent nested acquisition. A separate bounded coalescing cancellation control worker handles storage after the UI synchronously retires turn authority. Verified pending reports retain existing recovery precedence.

**Tech Stack:** Rust 1.99.1, standard-library file locking, mutex/condition-variable control queue, existing atomic store writes.

**Spec:** Owner-approved controller brief, recorded in `docs/upstream-sync/int1454/ledger.md`, terminal transaction section. Existing contract: `docs/mastra-runtime.md` §§4, 9.2–9.3.

## Global constraints

- No permission, wire schema, design-format, five-terminal-state or recovery-precedence changes.
- Stable lock outside retained runs; never replace/unlink/clone its handle or steal by PID.
- Lock order: admission mutex if needed, store transaction, candidate pins.
- Cover submit/resume, repair, cancel, complete, start, reconciliation and retention; never hold across runner execution.
- Existing terminal state wins; otherwise a fully verified existing report wins before the marker.
- Report publication is once-only; return actual persisted state; deleted runs cannot be resurrected.
- Stop/reset revoke authority before storage waits. Cancellation uses one independent bounded coalescing worker, never one thread per click.
- Explicit cancellation queue/start/deadline/shutdown failures; late registration inherits cancellation.
- No local Rust build, new target, reflink, VM or large copy. RED/GREEN execution is remote CI only.
- Mixed old/new concurrent writers unsupported: stop all participants on upgrade/rollback, retain lock and records.

## Review focus

- Long-held external store lock: UI retirement and late-write refusal remain immediate.
- Crash after report but before state: verified report wins, including legacy report plus marker.
- Retention and late completion: no deleted-run resurrection.
- Shutdown or saturated cancellation queue: no false claim that cancellation reached disk.
- Delayed submission registration: cancellation remains bound to its original session/turn/run.

### Task 1: Pin the regressions and observe RED remotely

**Files:** existing `crates/engine/src/validation/tests.rs`, `crates/ui-egui/src/agent_chat/tests.rs`, `apps/solidcraft-cli/tests/script_roundtrip.rs`.
**Interfaces:** existing public port/host operations; persisted-state fixtures use existing record shapes.

- [ ] Add deterministic report/state-gap, pending pass plus marker, incompatible report and actual-state regressions.
- [ ] Add native lock contention/child crash, writer-order, claim/cancel and retention fixtures in existing tests.
- [ ] Add Stop/reset, late registration, saturation/start/shutdown control tests; no sleeps as race-order proof.
- [ ] Extend existing CLI cancellation test with verified terminal report and stable repeat-cancel digest.
- [ ] Commit test-only checkpoint; push only task branch; dispatch CI `affected`; observe revision-bound RED.

Expected RED: pending Cancelled report currently produces REPORT_MISMATCH, and pending Passed report plus marker cannot be cancelled consistently.

### Task 2: Native validation transaction

**Files:** `crates/engine/src/validation/{mod.rs,store.rs,tests.rs}`.
**Interfaces:** `Store::transaction() -> Result<TransactionGuard>`; locked helpers require `&TransactionGuard`; commit returns actual `ValidationState`.

- [ ] Add independently opened native lock guard; unsupported/error fails closed.
- [ ] Move all persistence mutation decisions under the guard, including retention eligibility/deletion and state repair.
- [ ] Make verified-report recovery compatible with current terminal state; reject unverified/foreign reports.
- [ ] Remove live report remove/recreate; choose marker before publishing; propagate actual state.
- [ ] Verify deterministic native tests in remote CI; inspect source diff and lightweight formatting.
- [ ] Commit recoverable engine checkpoint.

### Task 3: Background cancellation control

**Files:** `crates/ui-egui/src/agent_chat/mastra_backend.rs`, new native helper beside it if needed, existing `tests.rs`.
**Interfaces:** memory-only `cancel_turn` retires a binding and enqueues work; one control worker owns disk cancellation independently of runner pump.

- [ ] Add bounded coalescing cancellation queue with explicit pending/completed/failed states and original binding.
- [ ] Register in-flight submissions before disk admission; cancelled registration queues its eventual run without restoring authority.
- [ ] Process cancellation outside book/queue locks; bounded deadline and shutdown interrupt lock acquisition.
- [ ] Keep wire cancellation authorized and off-thread, returning actual persisted result.
- [ ] Keep UI event state truthful: queued cancellation is historical/cancelled presentation, not persisted Cancelled.
- [ ] Verify remote UI/engine/CLI suite and commit.

### Task 4: Evidence and delivery

- [ ] Document upgrade/rollback coordination, stable lock lifetime, failures and resource limits.
- [ ] Dispatch GREEN CI on final task revision; retain exact RED/GREEN commands, runs and raw evidence paths.
- [ ] Record tests NOT_RUN or BLOCKED rather than infer passing.
- [ ] Hand off branch/diff/ledger to controller for independent review; no integration or merge.

## Execution ledger

- Base `c5f2f42fe6f06ed5c2c4ec1df04f7e9c6aae50ad`; sparse worktree initially 2,359,296 bytes.
- Full tracked checkout estimate 676,052,072 bytes; sparse checkout chosen under significant-growth gate.
- Host free space at setup 632,470,499,328 bytes. Baseline NOT_RUN: local compilation prohibited.
- Native git `worktree add --no-checkout` used as explicitly directed; OpenCode worktree helper cannot request sparse/no-checkout creation.
- Controller owns independent reviews and CI log extraction. No delegation or self-approval.
