import assert from "node:assert/strict";
import { test } from "node:test";

import type { Agent } from "@mastra/core/agent";

import { ProtocolError } from "../src/errors.js";
import { DEFAULT_LIMITS } from "../src/limits.js";
import { ContinuationBranch, createHostAiAgent, createHostAiLanguageModel, toMastraMessages } from "../src/host-ai/language-model.js";
import { HostAiModelClient, type HostAiTransport, type ModelMessage } from "../src/host-ai/model-client.js";
import type { JsonObject, Scope } from "../src/protocol.js";

class QueueTransport implements HostAiTransport {
  readonly calls: { params: JsonObject; scope: Scope }[] = [];
  readonly #replies: (() => JsonObject | Promise<JsonObject>)[];

  constructor(replies: (() => JsonObject | Promise<JsonObject>)[]) {
    this.#replies = replies;
  }

  async call(_method: string, scope: Scope, params: JsonObject): Promise<JsonObject> {
    this.calls.push({ params, scope });
    const next = this.#replies.shift();
    if (next === undefined) {
      throw new Error("the runtime made an unexpected extra HostAi call");
    }
    return next();
  }
}

const tools = [
  {
    name: "set_width",
    description: "Propose a width for the active block.",
    inputSchema: {
      type: "object",
      properties: { width: { type: "number" } },
      required: ["width"],
      additionalProperties: false,
    },
  },
] as const;

function client(transport: HostAiTransport, branch: ContinuationBranch): HostAiModelClient {
  return new HostAiModelClient(
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
    undefined,
  );
}

function agentFor(hostClient: HostAiModelClient, branch: ContinuationBranch): Agent {
  return createHostAiAgent({
    client: hostClient,
    connectionId: "byok-1",
    modelId: "model-a",
    timeoutSeconds: 300,
    instructions: "Return one native proposal.",
    tools: [...tools],
    branch,
  });
}

test("a real Mastra Agent performs exactly one HostAi call per invocation and returns tool calls unexecuted", async () => {
  const branch = new ContinuationBranch();
  const transport = new QueueTransport([
    () => ({
      content: [],
      toolCalls: [{ id: "call-1", name: "set_width", arguments: { width: 40 } }],
      finishReason: "tool_calls",
      usage: null,
      continuation: "cont-1",
      delivery: "buffered",
    }),
    () => ({
      content: [{ type: "text", text: "done" }],
      toolCalls: [],
      finishReason: "stop",
      usage: null,
      continuation: "cont-2",
      delivery: "buffered",
    }),
  ]);
  const hostClient = client(transport, branch);
  const agent = agentFor(hostClient, branch);

  const firstMessages: ModelMessage[] = [
    { role: "user", content: [{ type: "text", text: "Set width to 40 mm." }], toolCalls: [], toolCallId: null },
  ];
  const first = await agent.generateLegacy(toMastraMessages(firstMessages) as Parameters<Agent["generateLegacy"]>[0], { maxSteps: 1 });

  assert.equal(transport.calls.length, 1);
  assert.equal(hostClient.calls, 1);
  assert.equal((first.steps ?? []).length, 1, "Mastra must not run a second hidden step");
  assert.deepEqual(
    (first.toolCalls ?? []).map((call) => ({ id: call.toolCallId, name: call.toolName, args: call.args })),
    [{ id: "call-1", name: "set_width", args: { width: 40 } }],
  );
  assert.deepEqual(first.toolResults ?? [], [], "a tool declaration must not execute");
  assert.equal(first.finishReason, "tool-calls");

  const firstParams = transport.calls[0]?.params;
  assert.equal((firstParams?.["continuation"] as string | null), null);
  const sentTools = firstParams?.["tools"] as JsonObject[];
  assert.equal(sentTools.length, 1);
  assert.equal(sentTools[0]?.["name"], "set_width");
  assert.deepEqual(sentTools[0]?.["inputSchema"], tools[0].inputSchema);
  const sentMessages = firstParams?.["messages"] as JsonObject[];
  assert.equal(sentMessages[0]?.["role"], "system");
  assert.equal(sentMessages.at(-1)?.["role"], "user");

  // The outer workflow owns the result and the next explicit round.
  const secondMessages: ModelMessage[] = [
    ...firstMessages,
    {
      role: "assistant",
      content: [],
      toolCalls: [{ id: "call-1", name: "set_width", arguments: { width: 40 } }],
      toolCallId: null,
    },
    { role: "tool", content: [{ type: "text", text: '{"ok":true}' }], toolCalls: [], toolCallId: "call-1" },
  ];
  const second = await agent.generateLegacy(toMastraMessages(secondMessages) as Parameters<Agent["generateLegacy"]>[0], { maxSteps: 1 });

  assert.equal(transport.calls.length, 2);
  assert.equal((transport.calls[1]?.params["continuation"] as string | null), "cont-1");
  const secondSent = transport.calls[1]?.params["messages"] as JsonObject[];
  assert.equal(secondSent[2]?.["role"], "assistant");
  assert.deepEqual((secondSent[2]?.["toolCalls"] as JsonObject[])[0], {
    id: "call-1",
    name: "set_width",
    arguments: { width: 40 },
  });
  assert.equal(secondSent[3]?.["role"], "tool");
  assert.equal(secondSent[3]?.["toolCallId"], "call-1");
  assert.equal(second.text, "done");
});

test("a failed HostAi call is not retried by the Agent", async () => {
  const branch = new ContinuationBranch();
  const transport = new QueueTransport([
    () => {
      throw new ProtocolError("MODEL_ERROR", "provider failed", "read_only", { connectionId: "byok-1", httpStatus: 503 });
    },
  ]);
  const hostClient = client(transport, branch);
  const agent = agentFor(hostClient, branch);
  await assert.rejects(
    agent.generateLegacy(
      toMastraMessages([{ role: "user", content: [{ type: "text", text: "hi" }], toolCalls: [], toolCallId: null }]) as Parameters<Agent["generateLegacy"]>[0],
      { maxSteps: 1 },
    ),
  );
  assert.equal(transport.calls.length, 1);
  assert.equal(hostClient.calls, 1);
});

test("images, documents and structured output fail before HostAi is called", async () => {
  const branch = new ContinuationBranch();
  const transport = new QueueTransport([]);
  const model = createHostAiLanguageModel({
    client: client(transport, branch),
    connectionId: "byok-1",
    modelId: "model-a",
    timeoutSeconds: 300,
    branch,
  });
  const call = (options: JsonObject): Promise<unknown> =>
    (model.doGenerate as unknown as (options: JsonObject) => Promise<unknown>)(options);

  await assert.rejects(
    call({
      prompt: [{ role: "user", content: [{ type: "image", image: new Uint8Array([1]), mimeType: "image/png" }] }],
      mode: { type: "regular", tools: [] },
    }),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "UNSUPPORTED_CAPABILITY",
  );
  await assert.rejects(
    call({
      prompt: [{ role: "user", content: [{ type: "file", data: new Uint8Array([1]), mimeType: "application/pdf" }] }],
      mode: { type: "regular", tools: [] },
    }),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "UNSUPPORTED_CAPABILITY",
  );
  await assert.rejects(
    call({
      prompt: [{ role: "user", content: [{ type: "text", text: "json" }] }],
      mode: { type: "regular", tools: [] },
      responseFormat: { type: "json" },
    }),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "UNSUPPORTED_CAPABILITY",
  );
  assert.equal(transport.calls.length, 0);
});

test("a continuation handle is dropped when the normalized history branch does not extend", () => {
  const branch = new ContinuationBranch();
  const base: ModelMessage[] = [{ role: "user", content: [{ type: "text", text: "a" }], toolCalls: [], toolCallId: null }];
  branch.store("cont-1", base, 1000);
  assert.equal(branch.take(base, 1001), "cont-1");
  const other: ModelMessage[] = [{ role: "user", content: [{ type: "text", text: "b" }], toolCalls: [], toolCallId: null }];
  assert.equal(branch.take(other, 1001), null);
  assert.equal(branch.handle, null);
});
