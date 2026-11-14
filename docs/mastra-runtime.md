# Mastra workflow hosting and validation contract

**Status:** design contract 1.0 for [T01 / #2](https://github.com/intgral-ai/open-skyfall-desktop/issues/2),
under [the workflow epic](https://github.com/intgral-ai/open-skyfall-desktop/issues/1).
The code baseline is `06749ac464e16357d0375b78e6b3cac214066493`.
The modules and wire methods below are proposed unless the baseline table says otherwise.
This document specifies interfaces; it does not announce an installed runtime.

The product owner approved the narrow desktop TypeScript distribution exception,
live modeling with validation-gated verified success, and keeping the desktop host
and runtime source in one repository. The owner also selected adapting Vector
Studio's existing orchestration and multi-round authoring to native SolidCraft
commands, parameters and feature history (section 1.5).
The remaining implementation choices below are T01's selected design, not additional
claims of product-owner approval. Dependent tasks implement this contract and link
their deviations here for review, rather than defining local protocol variants.

## 1. Product and distribution decisions

### 1.1 Approved exception and package ownership

The desktop distribution may contain a standalone Mastra runtime, its TypeScript
workflow and client adapters, a pinned Node executable, and the necessary production
dependencies. This is an explicit exception to the Rust-only distribution policy;
putting code in a subprocess or another repository is not, by itself, an exception.
The exception does not relax independence, licensing, credentials or document rules.

The runtime source lives in this desktop repository under `packages/mastra-runtime/`,
as the private npm workspace package `@intgral-ai/solidcraft-mastra-runtime`.
Workspace and build integration belong to
[#7](https://github.com/intgral-ai/open-skyfall-desktop/issues/7). The Rust Cargo
workspace and TypeScript package workspace share the repository, pull requests and
release source revision. The runtime remains a separate process and build artifact.
`intgral-ai` owns the repository and releases; desktop maintainers own the host
contract and the runtime versions admitted by packaging. Packaging binds host,
runtime and contract artifacts to the same reviewed repository revision while
retaining explicit runtime and protocol versions. Original runtime code uses the
desktop project's `MIT OR Apache-2.0` license.

The native host, engine, MCP server and local validation runner remain Rust. The
runtime's TypeScript clients live together under `src/host-ai/`, `src/solidcraft/`
and `src/validation/`; workflows live under `src/workflows/`. They are not separate
account, CAD or validation implementations. The native crates still compile for
wasm; desktop process hosting is gated with `cfg(not(target_arch = "wasm32"))`.
The browser build advertises this runtime as unavailable and has no Node dependency.

[#15](https://github.com/intgral-ai/open-skyfall-desktop/issues/15) must apply the
approved exception consistently to policy guidance and artifact/license enforcement
before #14 bundles it. Until that lands, the existing Rust-only rule still applies
to application implementation in this repository. T01 itself adds documentation only.

### 1.2 Reuse and provenance

| Source | Reuse allowed by this design |
|---|---|
| This repository's `agent_chat/api_client.rs`, `acp.rs`, `acp_client.rs`, `process.rs`, `discover.rs` | Extract/reuse Rust model invocation, ACP session mechanics, authentication and process lifecycle. Preserve provider behavior and original connection identities. |
| This repository's MCP, engine commands, schemas, native design writer and safe writes | Reuse existing Rust implementations through their interfaces; discover command schemas instead of copying a TypeScript command catalog. |
| Mastra's public workflow SDK | Use permissively licensed entry points after the exact locked release and distributed files pass #15's inventory. No enterprise modules. |
| Vector Studio authoring workflow and agent loop | Selected orchestration baseline at the fixed revision in section 1.5. Adapt its stages and tool-turn logic to native modeling. Source imports require the per-file rights and policy record below; this is not a blanket license for the product server. |
| Vector Studio provider/account/database/route bootstrap, script executor and script-specific prompts/skills | Replace with the desktop ports and native command guidance described in section 1.5. These implementations are outside the selected migration. |

The source repository is private; this desktop repository is public. No root source
license was identified during the source audit. Before copying or adapting source
files into this public repository, #15 must record the exact source path/revision,
rights-holder authorization for public distribution, applicable license, required
notices and transitive dependencies, and reconcile the independence policy for this
specific migration. Organization ownership and the option-A decision alone do not
establish every file's redistribution rights. Imported files retain their applicable
notices; the project's dual license is not automatically applied to them. The source
map below is a migration specification, not a claim that private code has been
published or is already licensed here. This documentation change imports no code.

The [Mastra root license](https://github.com/mastra-ai/mastra/blob/main/LICENSE.md)
was checked during T01: it assigns Apache-2.0 to eligible content and excludes every
`ee/` directory, including areas inside `@mastra/core`. Therefore a package-level
license string is insufficient. The delivered artifact must exclude those modules,
not merely avoid calling them. #7 locks the actual dependency versions; #15 checks
the full shipped dependency/file inventory, Node notices and pending attributions.
Cargo's license check alone does not cover this TypeScript artifact.

### 1.3 Installed targets and Codex

| Initial runtime target | Installed execution dependency | Verification owner |
|---|---|---|
| Windows x86_64, MSVC desktop packages | Matching pinned `node.exe` and runtime artifact | #14 |
| macOS aarch64 and x86_64, each slice of the universal app | Matching pinned Node executable and runtime artifact | #14 |
| Linux x86_64, glibc desktop packages | Matching pinned Node executable and runtime artifact | #14 |

Windows x86/arm64, Linux arm64, FreeBSD and wasm do not advertise the built-in
Mastra workflow in 1.0. This is a runtime feature matrix, not a change to the
application's broader release matrix. A platform is supported only after an
installed-package test succeeds on that platform. Sandboxed Linux formats must
test their packaged executable, artifact paths and allowed network/filesystem
access; availability in an ordinary tarball does not prove Flatpak or Snap support.

Use an application-relative `mastra/` resource directory containing `runtime.json`,
the JS entry point, production dependencies and `node`/`node.exe`. `runtime.json`
records the runtime version, protocol versions, target triple, Node version,
repository source revision, contract digest, artifact digests and provenance inventory.
The host resolves resources from the
installed application, not the development checkout, a shell, `npx` or the user's
Node installation. Missing/mismatched files fail before a session starts.
The installed locations, diagnostics and current unapproved-artifact gate are in
[`mastra-packaging.md`](mastra-packaging.md).

Codex ACP is **discovered, not bundled**, in 1.0. Reuse `discover.rs` to locate the
user-installed `codex-acp`, probe its actual execution dependencies and version,
and reuse its own authentication flow. A clean machine can use the bundled runtime
with BYOK without installing Node. Codex mode additionally requires a functioning
external Codex ACP installation; missing executable, incompatible dependency and
authentication-required are separate states. Desktop bundling of Codex would be
a separate distribution decision, not implied by the Mastra exception.

### 1.4 What validation controls

Validation gates **the turn's `validated_success` outcome**. Modeling edits the live
document through the existing chat bridge and retains per-turn undo. A failed or
cancelled turn can have completed edits; its outcome must say so. Validation does
not automatically undo them. The host owns this gate, including the final report
lookup and identity checks; an LLM's text cannot satisfy it.

Neither document adoption nor export/publication is a production gate in 1.0.
A native snapshot freezes validation input; it does not isolate earlier live edits.
An undo checkpoint is not an isolated candidate workspace. Validation-before-adoption
would require separate candidate-construction and atomic host-adoption tasks;
publication gating would require enforcement at the host's export/publication entry
points. These guarantees are not prerequisites or promises of this delivery.

The independently deliverable foundation is #11's immutable snapshot plus #12's
local runner, callable without Mastra. Remote CI vendors, deployment credentials
and production-specific profiles are subsequent integrations. Here, “validation”
means checks on a design candidate; repository `cargo xtask ci` is a different gate.

### 1.5 Vector Studio orchestration adapted to native modeling

The selected source baseline is
[`intgral-ai/intgral-vector-studio` at `acb5048120b53ae1b4502724b70d1283e8c9daf4`](https://github.com/intgral-ai/intgral-vector-studio/tree/acb5048120b53ae1b4502724b70d1283e8c9daf4).
That link requires source-repository access. Paths in the table are relative to
that revision; their descriptions make the intended desktop behavior reviewable
without access to the private source. #7 and #10 record migrated paths and semantic
deviations against this baseline, rather than silently following its moving branch.

| Source module / responsibility | Desktop adaptation and owner |
|---|---|
| `apps/server/src/mastra/workflows/query-proposal.ts`: `read-query → draft-script → validate-proposal`, optionally using the agent loop | #10 keeps staged authoring, explicit decisions and bounded model/tool rounds. Read the frozen turn and native design; plan native operations; apply/check them; then request snapshot validation through #13. Source proposal validation checks script/parameter shape, not this contract's design-quality gate. |
| `apps/server/src/mastra/agent-loop/agent-loop.ts` and its tool, read, parameter-edit and continuity modules | #10 adapts ordered tool calls, matching results, decision state, distinct budgets, cancellation and unknown outcomes. #9 supplies native reads and guarded writes; the tool mapping below replaces script execution and candidate adoption. |
| `packages/ai-runtime/src/models/model-runtime.ts`: `ModelRuntime.createAgent`, model interface and lifecycle | #7 provides a desktop Agent factory backed by HostAi at the Mastra model seam. The source factory is not a direct `model.generate` client. Keep compatible Agent/tool-call behavior; replace provider routing, admission and trace storage with desktop scope/capability/budget handling. |
| `packages/ai-runtime/src/workflows/cad-generation.ts` and `src/tools/cad-execution.ts`: prepared-script execution using an injected `CadExecutionPort` | Use the separation of workflow and injected executor as a reference. This wrapper is not the complete authoring loop. Replace SCRIPT inputs, script hashes and the remote executor with #9's discovered native schemas and `WriteOutcome`; no Python/script-execution dependency is introduced by this migration. |
| `apps/server/src/mastra/services/product-generation-runtime.ts`: admission, claims, product persistence, replies and recovery | #5/#6 bind runtime sessions to host turns and cancellation; #7 owns session-local normalized history; #12 owns durable validation records. Replace PostgreSQL, account/quota services, product routes and publication with these ports. Do not import the product-server bootstrap. |
| Source transcript/trace ports and CAD skills | #7/#10 preserve model-call/tool-result correlation and stage diagnostics under host IDs. Use native command schemas and native modeling guidance instead of script prompts or `anatomy-smlibpy`. Persisted desktop projections remain host-owned as described below. |

#### Model and conversation seam

The runtime's Agent adapter must expose the capabilities the migrated authoring
stages actually consume. A BYOK Agent invocation performs **one model step** through
`HostAi.model.generate`; the outer workflow validates and dispatches returned tool
calls and feeds their matching results into the next explicit step. Override SDK
multi-step/retry defaults. Neither the SDK nor HostAi may execute another hidden
CAD loop, model retry or provider fallback. Each actual invocation consumes the
section 4 budget and keeps the frozen connection/model. #7 verifies this with an
actual Mastra Agent using deterministic HostAi replies, not only a mock factory.

Adapt the locked SDK's messages, tool IDs/arguments, finish reason and available
usage to section 6. Preserve opaque continuation handles without inspecting their
provider state. A buffered reply may be wrapped in the SDK's stream shape for
compatibility, but must remain advertised as buffered. Missing tool/structured/image
capabilities fail explicitly. `agent.run` is a separate proposal-only ACP operation;
it is not a LanguageModel implementation or a way around the workflow's tool gate.

The runtime holds normalized conversation, tool results, decision state and repair
accounting within the active session, bounded by section 4. Stage/text/plan/tool
events in section 5 project progress to the existing host chat history. They retain
session/turn/workflow/request identities and redact credentials/provider-private
state. The host's bridge outcomes and validation records remain authoritative.
Version 1.0 adds no durable workflow/transcript resume service: process replacement
retires the session and its continuation handles, while host chat projections and
durable candidate/run/report records can remain available. Reading such records
does not resume an old tool loop or authorize replay of an unresolved write.

#### Native tool and outcome mapping

The source tool names below explain migration semantics; they are not new SolidCraft
engine commands or an extra wire protocol. #10 builds its workflow-facing tools over
#9's discovered command/schema interface.

| Source concept | Required native behavior |
|---|---|
| Read tools and `record_decision` | Read the bound document, selection, capabilities, fixed requirements and exact baselines. An edit decision permits planning, not permission bypass. Ask/decline stops further mutations; any earlier live edits are reported and retain undo. |
| `run_cad` | Validate native command parameters, capabilities and source stamp, then apply through guarded execute/batch. Preserve command failure, rollback, expectation mismatch, pending and unknown facts. A returned tool call is a proposal, not proof of an applied edit. |
| `set_params` | Resolve existing native parameters/features and use discovered parameter-change commands. Recompute/inspect and check protected dimensions; do not replace the document with a regenerated script artifact. |
| `probe_cad` | Inspection of the current design may be read-only. A speculative candidate-building probe has no equivalent in 1.0: return `UNSUPPORTED_CAPABILITY` before writes. Never simulate isolation by editing and undoing the live design. Such a probe needs a separately designed isolated workspace. |
| `check_continuity` | Verify fixed protected dimensions/requirements using exact native evidence bound to the relevant document stamp or snapshot. Missing evidence cannot count as a pass. The model cannot change the baseline or tolerances; #13 carries these checks into the required profile. |
| Passing-run selection and publication | Replace source adoption with host-verified `validated_success` for the matching current snapshot. A previously passing candidate cannot approve later edits, a cancelled turn or an unknown mutation. No fallback adoption/publication of an earlier run is implied. |

The desktop stage flow is inspect/freeze requirements → plan/decision → ordered
native apply/check rounds → capture immutable candidate → submit/status/report →
host verification. A quality failure may return to planning within the repair budget,
with a new candidate/run for every changed design. #10 supplies native stage checks;
#11–#13 supply the snapshot gate. An LLM end-of-turn or a source proposal-schema pass
alone never produces verified success.

Preserve the source loop's intent to reserve time and tool-result capacity before
starting work, using the negotiated desktop limits rather than copying its server
timeout constants. Model calls, ACP tasks, native command applications, repairs and
validation submissions are distinct counters, including batch command applications.
Cancellation is checked before dispatch and after replies; host cancellation closes
write admission independently. Unknown write effects stop further modeling until
the original request is reconciled (section 7); do not run a replacement command.
Source stop labels map to section 5's existing terminal states and truthful `changes`
facts. Budget exhaustion, failure or cancellation never selects an old passing run
as a substitute for the host gate.

For this modeling workflow, ask/decline or a text-only answer without a result to
validate emits the answer text and proposes `task_failed`, with a reason explaining
that clarification is needed, the task was declined, or no modeling result was
produced. It does not start validation just to finish the conversation. Earlier
edits are reported as `changes: "applied"` when confirmed; undo remains available.
Cancellation, transport loss and unresolved write effects take their corresponding
terminal states instead. These outcomes do not add a conversational-success state
to protocol 1.0.

## 2. Modules, ownership and existing guarantees

| Module | Interface and ownership |
|---|---|
| `MastraBackend` (Rust, `ui-egui/agent_chat`) | Implements `ChatBackend`; freezes workflow/connection/model/turn binding, maps events to the existing panel, injects the chat bridge and delegates to host services. Calls never block an egui frame. |
| `HostAi` (Rust, proposed `agent_chat/host_ai`) | Resolves the original `AgentConfig.id`, uses injected `Secrets`/`HttpFn`, owns provider-private continuation data and downstream ACP sessions. Provides one model call or one agent task, without a CAD tool loop. |
| Standalone Mastra runtime (TypeScript) | Owns normalized workflow conversation, stages, plan and bounded repair scheduling. Calls host capabilities and the injected MCP bridge. No key store, provider-account bootstrap or direct document implementation. |
| SolidCraft MCP adapter (TypeScript client, existing Rust server) | Discovers tools/schemas, serializes guarded mutations, preserves original MCP request identity and normalizes real results. The Rust host owns permission decisions, cancellation enforcement and undo. |
| `ValidationPort` (Rust host service, TypeScript client) | Owns snapshot references, idempotent job admission, local-runner lifecycle, durable run/report records and report verification. The CLI runner checks frozen inputs without editing the live document. |

Host orchestration stays at the app/`ui-egui` layer, using injected services. Native
snapshot capture reaches engine commands and `io`; CLI loading/checking uses those
lower layers without depending on UI. No lower crate depends on Mastra or egui.

### Baseline evidence and proposed guarantees

| Baseline fact | Contract addition and implementation owner |
|---|---|
| [`backend.rs`](../crates/ui-egui/src/agent_chat/backend.rs) defines `ChatBackend`, normalized prompt parts, `Secrets` and buffered `HttpFn`. | #3/#6 expose shared calls and Mastra panel adaptation without a new account identity. |
| [`api_client.rs`](../crates/ui-egui/src/agent_chat/api_client.rs) owns a provider history/tool loop and private Gemini signatures; cancelled HTTP work may continue until timeout while its answer is dropped. | #3 extracts one-call invocation, continuation handles and bounded abandoned-call accounting. It does not claim physical HTTP interruption or incremental tokens. |
| [`acp_client.rs`](../crates/ui-egui/src/agent_chat/acp_client.rs) owns ACP process/session/authentication and a five-second cancellation fallback. | #4 gives workflow tasks distinct ACP sessions and proposal-only CAD authority. |
| [`agents.rs`](../crates/ui-egui/src/agent_chat/agents.rs) preserves configured connection IDs; [`mod.rs`](../crates/ui-egui/src/agent_chat/mod.rs) currently treats `Event::Ready` as signed in for `running_id`. | #6 separates execution choice, runtime readiness and original-connection authentication. |
| [`process.rs`](../crates/ui-egui/src/agent_chat/process.rs) keeps I/O off-frame and shuts down the process tree; stdout lines have an 8 MiB cap. Its channels and oversized-line draining are not an end-to-end resource bound. | #5 reuses lifecycle behavior while enforcing the limits in section 4, including queues and stderr reading. |
| [`document.revision`](../crates/engine/src/cmd/inspect.rs) returns `document_id` and a process-local `u64` revision. The stamp reflects observed design/visibility/pending-position state; an edit undone before inspection need not produce a new stamp. | #8 adds atomic expected-document checks at application time. A read followed by a write is not already protected. |
| [`tools.rs`](../crates/mcp/src/tools.rs) distinguishes batch rollback facts, expectation mismatches and still-running requests. An expectation mismatch leaves the model built. | #9 preserves those distinctions and never retries an unresolved mutation. |
| [Control pending results](control-protocol.md) belong to their original connection; a new connection has no old pending slot. | #9 can reconcile a retained connection. A lost connection may leave an unknown outcome; 1.0 does not promise a durable CAD execution journal. |
| Native design serialization and per-turn undo exist. | #11/#12/#13 add immutable manifests, asynchronous validation and report-bound verified success. Neither snapshots nor isolation/production gates already exist. |

## 3. Protocol 1.0

### 3.1 Encoding, negotiation and identities

Transport is full-duplex UTF-8 newline-delimited JSON on a child process's stdin/stdout.
Stdout contains frames only; stderr contains bounded diagnostic text. A frame is one
JSON object and one LF. Reject invalid UTF-8, duplicate object keys, nonfinite numbers,
excess depth, oversized frames and invalid envelopes. No compression or executable
payloads. A protocol violation closes the transport and resolves outstanding calls
with `TRANSPORT_ERROR`; it does not authorize replay of writes.

The stable bootstrap request is `runtime.hello`. The host advertises supported
versions, and the runtime selects the highest exact common version. Version 1.0
supports only `"1.0"`. No common version returns `INCOMPATIBLE_VERSION` and exits
before starting a session. Bootstrap parsing accepts the envelope's version string
to negotiate/refuse it; all later frames must use the selected version. New required
fields or changed semantics require a new negotiated version. Unknown fields are
rejected except within the explicitly optional `extensions` object; extensions
cannot affect 1.0 semantics. Unknown methods return `METHOD_NOT_FOUND`.

| Name | Required representation and meaning |
|---|---|
| `Id` | ASCII string matching `[A-Za-z0-9][A-Za-z0-9._:-]{0,127}`; never a credential or filesystem path. Production generators use random 128-bit identities. |
| `Digest` | `sha256:` followed by exactly 64 lowercase hex digits. |
| `Timestamp` | UTC RFC 3339 string with millisecond precision, such as `2026-11-05T12:00:00.000Z`. Ordering and deadlines use a monotonic clock within a process. |
| `Revision` | Canonical decimal string for an unsigned 64-bit integer: exactly `0`, or a nonzero digit followed by digits, at most `18446744073709551615`. Reject leading zeros except the single `0`. Rust parses `u64`; TS preserves the string or uses `bigint`, never `Number`. |
| Other integers | Nonnegative JSON integers at most `2147483647`, with the stricter field limits below. |
| `runtimeInstanceId` | Runtime-generated at each process start; returned by hello. A transport is tied to exactly this instance. No session resumes on a replacement process. |
| `sessionId` | Host-generated conversation identity. Reset creates a new one. |
| `turnId` | Host-generated user-message identity; one active turn per session. |
| `workflowRunId` | Host-generated workflow execution identity for that turn, preserved across its repairs. |
| Envelope `requestId` | Sender-generated call identity, prefixed `h:` or `r:` by sender. Unique for the transport lifetime. A response echoes it; neither direction can resolve the other's requests. |
| Validation `runId` | Host-generated durable identity for one candidate/profile validation job. Different from `workflowRunId`, ACP session IDs and transport request IDs. |

Every envelope has `protocolVersion`, `kind`, `scope`, and its kind-specific fields.
`scope` is exactly `{sessionId: Id|null, turnId: Id|null, workflowRunId: Id|null}`.
Hello/shutdown use three nulls; session calls use only `sessionId`; turn-scoped calls
use all three IDs. A partial turn scope is invalid. The bound document, connection
and model are in the turn's immutable binding, not inferred from the active tab.

| `kind` | Additional required fields |
|---|---|
| `request` | `requestId`, `method`, `params` (object). |
| `response`, `outcome: "ok"` | `requestId`, `outcome`, `result` (object). Echo the request's scope. |
| `response`, `outcome: "error"` | `requestId`, `outcome`, `error: Error`. No `result`. Echo scope. |
| `event` | `sequence`, `event`, `requestId: Id|null`, `data` (object). `requestId` names a causative request when one exists. |

`extensions?: object` is the only optional common field. All objects defined here
reject undeclared fields. Event sequence starts at 1 and increases per sender for
the transport lifetime. Duplicate or decreasing sequence is a protocol error;
exhaustion requires orderly shutdown rather than wraparound. Events from retired
scopes and late responses to explicitly cancelled calls are discarded. An otherwise
unknown response ID is a protocol error. Cancelled-ID tombstones count against a
bounded ledger; exhaustion restarts the runtime rather than forgetting identities.

The host and runtime must continue reading and dispatching callbacks while awaiting
a response. For example, a pending host `turn.start` cannot block the runtime's
`model.generate`. Never hold the document lock or an IPC dispatch lock while waiting
for external I/O. Backpressure errors are explicit, never silent frame loss.

### 3.2 Errors

`Error` has required `code`, `message`, `retry` and `details`. `message` is redacted
human-readable text (at most 2,048 UTF-8 bytes). `retry` is `never`, `read_only` or
`same_submission`; it is advice for the defined operation, not permission to repeat
a mutation. `details` is either `{}` (permitted for every code), or a nonempty
object containing the fields specified for that code below. In a nonempty object,
fields without `?` are required and fields marked `?` may be omitted. Undeclared
fields are rejected. Error handling must work without diagnostic details.

| Code | Meaning / permitted detail fields |
|---|---|
| `INCOMPATIBLE_VERSION` | `supported: string[]`; no compatible handshake. |
| `INVALID_REQUEST`, `METHOD_NOT_FOUND` | `field?: string`, `method?: string`; invalid shape or method. |
| `UNSUPPORTED_CAPABILITY` | `capability: string`; includes unsupported guarded mutation paths. |
| `AUTH_REQUIRED` | `connectionId: Id`; the original connection owns sign-in. |
| `CONNECTION_CHANGED` | `connectionId: Id`; deleted or changed frozen endpoint/configuration. |
| `PERMISSION_DENIED` | `permissionId: Id`; host user denied a request. |
| `MODEL_ERROR`, `MALFORMED_RESPONSE` | `connectionId: Id`, `httpStatus?: integer`; provider failure or invalid response. |
| `TASK_FAILED`, `INVALID_PROPOSAL` | `connectionId: Id`; ACP failure or unusable returned proposal. |
| `STALE_DOCUMENT` | `expected: DocumentStamp`, `current: DocumentStamp|null`; no guarded write occurred. |
| `DEPENDENCY_CHANGED`, `INVALID_CANDIDATE` | `candidateId?: Id`, `dependencyId?: Id`; input integrity/capture failure. |
| `PROFILE_UNSUPPORTED`, `REPORT_MISMATCH` | `runId?: Id`, `profileId?: Id`; cannot prove the required checks. |
| `NOT_READY`, `NOT_FOUND` | `runId?: Id`, `artifactId?: Id`; no report yet or absent/expired reference. |
| `IDEMPOTENCY_CONFLICT`, `REQUEST_EXPIRED` | `submissionRequestId: Id`; different payload for a key, or expired key. |
| `RESOURCE_EXHAUSTED` | `resource: string`, `limit: integer`; admission refused before new work. |
| `TIMEOUT`, `CANCELLED`, `TRANSPORT_ERROR`, `OUTCOME_UNKNOWN`, `INTERNAL_ERROR` | `operation?: string`; timeout, cancellation, transport, unresolved effect or internal failure. |

Authentication-required, transport-error and task-failed are separate outcomes.
Unexpected provider finish reasons map to `unknown`, never silently to normal
completion. No error contains keys, authentication headers, raw credentials or
unbounded provider bodies. Unknown future error codes cannot produce success.

## 4. Finite resources and cancellation

These are 1.0 hard ceilings; the host may negotiate lower limits in hello. They are
per runtime/session unless “host” is stated. No user/model-supplied budget can raise
them. Adapters validate before allocation and before scheduling work.

| Resource | Ceiling / behavior |
|---|---|
| IPC frame / JSON nesting | 8 MiB including LF / 64 nested containers; abort oversized input without accumulating its remainder. |
| Queued IPC bytes | 16 MiB per direction, including a reserved 64 KiB control lane for cancel/shutdown; overflow refuses admission. |
| Outstanding calls | 32 per direction; cancel/shutdown retain one reserved slot in addition. |
| Cancelled/completed request ledger | 4,096 IDs per transport; rotate the process at an idle boundary before exhaustion. |
| Runtime instances / sessions / turns | 8 per host / 1 active session per runtime / 1 active turn per session. |
| Stderr | 40 retained lines, 400 Unicode scalars per line, 64 KiB total; cap reads before newline and discard excess incrementally. |
| Message / tool lists | 128 model messages, 64 tool definitions, 32 returned tool calls; oversize is an error, not silent truncation. |
| Text / tool schema or arguments | 256 KiB per text block / 64 KiB per tool schema or argument object; total frame cap also applies. |
| Attachments | 4 per call, 4 MiB decoded each, 16 MiB total; transferred as host artifact references. |
| Workflow memory | 16 MiB normalized history and 128 messages; trim only complete user/tool rounds or fail explicitly. |
| Continuation storage | 32 handles, 16 MiB total per session; expire after 24 hours or session retirement. |
| Physical host HTTP calls | 2 per connection, 8 per host, including logically cancelled calls still in flight. No replacement beyond the cap. |
| Turn work | 20 model calls, 4 ACP tasks, 64 CAD command applications, 2 repairs total and 3 validation submissions; 1,200 s wall time. |
| CAD scheduling | 1 unresolved mutation per turn; at most 4 concurrent reads, with reads serialized behind that mutation when required by the bridge. |
| Handshake / session start | 10 s / 30 s; failure ends the attempted startup. |
| Model call / ACP task | Default 300 s; minimum of configured timeout, 600 s and remaining turn time. |
| Cancel acknowledgement / forced process stop | 5 s to acknowledge; then invoke existing process-tree shutdown, bounded to 3 s. |
| Validation workers / waiting jobs | 2 / 8 per host; `RESOURCE_EXHAUSTED` before admission beyond the queue. |
| Validation execution | 600 s from worker start; queued admission expires after 300 s. |
| Snapshot | 1 GiB aggregate persisted input, 256 inputs, 1 MiB manifest; unresolved/excess dependencies fail capture. |
| Reports / evidence | 4 MiB report, 128 checks, 64 evidence artifacts, 16 MiB per evidence artifact, 64 MiB aggregate evidence per run. |
| Artifact store / job records | 8 GiB / 10,000 records per host; reject new work if safe cleanup cannot make room. |
| Retention | Completed job inputs/reports/evidence and idempotency mappings retained for 7 days; active jobs pin inputs. No eviction of active references. |

Defaults for a turn equal the listed turn-work ceilings. A workflow may request lower
limits in `turn.start`; the host freezes the admitted values. Model requests, ACP
delegations, CAD applications, repairs and validation attempts are separate counters.
An ACP delegation limit says nothing about that external agent's hidden model calls.
Retries consume their corresponding budgets. An empty or malformed proposal cannot
consume an unbounded repair loop. Infrastructure reconciliation does not consume a
geometry repair, but remains bounded by time and validation-attempt limits.

The host owns the trusted turn cancellation flag. On Stop, reset, turn deadline or
runtime loss, it first closes write admission for that turn, then cancels outstanding
HostAi work, bridge work and validation jobs, and notifies the runtime. The flag is
checked again at actual document application, including deferred work. Runtime
cooperation is not the enforcement mechanism.

A write already committed before cancellation remains an edit. An admitted but
unapplied mutation must be refused after cancellation. If its result cannot be
established, report `unknown` rather than promise rollback. The host does not start
another mutation in that document until reconciliation/re-inspection has resolved
the ambiguity. A later validation report can be retained as history but cannot
promote a cancelled or failed turn to success.

For buffered HTTP, logical cancellation drops the late result and prevents tool
execution; the physical request may continue until its host timeout. It keeps its
physical-call slot until exit. ACP uses `session/cancel`, cancels open permissions,
then the process-tree fallback. Validation cancellation persists a cancel marker
before signaling the worker. Shutdown closes input, cancels work, drains bounded
terminal replies and reaps the owned process tree; it never blocks an egui frame.

## 5. Lifecycle, states and events

All listed fields are required unless marked `?`. Direction H→R means host to
runtime; R→H means runtime to host. Calls use the envelopes in section 3.

| Method / scope / direction | Parameters | Successful response |
|---|---|---|
| `runtime.hello` / none / H→R | `supportedVersions: string[]`, `hostVersion: string`, `limits: Limits` | `selectedVersion`, `runtimeInstanceId`, `runtimeVersion`, `capabilities: RuntimeCapabilities`, `limits: Limits` |
| `session.start` / session / H→R | `workflowId: Id`, `instructions: string`, `bridge: BridgeRef|null` | `state: "ready"`, `workflowId` |
| `turn.start` / turn / H→R | `binding: TurnBinding`, `parts: ContentBlock[]`, `context: TurnContext`, `budgets: Budgets` | `state: "accepted"`; completion is a later `turn.finished` event. |
| `turn.cancel` / turn / H→R | `reason: "user"|"reset"|"deadline"|"shutdown"` | `state: "cancelling"|"terminal"`; repeated cancel is idempotent. |
| `request.cancel` / same scope as target / either | `targetRequestId: Id` (a call originated by this sender) | `state: "cancelling"|"terminal"`; target receives one terminal response if transport survives. |
| `session.reset` / old session / H→R | `nextSessionId: Id`, `instructions: string` | `state: "ready"`, `sessionId: nextSessionId`; retires old turn, history and continuation, keeps workflow/bridge configuration. |
| `runtime.shutdown` / none / H→R | `reason: "app_exit"|"reconfigure"|"protocol_error"` | `state: "stopping"`, followed by EOF. |
| `connection.describe` / session or turn / R→H | `connectionId: Id`, `modelId: string` | `state: "available"|"auth_required"|"unavailable"`, `capabilities: AiCapabilities`, `authMethods: AuthMethod[]` |

`Limits` contains all IPC/resource ceilings above as named camelCase fields.
All are integers except `storeBytes`, as specified below. The normative names are:
`frameBytes`, `jsonDepth`, `queueBytes`, `controlLaneBytes`,
`pendingRequests`, `controlSlots`, `requestLedgerEntries`, `runtimeInstances`,
`sessions`, `activeTurns`, `stderrLines`, `stderrLineScalars`, `stderrBytes`,
`modelMessages`, `toolDefinitions`, `toolCalls`, `textBytes`, `toolJsonBytes`,
`attachments`, `attachmentBytes`, `attachmentTotalBytes`, `historyBytes`,
`historyMessages`, `continuationHandles`, `continuationBytes`, `continuationTtlSeconds`,
`httpPerConnection`, `httpPerHost`, `cadWritesInFlight`, `cadReadsInFlight`,
`handshakeSeconds`, `sessionStartSeconds`, `callDefaultSeconds`, `callMaxSeconds`,
`cancelAckSeconds`, `shutdownSeconds`, `validationWorkers`, `validationQueue`,
`validationSeconds`, `validationQueueSeconds`, `snapshotBytes`, `snapshotInputs`,
`manifestBytes`, `reportBytes`, `checks`, `evidenceArtifacts`, `evidenceBytes`,
`evidenceTotalBytes`, `storeBytes`, `jobRecords`, `retentionSeconds`.
`storeBytes` uses the canonical decimal-string representation of `Revision`
because 8 GiB exceeds the integer range above; it is the only string-typed limit.
`Budgets` has `modelCalls`, `agentTasks`, `cadCommands`, `repairs`, `validationRuns`,
`wallTimeSeconds`. It carries the turn-work limits separately from `Limits`.
The runtime may only lower host limits; acceptance freezes the effective limits.

`RuntimeCapabilities` has `workflowIds: Id[]`, `hostModelCalls: bool`,
`hostAgentTasks: bool`, `guardedExecute: bool`, `guardedBatch: bool`,
`snapshotValidation: bool`. These describe the runtime implementation; usable
capabilities are their intersection with host/bridge/selected-connection support.
`BridgeRef` is `{name: "solidcraft-app", bridgeId: Id}`. The host-provisioned launch
configuration resolves this opaque ID to the chat-specific CLI/endpoint; no general
control token or AI secret crosses the runtime protocol.

`TurnBinding` is `{workflowId, connectionId, modelId, document: DocumentStamp,
profile: ProfileRef}`. The host also pins the connection's endpoint/configuration
generation; edits/deletion yield `CONNECTION_CHANGED` rather than retarget a turn.
Credentials are read from the original `Secrets` entry for each call, so a key
rotation applies to the next call. Changing the picker affects the next turn only.
`TurnContext` is `{selection: object[], requirements: ArtifactRef|null}`; selection
objects come from native inspection/schema discovery and reference the bound stamp.
They are not authorization to resolve stale topology against a different revision.

Runtime lifecycle is `starting → ready → stopping → stopped`, or `starting/ready →
transport_error`. Authentication is a separate host-connection state: `unknown`,
`available`, `auth_required`, `unavailable`. A ready runtime does not authenticate
any connection. `MastraBackend` must not let its runtime-ready event trigger the
existing `running_id` signed-in association; #6 adapts these states explicitly.

Turn lifecycle is `accepted → running → validating → running` (bounded repair),
then exactly one of `validated_success`, `quality_failed`, `task_failed`,
`auth_required`, `budget_exhausted`, `cancelled`, `transport_error`, `outcome_unknown`.
The host assigns a terminal state on runtime loss/deadline even if the child emits
none. A child's `turn.finished` is a proposal until the host checks it. Repeated
terminal events do not change the accepted terminal outcome. A normal end of an
individual model or ACP task is not turn success.

The runtime may propose `validated_success` after observing a passing report,
before the host performs section 9.3. The authoritative outcome is the host's
terminal record and UI publication, not the received event. Protocol 1.0 has no
H→R terminal confirmation event. After emitting `turn.finished`, the runtime
issues no further work for that turn and cannot infer host acceptance. The host
admits a subsequent turn only after its own terminal transition completes.

| Event | Direction / required `data` |
|---|---|
| `runtime.ready` | R→H, no scope: `{runtimeInstanceId}`; after hello, not authentication. |
| `connection.state` | H→R, session/turn: `{connectionId, state, authMethods}`. `AuthMethod` is `{id: Id, name: string, description: string}`; the host UI owns sign-in. |
| `workflow.stage` | R→H, turn: `{stageId: Id, title: string, state: "running"|"completed"|"failed"}`. |
| `workflow.text` | R→H, turn: `{text: string, channel: "answer"|"progress"}`. Text is not a verification decision. |
| `workflow.plan` | R→H, turn: `{steps: [{id: Id, title: string, state: "pending"|"running"|"completed"|"failed"}]}`; at most 64 steps. |
| `workflow.tool` | R→H, turn: `{callId: Id, name: string, state: "running"|"completed"|"failed", output: object|null}`. Host bridge events remain authoritative for actual effects. |
| `agent.progress` | H→R, turn: `{agentSessionId: Id, kind: "text"|"tool"|"plan"|"permission", content: object}`; bounded ACP-origin information, not invented hidden events. |
| `model.delta` | H→R, turn: `{index: integer, text: string}`; only if genuine streaming was negotiated. |
| `turn.finished` | R→H, turn: `{state: TurnTerminal, reason: string, document: DocumentStamp, changes: "none"|"applied"|"rolled_back"|"unknown", validationRunId: Id|null, report: ArtifactRef|null}`. |

`agent.progress.content` is a tagged payload selected by `kind`: text has `{text}`;
tool has `{callId, title, state}` using the workflow-tool state values; plan has
`{steps}` using the workflow-plan shape; permission has `{permissionId, title,
state: "waiting"|"allowed"|"denied"}`. Permission answers come from the existing
host UI, never from the model/runtime approving its own action. No arbitrary ACP
object is forwarded as an unbounded protocol extension.

## 6. HostAi calls and conversation state

### 6.1 Shared types

`ArtifactRef` is `{artifactId: Id, digest: Digest, sizeBytes: integer,
mediaType: string}`. Artifacts are host-owned, immutable, authorized references,
not arbitrary paths/URLs. Missing, expired, changed or out-of-scope artifacts fail
before use. Attachments are imported by the host before the turn starts.

`ContentBlock` is one of:

- `{type: "text", text: string}`;
- `{type: "image", artifact: ArtifactRef}` with `image/png` or `image/jpeg`;
- `{type: "document", name: string, artifact: ArtifactRef}` with `application/pdf`.

`AiCapabilities` has booleans `modelGenerate`, `agentRun`, `tools`, `images`,
`documents`, `structuredOutput`, `modelSelection`, `usage`, plus
`delivery: "buffered"|"streaming"`. Unsupported inputs fail before invocation.
An unknown capability is false. The normalized `model.generate` path remains
`buffered`; splitting a completed answer into pieces is not token streaming. The
desktop direct-chat backend (`ApiBackend`) can stream verified providers only
(OpenAI Chat, Anthropic Messages, DeepSeek Chat over the OpenAI wire). The
`agent_direct_streaming` preference is off by default, so the old buffered
behaviour stays unless the user opts in; unknown providers start buffered either
way. A stream that fails before any text or tool output may make one buffered
attempt; after observable output it fails without replay.

`ModelMessage` has `{role: "system"|"user"|"assistant"|"tool", content: ContentBlock[],
toolCalls: ToolCall[], toolCallId: Id|null}`. Only assistant messages have nonempty
`toolCalls`; only tool messages have a non-null `toolCallId`. Every tool result
matches a preceding call exactly once before another user turn. `ToolCall` is
`{id: Id, name: string, arguments: object}`. `ToolDefinition` is
`{name: string, description: string, inputSchema: object}`; schemas come from the
authoritative tools. Structured output schemas are JSON Schema Draft 2020-12;
unsupported schema features return `UNSUPPORTED_CAPABILITY` before invocation.

`Usage` is `{inputTokens: integer|null, outputTokens: integer|null,
totalTokens: integer|null}` or null when unavailable. Never infer cost, usage or
hidden ACP calls from provider/model names. Provider tool IDs which do not fit `Id`
are mapped bijectively to host-generated wire IDs in continuation state.

### 6.2 `model.generate`

R→H, turn scope. Required params:

```text
{connectionId, modelId, messages: ModelMessage[], tools: ToolDefinition[],
 options: {delivery: "buffered"|"streaming", timeoutSeconds: integer,
           maxOutputTokens: integer|null, temperature: number|null,
           reasoningEffort: string|null, outputSchema: object|null},
 continuation: Id|null}
```

`timeoutSeconds` is positive and clamped down to the admitted remaining budget.
`maxOutputTokens` is null or 1..131072; `temperature` is null or finite 0..2;
`reasoningEffort` is null or at most 64 bytes, validated for the provider.
Unsupported options fail explicitly rather than silently disappear.

The terminal success result is `{content: ContentBlock[], toolCalls: ToolCall[],
finishReason: "stop"|"tool_calls"|"length"|"refusal"|"unknown",
usage: Usage, continuation: Id|null, delivery: "buffered"|"streaming"}`.
Returned content contains text only in 1.0. Buffered calls have exactly one full
reply and no `model.delta`. For streaming-capable future adapters, ordered deltas
precede one terminal reply containing the complete answer; cancellation discards
partial tool declarations. An incomplete/invalid declaration is never executed.

HostAi performs one model invocation. It does not run returned tools or secretly
start another model round. The workflow consumes calls and sends results in its
next explicit invocation. Direct chat retains its own history/tool loop in
`ApiBackend` while reusing the same extracted single-call implementation.

The runtime owns normalized message history. The host owns opaque provider-specific
continuation (reasoning blocks/signatures, provider message IDs and routing IDs).
A continuation handle is scoped to session, connection, model and the normalized
history branch that produced it. A following call must extend that branch with
matching tool results/new input; reuse on another branch/model/session fails
`INVALID_REQUEST`. Handles are invalidated by reset/expiry. If history trimming
requires dropping provider state, restart explicitly with `continuation: null`;
never replay fragments as though they belonged to the new branch. Raw credentials
and provider-private state are not serialized to runtime messages or panel traces.

### 6.3 `agent.run`

R→H, turn scope. Required params:

```text
{connectionId, modelId, task: ContentBlock[],
 context: {document: DocumentStamp, requirements: ArtifactRef|null,
           delegationMode: "proposal_only"}, timeoutSeconds: integer}
```

The successful result is `{agentSessionId: Id, state: "completed",
content: ContentBlock[], stopReason: string, usage: Usage}`. Content is text only
in 1.0; an agent failure returns `TASK_FAILED`, not a successful model reply.
`agent.progress` events may precede the terminal response. `usage` is null unless
the agent actually supplied it. Model selection must be confirmed or the request
fails `UNSUPPORTED_CAPABILITY`; silently using another model is forbidden.

HostAi creates a fresh ACP session for each task; it may reuse a healthy process
but never the direct-chat session/history. It forwards the original connection's
auth methods/status to the panel and invokes ACP authentication there. Codex owns
its account credentials. The proposal-only session receives no write-capable CAD
bridge or general host control token. Its empty working directory and tool
configuration convey this mode; this is not a general OS-sandbox guarantee.

Before normalizing an ACP event, HostAi must preserve and validate its originating
process instance, upstream ACP session and active task association. This applies
to text, tools, plans, permission requests and prompt replies before they become
progress events, proposal content or UI permission questions. Drop retired-session
updates and answer stale permission requests as cancelled. Never relabel a task-A
message with task B's outer runtime scope. The baseline ACP parser drops session
identity from several event variants, so #4 must add this filtering while extracting
the shared executor. A peer lacking verifiable session identities requires a fresh
process for each task; process reuse is allowed only when this association is proven.

The expected proposal, extracted from the returned text, is exactly
`{source: DocumentStamp, commands: [{command: string, params: object}]}` with 1..64
commands. The runtime validates JSON, source stamp, discovered schemas, allowed
capabilities, fixed requirements and remaining budgets before scheduling it through
the guarded MCP adapter. Invalid output yields `INVALID_PROPOSAL`. External-agent
tool activity is not evidence that these CAD commands were applied.

## 7. Guarded document execution and reconciliation

`DocumentStamp` is `{documentId: Id, revision: Revision}`. The adapter maps the
existing engine result `document_id` to `documentId` and converts its integer
revision losslessly. This identity is process-local and cannot name a durable
candidate after restart.

The new optional `expectedDocument: DocumentStamp` lives in the execution envelope,
alongside `command`/`params` or `commands`/`rollback`, not inside a CAD command's
parameters. MCP `execute` and `batch` expose it and forward it unchanged to the
corresponding control/engine boundary. Existing callers may omit it; workflow
mutations must supply it. Successful guarded replies add `documentStamp` naming
the resulting live state. #8 also specifies this additive contract in the control
and MCP documentation when implementing it.

Example MCP request (the guard is proposed, the command is existing):

```json
{"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"execute","arguments":{"command":"parameters.change","params":{"name":"width","expression":"40"},"expectedDocument":{"documentId":"doc-demo","revision":"41"}}}}
```

The chat bridge injects the trusted turn association/cancel flag. A runtime cannot
gain another turn's permission by inventing a turn ID in tool arguments. The host
pins the intended document, compares both identity and revision before touching
its undo history, and repeats the check at actual application after deferred work.
Wrong identity, tab switch, intervening edit or closed document fails before the
guarded mutation. A closed document returns `current: null`. The workflow then
re-inspects and replans; it does not replace the expected stamp and retry blindly.

A guarded batch is admitted/applied as one host operation. Its base-state check
protects the entire batch, including deferred evaluation and application; its own
steps do not each compare against the original revision. No intermediate partial
result may overwrite an intervening user edit. Unsupported atomic/guarded paths
return `UNSUPPORTED_CAPABILITY`; the adapter cannot fall back to unguarded or
sequential calls while claiming the batch guard. Existing batch rollback facts
remain explicit and distinct from these admission checks.

The adapter's `WriteOutcome` has required fields `state`, `request`, `before`,
`after`, `completed`, `rollback`, `expectation`, `error`:

- `state`: `applied`, `command_failed`, `expectation_failed`, `pending`, `stale`,
  `cancelled` or `unknown`.
- `request`: `{bridgeId: Id, mcpSessionId: Id, requestId: Id}`; retain the original
  MCP identity (map numeric transport IDs bijectively when needed).
- `before: DocumentStamp`, `after: DocumentStamp|null`, `completed: integer|null`;
  null means unknown, not zero applied commands.
- `rollback`: `{requested: bool, attempted: bool, restored: bool|null, undone: integer|null}`.
- `expectation`: null or `{ok: bool, measured: object, expected: object, mismatch: string[]}`.
- `error: Error|null`.

`expectation_failed` means the model remains built. `command_failed` may have full,
partial or failed rollback as reported. `pending` means still running, not cancelled.
The adapter calls `pending` on the **same** MCP/control connection and preserves
its request identity. The application-wide `running` tool is diagnostic and cannot
prove the result of a lost connection. After disconnect, use `unknown` until facts
establish the effect; 1.0 may require user re-inspection. Never resubmit that mutation
or infer success from “nothing pending” on a new connection.

## 8. Immutable candidates and manifests

The proposed native command `document.snapshot.capture` is exposed through the
discovered command/schema interface by #11. MCP `execute` supplies `params: {}`
and the `expectedDocument` envelope field; its native result is
`{candidate: CandidateRef}`, with the guarded executor also returning
`documentStamp` equal to the captured source stamp. This is a proposed command,
not an existing capability at the baseline. Capture pins a coherent design state before
off-thread serialization, checks the stamp without marking the document saved or
adding undo entries, and captures every supported external dependency coherently.
If a dependency changes, is unresolved, exceeds limits or cannot be frozen, fail
explicitly rather than keep a mutable path in the snapshot.

`CandidateRef` has required `candidateId: Id`, `source: DocumentStamp`,
`candidateDigest: Digest`, `manifestDigest: Digest`, `artifact: ArtifactRef`.
The artifact identifies a host-owned immutable directory/container with its native
design and all manifest inputs. It is not the live file path.

`SnapshotManifest` has required `schemaVersion: "1.0"`, `candidateId`, `source`,
`capturedAt: Timestamp`, `formatVersion: string`, `engineVersion: string`,
`inputs: ManifestInput[]`. Each input is exactly
`{path: string, kind: "design"|"component"|"import"|"requirements",
dependencyId: Id, dependencyVersion: string|null, digest: Digest, sizeBytes: integer}`.
There is exactly one `design` input, a native `.sc` design retaining parameters and
feature history. Requirements/protected-dimension baselines, when supplied, are
frozen inputs too. Every dependency has a stable logical ID plus its persisted-byte
digest, even if it has no upstream version.

Paths are normalized relative POSIX paths inside the artifact, at most 240 bytes;
no absolute path, `..`, empty component, backslash or symlink escape. Inputs are
sorted by bytewise UTF-8 path order, with unique paths and dependency IDs. The runner
resolves all native external references to captured inputs or refuses the candidate;
it must not fall back to network/live filesystem content. Retain metadata outside
the native design where possible; a required native format addition follows the
[pending-format process](../crates/doc/src/format/pending/README.md).

Hash rules are normative:

1. Each input digest is SHA-256 of its exact persisted bytes, not a mesh, parameter
   summary or in-memory object.
2. `candidateDigest` is SHA-256 of UTF-8 RFC 8785 JSON Canonicalization Scheme (JCS)
   encoding of `{formatVersion, engineVersion, inputs}`. Dependency identity, path,
   version and digest are thus included. No floating-point geometry is reserialized
   to manufacture a replacement for native bytes.
3. `manifestDigest` is SHA-256 of JCS encoding of the complete `SnapshotManifest`.
   Neither digest is a field inside that manifest, avoiding recursive hashing.
4. `artifact.digest` hashes the exact persisted candidate container bytes (a packaging
   identity in addition to the manifest/input identities). Packaging must define a
   reproducible container for fixtures, but acceptance never substitutes its hash
   for verification of the manifest's entries.

The host allocates candidate IDs, retains active references, performs safe atomic
writes, and persists the lookup before returning. An incomplete capture is never
visible as a valid candidate. Restart reopens and verifies retained artifacts.
Cleanup removes only unpinned expired inputs and returns `NOT_FOUND` for expired
handles; store pressure refuses new captures rather than deleting active evidence.

## 9. ValidationPort

### 9.1 Profiles and required checks

`ProfileRef` is `{profileId: Id, version: string, digest: Digest}`. Its digest is
SHA-256 of JCS encoding of the complete profile object. A `CheckProfile` is
`{schemaVersion: "1.0", profileId, version, checks: CheckRequirement[]}`.
Each requirement is `{checkId: Id, kind: string, required: bool, parameters: object}`.
IDs are unique, checks are ordered by `checkId`, and there are 1..128 checks.
The host resolves the actual profile from its configured registry, verifies the
digest and freezes it for the turn. The runtime/model cannot change requirements,
mark a check optional or widen tolerances to make a candidate pass.

The initial `native-basic` profile version `1.0.0` always includes:

| Check ID / kind | Required parameters and evidence |
|---|---|
| `input.integrity` / `input.integrity` | `{}`; verify container, manifest, all input digests and resolved dependency identities; evidence is the verified inventory. |
| `native.rebuild` / `native.rebuild` | `{}`; reopen the native design and evaluate without unresolved dependencies or failed/error features; evidence is rebuild diagnostics. |

Configured task profiles add required `parameters.required` and `geometry.exact`
checks when the task declares dimensions or geometry requirements. A parameter
check has `{requirements: [{name: string, value: number, absTolerance: number}]}`.
An exact-geometry check has `{measurements: [{command: string, params: object,
resultPointer: string, expected: number, absTolerance: number, relTolerance: number}]}`.
Arrays contain 1..64 entries; expected values are finite, tolerances are finite and
nonnegative. Measurements use supported native exact-geometry commands/schema
results, never display meshes. Numeric acceptance is
`abs(actual - expected) <= max(absTolerance, relTolerance * abs(expected))`;
parameter checks use absolute tolerance only. A missing/nonfinite result is failure.

The host forms the task profile before `turn.start`, including protected dimensions
and their captured baseline. The minimum profile alone does not prove user-requested
dimensions. A profile demanding an unavailable exact measurement is unsupported;
the runner records `unsupported` and verified success is impossible.

### 9.2 Operations and persistence

R→H wire names are prefixed `validation.`. `submit` requires turn scope; `status`,
`report` and `cancel` permit session or turn scope after host authorization of the
run. Only the host can attach a historical run to a new session; an arbitrary ID
from runtime text grants no access.

| Interface | Required params | Successful result |
|---|---|---|
| `submit(candidate, profile, requestId)` | `{candidate: CandidateRef, profile: ProfileRef, requestId: SubmissionKey}` | `{runId: Id, state: ValidationState, candidate: CandidateRef, profile: ProfileRef}` |
| `status(runId)` | `{runId: Id}` | `{runId, state: ValidationState, candidate: CandidateRef, profile: ProfileRef, updatedAt: Timestamp, reportAvailable: bool}` |
| `report(runId)` | `{runId: Id}` | `{report: ValidationReport, reference: ArtifactRef}`; `NOT_READY` before a report exists. |
| `cancel(runId)` | `{runId: Id}` | `{runId, state: ValidationState}`; idempotent, preserves an already-terminal state. |

The params' `requestId` is the durable submission key, distinct from the transport
envelope `requestId`. `SubmissionKey` is `<unix-milliseconds>:<uuid-v4>`, within `Id`
length. The host accepts timestamps no more than five minutes in the future and
less than seven days old. It records the key, exact candidate/profile identities,
owner binding and run ID atomically before starting a worker. Same key/same payload
returns the same run; same key/different payload returns `IDEMPOTENCY_CONFLICT`.
Payload equality compares JCS bytes of `{candidate, profile}`, including every
field of the supplied `CandidateRef` and `ProfileRef`. JSON whitespace, object-key
order and the transport envelope's request ID do not affect equality. Authorization
and original turn ownership still apply; replay cannot transfer job ownership.
Expired keys return `REQUEST_EXPIRED`, including after cleanup, rather than create
a second job. Persist mappings until the later of the key's expiry and seven days
after job termination, within the host record cap. A lost submit response is retried
with the same key; it is never converted into a new submission implicitly.

Validation states are `queued`, `running`, `passed`, `quality_failed`,
`infrastructure_error`, `cancelled`, `unknown`. `queued` may enter `running`,
`infrastructure_error` (queue deadline), or `cancelled`. `running` enters a terminal
state. Required measured failures yield `quality_failed`; worker failure, timeout,
malformed output or unavailable checks yield `infrastructure_error`. Ambiguous
worker outcome after loss/restart yields `unknown`. All five terminal states are
immutable for that run. Bounded reconciliation may find an already committed
terminal record before publishing `unknown`; once unknown is published, a new,
explicit submission is required for another run. Polling does not silently rerun it.

Persist job admission, worker identity, cancellation marker, final report and final
state with crash-consistent transitions. On restart, reconcile owned workers and
persisted reports; never restart an admitted job as new work. Cancel can win only
before terminal commit. If it wins, a late passing worker result cannot overwrite
`cancelled`; if pass committed first, cancel returns `passed` but a cancelled turn
still cannot be promoted. Local runner sessions are isolated from the live document
and read immutable inputs; this validation isolation does not isolate earlier modeling.

### 9.3 Reports and host acceptance

`ValidationReport` requires:

```text
{schemaVersion: "1.0", runId, candidateId,
 candidateDigest, manifestDigest, profile: ProfileRef,
 runner: {runnerId: Id, version: string, engineVersion: string},
 state: "passed"|"quality_failed"|"infrastructure_error"|"cancelled"|"unknown",
 startedAt: Timestamp|null, finishedAt: Timestamp,
 checks: CheckResult[], error: Error|null}
```

`CheckResult` is `{checkId: Id, required: bool,
state: "passed"|"failed"|"error"|"unsupported"|"not_run",
expected: object, actual: object, evidence: ArtifactRef[]}`.
Reports contain exactly one result for every profile check, even if cancelled or
not run; no duplicates or missing checks. `required` must match the profile. A
passing required check has at least one resolvable digest-checked evidence artifact.
Diagnostics and evidence are produced by the trusted runner, not supplied by an
LLM. The report reference hashes the exact persisted report bytes.

Before accepting `validated_success`, the host checks all of the following:

1. The run was submitted by this turn for the candidate and frozen profile; the
   report is loaded from that host run, not trusted from runtime-supplied JSON.
2. Both the persisted run terminal state and the persisted report's `state` are
   `passed`, and the report's `error` is null. Passing check entries alone are
   insufficient: a run cancelled after checks finish is still not a passing run,
   even if the owning turn has not been cancelled.
3. Candidate ID, candidate digest, manifest digest, profile ID/version/digest and
   approved runner ID/version match the admission record; the runner engine version
   matches the captured manifest's engine version in 1.0.
4. The persisted inputs, profile and report/evidence digests verify; required check
   coverage is exact and every required result is `passed`. Unknown, unsupported,
   missing, malformed, incomplete and infrastructure-error results cannot pass.
5. The turn is active, uncancelled, within budget and has no pending/unknown writes.
   It has performed no writes after capture. The current live `DocumentStamp`
   still equals the candidate's source stamp at final acceptance.

The host performs the last turn/document check and terminal publication in one
host-owned transition. Every terminal outcome closes that turn's write admission;
late callbacks cannot schedule new changes after completion.

If the live document changed after capture, the passing report remains historical
evidence for that candidate, not approval of current content. Re-inspect and create
a new candidate/run within budget, or end without verified success. Every repair
creates a new candidate and submission key; a report for A cannot approve B.
A quality failure may consume a repair; a runner/transport error calls for bounded
job reconciliation or an explicit validation retry, not an automatic geometry edit.

## 10. Protocol traces

These examples use short IDs and illustrative 64-hex digests. They demonstrate
wire shape and identity checks, not runnable native `.sc` fixtures or actual
measured results. Implementation tests create original native artifacts and compute
their real digests. Each JSON block is one complete frame/object unless described
as an array of frames. `requestId` values are unique within each trace.

### 10.1 BYOK turn

Precondition: hello negotiated 1.0; runtime instance `rt-1` is ready. The host's
`solidcraft-app` bridge has ID `bridge-1`. The host selected existing BYOK connection
`byok-1` and model `model-a`. `doc-demo` is an editable block with named parameters
`width=20`, `depth=10`, `height=5`; the task changes width to 40 and preserves the
other dimensions. The frozen task profile includes the two baseline checks and
required parameter/exact-volume checks (volume 2,000 mm³). Its illustrative digest
is `sha256:` plus 64 `b` characters.

Host starts a session; runtime readiness does not imply connection authentication:

```json
[{"protocolVersion":"1.0","kind":"request","requestId":"h:1","scope":{"sessionId":"s-1","turnId":null,"workflowRunId":null},"method":"session.start","params":{"workflowId":"native-modeling","instructions":"Use native features and fixed checks.","bridge":{"name":"solidcraft-app","bridgeId":"bridge-1"}}},{"protocolVersion":"1.0","kind":"response","requestId":"h:1","scope":{"sessionId":"s-1","turnId":null,"workflowRunId":null},"outcome":"ok","result":{"state":"ready","workflowId":"native-modeling"}}]
```

The host freezes the connection, model, document, profile and budgets:

```json
{"protocolVersion":"1.0","kind":"request","requestId":"h:2","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"method":"turn.start","params":{"binding":{"workflowId":"native-modeling","connectionId":"byok-1","modelId":"model-a","document":{"documentId":"doc-demo","revision":"41"},"profile":{"profileId":"block-width-40","version":"1.0.0","digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}},"parts":[{"type":"text","text":"Set width to 40 mm and preserve depth and height."}],"context":{"selection":[],"requirements":null},"budgets":{"modelCalls":20,"agentTasks":4,"cadCommands":64,"repairs":2,"validationRuns":3,"wallTimeSeconds":1200}}}
```

Runtime acknowledges acceptance and makes one HostAi call. The schema in the example
is a reduced declared tool for this single parameter operation; production tools
are discovered from MCP, and all schema constraints from discovery are preserved.

```json
[{"protocolVersion":"1.0","kind":"response","requestId":"h:2","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"outcome":"ok","result":{"state":"accepted"}},{"protocolVersion":"1.0","kind":"request","requestId":"r:1","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"method":"model.generate","params":{"connectionId":"byok-1","modelId":"model-a","messages":[{"role":"user","content":[{"type":"text","text":"Set width to 40 mm; preserve depth=10 and height=5."}],"toolCalls":[],"toolCallId":null}],"tools":[{"name":"set_width","description":"Propose the requested width parameter.","inputSchema":{"type":"object","properties":{"width":{"type":"number"}},"required":["width"],"additionalProperties":false}}],"options":{"delivery":"buffered","timeoutSeconds":300,"maxOutputTokens":2048,"temperature":null,"reasoningEffort":null,"outputSchema":null},"continuation":null}},{"protocolVersion":"1.0","kind":"response","requestId":"r:1","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"outcome":"ok","result":{"content":[],"toolCalls":[{"id":"call-1","name":"set_width","arguments":{"width":40}}],"finishReason":"tool_calls","usage":null,"continuation":"cont-1","delivery":"buffered"}}]
```

HostAi has made no CAD changes. The workflow validates the declaration and maps it
to `parameters.change` using the discovered native schema, then sends the guarded
MCP request in section 7 on `bridge-1`. Its confirmed result advances the stamp to
`{documentId: "doc-demo", revision: "42"}`. The workflow records the tool result
against `call-1` before any later model call using `cont-1`. A final text-only model
call is optional; the workflow can proceed directly to capture and validation.

Capture returns candidate `cand-1`, source revision `42`. The workflow follows the
validation trace below. After observing the passing report, the runtime proposes
the terminal event (`runtime.ready` used sequence 1):

```json
{"protocolVersion":"1.0","kind":"event","sequence":2,"event":"turn.finished","requestId":"h:2","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"data":{"state":"validated_success","reason":"Required checks passed for cand-1.","document":{"documentId":"doc-demo","revision":"42"},"changes":"applied","validationRunId":"v-1","report":{"artifactId":"report-1","digest":"sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","sizeBytes":3200,"mediaType":"application/json"}}}
```

The host then performs section 9.3's verification and atomic terminal acceptance
before publishing the authoritative outcome. The runtime stops this turn after
its proposal; it does not wait for a terminal-confirmation message.

### 10.2 ACP task and authentication

An independent session `s-2`, turn `t-2`, workflow run `w-2` freezes `codex`,
`codex-model`, the same document at revision `42`, and a task profile requiring
width 50 with protected depth/height. It starts with the same session/turn envelopes
as above and the corresponding binding. Runtime requests a proposal:

```json
{"protocolVersion":"1.0","kind":"request","requestId":"r:10","scope":{"sessionId":"s-2","turnId":"t-2","workflowRunId":"w-2"},"method":"agent.run","params":{"connectionId":"codex","modelId":"codex-model","task":[{"type":"text","text":"Return a native proposal setting width to 50. Preserve depth=10 and height=5."}],"context":{"document":{"documentId":"doc-demo","revision":"42"},"requirements":null,"delegationMode":"proposal_only"},"timeoutSeconds":300}}
```

If authentication is missing, HostAi returns this terminal request error and exposes
Codex's actual auth methods through the original connection. The runtime remains ready:

```json
{"protocolVersion":"1.0","kind":"response","requestId":"r:10","scope":{"sessionId":"s-2","turnId":"t-2","workflowRunId":"w-2"},"outcome":"error","error":{"code":"AUTH_REQUIRED","message":"Sign in to the configured Codex connection.","retry":"never","details":{"connectionId":"codex"}}}
```

That turn ends `auth_required`, with no CAD changes. The host's existing sign-in
flow runs, then a new turn `t-3`/`w-3` starts with the same selected connection and
a freshly inspected stamp/profile. It sends the same `agent.run` parameters under
new request `r:11`. HostAi opens ACP session `acp-task-1`, independent of direct chat,
and returns an actual completed task (authenticated first attempts use this path too):

```json
{"protocolVersion":"1.0","kind":"response","requestId":"r:11","scope":{"sessionId":"s-2","turnId":"t-3","workflowRunId":"w-3"},"outcome":"ok","result":{"agentSessionId":"acp-task-1","state":"completed","content":[{"type":"text","text":"{\"source\":{\"documentId\":\"doc-demo\",\"revision\":\"42\"},\"commands\":[{\"command\":\"parameters.change\",\"params\":{\"name\":\"width\",\"expression\":\"50\"}}]}"}],"stopReason":"end_turn","usage":null}}
```

The workflow validates this proposal and executes one guarded batch with expected
revision `42`, through its own chat bridge. ACP has performed no live CAD writes.
After confirmed application at revision `43`, it captures `cand-2` and submits a
new validation run under the width-50 profile. Only a matching passing report for
`cand-2` can finish `t-3` as validated. Agent text, `end_turn`, or `cand-1`'s report
cannot do so. Permission denial, invalid proposal and agent exit terminate through
their distinct errors instead of being presented as a raw model reply.

### 10.3 Asynchronous validation, retry and cancellation

This continues BYOK turn `s-1/t-1/w-1`. Here is the shape of its persisted manifest;
the requirements input contains protected dimensions fixed before modeling:

```json
{"schemaVersion":"1.0","candidateId":"cand-1","source":{"documentId":"doc-demo","revision":"42"},"capturedAt":"2026-11-05T12:00:00.000Z","formatVersion":"solidcraft/fixture","engineVersion":"0.40.0-rc.2","inputs":[{"path":"design.sc","kind":"design","dependencyId":"design-root","dependencyVersion":null,"digest":"sha256:1111111111111111111111111111111111111111111111111111111111111111","sizeBytes":8192},{"path":"requirements.json","kind":"requirements","dependencyId":"protected-dimensions","dependencyVersion":"1","digest":"sha256:2222222222222222222222222222222222222222222222222222222222222222","sizeBytes":128}]}
```

The format version is illustrative too; production capture uses the actual native
writer's format tag. Submission uses the candidate's aggregate digest and a separate
manifest digest. `1793880000000` is the example issuance time, 2026-11-05 12:00 UTC:

```json
{"protocolVersion":"1.0","kind":"request","requestId":"r:2","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"method":"validation.submit","params":{"candidate":{"candidateId":"cand-1","source":{"documentId":"doc-demo","revision":"42"},"candidateDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","manifestDigest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","artifact":{"artifactId":"candidate-archive-1","digest":"sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","sizeBytes":9216,"mediaType":"application/zip"}},"profile":{"profileId":"block-width-40","version":"1.0.0","digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"requestId":"1793880000000:550e8400-e29b-41d4-a716-446655440000"}}
```

Host persists admission and returns `v-1` in `queued` state:

```json
{"protocolVersion":"1.0","kind":"response","requestId":"r:2","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"outcome":"ok","result":{"runId":"v-1","state":"queued","candidate":{"candidateId":"cand-1","source":{"documentId":"doc-demo","revision":"42"},"candidateDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","manifestDigest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","artifact":{"artifactId":"candidate-archive-1","digest":"sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","sizeBytes":9216,"mediaType":"application/zip"}},"profile":{"profileId":"block-width-40","version":"1.0.0","digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}}
```

If that response is lost, a new transport request repeats the identical params and
submission key, obtaining `v-1`; it does not launch another worker. Status is queried
with `{"runId":"v-1"}` as params. It returns the same candidate/profile plus state,
timestamp and report availability; for example:

```json
{"protocolVersion":"1.0","kind":"response","requestId":"r:3","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"outcome":"ok","result":{"runId":"v-1","state":"running","candidate":{"candidateId":"cand-1","source":{"documentId":"doc-demo","revision":"42"},"candidateDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","manifestDigest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","artifact":{"artifactId":"candidate-archive-1","digest":"sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","sizeBytes":9216,"mediaType":"application/zip"}},"profile":{"profileId":"block-width-40","version":"1.0.0","digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"updatedAt":"2026-11-05T12:00:01.000Z","reportAvailable":false}}
```

When status becomes `passed`, `validation.report` with the same run ID returns:

```json
{"protocolVersion":"1.0","kind":"response","requestId":"r:4","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"outcome":"ok","result":{"report":{"schemaVersion":"1.0","runId":"v-1","candidateId":"cand-1","candidateDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","manifestDigest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","profile":{"profileId":"block-width-40","version":"1.0.0","digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"runner":{"runnerId":"local-solidcraft","version":"1.0.0","engineVersion":"0.40.0-rc.2"},"state":"passed","startedAt":"2026-11-05T12:00:01.000Z","finishedAt":"2026-11-05T12:00:02.000Z","checks":[{"checkId":"geometry.exact","required":true,"state":"passed","expected":{"volume_mm3":2000},"actual":{"volume_mm3":2000},"evidence":[{"artifactId":"evidence-1","digest":"sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","sizeBytes":1024,"mediaType":"application/json"}]},{"checkId":"input.integrity","required":true,"state":"passed","expected":{"inputs":2},"actual":{"verifiedInputs":2},"evidence":[{"artifactId":"evidence-1","digest":"sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","sizeBytes":1024,"mediaType":"application/json"}]},{"checkId":"native.rebuild","required":true,"state":"passed","expected":{"errors":0},"actual":{"errors":0},"evidence":[{"artifactId":"evidence-1","digest":"sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","sizeBytes":1024,"mediaType":"application/json"}]},{"checkId":"parameters.required","required":true,"state":"passed","expected":{"width":40,"depth":10,"height":5},"actual":{"width":40,"depth":10,"height":5},"evidence":[{"artifactId":"evidence-1","digest":"sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","sizeBytes":1024,"mediaType":"application/json"}]}],"error":null},"reference":{"artifactId":"report-1","digest":"sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","sizeBytes":3200,"mediaType":"application/json"}}}
```

The evidence artifact in this small example contains all four named check records.
The host applies section 9.3 before accepting the terminal event in section 10.1.
A repaired revision `43` would instead need a new candidate, submission key and
report; changing just the report's candidate ID cannot make it valid.

Alternative cancellation branch, before terminal commit:

```json
[{"protocolVersion":"1.0","kind":"request","requestId":"r:5","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"method":"validation.cancel","params":{"runId":"v-1"}},{"protocolVersion":"1.0","kind":"response","requestId":"r:5","scope":{"sessionId":"s-1","turnId":"t-1","workflowRunId":"w-1"},"outcome":"ok","result":{"runId":"v-1","state":"cancelled"}}]
```

After a host Stop, the host closes turn write admission even if this message never
arrives. A late worker pass cannot overwrite the cancel marker. An unavailable
worker produces `infrastructure_error` or `unknown`, not `quality_failed` and not
an instruction to rebuild geometry blindly.

### 10.4 Lifecycle and error examples

Reset cancels the old turn and retires the old session before the next session is
usable. The response still echoes the old scope; `result.sessionId` names the new one:

```json
[{"protocolVersion":"1.0","kind":"request","requestId":"h:20","scope":{"sessionId":"s-1","turnId":null,"workflowRunId":null},"method":"session.reset","params":{"nextSessionId":"s-4","instructions":"Use native features."}},{"protocolVersion":"1.0","kind":"response","requestId":"h:20","scope":{"sessionId":"s-1","turnId":null,"workflowRunId":null},"outcome":"ok","result":{"state":"ready","sessionId":"s-4"}},{"protocolVersion":"1.0","kind":"request","requestId":"h:21","scope":{"sessionId":"s-4","turnId":"t-4","workflowRunId":"w-4"},"method":"turn.cancel","params":{"reason":"user"}},{"protocolVersion":"1.0","kind":"response","requestId":"h:21","scope":{"sessionId":"s-4","turnId":"t-4","workflowRunId":"w-4"},"outcome":"ok","result":{"state":"cancelling"}},{"protocolVersion":"1.0","kind":"request","requestId":"h:22","scope":{"sessionId":null,"turnId":null,"workflowRunId":null},"method":"runtime.shutdown","params":{"reason":"app_exit"}},{"protocolVersion":"1.0","kind":"response","requestId":"h:22","scope":{"sessionId":null,"turnId":null,"workflowRunId":null},"outcome":"ok","result":{"state":"stopping"}}]
```

Here `t-4/w-4` was started after reset with the same session/turn protocol above.
A stale guarded write returns this error shape at the adapter interface:

```json
{"code":"STALE_DOCUMENT","message":"The document changed before this write could apply.","retry":"never","details":{"expected":{"documentId":"doc-demo","revision":"42"},"current":{"documentId":"doc-demo","revision":"43"}}}
```

An incompatible hello gets an error response with `code: INCOMPATIBLE_VERSION`,
`retry: never`, `details: {supported: ["1.0"]}` and the original request ID/scope,
then EOF. Transport failure produces host-owned `transport_error` outcomes for
unresolved turns; ACP task failure yields `TASK_FAILED` for that task without
asserting that the runtime process died.

## 11. Rust and TypeScript compatibility evidence

This document is the semantic authority. #5 owns the machine-readable contract
bundle under a new `schema/mastra-runtime/v1/` directory in this repository: JSON
Schema Draft 2020-12 envelopes/data types, valid/invalid fixtures and expected
state-machine transcripts. Its files form one versioned artifact with a SHA-256
manifest. #7 consumes that exact workspace artifact from the same repository
revision, verified by digest; it does not maintain an independently edited schema
copy. The root location is outside engine command schemas because the IPC contract
belongs to the host layer.

Rust serde types and TypeScript types/validators must both validate the same fixture
bytes and agree on acceptance, error category and terminal-state transcript. A
successful type check in each language is insufficient. Serialize accepted fixtures
in both directions and compare canonical JSON; compare preserved decimal revisions
and digest bytes, not JavaScript numeric approximations. The release pins host,
runtime and contract artifact versions together.

[#17](https://github.com/intgral-ai/open-skyfall-desktop/issues/17) owns fork-specific
branch governance and CI wiring: path-aware Rust/TypeScript checks, shared-schema
consumers, host/runtime integration, real runner evidence and release gates. Its
proposed `develop`/stable-`main` branch model is not an approved product decision.
It blocks #14's final release acceptance, not independent #5/#7 development. The
2026-11-05 audit found no recorded Actions runs and no branch protection/rulesets;
inherited workflow files and an empty checks list are not passing CI evidence.

Required compatibility cases, owned by the linked implementation tasks:

| Fixtures / assertions | Owners |
|---|---|
| Every request, response, error and event variant; required/null/unknown fields; empty versus nonempty error details; integer limits versus string `storeBytes`; malformed UTF-8, duplicate keys, depth/size limits | #5, #7 |
| Hello exact-version success/refusal; unknown method/capability; nested HostAi callback while awaiting runtime work; bounded queue/EOF cleanup | #5 |
| Revision `9007199254740993` and max-u64 round trips; overflow/leading-zero rejection; `document_id` normalization | #5, #8 |
| Buffered result versus genuine deltas; tool-call identity, provider continuation branch/reset, cancelled HTTP still occupying its physical slot | #3, #7 |
| Real Mastra Agent backed by deterministic HostAi: exactly one model step, no hidden retry/tool execution, tool-result identity and capability rejection; process replacement cannot resume old transcript/continuation | #5, #7 |
| ACP auth-required versus ready, independent sessions, unsupported model selection, permission denial and invalid proposals; late task-A text/tools/permissions during task B cannot enter B's output or UI | #4, #6 |
| Cancel/reset/late terminal races; exactly one host-owned turn terminal outcome; a proposed success cannot bypass host acceptance; no stale turn writes | #5, #6, #9 |
| Guarded execute/batch application interleavings, tab changes, deferred evaluation, failed rollback, expectation failure, same-connection pending and lost-connection unknown | #8, #9 |
| Adapted authoring: native editable create/parameter edit, protected dimensions, ask/decline, unsupported speculative probe with zero writes, bounded repairs, stop after unknown effects; old passing candidate cannot certify a later failed edit | #9, #10, #13 |
| JCS known vectors and byte/identity/dependency changes; candidate artifacts survive live edits/restart; active references survive cleanup | #11 |
| Duplicate submit keys with reordered/whitespace-different JSON return the same run; changed candidate/profile fields conflict; ownership cannot transfer; expiry, worker crash/restart, timeout/cancel race, queue/store exhaustion | #12 |
| A report for A cannot approve B; missing required check/evidence, profile or runner mismatch, changed live stamp, old cancelled turn; a cancelled run with passing checks cannot approve even an active turn | #12, #13 |
| Installed runtime without developer Node/PATH; Codex missing/authenticated paths; unsupported targets; licensed artifact inventory | #14, #15 |
| Same-source host/runtime/schema manifest; runtime-only and shared-schema changes select all affected checks; fork runner and release dry-run evidence | #5, #7, #14, #17 |

These are obligations for implementation, not test results claimed by this document.
Tests use existing area test files on the Rust side. Native analytic fixtures use
original simple geometry and exact measurements; they do not import another CAD
product's source, templates or expected results.

## 12. Delivery ownership and acceptance map

| Task | Contract sections / deliverable |
|---|---|
| [#3 Shared BYOK calls](https://github.com/intgral-ai/open-skyfall-desktop/issues/3) | 2–6: extract single-call HostAi, original Secrets IDs and continuation ownership. |
| [#4 ACP tasks](https://github.com/intgral-ai/open-skyfall-desktop/issues/4) | 5–6: workflow-owned task sessions, authentication and proposal-only mode. |
| [#5 IPC/lifecycle](https://github.com/intgral-ai/open-skyfall-desktop/issues/5) | 3–5, 11: bounded transport, lifecycle and authoritative schema/fixture bundle. |
| [#6 Panel backend](https://github.com/intgral-ai/open-skyfall-desktop/issues/6) | 2, 4–5: frozen binding, readiness/auth split, permissions, queue/cancel/reset/undo presentation. |
| [#7 Runtime](https://github.com/intgral-ai/open-skyfall-desktop/issues/7) | 1–6, 11: integrate the in-repository workspace, adapt the Agent/model and transcript seams, and consume the same-revision contract bundle. |
| [#8 Guarded writes](https://github.com/intgral-ai/open-skyfall-desktop/issues/8) | 7: atomic `expectedDocument` checks through deferred application and result stamps. |
| [#9 MCP adapter](https://github.com/intgral-ai/open-skyfall-desktop/issues/9) | 7: discovery, serialized guarded writes and honest outcome reconciliation. |
| [#10 Native workflow](https://github.com/intgral-ai/open-skyfall-desktop/issues/10) | 1.5, 4, 6–7: adapt pinned Vector Studio orchestration to native stages, fixed checks and bounded repair, with a source/deviation record. |
| [#11 Snapshots](https://github.com/intgral-ai/open-skyfall-desktop/issues/11) | 8: coherent native capture, dependency closure, manifests, hashing and retention. |
| [#12 Local validation](https://github.com/intgral-ai/open-skyfall-desktop/issues/12) | 9: independently callable local jobs, profiles, idempotency and durable reports. |
| [#13 Verified outcomes](https://github.com/intgral-ai/open-skyfall-desktop/issues/13) | 5, 9: host verification, repair/new candidate, historical evidence and terminal outcomes. |
| [#14 Packaging](https://github.com/intgral-ai/open-skyfall-desktop/issues/14) | 1, 11: same-revision target/package matrix, private Node, discovered Codex and installed evidence; depends on #15 and #17. |
| [#15 Distribution enforcement](https://github.com/intgral-ai/open-skyfall-desktop/issues/15) | 1: reconcile the narrow policy exception and selected source-migration rights before import; enforce file/license/provenance checks before bundling. |
| [#17 Fork governance/CI](https://github.com/intgral-ai/open-skyfall-desktop/issues/17) | 11: resolve branch policy, configure/test real fork CI and same-revision release gates; no settings are changed by T01. |

T01 acceptance is traceable without this design conversation: section 10 covers a
BYOK turn, ACP task and asynchronous run; sections 3–6 distinguish readiness,
authentication, transport and task errors; sections 3–9 define fields, limits,
identity and terminal rules; section 1 records distribution/gate decisions and
the bounded extra task; section 2 separates existing and proposed guarantees.

For this documentation change, verify all JSON examples parse, relative file links
and anchors resolve, examples agree on candidate/profile/report identities, and
each acceptance item has a section above. Run the repository documentation-related
checks and `cargo xtask ci` before a commit, reporting actual outcomes. Observable
runtime/UI evidence belongs to the implementing tasks; no screenshot can prove
an interface which this documentation-only change has not implemented.
