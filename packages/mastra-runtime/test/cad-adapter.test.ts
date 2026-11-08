import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";

import { ProtocolError } from "../src/errors.js";
import { SchemaBundle } from "../src/protocol.js";
import type { JsonObject } from "../src/protocol.js";
import {
  CadAdapter,
  readTopLevelUnsignedInteger,
  type CadBatchStep,
  type CadWriteResult,
  type DocumentStamp,
  type WriteOutcome,
} from "../src/solidcraft/cad-adapter.js";
import { McpClient } from "../src/solidcraft/mcp-client.js";
import {
  FakeMcpPeer,
  defaultTools,
  deferred,
  guardedReply,
  textReply,
  type FakePeerOptions,
  type ToolHandler,
  type ToolReply,
} from "./support/mcp-peer.js";

const bundle = SchemaBundle.load();

const STAMP_41: DocumentStamp = { documentId: "doc-A", revision: "41" };
const STAMP_42: DocumentStamp = { documentId: "doc-A", revision: "42" };
const STAMP_43: DocumentStamp = { documentId: "doc-A", revision: "43" };

function assertConforms(result: CadWriteResult): void {
  assert.equal(
    bundle.validate("#/$defs/WriteOutcome", result.outcome),
    true,
    `WriteOutcome does not conform to the repository schema: ${JSON.stringify(result.outcome)}`,
  );
}

function stampText(stamp: DocumentStamp): string {
  return `{\n  "document_id": "${stamp.documentId}",\n  "revision": ${stamp.revision}\n}`;
}

function revisionReply(peer: FakeMcpPeer): ToolReply {
  return textReply(stampText(peer.stamp()));
}

function wireErrorReply(code: string, message: string, details: JsonObject = {}): ToolReply {
  return guardedReply({ error: { code, message, retry: "never", details }, message }, true);
}

interface Harness {
  readonly peer: FakeMcpPeer;
  readonly client: McpClient;
  readonly adapter: CadAdapter;
  setExecute(handler: ToolHandler): void;
  setBatch(handler: ToolHandler): void;
  setPending(handler: ToolHandler): void;
}

interface HarnessOptions extends FakePeerOptions {
  readonly writeTimeoutMs?: number;
  readonly readTimeoutMs?: number;
  readonly pendingTimeoutMs?: number;
}

async function harness(t: TestContext, options: HarnessOptions = {}): Promise<Harness> {
  const peer = new FakeMcpPeer(options);
  let executeHandler: ToolHandler | null = null;
  let batchHandler: ToolHandler | null = null;
  peer.execute((args) => {
    if (args["command"] === "document.revision") {
      return revisionReply(peer);
    }
    return executeHandler === null ? textReply({ error: "no execute handler" }, true) : executeHandler(args);
  });
  peer.batch((args) => (batchHandler === null ? textReply({ error: "no batch handler" }, true) : batchHandler(args)));
  const client = new McpClient({
    channel: peer,
    bridge: { name: "solidcraft-app", bridgeId: "bridge-1" },
    turn: { sessionId: "s-1", turnId: "t-1", workflowRunId: "w-1" },
    mcpSessionId: "mcp-session-fixture",
    requestTimeoutMs: 1_000,
    handshakeTimeoutMs: 1_000,
  });
  await client.connect();
  const adapter = new CadAdapter({
    client,
    turn: {
      bridgeId: "bridge-1",
      sessionId: "s-1",
      turnId: "t-1",
      workflowRunId: "w-1",
      document: STAMP_41,
    },
    writeTimeoutMs: options.writeTimeoutMs ?? 1_000,
    readTimeoutMs: options.readTimeoutMs ?? 1_000,
    pendingTimeoutMs: options.pendingTimeoutMs ?? 1_000,
  });
  t.after(() => adapter.dispose());
  return {
    peer,
    client,
    adapter,
    setExecute: (handler) => {
      executeHandler = handler;
    },
    setBatch: (handler) => {
      batchHandler = handler;
    },
    setPending: (handler) => peer.pending(handler),
  };
}

function tick(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

async function waitFor(predicate: () => boolean, attempts = 100): Promise<void> {
  for (let i = 0; i < attempts; i += 1) {
    if (predicate()) {
      return;
    }
    await tick();
  }
  assert.fail("condition did not become true");
}

test("a guarded execute uses the frozen stamp and records the original request identity", async (t) => {
  const { peer, adapter, setExecute } = await harness(t);
  setExecute((args) => {
    assert.deepEqual(args["expectedDocument"], STAMP_41);
    assert.equal(args["command"], "parameters.change");
    assert.deepEqual(args["params"], { name: "width", expression: "40" });
    assert.equal((args["params"] as JsonObject)["expectedDocument"], undefined);
    return guardedReply({ result: { changed: true }, documentStamp: STAMP_42 });
  });
  const result = await adapter.execute("parameters.change", { name: "width", expression: "40" });
  assertConforms(result);
  assert.equal(result.outcome.state, "applied");
  assert.equal(result.outcome.after?.revision, "42");
  assert.equal(result.outcome.completed, 1);
  assert.equal(result.outcome.expectation, null);
  assert.equal(result.outcome.error, null);
  assert.deepEqual(result.outcome.request, {
    bridgeId: "bridge-1",
    mcpSessionId: "mcp-session-fixture",
    requestId: `mcp-${String(peer.toolCallsNamed("execute")[0]?.id)}`,
  });
  assert.deepEqual(adapter.stamp, STAMP_42);
});

test("a guarded batch is one host operation with the stamp at the envelope level", async (t) => {
  const { adapter, setBatch } = await harness(t);
  setBatch((args) => {
    const commands = args["commands"] as JsonObject[];
    assert.equal(commands.length, 3);
    for (const step of commands) {
      assert.equal(step["expectedDocument"], undefined);
      assert.equal((step["params"] as JsonObject)["expectedDocument"], undefined);
    }
    assert.deepEqual(args["expectedDocument"], STAMP_41);
    return guardedReply({
      ok: true,
      completed: 3,
      results: [{ ok: true }, { ok: true }, { ok: true }],
      documentStamp: STAMP_42,
    });
  });
  const result = await adapter.batch([
    { command: "solid.box", params: { length: 1, width: 2, height: 3 } },
    { command: "solid.box", params: { length: 4, width: 5, height: 6 } },
    { command: "solid.box", params: { length: 7, width: 8, height: 9 } },
  ]);
  assertConforms(result);
  assert.equal(result.outcome.state, "applied");
  assert.equal(result.outcome.completed, 3);
  assert.deepEqual(result.outcome.rollback, { requested: true, attempted: false, restored: null, undone: null });
  assert.equal(result.outcome.after?.revision, "42");
  assert.equal(adapter.stamp.revision, "42");
});

test("a failed batch with rollback preserves completed, applied and undone facts", async (t) => {
  const { adapter, setBatch } = await harness(t);
  setBatch(() =>
    guardedReply(
      {
        ok: false,
        completed: 2,
        failed: { index: 2, command: "solid.box", stage: "run", error: "a box with no height" },
        rolled_back: true,
        undone: 2,
        applied: [],
        not_run: [],
        results: [{ ok: true }, { ok: true }],
        documentStamp: STAMP_41,
      },
      true,
    ),
  );
  const result = await adapter.batch([
    { command: "solid.box", params: { length: 1, width: 2, height: 3 } },
    { command: "solid.box", params: { length: 4, width: 5, height: 6 } },
    { command: "solid.box", params: { length: 7, width: 8, height: 9 } },
  ]);
  assertConforms(result);
  assert.equal(result.outcome.state, "command_failed");
  assert.equal(result.outcome.completed, 2);
  assert.deepEqual(result.outcome.rollback, { requested: true, attempted: true, restored: true, undone: 2 });
  assert.equal(result.outcome.after?.revision, "41");
  assert.deepEqual(result.evidence.applied, []);
  assert.deepEqual(result.evidence.failed, {
    index: 2,
    command: "solid.box",
    stage: "run",
    error: "a box with no height",
  });
  assert.equal(adapter.stamp.revision, "41", "the rolled-back stamp is retained");
});

test("a partial batch without rollback keeps the applied step indices", async (t) => {
  const { adapter, setBatch } = await harness(t);
  setBatch(() =>
    guardedReply(
      {
        ok: false,
        completed: 2,
        failed: { index: 2, command: "solid.box", stage: "run", error: "bad box" },
        rolled_back: false,
        undone: 0,
        applied: [0, 1],
        not_run: [],
        results: [{ ok: true }, { ok: true }],
        documentStamp: STAMP_43,
      },
      true,
    ),
  );
  const result = await adapter.batch(
    [
      { command: "solid.box", params: { length: 1, width: 2, height: 3 } },
      { command: "solid.box", params: { length: 4, width: 5, height: 6 } },
      { command: "solid.box", params: { length: 7, width: 8, height: 9 } },
    ],
    { rollback: false },
  );
  assertConforms(result);
  assert.equal(result.outcome.state, "command_failed");
  assert.deepEqual(result.outcome.rollback, { requested: false, attempted: false, restored: false, undone: 0 });
  assert.deepEqual(result.evidence.applied, [0, 1]);
  assert.equal(result.outcome.after?.revision, "43");
  assert.equal(adapter.stamp.revision, "43");
});

test("a command failure keeps the native failure and does not claim an effect", async (t) => {
  const { adapter, setExecute } = await harness(t);
  setExecute(() => guardedReply({ error: "the sketch is not closed" }, true));
  const result = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(result);
  assert.equal(result.outcome.state, "command_failed");
  assert.equal(result.outcome.completed, 0);
  assert.equal(result.outcome.after, null);
  assert.equal(result.outcome.error?.code, "INTERNAL_ERROR");
  assert.match(result.outcome.error?.message ?? "", /sketch is not closed/u);
  assert.equal(adapter.stamp.revision, "41");

  setExecute(() => textReply("the engine refused the command", true));
  const plain = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(plain);
  assert.equal(plain.outcome.state, "command_failed");
  assert.equal(plain.outcome.completed, 0);
  assert.match(plain.outcome.error?.message ?? "", /engine refused/u);
});

test("a rejected guarded write is a definite pre-send failure", async (t) => {
  const { adapter, setExecute } = await harness(t);
  setExecute(() => wireErrorReply("INVALID_REQUEST", "unknown key `distance`", { field: "params" }));
  const result = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(result);
  assert.equal(result.outcome.state, "command_failed");
  assert.equal(result.outcome.completed, 0);
  assert.equal(result.outcome.error?.code, "INVALID_REQUEST");
  assert.deepEqual(result.outcome.error?.details, { field: "params" });
});

test("an expect mismatch keeps the built model and reports the failed check", async (t) => {
  const { adapter, setExecute } = await harness(t);
  setExecute(() =>
    guardedReply(
      {
        result: { built: true },
        documentStamp: STAMP_42,
        expectation: {
          ok: false,
          measured: { volume_mm3: 990, bodies: 1, bbox: { min: [0, 0, 0], max: [1, 1, 1] } },
          expected: { volume_mm3: 1000 },
          mismatch: ["volume: measured 990, expected 1000"],
        },
      },
      true,
    ),
  );
  const result = await adapter.execute("parameters.change", { name: "width" }, { expect: { volume_mm3: 1000 } });
  assertConforms(result);
  assert.equal(result.outcome.state, "expectation_failed");
  assert.equal(result.outcome.after?.revision, "42");
  assert.equal(result.outcome.completed, 1);
  assert.equal(result.outcome.error, null);
  assert.equal(result.outcome.expectation?.ok, false);
  assert.deepEqual(result.outcome.expectation?.mismatch, ["volume: measured 990, expected 1000"]);
  assert.deepEqual(result.evidence.native?.["result"], { built: true }, "the model remains built");
  assert.equal(adapter.stamp.revision, "42");
});

test("a missing expectation reply is not treated as a pass", async (t) => {
  const { adapter, setExecute } = await harness(t);
  setExecute(() => guardedReply({ result: { built: true }, documentStamp: STAMP_42 }));
  const result = await adapter.execute("parameters.change", { name: "width" }, { expect: { volume_mm3: 1000 } });
  assertConforms(result);
  assert.equal(result.outcome.state, "expectation_failed");
  assert.equal(result.outcome.after?.revision, "42");
  assert.equal(result.outcome.expectation?.ok, false);
  assert.match(result.outcome.expectation?.mismatch.join(" ") ?? "", /did not return an expectation/u);
  assert.equal(result.evidence.expectationUnchecked, true);
});

test("still running is reconciled on the same connection without resending the mutation", async (t) => {
  const { peer, adapter, setExecute, setPending } = await harness(t);
  setExecute(() =>
    guardedReply(
      {
        error: "still running: parameters.change",
        pending: true,
        expectation: null,
        note: "retrieve the original result with pending; do not resend it",
      },
      true,
    ),
  );
  let pendingCalls = 0;
  setPending(() => {
    pendingCalls += 1;
    if (pendingCalls === 1) {
      return textReply({ pending: "parameters.change", done: false, running_s: 2, note: "still running" });
    }
    return textReply({
      pending: "parameters.change",
      done: true,
      result: { result: { changed: true }, documentStamp: STAMP_42 },
      expectation: null,
      note: "original result",
    });
  });
  const first = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(first);
  assert.equal(first.outcome.state, "pending");
  assert.equal(first.outcome.completed, null);
  assert.equal(first.outcome.error, null);
  assert.equal(adapter.requiresReconciliation, true);

  const second = await adapter.reconcile();
  assert.ok(second !== null);
  assertConforms(second);
  assert.equal(second.outcome.state, "applied");
  assert.equal(second.outcome.after?.revision, "42");
  const executeCall = peer.toolCallsNamed("execute")[0];
  assert.equal(second.outcome.request.requestId, `mcp-${String(executeCall?.id)}`);
  assert.equal(peer.toolCallsNamed("execute").length, 1, "the mutation is never resent");
  assert.equal(peer.toolCallsNamed("pending").length, 2);
  const pendingIds = peer.toolCallsNamed("pending").map((call) => call.id);
  assert.ok(pendingIds.every((id) => id !== executeCall?.id), "the pending query has its own transport id");
  assert.equal(adapter.requiresReconciliation, false);
  assert.equal(adapter.stamp.revision, "42");
  assert.equal(await adapter.reconcile(), null);
});

test("a lost pending slot is unknown, blocks new writes and is never resubmitted", async (t) => {
  const { peer, adapter, setExecute, setPending } = await harness(t);
  setExecute(() => guardedReply({ error: "still running: batch", pending: true }, true));
  setPending(() => textReply({ pending: "batch", lost: true, outcomeUnknown: true }, true));

  const first = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(first);
  assert.equal(first.outcome.state, "unknown");
  assert.equal(first.outcome.completed, null);
  assert.equal(first.outcome.after, null);
  assert.equal(first.outcome.error?.code, "OUTCOME_UNKNOWN");
  assert.equal(adapter.requiresReconciliation, true);

  const blocked = await adapter.execute("parameters.change", { name: "height" });
  assertConforms(blocked);
  assert.equal(blocked.outcome.state, "command_failed");
  assert.equal(blocked.outcome.completed, 0);
  assert.equal(blocked.outcome.error?.code, "OUTCOME_UNKNOWN");
  assert.equal(peer.toolCallsNamed("execute").length, 1, "no replacement mutation is sent");
  assert.throws(
    () => adapter.adoptStamp(STAMP_42),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "OUTCOME_UNKNOWN",
  );

  const again = await adapter.reconcile();
  assert.ok(again !== null);
  assertConforms(again);
  assert.equal(again.outcome.state, "unknown");
  assert.equal(peer.toolCallsNamed("execute").length, 1);
});

test("a disconnect after submission returns unknown and admits no replacement write", async (t) => {
  const { peer, adapter, setExecute } = await harness(t);
  const arrived = deferred<void>();
  const held = deferred<void>();
  setExecute(async () => {
    arrived.resolve();
    await held.promise;
    return guardedReply({ result: {}, documentStamp: STAMP_42 });
  });
  const pendingWrite = adapter.execute("parameters.change", { name: "width" });
  await arrived.promise;
  peer.disconnect("the app closed");
  const result = await pendingWrite;
  assertConforms(result);
  assert.equal(result.outcome.state, "unknown");
  assert.equal(result.outcome.completed, null);
  assert.equal(result.outcome.after, null);
  assert.equal(result.outcome.error?.code, "OUTCOME_UNKNOWN");
  assert.equal(result.outcome.request.requestId, `mcp-${String(peer.toolCallsNamed("execute")[0]?.id)}`);
  assert.equal(peer.toolCallsNamed("execute").length, 1);

  const blocked = await adapter.execute("parameters.change", { name: "height" });
  assertConforms(blocked);
  assert.equal(blocked.outcome.state, "command_failed");
  assert.equal(blocked.outcome.error?.code, "OUTCOME_UNKNOWN");
  assert.equal(peer.toolCallsNamed("execute").length, 1, "a lost connection never resends");

  const reconciled = await adapter.reconcile();
  assert.ok(reconciled !== null);
  assertConforms(reconciled);
  assert.equal(reconciled.outcome.state, "unknown");
  assert.equal(peer.toolCallsNamed("execute").length, 1);
  held.resolve();
});

test("a stale stamp returns the current stamp and control for re-inspection", async (t) => {
  const { peer, adapter, setExecute } = await harness(t);
  peer.revision = 42n;
  setExecute(() => wireErrorReply("STALE_DOCUMENT", "the design moved", { expected: STAMP_41, current: STAMP_42 }));
  const stale = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(stale);
  assert.equal(stale.outcome.state, "stale");
  assert.equal(stale.outcome.completed, 0);
  assert.equal(stale.outcome.after?.revision, "42");
  assert.equal(stale.outcome.error?.code, "STALE_DOCUMENT");
  assert.deepEqual(stale.outcome.error?.details, { expected: STAMP_41, current: STAMP_42 });
  assert.equal(adapter.stamp.revision, "41", "stale control is returned; the stamp is not silently replaced");

  const inspection = await adapter.inspect();
  assert.equal(inspection.stamp.revision, "42");
  assert.equal(adapter.stamp.revision, "42");

  setExecute((args) => {
    assert.deepEqual(args["expectedDocument"], STAMP_42);
    return guardedReply({ result: {}, documentStamp: STAMP_43 });
  });
  const applied = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(applied);
  assert.equal(applied.outcome.state, "applied");
  const mutations = peer.toolCallsNamed("execute").filter((call) => call.args["command"] !== "document.revision");
  assert.equal(mutations.length, 2);
});

test("a stale closed document reports a null current stamp", async (t) => {
  const { adapter, setExecute } = await harness(t);
  setExecute(() => wireErrorReply("STALE_DOCUMENT", "the design was closed", { expected: STAMP_41, current: null }));
  const result = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(result);
  assert.equal(result.outcome.state, "stale");
  assert.equal(result.outcome.after, null);
  assert.deepEqual(result.outcome.error?.details, { expected: STAMP_41, current: null });
});

test("cancellation while a read holds the connection stops the queued write before it is sent", async (t) => {
  const { peer, adapter } = await harness(t);
  const arrived = deferred<void>();
  const held = deferred<void>();
  peer.setHandler("inspect_design", async () => {
    arrived.resolve();
    await held.promise;
    return textReply({ name: "fixture" });
  });
  const read = adapter.readTool("inspect_design");
  await arrived.promise;
  const write = adapter.execute("parameters.change", { name: "width" });
  await tick();
  adapter.cancel("user pressed stop");
  const result = await write;
  assertConforms(result);
  assert.equal(result.outcome.state, "cancelled");
  assert.equal(result.outcome.completed, 0);
  assert.equal(result.outcome.error?.code, "CANCELLED");
  assert.equal(peer.toolCallsNamed("execute").length, 0);
  held.resolve();
  await read;
});

test("cancelling a queued write does not send it, while an applied in-flight write is reported", async (t) => {
  const { peer, adapter, setExecute } = await harness(t);
  const arrived = deferred<void>();
  const held = deferred<void>();
  setExecute(async () => {
    arrived.resolve();
    await held.promise;
    return guardedReply({ result: {}, documentStamp: STAMP_42 });
  });
  const first = adapter.execute("parameters.change", { name: "first" });
  await arrived.promise;
  const second = adapter.execute("parameters.change", { name: "second" });
  await tick();
  adapter.cancel("user pressed stop");
  const cancelled = await second;
  assertConforms(cancelled);
  assert.equal(cancelled.outcome.state, "cancelled");
  assert.equal(cancelled.outcome.completed, 0);
  held.resolve();
  const applied = await first;
  assertConforms(applied);
  assert.equal(applied.outcome.state, "applied");
  assert.equal(applied.outcome.after?.revision, "42");
  assert.equal(peer.toolCallsNamed("execute").length, 1, "only the in-flight write reached the app");

  const later = await adapter.execute("parameters.change", { name: "third" });
  assertConforms(later);
  assert.equal(later.outcome.state, "cancelled");
  assert.equal(peer.toolCallsNamed("execute").length, 1, "no write is scheduled after a terminal turn");
});

test("reads are bounded to four at once", async (t) => {
  const { peer, adapter } = await harness(t);
  const release = deferred<void>();
  peer.setHandler("inspect_design", async () => {
    await release.promise;
    return textReply({ name: "fixture" });
  });
  const reads = Array.from({ length: 6 }, () => adapter.readTool("inspect_design"));
  await waitFor(() => peer.toolCallsNamed("inspect_design").length >= 4);
  assert.equal(peer.toolCallsNamed("inspect_design").length, 4);
  assert.equal(peer.maxConcurrentToolCalls, 4);
  release.resolve();
  await Promise.all(reads);
  assert.equal(peer.toolCallsNamed("inspect_design").length, 6);
  assert.ok(peer.maxConcurrentToolCalls <= 4);
});

test("reads wait behind an unresolved mutation", async (t) => {
  const { peer, adapter, setExecute } = await harness(t);
  const arrived = deferred<void>();
  const held = deferred<void>();
  setExecute(async () => {
    arrived.resolve();
    await held.promise;
    return guardedReply({ result: {}, documentStamp: STAMP_42 });
  });
  const write = adapter.execute("parameters.change", { name: "width" });
  await arrived.promise;
  const read = adapter.readTool("inspect_design");
  await tick();
  await tick();
  assert.equal(peer.toolCallsNamed("inspect_design").length, 0, "the read must not overtake the mutation");
  const executeSequence = peer.toolCallsNamed("execute")[0]?.sequence ?? 0;
  held.resolve();
  await write;
  await read;
  const readSequence = peer.toolCallsNamed("inspect_design")[0]?.sequence ?? 0;
  assert.ok(readSequence > executeSequence);
});

test("the adapter refuses a connection bound to another bridge or chat turn", async (t) => {
  const { client } = await harness(t);
  assert.throws(
    () =>
      new CadAdapter({
        client,
        turn: {
          bridgeId: "bridge-2",
          sessionId: "s-1",
          turnId: "t-1",
          workflowRunId: "w-1",
          document: STAMP_41,
        },
        writeTimeoutMs: 1_000,
      }),
    (error: unknown) =>
      error instanceof ProtocolError && error.value.code === "INVALID_REQUEST" && error.value.details["field"] === "turn",
  );
});

test("a missing execute tool is an actionable capability error with zero writes", async (t) => {
  const peer = new FakeMcpPeer({ tools: defaultTools().filter((tool) => tool.name !== "execute") });
  const client = new McpClient({
    channel: peer,
    bridge: { name: "solidcraft-app", bridgeId: "bridge-1" },
    turn: { sessionId: "s-1", turnId: "t-1", workflowRunId: "w-1" },
    mcpSessionId: "mcp-session-fixture",
    requestTimeoutMs: 1_000,
  });
  await client.connect();
  const adapter = new CadAdapter({
    client,
    turn: { bridgeId: "bridge-1", sessionId: "s-1", turnId: "t-1", workflowRunId: "w-1", document: STAMP_41 },
    writeTimeoutMs: 1_000,
    readTimeoutMs: 1_000,
  });
  t.after(() => adapter.dispose());
  assert.equal(adapter.capabilities().guardedExecute, false);
  const result = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(result);
  assert.equal(result.outcome.state, "command_failed");
  assert.equal(result.outcome.error?.code, "UNSUPPORTED_CAPABILITY");
  assert.deepEqual(result.outcome.error?.details, { capability: "mcp-tool:execute" });
  assert.equal(peer.toolCallsNamed("execute").length, 0);
});

test("an unguarded execute tool is refused rather than used without a stamp", async (t) => {
  const peer = new FakeMcpPeer({ guarded: false });
  const client = new McpClient({
    channel: peer,
    bridge: { name: "solidcraft-app", bridgeId: "bridge-1" },
    turn: { sessionId: "s-1", turnId: "t-1", workflowRunId: "w-1" },
    mcpSessionId: "mcp-session-fixture",
    requestTimeoutMs: 1_000,
  });
  await client.connect();
  const adapter = new CadAdapter({
    client,
    turn: { bridgeId: "bridge-1", sessionId: "s-1", turnId: "t-1", workflowRunId: "w-1", document: STAMP_41 },
    writeTimeoutMs: 1_000,
  });
  t.after(() => adapter.dispose());
  assert.equal(adapter.capabilities().guardedExecute, false);
  const result = await adapter.execute("parameters.change", { name: "width" });
  assertConforms(result);
  assert.equal(result.outcome.state, "command_failed");
  assert.deepEqual(result.outcome.error?.details, { capability: "guarded-execute" });
  assert.equal(peer.toolCallsNamed("execute").length, 0);
});

test("missing, mismatched and untrusted command schemas never guess arguments", async (t) => {
  const { peer, adapter } = await harness(t);
  const missing = await adapter.execute("solid.unconverted", { length: 1 });
  assertConforms(missing);
  assert.equal(missing.outcome.state, "command_failed");
  assert.deepEqual(missing.outcome.error?.details, { capability: "command-schema:solid.unconverted" });
  assert.equal(peer.toolCallsNamed("execute").length, 0);

  const mismatch = await adapter.execute("solid.mismatch", { radius: "wide" });
  assertConforms(mismatch);
  assert.equal(mismatch.outcome.state, "command_failed");
  assert.equal(mismatch.outcome.error?.code, "INVALID_REQUEST");
  assert.equal(peer.toolCallsNamed("execute").length, 0);

  const unknown = await adapter.execute("does.not.exist", {});
  assertConforms(unknown);
  assert.deepEqual(unknown.outcome.error?.details, { capability: "command:does.not.exist" });
  assert.equal(peer.toolCallsNamed("execute").length, 0);

  peer.commands.set("solid.bad", { id: "solid.bad", input: { $ref: "#/$defs/does-not-exist" } });
  const broken = await adapter.execute("solid.bad", {});
  assertConforms(broken);
  assert.deepEqual(broken.outcome.error?.details, { capability: "command-schema:solid.bad" });
  assert.equal(peer.toolCallsNamed("execute").length, 0);
});

test("tool-level argument errors are refused before any write", async (t) => {
  const { peer, adapter } = await harness(t);
  const badExpect = await adapter.execute("parameters.change", { name: "width" }, { expect: { volume_mm3: "large" } });
  assertConforms(badExpect);
  assert.equal(badExpect.outcome.state, "command_failed");
  assert.equal(badExpect.outcome.error?.code, "INVALID_REQUEST");
  assert.equal(badExpect.outcome.error?.details["field"], "arguments");

  const reserved = await adapter.execute("parameters.change", { name: "width", expectedDocument: STAMP_41 });
  assertConforms(reserved);
  assert.equal(reserved.outcome.error?.code, "INVALID_REQUEST");
  assert.equal(peer.toolCallsNamed("execute").length, 0);
  assert.equal(peer.toolCallsNamed("batch").length, 0);
});

test("readTool refuses a write-capable tool instead of using it as a read", async (t) => {
  const { peer, adapter } = await harness(t);
  await assert.rejects(
    () => adapter.readTool("execute", {}),
    (error: unknown) =>
      error instanceof ProtocolError &&
      error.value.code === "UNSUPPORTED_CAPABILITY" &&
      error.value.details["capability"] === "read-tool:execute",
  );
  assert.equal(peer.toolCalls.length, 0);
});

test("batch steps are exactly {command, params}", async (t) => {
  const { peer, adapter } = await harness(t);
  const extra = await adapter.batch([
    { command: "solid.box", params: { length: 1, width: 2, height: 3 }, verbose: true } as unknown as CadBatchStep,
  ]);
  assertConforms(extra);
  assert.equal(extra.outcome.state, "command_failed");
  assert.equal(extra.outcome.error?.code, "INVALID_REQUEST");
  const nullParams = await adapter.batch([
    { command: "solid.box", params: null } as unknown as CadBatchStep,
  ]);
  assertConforms(nullParams);
  assert.equal(nullParams.outcome.state, "command_failed");
  assert.equal(nullParams.outcome.error?.code, "INVALID_REQUEST");
  assert.equal(peer.toolCallsNamed("batch").length, 0);
});

test("a batch is refused before sending when a step has no discoverable schema", async (t) => {
  const { peer, adapter } = await harness(t);
  const result = await adapter.batch([{ command: "solid.unconverted", params: {} }]);
  assertConforms(result);
  assert.equal(result.outcome.state, "command_failed");
  assert.equal(result.outcome.completed, 0);
  assert.deepEqual(result.outcome.error?.details, { capability: "command-schema:solid.unconverted" });
  assert.equal(peer.toolCallsNamed("batch").length, 0);
});

test("a speculative candidate probe is unsupported and performs zero writes", async (t) => {
  const { peer, adapter } = await harness(t);
  assert.throws(
    () => adapter.probe(),
    (error: unknown) =>
      error instanceof ProtocolError &&
      error.value.code === "UNSUPPORTED_CAPABILITY" &&
      error.value.details["capability"] === "speculative-candidate-probe",
  );
  assert.equal(peer.toolCalls.length, 0, "a probe never edits, undoes or otherwise calls the document");
});

test("document revision decoding keeps u64 values above 2^53 exact", async (t) => {
  const { peer, adapter } = await harness(t);
  peer.revision = 9_007_199_254_740_993n;
  assert.deepEqual(await adapter.readDocumentStamp(), { documentId: "doc-A", revision: "9007199254740993" });
  peer.revision = 18_446_744_073_709_551_615n;
  assert.deepEqual(await adapter.readDocumentStamp(), { documentId: "doc-A", revision: "18446744073709551615" });
  assert.equal(readTopLevelUnsignedInteger('{"meta":{"revision":1},"document_id":"doc-A","revision":42}', "revision"), "42");
  assert.equal(readTopLevelUnsignedInteger('{"revision": 1.5}', "revision"), null);
});

test("the section 7 schema validator is the repository bundle, not a local copy", () => {
  const applied: WriteOutcome = {
    state: "applied",
    request: { bridgeId: "bridge-1", mcpSessionId: "mcp-session-fixture", requestId: "mcp-1" },
    before: STAMP_41,
    after: STAMP_42,
    completed: 1,
    rollback: { requested: false, attempted: false, restored: null, undone: null },
    expectation: null,
    error: null,
  };
  assert.equal(bundle.validate("#/$defs/WriteOutcome", applied), true);
  const rounded: Record<string, unknown> = { ...applied, after: { documentId: "doc-A", revision: 42 } };
  assert.equal(bundle.validate("#/$defs/WriteOutcome", rounded), false, "a numeric revision is not a wire stamp");
});
