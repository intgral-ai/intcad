# DFM Review safe-stop bound: measurement record

Status: **measurement only**. This document records how long the Review (`dfm.check`) run path
takes on this machine. It does not decide or install a budget entry, it does not call
`set_verified`, and it changes no production behaviour. `BudgetTable::unverified_default()` in
`apps/solidcraft/src/account.rs` stays unverified after this record.

The question it answers: what is the wall time of the code a controlled
`(Capability::Review, EntryCategory::ReviewPanel)` run executes, and what dominates it?

## What was measured

The timed path is the one a controlled Review press uses:

1. `dfm::service::start_reserved` — project freeze, `document.snapshot.capture`, queue send.
2. The background worker's `run_once_reserved` — controlled submit and claim.
3. `LocalRunner::run` — candidate verification, workspace extraction, `input.integrity`,
   `native.rebuild` (open the captured design in an isolated session and evaluate the whole
   feature history), one `dfm.sheet` check per sheet body, the App's `dfm.sheet.flat_overlap`
   check, and the findings/report writes.

The timer starts immediately before `start_reserved` and stops when `dfm::service::status()`
reports that the worker is no longer running and the new run id is current. It therefore covers
everything between a Review press and a settled run except the paid admission, ledger and
settlement calls around it.

The four models are built through the normal commands:

| size | how it is built |
|---|---|
| `sample_bracket` | the bundled "Sheet Metal Bracket" sample script (`crates/engine/src/sample.rs`) |
| `small` | one 120 x 60 x 2 mm plate, eight Ø4 holes as `solid.hole` features, two 20 mm edge flanges |
| `large` | four 400 x 300 x 1 mm plates, one `sheet.perforate` field each (Ø5, 15 mm square pitch, 6 mm margin) and one 20 mm edge flange each |
| `rebuild` | one 200 x 120 x 2 mm plate with 32 Ø4 `solid.hole` features (B-rep cuts) |

Every project also gets `process = press brake`, `material = 5052-H32` and the repository's test
shop (`crates/engine/src/project/test_data/critical_shop.json`) so the full rule catalogue runs.

## Harness and command

Harness: `crates/engine/examples/dfm_budget_bench.rs` (a measurement example, like the other
`*_bench` examples in `crates/engine/examples/`; not a test and not part of CI).

```sh
cd worktrees/int1497-crt-check
CARGO_TARGET_DIR=$PWD/.targets/dfm-budget \
    cargo run --release -p solidcraft-engine --example dfm_budget_bench -- --repeats 5
```

Machine: Linux x86_64, 16 logical CPUs, AMD Ryzen 7 7800X3D (8 cores / 16 threads). This is a
**shared development host, not a CI runner**; other builds and agents on the host move the
numbers. Each size ran one warm-up pass and then five measured passes. The warm-up also opens the
validation port. `rebuild ms*` and `facts+overlap ms*` are one separate pass over the saved
design made after the timed runs, to attribute cost; they are not part of the median.

Measured at commit `11a3f345` (branch `task/int1497-billing-wiring`) on 2026-11-23. The worktree
also contained unrelated uncommitted edits in app/UI files from another task; the engine crate
used here is unmodified apart from this new example.

## Raw output

```text
# DFM Review run-path measurement

Machine: linux / x86_64, 16 logical CPUs, AMD Ryzen 7 7800X3D 8-Core Processor. This is a shared development host, not a CI runner; other work on the machine moves the numbers.

Each size: one warm-up pass, then 5 measured passes. Wall time is measured from `service::start_reserved` until the background worker stops running, so it covers project freeze, snapshot capture, candidate verification, the native rebuild, every `dfm.sheet` check, the flat-overlap check and the findings write. `rebuild ms*` and `facts+overlap ms*` are one separate out-of-band pass over the saved design, taken after the timed runs to attribute the cost; they are not part of the median.

| size | bodies | holes | flanges | features | design KiB | cold ms | median ms | worst ms | max incl. cold ms | rebuild ms* | facts+overlap ms* | runs (ms) | outcome |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| `sample_bracket` | 1 | 0 | 4 | 5 | 35.9 | 98.2 | 79.0 | 81.6 | 98.2 | 23.9 | 49.7 | 79.2, 77.7, 77.5, 79.0, 81.6 | current |
| `small` | 1 | 8 | 2 | 11 | 83.5 | 2215.4 | 2228.0 | 2267.9 | 2267.9 | 2014.0 | 213.5 | 2211.8, 2228.0, 2267.9, 2234.1, 2193.9 | current |
| `large` | 4 | 2080 | 4 | 16 | 27.6 | 168.5 | 209.2 | 245.9 | 245.9 | 18.8 | 67.1 | 209.2, 226.0, 245.9, 195.6, 164.6 | current |
| `rebuild` | 1 | 32 | 0 | 34 | 719.5 | 35365.6 | 33173.6 | 35910.9 | 35910.9 | 34414.2 | 394.9 | 35910.9, 33690.3, 33173.6, 31603.9, 29470.9 | current |
```

`holes` counts round cut-outs plus the holes a perforation field places on demand. `features` is
the number of evaluated feature results in the session.

### One load-stressed observation

An earlier pass of the same harness, same models, `--repeats 1`, ran while other work was
compiling on the host. The 32-hole `rebuild` model took **96 509.8 ms** (small: 2 929.7 ms,
large: 155.2 ms, sample: 82.7 ms). That is 2.7x the worst case in the recorded table above. It is
the reason the proposed multiplier below is not based on the quiet-host worst case alone.

## What the numbers say

1. **The native rebuild dominates.** For `rebuild`, the separate rebuild pass costs 34.4 s of a
   33-36 s run. For `small`, 2.0 s of 2.2 s. For `sample_bracket`, 0.024 s of 0.08 s. The DFM
   facts and flat-overlap checks are not the cost: 49.7 ms for the sample, 213.5 ms for the
   8-hole part, 67.1 ms for 4 bodies with 2 080 cosmetic perforation holes.

2. **Cost grows faster than the sheet/hole count.** Going from 8 to 32 `solid.hole` features
   (4x the features) takes the rebuild-only pass from 2.0 s to 34.4 s, about 17x, and the whole
   run from 2.23 s to 33.2 s, about 15x. That is consistent with a feature history that is
   re-evaluated with a B-rep boolean per hole, where each new cut also meets a longer, more
   complex body. It is the one term that grows without a visible cap in the path.

3. **Cosmetic perforation fields are cheap.** 2 080 holes across four bodies, produced by four
   features, add about 0.07 s of facts+overlap and no B-rep work. Hole count alone is a bad
   predictor of Review cost; feature history and its B-rep booleans are the predictor.

4. **Host load is a large factor.** The same 32-hole model measured 29.5-35.9 s on a quiet host
   and 96.5 s while the host was also compiling. A safe-stop window has to cover the loaded
   case, not the median.

5. **The safe stop is a deadline, not preemption.** `RunControl` is checked between checks
   (`validation/runner.rs`), so a rebuild that overruns is not interrupted in the middle. The
   bound must exceed the whole run; setting it below the real time does not shorten the work, it
   only makes the run report `TimedOut` after that work has already been paid for.

## Proposed bound for `(Capability::Review, EntryCategory::ReviewPanel)`

**600 s**, not wired anywhere by this work.

- 600 s is 6.2x the worst observed run (96.5 s, load-stressed) and 16.7x the worst run in the
  recorded five-pass table (35.9 s).
- Why that multiplier: the observed host-load spread for the same model is 2.7x (35.9 s to
  96.5 s). The 8-to-32 B-rep cut-feature data then shows roughly quadratic growth, so one
  doubling of the B-rep cut features is about 4x. 2.7 x 4 is about 11x, which would be over 1 000 s; 600 s is the
  engine's own execution ceiling (`validation::MAX_EXECUTION`), and a larger table value cannot
  extend a run past it.
- The bound is therefore safe for the measured envelope and for an extrapolated model with about
  64 B-rep cut features under load (about 386 s projected), with 1.55x headroom.
- It is **not** a mathematical bound on arbitrary designs. A 100-hole model projects to roughly
  940 s at the same load-stressed growth, which already exceeds 600 s. For that reason the value is only sound
  together with a documented input envelope (for example "at most 32 B-rep cut features; or at
  most 4 sheet bodies and 2 080 cosmetic holes") enforced before dispatch, or with a rebuild
  budget added to the run path. Neither exists today.
- A tighter 300 s (3.1x the load-stressed worst) would be defensible only if the 32-feature
  envelope were enforced; it does not cover a 64-feature model.

## What would make the bound unsafe

- **Feature history / B-rep booleans.** Unbounded in the path. This is the dominant term and
  the reason no small bound is safe for arbitrary user designs. The size metric that matters is
  the number of evaluated B-rep cut features and the growing face count they leave, not the
  number of holes the DFM facts report.
- **Other rebuild work not measured here.** Complex sketches, fillets and shell operations,
  imported/converted (STEP) bodies, component links and `world_state` display geometry
  evaluation are all inside `native.rebuild`. The measured models have simple plate bases.
- **Very large perforation fields.** `perforation_holes` enumerates every hole and the facts
  compute nearest-hole walls pairwise (O(H²) per body). 2 080 holes were cheap; 10⁴+ holes in one
  body were not measured and that term is not covered by the 600 s figure without evidence.
- **Many sheet bodies.** Each adds one `dfm.sheet` check. Four bodies cost 0.25 s total here;
  tens of bodies were not measured.
- **Machine and store.** A slower host, a cold file cache, a slower disk, a network home
  directory for the validation store, or antivirus-style scanning of the candidate and run
  files can all add time that this host does not show.
- **Concurrent app activity.** The 96.5 s observation shows 2.7x sensitivity to other local
  work; heavier contention is possible.
- **The run control is cooperative.** Cancellation and the deadline are checked between checks.
  A bound cannot be treated as a hard preemption guarantee.

## What was NOT measured

- The paid admission, renewal, ledger and settlement calls around the run (no account service
  was installed; this measures the engine path only).
- Opening the project, the window, the GUI thread and its frame cost.
- Windows, macOS, CI runners and end-user hardware; this is one Linux dev host.
- Models with more than 32 B-rep cut features, more than 4 sheet bodies, or more than 2 080
  perforation holes; converted/imported bodies; components and assemblies.
- Memory use, disk use, crash/kill behaviour, and the time to write very large reports.
- Anything about the correctness of DFM findings; every run here ended `current`, but rule
  outcomes are out of scope.
