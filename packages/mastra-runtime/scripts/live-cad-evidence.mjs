#!/usr/bin/env node
/**
 * Live evidence: drive the real Rust `solidcraft-cli mcp --headless` server
 * through the same TypeScript MCP client and CAD adapter the runtime uses.
 *
 * This is not part of `npm test` (CI has no Rust binary). Run it after
 * `cargo build -p solidcraft-cli`:
 *
 *   SOLIDCRAFT_CLI=path/to/solidcraft-cli node scripts/live-cad-evidence.mjs
 *
 * It prints one JSON record: the discovered tools, the initial stamp, a real
 * guarded two-step batch, its measurement, one guarded `edit.undo`, and the
 * measurement afterwards. It exits non-zero if any of those expectations fail.
 */

import { spawn } from "node:child_process";

import { CadAdapter } from "../dist/src/solidcraft/cad-adapter.js";
import { McpClient, streamChannel } from "../dist/src/solidcraft/mcp-client.js";

const cli = process.env["SOLIDCRAFT_CLI"];
if (cli === undefined || cli.length === 0) {
  process.stderr.write("live-cad-evidence: set SOLIDCRAFT_CLI to the built solidcraft-cli binary\n");
  process.exit(2);
}

const child = spawn(cli, ["mcp", "--headless"], { stdio: ["pipe", "pipe", "inherit"] });
const turn = {
  sessionId: "session-live",
  turnId: "turn-live",
  workflowRunId: "workflow-live",
};
const client = new McpClient({
  channel: streamChannel(child.stdout, child.stdin),
  bridge: { name: "solidcraft-app", bridgeId: "bridge-live" },
  turn,
  mcpSessionId: "mcp-session-live",
  requestTimeoutMs: 120_000,
  handshakeTimeoutMs: 20_000,
});
const adapter = new CadAdapter({
  client,
  turn: {
    bridgeId: "bridge-live",
    ...turn,
    document: { documentId: "doc-live", revision: "0" },
  },
  writeTimeoutMs: 120_000,
  readTimeoutMs: 120_000,
});

function close(code) {
  adapter.dispose();
  child.kill();
  process.exitCode = code;
}

try {
  const connection = await client.connect();
  const inspection = await adapter.inspect({ design: false });
  const batch = await adapter.batch([
    { command: "solid.box", params: { length: 40, width: 30, height: 20 } },
    {
      command: "solid.cylinder",
      params: { radius: 5, height: 30, base: [20, 15, -5], operation: "cut", name: "LiveHole" },
    },
  ]);
  const afterBatch = await adapter.readTool("measure", {});
  const undo = await adapter.execute("edit.undo", {});
  const afterUndo = await adapter.readTool("measure", {});
  const single = await adapter.batch(
    [{ command: "solid.box", params: { length: 40, width: 30, height: 20 } }],
    { expect: { volume_mm3: 48000, bodies: 2, tol: 0.002 } },
  );
  const afterSingle = await adapter.readTool("measure", {});
  const undoSingle = await adapter.execute("edit.undo", {});
  const afterUndoSingle = await adapter.readTool("measure", {});

  // Simulate another client editing the same design: the adapter's next
  // guarded write must be refused as stale and return control for inspection.
  const beforeStale = await adapter.inspect({ design: false });
  await client.callTool("execute", {
    command: "solid.box",
    params: { length: 10, width: 10, height: 10, name: "OtherClientBox" },
  });
  const stale = await adapter.execute("solid.box", { length: 20, width: 20, height: 20 });
  const reinspected = await adapter.inspect({ design: false });
  const recovered = await adapter.execute("solid.box", { length: 20, width: 20, height: 20 });

  // A real two-step batch whose second step cannot run: the checkpoint rollback
  // must take the first step back and report the applied/undone facts.
  const beforeFailure = await adapter.readTool("measure", {});
  const failed = await adapter.batch([
    { command: "solid.box", params: { length: 10, width: 10, height: 10, name: "RolledBackBox" } },
    { command: "solid.fillet", params: { edges: [[100, 100, 100]], radius: 1 } },
  ]);
  const afterFailure = await adapter.readTool("measure", {});

  const volume = (read) => {
    const value = read.value ?? {};
    const total = value["total"];
    return total && typeof total["volume_mm3"] === "number" ? total["volume_mm3"] : null;
  };
  const bodyCount = (read) => {
    const value = read.value ?? {};
    return typeof value["body_count"] === "number" ? value["body_count"] : null;
  };

  const evidence = {
    cli,
    mcpProtocolVersion: connection.protocolVersion,
    tools: connection.tools.map((tool) => tool.name),
    initialStamp: inspection.stamp,
    batch: {
      state: batch.outcome.state,
      before: batch.outcome.before,
      after: batch.outcome.after,
      completed: batch.outcome.completed,
      rollback: batch.outcome.rollback,
      error: batch.outcome.error,
      applied: batch.evidence.applied,
    },
    measuredAfterBatch: { volume_mm3: volume(afterBatch), bodyCount: bodyCount(afterBatch) },
    undo: {
      state: undo.outcome.state,
      before: undo.outcome.before,
      after: undo.outcome.after,
      completed: undo.outcome.completed,
      error: undo.outcome.error,
    },
    measuredAfterUndo: { volume_mm3: volume(afterUndo), bodyCount: bodyCount(afterUndo) },
    oneStepBatch: {
      state: single.outcome.state,
      completed: single.outcome.completed,
      before: single.outcome.before,
      after: single.outcome.after,
      expectation: single.outcome.expectation,
    },
    measuredAfterOneStepBatch: { volume_mm3: volume(afterSingle), bodyCount: bodyCount(afterSingle) },
    oneStepUndo: {
      state: undoSingle.outcome.state,
      before: undoSingle.outcome.before,
      after: undoSingle.outcome.after,
    },
    measuredAfterOneStepUndo: { volume_mm3: volume(afterUndoSingle), bodyCount: bodyCount(afterUndoSingle) },
    stale: {
      before: beforeStale.stamp,
      state: stale.outcome.state,
      expected: stale.outcome.error?.details?.["expected"],
      current: stale.outcome.error?.details?.["current"],
      after: stale.outcome.after,
    },
    reinspected: reinspected.stamp,
    recovered: {
      state: recovered.outcome.state,
      before: recovered.outcome.before,
      after: recovered.outcome.after,
    },
    failedBatch: {
      state: failed.outcome.state,
      completed: failed.outcome.completed,
      rollback: failed.outcome.rollback,
      applied: failed.evidence.applied,
      failedStep: failed.evidence.failed,
      error: failed.outcome.error,
      before: failed.outcome.before,
      after: failed.outcome.after,
    },
    measuredBeforeFailure: { volume_mm3: volume(beforeFailure), bodyCount: bodyCount(beforeFailure) },
    measuredAfterFailure: { volume_mm3: volume(afterFailure), bodyCount: bodyCount(afterFailure) },
  };
  process.stdout.write(`${JSON.stringify(evidence, null, 2)}\n`);

  const failures = [];
  if (batch.outcome.state !== "applied") failures.push(`batch state is ${batch.outcome.state}`);
  if (batch.outcome.completed !== 2) failures.push(`batch completed ${String(batch.outcome.completed)}, expected 2`);
  if (batch.outcome.after === null) failures.push("batch returned no resulting document stamp");
  if (!(volume(afterBatch) > 0)) failures.push("the batch produced no measurable volume");
  if (undo.outcome.state !== "applied") failures.push(`undo state is ${undo.outcome.state}`);
  if (bodyCount(afterUndo) !== 1) failures.push(`one undo left ${String(bodyCount(afterUndo))} bodies, expected the box`);
  if (!(Math.abs((volume(afterUndo) ?? 0) - 24000) < 1e-6)) failures.push(`one undo left volume ${String(volume(afterUndo))}, expected 24000`);
  if (single.outcome.state !== "applied" || single.outcome.completed !== 1) failures.push("the one-step batch did not apply");
  if (single.outcome.expectation?.ok !== true) failures.push("the real expectation check did not pass");
  if (undoSingle.outcome.state !== "applied") failures.push(`one-step undo state is ${undoSingle.outcome.state}`);
  if (bodyCount(afterUndoSingle) !== 1) failures.push(`one-step undo left ${String(bodyCount(afterUndoSingle))} bodies, expected the prior box`);
  if (!(Math.abs((volume(afterUndoSingle) ?? 0) - 24000) < 1e-6)) failures.push(`one-step undo left volume ${String(volume(afterUndoSingle))}, expected 24000`);
  if (stale.outcome.state !== "stale") failures.push(`the stale write state is ${stale.outcome.state}`);
  if (stale.outcome.after === null) failures.push("the stale refusal returned no current stamp");
  if (reinspected.stamp.revision !== stale.outcome.after?.revision) failures.push("re-inspection did not establish the current stamp");
  if (recovered.outcome.state !== "applied") failures.push(`the re-planned write state is ${recovered.outcome.state}`);
  if (failed.outcome.state !== "command_failed") failures.push(`the failed batch state is ${failed.outcome.state}`);
  if (failed.outcome.completed !== 1) failures.push(`the failed batch completed ${String(failed.outcome.completed)}, expected 1`);
  if (failed.outcome.rollback.restored !== true) failures.push("the failed batch did not report a restored checkpoint");
  if (failed.outcome.rollback.undone !== 1) failures.push(`the failed batch undid ${String(failed.outcome.rollback.undone)}, expected 1`);
  if (failed.evidence.applied.length !== 0) failures.push("the rolled-back batch claims applied steps");
  if (Math.abs((volume(beforeFailure) ?? 0) - (volume(afterFailure) ?? 0)) > 1e-6) failures.push("the rollback changed the measured volume");
  if (failures.length > 0) {
    process.stderr.write(`live-cad-evidence: ${failures.join("; ")}\n`);
    close(1);
  } else {
    close(0);
  }
} catch (error) {
  process.stderr.write(`live-cad-evidence: ${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
  close(1);
}
