# Admitting an agent's stoppable read on a copy

**Status: proposal, for review by the owners of the paid-authority chain (#240) and the packaged
Agent's route (#250).**

An agent's stress estimate is a read: it runs on a copy of the design and changes nothing in it.
Since #240 the desktop refuses it before it starts. This note proposes the smallest change that
lets it run, charges it once, and keeps it from running past its authorization. It opens no other
command.

## What happens today

1. `control_job.rs` `admit_copy_path` refuses a controlled entry before a copy answer starts:
   `UnsafeToStart`, "this controlled entry needs a durable commit path that is not available yet"
   (`control.rs` `async_controlled_refusal`). This covers `GuiBridgeMcp`, which #250's packaged
   Agent and the Mastra lane use, and `DirectControl`.
   - The FEA stack adds `inspect.stress_estimate` to `READ_ON_A_COPY`. Its dry run, which the chat
     runs without asking, and its solve, which runs after the user's Allow, are both refused.
   - This is the copy path only. `offload_control` hands a request back to the window thread, where
     it is admitted normally, in three cases: the app's off-thread wait is unset, a file is loading
     or being picked, or four copy answers are already running.
2. On #250's route, `host_bridge.rs` `raw_call` stamps `expectedDocument` on a call that names
   none. `execution_guard.rs` `check_step` then refuses the estimate as unsupported, because it is
   in no listed area. `crates/mcp/src/document_guard.rs` has the same list.
3. A paid call is admitted against a 280 s declared bound (`apps/solidcraft/src/account.rs`
   `desktop_budget`) inside one lease of at most 300 s. A result after the cutoff never commits.
   An estimate can run for many minutes; the stack's MCP bridge waits up to an hour for one.

Decided (FEA spec, decision 30): an agent's estimate counts toward credits, one unit per
successful logical operation, read-only included, as [the entry matrix](entry-matrix.md) says.
The user's own use by hand is free; the stack has no by-hand run today.

## Proposal

```
window thread                                         copy worker           bridge worker
offload_control
 └ admit_copy_path ─ stoppable read? ─ no ──> refused, as today
                    └ yes: admit_once, start_dispatch()   (handle's checks before dispatch;
                           renewal registered; watchdog started; work_started = true)
                           spawn ───────────────────────> copy.execute(...)
poll_control_jobs, each frame                                │
 └ cutoff - now < STOP_MARGIN, turn ended, Stop? ── stop() ─>│ stops, shows nothing new
   (watchdog thread: the same, when no frame is drawn)       │
answered(reply) <────────────────────────────────────────────┘
 └ finish_dispatch()       (handle's code after dispatch: the commit gate, evidence,
        then settle_result, or the disposition ──────────────────────────> settles once,
        for the bridge worker)                                               original id
```

### 1. Only reads the host can stop

A controlled entry may run a read on a copy only if the read is listed in a new table,
`STOPPABLE_READS` in `control_job.rs`. Each row names the command and two host functions:
- `stop` ends the run. The host calls it again until the run answers, so a stop that comes before
  the run can see it is not lost.
- `withdraw` takes away what a finished run shows. The host calls it only for a finished run whose
  reply it withholds, so an earlier result stays on screen.

The table is empty on `develop`. The FEA stack adds `inspect.stress_estimate`, with `fea::cancel`
and a call that clears the estimate's colour map or dry-run preview.

`develop`'s own copy reads (`inspect.measure`, `document.inspect`, clearance, interference,
properties), exports and snapshots stay refused on the copy path for controlled entries, as now.
They cannot be stopped once started, so they cannot meet the safe-stop requirement this design
relies on (question 4).

### 2. Carrying the grant through the copy

Everything below runs on the window thread, as `handle` does now. `handle` is split into two
halves, with no change in behaviour. `admit_copy_path` and `answered` call the same two halves.

- **Before the worker starts.** `admit_copy_path` takes the admission with `admit_once`, as
  `run_control` does before `handle`. The admission is the one the bridge worker stored, or a new
  one for `DirectControl`. Then `start_dispatch` runs `handle`'s checks:
  - the entry, async-native, multi-step and capture refusals, and a turn that has ended;
  - `task_runtime::before_tool`;
  - `authority_refusal` with the start budget (remaining authorization at least the entry's
    declared bound, unchanged at 280 s).

  Then `work_started` is set and the worker is spawned. A refusal before that point releases the
  admission, as today. A read that cannot start on a copy after admission (a bad folder scope, or
  no threads) is answered and settled at once as having no effect.
- **While it runs.** The job keeps the request: its admission, scope, operation and `bridge_effect`.
- **When it answers.** `finish_dispatch` runs `handle`'s code after dispatch:
  - `commit_decision`, which rechecks the live scope, status, version and cutoff;
  - `control_finished`;
  - the evidence;
  - the bridge worker's disposition, or `settle_result`;
  - `task_runtime::after_tool`.
- **Every way a started read ends settles once.** These are an answer, a chat turn that ended,
  the Stop button, and a document that changed while it read. A turn that ends no longer drops
  the job: the read is stopped and settles when it answers.

### 3. Runs longer than one lease

- **a. Host stop, always.** Each frame while a controlled read runs, the window reads the live
  cutoff (`authorization_bound`, including a renewal) on the host clock.
  - When less than `STOP_MARGIN` is left, it calls the row's `stop`, and calls it again each frame
    until the read answers.
  - A watchdog thread per read does the same from the last cutoff the window saw, because a
    minimised window draws no frames. Its clock does not count a suspend. After a resume the
    window's own check, on the host clock, covers that case.
  - The reply is withheld, and the agent is told the run was stopped because its authorization
    was about to end. The operation is released (section 5).

  `STOP_MARGIN` is proposed at 30 s. It is set from a measurement of the time from `stop` to the
  window's answer, taken while meshing and while solving, recorded beside the
  [desktop bounds](control-safe-stop-bound.md). A first measurement on the FEA stack: one
  agent's solve over the bridge (a 100 x 20 x 1.5 mm sheet at a 0.4 mm mesh, Linux x86-64)
  answered 0.06 s after the host stopped it while meshing, and 0.04 s after while solving.
- **b. Lease renewal.** A stoppable copy read registers its operation with the existing renewal
  job (`AccountPort::control_admitted`), and `authorization_bound` uses the renewed cutoff for it.
  - `DirectControl` already does both, on a fresh admission.
  - For `GuiBridgeMcp` the copy path registers the read itself, since it arrives already admitted.
  - `OperationRenewal` (`crates/account-native/src/http.rs`) must also accept an `mcp` grant and
    renew it under that capability. Today it accepts `control` only.
  - The grant must carry a renewal schedule, which it does only when the server sends
    `renew_after_seconds`.
  - The server must renew `mcp` authorizations (question 1).

Without (b), a read on the bridge gets one lease, at least 250 s of running, before (a) stops it.
The stack's forecast can then refuse a run predicted to take longer.

The start bound stays the entry's 280 s. A read's first phase evaluates the copy before any
program starts, which is the "one comparable rebuild" the envelope covers. From the first program
on, the read stops within the measured stop time.

### 4. The packaged route's guard

- `execution_guard::check_step` also accepts a command in `STOPPABLE_READS`, looked up in the
  table, not by name. A read that names a file is still refused.
- A call that #250's `raw_call` stamps therefore passes the guard. Its answer is still checked
  against the live document in `answered` (`check_application`) and is stamped there.
- `solidcraft-mcp`'s `document_guard` cannot see the table, so the stack adds the estimate to that
  list itself.

### 5. Evidence

`result_evidence` is unchanged, with one addition, which applies only while the commit gate is
still open. When the host withholds a stoppable read's reply, the evidence is no-effect.
Withholding happens when the host stopped the read (3a), when its turn ended, when Stop was
pressed, or when the document is stale at answer. The read ran on a copy that is dropped. A
stopped run shows nothing new, and `withdraw` takes away what a finished one shows.

The other outcomes:

- A delivered `ok: true` reply is an effect: one unit.
- A read that fails on its own (`ok: false`, with no stop from the host) stays unknown, as every
  started controlled operation does now (question 3).
- Once the gate is closed (cutoff passed, scope switched, status changed), the copy path decides
  nothing. The result takes the existing late path: it is discarded, and its settlement is
  deferred under the original operation. It is never released on the expired path, and never
  settled through a new account.

## What does not change

- Free GUI and browser copy answers.
- `develop`'s copy reads, exports and snapshots for controlled entries.
- `handle`'s behaviour.
- The 280 s declared bounds, and the one-lease admission rule.
- The ledger states.
- The late-result path.

What does change:
- The bridge worker's admission is now carried to a commit on the copy path, where before it was
  only refused.
- The window's renewal job may now renew a lease the bridge worker took.

## Known limits

- **One bridge call at a time.** A bridge read holds the bridge's admission slot until it settles,
  so the agent's other paid calls are refused meanwhile with "a bridge admission is already in
  progress". Free helpers such as `running` and `pending` still pass.
- **One renewal slot.** A second long operation runs without renewal and is stopped at the end of
  its lease.
- **Shared estimate state.** `fea::cancel` stops every running estimate, and the estimate keeps
  one shown result. Two estimates at once, possible over the control port, stop together.

## Questions for the owners

1. Does the server renew an `mcp` authorization, and may the window's renewal job renew a lease
   the bridge worker took? **Recommended: yes.** If not, ship 3a alone; runs then stop after one
   lease.
2. Is a withheld stoppable read no-effect while the gate is open, so that it is released?
   **Recommended: yes.** The alternative is unknown, which leaves every stopped estimate in
   recovery.
3. Does a read that fails on its own stay unknown? **Recommended: yes in this PR**, as for every
   started controlled operation. Releasing it is a separate decision.
4. Should `develop`'s own copy reads (`inspect.measure`, `document.inspect`, and others) be opened to
   agents on the copy path? They cannot be stopped. **Recommended: a separate PR**, which would
   admit them under the 280 s declaration as synchronous commands are admitted.
5. `STOP_MARGIN` is 30 s until the stop time is measured. **Recommended:** at least ten times the
   worst measured stop.
6. Should the one renewal slot stay as it is? **Recommended: keep it for now.**

For the product owner:

7. Does an agent's dry run (sizing only, no program started) count? **The default is yes**, as the
   matrix says for read-only queries. Making it free would mean a published free helper that
   matches that one exact request, and it needs the owners' review.

## Tests

| Case | Input and action | Pass | Real or faked |
| --- | --- | --- | --- |
| Charged once (`DirectControl`) | A stoppable read, held on its worker, then answered | One commit, `Succeeded`; no release | Real control path; fake billing and clock |
| Charged once (bridge) | The same through `queue_bridge_admission` | The worker finalizes `Succeeded` once | Real bridge worker; fake account and journal |
| Stopped near the cutoff | Clock at 271 s of a 300 s grant while the read computes | `stop` each frame; reply withheld; finished run withdrawn; released once | Fake clock |
| Stopped near the cutoff (bridge) | The same on the bridge | The worker finalizes `FailedNoEffect` once | As above |
| Late answer | The read answers at 301 s | Not charged or released; settlement deferred | Fake clock |
| Stop pressed | `stop_agent_command` mid-run | Stopped reply; released once | Real |
| Turn ended | The chat turn ends mid-run | Stopped; one reply when it answers; released once | Real |
| Renewed | Renewal to 600 s; answer at 400 s | Not stopped; charged once; renewal retired | Fake account port |
| Nothing else opened | A measure, a snapshot and an export on a controlled entry | Still `UnsafeToStart`; nothing computed | Real |
| Guard | Stamped stoppable read, unlisted read, stoppable read naming a file | First passes; the other two are refused | Real |
| `mcp` renewal | A renewal driver for an `mcp` grant, and one for `chat` | `mcp` renews under its own capability; `chat` gets no driver | Scripted transport |
| Estimate (FEA stack) | Agent's dry run, then its solve, over the bridge | Each admitted and charged once; a solve held past the margin is stopped and released | Real command; fake account |
