# SolidCraft Mastra runtime

Private npm workspace package `@intgral-ai/solidcraft-mastra-runtime`.

This package is the standalone desktop workflow subprocess described by
[`docs/mastra-runtime.md`](../../docs/mastra-runtime.md). It speaks protocol 1.0
over UTF-8 NDJSON on stdin/stdout and obtains AI capabilities from SolidCraft
HostAi. It is not a second CAD implementation, account service or validation
service.

## Requirements and exact Node/npm acquisition

- Node 22.23.4, pinned in the repository `.nvmrc` and in `engines`.
- npm 11.19.0, pinned in the repository `packageManager`.
- Run `npm ci` from the repository root. After that install, every command below
  works offline unless the lockfile changes.

The desktop host's Node 24 is intentionally **not** accepted: `.npmrc` sets
`engine-strict=true`, so `npm ci` on Node 24 fails with `EBADENGINE`. Use the
pinned line for local runs and CI. On a machine with `nvm`:

```sh
nvm install 22.23.4
nvm use 22.23.4
npm install --global npm@11.19.0
node --version   # v22.23.4
npm --version    # 11.19.0
npm ci
```

On a Nix host, `nix shell nixpkgs#nodejs_22` supplies Node 22.23.4 but currently
ships npm 10.9.9. Install the pinned npm into a temporary prefix and run its CLI
with that Node:

```sh
nix shell nixpkgs#nodejs_22 --command bash -c '
  npm install --global --prefix /tmp/npm-11 npm@11.19.0
  export PATH="/tmp/npm-11/bin:$PATH"
  node --version   # v22.23.4
  npm --version    # 11.19.0
  npm ci
'
```

#17 should reuse one of these two paths and assert both versions before the
runtime checks. Do not commit a Node binary or a generated npm prefix.

## Source adaptation record

This package is original code. No Vector Studio source file was imported from
revision `acb5048120b53ae1b4502724b70d1283e8c9daf4`; there is therefore no
per-file public-distribution authorization to wait for here.

| Baseline at the pinned revision | This package | Status |
|---|---|---|
| `packages/ai-runtime/src/models/model-runtime.ts` (`ModelRuntime.createAgent`, Mastra model seam) | `src/host-ai/language-model.ts` | Original adapter; behavior adapted from the public Mastra model interface, no source copied |
| `apps/server/src/mastra/workflows/query-proposal.ts` and `apps/server/src/mastra/agent-loop/` (read/decision, ordered tool rounds, parameter edit, continuity checks, bounded repair) | `src/workflows/` | Original independent adaptation; no source imported. The source audit keeps these records `needs-human`, so only the stage/role design is referenced |
| `packages/ai-runtime/src/workflows/cad-generation.ts` and `src/tools/cad-execution.ts` | `src/workflows/apply.ts` uses #9's native guarded adapter | The prepared-script executor and its `SCRIPT` inputs are replaced, not imported |
| `apps/server/src/mastra/services/product-generation-runtime.ts` | Not here; host sessions and durable records belong to #5/#6/#12 | Not imported |
| Provider/account/PostgreSQL/asset-service/route bootstrap | Not here | Rejected; this runtime has no database, account or route bootstrap |

The workflow's source/deviation record is
[`docs/mastra-native-workflow.md`](../../docs/mastra-native-workflow.md). No
Vector Studio bytes are present in this package.

## Commands

```sh
cd packages/mastra-runtime
npm run typecheck       # strict tsc, no output
npm run build           # clean and compile src/ and test/ into dist/
npm test                # build, then node --test on dist/test/*.test.js
npm run verify:schema   # print the conformance counts for the repository bundle
npm run check:deps      # direct production licences and ee/ import check
node dist/src/main.js   # start the built artifact on stdio
```

`npm test` currently runs 123 tests. The schema suite checks **899 of 1002**
catalog cases that this layer owns and agrees with the catalog on every one. The
remaining 103 are reported, not claimed: 93 raw `guard-*` / `native-stamp-*`
carriers whose decision belongs to the Rust component adapters, and 10 host
state-machine transcripts owned by the Rust `RuntimeMachine`.

56 of the 123 tests are the SolidCraft MCP adapter suites: a deterministic
in-memory MCP peer (`test/support/mcp-peer.ts`) supplies every guarded outcome
class, and every produced `WriteOutcome` is validated against the repository's
`#/$defs/WriteOutcome` schema rather than a TypeScript copy of it. The 30
workflow tests in `test/workflow.test.ts` drive a real `McpClient` + `CadAdapter`
over that peer with scripted HostAi replies and analytic 20×10×5 geometry. Four
`test/server-native.test.ts` cases spawn the built `dist/src/main.js` with an
inherited `--bridge-fd`, drive real protocol 1.0 turns and cover create, edit,
bounded repair, cancellation, ask and a bridge with no channel. Two
`test/bridge-lane.test.ts` cases cover per-turn session isolation and partial
line discard.

## Contract bundle

`src/protocol.ts` locates `schema/mastra-runtime/v1/` from the repository
revision, verifies every `manifest.json` size and SHA-256 digest, rejects
unlisted files and symlinks, and reads `protocol.schema.json`, `cases.json` and
the fixtures directly. There is no copied schema in this package.

The JSON Schema evaluator is locked ajv 8.20.0 with Draft 2020-12. Explicit
layers cover what JSON Schema cannot: LF framing, invalid UTF-8, duplicate keys,
nonfinite numbers, depth and frame-size ceilings, scope and direction, textual
byte caps, tool-result identity, manifest paths and decimal revision strings.

`npm run verify:schema` prints the bundle digest, total/checked/passed/skipped
counts and any mismatch.

## What is implemented

- `runtime.hello` negotiation (1.0 only), `session.start`, `turn.start`,
  `turn.cancel`, `request.cancel`, `session.reset`, `runtime.shutdown`.
- One active session and one active turn, monotonically increasing event
  sequences, bounded pending calls and cancelled-request tombstones.
- The normalized session history is bounded by both `historyMessages` and
  `historyBytes` (16 MiB). Appends are atomic; only complete old rounds are
  trimmed, and an oversized round fails explicitly without a partial append.
- Continuation handles are keyed by session history **and** the frozen
  `connectionId`/`modelId`. A different connection or model resets the branch
  and sends `continuation: null`.
- The 64 KiB control lane is reserved: data frames admit only up to
  `queueBytes - controlLaneBytes`, while cancel, shutdown, hello and terminal
  frames still use their lane under data backpressure.
- A Mastra `createWorkflow` run for each turn with an `inspect` and one `model`
  step. The model step is an actual Mastra `Agent` with `maxSteps: 1` and
  `maxRetries: 0`.
- `model.generate` as one buffered HostAi request per Agent invocation. There is
  no hidden retry, provider fallback or tool execution.
- Tool calls returned by the model are surfaced to the workflow and never
  executed by the SDK. The outer workflow owns native tool execution and the
  next explicit model call. When `session.start` has a `solidcraft-app` bridge
  and the host injected a CAD channel, the turn runner calls
  `runNativeModelingWorkflow` with #7's `HostAiModelClient` and #9's
  `CadAdapter`; stage/plan/tool/text events and the workflow's terminal,
  `changes` and resulting document stamp become the turn result.
- Native turns require a host-provisioned bridge channel. `dist/src/main.js`
  accepts `--bridge-fd <fd>` for one inherited duplex MCP stream (one MCP
  session per turn on the same lane) and `--requirements-file <path>` for the
  externally frozen check profiles. Without a configured bridge the prior
  single-step no-tools path remains. Without #13 the workflow cannot claim
  validated success; passing stage checks end `task_failed` with a clear
  validation-unavailable reason.
- `agent.run` is explicitly unavailable: the runtime advertises
  `hostAgentTasks: false` and returns `UNSUPPORTED_CAPABILITY` until the ACP task
  executor (#4) lands.
- `src/workflows/` implements the #10 native stage machine as an independently
  tested module: inspect/freeze, plan/decision, ordered guarded apply/check
  rounds, fixed check policy, bounded repair and a host validation handoff.
  It is not a second command catalog and it never turns model text into success.
  See [`docs/mastra-native-workflow.md`](../../docs/mastra-native-workflow.md).
- `src/bridge-lane.ts` wraps one host-provisioned duplex fd as reusable MCP
  line lanes, so each turn gets a fresh MCP session without closing the host
  transport. `--bridge-fd` and `--requirements-file` are launch configuration,
  not new protocol methods or copied schemas.
- Images and documents are refused before a model call. Structured output and
  streaming are refused before a HostAi call.
- A process replacement has no session, normalized history or continuation to
  resume.
- `src/solidcraft/mcp-client.ts` connects to the host-injected
  `solidcraft-app` bridge, performs the MCP handshake, discovers tools through
  `tools/list` and each command's schemas through `describe_command`, validates
  arguments against those schemas, keeps numeric transport ids bijectively
  mapped to protocol `Id`s, and exposes the existing `pending`/`running` query
  on the same connection.
- `src/solidcraft/cad-adapter.ts` serializes one guarded mutation per turn,
  bounds every MCP read (including `describe_command` discovery) at four shared
  slots and serializes reads behind a mutation, binds `expectedDocument` at the
  execution envelope, and normalizes every outcome to the contract's section 7
  `WriteOutcome` (`applied`, `command_failed`, `expectation_failed`, `pending`,
  `stale`, `cancelled`, `unknown`). It reconciles a still-running write through
  `pending` on the same connection without resending it, reports `unknown` after
  a disconnect, refuses further writes until reconciliation, and returns
  `UNSUPPORTED_CAPABILITY` with zero writes for a speculative candidate probe.
  A definite no-write failure (`command_failed` or a pre-send `cancelled`)
  reports `after` equal to `before`; `stale` carries the app's current stamp (or
  null when closed) and is never adopted automatically. 1.0 has no in-turn
  escape from `unknown`: writes stay refused, the turn ends `outcome_unknown`,
  and the user starts a new turn and re-inspects.

## Host bridge provisioning contract (open #9 follow-up, not in this PR)

This contract is not implemented by merged #25 and is not part of this PR. The
open #9 remains its owner; it is a follow-up on #9, not a new issue.
`src/solidcraft/mcp-client.ts` receives the bridge as an already-open channel,
not a URL or a token it resolves itself. The host side must:

- resolve `session.start`'s `BridgeRef {name: "solidcraft-app", bridgeId}` to a
  channel bound to **this turn's** bridge (a fresh `solidcraft-cli mcp` bridge or
  an equivalent in-process channel). The channel is the same MCP connection for
  the whole turn, so the app's `pending` slot belongs to it;
- inject the trusted turn association and cancellation flag. The runtime never
  sends `sessionId`, `turnId`, `workflowRunId` or `expectedDocument` from model
  arguments; `CadAdapter` rejects those keys inside CAD params;
- wire `turn.cancel` to the `AbortSignal` passed in `CadTurnAssociation.signal`
  (write admission closes independently of the runtime);
- advertise `guardedExecute`/`guardedBatch` in `runtime.hello` once this
  provisioning exists. Until then the runtime's hello reports both `false`.

The channel shape is `McpBridgeChannel` (`send`, `receive`, `close`) or
`streamChannel(readable, writable)` for a child process's pipes. A bridge that
lacks `describe_command`, a usable command schema or the guarded
`expectedDocument` field is refused rather than used unguarded.

## Not in this package

- The native authoring workflow, stage checks and repair scheduling (#10).
- Host-side bridge provisioning and the chat tool-event/undo projection:
  resolving `session.start`'s `BridgeRef` to a per-turn channel, injecting the
  trusted turn/cancel association, projecting the adapter's evidence and stamps
  through the existing chat tool events and undo boundary, and advertising
  `guardedExecute`/`guardedBatch` in `runtime.hello`. The open #9 remains the
  owner of this follow-up; merged #25 does not provide it.
- ACP task execution (#4).
- Snapshot capture, local validation and report verification (#11–#13).
- Installed packaging, Node bundling and the full dependency/licence inventory
  (#14, #15).

Those tasks consume this package; they do not get a second protocol or a copied
command catalog from it.
