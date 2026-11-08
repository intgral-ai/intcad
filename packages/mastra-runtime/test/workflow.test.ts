import assert from "node:assert/strict";
import { test } from "node:test";

import type { JsonObject } from "../src/protocol.js";
import { DEFAULT_LIMITS } from "../src/limits.js";
import { HostAiModelClient, type HostAiTransport } from "../src/host-ai/model-client.js";
import { evaluateFixedChecks, runNativeModelingWorkflow } from "../src/workflows/index.js";
import { textReply } from "./support/mcp-peer.js";
import {
  boxState,
  decisionAsk,
  decisionDecline,
  decisionEdit,
  applyCall,
  DEFAULT_BUDGETS,
  FixedValidation,
  fixedRequirements,
  input,
  parameterCheck,
  parameterRequirements,
  passedValidation,
  recordEvents,
  replyText,
  replyToolCalls,
  ScriptedModel,
  ScriptedProposalSource,
  toolCall,
  volumeCheck,
  WorkflowHarness,
} from "./support/workflow-harness.js";

function boxCommands(): { command: string; params: JsonObject }[] {
  return [{ command: "solid.box", params: { length: 20, width: 10, height: 5 } }];
}

function widthCommands(value: number): { command: string; params: JsonObject }[] {
  return [{ command: "parameters.change", params: { name: "width", expression: String(value) } }];
}

test("create produces editable native features and preserves tool-result identity", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const validation = passedValidation("v-create");
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-apply-1", boxCommands())]),
      replyText("Created one editable native box feature."),
    ],
    requirements: fixedRequirements(volumeCheck(1_000)),
    validation,
  });

  assert.equal(result.state, "validated_success");
  assert.equal(result.reasonCode, "VALIDATED_SUCCESS");
  assert.equal(result.changes, "applied");
  assert.equal(result.document.revision, "42");
  assert.equal(result.budgets.modelCalls, 3);
  assert.equal(result.budgets.cadCommands, 1);

  // The result is a native feature timeline and analytic body, not an artifact.
  assert.equal(harness.state.timeline.length, 1);
  assert.equal(harness.state.timeline[0]?.type, "box");
  assert.equal(harness.state.bodies[0]?.volume_mm3, 1_000);
  assert.equal(harness.writes.length, 1);
  assert.deepEqual(harness.stamps, [{ documentId: "doc-A", revision: "41" }]);

  const checks = validation.requests[0]?.checks ?? [];
  assert.equal(checks[0]?.checkId, "geometry.exact");
  assert.equal(checks[0]?.state, "passed");
  assert.equal(checks[0]?.actual["measure:/total/volume_mm3"], 1_000);

  const requests = harness.model?.requests ?? [];
  assert.equal(requests.length, 3, "one explicit model call per round, no hidden loop");
  const decisionTool = requests[1]?.messages.find((message) => message.role === "tool");
  assert.equal(decisionTool?.toolCallId, "call-decision");
  const applyAssistant = requests[2]?.messages.filter((message) => message.role === "assistant").at(-1);
  assert.equal(applyAssistant?.toolCalls[0]?.id, "call-apply-1");
  const applyTool = requests[2]?.messages.find(
    (message) => message.role === "tool" && message.toolCallId === "call-apply-1",
  );
  assert.equal(applyTool?.toolCallId, "call-apply-1");
  assert.match(applyTool?.content[0]?.text ?? "", /"state":"applied"/u);
  assert.match(applyTool?.content[0]?.text ?? "", /"checkId":"geometry.exact"/u);

  assert.deepEqual(result.plan.map((step) => step.id), ["inspect", "decide", "apply", "check", "validate"]);
  assert.equal(result.plan.find((step) => step.id === "apply")?.state, "completed");
  assert.equal(result.plan.find((step) => step.id === "check")?.state, "completed");
  assert.equal(result.plan.find((step) => step.id === "validate")?.state, "completed");
  assert.equal(result.stages.some((stage) => stage.stageId === "inspect" && stage.state === "completed"), true);
  assert.equal(result.stages.some((stage) => stage.stageId === "check" && stage.state === "completed"), true);
  assert.equal(result.stages.some((stage) => stage.stageId === "validate" && stage.state === "completed"), true);
  assert.deepEqual([...new Set(result.tools.map((tool) => tool.callId))], ["call-decision", "call-apply-1"]);
  assert.equal(result.tools.at(-1)?.state, "completed");
  assert.equal(result.tools.at(-1)?.state, "completed");
});

test("the workflow consumes #7's HostAiModelClient as one model.generate per explicit round", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  await harness.client.connect();
  const calls: JsonObject[] = [];
  const replies: JsonObject[] = [
    replyToolCalls([decisionEdit()]) as unknown as JsonObject,
    replyToolCalls([applyCall("call-host-apply", boxCommands())]) as unknown as JsonObject,
    replyText("Created through the real HostAi seam.") as unknown as JsonObject,
  ];
  const transport: HostAiTransport = {
    async call(method, _scope, params, _timeoutSeconds, signal) {
      assert.equal(method, "model.generate");
      assert.equal(signal.aborted, false);
      calls.push(params);
      const reply = replies.shift();
      if (reply === undefined) {
        throw new Error("unexpected extra model.generate");
      }
      return reply;
    },
  };
  const model = new HostAiModelClient(
    transport,
    {
      sessionId: "s-1",
      turnId: "t-1",
      workflowRunId: "w-1",
      connectionId: "byok-1",
      modelId: "model-a",
      remainingSeconds: 1_200,
    },
    DEFAULT_LIMITS,
  );
  const events = recordEvents();
  const result = await runNativeModelingWorkflow(input({ requirements: fixedRequirements(volumeCheck(1_000)) }), {
    model,
    cad: harness.adapter,
    events: events.events,
    signal: new AbortController().signal,
    validation: passedValidation("v-host"),
  });

  assert.equal(result.state, "validated_success");
  assert.equal(calls.length, 3, "one explicit HostAi request per model round");
  assert.equal(calls.every((params) => params["connectionId"] === "byok-1"), true);
  assert.equal(calls.every((params) => params["continuation"] === null), true);
  const tool = (calls[2]?.["messages"] as JsonObject[] | undefined)?.find(
    (message) => message["role"] === "tool" && message["toolCallId"] === "call-host-apply",
  );
  assert.equal(tool?.["toolCallId"], "call-host-apply");
});

test("a completed native workflow turn remains a normal undoable edit", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-create", boxCommands())]),
      replyText("Created one editable native box feature."),
    ],
    requirements: fixedRequirements(volumeCheck(1_000)),
    validation: passedValidation("v-undo"),
  });
  assert.equal(result.state, "validated_success");
  assert.equal(harness.state.timeline.length, 1);
  assert.equal(harness.state.bodies.length, 1);

  const undo = await harness.adapter.execute("edit.undo");
  assert.equal(undo.outcome.state, "applied");
  assert.equal(harness.state.timeline.length, 0, "the native feature history is restored by the ordinary undo command");
  assert.equal(harness.state.bodies.length, 0);
  assert.equal(undo.outcome.after?.revision, "43");
});

test("a parameter edit verifies the requested value and every protected dimension", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const validation = passedValidation("v-edit");
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-apply-width", widthCommands(40))]),
      replyText("Width changed to 40 mm; depth and height are unchanged."),
    ],
    requirements: fixedRequirements(
      parameterRequirements([
        { name: "width", value: 40 },
        { name: "depth", value: 10 },
        { name: "height", value: 5 },
      ]),
    ),
    validation,
  });

  assert.equal(result.state, "validated_success");
  assert.equal(result.changes, "applied");
  assert.equal(harness.state.parameters.get("width")?.value, 40);
  assert.equal(harness.state.parameters.get("depth")?.value, 10);
  assert.equal(harness.state.parameters.get("height")?.value, 5);
  const check = validation.requests[0]?.checks[0];
  assert.equal(check?.state, "passed");
  assert.deepEqual(check?.actual, { width: 40, depth: 10, height: 5 });
  assert.deepEqual(check?.expected, { width: 40, depth: 10, height: 5 });
});

test("a failed check uses at most the configured repairs, then reports a clear quality failure", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const validation = passedValidation();
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-a1", widthCommands(30))]),
      replyToolCalls([applyCall("call-a2", widthCommands(30))]),
      replyToolCalls([applyCall("call-a3", widthCommands(30))]),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
    budgets: { ...DEFAULT_BUDGETS, repairs: 2 },
    validation,
  });

  assert.equal(result.state, "quality_failed");
  assert.equal(result.reasonCode, "QUALITY_FAILED");
  assert.equal(result.changes, "applied");
  assert.equal(result.budgets.repairs, 2);
  assert.equal(harness.writes.length, 3, "the initial stage plus exactly two bounded repairs");
  assert.equal(validation.requests.length, 0, "a failed check never reaches validation");
  assert.match(result.reason, /2 repair attempt/u);
  assert.equal(harness.state.parameters.get("width")?.value, 30);
});

test("model, CAD, repair and wall-time budgets are distinct and reserve before work", async (t) => {
  const modelBound = new WorkflowHarness(t, boxState());
  const modelBudget = await modelBound.run({
    modelReplies: [replyToolCalls([decisionEdit()]), replyToolCalls([applyCall("call-apply", boxCommands())])],
    requirements: fixedRequirements(volumeCheck(1_000)),
    budgets: { modelCalls: 1, agentTasks: 4, cadCommands: 64, repairs: 2, validationRuns: 3, wallTimeSeconds: 1_200 },
  });
  assert.equal(modelBudget.state, "budget_exhausted");
  assert.equal(modelBudget.reasonCode, "BUDGET_EXHAUSTED");
  assert.equal(modelBudget.changes, "none");
  assert.equal(modelBound.writes.length, 0);
  assert.match(modelBudget.reason, /modelCalls/u);

  const cadBound = new WorkflowHarness(t, boxState());
  const cadBudget = await cadBound.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([
        applyCall("call-apply", [
          { command: "parameters.change", params: { name: "width", expression: "40" } },
          { command: "parameters.change", params: { name: "depth", expression: "10" } },
        ]),
      ]),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
    budgets: { modelCalls: 20, agentTasks: 4, cadCommands: 1, repairs: 2, validationRuns: 3, wallTimeSeconds: 1_200 },
  });
  assert.equal(cadBudget.state, "budget_exhausted");
  assert.equal(cadBound.writes.length, 0, "a two-command batch is refused before dispatch when only one command remains");
  assert.match(cadBudget.reason, /cadCommands/u);

  const wallBound = new WorkflowHarness(t, boxState());
  const wallBudget = await wallBound.run({
    modelReplies: [replyToolCalls([decisionEdit()])],
    budgets: { modelCalls: 20, agentTasks: 4, cadCommands: 64, repairs: 2, validationRuns: 3, wallTimeSeconds: 0 },
  });
  assert.equal(wallBudget.state, "budget_exhausted");
  assert.match(wallBudget.reason, /wallTimeSeconds/u);
  assert.equal(wallBound.model?.requests.length ?? 0, 0);
});

test("cancellation before dispatch and after an earlier live edit are distinct from other terminals", async (t) => {
  const before = new WorkflowHarness(t, boxState());
  const preAborted = new AbortController();
  preAborted.abort();
  const cancelledBefore = await before.run({
    modelReplies: [replyToolCalls([decisionEdit()])],
    signal: preAborted.signal,
  });
  assert.equal(cancelledBefore.state, "cancelled");
  assert.equal(cancelledBefore.changes, "none");
  assert.equal(before.writes.length, 0);

  const afterEdit = new WorkflowHarness(t, boxState());
  const controller = new AbortController();
  const cancelledAfterEdit = await afterEdit.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-apply-width", widthCommands(40))]),
      replyText("This reply is never consumed because Stop arrived."),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
    signal: controller.signal,
    onModelReply: (index) => {
      if (index === 2) {
        controller.abort();
      }
    },
  });
  assert.equal(cancelledAfterEdit.state, "cancelled");
  assert.equal(cancelledAfterEdit.changes, "applied");
  assert.equal(afterEdit.writes.length, 1, "no write starts after cancellation");
  assert.equal(afterEdit.state.parameters.get("width")?.value, 40);
});

test("a speculative probe returns UNSUPPORTED_CAPABILITY with zero writes", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const validation = passedValidation();
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([toolCall("call-probe", "probe_cad", { command: "solid.box" })]),
      replyText("The isolated probe is unavailable."),
    ],
    validation,
  });

  assert.equal(result.state, "task_failed");
  assert.equal(result.reasonCode, "NO_RESULT");
  assert.equal(result.changes, "none");
  assert.equal(harness.writes.length, 0);
  assert.equal(validation.requests.length, 0);
  const probe = result.tools.find((tool) => tool.callId === "call-probe");
  assert.equal(probe?.state, "failed");
  assert.equal(probe?.output?.["code"], "UNSUPPORTED_CAPABILITY");
});

test("a stale stamp re-inspects and requires a new model round before another write", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  harness.staleOnce = true;
  const validation = passedValidation("v-stale");
  const twoSteps = [
    { command: "parameters.change", params: { name: "width", expression: "40" } },
    { command: "parameters.change", params: { name: "depth", expression: "10" } },
  ];
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-a1", twoSteps)]),
      replyToolCalls([applyCall("call-a2", twoSteps)]),
      replyText("Replanned after the stale stamp."),
    ],
    requirements: fixedRequirements(parameterRequirements([{ name: "width", value: 40 }])),
    validation,
  });

  assert.equal(result.state, "validated_success");
  assert.equal(result.changes, "applied");
  assert.equal(harness.writes.length, 2);
  assert.deepEqual(harness.stamps, [
    { documentId: "doc-A", revision: "41" },
    { documentId: "doc-A", revision: "42" },
  ]);
  assert.equal(harness.model?.requests.length, 4, "the workflow required another explicit model round");
  const staleTool = result.tools.filter((tool) => tool.callId === "call-a1").at(-1);
  assert.equal(staleTool?.output?.["state"], "stale");
  assert.equal(staleTool?.output?.["replan"], true);
  const inspections = harness.peer
    .toolCallsNamed("execute")
    .filter((call) => call.args["command"] === "document.revision");
  assert.equal(inspections.length >= 3, true, "inspection ran at turn start and again after the stale reply");
  assert.equal(validation.requests[0]?.document.revision, "43");
});

test("unknown write effects stop further modeling and end outcome_unknown", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  harness.unknownOnce = true;
  const validation = passedValidation();
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([
        applyCall("call-a1", [
          { command: "parameters.change", params: { name: "width", expression: "40" } },
          { command: "parameters.change", params: { name: "depth", expression: "10" } },
        ]),
      ]),
      replyText("This must never be asked for."),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
    validation,
  });

  assert.equal(result.state, "outcome_unknown");
  assert.equal(result.reasonCode, "OUTCOME_UNKNOWN");
  assert.equal(result.changes, "unknown");
  assert.equal(harness.writes.length, 1);
  assert.equal(harness.model?.requests.length, 2, "no replacement command and no further model round");
  assert.equal(validation.requests.length, 0);
});

test("a pending reply is reconciled on the same connection and never resent", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  harness.pendingOnce = true;
  const validation = passedValidation("v-pending");
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([
        applyCall("call-a1", [
          { command: "parameters.change", params: { name: "width", expression: "40" } },
          { command: "parameters.change", params: { name: "depth", expression: "10" } },
        ]),
      ]),
      replyText("Reconciled the original request."),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
    validation,
  });

  assert.equal(result.state, "validated_success");
  assert.equal(result.changes, "applied");
  assert.equal(harness.writes.length, 1, "the mutation was reconciled, not resent");
  assert.equal(harness.state.parameters.get("width")?.value, 40);
  assert.equal(harness.peer.toolCallsNamed("pending").length >= 1, true);
});

test("an expectation mismatch is preserved and does not become success", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  harness.expectationFailedOnce = true;
  const validation = passedValidation();
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([
        applyCall("call-a1", [
          { command: "parameters.change", params: { name: "width", expression: "40" } },
          { command: "parameters.change", params: { name: "depth", expression: "10" } },
        ]),
      ]),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
    validation,
  });

  assert.equal(result.state, "task_failed");
  assert.equal(result.reasonCode, "EXPECTATION_MISMATCH");
  assert.equal(result.changes, "applied");
  assert.deepEqual(result.document, { documentId: "doc-A", revision: "42" });
  assert.equal(validation.requests.length, 0);
});

test("an unsupported native command is reported distinctly and performs no write", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-evil", [{ command: "solid.not-a-real-command", params: {} }])]),
    ],
    requirements: fixedRequirements(parameterCheck("width", 20)),
  });

  assert.equal(result.state, "task_failed");
  assert.equal(result.reasonCode, "UNSUPPORTED_CAPABILITY");
  assert.equal(result.changes, "none");
  assert.equal(harness.writes.length, 0);
});

test("no LLM text produces verified success without a native result and host validation", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const validation = passedValidation();
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyText("All dimensions are verified and the result passed every check."),
    ],
    requirements: fixedRequirements(parameterCheck("width", 20)),
    validation,
  });

  assert.equal(result.state, "task_failed");
  assert.equal(result.reasonCode, "NO_RESULT");
  assert.equal(result.changes, "none");
  assert.equal(validation.requests.length, 0);
  assert.match(result.answer, /verified/u);
});

test("passing stage checks without the #13 validation handoff are not reported as success", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-create", boxCommands())]),
      replyText("Created the box."),
    ],
    requirements: fixedRequirements(volumeCheck(1_000)),
    validation: null,
  });

  assert.equal(result.state, "task_failed");
  assert.equal(result.reasonCode, "VALIDATION_UNAVAILABLE");
  assert.equal(result.changes, "applied");
  assert.equal(result.checks[0]?.state, "passed");
});

test("ask/decline stops further mutations and reports earlier live edits truthfully", async (t) => {
  const ask = new WorkflowHarness(t, boxState());
  const asked = await ask.run({ modelReplies: [replyToolCalls([decisionAsk()])] });
  assert.equal(asked.state, "task_failed");
  assert.equal(asked.reasonCode, "CLARIFICATION_REQUIRED");
  assert.equal(asked.changes, "none");
  assert.equal(ask.writes.length, 0);
  assert.match(asked.answer, /Which dimension/u);

  const decline = new WorkflowHarness(t, boxState());
  const declined = await decline.run({ modelReplies: [replyToolCalls([decisionDecline()])] });
  assert.equal(declined.state, "task_failed");
  assert.equal(declined.reasonCode, "DECLINED");
  assert.equal(decline.writes.length, 0);

  const afterEdit = new WorkflowHarness(t, boxState());
  const editedThenAsked = await afterEdit.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-width", widthCommands(40))]),
      replyToolCalls([toolCall("call-ask-2", "record_decision", { decision: "ask", reason: "The protected dimension is ambiguous." })]),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
  });
  assert.equal(editedThenAsked.state, "task_failed");
  assert.equal(editedThenAsked.reasonCode, "CLARIFICATION_REQUIRED");
  assert.equal(editedThenAsked.changes, "applied");
  assert.equal(afterEdit.writes.length, 1, "the earlier live edit remains and undo stays available");
});

test("ACP proposal-only mode validates the source stamp before any guarded write", async (t) => {
  const invalid = new WorkflowHarness(t, boxState());
  const invalidProposal = await invalid.run({
    mode: "acp",
    proposalSource: new ScriptedProposalSource(["{not json"]),
    requirements: fixedRequirements(parameterCheck("width", 40)),
  });
  assert.equal(invalidProposal.state, "task_failed");
  assert.equal(invalidProposal.reasonCode, "INVALID_PROPOSAL");
  assert.equal(invalid.writes.length, 0);

  const unavailable = new WorkflowHarness(t, boxState());
  const noSource = await unavailable.run({
    mode: "acp",
    proposalSource: null,
    requirements: fixedRequirements(parameterCheck("width", 40)),
  });
  assert.equal(noSource.state, "task_failed");
  assert.equal(noSource.reasonCode, "UNSUPPORTED_CAPABILITY");
  assert.equal(unavailable.writes.length, 0);

  const valid = new WorkflowHarness(t, boxState());
  const validation = passedValidation("v-acp");
  const accepted = await valid.run({
    mode: "acp",
    proposalSource: new ScriptedProposalSource([
      JSON.stringify({
        source: { documentId: "doc-A", revision: "41" },
        commands: [{ command: "parameters.change", params: { name: "width", expression: "40" } }],
      }),
    ]),
    requirements: fixedRequirements(parameterRequirements([{ name: "width", value: 40 }])),
    validation,
  });
  assert.equal(accepted.state, "validated_success");
  assert.equal(accepted.changes, "applied");
  assert.equal(valid.state.parameters.get("width")?.value, 40);
  assert.equal(valid.writes.length, 1);
  assert.equal(validation.requests.length, 1);
});

test("fixed requirements reject malformed or non-finite tolerances before any planning", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const result = await harness.run({
    modelReplies: [replyToolCalls([decisionEdit()])],
    requirements: {
      checks: [
        {
          checkId: "geometry.exact",
          kind: "geometry.exact",
          required: true,
          parameters: {
            measurements: [
              {
                command: "measure",
                params: {},
                resultPointer: "/total/volume_mm3",
                expected: Number.NaN,
                absTolerance: 0,
                relTolerance: 0,
              },
            ],
          },
        },
      ],
    } as never,
  });

  assert.equal(result.state, "task_failed");
  assert.equal(result.reasonCode, "INVALID_REQUEST");
  assert.equal(harness.writes.length, 0);
  assert.equal(harness.model?.requests.length ?? 0, 0);
});

test("protected parameter checks read the real native document.inspect params key", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  await harness.client.connect();
  harness.peer.setHandler("inspect_design", () =>
    textReply(
      JSON.stringify({
        name: "native-reply",
        params: [
          { name: "width", expression: "40", value: 40, unit: "mm", error: null },
          { name: "depth", expression: "10", value: 10, unit: "mm", error: null },
          { name: "height", expression: "5", value: 5, unit: "mm", error: null },
        ],
        timeline: [],
        bodies: [],
      }),
    ),
  );

  const checks = await evaluateFixedChecks(
    harness.adapter,
    fixedRequirements(parameterRequirements([
      { name: "width", value: 40 },
      { name: "depth", value: 10 },
      { name: "height", value: 5 },
    ])),
  );
  assert.equal(checks[0]?.state, "passed", JSON.stringify(checks[0]));
  assert.deepEqual(checks[0]?.actual, { width: 40, depth: 10, height: 5 });
});

test("a model answer after a failed check cannot reach validation or claim checks passed", async (t) => {
  for (const validation of [null, passedValidation("v-must-not-run")]) {
    const harness = new WorkflowHarness(t, boxState());
    const result = await harness.run({
      modelReplies: [
        replyToolCalls([decisionEdit()]),
        replyToolCalls([applyCall("call-wrong", widthCommands(30))]),
        replyText("Everything passed, honestly."),
      ],
      requirements: fixedRequirements(parameterCheck("width", 40)),
      validation,
    });

    assert.equal(result.state, "quality_failed");
    assert.equal(result.reasonCode, "QUALITY_FAILED");
    assert.match(result.reason, /did not pass/u);
    assert.doesNotMatch(result.reason, /checks passed/u);
    assert.equal(result.checks[0]?.state, "failed");
    assert.equal(result.plan.find((step) => step.id === "check")?.state, "failed");
    assert.equal(result.tools.at(-1)?.state, "failed");
    if (validation !== null) {
      assert.equal(validation.requests.length, 0);
    }
  }
});

test("a partial command failure adopts its resulting stamp and reports applied", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  harness.partialFailureOnce = true;
  const validation = passedValidation();
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([
        applyCall("call-partial", [
          { command: "parameters.change", params: { name: "width", expression: "40" } },
          { command: "solid.fail", params: {} },
        ]),
      ]),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
    validation,
  });

  assert.equal(result.state, "task_failed");
  assert.equal(result.reasonCode, "COMMAND_FAILED");
  assert.equal(result.changes, "applied");
  assert.deepEqual(result.document, { documentId: "doc-A", revision: "42" });
  assert.equal(validation.requests.length, 0);
});

test("a rollback-restored command failure can replan and still finish its checks", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const validation = passedValidation("v-rollback");
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([
        applyCall("call-fail", [
          { command: "parameters.change", params: { name: "width", expression: "40" } },
          { command: "solid.fail", params: {} },
        ]),
      ]),
      replyToolCalls([applyCall("call-width", widthCommands(40))]),
      replyText("Recovered after the rolled-back command."),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
    validation,
  });

  assert.equal(result.state, "validated_success");
  assert.equal(result.changes, "applied");
  assert.equal(harness.writes.length, 2);
  assert.equal(result.budgets.repairs, 1);
  assert.equal(validation.requests.length, 1);
});

test("cancellation during an in-flight write keeps the applied edit and its stamp", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const controller = new AbortController();
  harness.duringWrite = () => controller.abort();
  const validation = passedValidation();
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-cancel-write", widthCommands(40))]),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
    validation,
    signal: controller.signal,
  });

  assert.equal(result.state, "cancelled");
  assert.equal(result.changes, "applied");
  assert.deepEqual(result.document, { documentId: "doc-A", revision: "42" });
  assert.equal(harness.state.parameters.get("width")?.value, 40);
  assert.equal(harness.writes.length, 1);
  assert.equal(validation.requests.length, 0);
});

test("the initial inspect must match the frozen turn binding", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const result = await harness.run({
    modelReplies: [replyToolCalls([decisionEdit()])],
    document: { documentId: "doc-A", revision: "40" },
  });

  assert.equal(result.state, "task_failed");
  assert.equal(result.reasonCode, "STALE_DOCUMENT");
  assert.equal(result.changes, "none");
  assert.equal(harness.model?.requests.length ?? 0, 0);
  assert.equal(harness.writes.length, 0);
});

test("prior session history is carried into the next native model round", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyText("No native result this time."),
    ],
    history: [
      {
        role: "user",
        content: [{ type: "text", text: "Earlier user request." }],
        toolCalls: [],
        toolCallId: null,
      },
      {
        role: "assistant",
        content: [{ type: "text", text: "Earlier assistant answer." }],
        toolCalls: [],
        toolCallId: null,
      },
    ],
  });

  assert.equal(result.state, "task_failed");
  const messages = harness.model?.requests[0]?.messages ?? [];
  assert.equal(messages[1]?.role, "user");
  assert.equal(messages[1]?.content[0]?.text, "Earlier user request.");
  assert.equal(messages[2]?.role, "assistant");
  assert.equal(messages[2]?.content[0]?.text, "Earlier assistant answer.");
  assert.equal(messages.at(-1)?.content[0]?.text, "Make the requested native model change.");
});

test("tool-call identities are unique across rounds, not only within one reply", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([toolCall("call-dupe", "record_decision", { decision: "edit", reason: "Edit." })]),
      replyToolCalls([toolCall("call-dupe", "apply_native_commands", { commands: boxCommands() })]),
    ],
    requirements: fixedRequirements(volumeCheck(1_000)),
  });

  assert.equal(result.state, "task_failed");
  assert.equal(result.reasonCode, "MALFORMED_RESPONSE");
  assert.equal(harness.writes.length, 0);
});

test("apply and ACP proposals reject undeclared fields", async (t) => {
  const applyHarness = new WorkflowHarness(t, boxState());
  const applyResult = await applyHarness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([
        toolCall("call-extra", "apply_native_commands", {
          commands: widthCommands(40),
          surprise: true,
        }),
      ]),
      replyText("No write was made."),
    ],
    requirements: fixedRequirements(parameterCheck("width", 40)),
  });
  assert.equal(applyResult.state, "task_failed");
  assert.equal(applyHarness.writes.length, 0);
  assert.equal(applyResult.tools.at(-1)?.output?.["code"], "INVALID_REQUEST");

  const acpHarness = new WorkflowHarness(t, boxState());
  const acpResult = await acpHarness.run({
    mode: "acp",
    proposalSource: new ScriptedProposalSource([
      JSON.stringify({
        source: { documentId: "doc-A", revision: "41" },
        commands: [{ command: "parameters.change", params: { name: "width", expression: "40" } }],
        surprise: true,
      }),
    ]),
    requirements: fixedRequirements(parameterCheck("width", 40)),
  });
  assert.equal(acpResult.state, "task_failed");
  assert.equal(acpResult.reasonCode, "INVALID_PROPOSAL");
  assert.equal(acpHarness.writes.length, 0);
});

test("a probe that unexpectedly returns still gets a matching tool result", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  await harness.client.connect();
  const cad = {
    get stamp() {
      return harness.adapter.stamp;
    },
    get requiresReconciliation() {
      return harness.adapter.requiresReconciliation;
    },
    capabilities: () => harness.adapter.capabilities(),
    inspect: (options?: { readonly design?: boolean }) => harness.adapter.inspect(options),
    readTool: (name: string, args?: JsonObject) => harness.adapter.readTool(name, args),
    execute: (command: string, params?: JsonObject, options?: { readonly expect?: JsonObject; readonly verbose?: boolean }) =>
      harness.adapter.execute(command, params, options),
    batch: (steps: Parameters<typeof harness.adapter.batch>[0], options?: Parameters<typeof harness.adapter.batch>[1]) =>
      harness.adapter.batch(steps, options),
    probe: (() => undefined) as unknown as import("../src/workflows/index.js").CadPort["probe"],
    reconcile: () => harness.adapter.reconcile(),
    cancel: (reason?: string) => harness.adapter.cancel(reason),
  };
  const model = new ScriptedModel([
    replyToolCalls([decisionEdit()]),
    replyToolCalls([toolCall("call-probe-return", "probe_cad", {})]),
    replyText("Probe unavailable."),
  ]);
  const events = recordEvents();
  const result = await runNativeModelingWorkflow(input(), {
    model,
    cad,
    events: events.events,
    signal: new AbortController().signal,
    validation: passedValidation(),
  });

  assert.equal(result.state, "task_failed");
  const tool = model.requests[2]?.messages.find(
    (message) => message.role === "tool" && message.toolCallId === "call-probe-return",
  );
  assert.notEqual(tool, undefined);
  assert.equal(result.tools.find((event) => event.callId === "call-probe-return")?.state, "failed");
});

test("ACP task and validation budgets are distinct and reserved before dispatch", async (t) => {
  const acpHarness = new WorkflowHarness(t, boxState());
  const acpSource = new ScriptedProposalSource([
    JSON.stringify({
      source: { documentId: "doc-A", revision: "41" },
      commands: [{ command: "parameters.change", params: { name: "width", expression: "40" } }],
    }),
  ]);
  const acpResult = await acpHarness.run({
    mode: "acp",
    proposalSource: acpSource,
    requirements: fixedRequirements(parameterCheck("width", 40)),
    budgets: { ...DEFAULT_BUDGETS, agentTasks: 0 },
  });
  assert.equal(acpResult.state, "budget_exhausted");
  assert.match(acpResult.reason, /agentTasks/u);
  assert.equal(acpSource.requests.length, 0);
  assert.equal(acpHarness.writes.length, 0);

  const validationHarness = new WorkflowHarness(t, boxState());
  const validation = passedValidation();
  const validationResult = await validationHarness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-box", boxCommands())]),
      replyText("Created."),
    ],
    requirements: fixedRequirements(volumeCheck(1_000)),
    budgets: { ...DEFAULT_BUDGETS, validationRuns: 0 },
    validation,
  });
  assert.equal(validationResult.state, "budget_exhausted");
  assert.match(validationResult.reason, /validationRuns/u);
  assert.equal(validation.requests.length, 0);
});

test("a passing report for a changed live stamp is historical, not validated success", async (t) => {
  const harness = new WorkflowHarness(t, boxState());
  const validation = new FixedValidation(() => {
    harness.peer.revision += 1n;
    return {
      state: "passed",
      runId: "v-stale",
      report: { schemaVersion: "1.0", runId: "v-stale", state: "passed" },
      reason: "Passed for the captured candidate.",
    };
  });
  const result = await harness.run({
    modelReplies: [
      replyToolCalls([decisionEdit()]),
      replyToolCalls([applyCall("call-box", boxCommands())]),
      replyText("Created."),
    ],
    requirements: fixedRequirements(volumeCheck(1_000)),
    validation,
  });

  assert.equal(result.state, "quality_failed");
  assert.equal(result.reasonCode, "STALE_DOCUMENT");
  assert.equal(result.validationRunId, "v-stale");
  assert.equal(result.changes, "applied");
  assert.deepEqual(result.document, { documentId: "doc-A", revision: "43" });
});
