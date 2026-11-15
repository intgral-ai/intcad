import assert from "node:assert/strict";
import { test } from "node:test";
import type { Writable } from "node:stream";

import { ProtocolError } from "../src/errors.js";
import { isControlFrame, NdjsonTransport } from "../src/transport.js";
import type { Frame, Scope } from "../src/protocol.js";

class FakeOutput {
  writableLength = 0;
  readonly chunks: Buffer[] = [];
  write(bytes: Buffer): boolean {
    this.chunks.push(bytes);
    this.writableLength += bytes.length;
    return true;
  }
  once(): this {
    return this;
  }
  end(): void {
    this.writableLength = 0;
  }
}

const scope: Scope = { sessionId: "s", turnId: "t", workflowRunId: "w" };
const cancel: Frame = {
  protocolVersion: "1.0",
  kind: "request",
  requestId: "h:cancel",
  scope,
  method: "turn.cancel",
  params: { reason: "user" },
};
const controlShutdown: Frame = {
  protocolVersion: "1.0",
  kind: "request",
  requestId: "h:shutdown",
  scope: { sessionId: null, turnId: null, workflowRunId: null },
  method: "runtime.shutdown",
  params: { reason: "app_exit" },
};
const terminal: Frame = {
  protocolVersion: "1.0",
  kind: "event",
  requestId: "h:turn",
  scope,
  sequence: 1,
  event: "turn.finished",
  data: {
    state: "cancelled",
    reason: "cancelled",
    document: { documentId: "doc", revision: "1" },
    changes: "none",
    validationRunId: null,
    report: null,
  },
};

function transport(output: FakeOutput): NdjsonTransport {
  return new NdjsonTransport(
    { on: () => undefined, pause: () => undefined } as never,
    output as unknown as Writable,
    { frameBytes: 8_388_608, queueBytes: 1_000_000, controlLaneBytes: 100_000 },
    { onFrame: () => undefined, onViolation: () => undefined, onEnd: () => undefined },
  );
}

test("cancel and terminal frames keep their reserved control lane under data backpressure", () => {
  const output = new FakeOutput();
  const lane = transport(output);
  assert.equal(isControlFrame(cancel), true);
  assert.equal(isControlFrame(controlShutdown), true);
  assert.equal(isControlFrame(terminal), true);

  // Fill the data lane to its edge, leaving the control lane free.
  output.writableLength = 900_000 - 2;
  assert.throws(
    () => lane.write({ ...cancel, kind: "event", event: "workflow.text", params: undefined } as unknown as Frame),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "RESOURCE_EXHAUSTED",
  );
  const before = output.chunks.length;
  lane.write(cancel);
  lane.write(controlShutdown);
  lane.write(terminal);
  assert.equal(output.chunks.length, before + 3);
  assert.ok(output.chunks.length >= 3);
});

test("control frames still respect the total queue ceiling", () => {
  const output = new FakeOutput();
  const lane = transport(output);
  output.writableLength = 1_000_000;
  assert.throws(
    () => lane.write(cancel),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "RESOURCE_EXHAUSTED",
  );
});
