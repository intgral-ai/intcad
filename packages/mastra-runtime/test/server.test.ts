import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

import { DEFAULT_LIMITS } from "../src/limits.js";
import type { JsonObject, Scope } from "../src/protocol.js";

const MAIN = fileURLToPath(new URL("../src/main.js", import.meta.url));

interface Frame {
  readonly protocolVersion: string;
  readonly kind: "request" | "response" | "event";
  readonly requestId?: string | null;
  readonly method?: string;
  readonly event?: string;
  readonly scope?: Scope;
  readonly outcome?: string;
  readonly result?: JsonObject;
  readonly error?: JsonObject;
  readonly data?: JsonObject;
  readonly sequence?: number;
  readonly params?: JsonObject;
}

class Peer {
  readonly child: ChildProcessWithoutNullStreams;
  readonly frames: Frame[] = [];
  readonly stderr: string[] = [];
  readonly exit: Promise<number>;
  readonly #consumed = new Set<Frame>();
  readonly #waiters: { predicate: (frame: Frame) => boolean; resolve: (frame: Frame) => void; reject: (error: Error) => void; timer: NodeJS.Timeout }[] = [];

  constructor() {
    this.child = spawn(process.execPath, [MAIN], { stdio: ["pipe", "pipe", "pipe"] });
    this.exit = new Promise<number>((resolve) => {
      this.child.once("exit", (code) => resolve(code ?? -1));
    });
    this.child.stdin.on("error", () => undefined);
    let buffer = "";
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (chunk: string) => {
      buffer += chunk;
      for (;;) {
        const newline = buffer.indexOf("\n");
        if (newline === -1) {
          break;
        }
        const line = buffer.slice(0, newline);
        buffer = buffer.slice(newline + 1);
        if (line.length > 0) {
          this.#onFrame(JSON.parse(line) as Frame);
        }
      }
    });
    this.child.stderr.setEncoding("utf8");
    this.child.stderr.on("data", (chunk: string) => {
      this.stderr.push(chunk);
    });
  }

  #onFrame(frame: Frame): void {
    this.frames.push(frame);
    for (const waiter of [...this.#waiters]) {
      if (!this.#consumed.has(frame) && waiter.predicate(frame)) {
        this.#consumed.add(frame);
        clearTimeout(waiter.timer);
        this.#waiters.splice(this.#waiters.indexOf(waiter), 1);
        waiter.resolve(frame);
      }
    }
  }

  async waitFor(predicate: (frame: Frame) => boolean, timeoutMillis = 5000): Promise<Frame> {
    const existing = this.frames.find((frame) => !this.#consumed.has(frame) && predicate(frame));
    if (existing !== undefined) {
      this.#consumed.add(existing);
      return existing;
    }
    return new Promise<Frame>((resolve, reject) => {
      const waiter = {
        predicate,
        resolve,
        reject,
        timer: setTimeout(() => {
          this.#waiters.splice(this.#waiters.indexOf(waiter), 1);
          reject(new Error(`timed out waiting for frame; saw ${JSON.stringify(this.frames.map((frame) => frame.method ?? frame.event ?? frame.requestId))}`));
        }, timeoutMillis),
      };
      this.#waiters.push(waiter);
    });
  }

  send(method: string, scope: Scope, params: JsonObject, requestId: string): void {
    this.child.stdin.write(`${JSON.stringify({ protocolVersion: "1.0", kind: "request", requestId, scope, method, params })}\n`);
  }

  sendEvent(event: string, scope: Scope, sequence: number, data: JsonObject): void {
    this.child.stdin.write(
      `${JSON.stringify({ protocolVersion: "1.0", kind: "event", requestId: null, scope, sequence, event, data })}\n`,
    );
  }

  async request(method: string, scope: Scope, params: JsonObject, requestId: string): Promise<Frame> {
    this.send(method, scope, params, requestId);
    return this.waitFor((frame) => frame.kind === "response" && frame.requestId === requestId);
  }

  replyOk(request: Frame, result: JsonObject): void {
    this.child.stdin.write(
      `${JSON.stringify({ protocolVersion: "1.0", kind: "response", requestId: request.requestId, scope: request.scope, outcome: "ok", result })}\n`,
    );
  }

  replyModel(request: Frame, text: string, continuation: string | null = null): void {
    this.replyOk(request, {
      content: [{ type: "text", text }],
      toolCalls: [],
      finishReason: "stop",
      usage: null,
      continuation,
      delivery: "buffered",
    });
  }

  async closeInput(): Promise<number> {
    if (!this.child.stdin.destroyed) {
      this.child.stdin.end();
    }
    return this.exit;
  }

  kill(): void {
    if (!this.child.killed) {
      this.child.kill();
    }
  }
}

const runtimeScope: Scope = { sessionId: null, turnId: null, workflowRunId: null };
const sessionScope: Scope = { sessionId: "s-1", turnId: null, workflowRunId: null };
const turnScope: Scope = { sessionId: "s-1", turnId: "t-1", workflowRunId: "w-1" };

function helloParams(versions: string[] = ["1.0"]): JsonObject {
  return { supportedVersions: versions, hostVersion: "0.40.0", limits: DEFAULT_LIMITS as unknown as JsonObject };
}

function sessionParams(): JsonObject {
  return { workflowId: "native-modeling", instructions: "Use native features.", bridge: null };
}

function turnParams(): JsonObject {
  return {
    binding: {
      workflowId: "native-modeling",
      connectionId: "byok-1",
      modelId: "model-a",
      document: { documentId: "doc-demo", revision: "41" },
      profile: { profileId: "p-1", version: "1.0.0", digest: `sha256:${"b".repeat(64)}` },
    },
    parts: [{ type: "text", text: "Set width to 40 mm." }],
    context: { selection: [], requirements: null },
    budgets: { modelCalls: 20, agentTasks: 4, cadCommands: 64, repairs: 2, validationRuns: 3, wallTimeSeconds: 1200 },
  };
}

async function negotiatedPeer(): Promise<{ peer: Peer; instanceId: string }> {
  const peer = new Peer();
  const hello = await peer.request("runtime.hello", runtimeScope, helloParams(), "h:1");
  assert.equal(hello.outcome, "ok");
  const instanceId = String(hello.result?.["runtimeInstanceId"]);
  const ready = await peer.waitFor((frame) => frame.kind === "event" && frame.event === "runtime.ready");
  assert.equal(ready.sequence, 1);
  assert.equal((ready.data as JsonObject)["runtimeInstanceId"], instanceId);
  return { peer, instanceId };
}

test("a built runtime executes a deterministic Mastra workflow through the host protocol", async () => {
  const { peer } = await negotiatedPeer();
  try {
    const start = await peer.request("session.start", sessionScope, sessionParams(), "h:2");
    assert.deepEqual(start.result, { state: "ready", workflowId: "native-modeling" });
    const accepted = await peer.request("turn.start", turnScope, turnParams(), "h:3");
    assert.deepEqual(accepted.result, { state: "accepted" });

    const model = await peer.waitFor((frame) => frame.kind === "request" && frame.method === "model.generate");
    const params = model.params as JsonObject;
    assert.equal(params["connectionId"], "byok-1");
    assert.equal(params["modelId"], "model-a");
    assert.equal(Object.hasOwn(params, "apiKey"), false, "the runtime never carries provider credentials");
    assert.equal(Object.hasOwn(params, "key"), false);
    assert.equal((params["options"] as JsonObject)["delivery"], "buffered");
    assert.deepEqual((params["messages"] as JsonObject[]).at(-1)?.["content"], [{ type: "text", text: "Set width to 40 mm." }]);
    peer.replyModel(model, "The width can be 40 mm.", "cont-1");

    const answer = await peer.waitFor((frame) => frame.kind === "event" && frame.event === "workflow.text");
    assert.equal((answer.data as JsonObject)["text"], "The width can be 40 mm.");
    const finished = await peer.waitFor((frame) => frame.kind === "event" && frame.event === "turn.finished");
    assert.equal((finished.data as JsonObject)["state"], "task_failed");
    assert.equal((finished.data as JsonObject)["changes"], "none");
    assert.deepEqual((finished.data as JsonObject)["document"], { documentId: "doc-demo", revision: "41" });
    assert.equal((finished.data as JsonObject)["validationRunId"], null);

    const sequences = peer.frames.filter((frame) => frame.kind === "event").map((frame) => frame.sequence ?? 0);
    assert.deepEqual(sequences, [...sequences].sort((left, right) => left - right));
    assert.equal(new Set(sequences).size, sequences.length);

    const shutdown = await peer.request("runtime.shutdown", runtimeScope, { reason: "app_exit" }, "h:4");
    assert.deepEqual(shutdown.result, { state: "stopping" });
    assert.equal(await peer.closeInput(), 0);
  } finally {
    peer.kill();
  }
});

test("an unknown method returns METHOD_NOT_FOUND without closing the transport", async () => {
  const { peer } = await negotiatedPeer();
  try {
    const reply = await peer.request("future.method", turnScope, {}, "h:2");
    assert.equal(reply.outcome, "error");
    assert.equal(reply.error?.["code"], "METHOD_NOT_FOUND");
    const still = await peer.request("session.start", sessionScope, sessionParams(), "h:3");
    assert.equal(still.outcome, "ok");
  } finally {
    peer.kill();
  }
});

test("an incompatible hello is refused and the process exits before a session starts", async () => {
  const peer = new Peer();
  try {
    const refusal = await peer.request("runtime.hello", runtimeScope, helloParams(["2.0", "1.1"]), "h:1");
    assert.equal(refusal.outcome, "error");
    assert.equal(refusal.error?.["code"], "INCOMPATIBLE_VERSION");
    assert.deepEqual((refusal.error?.["details"] as JsonObject)["supported"], ["1.0"]);
    assert.equal(await peer.closeInput(), 0);
  } finally {
    peer.kill();
  }
});

test("cancellation reaches the in-flight HostAi request and prevents a late reply from finishing the turn", async () => {
  const { peer } = await negotiatedPeer();
  try {
    await peer.request("session.start", sessionScope, sessionParams(), "h:2");
    await peer.request("turn.start", turnScope, turnParams(), "h:3");
    const model = await peer.waitFor((frame) => frame.kind === "request" && frame.method === "model.generate");
    const wrongRun = await peer.request(
      "turn.cancel",
      { ...turnScope, workflowRunId: "w-other" },
      { reason: "user" },
      "h:3b",
    );
    assert.deepEqual(wrongRun.result, { state: "terminal" }, "a different workflowRunId must not cancel the active turn");
    const cancel = peer.request("turn.cancel", turnScope, { reason: "user" }, "h:4");
    const cancelReply = await cancel;
    assert.deepEqual(cancelReply.result, { state: "cancelling" });
    const runtimeCancel = await peer.waitFor((frame) => frame.kind === "request" && frame.method === "request.cancel");
    assert.equal((runtimeCancel.params as JsonObject)["targetRequestId"], model.requestId);
    const finished = await peer.waitFor((frame) => frame.kind === "event" && frame.event === "turn.finished");
    assert.equal((finished.data as JsonObject)["state"], "cancelled");

    peer.replyModel(model, "late answer", "cont-late");
    await new Promise((resolve) => setTimeout(resolve, 120));
    assert.equal(peer.frames.filter((frame) => frame.kind === "event" && frame.event === "turn.finished").length, 1);
  } finally {
    peer.kill();
  }
});

test("image and document parts reach HostAi as artifact references", async () => {
  const { peer } = await negotiatedPeer();
  try {
    await peer.request("session.start", sessionScope, sessionParams(), "h:2");
    const attached = turnParams();
    const image = {
      artifactId: "image-1",
      digest: `sha256:${"c".repeat(64)}`,
      sizeBytes: 10,
      mediaType: "image/png",
    };
    const document = {
      artifactId: "doc-1",
      digest: `sha256:${"d".repeat(64)}`,
      sizeBytes: 20,
      mediaType: "application/pdf",
    };
    attached["parts"] = [
      { type: "text", text: "Use the attached picture and brief." },
      { type: "image", artifact: image },
      { type: "document", name: "brief.pdf", artifact: document },
    ];
    const accepted = await peer.request("turn.start", turnScope, attached, "h:3");
    assert.deepEqual(accepted.result, { state: "accepted" });

    const model = await peer.waitFor((frame) => frame.kind === "request" && frame.method === "model.generate");
    const messages = (model.params as JsonObject)["messages"] as JsonObject[];
    assert.deepEqual(messages.at(-1)?.["content"], [
      { type: "text", text: "Use the attached picture and brief." },
      { type: "image", artifact: image },
      { type: "document", name: "brief.pdf", artifact: document },
    ]);
    assert.equal(JSON.stringify(model.params).includes("data:"), false, "the runtime never forwards attachment bytes");
    peer.replyModel(model, "The attachments describe the requested change.", null);
    const finished = await peer.waitFor((frame) => frame.kind === "event" && frame.event === "turn.finished");
    assert.equal((finished.data as JsonObject)["state"], "task_failed");
  } finally {
    peer.kill();
  }
});

test("attachment references are stripped from history before the next turn", async () => {
  const { peer } = await negotiatedPeer();
  try {
    await peer.request("session.start", sessionScope, sessionParams(), "h:2");
    const first = turnParams();
    first["parts"] = [
      { type: "text", text: "First turn with files." },
      {
        type: "image",
        artifact: { artifactId: "image-1", digest: `sha256:${"c".repeat(64)}`, sizeBytes: 10, mediaType: "image/png" },
      },
      {
        type: "document",
        name: "brief.pdf",
        artifact: { artifactId: "doc-1", digest: `sha256:${"d".repeat(64)}`, sizeBytes: 20, mediaType: "application/pdf" },
      },
    ];
    assert.deepEqual((await peer.request("turn.start", turnScope, first, "h:3")).result, { state: "accepted" });
    const firstModel = await peer.waitFor((frame) => frame.kind === "request" && frame.method === "model.generate");
    peer.replyModel(firstModel, "Noted the attachments.", null);
    await peer.waitFor((frame) => frame.kind === "event" && frame.event === "turn.finished");

    const second = turnParams();
    second["parts"] = [{ type: "text", text: "Second turn is text only." }];
    const secondScope = { ...turnScope, turnId: "t-2", workflowRunId: "w-2" };
    assert.deepEqual((await peer.request("turn.start", secondScope, second, "h:5")).result, { state: "accepted" });
    const secondModel = await peer.waitFor((frame) => frame.kind === "request" && frame.method === "model.generate");
    const history = JSON.stringify((secondModel.params as JsonObject)["messages"]);
    assert.equal(history.includes('"type":"image"'), false, "an expired image reference never reaches the second call");
    assert.equal(history.includes('"type":"document"'), false, "an expired PDF reference never reaches the second call");
    assert.equal(history.includes("[image attachment]"), true);
    assert.equal(history.includes("[document attachment: brief.pdf]"), true);
    peer.replyModel(secondModel, "Second turn answered.", null);
    const finished = await peer.waitFor(
      (frame) => frame.kind === "event" && frame.event === "turn.finished" && frame.scope?.turnId === "t-2",
    );
    assert.equal((finished.data as JsonObject)["state"], "task_failed");
    const frames = JSON.stringify(peer.frames);
    assert.equal(frames.includes("data:"), false);
    assert.equal(frames.includes("apiKey"), false);
    assert.equal(frames.includes("Authorization"), false);
  } finally {
    peer.kill();
  }
});

test("unsupported attachment type or size is refused before a model call", async () => {
  const { peer } = await negotiatedPeer();
  try {
    await peer.request("session.start", sessionScope, sessionParams(), "h:2");
    const badType = turnParams();
    badType["parts"] = [
      {
        type: "image",
        artifact: { artifactId: "image-1", digest: `sha256:${"c".repeat(64)}`, sizeBytes: 10, mediaType: "image/gif" },
      },
    ];
    const refusedType = await peer.request("turn.start", turnScope, badType, "h:3");
    assert.equal(refusedType.outcome, "error");
    assert.equal(refusedType.error?.["code"], "INVALID_REQUEST");

    const tooLarge = turnParams();
    tooLarge["parts"] = [
      {
        type: "document",
        name: "brief.pdf",
        artifact: {
          artifactId: "doc-1",
          digest: `sha256:${"d".repeat(64)}`,
          sizeBytes: DEFAULT_LIMITS.attachmentBytes + 1,
          mediaType: "application/pdf",
        },
      },
    ];
    const refusedSize = await peer.request("turn.start", turnScope, tooLarge, "h:4");
    assert.equal(refusedSize.outcome, "error");
    // The wire schema caps artifact size at 4 MiB before the handler sees it.
    assert.equal(refusedSize.error?.["code"], "INVALID_REQUEST");
    assert.equal(peer.frames.filter((frame) => frame.kind === "request" && frame.method === "model.generate").length, 0);

    const noBudget = turnParams();
    noBudget["budgets"] = { modelCalls: 0, agentTasks: 0, cadCommands: 0, repairs: 0, validationRuns: 0, wallTimeSeconds: 120 };
    const accepted = await peer.request("turn.start", turnScope, noBudget, "h:5");
    assert.deepEqual(accepted.result, { state: "accepted" });
    const finished = await peer.waitFor((frame) => frame.kind === "event" && frame.event === "turn.finished");
    assert.equal((finished.data as JsonObject)["state"], "budget_exhausted");
    assert.equal(peer.frames.filter((frame) => frame.kind === "request" && frame.method === "model.generate").length, 0);
  } finally {
    peer.kill();
  }
});

test("session.reset retires the old session identity", async () => {
  const { peer } = await negotiatedPeer();
  try {
    await peer.request("session.start", sessionScope, sessionParams(), "h:2");
    const reset = await peer.request(
      "session.reset",
      sessionScope,
      { nextSessionId: "s-2", instructions: "Use native features again." },
      "h:3",
    );
    assert.deepEqual(reset.result, { state: "ready", sessionId: "s-2" });
    const oldTurn = await peer.request("turn.start", turnScope, turnParams(), "h:4");
    assert.equal(oldTurn.outcome, "error");
    assert.equal(oldTurn.error?.["code"], "INVALID_REQUEST");
    const newTurn = await peer.request(
      "turn.start",
      { sessionId: "s-2", turnId: "t-2", workflowRunId: "w-2" },
      turnParams(),
      "h:5",
    );
    assert.deepEqual(newTurn.result, { state: "accepted" });
    const model = await peer.waitFor((frame) => frame.kind === "request" && frame.method === "model.generate");
    assert.equal((model.scope as Scope).sessionId, "s-2");
    peer.replyModel(model, "ok");
    const finished = await peer.waitFor((frame) => frame.kind === "event" && frame.event === "turn.finished");
    assert.equal((finished.scope as Scope).sessionId, "s-2");
  } finally {
    peer.kill();
  }
});

test("duplicate or decreasing host event sequence is a protocol error", async () => {
  const { peer } = await negotiatedPeer();
  try {
    await peer.request("session.start", sessionScope, sessionParams(), "h:2");
    const data = { connectionId: "byok-1", state: "available", authMethods: [] };
    peer.sendEvent("connection.state", sessionScope, 1, data);
    peer.sendEvent("connection.state", sessionScope, 1, data);
    assert.equal(await peer.exit, 2);
    assert.match(peer.stderr.join(""), /host event sequence/u);
  } finally {
    peer.kill();
  }
});

test("a replacement runtime process cannot resume an old session or continuation", async () => {
  const first = new Peer();
  const firstHello = await first.request("runtime.hello", runtimeScope, helloParams(), "h:1");
  assert.equal(firstHello.outcome, "ok");
  const firstInstance = String(firstHello.result?.["runtimeInstanceId"]);
  assert.equal((await first.request("session.start", sessionScope, sessionParams(), "h:2")).outcome, "ok");
  assert.equal(await first.closeInput(), 0);

  const second = new Peer();
  try {
    const secondHello = await second.request("runtime.hello", runtimeScope, helloParams(), "h:1");
    assert.equal(secondHello.outcome, "ok");
    assert.notEqual(String(secondHello.result?.["runtimeInstanceId"]), firstInstance);
    const reply = await second.request("turn.start", turnScope, turnParams(), "h:2");
    assert.equal(reply.outcome, "error");
    assert.equal(reply.error?.["code"], "INVALID_REQUEST");
    await new Promise((resolve) => setTimeout(resolve, 80));
    assert.equal(second.frames.filter((frame) => frame.kind === "request" && frame.method === "model.generate").length, 0);
    assert.equal(await second.closeInput(), 0);
  } finally {
    second.kill();
  }
});
