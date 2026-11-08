# Native modeling workflow (#10)

Status: implemented as a runtime module in
`packages/mastra-runtime/src/workflows/`. This document records the stage
contract, the source-to-native adaptation and the remaining integration work.
It does not claim the #13 host validation gate.

## Stage flow

```text
inspect/freeze requirements
  -> plan / explicit decision (edit, ask, decline)
  -> ordered native apply/check rounds (bounded repair)
  -> host snapshot-validation handoff (#13)
```

The workflow owns every HostAi model step and every native tool execution. One
`ModelStep.generate` call is one #7 `model.generate` request; the next round is a
new explicit call with the assistant tool calls and their matching `tool`
messages. A returned tool call is a proposal, not proof of an edit.

| Module | Responsibility |
|---|---|
| `src/workflows/inspect.ts` | Read the bound document, selection, capabilities and fixed requirements, then freeze them before planning |
| `src/workflows/proposals.ts` | Workflow tools (`record_decision`, `apply_native_commands`, read tools, `probe_cad`) and strict proposal parsing |
| `src/workflows/budget.ts` | Separate model/ACP/CAD/repair/validation counters and the host-frozen wall clock, reserved before work |
| `src/workflows/apply.ts` | Dispatch one guarded `execute` or `batch` through #9 and keep the `WriteOutcome` facts |
| `src/workflows/checks.ts` | Fixed external check policy over exact native inspection and read-only measurements |
| `src/workflows/workflow.ts` | State machine, ordered rounds, repair and terminal mapping |

## Fixed requirements

`FixedRequirements` is supplied before the turn by the host, not by the model:

- `parameters.required` compares native parameter values with an absolute
  tolerance.
- `geometry.exact` reads a read-only exact measurement (for example `measure`
  with result pointer `/total/volume_mm3`) and applies
  `abs(actual - expected) <= max(absTolerance, relTolerance * abs(expected))`.
- Unknown check kinds are reported `unsupported` and can never pass.
- Missing, non-finite or read-failed evidence is not a pass. Display meshes are
  never used as evidence.

The model can read the requirements in its frozen prompt but cannot add,
remove, mark optional or widen them. A passing stage check is not a production
validation result.

## Decisions, writes and stale state

- `record_decision` is required before any native write. An `edit` decision
  permits planning; it is not a permission bypass. `ask` and `decline` stop
  further mutations, emit the answer text and end `task_failed` with
  `CLARIFICATION_REQUIRED` or `DECLINED`. Earlier confirmed edits remain and are
  reported as `changes: "applied"`.
- `apply_native_commands` is parsed first, every command is reserved against the
  CAD budget, then #9's adapter applies one guarded operation with
  `expectedDocument`. Batch command count includes every step.
- `probe_cad` calls #9's unsupported probe and returns
  `UNSUPPORTED_CAPABILITY` with zero writes. The workflow never edits and undoes
  the live design to simulate isolation.
- `stale` means no write occurred. The workflow re-inspects the same document,
  records the fresh stamp and requires another model round; it never retries the
  old command blindly.
- `pending` is reconciled through `pending` on the same MCP connection. An
  unresolved `pending` or `unknown` ends `outcome_unknown`; no replacement
  command or further model round is scheduled.
- `command_failed`, `expectation_failed`, rollback facts, cancellation and
  unknown effects stay distinct in the tool output. An expectation mismatch
  still reports that the model remains built.

## Budgets and terminals

The host-freezed `Budgets` are consumed by separate counters: `modelCalls`,
`agentTasks`, `cadCommands`, `repairs`, `validationRuns` and `wallTimeSeconds`.
All are reserved before the corresponding work starts. Every batch step counts.
A CAD budget shortfall refuses the batch before dispatch.

Terminal states use protocol 1.0. The result also carries a machine-readable
`reasonCode` so these remain distinct even when two map to `task_failed`:

| Situation | Terminal | `reasonCode` |
|---|---|---|
| Passing host validation handoff | `validated_success` | `VALIDATED_SUCCESS` |
| Fixed checks still fail after the repair budget | `quality_failed` | `QUALITY_FAILED` |
| Model text without a native result, ask/decline, invalid proposal | `task_failed` | `NO_RESULT`, `CLARIFICATION_REQUIRED`/`DECLINED`, `INVALID_PROPOSAL` |
| Unsupported capability or probe | `task_failed` | `UNSUPPORTED_CAPABILITY` |
| Budget or wall-clock exhaustion | `budget_exhausted` | `BUDGET_EXHAUSTED` |
| User/host cancellation | `cancelled` | `CANCELLED` |
| Unresolved/pending/unknown write effect | `outcome_unknown` | `OUTCOME_UNKNOWN` |
| Validation infrastructure failure | `task_failed` | `VALIDATION_INFRASTRUCTURE_ERROR` |

No model text can produce `validated_success`. Without the host validation
handoff, passing stage checks end `task_failed` with `VALIDATION_UNAVAILABLE`.

## Validation hand-off (#13)

`ValidationHandoff.submit` is an injected host port. The workflow sends the
current native `DocumentStamp`, the frozen profile, the fixed requirements and
the local exact check results. It consumes only the host decision:

- `passed` must still match the live stamp after the decision, otherwise the
  report is historical evidence and the turn ends `quality_failed`.
- `quality_failed`, `cancelled`, `unknown` and `infrastructure_error` map to
  their distinct terminals above.
- The workflow does not read, hash or verify report bytes, candidates,
  manifests, evidence or runner identity. That is #13's host-owned gate. A test
  stub in this repository is not a validation gate.

The runtime turn runner (`src/server.ts`) now invokes
`runNativeModelingWorkflow` when the session has a `solidcraft-app` bridge and
the host injected a CAD channel. It maps workflow stage/plan/tool/text events to
the protocol events and publishes the workflow's terminal state, `changes` and
resulting document stamp. The host still owns report verification; until #13
delivers the real `ValidationHandoff`, every passing stage check ends
`task_failed` with the validation-unavailable reason.

### Turn-runner wiring

| File | Responsibility |
|---|---|
| `src/server.ts` | Per-turn `McpClient` + `CadAdapter`, the `HostAiModelClient` seam, workflow event mapping and terminal/`changes`/document publication. A configured bridge without an injected channel fails truthfully with no write. A session without a bridge keeps the prior no-tools path. |
| `src/main.ts` | Host launch seam: `--bridge-fd <fd>` for one inherited duplex MCP stream and `--requirements-file <path>` for the host-frozen profile table. Neither changes protocol 1.0 or the schema bundle. |
| `src/bridge-lane.ts` | One reusable line lane over the inherited stream; each turn gets a fresh MCP session and closing the channel does not destroy the host transport. |

`--requirements-file` is a temporary delivery seam, not profile ownership. #13
must replace it with the host-resolved frozen profile and report gate. An empty
or missing profile list means no extra stage checks, so the workflow still ends
without validated success.

## Source-to-native adaptation record

Pinned baseline:
[`intgral-ai/intgral-vector-studio` at `acb5048120b53ae1b4502724b70d1283e8c9daf4`](https://github.com/intgral-ai/intgral-vector-studio/tree/acb5048120b53ae1b4502724b70d1283e8c9daf4).
No source bytes were imported. The source audit
([`docs/mastra-source-audit.md`](mastra-source-audit.md)) records
`query-proposal.ts`, `agent-loop/` and `model-runtime.ts` as `needs-human` and
the script executor/product runtime as rejected. This workflow is an original
independent adaptation of the roles described in contract section 1.5.

| Source path / role | Native adaptation | Semantic change |
|---|---|---|
| `apps/server/src/mastra/workflows/query-proposal.ts`: read-query → draft-script → validate-proposal | `inspect.ts` → decision tool → validated native proposal | Reads native design/parameters/selection/capabilities; no script proposal or schema-only quality pass |
| `apps/server/src/mastra/agent-loop/agent-loop.ts`: ordered model/tool rounds, decision state | `workflow.ts` ordered rounds; one explicit model call per round | Tool results are native schema results with matching tool-call IDs; the outer workflow executes them |
| `agent-loop/` read and `record_decision` modules | Read tools + `record_decision` | Ask/decline stops mutations; earlier live edits are reported truthfully |
| `agent-loop/` `run_cad` | `apply_native_commands` → #9 guarded execute/batch | `expectedDocument` guard, native command schemas, `WriteOutcome` facts; no script AST |
| `agent-loop/` `set_params` | discovered `parameters.change` through the same guarded path | Edits the existing native parameter/feature timeline; no regenerated document artifact |
| `agent-loop/` `probe_cad` | `probe_cad` returns `UNSUPPORTED_CAPABILITY` | Zero writes; no edit-then-undo isolation claim |
| `agent-loop/` `check_continuity` | `checks.ts` fixed exact stage checks | Baselines/tolerances are external and cannot be relaxed by the model |
| Passing-run adoption/publication | #13 host report gate | An old passing snapshot cannot certify later failed, cancelled or unknown edits |
| `packages/ai-runtime/src/workflows/cad-generation.ts`, `src/tools/cad-execution.ts` | Reference only for workflow/executor separation | Script inputs, hashes, remote executor and Python/smlibpy guidance are not retained |
| `model-runtime.ts` model seam | #7 `HostAiModelClient` / `ModelStep` | One buffered model call per round, no SDK retry/tool execution or provider fallback |

No product-server bootstrap, database, account or route module was adapted. The
workflow has no credentials, script runtime or durable store.

## Deliberate deviations and seams

- **Direct `HostAiModelClient`, not `createHostAiAgent`.** The turn runner calls
  #7's one-call client directly and owns every tool round. The Mastra `Agent`
  wrapper remains a compatibility seam for #7; using it here would hide the
  ordered tool-call/result identity and the exact next-call boundary this
  workflow must control. Every round is still exactly one HostAi
  `model.generate` call with no retry or hidden tool execution.
- **Prior session history is included.** The server passes the normalized
  session history (complete prior user/assistant rounds) before the current
  user turn. The workflow does not resume an old tool loop; it only supplies
  conversation context.
- **Initial binding mismatch is stale.** If the first inspection's stamp differs
  from `TurnBinding.document`, the workflow refuses before planning with
  `task_failed`/`STALE_DOCUMENT` instead of silently adopting a newer document.
  A user edit needs a new turn and a fresh binding.
- **Hello capabilities do not infer guarded writes from channel presence.** The
  bridge schema is known only after the per-turn MCP handshake, so hello keeps
  `guardedExecute`/`guardedBatch` false. The workflow verifies the actual
  discovered execute/batch schemas before any write.
- **`--requirements-file` is temporary profile delivery.** Until #13 provides
  the host-resolved frozen profile and report gate, the launch file supplies
  externally fixed checks. It is not profile ownership, and missing checks never
  become a pass.

## Tests and evidence

`test/workflow.test.ts` drives the real `McpClient` + `CadAdapter` over the
deterministic MCP peer with scripted model replies and original analytic
geometry (20 × 10 × 5 = 1000 mm³). `test/server-native.test.ts` spawns the built
`dist/src/main.js`, injects the MCP bridge on an inherited fd and the frozen
profiles on disk, and speaks real protocol 1.0 turns. `test/bridge-lane.test.ts`
covers the reusable bridge lane. Coverage includes:

- the workflow consumes #7's real `HostAiModelClient` with one explicit
  `model.generate` per round, and carries prior session history into the next
  native round;
- native create with an editable feature timeline and tool-result identity;
- one completed turn remains undoable through the ordinary native `edit.undo`;
- parameter checks read the native `document.inspect` `params` key, not the
  fixture-only `parameters` spelling;
- parameter edit with protected dimensions and the resulting document stamp;
- bounded repairs, a clear quality failure, and no validation after a failed
  check even when a passing validation stub is supplied;
- command failure with and without rollback, plus the adopted terminal stamp
  for partial failure and expectation mismatch;
- distinct model/CAD/ACP/validation/wall budgets, cancellation during a write,
  unsupported probe and unknown effects;
- stale re-inspection/replanning, the initial binding-stamp check, and a
  passing report for a changed live stamp remaining historical;
- pending reconciliation without resending the mutation;
- no LLM-only success, no success without the #13 handoff;
- ask/decline after an earlier edit, every batch command counted;
- strict ACP/apply proposal field rejection, cross-round tool-call identity,
  and a probe that unexpectedly returns still matching its tool result;
- malformed fixed requirements refused before planning;
- server create/edit/repair/cancel/ask turns and a bridge with no channel;
- bridge-lane partial-line discard between turns.

Run:

```sh
npm run typecheck -w @intgral-ai/solidcraft-mastra-runtime
npm test -w @intgral-ai/solidcraft-mastra-runtime
npm run build -w @intgral-ai/solidcraft-mastra-runtime
npm run verify:schema -w @intgral-ai/solidcraft-mastra-runtime
npm run check:deps -w @intgral-ai/solidcraft-mastra-runtime
```

These are workflow-stage and application-level server tests. They are not a
host validation gate, an installed-package test, an ACP executor test or real
panel evidence. #13 still owns snapshot report verification; #4 still owns ACP
task execution.
