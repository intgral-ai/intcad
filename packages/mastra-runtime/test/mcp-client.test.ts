import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";

import { ProtocolError } from "../src/errors.js";
import type { JsonObject } from "../src/protocol.js";
import { McpClient, McpRpcError, streamChannel } from "../src/solidcraft/mcp-client.js";
import { FakeMcpPeer, deferred, textReply, type FakePeerOptions } from "./support/mcp-peer.js";

function makeClient(peer: FakeMcpPeer, overrides: { requestTimeoutMs?: number; mcpSessionId?: string } = {}): McpClient {
  return new McpClient({
    channel: peer,
    bridge: { name: "solidcraft-app", bridgeId: "bridge-1" },
    turn: { sessionId: "s-1", turnId: "t-1", workflowRunId: "w-1" },
    mcpSessionId: overrides.mcpSessionId ?? "mcp-session-fixture",
    requestTimeoutMs: overrides.requestTimeoutMs ?? 500,
    handshakeTimeoutMs: 500,
  });
}

async function connected(t: TestContext, options: FakePeerOptions = {}): Promise<{ peer: FakeMcpPeer; client: McpClient }> {
  const peer = new FakeMcpPeer(options);
  const client = makeClient(peer);
  t.after(() => client.close("test finished"));
  await client.connect();
  return { peer, client };
}

function tick(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

test("connects to the injected bridge and discovers the tool surface and annotations", async (t) => {
  const { client } = await connected(t);
  assert.equal(client.serverName, "solidcraft");
  assert.equal(client.protocolVersion, "2025-06-18");
  assert.equal(client.mcpSessionId, "mcp-session-fixture");
  assert.ok(client.hasTool("execute"));
  assert.ok(client.hasTool("batch"));
  assert.ok(client.hasTool("pending"));
  assert.equal(client.tool("inspect_design")?.readOnly, true);
  assert.equal(client.tool("execute")?.readOnly, false);
  assert.equal(client.tool("batch")?.destructive, false);
});

test("numeric JSON-RPC ids map bijectively to protocol Ids", () => {
  const peer = new FakeMcpPeer();
  const client = makeClient(peer);
  const first = client.reserveRequest();
  const second = client.reserveRequest();
  assert.deepEqual(first, { jsonId: 1, wireId: "mcp-1" });
  assert.deepEqual(second, { jsonId: 2, wireId: "mcp-2" });
  assert.equal(client.wireIdFor(1), "mcp-1");
  assert.equal(client.wireIdFor(2), "mcp-2");
  assert.equal(client.jsonIdFor("mcp-2"), 2);
  assert.equal(client.jsonIdFor("mcp-0"), null);
  assert.equal(client.jsonIdFor("mcp-999"), null);
  assert.equal(client.wireIdFor(999), null);
});

test("the chat turn association never crosses the MCP wire", async (t) => {
  const { peer } = await connected(t);
  assert.ok(peer.requests.length > 0);
  for (const request of peer.requests) {
    const text = JSON.stringify(request.params);
    assert.ok(!text.includes("s-1"), "session id must not be sent");
    assert.ok(!text.includes("t-1"), "turn id must not be sent");
    assert.ok(!text.includes("w-1"), "workflow run id must not be sent");
  }
});

test("describe_command is discovered once and cached", async (t) => {
  const { peer, client } = await connected(t);
  const first = await client.describeCommand("solid.box");
  const second = await client.describeCommand("solid.box");
  assert.equal(first.id, "solid.box");
  assert.ok(first.inputSchema !== null);
  assert.equal(second, first);
  assert.equal(peer.toolCallsNamed("describe_command").length, 1);
});

test("an unknown command is an actionable capability error", async (t) => {
  const { client } = await connected(t);
  await assert.rejects(
    () => client.describeCommand("does.not.exist"),
    (error: unknown) =>
      error instanceof ProtocolError &&
      error.value.code === "UNSUPPORTED_CAPABILITY" &&
      error.value.details["capability"] === "command:does.not.exist" &&
      error.value.message.includes("does.not.exist"),
  );
});

test("a malformed command description is refused rather than used", async (t) => {
  const { peer, client } = await connected(t);
  peer.setHandler("describe_command", () =>
    textReply({ id: "another.command", label: "x", params: "", input: {}, output: null }),
  );
  await assert.rejects(
    () => client.describeCommand("solid.box"),
    (error: unknown) =>
      error instanceof ProtocolError &&
      error.value.code === "UNSUPPORTED_CAPABILITY" &&
      error.value.details["capability"] === "command-schema:solid.box",
  );
});

test("missing command schemas refuse write arguments but allow exact empty reads", async (t) => {
  const { client } = await connected(t);
  const command = await client.describeCommand("solid.unconverted");
  assert.equal(command.inputSchema, null);
  assert.throws(
    () => client.assertCommandArguments(command, { length: 1 }, { allowMissingSchema: false }),
    (error: unknown) =>
      error instanceof ProtocolError && error.value.code === "UNSUPPORTED_CAPABILITY" && error.value.details["capability"] === "command-schema:solid.unconverted",
  );
  assert.doesNotThrow(() => client.assertCommandArguments(command, {}, { allowMissingSchema: true }));
  assert.throws(
    () => client.assertCommandArguments(command, { length: 1 }, { allowMissingSchema: true }),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "UNSUPPORTED_CAPABILITY",
  );
});

test("mismatched parameters produce INVALID_REQUEST naming the field", async (t) => {
  const { client } = await connected(t);
  const command = await client.describeCommand("solid.mismatch");
  assert.throws(
    () => client.assertCommandArguments(command, { radius: "wide" }, { allowMissingSchema: false }),
    (error: unknown) =>
      error instanceof ProtocolError &&
      error.value.code === "INVALID_REQUEST" &&
      error.value.details["field"] === "params" &&
      error.value.message.includes("radius"),
  );
  assert.throws(
    () => client.assertCommandArguments(command, {}, { allowMissingSchema: false }),
    (error: unknown) =>
      error instanceof ProtocolError && error.value.code === "INVALID_REQUEST" && error.value.message.includes("required"),
  );
});

test("envelope keys are refused inside CAD params", async (t) => {
  const { client } = await connected(t);
  const command = await client.describeCommand("solid.box");
  assert.throws(
    () =>
      client.assertCommandArguments(
        command,
        { length: 1, width: 1, height: 1, expectedDocument: { documentId: "doc-A", revision: "1" } },
        { allowMissingSchema: false },
      ),
    (error: unknown) =>
      error instanceof ProtocolError && error.value.code === "INVALID_REQUEST" && error.value.message.includes("execution envelope"),
  );
});

test("tool arguments are validated against the discovered tool schema", async (t) => {
  const { client } = await connected(t);
  assert.throws(
    () =>
      client.assertToolArguments("execute", {
        command: "parameters.change",
        params: { name: "width" },
        expect: { volume_mm3: "large" },
      }),
    (error: unknown) =>
      error instanceof ProtocolError && error.value.code === "INVALID_REQUEST" && error.value.details["field"] === "arguments",
  );
});

test("a JSON-RPC error becomes McpRpcError and is not a tool result", async (t) => {
  const { peer, client } = await connected(t);
  peer.setHandler("measure", (args) => {
    throw new Error(`bad measure ${JSON.stringify(args)}`);
  });
  await assert.rejects(
    () => client.callTool("measure", { summary: true }),
    (error: unknown) => error instanceof McpRpcError && error.code === -32602 && error.message.includes("bad measure"),
  );
});

test("pending and running use the same connection and distinct request identities", async (t) => {
  const { peer, client } = await connected(t);
  peer.pending(() => textReply({ pending: "execute", done: false, running_s: 1 }));
  const pending = await client.pending(500);
  assert.match(pending.wireRequestId, /^mcp-[1-9][0-9]*$/u);
  assert.equal(pending.value?.["done"], false);
  const running = await client.running(500);
  assert.match(running.wireRequestId, /^mcp-[1-9][0-9]*$/u);
  assert.notEqual(pending.wireRequestId, running.wireRequestId);
  assert.equal(peer.toolCallsNamed("pending").length, 1);
  assert.equal(peer.toolCallsNamed("running").length, 1);
  assert.equal(`mcp-${String(peer.toolCallsNamed("pending")[0]?.id)}`, pending.wireRequestId);
  assert.equal(`mcp-${String(peer.toolCallsNamed("running")[0]?.id)}`, running.wireRequestId);
});

test("a late reply after a client timeout is ignored, not a protocol violation", async (t) => {
  const { peer, client } = await connected(t);
  const held = deferred<void>();
  const arrived = deferred<void>();
  peer.setHandler("measure", async () => {
    arrived.resolve();
    await held.promise;
    return textReply({ body_count: 1 });
  });
  await assert.rejects(
    () => client.callTool("measure", {}, { timeoutMs: 10 }),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "TIMEOUT",
  );
  await arrived.promise;
  held.resolve();
  await tick();
  await tick();
  assert.equal(client.isClosed, false, "the tombstoned late response must not close the connection");
  peer.pending(() => textReply({ pending: null }));
  const status = await client.pending(500);
  assert.equal(status.value?.["pending"], null);
});

test("a disconnect rejects outstanding calls with TRANSPORT_ERROR", async (t) => {
  const { peer, client } = await connected(t);
  const arrived = deferred<void>();
  const held = deferred<void>();
  peer.setHandler("measure", async () => {
    arrived.resolve();
    await held.promise;
    return textReply({ body_count: 1 });
  });
  const call = client.callTool("measure", {});
  await arrived.promise;
  peer.disconnect("the app closed");
  await assert.rejects(
    () => call,
    (error: unknown) =>
      error instanceof ProtocolError &&
      error.value.code === "TRANSPORT_ERROR" &&
      error.value.message.includes("the app closed"),
  );
  assert.equal(client.isClosed, true);
  held.resolve();
  await assert.rejects(
    () => client.pending(100),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "TRANSPORT_ERROR",
  );
});

test("a malformed inbound frame closes the bridge", async (t) => {
  const { peer, client } = await connected(t);
  peer.inject("{not json");
  assert.equal(client.isClosed, true);
  await assert.rejects(
    () => client.pending(100),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "TRANSPORT_ERROR",
  );
});

test("an unsupported MCP version fails the handshake", async () => {
  const peer = new FakeMcpPeer();
  peer.protocolVersion = "9.9";
  const client = makeClient(peer);
  await assert.rejects(
    () => client.connect(),
    (error: unknown) =>
      error instanceof ProtocolError &&
      error.value.code === "UNSUPPORTED_CAPABILITY" &&
      error.value.details["capability"] === "mcp-protocol:9.9",
  );
});

test("invalid bridge and turn identities are refused before connecting", () => {
  const peer = new FakeMcpPeer();
  assert.throws(
    () =>
      new McpClient({
        channel: peer,
        bridge: { name: "solidcraft-app", bridgeId: "not a bridge id" },
        turn: { sessionId: "s-1", turnId: "t-1", workflowRunId: "w-1" },
      }),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "INVALID_REQUEST",
  );
  assert.throws(
    () =>
      new McpClient({
        channel: peer,
        bridge: { name: "solidcraft-app", bridgeId: "bridge-1" },
        turn: { sessionId: "s-1", turnId: "t 1", workflowRunId: "w-1" },
      }),
    (error: unknown) => error instanceof ProtocolError && error.value.code === "INVALID_REQUEST",
  );
});

test("streamChannel frames lines and reports a closed output", async () => {
  const { PassThrough } = await import("node:stream");
  const input = new PassThrough();
  const output = new PassThrough();
  const channel = streamChannel(input, output);
  const lines: string[] = [];
  let closed = false;
  channel.receive(
    (line) => lines.push(line),
    () => {
      closed = true;
    },
  );
  channel.send('{"jsonrpc":"2.0","id":1,"method":"ping"}');
  assert.equal(output.read()?.toString("utf8"), '{"jsonrpc":"2.0","id":1,"method":"ping"}\n');
  input.write('{"a":1}\n{"b":2}\n');
  await tick();
  assert.deepEqual(lines, ['{"a":1}', '{"b":2}']);
  input.end();
  await tick();
  assert.equal(closed, true);
});

test("requests are only sent as JSON objects", async (t) => {
  const { peer } = await connected(t);
  for (const request of peer.requests) {
    assert.equal(typeof request.params, "object");
    assert.ok(request.params !== null);
    assert.equal(Array.isArray(request.params), false);
  }
  const call = peer.toolCalls[0] as { args: JsonObject } | undefined;
  assert.ok(call === undefined || typeof call.args === "object");
});
