# Rig CAD-harness spike report (Track B-B, 2026-11-09)

Status: **isolated spike; not merged to develop.** Worktree
`/tmp/opencode/osc-rig-spike`, branch `spike/rig-cad-harness`, base
`origin/develop` `1b13fdf87d2bb76a233b780c979f62d64db851b6`. No product
crate, CI gate, main checkout, Node/TS package or Phase 1 code was changed.

The PoC lives in `poc/rig-cad-harness/` with its own `[workspace]`,
`Cargo.toml` and `Cargo.lock`, so it cannot change the product workspace's
Rust 1.95 MSRV or dependency graph.

## 1. Answer

**Rig 0.46 can be the single in-process Agent Execution Runtime for the same
CAD Task Contract, provided the CAD host remains the authority.** Rig owns the
model/tool/history loop; the host-side `TaskController` owns permission,
expected document version, snapshot, validation, undo, cancellation, unknown
writes and the task terminal. The spike found no need for a second loop: Rig's
official `AgentHook` API has the tool and turn lifecycle the task contract
needs (`on_dispatch`, `on_outcome`, `on_model_turn_finished`,
`on_run_settled`).

This is a **replacement candidate for the native Direct `ApiBackend` loop**,
not a second stack. If adopted, the native Direct loop/provider/session/tool
dispatch is retired or re-homed (§9). The spike does **not** prove product
integration: it does not wire the real `ChatBackend`, panel, `gate.rs`,
`pump_tools`, `crates/control`, BYOK transport, or a real ACP session (§8).
ACP remains an external agent loop by definition; Rig cannot "own" an external
agent, but every ACP CAD call must still pass the same host gate.

The real-engine L-bracket acceptance passes: 5 mm / four holes becomes 8 mm,
hole diameter and four-hole count are preserved, and the real
`ValidationPort` passes four required checks on the exact final candidate.

## 2. Exact Rig facts used

| Fact | Value | Evidence |
|---|---|---|
| Version | `rig = "=0.46.0"` | `poc/rig-cad-harness/Cargo.toml`, `Cargo.lock` |
| Source commit | `6e039cd24994bc1e7d963af6f6278df73a81b130` (`v0.46.0`) | shallow checkout `/tmp/opencode/rig-research` |
| Licence | MIT (all of `rig`, `rig-core`, `rig-agent`, `rig-derive`, `rig-harness-protocol`) | package metadata and `LICENSE` |
| MSRV | `1.97.1`; edition 2024 | `Cargo.toml:217`, package metadata |
| Features used | `default-features = false`, `features = ["agent", "test-utils", "derive"]` | PoC manifest |
| Provider | **fake**: `rig::test_utils::MockCompletionModel` (official test utility, `test-utils` feature) | `crates/rig-core/src/test_utils/completion.rs:403` |
| Network stack | **none selected**: `cargo tree` has no `reqwest`, `rustls`, `aws-lc-sys`, websocket or provider crate | `cargo tree -e normal` |
| Runtime | `tokio` 1.54.1, current-thread runtime on a worker/test thread; Rig core/agent are runtime-agnostic | PoC `Cargo.toml`, `bin/cad_harness.rs` |
| `max_turns` default | **1**; the spike sets 24 explicitly | `crates/rig-agent/src/run/spec.rs:122-123` |
| Run persistence | `AgentRun` serializable, `RUN_FORMAT = 5`; other formats are refused | `crates/rig-agent/src/run/mod.rs:304,356-368` |
| Task terminal | Rig has **no** task terminal; `ModelTurnAction::Stop` ends a run (surfaces as `PromptError::Cancelled`) | `crates/rig-agent/src/agent/hook.rs:439-467` |

The provider packages that appear in `cargo metadata` as optional dependencies
(`reqwest`, `rig-reqwest`, provider crates) are **not** enabled. This spike
therefore evaluates Rig's loop/tool/hook API, not its HTTP provider clients.

## 3. What was built

```
poc/rig-cad-harness/
  src/types.rs          task contract types, CadHost port, events/terminal
  src/controller.rs     TaskController: policy/state machine, permit, versions,
                        snapshots, validation, bounded repair, cancel, reconciliation
  src/hooks.rs          TaskControlHook: on_dispatch, on_outcome,
                        on_model_turn_finished, on_run_settled
  src/tools.rs          cad_inspect, cad_capture_snapshot, cad_apply,
                        cad_validate, cad_undo; every action goes through the controller
  src/rig_runner.rs     one AgentBuilder/model/tool registry; task run with hook,
                        plain chat without the hook
  src/mock_host.rs      labelled in-memory host + fault injection
  src/engine_host.rs    real solidcraft engine on a worker thread; real
                        snapshot/ValidationPort; checkpoint rollback
  src/scenario.rs       product scenario + scripted fake-provider turns
  src/bin/cad_harness.rs headless invocation
  tests/mock_state_machine.rs  17 tool/turn/task tests
  tests/real_engine.rs         real-engine acceptance (sequential)
```

The `CadHost` port is deliberately small and has **no raw engine handle**:
`document`, `inspect`, `capture_snapshot`, `dispatch_write`, `undo`,
`validate`, `reconcile`, `request_permission`, `register_permit`,
`revoke_permit`, `cancel`, `is_cancelled`.

The controller mints the only `WritePermit` for the run. A write with no permit
(external/ACP-style caller), an unregistered permit, a wrong expected revision,
a cancelled turn, or an unresolved unknown write is refused before the engine
sees it. The model never supplies the expected version, the permit, or the
terminal.

## 4. Rig lifecycle: what is actually exposed

Checked against the v0.46.0 source, not summaries:

- `AgentHook` trait: `crates/rig-agent/src/agent/hook.rs:938`.
- `on_model_turn_finished`: `:1005`; returns `ModelTurnAction::{Continue, Retry, Stop}`
  (`:439`). This is the turn-end hook. A **tool-free** turn may be rejected and
  retried with feedback; a tool-bearing turn may not be retried (Rig refuses to
  leave unanswered tool calls). This is enough to refuse model-only success.
- `on_dispatch`: `:1085`; sees tool name/args before execution and returns
  `DispatchAction::{Proceed, Patch, Deny}` (`:639`). `Deny` with a normal report
  becomes a skipped tool result; `ErrorKind::Cancelled` stops the run.
- `on_outcome`: `:1104`; sees the tool result/error after execution and can
  replace it or stop the run (`OutcomeAction`, `:770`).
- `on_run_settled`: `:962`; observes success/error after the run.
- `Tool` trait: `crates/rig-core/src/tool/contextual.rs:32`; async `call`, typed
  args/output, `Send + Sync`.
- Model/transport seam: `Model<Wire, Transport>` in
  `crates/rig-core/src/driver.rs`; a HostAi transport can be implemented so
  BYOK keys never enter Rig's provider clients.

**Conclusion on requirement 4:** Rig *does* expose the necessary turn-end and
tool lifecycle hooks. The spike implements the minimal wrapper as an
`AgentHook` (`src/hooks.rs`); it is **not** a loop and does not create a second
provider/session/tool registry. The only gap is that a tool-bearing turn cannot
be retried by the hook (Rig rejects that by design), but the tool result feeds
the next model turn, so bounded repair still works without a second loop.

## 5. Tool / turn / task boundaries

### Tool boundary

- `cad_apply` calls `TaskController::apply`; the controller pins
  `expected = document()`, requires a baseline snapshot bound to that revision,
  and calls `CadHost::dispatch_write` with its permit.
- The real engine checks the revision (`Session::check_document_stamp`) and
  wraps the ops in `Session::checkpoint`; any command failure or cancellation
  calls `restore_checkpoint` and returns `Failed`/`Cancelled`/`Unknown`.
- `cad_undo` uses the real engine's `edit.undo` and clears snapshots/validation.
- `cad_validate` uses only the exact final snapshot whose source revision equals
  the current revision; a stale snapshot is refused.
- `cad_capture_snapshot` is host-decided: baseline before a write, final after a
  committed write (`final_pending`), never model-decided.
- Permission is requested by the host (`PermissionMode`/`EnginePermission`);
  Deny/Ask/AllowList/DenyAll never reach the engine.
- `write_id` is host-generated; a duplicate `write_id` returns the recorded
  `Applied { replayed: true }` without writing twice.

### Turn boundary

`TaskControlHook::on_model_turn_finished` calls `controller.turn_end(has_tool_calls)`:

- terminal success -> `Continue`;
- any failure terminal -> `Stop(reason)`;
- tool-bearing turn -> `Continue` (tool results force another model turn);
- tool-free turn without trusted validation -> bounded `Retry(feedback)` while
  budget remains, otherwise `Stop` with `Incomplete`/`QualityFailed`.

The model's prose never sets `ValidatedSuccess`. The controller's terminal is
the only success authority.

### Task boundary

`validated_success` requires: inspected, a committed expected-version write,
a final snapshot bound to the current revision, no pending/unknown write, a
passing host report whose required checks all passed, and no later write. A
post-success write is denied by the controller, so it cannot silently invalidate
the terminal.

## 6. Evidence: real engine vs mock

### Real engine (acceptance)

`cargo run --release --bin cad-harness -- --scenario happy --host engine`
(also exercised in `tests/real_engine.rs`):

| Stage | Value |
|---|---|
| Initial document | 5 mm L bracket, 4 mounting holes, volume `19248.672587754347 mm³` |
| Guarded write | `parameters.change thickness = 8 mm`, expected revision pinned by controller |
| Final document | 8 mm, hole diameter `4 mm` unchanged, 4 holes, volume `29357.87614040794 mm³` |
| Expected volume | `29357.876140340508 mm³` |
| Required checks | `input.integrity` passed, `native.rebuild` passed, `parameters.required` passed, `geometry.exact` passed |
| Terminal | `ValidatedSuccess`, artifacts contain `snapshot:cand-…` and `report:run-…` |
| Parametric relation | hole diameter remains `hole_diameter = 4 mm`; four cylindrical faces counted at radius 2 mm |

The validation is the real `solidcraft-engine::validation::ValidationPort` with
a host-owned `l-bracket-task` profile (`parameters.required` + `geometry.exact`
on `/total/volume_mm3`), not a mock report.

Real-engine cases also pass:

- **Repair:** first apply 7 mm -> real `quality_failed`, one bounded repair to
  8 mm -> real `passed`.
- **Undo:** apply 8 mm -> real `edit.undo` -> re-capture -> apply 8 -> validate.
- **Early stop / missing validation:** write happens, terminal `Incomplete`;
  never `ValidatedSuccess`.
- **Quality failure:** 7 mm with zero repair budget -> `QualityFailed`.
- **Stale / permission / bypass:** old expected revision -> `Stale`; no permit
  -> `Denied`; `DenyAll` -> `Denied`, document still 5 mm.
- **In-flight cancel:** 500 `solid.box` ops started, cancel after write start;
  engine `restore_checkpoint` restored thickness 5 mm, body count 1, 4 holes;
  terminal `Cancelled`.

### Mock (state machine/fault injection only; labelled)

`--host mock` and `tests/mock_state_machine.rs` cover the faults a real
acceptance run cannot inject safely:

- permission Ask/Deny then Allow; DenyAll; permission requests recorded;
- stale-once; unknown-once; partial-once (one of two ops applied);
- duplicate `write_id` replay (revision moves once);
- unknown write refused for a second write and for validation; one-shot
  reconcile then final snapshot + validation;
- in-flight cancel; wall-clock timeout (budget 0); post-success write denied;
- turn-end with no validation; early stop; missing validation; bounded repair;
  quality failure with zero budget;
- plain chat with no CAD hook and no task/tool events.

Every mock result is labelled `MOCK` in the task event/CLI. Mock evidence is not
CAD evidence.

## 7. Commands and results

From `/tmp/opencode/osc-rig-spike/poc/rig-cad-harness`:

```text
cargo test --test mock_state_machine
  17 passed; 0 failed; finished in 0.07s

cargo test --test real_engine
  1 passed; 0 failed; finished in 72.20s
  (single sequential test: happy, repair, undo, early stop, missing validation,
   quality failure, stale/permission bypass, in-flight cancel)

cargo test
  17 passed (mock) + 1 passed (real engine) + 0 doctests; exit 0

cargo clippy --all-targets -- -D warnings
  exit 0

cargo run --bin cad-harness -- --scenario all
  happy              real-engine  ValidatedSuccess
  early_stop         real-engine  Incomplete
  missing_validation real-engine  Incomplete
  repair             real-engine  ValidatedSuccess
  quality_failed     real-engine  QualityFailed
  write_without_inspect real-engine Incomplete
  plain_chat         real-engine  (not a CAD task)
  stale              mock         Error
  unknown            mock         Unknown
  partial            mock         Unknown
  denied             mock         Incomplete
  cancel             mock         Cancelled
  timeout            mock         Error
```

The headless CLI is not mock-only: `--host engine` is the default and runs the
real EngineCadHost, real snapshot and real ValidationPort. The provider remains
the labelled fake `MockCompletionModel` (no network, no BYOK claim).

## 8. Twelve acceptance points for this spike

| # | Acceptance | Status | Evidence / gap |
|---|---|---|---|
| 1 | Direct loop executes CAD tools through gate/control | **Spike-equivalent** | Rig `on_dispatch` -> `TaskController` -> `CadHost`; product `gate.rs`/`pump_tools`/`crates/control` not wired |
| 2 | Real parametric create/modify with real engine evidence | **Pass** | 5 -> 8 mm, volume/4 holes, `Session`/`Snapshot`/`ValidationPort` |
| 3 | Host forces final snapshot + validation; no model-only success | **Pass** | `cad_validate` + `turn_end`; real 4-check report |
| 4 | Early stop cannot `validated_success` | **Pass** | real + mock `Incomplete` |
| 5 | Validation failure -> bounded repair or safe terminal | **Pass** | real 7 -> 8 repair; zero-budget `QualityFailed` |
| 6 | Cancel/timeout/stale: no unauthorized write, no false success | **Pass** | real in-flight cancel + rollback; mock timeout/stale; real stale/DenyAll |
| 7 | Lost/unknown tool result: no blind replay; truthful terminal | **Pass in mock** | unknown/partial fault injection, one-shot reconcile; real unknown not injected |
| 8 | Plain chat unaffected by CAD policy | **Pass** | same agent/model/tools, no hook; no task/tool events |
| 9 | No duplicate provider/session/tool registry/agent loop | **Pass in PoC** | one `AgentBuilder`, one `MockCompletionModel`, one tool set, one hook; product integration not proven |
| 10 | ACP bridge cannot bypass the same CAD gate | **Partial / code-level guard** | host refuses `permit: None`/unregistered permit; real ACP E2E not run; analysis says ACP must route through `pump_tools`/`CadHost` |
| 11 | Headless invocation | **Pass** | `cad-harness` default real engine, no UI |
| 12 | Same Task Contract/turn-end lifecycle/bounded task | **Pass** | `types.rs`, `controller.rs`, `hooks.rs`; `on_model_turn_finished` |

## 9. If Rig replaces the native Direct loop: retire/re-home

This spike does **not** modify these modules. A production replacement would:

| Native code | Action |
|---|---|
| `crates/ui-egui/src/agent_chat/api_client.rs` (`ApiBackend:78`, `Worker:196`, `turn:335`, tool loop, `TurnControl` use) | **Retire** the manual model/tool loop; replace with a `RigBackend` implementing `ChatBackend` |
| `crates/ui-egui/src/agent_chat/backend.rs` (`ChatBackend`, `Event`, `PromptPart`, `HttpFn`, `Secrets`) | **Keep** as the UI seam; add a Rig adapter and validation/snapshot items |
| `crates/ui-egui/src/agent_chat/host_ai/provider.rs`, `call.rs` (`TurnControl:236`) | **Re-home** as Rig `Transport`/`Wire` (HostAi transport), preserving keychain/admission/cancel; do not use Rig's reqwest providers directly |
| `crates/ui-egui/src/agent_chat/gate.rs`; `mod.rs::pump_tools` | **Keep**; Rig tools call `HostDispatch`/control; permission UI unchanged |
| `crates/ui-egui/src/control.rs` (`ControlRequest:62`, `chat_turn`, `guarded`) | **Keep**; the CAD host port wraps the same control queue/turn token |
| `crates/mcp/src/tools.rs` `tool_definitions()`, `crates/mcp/src/backend.rs` | **Keep/re-home** as Rig `DynamicTool`/`Tool` adapters; one registry, no second catalog |
| `crates/engine` snapshot/validation, `crates/io/src/candidate.rs` | **Keep**; wrapped by the host port |
| `crates/ui-egui/src/agent_chat/acp_client.rs` / `acp.rs` | **Keep** as external ACP; route its bridge tool calls through the same controller/host |
| `mastra_backend.rs`, `runtime_client/**`, `mastra_protocol/**`, `host_bridge.rs`, `packaged_runtime.rs`, TS runtime | Already **retire/defer** per roadmap; Rig does not need them |
| Session/history | Re-home the native `Worker.history` to Rig `AgentRun`/memory or keep a UI transcript; no persistent TaskStore in this spike |
| Panel | Keep; map Rig `Text`/tool/hook events; synthesize Plan/Permission/terminal in the host |

The key is **one** in-process loop. Compiling both `ApiBackend` and a Rig
backend and selecting between them would be a second stack; the recommendation
is replacement, with ACP remaining an external agent (not a second in-process
loop).

## 10. Desktop / async / integration cost

- **Async:** Rig is async. The PoC runs it on a dedicated current-thread Tokio
  runtime on a worker/test thread; the engine owns its own worker thread. Events
  are plain data, so the egui side would use the existing channel + repaint
  pattern. No Tokio/await in the egui frame.
- **MSRV:** Rig requires 1.97.1; the product workspace is 1.95. A production
  replacement must raise the toolchain/CI pin or isolate Rig (sidecar/feature).
  The PoC uses its own workspace so it does not change the product.
- **Dependencies:** with `default-features = false` the selected Rig closure has
  no reqwest/rustls/websocket/provider stack. A real BYOK provider adds that
  stack or, preferably, reuses HostAi through a custom `Transport`.
- **Session/history/persistence:** `AgentRun` is serializable (`RUN_FORMAT=5`)
  and refuses other formats; the spike keeps task/history in memory. There is no
  durable TaskStore or resume; Phase 1 still needs one. Engine undo is a real
  `Session` checkpoint, not a Rig feature.
- **Cancellation:** no `CancellationToken`; the host `AtomicBool` is
  authoritative, backed by `DispatchAction::stop`/`OutcomeAction::stop` and the
  engine checkpoint rollback. This matches the native `TurnControl` model.
- **Retry/exactly-once:** Rig rejects retrying a tool-bearing turn; the host
  uses `write_id` idempotency and one-shot reconciliation. Unknown writes are
  never replayed. The duplicate-`write_id` test moves revision once.
- **Provider/BYOK:** only the fake provider is tested. The production seam is
  Rig's `Model<Wire, Transport>`; a HostAi transport keeps keys in Rust and
  preserves the existing admission/redaction policy.
- **MCP/ACP:** the PoC has the five CAD tools, not the full 21-tool catalog; in
  product they should be generated from `crates/mcp` rather than duplicated. ACP
  remains external; the host permit/version checks are the shared gate.
- **Packaging/licence:** Rig is MIT; the selected closure is Cargo-only, no
  Node/TS, no GPL/LGPL/AGPL. A release still needs the full transitive licence
  audit; this spike only records the selected closure.

## 11. Gaps / what this spike does not prove

1. **Product integration:** no real `ChatBackend`, panel, `gate.rs`,
   `pump_tools`, `crates/control`, or `crates/mcp` wiring. The PoC re-creates the
   boundaries with a small host port.
2. **Real ACP:** no ACP session. The code-level guard (permit/version) is shown;
   the claim "ACP cannot bypass" still needs an ACP E2E through `pump_tools`.
3. **Real BYOK/provider:** fake provider only. HostAi-as-Rig-transport is the
   proposed seam, not implemented.
4. **Real unknown write:** unknown/partial are mock fault injection. The real
   engine path tests cancel rollback, stale, permission and validation, but not
   a lost reply after a real commit.
5. **Persistence/resume:** in-memory only; no TaskStore/checkpoint file.
6. **Timeout:** controller wall-clock budget, not a Rig-level run timeout; the
   mock test uses a zero budget.
7. **Turn retry limitation:** `on_model_turn_finished` cannot retry a
   tool-bearing turn (Rig design). The controller relies on tool results and
   tool-free turn feedback; this is sufficient for the tested contract but is a
   real API shape difference from the native loop.

None of these are silently claimed as passing. The acceptance table marks #1,
#9 and #10 with their exact scope.

## 12. Recommendation

- **Go for a Phase 1 pilot** of Rig as the replacement for the in-process Direct
  loop, not as a parallel stack. The lifecycle hooks exist and the host can keep
  all CAD authority.
- Before Phase 1, prototype the two integration seams not covered here:
  1. a `RigBackend` behind the real `ChatBackend`/panel with `gate.rs` +
     `pump_tools` + `crates/control`;
  2. a HostAi `Transport` for real BYOK, with no key in Rig/provider clients.
- Keep the native engine/snapshot/validation/undo/gate/control code; retire the
  native Direct loop, its history, and the Mastra/Node stack per the roadmap.
- Do **not** keep Rig and `ApiBackend` selectable as two live loops. That would
  be a second runtime and is not what this spike proves.
