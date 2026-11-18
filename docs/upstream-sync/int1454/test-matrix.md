# INT-1454 Phase 0 — validation and acceptance matrix

Gates follow `docs/validation-policy.md`: a check is reused only when its inputs are unchanged, and
a check that did not run is never reported as passed. Every row names the command, the SHA and the
artifact that proves it.

## 1. Baseline to protect

| Item | Baseline at `0fd92bea2` | Rule |
| --- | --- | --- |
| CI (Linux) | run `43955685642` success | no new failure |
| Lists / Kernel rig / Fillet rig / Packaging lint | `43955685446` / `43955685477` / `43955685536` / `43955685558` success | unchanged |
| Known failures | `tests/KNOWN_FAILING_TESTS`, 33 test entries (42 lines) | list may only shrink; no skip/ignore additions |
| Local full gate | `cargo xtask ci` | run for shared/contract changes |
| Vendored kernel suite | solidcore's own `xtask`/CI inside `solidcore/` | the kernel PR names which of those gates IntCAD runs and which it explicitly does not |
| E2E entry | `/home/hyjing/Code/intgral/intcad/.dev/e2e/bin/intcad-e2e` | only in the main checkout |

## 2. Per-area PR gates

| Area | Commands | Notes |
| --- | --- | --- |
| solidcore snapshot | `cargo check -p solidcraft-kernel`, `cargo test --profile ci -p solidcraft-kernel`, `cargo xtask kernel-cases --known-failing tests/kernel-cases/KNOWN_FAILING` | rig is the real proof for volume/bbox/topology |
| kernel source | above + `cargo xtask oracle`, `cargo xtask action-matrix` when behaviour changes | oracle/external corpus stay nightly unless asked |
| doc / format | `cargo test --profile ci -p solidcraft-doc -p solidcraft-format`, `cargo xtask mesh-decisions`, `cargo xtask layers` | persistence tests in `crates/doc`/`crates/io` |
| engine | `cargo test --profile ci -p solidcraft-engine`, `cargo xtask mesh-decisions`, `cargo xtask ui-containers` (if UI contract) | command schemas: `crates/engine/src/schema` |
| io / STEP | `cargo test --profile ci -p solidcraft-io`, `cargo xtask external-corpus --only <sample>` locally only on request | STEP conformance lives in `tests/step-conformance` |
| sketch | `cargo test --profile ci -p solidcraft-sketch` | no IntCAD divergence today |
| mcp | `cargo test --profile ci -p solidcraft-mcp` (+ IntCAD HostDispatch tests) | security paths need refusal tests |
| ui-egui | `cargo test --profile ci -p solidcraft-ui-egui`, `cargo xtask ui-containers`, `cargo xtask lists` | GUI proof by E2E, not unit tests |
| assets / locales / icons | `cargo xtask assets`, `cargo xtask icons`, `cargo xtask i18n --check`, `cargo xtask lists` | pending files, never shared tables |
| deps / workspace | `cargo xtask licences --check`, `cargo deny check advisories`, `cargo xtask wasm` | licences file is generated |
| docs / skills | `cargo xtask skills`, `cargo xtask lists`, `cargo xtask book --no-build` | book only when commands change |

Any unit that changes a shared input runs the consumers' checks too (`docs/validation-policy.md`).

## 3. Functional acceptance (Phase 4, at one fixed SHA)

| Capability | Evidence |
| --- | --- |
| geometry | volumes, bounding boxes, topology counts and Boolean outcomes through `cargo xtask kernel-cases --known-failing`; existing tolerances only |
| parameters / commands | `cargo xtask oracle` and the command schema audit; `cargo xtask features` |
| sketch constraints | `cargo test -p solidcraft-sketch`, action matrix rows for constrained profiles |
| assembly / joints | engine assembly tests, plus a saved-and-reopened assembly |
| import / export | STEP/STL/3MF round trip on repo samples; `tests/step-conformance` |
| **doc persistence** | save a real design, reopen it, and compare the stored data; one implementation only — no IntCAD-specific backup/format layer |
| DFM + viewport location | DFM acceptance tests plus a GUI run that selects a finding and locates it |
| Files / Review | GUI run through the real panels |
| Assistant / MCP / HostDispatch / Undo | tool call, approval, refusal and undo tests, then one GUI flow |
| security gating | refusal paths: out-of-scope file access, unapproved execution |

## 4. Platform matrix

| Platform | Gate | Who |
| --- | --- | --- |
| Linux | native CI (`ci.yml`), real GUI E2E via `.dev/e2e` | agent |
| Windows | `windows-fxc` + `windows-startup` CI jobs (startup, resize, assistant smoke) and a Windows VM GUI E2E through `intcad-e2e --platform windows` | agent |
| macOS | `release.yml` macOS build jobs and native automated tests; real desktop UI acceptance | PP (human) |

Evidence binds: commit SHA, command, environment (OS image, package type), CI run id or log path,
result, known limitations. A simulated or VM result is never reported as native desktop
acceptance, and the reverse.

## 5. GUI E2E protocol (local, per workstation rules)

- One stable, non-shared `--task <name>` per acceptance run; the same source+platform+task for
  `up`, `accept`, `control`, `shot`, `logs`, `stop`, `clean`.
- Never share the default `accept` task between agents; never operate another task's VM.
- Record: source worktree, platform, task, VM name, evidence directory, package type and hash.
- `intcad-e2e --source <abs worktree> --task <name> status|up|accept|...`; global options before
  the subcommand.

## 6. Reporting format

Each unit's ledger row carries: `test command`, `result`, `evidence path/run id`, `SHA`, and
`known limitations`. A claim without a command and a result is treated as unverified.
