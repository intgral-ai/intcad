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

**Shared work term.** The [Review record](dfm-review-safe-stop-bound.md#raw-output) reports
33.2 s median, 35.9 s worst quiet, and 96.5098 s in one heavy-load run for the 32-cut shape.
These are full Review rebuild-case wall times, not a distribution of pure kernel time.
The separate native rebuild pass was 34.4 s on the quiet host. Use the full 96.5098 s as
a conservative proxy for one comparable rebuild, including the high-load observation
without assuming its entire wall time was kernel work. The other harnesses' reads do not
incur a rebuild; their tables estimate only the local dispatch and protocol component.

The figures below are not pair-wide bounds. The stop window can prevent a new step only if
the entry checks it in time. Neither the duration printed by a reply waiter nor the
engine's validation deadline proves that a synchronous step has stopped. The 600 s constant
in [`validation/mod.rs:126`](../crates/engine/src/validation/mod.rs#L126) limits the validation
runner's execution window, not kernel work already in progress, and is not a universal
Control, MCP or Harness deadline. The entry shapes below were audited at `ac22ef39f1`,
before this documentation correction.

## DFM / `CliDfm`

Harness: [`cli_dfm_stop.rs`](../crates/engine/examples/cli_dfm_stop.rs).
Starts with an already captured candidate; times port open, profile registration, controlled
submit, exact claim, `LocalRunner`, durable completion, status read and JSON encoding. The
runner wraps the same `RunControl` used by `intcad-cli validate submit`; “outside LocalRunner”
includes report persistence. No CLI subprocess, request/profile file parsing, billing authority,
or settlement is timed. `validate skill` refuses mixed DFM profiles before work.

**Operation shape.** One `validate submit` registers one profile, submits and claims one
durable run ([`apps/solidcraft-cli/src/validation.rs:234–268`](../apps/solidcraft-cli/src/validation.rs#L234-L268)).
That run performs candidate integrity, one native rebuild and the remaining profile checks,
not one engine command ([`crates/engine/src/validation/runner.rs:155–206`](../crates/engine/src/validation/runner.rs#L155-L206)).

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

**Composed proposal: 300 s for the measured profile and fixture only.** Shared rebuild
proxy: 33.2 s median / 35.9 s worst quiet / 96.5098 s under load. Entry term: at most
0.001414 s outside `LocalRunner`; extra checks inside `LocalRunner` are already in the
37.255 s measured CLI maximum. That gives 96.511214 s before margin using the loaded
proxy, and about 203.49 s of margin for load and residual validation work (3.11× the
composed base; 8.05× the CLI maximum). This does not authorize a longer history,
guarantee a stop on arbitrary load, or include settlement.

## Control / `DirectControl`

Harness: [`direct_control_stop.rs`](../crates/ui-egui/examples/direct_control_stop.rs).
Times a real Direct Control request rejected before dispatch, then a companion request via the
free local entry that uses the same `control::handle` command dispatch and reply channel.
The command is one read-only `engine.execute` → `inspect.measure {names_only:true}` on an
evaluated session. The engine-only row uses the same command without control handling.
No actual TCP/token/framing work, signed-in admission/settlement, queue backlog, rendering,
permission wait, or agent cursor animation is included.

**Operation shape.** The TCP server turns one wire method into one `trusted_control` request
([`apps/solidcraft/src/control_server.rs:947–969`](../apps/solidcraft/src/control_server.rs#L947-L969)).
`engine.execute` / `command` passes one command ID to `app.run` with UI follow-up
([`crates/ui-egui/src/control.rs:2223–2229`](../crates/ui-egui/src/control.rs#L2223-L2229),
[`:1503–1572`](../crates/ui-egui/src/control.rs#L1503-L1572)). There is no cheap-command
allowlist for generic unguarded execute. For example `solid.hole` is a registered engine
command ([`crates/engine/src/cmd/features.rs:104`](../crates/engine/src/cmd/features.rs#L104)).
The guarded-document path has its own supported classes, not a cost cap
([`crates/ui-egui/src/execution_guard.rs:178–230`](../crates/ui-egui/src/execution_guard.rs#L178-L230)).

One request can also dispatch a controlled `engine.batch` of up to 10,000 steps
([`crates/ui-egui/src/control.rs:1586–1587`](../crates/ui-egui/src/control.rs#L1586-L1587),
[`:1758–1765`](../crates/ui-egui/src/control.rs#L1758-L1765)). The exact batch allowlist
excludes `solid.hole`, but **includes `parameters.change`, `solid.extrude`, and
`solid.revolve`** ([`:1630–1675`](../crates/ui-egui/src/control.rs#L1630-L1675)).
`parameters.change` can invalidate upstream geometry and rebuild the dependent B-rep history:
the command settles the edit and refreshes the document
([`crates/engine/src/cmd/parameters.rs:258–281`](../crates/engine/src/cmd/parameters.rs#L258-L281),
[`crates/engine/src/lib.rs:1593–1624`](../crates/engine/src/lib.rs#L1593-L1624)).
The controlled gate refuses `engine.script`, explicitly nested batch/script IDs, and certain
async file methods, not heavy modelling in general ([`crates/ui-egui/src/control.rs:226–247`](../crates/ui-egui/src/control.rs#L226-L247),
[`:900–911`](../crates/ui-egui/src/control.rs#L900-L911)).

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

**Retraction and composition.** Retract **0.025 s** as a stop bound. Shared term: 33.2 s
median / 35.9 s worst quiet / 96.5098 s under load. Measured entry term: use the entire
**0.005144 s** worst cold shared dispatch, without subtracting the few microseconds of the
read itself. **Proposed 300 s per comparable synchronous command**: 96.514944 s observed
proxy plus about 203.49 s of margin (3.11×), using the observed load spread rather than
assuming the cold protocol time is the largest source of variance.

This proposal covers one command that does at most one comparable rebuild of the simple
32-cut fixture with a ready consumer, small input and output and no pending work. For a
batch, the current code reserves `N × step_bound` for `N` steps and checks remaining time
again before substeps ([`crates/ui-egui/src/control.rs:1689–1730`](../crates/ui-egui/src/control.rs#L1689-L1730)).
A completion estimate for `N` comparable steps is **`N × 300 s` plus separate batch
overhead and checkpoint/rollback**, not 300 s for the request. A coalesced edit may be
attempted before fallback to individual steps; rollback can run after a failed gate and was
not measured ([`crates/engine/src/batch.rs:64–147`](../crates/engine/src/batch.rs#L64-L147)).
This is not evidence that the code's reserve covers that tail. No full-batch or pair-wide
bound is proposed. Arbitrary feature history, more complex operations, uninterruptible
steps, UI work after a mutation, slow I/O, queue and permission waits, and live settlement
are outside the envelope. The 60 s short and 3,600 s patient/batch TCP waits report
running work; they do not cancel it ([`apps/solidcraft/src/control_server.rs:558–563`](../apps/solidcraft/src/control_server.rs#L558-L563),
[`:960–969`](../apps/solidcraft/src/control_server.rs#L960-L969)).

## MCP / `HeadlessMcp`

Harness: [`headless_mcp_stop.rs`](../crates/mcp/examples/headless_mcp_stop.rs).
Uses the real MCP line server and `Headless` backend. The harness-only composition uses
forwarding with a synthetic host identity to reach the charged `execute` tool without a
budget or billing; it is not the production headless admission route. The stdio buffers are
in memory. The command and model match Direct Control. “Outside Headless” is MCP receive,
parse, dispatch/format, and reply decode, not the engine command.

**Operation shape.** The CLI production composition currently leaves paid authority absent
([`apps/solidcraft-cli/src/account.rs:119–137`](../apps/solidcraft-cli/src/account.rs#L119-L137),
[`:153–175`](../apps/solidcraft-cli/src/account.rs#L153-L175)). This is fail-closed, not a
small execution bound. In the metered path one admitted `tools/call` wraps the entire tool
and settles afterward; preflight checks available time once, not between commands
([`crates/mcp/src/account.rs:277–295`](../crates/mcp/src/account.rs#L277-L295),
[`:419–435`](../crates/mcp/src/account.rs#L419-L435)). Plain `execute` with no `expect` can
dispatch any available registered command, including heavy modelling, through `Headless`
([`crates/mcp/src/tools.rs:1118–1121`](../crates/mcp/src/tools.rs#L1118-L1121),
[`crates/mcp/src/backend.rs:1091–1101`](../crates/mcp/src/backend.rs#L1091-L1101)).
With `expect`, it can add `inspect.measure`
([`crates/mcp/src/tools.rs:848–850`](../crates/mcp/src/tools.rs#L848-L850),
[`:933–938`](../crates/mcp/src/tools.rs#L933-L938)). One tool can also be a `batch` (up to
10,000 steps, with sequential fallback and optional rollback) or `check_parameters` (read,
checkpoint, edits and measurements, restore)
([`crates/mcp/src/tools.rs:15`](../crates/mcp/src/tools.rs#L15),
[`:1588–1597`](../crates/mcp/src/tools.rs#L1588-L1597),
[`:1681–1769`](../crates/mcp/src/tools.rs#L1681-L1769),
[`:2052–2068`](../crates/mcp/src/tools.rs#L2052-L2068)).
The multi-backend-call refusal applies only to forwarding, not metered Headless
([`crates/mcp/src/server.rs:291–325`](../crates/mcp/src/server.rs#L291-L325)).

| Cut features / phase | Cold ms | Median ms | Worst warm ms | Warm passes (ms) |
|---|---:|---:|---:|---|
| 0 / total | 1.883 | 0.233 | 0.250 | [0.250, 0.224, 0.223, 0.233, 0.236] |
| 0 / outside Headless | 1.849 | 0.226 | 0.241 | [0.241, 0.220, 0.218, 0.226, 0.230] |
| 8 / total | 0.329 | 0.234 | 0.246 | [0.246, 0.236, 0.227, 0.223, 0.234] |
| 8 / outside Headless | 0.308 | 0.230 | 0.238 | [0.238, 0.232, 0.223, 0.219, 0.230] |
| 32 / total | 0.347 | 0.230 | 0.312 | [0.234, 0.224, 0.220, 0.230, 0.312] |
| 32 / outside Headless | 0.325 | 0.224 | 0.303 | [0.230, 0.220, 0.216, 0.224, 0.303] |

**Retraction and composition.** Retract **0.010 s** as a stop bound. Shared term: 33.2 s
median / 35.9 s worst quiet / 96.5098 s under load. Entry term: **0.001849 s** worst cold
outside `Headless` (in-memory MCP line, decoding and reply). **Proposed 300 s** for one
`execute` without `expect` that does at most one comparable 32-cut rebuild, with small
JSON and responsive local I/O: 96.511649 s before margin, plus about 203.49 s (3.11×).
A tool that runs `N` comparable commands needs the sum of those command times, not the
longest one alone; an estimate would begin at **`N × 300 s`** with separate evidence
for additional work such as expectations, retries of coalesced edits and rollback.
The tool has no general stop gate between substeps. Unbounded rebuilds, multi-step tools,
blocked stdio, slow files, and paid authority/settlement are outside the single-command
envelope. No full-tool bound follows for the entry as implemented.

## MCP / `GuiBridgeMcp`

Harness: [`gui_bridge_mcp_stop.rs`](../crates/ui-egui/tests/gui_bridge_mcp_stop.rs).
Compiles the private `HostBridge` source unchanged and uses its real Unix socket lane, MCP
server, synthetic host provenance mint and control queue. The harness provides a minimal
recipient for the private request constructors. The recipient uses free local control
dispatch for the exact command above. “Outside control dispatch” times the round trip less
the recipient's work. This is not a measurement of the paid control recipient, regular window
scheduling, or settlement.

**Operation shape.** With authenticated provenance, `HostBridge` opens the forwarded MCP
server, mints a `GuiBridgeMcp` logical context, and each backend call enqueues a control
request with that turn's flag
([`crates/ui-egui/src/agent_chat/host_bridge.rs:213–238`](../crates/ui-egui/src/agent_chat/host_bridge.rs#L213-L238),
[`:285–295`](../crates/ui-egui/src/agent_chat/host_bridge.rs#L285-L295),
[`:351–359`](../crates/ui-egui/src/agent_chat/host_bridge.rs#L351-L359)).
The audited app composition does not supply the scope, so its charged tools still fail closed
([`crates/ui-egui/src/agent_chat/mod.rs:1455–1467`](../crates/ui-egui/src/agent_chat/mod.rs#L1455-L1467)).
The forwarded server refuses `expect`, `check_parameters`, `body_topology`, and
`inspect_design` with sketch/joint follow-ups before dispatch
([`crates/mcp/src/server.rs:291–301`](../crates/mcp/src/server.rs#L291-L301),
[`crates/mcp/src/account.rs:356–374`](../crates/mcp/src/account.rs#L356-L374)).
It does **not** refuse the `batch` tool outright: an in-limit batch can reach the GUI as one
`engine.batch` request and execute multiple engine commands under the same controlled batch
allowlist and per-step room checks as Direct Control
([`crates/mcp/src/tools.rs:1681–1696`](../crates/mcp/src/tools.rs#L1681-L1696),
[`crates/ui-egui/src/control.rs:1630–1730`](../crates/ui-egui/src/control.rs#L1630-L1730),
[`:1758–1765`](../crates/ui-egui/src/control.rs#L1758-L1765)).
The unguarded MCP batch code can also attempt sequential fallback when not sent as a native
batch; that does not give its backend requests independent authority
([`crates/mcp/src/tools.rs:1681–1755`](../crates/mcp/src/tools.rs#L1681-L1755),
[`crates/ui-egui/src/agent_chat/host_bridge.rs:213–234`](../crates/ui-egui/src/agent_chat/host_bridge.rs#L213-L234)).
The restrictions are on forwarding and batch command classes, not on rebuild cost: plain
`execute` has no cheap-command allowlist and `parameters.change` remains an allowed batch step.

| Cut features / phase | Cold ms | Median ms | Worst warm ms | Warm passes (ms) |
|---|---:|---:|---:|---|
| 0 / total | 5.908 | 0.439 | 0.758 | [0.540, 0.758, 0.439, 0.395, 0.434] |
| 0 / outside control dispatch | 0.670 | 0.412 | 0.729 | [0.496, 0.729, 0.412, 0.372, 0.412] |
| 8 / total | 0.775 | 0.449 | 0.594 | [0.449, 0.440, 0.547, 0.594, 0.439] |
| 8 / outside control dispatch | 0.690 | 0.417 | 0.562 | [0.417, 0.412, 0.512, 0.562, 0.410] |
| 32 / total | 0.506 | 0.395 | 0.425 | [0.395, 0.425, 0.351, 0.422, 0.334] |
| 32 / outside control dispatch | 0.426 | 0.370 | 0.396 | [0.370, 0.396, 0.318, 0.390, 0.311] |

**Retraction and composition.** Retract **0.030 s** as a stop bound. Shared term: 33.2 s
median / 35.9 s worst quiet / 96.5098 s under load. Entry term: **0.005908 s** worst
cold total for the bridge plus its windowless control recipient; the bridge-only
portion peaked at **0.000729 s**. Using the total accounts for both layers without
adding the Direct Control proxy again. **Proposed 300 s** for one forwarded command
with at most one comparable 32-cut rebuild and an immediately serviced queue:
96.515708 s before margin, plus about 203.48 s (3.11×). As with Direct Control, for
`N` comparable batch steps the estimate starts at **`N × 300 s`** plus unmeasured
batch/rollback overhead. No full-batch or pair-wide bound is proposed.

The envelope assumes a live turn, small request/reply, and no pending permission or
dispatch wait. Larger or more complex B-rep histories, retries/fallback, unavailable
consumers and settlement can violate it. `CALL_TIMEOUT = 3,600 s` limits waiting for
the app's reply, not app work. Turn invalidation can block queued dispatch, but cannot
interrupt a kernel step already running
([`crates/ui-egui/src/agent_chat/host_bridge.rs:32`](../crates/ui-egui/src/agent_chat/host_bridge.rs#L32),
[`:142–175`](../crates/ui-egui/src/agent_chat/host_bridge.rs#L142-L175),
[`:233–247`](../crates/ui-egui/src/agent_chat/host_bridge.rs#L233-L247),
[`crates/ui-egui/src/control.rs:1901–1904`](../crates/ui-egui/src/control.rs#L1901-L1904)).

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

**Operation shape.** The Text-to-CAD controller's turn is not one command: the task
budget allows multiple model/tool calls. The owning turn is checked before every CAD tool;
the controller allows the `solid`, `parameters`, and other modelling families and rejects
file/document/script writes, not expensive rebuilds. A successful write may also capture
a result snapshot
([`crates/ui-egui/src/agent_chat/task_runtime.rs:37–39`](../crates/ui-egui/src/agent_chat/task_runtime.rs#L37-L39),
[`:117–165`](../crates/ui-egui/src/agent_chat/task_runtime.rs#L117-L165),
[`crates/agent-task/src/controller.rs:238–263`](../crates/agent-task/src/controller.rs#L238-L263),
[`crates/agent-task/src/skill.rs:59–120`](../crates/agent-task/src/skill.rs#L59-L120)).
This seam is not an integrated Mastra skill turn: `MastraBackend::skill_support` returns
`Unsupported`, the chat checks that before starting a non-plain turn, and the explicit
`trusted_mastra` calls into `task_runtime`'s control harness are test-only
([`crates/ui-egui/src/agent_chat/mastra_backend.rs:2265–2268`](../crates/ui-egui/src/agent_chat/mastra_backend.rs#L2265-L2268),
[`crates/ui-egui/src/agent_chat/mod.rs:2253–2259`](../crates/ui-egui/src/agent_chat/mod.rs#L2253-L2259),
[`crates/ui-egui/src/agent_chat/task_runtime.rs:316–323`](../crates/ui-egui/src/agent_chat/task_runtime.rs#L316-L323),
[`:416–421`](../crates/ui-egui/src/agent_chat/task_runtime.rs#L416-L421)).
The plain Mastra workflow can make multiple host model calls and use the bridge; those
charged CAD calls take the separate `GuiBridgeMcp` route if composed with authority.
The model callback actually calls `host.generate` and checks its runtime cancellation flag
after it returns, so the harness's direct HostAi cancel is not evidence of that callback's
stop time
([`crates/ui-egui/src/agent_chat/mastra_backend.rs:841–850`](../crates/ui-egui/src/agent_chat/mastra_backend.rs#L841-L850),
[`:1724–1729`](../crates/ui-egui/src/agent_chat/mastra_backend.rs#L1724-L1729),
[`:1763–1776`](../crates/ui-egui/src/agent_chat/mastra_backend.rs#L1763-L1776)).

The GUI Stop clears the turn, calls `task_runtime::cancel` and declines held permissions;
`MastraBackend::cancel` requests cancellation and emits the terminal without joining work
([`crates/ui-egui/src/agent_chat/mod.rs:2466–2496`](../crates/ui-egui/src/agent_chat/mod.rs#L2466-L2496),
[`crates/ui-egui/src/agent_chat/task_runtime.rs:169–175`](../crates/ui-egui/src/agent_chat/task_runtime.rs#L169-L175),
[`crates/ui-egui/src/agent_chat/mastra_backend.rs:2246–2262`](../crates/ui-egui/src/agent_chat/mastra_backend.rs#L2246-L2262)).
The **5 s** cancellation acknowledgement and **3,600 s** bridge reply timeout do not
guarantee physical stop
([`crates/ui-egui/src/agent_chat/mastra_protocol.rs:304`](../crates/ui-egui/src/agent_chat/mastra_protocol.rs#L304),
[`crates/ui-egui/src/agent_chat/runtime_client.rs:734`](../crates/ui-egui/src/agent_chat/runtime_client.rs#L734),
[`crates/ui-egui/src/agent_chat/host_bridge.rs:32`](../crates/ui-egui/src/agent_chat/host_bridge.rs#L32)).
HostAi asks the buffered HTTP transport for at most **600 s**, or less if constrained by
the connection, call limits or deadline. This is a per-request timeout setting, not the
engine's execution ceiling or a measured cancel-to-drain guarantee for the packaged
runtime ([`crates/ui-egui/src/agent_chat/host_ai/mod.rs:425–438`](../crates/ui-egui/src/agent_chat/host_ai/mod.rs#L425-L438)).

| Cancel point | Cold ms | Median ms | Worst warm ms | Warm passes (ms) |
|---|---:|---:|---:|---|
| HostAi buffered model request (100 ms fake HTTP) / drained | 101.094 | 100.643 | 102.793 | [100.643, 100.504, 102.793, 100.399, 100.744] |
| HostAi buffered model request / waiter returned | 10.092 | 10.636 | 11.020 | [10.777, 10.514, 10.636, 11.020, 10.196] |
| In-flight tool callback (100 ms local fake) | 100.987 | 100.655 | 101.659 | [101.519, 100.371, 100.140, 101.659, 100.655] |
| Synchronous native rebuild (8 cuts; cancel after 20 ms) | 2412.814 | 2423.300 | 2538.717 | [2423.300, 2538.717, 2511.713, 2401.283, 2384.336] |

**Retraction and composition.** Retract **10 s** as a Harness stop bound. It included
a rebuild, but only 8 cuts. After 20 ms in that rebuild the harness measured **2.423 s
median / 2.539 s worst** to drain. No fresh 32-cut cancellation measurement is used:
the cached UI library needed to link the extended harness was unavailable during the
follow-up, so no new build is claimed. Instead allow the *full* shared step, as a
cancel can arrive just after it starts: **33.2 s median / 35.9 s worst quiet /
96.5098 s under load**, plus **0.102793 s** as an allowance equal to the largest
non-CAD drain measured (fake HTTP; the fake tool drained in 0.101659 s). The sum
allows one such drain tail after a rebuild without assuming it overlaps; it is not
an observation of that sequence. **Proposed 300 s** for that restricted cancel-to-drain
envelope: 96.612593 s before margin, plus about 203.39 s (3.11×).

The modeled envelope permits one started rebuild comparable to the simple 32-cut fixture
and no next CAD step after cancel, with at most the measured-size offline drain tail.
It excludes multiple in-flight calls, new work, pending writes, rollback, snapshot I/O,
permission UI, real HTTP transport, process teardown, and a busy UI thread that cannot
process Stop until work returns. Arbitrary uninterruptible native work can take longer
than the whole window. No pair-wide cancel bound or total task duration follows.

## Why not use 600 seconds?

Two comparable commands already give `2 × 300 s = 600 s` **before** batch setup,
rollback or delivery. That is arithmetic for a restricted workload, not a 600 s
measurement or an engine-imposed batch limit. The validation ceiling supplies no
evidence for a general paid pair: it clamps when validation should stop checking, not
the wall time of a running rebuild. None of these measurements justifies using that
ceiling in place of a bound.

## Verification limits

All five harnesses completed using the cached-library binaries above. `cargo fmt --all --
--check` passed. Each new target passed a focused `clippy-driver --emit=metadata -D warnings`
check against those same dependencies (`--test` for the bridge). This does not claim a
fresh Cargo build or a full workspace lint run. No budget, production default, production
behaviour, or Review evidence was changed.
