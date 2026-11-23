# Controlled entry safe-stop measurements

These are measurements of local execution and protocol work, not release bounds, a policy
decision, or a change to the budget table. The paid entry points are not opened for these
measurements. The [Review Panel record](dfm-review-safe-stop-bound.md) is existing evidence;
the panel was not rerun. The two external wire-only categories are outside this record.

## Reproduce

Use the repository's native dev shell. The runs use one shared Linux development host, not a
CI runner: Linux `7.3.5-cachyos` x86-64, `nproc` = 16, AMD Ryzen 7 7800X3D (8 cores, SMT),
Cargo/Rust 1.98.1 from the dev shell on 2026-11-23. The starting source revision was
`7d926db8b247ceb042af5afbba7aa2b6366fbd1d` on `task/int1497-billing-wiring`, with unrelated
account composition work in the worktree. `df -B1 .` reported 653,946,396,672 bytes available
before starting. No GUI, VM, real provider, sign-in, or network service is used. The GUI bridge
test uses a local Unix socket pair.

Portable Cargo commands:

```sh
cd /home/hyjing/Code/intgral/intcad/worktrees/int1497-crt-check
export CARGO_TARGET_DIR="$PWD/target"
df -B1 .
nproc
lscpu | grep 'Model name'
state=$(mktemp -d)
export XDG_DATA_HOME="$state/data" XDG_CONFIG_HOME="$state/config"
export XDG_CACHE_HOME="$state/cache" XDG_STATE_HOME="$state/state"

nice cargo run --locked --offline --profile ci -j2 -p solidcraft-engine --example cli_dfm_stop
nice cargo run --locked --offline --profile ci -j2 -p solidcraft-mcp --example headless_mcp_stop
nice cargo run --locked --offline --profile ci -j2 -p solidcraft-ui-egui --example direct_control_stop
nice cargo test --locked --offline --profile ci -j2 -p solidcraft-ui-egui --test gui_bridge_mcp_stop \
    gui_bridge_mcp_stop -- --ignored --exact --nocapture
nice cargo run --locked --offline --profile ci -j2 -p solidcraft-ui-egui --example mastra_cancel_stop
```

The tables below use cached libraries from this worktree's `ci` target. This links only
the new harnesses, not a clean rebuild of the desktop composition root. Commands for
the recorded binaries (`zsh`, same directory, shell and XDG setup):

```zsh
deps="$CARGO_TARGET_DIR/ci/deps"
out="$CARGO_TARGET_DIR/ci/examples"
args=(-L "dependency=$deps" --edition=2024 -C opt-level=2 -C overflow-checks=on)
for lib in solidcraft_engine-1532daf1bd1099ab solidcraft_mcp-821b8ae6b08eb68c \
    solidcraft_ui_egui-f5a51d2808701af9 solidcraft_agent_task-b8669196d02673e7 \
    solidcraft_agent_task_engine-a157661a30811524 serde_json-0435582d3f772579 \
    egui-74b920a426429b10 nix-ab6927a7e2d66ec4 getrandom-9a5bdc12b0044bd6; do
    args+=(--extern "${lib%-*}=$deps/lib$lib.rlib")
done
for source in crates/mcp/examples/headless_mcp_stop.rs \
    crates/ui-egui/examples/direct_control_stop.rs \
    crates/ui-egui/examples/mastra_cancel_stop.rs \
    crates/ui-egui/tests/gui_bridge_mcp_stop.rs crates/engine/examples/cli_dfm_stop.rs; do
    flags=()
    [[ $source = */tests/* ]] && flags+=(--test)
    nice rustc "${args[@]}" "${flags[@]}" "$source" -o "$out/cached-${${source:t}%.rs}"
done
"$out/cached-headless_mcp_stop"
"$out/cached-direct_control_stop"
"$out/cached-gui_bridge_mcp_stop" gui_bridge_mcp_stop --ignored --exact --nocapture
"$out/cached-mastra_cancel_stop"
"$out/cached-cli_dfm_stop"
```

The CAD fixture is one 200 × 120 × 2 mm sheet with 0, 8, or 32 separate 4 mm `solid.hole`
features. The 32-cut fixture is the same geometry and command sequence as Review's `rebuild`
fixture; 8 and 0 are prefixes. The label is the number of B-rep cuts, not the total number of
engine features. Fixture construction is untimed. Each row has a first pass (“cold”, not an
OS cache flush) and five warm passes. Where there is an “outside” row, it reports paired
differences computed on each pass, not a difference of medians.

The Review record reports 33.2 s median, 35.9 s worst quiet, and 96.5 s in one heavy-load run
for the 32-cut shape. Build profiles and host loads can differ. Use those observations for
scale and attribution, not as upper limits. A request on an already evaluated document does
not incur a second full rebuild.

The figures below are not pair-wide bounds. The stop window can prevent a new step only if
the entry checks it in time. Neither the duration printed by a reply waiter nor the
engine's validation deadline proves that a synchronous step has stopped.

## DFM / `CliDfm`

Harness: [`cli_dfm_stop.rs`](../crates/engine/examples/cli_dfm_stop.rs).
Starts with an already captured candidate; times port open, profile registration, controlled
submit, exact claim, `LocalRunner`, durable completion, status read and JSON encoding. The
runner wraps the same `RunControl` used by `intcad-cli validate submit`; “outside LocalRunner”
includes report persistence. No CLI subprocess, request/profile file parsing, billing authority,
or settlement is timed. `validate skill` refuses mixed DFM profiles before work.

`RunControl` checks for cancellation and expiry between checks. A rebuild already in progress
will finish before the runner sees expiry, even at the engine's 600 s execution ceiling.
`review_stop_at_ms` illustrates clamping by remaining authorization; the measured window
cannot substitute for enforcement of the earlier stop. This particular CLI source drops the
grant and passes its own `stop_at_ms` to the port, which then clamps against the engine ceiling.

Envelope: one packaged candidate, no external dependencies, one sheet, the above plate and
cut pattern through 32 cuts, and one optional `dfm.sheet` check with an empty requirement
list and the mandatory native checks.
Unbounded B-rep history, more/expensive requirements, hostile geometry, linked/complex models,
and slow storage can exceed a candidate bound. No finite release-safe figure for general CLI
profiles follows from this fixture.

| Cut features / phase | Cold ms | Median ms | Worst warm ms | Warm passes (ms) |
|---|---:|---:|---:|---|
| 0 / total | 3.346 | 2.804 | 2.977 | [2.804, 2.635, 2.977, 2.819, 2.681] |
| 0 / outside LocalRunner | 1.356 | 1.236 | 1.282 | [1.282, 1.106, 1.255, 1.236, 1.139] |
| 8 / total | 2392.921 | 2506.922 | 2550.955 | [2358.476, 2476.643, 2521.982, 2506.922, 2550.955] |
| 8 / outside LocalRunner | 1.413 | 1.293 | 1.414 | [1.264, 1.244, 1.293, 1.414, 1.373] |
| 32 / total | 37254.858 | 25398.540 | 31896.643 | [31896.643, 29385.351, 25091.188, 24474.913, 25398.540] |
| 32 / outside LocalRunner | 1.388 | 1.199 | 1.407 | [1.407, 1.343, 1.199, 1.017, 1.128] |

**Proposed envelope-only target: 300 s**, 8.05× the 37.255 s maximum observed
here. The non-runner portion never exceeded 1.414 ms. Use the existing 96.5 s
heavy-load Review observation of this same rebuild to account for CPU contention,
leaving 3.11× headroom over that observation. This does not authorize a longer
history, guarantee a stop on arbitrary load, or include settlement.

## Control / `DirectControl`

Harness: [`direct_control_stop.rs`](../crates/ui-egui/examples/direct_control_stop.rs).
Times a real Direct Control request rejected before dispatch, then a companion request via the
free local entry that uses the same `control::handle` command dispatch and reply channel.
The command is one read-only `engine.execute` → `inspect.measure {names_only:true}` on an
evaluated session. The engine-only row uses the same command without control handling.
No actual TCP/token/framing work, signed-in admission/settlement, queue backlog, rendering,
permission wait, or agent cursor animation is included.

The TCP server's 60 s short wait and 3,600 s patient/batch wait bound when the caller is told
that work is still running; neither cancels the work. Envelope: one synchronous read of one
resident evaluated body with a small response. Unbounded history when a command forces a
rebuild, synchronous kernel operations, or a window/queue that cannot respond invalidate
an extrapolation to arbitrary control methods. A late commit refusal is not proof that no
local command completed.

| Cut features / phase | Cold ms | Median ms | Worst warm ms | Warm passes (ms) |
|---|---:|---:|---:|---|
| 0 / direct entry (refused) | 0.026 | 0.001 | 0.002 | [0.002, 0.001, 0.001, 0.001, 0.001] |
| 0 / shared control dispatch | 5.144 | 0.011 | 0.025 | [0.025, 0.013, 0.011, 0.011, 0.011] |
| 0 / direct engine only | 0.003 | 0.003 | 0.003 | [0.003, 0.003, 0.003, 0.003, 0.003] |
| 8 / direct entry (refused) | 0.015 | 0.001 | 0.002 | [0.002, 0.001, 0.001, 0.001, 0.001] |
| 8 / shared control dispatch | 0.064 | 0.011 | 0.013 | [0.013, 0.012, 0.011, 0.011, 0.011] |
| 8 / direct engine only | 0.003 | 0.002 | 0.002 | [0.002, 0.002, 0.002, 0.002, 0.002] |
| 32 / direct entry (refused) | 0.014 | 0.002 | 0.003 | [0.003, 0.002, 0.002, 0.001, 0.001] |
| 32 / shared control dispatch | 0.066 | 0.012 | 0.015 | [0.015, 0.012, 0.012, 0.011, 0.011] |
| 32 / direct engine only | 0.003 | 0.002 | 0.002 | [0.002, 0.002, 0.002, 0.002, 0.002] |

**Proposed envelope-only target: 0.025 s**, 4.86× the 5.144 ms cold maximum. This
includes headroom for first-use initialization and scheduling in the harness. It is
not a bound on pending or admitted Direct Control work.

## MCP / `HeadlessMcp`

Harness: [`headless_mcp_stop.rs`](../crates/mcp/examples/headless_mcp_stop.rs).
Uses the real MCP line server and `Headless` backend. The harness-only composition uses
forwarding with a synthetic host identity to reach the charged `execute` tool without a
budget or billing; it is not the production headless admission route. The stdio buffers are
in memory. The command and model match Direct Control. “Outside Headless” is MCP receive,
parse, dispatch/format, and reply decode, not the engine command.

Envelope: one MCP `execute` with no `expect` or batch, one `inspect.measure {names_only:true}`,
small JSON, one resident evaluated body. The synchronous headless tool call has no hard stop
inside a kernel call; preflight does not interrupt the call already in progress. Unbounded
B-rep rebuild history, multi-step tools, slow file I/O, and blocked stdio are outside this
envelope. The 600 s validation-runner ceiling is not a general MCP deadline.

| Cut features / phase | Cold ms | Median ms | Worst warm ms | Warm passes (ms) |
|---|---:|---:|---:|---|
| 0 / total | 1.883 | 0.233 | 0.250 | [0.250, 0.224, 0.223, 0.233, 0.236] |
| 0 / outside Headless | 1.849 | 0.226 | 0.241 | [0.241, 0.220, 0.218, 0.226, 0.230] |
| 8 / total | 0.329 | 0.234 | 0.246 | [0.246, 0.236, 0.227, 0.223, 0.234] |
| 8 / outside Headless | 0.308 | 0.230 | 0.238 | [0.238, 0.232, 0.223, 0.219, 0.230] |
| 32 / total | 0.347 | 0.230 | 0.312 | [0.234, 0.224, 0.220, 0.230, 0.312] |
| 32 / outside Headless | 0.325 | 0.224 | 0.303 | [0.230, 0.220, 0.216, 0.224, 0.303] |

**Proposed envelope-only target: 0.010 s**, 5.31× the 1.883 ms cold maximum. This
leaves headroom over cold decoding/dispatch and shared-host jitter for a small local
request. Most of the measured time is outside `Headless`; none of it is a full rebuild.

## MCP / `GuiBridgeMcp`

Harness: [`gui_bridge_mcp_stop.rs`](../crates/ui-egui/tests/gui_bridge_mcp_stop.rs).
Compiles the private `HostBridge` source unchanged and uses its real Unix socket lane, MCP
server, synthetic host provenance mint and control queue. The harness provides a minimal
recipient for the private request constructors. The recipient uses free local control
dispatch for the exact command above. “Outside control dispatch” times the round trip less
the recipient's work. This is not a measurement of the paid control recipient, regular window
scheduling, or settlement.

Envelope: a single MCP call with a live turn, one synchronous read of one resident evaluated
body, and an immediately serviced windowless queue. `CALL_TIMEOUT = 3,600 s` is a wait for
the app's reply, not a stop for app work. Invalidation can block a queued late request, but
cannot kill a kernel step already running. Unbounded history, unavailable consumers, and
queued permission/dispatch or uninterruptible work invalidate a pair-wide bound.

| Cut features / phase | Cold ms | Median ms | Worst warm ms | Warm passes (ms) |
|---|---:|---:|---:|---|
| 0 / total | 5.908 | 0.439 | 0.758 | [0.540, 0.758, 0.439, 0.395, 0.434] |
| 0 / outside control dispatch | 0.670 | 0.412 | 0.729 | [0.496, 0.729, 0.412, 0.372, 0.412] |
| 8 / total | 0.775 | 0.449 | 0.594 | [0.449, 0.440, 0.547, 0.594, 0.439] |
| 8 / outside control dispatch | 0.690 | 0.417 | 0.562 | [0.417, 0.412, 0.512, 0.562, 0.410] |
| 32 / total | 0.506 | 0.395 | 0.425 | [0.395, 0.425, 0.351, 0.422, 0.334] |
| 32 / outside control dispatch | 0.426 | 0.370 | 0.396 | [0.370, 0.396, 0.318, 0.390, 0.311] |

**Proposed envelope-only target: 0.030 s**, 5.08× the 5.908 ms cold maximum. This
includes headroom for first-use control initialization and cross-thread wakeups. It
does not reserve the production queue or bound the 3,600 s reply wait.

## Harness / `Mastra`

Harness: [`mastra_cancel_stop.rs`](../crates/ui-egui/examples/mastra_cancel_stop.rs).
The timer starts at cancellation and ends after both worker join and a `Cancelled` terminal
from the real `LiveTask` controller. This uses `LiveTask::on_cancel`, the call made by
`task_runtime::cancel`, and the public offline HostAi seam. It does not run the packaged
Mastra turn, nor measure retirement of its process. The buffered HTTP fake and an already
started tool callback fake each return after 100 ms. For the rebuild case, the worker signals
just before entering a real cacheless native rebuild (8 cuts); the supervisor cancels 20 ms
after the signal, while the worker is still busy. Its join measures residual rebuild time.
For HTTP the harness first joins the HostAi waiter, then waits for the physical worker
to drop its injected HTTP callback. The two rows report those separate observations.

The app's `MastraBackend::cancel` can retire the active generation and emit the terminal
event without waiting for the runtime, HTTP worker, or tool work. The 5 s runtime cancel
acknowledgement limit is not physical drain. HostAi caps each buffered HTTP request's requested
timeout at 600 s (usually lower, with connection, negotiated call limits and deadline), but a
blocking transport is not physically interrupted by invalidating its eventual reply. The
bridge's 3,600 s reply wait and an arbitrary synchronous rebuild are also not interrupted by
the task's terminal event. The current `MastraBackend::skill_support` reports `Unsupported`;
the Text-to-CAD controller seam is not evidence of an integrated Mastra skill run.

Envelope: one in-flight worker, the specified fake delays or one 8-cut native rebuild, no
pending writes, no permissions, no real platform transport, and no overlapping callback or
process teardown. Unbounded B-rep history and uninterruptible tools, HTTP, or file I/O make a
small pair-wide cancel bound unsafe. No figure based on total task duration is proposed.

| Cancel point | Cold ms | Median ms | Worst warm ms | Warm passes (ms) |
|---|---:|---:|---:|---|
| HostAi buffered model request (100 ms fake HTTP) / drained | 101.094 | 100.643 | 102.793 | [100.643, 100.504, 102.793, 100.399, 100.744] |
| HostAi buffered model request / waiter returned | 10.092 | 10.636 | 11.020 | [10.777, 10.514, 10.636, 11.020, 10.196] |
| In-flight tool callback (100 ms local fake) | 100.987 | 100.655 | 101.659 | [101.519, 100.371, 100.140, 101.659, 100.655] |
| Synchronous native rebuild (8 cuts; cancel after 20 ms) | 2412.814 | 2423.300 | 2538.717 | [2423.300, 2538.717, 2511.713, 2401.283, 2384.336] |

**Proposed envelope-only target: 10 s** from cancel to physical drain, 3.94× the
2.539 s maximum. The rebuild dominates; the factor is larger than the 2.7× load
spread seen for the separate 32-cut Review rebuild, without extending this result to
32 cuts or telling a live worker it must have stopped when the caller returned.

## Verification limits

All five harnesses completed using the cached-library binaries above. `cargo fmt --all --
--check` passed. Each new target passed a focused `clippy-driver --emit=metadata -D warnings`
check against those same dependencies (`--test` for the bridge). This does not claim a
fresh Cargo build or a full workspace lint run. No budget, production default, production
behaviour, or Review evidence was changed.
