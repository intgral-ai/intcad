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
| `apps/server/src/mastra/agent-loop/agent-loop.ts` and related tool/read/continuity modules | Not here; native authoring loop belongs to #10 | Not imported |
| `packages/ai-runtime/src/workflows/cad-generation.ts` and `src/tools/cad-execution.ts` | Not here; native dispatch belongs to #9/#10 | Not imported |
| `apps/server/src/mastra/services/product-generation-runtime.ts` | Not here; host sessions and durable records belong to #5/#6/#12 | Not imported |
| Provider/account/PostgreSQL/asset-service/route bootstrap | Not here | Rejected; this runtime has no database, account or route bootstrap |

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

`npm test` currently runs 31 tests. The schema suite checks **899 of 1002**
catalog cases that this layer owns and agrees with the catalog on every one. The
remaining 103 are reported, not claimed: 93 raw `guard-*` / `native-stamp-*`
carriers whose decision belongs to the Rust component adapters, and 10 host
state-machine transcripts owned by the Rust `RuntimeMachine`.

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
  executed. A text-only or tool-proposal turn ends `task_failed` with
  `changes: "none"` until the native dispatch workflow (#10) lands.
- `agent.run` is explicitly unavailable: the runtime advertises
  `hostAgentTasks: false` and returns `UNSUPPORTED_CAPABILITY` until the ACP task
  executor (#4) lands.
- Images and documents are refused before a model call. Structured output and
  streaming are refused before a HostAi call.
- A process replacement has no session, normalized history or continuation to
  resume.

## Not in this package

- Native CAD tools, guarded execute/batch and the authoring loop (#9, #10).
- ACP task execution (#4).
- Snapshot capture, local validation and report verification (#11–#13).
- Installed packaging, Node bundling and the full dependency/licence inventory
  (#14, #15).

Those tasks consume this package; they do not get a second protocol or a copied
command catalog from it.
