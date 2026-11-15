import assert from "node:assert/strict";
import { test } from "node:test";

import { ProtocolError } from "../src/errors.js";
import { SessionHistory } from "../src/history.js";
import { canonicalize } from "../src/jcs.js";
import { DEFAULT_LIMITS, type Limits } from "../src/limits.js";
import { utf8Bytes } from "../src/protocol.js";
import type { ModelMessage } from "../src/host-ai/model-client.js";

function round(text: string): ModelMessage[] {
  return [
    { role: "user", content: [{ type: "text", text }], toolCalls: [], toolCallId: null },
    { role: "assistant", content: [{ type: "text", text: "ack" }], toolCalls: [], toolCallId: null },
  ];
}

const limits: Limits = DEFAULT_LIMITS;

test("normalized history trims only complete old rounds before appending a new round", () => {
  const history = new SessionHistory();
  // Each round is about 2 MiB of canonical text; ten rounds cross 16 MiB.
  const chunk = "x".repeat(1_000_000);
  for (let index = 0; index < 20; index += 1) {
    history.appendRound(round(`${index}:${chunk}`), limits);
  }
  assert.ok(history.byteSize <= limits.historyBytes, `byte size ${history.byteSize}`);
  assert.ok(history.length <= limits.historyMessages);
  assert.ok(history.roundCount < 20, "older complete rounds must be trimmed");
  assert.equal(history.messages.length, history.roundCount * 2);
  for (let index = 0; index < history.messages.length; index += 2) {
    assert.equal(history.messages[index]?.role, "user");
    assert.equal(history.messages[index + 1]?.role, "assistant");
  }
  // The stored byte counter is the true size of the retained messages.
  const recalculated = history.messages.reduce((sum, message) => sum + utf8Bytes(canonicalize(message)), 0);
  assert.equal(history.byteSize, recalculated);
});

test("a round larger than historyBytes fails atomically and leaves the invariant intact", () => {
  const history = new SessionHistory();
  history.appendRound(round("small"), limits);
  const beforeMessages = history.messages.map((message) => JSON.stringify(message));
  const beforeBytes = history.byteSize;
  const oversized: ModelMessage[] = [
    { role: "user", content: [{ type: "text", text: "y".repeat(9 * 1024 * 1024) }], toolCalls: [], toolCallId: null },
    { role: "assistant", content: [{ type: "text", text: "z".repeat(9 * 1024 * 1024) }], toolCalls: [], toolCallId: null },
  ];
  assert.throws(
    () => history.appendRound(oversized, limits),
    (error: unknown) =>
      error instanceof ProtocolError &&
      error.value.code === "RESOURCE_EXHAUSTED" &&
      error.value.details["resource"] === "historyBytes" &&
      error.value.details["limit"] === limits.historyBytes,
  );
  assert.equal(history.byteSize, beforeBytes);
  assert.deepEqual(
    history.messages.map((message) => JSON.stringify(message)),
    beforeMessages,
  );
  assert.ok(history.byteSize <= limits.historyBytes);
  assert.equal(history.roundCount, 1);
});

test("admission rejects a user message that cannot fit after trimming", () => {
  const history = new SessionHistory();
  const huge: ModelMessage[] = [{ role: "user", content: [{ type: "text", text: "q".repeat(17 * 1024 * 1024) }], toolCalls: [], toolCallId: null }];
  assert.equal(history.blockedBy(huge, limits), "historyBytes");
  assert.equal(history.canAppend(huge, limits), false);
  assert.equal(history.byteSize, 0);
  assert.equal(history.length, 0);
});
