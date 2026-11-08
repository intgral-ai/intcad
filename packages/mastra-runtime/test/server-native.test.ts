import assert from "node:assert/strict";
import { test } from "node:test";

import type { JsonObject } from "../src/protocol.js";
import {
  RUNTIME_SCOPE,
  SESSION_SCOPE,
  NativeRecordedPeer,
  applyCall,
  decisionAsk,
  decisionEdit,
  editBase,
  helloParams,
  parameterChecks,
  sessionParams,
  turnParams,
  turnScope,
  volumeCheck,
  type Frame,
} from "./support/native-server-harness.js";

function eventsFor(peer: NativeRecordedPeer, turnId: string): Frame[] {
  return peer.frames.filter((frame) => frame.kind === "event" && frame.scope?.turnId === turnId);
}

function eventData(frame: Frame): JsonObject {
  return frame.data ?? {};
}

function stage(events: readonly Frame[], stageId: string, state: string): Frame | undefined {
  return events.find(
    (frame) =>
      frame.event === "workflow.stage" &&
      eventData(frame)["stageId"] === stageId &&
      eventData(frame)["state"] === state,
  );
}

async function handshake(peer: NativeRecordedPeer): Promise<void> {
  const hello = await peer.request("runtime.hello", RUNTIME_SCOPE, helloParams(), "h:hello");
  assert.equal(hello.outcome, "ok");
  const capabilities = hello.result?.["capabilities"] as JsonObject | undefined;
  assert.equal(capabilities?.["guardedExecute"], false, "hello must not infer guarded writes from channel presence");
  assert.equal(capabilities?.["guardedBatch"], false);
  assert.equal(capabilities?.["snapshotValidation"], true, "the runtime implements the snapshot validation stage");
  await peer.waitFor((frame) => frame.kind === "event" && frame.event === "runtime.ready");
  const start = await peer.request("session.start", SESSION_SCOPE, sessionParams(), "h:session");
  assert.equal(start.outcome, "ok");
  assert.deepEqual(start.result, { state: "ready", workflowId: "native-modeling" });
}

function modelRequest(peer: NativeRecordedPeer): Promise<Frame> {
  return peer.waitFor((frame) => frame.kind === "request" && frame.method === "model.generate");
}

test("hello advertises guarded paths only when the host probed them", async (t) => {
  const plain = new NativeRecordedPeer(t, editBase(), {});
  const hello = await plain.request("runtime.hello", RUNTIME_SCOPE, helloParams(), "h:hello");
  const capabilities = hello.result?.["capabilities"] as JsonObject | undefined;
  assert.equal(capabilities?.["guardedExecute"], false);
  assert.equal(capabilities?.["guardedBatch"], false);

  const probed = new NativeRecordedPeer(t, editBase(), {}, { bridgeCapabilities: "execute,batch" });
  const guarded = await probed.request("runtime.hello", RUNTIME_SCOPE, helloParams(), "h:hello");
  const guardedCapabilities = guarded.result?.["capabilities"] as JsonObject | undefined;
  assert.equal(guardedCapabilities?.["guardedExecute"], true);
  assert.equal(guardedCapabilities?.["guardedBatch"], true);
});

test("a server turn runs the native workflow: create then parameter edit with protected dimensions", async (t) => {
  const peer = new NativeRecordedPeer(t, editBase(), {
    "p-create": { checks: [volumeCheck(1_000)] },
    "p-edit": { checks: [parameterChecks([
      { name: "width", value: 40 },
      { name: "depth", value: 10 },
      { name: "height", value: 5 },
    ])] },
  });
  await handshake(peer);

  const createTurn = turnParams("t-create", "w-create", "p-create", "Create a 20 x 10 x 5 box.", peer.stamp());
  const accepted = await peer.request("turn.start", turnScope("t-create", "w-create"), createTurn, "h:turn-create");
  assert.deepEqual(accepted.result, { state: "accepted" });

  let request = await modelRequest(peer);
  peer.replyTools(request, [decisionEdit()]);
  request = await modelRequest(peer);
  peer.replyTools(request, [
    applyCall("call-box", [{ command: "solid.box", params: { length: 20, width: 10, height: 5 } }]),
  ]);
  request = await modelRequest(peer);
  peer.replyText(request, "Created one editable native box feature.");

  const createFinish = await peer.waitFor(
    (frame) => frame.kind === "event" && frame.event === "turn.finished" && frame.scope?.turnId === "t-create",
  );
  const createData = eventData(createFinish);
  assert.equal(createData["state"], "validated_success");
  assert.equal(createData["changes"], "applied");
  assert.deepEqual(createData["document"], { documentId: "doc-native", revision: "42" });
  assert.equal(createData["validationRunId"], "v-1");
  assert.equal((createData["report"] as JsonObject | undefined)?.["artifactId"], "report-v-1");

  const createEvents = eventsFor(peer, "t-create");
  assert.equal(stage(createEvents, "inspect", "completed") !== undefined, true);
  assert.equal(stage(createEvents, "check", "completed") !== undefined, true);
  assert.equal(createEvents.some((frame) => frame.event === "workflow.plan"), true);
  assert.equal(createEvents.some((frame) => frame.event === "workflow.text"), true);
  const applyTool = createEvents.find(
    (frame) => frame.event === "workflow.tool" && eventData(frame)["callId"] === "call-box" && eventData(frame)["state"] === "completed",
  );
  assert.equal((eventData(applyTool ?? createEvents[0]!)["output"] as JsonObject | undefined)?.["state"], "applied");
  assert.equal(peer.state.timeline.length, 1);
  assert.equal(peer.state.timeline[0]?.type, "box");
  assert.equal(peer.state.bodies[0]?.volume_mm3, 1_000);

  const editTurn = turnParams("t-edit", "w-edit", "p-edit", "Set width to 40 mm and keep depth and height.", peer.stamp());
  const acceptedEdit = await peer.request("turn.start", turnScope("t-edit", "w-edit"), editTurn, "h:turn-edit");
  assert.deepEqual(acceptedEdit.result, { state: "accepted" });

  request = await modelRequest(peer);
  const editMessages = (request.params?.["messages"] as JsonObject[] | undefined) ?? [];
  assert.equal(editMessages.some((message) => message["role"] === "assistant" && JSON.stringify(message["content"]).includes("Created one editable")), true);
  assert.equal(editMessages.some((message) => message["role"] === "user" && JSON.stringify(message["content"]).includes("Create a 20 x 10 x 5 box")), true);
  peer.replyTools(request, [decisionEdit()]);
  request = await modelRequest(peer);
  peer.replyTools(request, [
    applyCall("call-width", [{ command: "parameters.change", params: { name: "width", expression: "40" } }]),
  ]);
  request = await modelRequest(peer);
  peer.replyText(request, "Width is 40 mm; depth and height are unchanged.");

  const editFinish = await peer.waitFor(
    (frame) => frame.kind === "event" && frame.event === "turn.finished" && frame.scope?.turnId === "t-edit",
  );
  const editData = eventData(editFinish);
  assert.equal(editData["state"], "validated_success");
  assert.equal(editData["changes"], "applied");
  assert.equal(editData["validationRunId"], "v-2");
  assert.deepEqual(editData["document"], { documentId: "doc-native", revision: "43" });
  assert.equal(peer.state.parameters.get("width")?.value, 40);
  assert.equal(peer.state.parameters.get("depth")?.value, 10);
  assert.equal(peer.state.parameters.get("height")?.value, 5);
  assert.equal(stage(eventsFor(peer, "t-edit"), "check", "completed") !== undefined, true);
  assert.equal(stage(eventsFor(peer, "t-edit"), "validate", "completed") !== undefined, true);
  assert.equal(
    peer.mcp.requests.filter((request) => request.method === "initialize").length >= 2,
    true,
    "each turn opened its own MCP session on the injected bridge lane",
  );
  const captures = peer.mcp
    .toolCallsNamed("execute")
    .filter((call) => call.args["command"] === "document.snapshot.capture");
  assert.equal(captures.length, 2, "each successful turn captured its own candidate");
  assert.equal(captures.every((call) => typeof call.args["expectedDocument"] === "object"), true);
});

test("a quality-failed report repairs with a new candidate, key and run before success", async (t) => {
  const peer = new NativeRecordedPeer(t, editBase(), {
    "p-repair": { checks: [] },
  });
  peer.validationStates.set("p-repair", ["quality_failed", "passed"]);
  await handshake(peer);

  const turn = turnParams("t-repair", "w-repair", "p-repair", "Make the box and adjust it.", peer.stamp());
  await peer.request("turn.start", turnScope("t-repair", "w-repair"), turn, "h:turn-repair");

  let request = await modelRequest(peer);
  peer.replyTools(request, [decisionEdit()]);
  request = await modelRequest(peer);
  peer.replyTools(request, [
    applyCall("call-box", [{ command: "solid.box", params: { length: 20, width: 10, height: 5 } }]),
  ]);
  request = await modelRequest(peer);
  peer.replyText(request, "Created the box.");

  // The failed report returns to planning; the model applies a corrected edit.
  request = await modelRequest(peer);
  const repairMessages = (request.params?.["messages"] as JsonObject[] | undefined) ?? [];
  assert.equal(
    repairMessages.some((message) => JSON.stringify(message["content"]).includes("Snapshot validation failed")),
    true,
    "the runtime feeds the failed report identity back into the repair round",
  );
  peer.replyTools(request, [
    applyCall("call-repair", [{ command: "parameters.change", params: { name: "width", expression: "40" } }]),
  ]);
  request = await modelRequest(peer);
  peer.replyText(request, "Adjusted the width; the new snapshot is validated.");

  const finished = await peer.waitFor(
    (frame) => frame.kind === "event" && frame.event === "turn.finished" && frame.scope?.turnId === "t-repair",
  );
  const data = eventData(finished);
  assert.equal(data["state"], "validated_success");
  assert.equal(data["validationRunId"], "v-2", "the repaired design has a new run");
  assert.equal((data["report"] as JsonObject | undefined)?.["artifactId"], "report-v-2");
  assert.equal(peer.validationSubmissions.length, 2);
  assert.notEqual(
    peer.validationSubmissions[0]?.["requestId"],
    peer.validationSubmissions[1]?.["requestId"],
    "every changed design gets a fresh durable submission key",
  );
  assert.notEqual(
    (peer.validationSubmissions[0]?.["candidate"] as JsonObject | undefined)?.["candidateId"],
    (peer.validationSubmissions[1]?.["candidate"] as JsonObject | undefined)?.["candidateId"],
    "a failed report is history for the old candidate, never approval of the new one",
  );
});

test("a server turn runs at most the configured repair budget and ends quality_failed", async (t) => {
  const peer = new NativeRecordedPeer(t, editBase(), {
    "p-fail": { checks: [parameterChecks([{ name: "width", value: 40 }])] },
  });
  await handshake(peer);

  const turn = turnParams("t-fail", "w-fail", "p-fail", "Make width 40 mm.", peer.stamp(), {
    modelCalls: 20,
    agentTasks: 4,
    cadCommands: 64,
    repairs: 2,
    validationRuns: 3,
    wallTimeSeconds: 1200,
  });
  await peer.request("turn.start", turnScope("t-fail", "w-fail"), turn, "h:turn-fail");

  let request = await modelRequest(peer);
  peer.replyTools(request, [decisionEdit()]);
  for (const id of ["call-fail-1", "call-fail-2", "call-fail-3"]) {
    request = await modelRequest(peer);
    peer.replyTools(request, [
      applyCall(id, [{ command: "parameters.change", params: { name: "width", expression: "30" } }]),
    ]);
  }

  const finished = await peer.waitFor(
    (frame) => frame.kind === "event" && frame.event === "turn.finished" && frame.scope?.turnId === "t-fail",
  );
  const data = eventData(finished);
  assert.equal(data["state"], "quality_failed");
  assert.equal(data["changes"], "applied");
  assert.match(String(data["reason"]), /2 repair attempt/u);
  assert.deepEqual(data["document"], { documentId: "doc-native", revision: "44" });
  const commandWrites = peer.mcp
    .toolCallsNamed("execute")
    .filter((call) => call.args["command"] === "parameters.change");
  assert.equal(commandWrites.length, 3, "initial check plus exactly two bounded repairs");
  const events = eventsFor(peer, "t-fail");
  assert.equal(stage(events, "check", "failed") !== undefined, true);
  assert.equal(stage(events, "repair", "failed") !== undefined, true);
  assert.equal(peer.frames.filter((frame) => frame.kind === "request" && frame.method === "model.generate").length, 4);
});

test("a configured bridge with no injected channel fails clearly without model or CAD work", async (t) => {
  const peer = new NativeRecordedPeer(t, editBase(), { "p-any": { checks: [] } }, { bridge: false });
  await handshake(peer);

  const turn = turnParams("t-no-channel", "w-no-channel", "p-any", "Try to edit.", peer.stamp());
  await peer.request("turn.start", turnScope("t-no-channel", "w-no-channel"), turn, "h:turn-no-channel");
  const finished = await peer.waitFor(
    (frame) => frame.kind === "event" && frame.event === "turn.finished" && frame.scope?.turnId === "t-no-channel",
  );
  const data = eventData(finished);
  assert.equal(data["state"], "task_failed");
  assert.match(String(data["reason"]), /did not provision a CAD channel/u);
  assert.equal(data["changes"], "none");
  assert.equal(peer.frames.filter((frame) => frame.kind === "request" && frame.method === "model.generate").length, 0);
  assert.equal(peer.mcp.requests.length, 0);
});

test("server cancellation stops after the current step and ask ends task_failed with truthful changes", async (t) => {
  const peer = new NativeRecordedPeer(t, editBase(), {
    "p-cancel": { checks: [parameterChecks([{ name: "width", value: 40 }])] },
    "p-ask": { checks: [parameterChecks([{ name: "width", value: 40 }])] },
  });
  await handshake(peer);

  const turn = turnParams("t-cancel", "w-cancel", "p-cancel", "Set width to 40 mm.", peer.stamp());
  await peer.request("turn.start", turnScope("t-cancel", "w-cancel"), turn, "h:turn-cancel");

  let request = await modelRequest(peer);
  peer.replyTools(request, [decisionEdit()]);
  request = await modelRequest(peer);
  peer.replyTools(request, [
    applyCall("call-cancel-width", [{ command: "parameters.change", params: { name: "width", expression: "40" } }]),
  ]);

  // The next model request is the post-check round. Cancel while it is in flight.
  await modelRequest(peer);
  const cancel = await peer.request("turn.cancel", turnScope("t-cancel", "w-cancel"), { reason: "user" }, "h:cancel");
  assert.deepEqual(cancel.result, { state: "cancelling" });

  const finished = await peer.waitFor(
    (frame) => frame.kind === "event" && frame.event === "turn.finished" && frame.scope?.turnId === "t-cancel",
  );
  const data = eventData(finished);
  assert.equal(data["state"], "cancelled");
  assert.equal(data["changes"], "applied", "the write that completed before Stop remains a live edit");
  assert.deepEqual(data["document"], { documentId: "doc-native", revision: "42" });
  const widthWrites = peer.mcp
    .toolCallsNamed("execute")
    .filter((call) => call.args["command"] === "parameters.change");
  assert.equal(widthWrites.length, 1, "no second write starts after cancellation");
  assert.equal(peer.state.parameters.get("width")?.value, 40);

  const askTurn = turnParams("t-ask", "w-ask", "p-ask", "Change the protected dimension.", peer.stamp());
  await peer.request("turn.start", turnScope("t-ask", "w-ask"), askTurn, "h:turn-ask");
  request = await modelRequest(peer);
  peer.replyTools(request, [decisionAsk()]);
  const askFinish = await peer.waitFor(
    (frame) => frame.kind === "event" && frame.event === "turn.finished" && frame.scope?.turnId === "t-ask",
  );
  const askData = eventData(askFinish);
  assert.equal(askData["state"], "task_failed");
  assert.match(String(askData["reason"]), /Clarification is required/u);
  assert.equal(askData["changes"], "none");
  const askText = eventsFor(peer, "t-ask").find((frame) => frame.event === "workflow.text");
  assert.match(String(eventData(askText ?? askFinish)["text"]), /Which dimension/u);
});
