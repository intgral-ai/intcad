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

- [x] Add deterministic report/state-gap, pending pass plus marker, incompatible report and actual-state regressions.
- [x] Add native lock contention/child crash, writer-order, claim/cancel and retention fixtures in existing tests.
- [x] Add Stop/shared-reset retirement, late registration, saturation/start/shutdown control tests; no sleeps as race-order proof.
- [x] Extend existing CLI cancellation test with verified terminal report and stable repeat-cancel digest.
- [x] Commit test-only checkpoint; push only task branch; observe revision-bound behavioral RED in the authorized diagnostic lane.

Expected RED: pending Cancelled report currently produces REPORT_MISMATCH, and pending Passed report plus marker cannot be cancelled consistently.

### Task 2: Native validation transaction

**Files:** `crates/engine/src/validation/{mod.rs,store.rs,tests.rs}`.
**Interfaces:** `Store::transaction() -> Result<TransactionGuard>`; locked helpers require `&TransactionGuard`; commit returns actual `ValidationState`.

- [x] Add independently opened native lock guard; unsupported/error fails closed.
- [x] Move all persistence mutation decisions under the guard, including retention eligibility/deletion and state repair.
- [x] Make verified-report recovery compatible with current terminal state; reject unverified/foreign reports.
- [x] Remove live report remove/recreate; choose marker before publishing; propagate actual state.
- [x] Verify deterministic native tests in remote CI; inspect source diff and lightweight formatting.
- [x] Commit recoverable engine checkpoint.

### Task 3: Background cancellation control

**Files:** `crates/ui-egui/src/agent_chat/mastra_backend.rs`, new native helper beside it if needed, existing `tests.rs`.
**Interfaces:** memory-only `cancel_turn` retires a binding and enqueues work; one control worker owns disk cancellation independently of runner pump.

- [x] Add bounded coalescing cancellation queue with explicit pending/completed/failed states and original binding.
- [x] Check retirement before disk admission and again at registration; cancelled registration queues its eventual run without restoring authority.
- [x] Process cancellation outside book/queue locks; bounded deadline and shutdown interrupt lock acquisition.
- [x] Keep wire cancellation authorized and off-thread, returning actual persisted result.
- [x] Keep UI event state truthful: queued cancellation is historical/cancelled presentation, not persisted Cancelled.
- [x] Verify remote UI/engine/CLI suite and commit.

### Task 4: Evidence and delivery

- [x] Document upgrade/rollback coordination, stable lock lifetime, failures and resource limits.
- [ ] Dispatch the full GREEN `ci.yml` on the final task revision; the matched diagnostic lane has passed but is not the required full gate.
- [x] Record tests NOT_RUN or BLOCKED rather than infer passing.
- [ ] Hand off branch/diff/ledger to controller for independent review; no integration or merge.

### Task 5: Authoritative tombstone repair (private, no new public surface)

**Files:** existing `crates/engine/src/validation/mod.rs`, `store.rs`, `tests.rs`.

**Interfaces:** private `ValidationPort::retirement(run_id, &TransactionGuard) -> Result<Option<TombstoneRecord>>`; private replay helper that returns the same run/candidate/profile/state for an unexpired key. No public API, record, wire, auth or schema change.

- [ ] Add deterministic tests first: tombstone blocks both replay routes and status/report/evidence/cancel/cancel_until/complete/start_next/counts/reconcile; expiry ends replay but not selection; invalid/unreadable tombstone is not absence; conflicting/unreadable GC is monotone; no tombstone keeps crash-after-key and verified-report-before-marker.
- [ ] Run the bounded RED lane and observe the new tests fail for the missing authority.
- [ ] Validate tombstone path/run id/schema/run/key binding/terminal state/expiry under the caller's existing TransactionGuard; invalid/unreadable is not absence.
- [ ] Consult one retirement decision before every recovery write, replay, status/report/evidence, both cancel entries, counts/selection, reconcile, and completion before evidence publication. Retired non-replay paths return ordinary post-deletion NotFound and do not mutate.
- [ ] Keep replay semantics: owner/payload checks first; same run/candidate/profile/state for an unexpired key; REQUEST_EXPIRED at expiry; no inference for legacy both-records-absent.
- [ ] Make GC monotone: never overwrite a conflicting or unreadable tombstone; keep tombstones while admission/state residue exists; expiry only ends replay; exclude retired residues from live counts, not the physical record cap; improve the no-key removal ordering without using it as a tombstone substitute.
- [ ] Run the bounded GREEN lane, then remove the temporary diagnostic workflow in a separate commit and record the tested/final net-diff equality, artifact and receipts.

## Execution ledger

- Base `c5f2f42fe6f06ed5c2c4ec1df04f7e9c6aae50ad`; sparse worktree initially 2,359,296 bytes.
- Full tracked checkout estimate 676,052,072 bytes; sparse checkout chosen under significant-growth gate.
- Host free space at setup 632,470,499,328 bytes. Baseline NOT_RUN: local compilation prohibited.
- Native git `worktree add --no-checkout` used as explicitly directed; OpenCode worktree helper cannot request sparse/no-checkout creation.
- Controller owns independent reviews and CI log extraction. No delegation or self-approval.
- Test-first checkpoint `3c91659` pushed; RED CI `44575356172` pending at that revision.
- Additional test-first coverage added for child-process terminal writers, invalid pending reports, late post-Stop submit and stable CLI report identity. Rust execution remains NOT_RUN pending remote CI.
- Superseding targeted behavioral RED: run `44589009058`, source `7f5bdde70`, engine test blob `14eddc3dde0e6a346e42dbad8bb3a2a666a07d83`, UI blob `407e5fd103ab4ce968dee78c2ca7859ac10e8feb`. Three engine assertions and the retired-turn UI assertion failed with exactly one selected outer test each; binding/evidence rejection already passed.
- Raw RED: `/tmp/opencode/int1454-valtx-targeted-red/{source.txt,commands.json,command-0.stderr,command-1.stderr,command-2.stderr,command-4.stderr}`. Toolchain is the artifact's `rustc.txt`/`cargo.txt`; package defaults, no extra features.
- Full run `44575356172` did not establish behavioral RED; test execution is unknown. No timing inference is used.
- Production checkpoint implements native persistence serialization and independent cancellation control; additional process barriers/crash points/queue/late-registration fixtures added. GREEN and independent review remain pending.
- RED source tracked files were unchanged; raw dirty metadata includes only the generated `evidence/` directory. GREEN records dirtiness before generating evidence.
- Ruling: 64 outstanding control requests matches bounded run history; 5 s lock-admission deadline follows contract §4. A 4,096-entry retirement ledger closes admission on exhaustion rather than evicting a possible in-flight binding.
- Verification pending: native Windows/macOS lock behavior, full required CI and GUI evidence. No local compilation or new target; direct Rust 1.99.1 rustfmt parsing and `git diff --check` only.
- Matched diagnostic run `44682474490` at `76167a5d7` passed engine 66/0, UI 11/0 plus retired card 1/0, CLI 1/0, Clippy 0 and fmt 0. Later scoped-review run `44696272611` at `b618f964` passed engine 70/0, UI 12/0, retired card 1/0, CLI 1/0, Clippy 0 and fmt 0. The temporary diagnostic workflow is not part of the delivered source diff; its runs and artifacts remain review provenance.
- Review follow-up: cleanup now keeps terminal records when state or admission is unreadable, and writes a tombstone from the surviving key record before retiring a state whose admission is already gone. A key-directory listing or entry read failure is also an error, never proof of absence. A same-key replay therefore returns the durable terminal outcome rather than recreating Queued.
- Legacy limitation (documented, no migration semantics): if a pre-tombstone store already removed both admission and state, no record remains for cleanup to inspect; a same-key replay is indistinguishable from the documented crash-after-key recovery. The key is retained until its normal expiry.
- Review follow-up: wire `validation.status`, wire `validation.cancel` and the control worker share one monotone state merge; a delayed Queued/Running snapshot cannot overwrite a settled terminal state. The regression now parks a real `validation.status` publication with a barrier while the real control lane persists cancellation, rather than only testing the merge helper. Wire calls remain on the `mastra-host-call` thread, `cancel_turn` remains memory-only plus the control queue, and no storage wait was added to the window thread.
- The final focused run and artifact, tested-head/final-head receipts, prior named failures and full net diff are recorded in the ignored review package; full required CI, independent review and GUI evidence remain outstanding.
