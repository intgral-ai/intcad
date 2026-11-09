# Rig CAD harness spike (Track B-B)

Isolated proof for `plan/2026-11-09-rig-spike-report.md`. This crate is **not**
a member of the product workspace; it has its own `[workspace]` and its own
`Cargo.lock`. It does not change the product MSRV, CI gates, or `main`.

## What is real vs fake

- **Real:** `solidcraft-engine` (parametric document, `Session`,
  `document.snapshot.capture`) and `solidcraft-engine::validation`
  (`ValidationPort`, immutable candidate, check profile). `src/engine_host.rs`.
- **Fake, labelled:** `rig::test_utils::MockCompletionModel` is the scripted
  provider. It performs no network calls. It is used to drive the same task
  contract and to inject faults. `src/scenario.rs`.
- **Mock, labelled:** `src/mock_host.rs` is an in-memory CAD host for the
  state-machine and fault-injection tests only. It is not CAD evidence.
- **Real Rig:** `rig = "=0.46.0"`, `default-features = false`,
  `features = ["agent", "test-utils", "derive"]`; MIT; MSRV 1.97.1.

## Headless invocation

The default host is the real engine. The provider is still the labelled fake
one (there is no network provider in this spike).

```bash
cargo run --release --bin cad-harness -- --scenario happy --host engine
cargo run --release --bin cad-harness -- --scenario all
cargo run --release --bin cad-harness -- --scenario stale --host mock
```

`--scenario all` runs the real-engine happy/repair/undo/early-stop/
missing-validation/quality-failure/write-without-inspect/plain-chat cases and
the mock-only stale/unknown/partial/denied/cancel/timeout cases.

The headless output is JSON: host, provider label, terminal, events, real
engine inspection (parameters, volume, hole count) and validation checks.

## Tests

```bash
cargo test --test mock_state_machine       # 17 passed (state machine/tool boundary)
cargo test --test real_engine              # 1 passed, sequential real-engine acceptance
cargo test                                  # all of the above
cargo clippy --all-targets -- -D warnings
```

The real-engine acceptance runs these sequentially because the candidate store
is process-global in the engine: happy path, real validation failure -> bounded
repair, undo -> re-apply -> validate, early stop, missing validation, quality
failure, stale/permission bypass refusal, and in-flight cancel rollback.

## Where the parts live

| File | Purpose |
|---|---|
| `src/types.rs` | CAD Task Contract types, `CadHost` port, terminal/events |
| `src/controller.rs` | Task controller/state machine; permit, version, snapshot, validation, repair, cancel, reconcile |
| `src/hooks.rs` | The only Rig hook: `on_dispatch`, `on_outcome`, `on_model_turn_finished`, `on_run_settled` |
| `src/tools.rs` | Five Rig tools; each calls the controller/HostDispatch port, never an engine |
| `src/rig_runner.rs` | One `AgentBuilder`, one tool registry; task run with the hook; plain chat without it |
| `src/mock_host.rs` | Labelled mock host + fault injection |
| `src/engine_host.rs` | Real engine worker thread, candidate/validation store, checkpoint rollback |
| `src/bin/cad_harness.rs` | Headless CLI |

## Known limits

- Not wired into the product `ChatBackend`, panel, `gate.rs`, `pump_tools`, or
  `crates/control`; this is a spike, not product integration.
- Real ACP E2E is not run. The host permit refusal is the code-level guard; the
  report analyses the ACP path.
- Real BYOK/provider transport is not implemented. The seam is Rig's
  `Model<Wire, Transport>`: a HostAi transport would keep keys in Rust. The
  spike deliberately uses no network provider.
- Unknown/partial write and timeout are fault-injected in the mock host; the
  real engine covers happy/repair/undo/stale/permission/cancel.
- Rig's MSRV is 1.97.1, above the product workspace's 1.95. A production
  replacement needs a toolchain/CI bump or an isolated runtime.
