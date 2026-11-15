import assert from "node:assert/strict";
import { test } from "node:test";

import { ProtocolError, redactedMessage } from "../src/errors.js";
import { DEFAULT_LIMITS } from "../src/limits.js";
import { HostAiAgentClient, AGENT_RUN_CAPABILITY } from "../src/host-ai/agent-client.js";
import {
  HostAiModelClient,
  type HostAiTransport,
  type ModelGenerateRequest,
  type ModelGenerateResult,
} from "../src/host-ai/model-client.js";
import type { JsonObject, Scope } from "../src/protocol.js";

class RecordingTransport implements HostAiTransport {
  readonly calls: { method: string; scope: Scope; params: JsonObject; timeoutSeconds: number }[] = [];
  #next: () => JsonObject | Promise<JsonObject>;

  constructor(next: () => JsonObject | Promise<JsonObject>) {
    this.#next = next;
  }

  async call(method: string, scope: Scope, params: JsonObject, timeoutSeconds: number): Promise<JsonObject> {
    this.calls.push({ method, scope, params, timeoutSeconds });
    return this.#next();
  }
}

function result(overrides: Partial<ModelGenerateResult> = {}): JsonObject {
  return {
    content: [{ type: "text", text: "ok" }],
    toolCalls: [],
    finishReason: "stop",
    usage: null,
    continuation: null,
    delivery: "buffered",
    ...overrides,
  } as JsonObject;
}

const request: ModelGenerateRequest = {
  messages: [{ role: "user", content: [{ type: "text", text: "hello" }], toolCalls: [], toolCallId: null }],
  tools: [
    {
      name: "set_width",
      description: "Propose a width.",
      inputSchema: { type: "object", properties: { width: { type: "number" } } },
    },
  ],
  options: {
    delivery: "buffered",
    timeoutSeconds: 300,
    maxOutputTokens: 2048,
    temperature: null,
    reasoningEffort: null,
    outputSchema: null,
  },
  continuation: null,
};

test("model.generate sends one buffered HostAi request and validates the full reply", async () => {
  const transport = new RecordingTransport(() => result({ continuation: "cont-1" }));
  const client = new HostAiModelClient(
    transport,
    {
      sessionId: "s",
      turnId: "t",
      workflowRunId: "w",
      connectionId: "byok-1",
      modelId: "model-a",
      remainingSeconds: 1200,
    },
    DEFAULT_LIMITS,
  );
  const reply = await client.generate(request, new AbortController().signal);
  assert.equal(transport.calls.length, 1);
  assert.equal(transport.calls[0]?.method, "model.generate");
  assert.deepEqual(transport.calls[0]?.scope, { sessionId: "s", turnId: "t", workflowRunId: "w" });
  assert.equal((transport.calls[0]?.params["options"] as JsonObject)["delivery"], "buffered");
  assert.equal(transport.calls[0]?.params["continuation"], null);
  assert.equal(reply.delivery, "buffered");
  assert.equal(reply.continuation, "cont-1");
  assert.equal(client.calls, 1);
});

test("unsupported structured output and streaming fail before any HostAi call", async () => {
  const transport = new RecordingTransport(() => result());
  const client = new HostAiModelClient(
    transport,
    { sessionId: "s", turnId: "t", workflowRunId: "w", connectionId: "c", modelId: "m", remainingSeconds: 1200 },
    DEFAULT_LIMITS,
  );
  await assert.rejects(
    client.generate({ ...request, options: { ...request.options, delivery: "streaming" as "buffered" } }, new AbortController().signal),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "UNSUPPORTED_CAPABILITY",
  );
  await assert.rejects(
    client.generate({ ...request, options: { ...request.options, outputSchema: {} as never } }, new AbortController().signal),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "UNSUPPORTED_CAPABILITY",
  );
  assert.equal(transport.calls.length, 0);
});

test("a malformed or non-buffered HostAi reply is rejected as MALFORMED_RESPONSE", async () => {
  for (const bad of [
    result({ delivery: "streaming" as "buffered" }),
    result({ finishReason: "wat" as "stop" }),
    result({ content: [{ type: "image", artifact: {} } as never] }),
    result({ toolCalls: [{ id: "dup", name: "a", arguments: {} }, { id: "dup", name: "b", arguments: {} }] as never }),
  ]) {
    const transport = new RecordingTransport(() => bad);
    const client = new HostAiModelClient(
      transport,
      { sessionId: "s", turnId: "t", workflowRunId: "w", connectionId: "c", modelId: "m", remainingSeconds: 1200 },
      DEFAULT_LIMITS,
    );
    await assert.rejects(
      client.generate(request, new AbortController().signal),
      (error: unknown) => error instanceof ProtocolError && error.value.code === "MALFORMED_RESPONSE",
    );
  }
});

test("agent.run is explicitly unavailable until the ACP executor lands", async () => {
  assert.equal(AGENT_RUN_CAPABILITY.hostAgentTasks, false);
  const client = new HostAiAgentClient();
  assert.equal(client.available, false);
  await assert.rejects(
    client.run({ connectionId: "codex", modelId: "codex-model", task: [], document: { documentId: "doc", revision: "1" }, requirements: null }),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "UNSUPPORTED_CAPABILITY",
  );
});

test("diagnostic text is redacted and bounded to 2048 UTF-8 bytes", () => {
  assert.equal(redactedMessage("Authorization: Bearer sk-abcdefgh12345678"), "Authorization: [redacted]");
  assert.doesNotMatch(redactedMessage("Bearer abc.def.ghi"), /abc\.def/u);
  const bounded = redactedMessage("é".repeat(2000));
  assert.ok(Buffer.byteLength(bounded, "utf8") <= 2048);
  assert.match(bounded, /^é+$/u);
});
