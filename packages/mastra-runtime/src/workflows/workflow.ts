/**
 * Native SolidCraft modeling workflow.
 *
 * Stage flow: inspect/freeze -> plan/decision -> ordered native apply/check
 * rounds (bounded repair) -> host snapshot-validation handoff (#13).
 *
 * The outer workflow owns every HostAi model step and every native tool
 * execution. A returned tool call is a proposal; only #9's guarded adapter
 * says whether anything was applied. No model text is a verification decision.
 */

import { monotonicClock, type Clock } from "../clock.js";
import { asProtocolError, cancelled, invalidRequest, ProtocolError } from "../errors.js";
import { DEFAULT_LIMITS, isObject, type Limits } from "../limits.js";
import { isId, type JsonObject } from "../protocol.js";
import type { ModelMessage } from "../host-ai/model-client.js";
import type { CadWriteResult, DocumentStamp } from "../solidcraft/cad-adapter.js";
import { applyProposal, failedCheckSummary, rollbackRestored, writeOutput } from "./apply.js";
import { BudgetMeter } from "./budget.js";
import {
  evaluateFixedChecks,
  failedCheckIds,
  requiredChecksPassed,
  replyValue,
  validateRequirements,
} from "./checks.js";
import { documentStampEquals, inspectAndFreeze, prepareTurn } from "./inspect.js";
import {
  APPLY_TOOL,
  DECISION_TOOL,
  PROBE_TOOL,
  READ_TOOLS,
  assistantMessage,
  errorOutput,
  parseAcpProposal,
  parseApplyProposal,
  parseDecision,
  toolMessage,
  type ApplyProposal,
  type Decision,
} from "./proposals.js";
import type {
  ChangeState,
  CheckResult,
  FrozenInspection,
  NativeWorkflowInput,
  PlanStep,
  PlanStepState,
  StageEvent,
  StageState,
  TextChannel,
  ToolEvent,
  ToolState,
  WorkflowEvents,
  WorkflowOptions,
  WorkflowResult,
  WorkflowTerminalState,
} from "./types.js";

const PLAN_TEMPLATE: readonly Omit<PlanStep, "state">[] = Object.freeze([
  { id: "inspect", title: "Inspect and freeze the bound design" },
  { id: "decide", title: "Record the edit, ask or decline decision" },
  { id: "apply", title: "Apply ordered native commands through the guarded adapter" },
  { id: "check", title: "Check the fixed requirements with exact native evidence" },
  { id: "validate", title: "Hand the native snapshot to host validation" },
]);

class Recorder implements WorkflowEvents {
  readonly #sink: WorkflowEvents;
  readonly #stages: StageEvent[] = [];
  readonly #tools: ToolEvent[] = [];
  #plan: PlanStep[] = PLAN_TEMPLATE.map((step) => ({ ...step, state: "pending" as const }));

  constructor(sink: WorkflowEvents) {
    this.#sink = sink;
  }

  get stages(): readonly StageEvent[] {
    return [...this.#stages];
  }

  get planSteps(): readonly PlanStep[] {
    return this.#plan.map((step) => ({ ...step }));
  }

  get tools(): readonly ToolEvent[] {
    return [...this.#tools];
  }

  stage(stageId: string, title: string, state: StageState): void {
    const event = { stageId, title, state };
    this.#stages.push(event);
    this.#sink.stage(stageId, title, state);
  }

  plan(steps: readonly PlanStep[]): void {
    this.#plan = steps.map((step) => ({ ...step }));
    this.#sink.plan(this.#plan.map((step) => ({ ...step })));
  }

  step(id: string, state: PlanStepState): void {
    let changed = false;
    this.#plan = this.#plan.map((step) => {
      if (step.id !== id || step.state === state) {
        return step;
      }
      changed = true;
      return { ...step, state };
    });
    if (changed) {
      this.#sink.plan(this.#plan.map((step) => ({ ...step })));
    }
  }

  text(text: string, channel: TextChannel): void {
    this.#sink.text(text, channel);
  }

  tool(callId: string, name: string, state: ToolState, output: JsonObject | null): void {
    const event = { callId, name, state, output };
    this.#tools.push(event);
    this.#sink.tool(callId, name, state, output);
  }
}

interface Disposition {
  readonly kind: "continue";
  readonly output: JsonObject;
}

type Outcome = Disposition | { readonly kind: "terminal"; readonly result: WorkflowResult };

interface RunContext {
  readonly input: NativeWorkflowInput;
  readonly options: WorkflowOptions;
  readonly limits: Limits;
  readonly clock: Clock;
  readonly meter: BudgetMeter;
  readonly recorder: Recorder;
  readonly signal: AbortSignal;
  document: DocumentStamp;
  changes: ChangeState;
  answer: string;
  appliedAny: boolean;
  checks: CheckResult[];
  checksPassed: boolean;
  readonly usedCallIds: Set<string>;
}

export async function runNativeModelingWorkflow(
  input: NativeWorkflowInput,
  options: WorkflowOptions,
): Promise<WorkflowResult> {
  const clock = options.clock ?? monotonicClock;
  const recorder = new Recorder(options.events);
  const context: RunContext = {
    input,
    options,
    limits: options.limits ?? DEFAULT_LIMITS,
    clock,
    meter: new BudgetMeter(input.budgets, clock),
    recorder,
    signal: options.signal,
    document: cloneStamp(input.document),
    changes: "none",
    answer: "",
    appliedAny: false,
    checks: [],
    checksPassed: false,
    usedCallIds: new Set<string>(),
  };
  const finish = (state: WorkflowTerminalState, reason: string, reasonCode: string, extra: Partial<WorkflowResult> = {}): WorkflowResult =>
    buildResult(context, state, reason, reasonCode, extra);
  const fail = (error: unknown): WorkflowResult => finishFromError(context, error);

  try {
    assertActive(context);
    recorder.plan(PLAN_TEMPLATE.map((step) => ({ ...step, state: "pending" as const })));
    recorder.step("inspect", "running");
    recorder.stage("inspect", "Inspect and freeze the bound design", "running");
    const frozen = await inspectAndFreeze(options.cad, input, context.limits);
    if (!documentStampEquals(input.document, frozen.stamp)) {
      recorder.stage("inspect", "Inspect and freeze the bound design", "failed");
      throw new ProtocolError(
        "STALE_DOCUMENT",
        "The bound design changed before planning; no native command was executed. Re-inspect and start a new turn.",
        "never",
        { expected: input.document, current: frozen.stamp },
      );
    }
    context.document = cloneStamp(frozen.stamp);
    recorder.stage("inspect", "Inspect and freeze the bound design", "completed");
    recorder.step("inspect", "completed");
    assertActive(context);

    if (!frozen.capabilities.guardedExecute && !frozen.capabilities.guardedBatch) {
      return finish(
        "task_failed",
        "The SolidCraft bridge exposes no guarded execute/batch path; no write was attempted.",
        "UNSUPPORTED_CAPABILITY",
      );
    }

    if ((input.mode ?? "byok") === "acp") {
      return await runAcp(context, frozen, finish);
    }
    return await runByok(context, frozen, finish);
  } catch (error) {
    return fail(error);
  }
}

async function runByok(
  context: RunContext,
  frozen: FrozenInspection,
  finish: ResultFactory,
): Promise<WorkflowResult> {  const prepared = prepareTurn(context.input, frozen, context.limits);
  const messages: ModelMessage[] = [...prepared.messages];
  const tools = [...prepared.tools];
  let continuation: string | null = null;
  let decision: Decision | null = null;

  context.recorder.step("decide", "running");
  context.recorder.stage("plan", "Plan the native modeling stages", "running");

  for (;;) {
    assertActive(context);
    context.meter.assertTime();
    context.meter.reserveModel();
    const timeoutSeconds = Math.min(context.limits.callDefaultSeconds, context.meter.remainingSeconds());
    const result = await context.options.model.generate(
      {
        messages,
        tools,
        options: {
          delivery: "buffered",
          timeoutSeconds,
          maxOutputTokens: 4096,
          temperature: null,
          reasoningEffort: null,
          outputSchema: null,
        },
        continuation,
      },
      context.signal,
    );
    assertActive(context);
    continuation = result.continuation;
    const text = result.content.map((block) => block.text).join("");

    if (result.finishReason === "refusal") {
      context.answer = text;
      if (text.length > 0) {
        context.recorder.text(text, "answer");
      }
      return finish("task_failed", "The model refused the modeling request.", "MODEL_REFUSED");
    }
    if (result.finishReason === "length") {
      return finish("task_failed", "The model response was truncated before a complete proposal.", "MODEL_TRUNCATED");
    }
    if (result.finishReason === "unknown") {
      return finish("task_failed", "The model returned an unknown finish reason.", "MODEL_UNKNOWN");
    }

    if (result.toolCalls.length === 0) {
      context.answer = text;
      if (text.length > 0) {
        context.recorder.text(text, "answer");
      }
      if (!context.appliedAny) {
        return finish(
          "task_failed",
          text.length > 0
            ? "The model produced an answer but no native modeling result to validate."
            : "The model produced no native modeling result.",
          "NO_RESULT",
        );
      }
      context.recorder.step("decide", "completed");
      context.recorder.stage("plan", "Plan the native modeling stages", "completed");
      if (!context.checksPassed || !requiredChecksPassed(context.checks)) {
        context.recorder.step("check", "failed");
        return finish(
          "quality_failed",
          `Fixed stage checks did not pass: ${failedCheckSummary(context.checks) || "no exact check result"}.`,
          "QUALITY_FAILED",
        );
      }
      return validateAndFinish(context, finish);
    }

    if (text.length > 0) {
      context.recorder.text(text, "progress");
    }
    if (result.toolCalls.length > context.limits.toolCalls) {
      return finish("task_failed", "The model returned more tool calls than the negotiated limit.", "MALFORMED_RESPONSE");
    }
    const seen = context.usedCallIds;
    for (const call of result.toolCalls) {
      if (!isId(call.id) || seen.has(call.id)) {
        return finish(
          "task_failed",
          "The model returned a duplicate or invalid tool-call identity; no tool was executed.",
          "MALFORMED_RESPONSE",
        );
      }
      seen.add(call.id);
    }
    messages.push(assistantMessage(text, result.toolCalls));

    for (const call of result.toolCalls) {
      assertActive(context);
      if (call.name === DECISION_TOOL) {
        let parsed: Decision;
        try {
          parsed = parseDecision(call.arguments);
        } catch (error) {
          appendToolFailure(context, messages, call.id, call.name, error);
          continue;
        }
        if (decision !== null) {
          if (parsed.decision === "edit") {
            appendToolFailure(
              context,
              messages,
              call.id,
              call.name,
              invalidRequest("An edit decision was already recorded for this turn.", { field: "decision" }),
            );
            continue;
          }
          // A later ask/decline stops further mutations even after earlier edits.
          context.recorder.tool(call.id, call.name, "completed", {
            decision: parsed.decision,
            reason: parsed.reason,
          });
          messages.push(
            toolMessage(call.id, { ok: true, decision: parsed.decision, reason: parsed.reason }, context.limits.textBytes),
          );
          context.answer = parsed.answer;
          if (parsed.answer.length > 0) {
            context.recorder.text(parsed.answer, "answer");
          }
          context.recorder.stage("plan", "Plan the native modeling stages", "completed");
          return finish(
            "task_failed",
            parsed.decision === "ask"
              ? `Clarification is required: ${parsed.reason}`
              : `The task was declined: ${parsed.reason}`,
            parsed.decision === "ask" ? "CLARIFICATION_REQUIRED" : "DECLINED",
          );
        }
        if (parsed.decision !== "edit") {
          context.recorder.tool(call.id, call.name, "completed", {
            decision: parsed.decision,
            reason: parsed.reason,
          });
          messages.push(
            toolMessage(call.id, { ok: true, decision: parsed.decision, reason: parsed.reason }, context.limits.textBytes),
          );
          context.answer = parsed.answer;
          if (parsed.answer.length > 0) {
            context.recorder.text(parsed.answer, "answer");
          }
          context.recorder.step("decide", "completed");
          context.recorder.stage("plan", "Plan the native modeling stages", "completed");
          return finish(
            "task_failed",
            parsed.decision === "ask"
              ? `Clarification is required: ${parsed.reason}`
              : `The task was declined: ${parsed.reason}`,
            parsed.decision === "ask" ? "CLARIFICATION_REQUIRED" : "DECLINED",
          );
        }
        decision = parsed;
        context.recorder.tool(call.id, call.name, "completed", { decision: "edit", reason: parsed.reason });
        messages.push(
          toolMessage(call.id, { ok: true, decision: "edit", reason: parsed.reason }, context.limits.textBytes),
        );
        context.recorder.step("decide", "completed");
        context.recorder.step("apply", "running");
        continue;
      }

      if (call.name === APPLY_TOOL) {
        if (decision === null) {
          // An edit decision permits planning, not permission bypass.
          appendToolFailure(
            context,
            messages,
            call.id,
            call.name,
            invalidRequest("Record an edit decision before proposing native commands.", { field: "decision" }),
          );
          continue;
        }
        let proposal: ApplyProposal;
        try {
          proposal = parseApplyProposal(call.arguments);
        } catch (error) {
          appendToolFailure(context, messages, call.id, call.name, error);
          continue;
        }
        context.meter.reserveCad(proposal.commands.length);
        assertActive(context);
        context.recorder.tool(call.id, call.name, "running", { commands: proposal.commands.length });
        let write: CadWriteResult;
        try {
          write = await applyProposal(context.options.cad, proposal);
        } catch (error) {
          context.recorder.tool(call.id, call.name, "failed", protocolOutput(error));
          throw error;
        }
        // Process the returned write facts before the cancellation check: a
        // mutation that committed while Stop arrived is still a live edit.
        const outcome = await handleWrite(context, call.id, call.name, write);
        if (outcome.kind === "terminal") {
          return outcome.result;
        }
        messages.push(toolMessage(call.id, outcome.output, context.limits.textBytes));
        continue;
      }

      if (call.name === PROBE_TOOL) {
        let output: JsonObject = {
          ok: false,
          code: "UNSUPPORTED_CAPABILITY",
          message: "A speculative candidate probe is unsupported.",
        };
        try {
          context.options.cad.probe();
        } catch (error) {
          output = protocolOutput(error);
        }
        context.recorder.tool(call.id, call.name, "failed", output);
        messages.push(toolMessage(call.id, output, context.limits.textBytes));
        continue;
      }

      if (READ_TOOLS.has(call.name)) {
        try {
          const read = await context.options.cad.readTool(call.name, call.arguments);
          assertActive(context);
          const body = replyValue(read);
          const output: JsonObject = { ok: true, tool: call.name };
          if (body !== null) {
            output["result"] = body;
          } else if (read.text !== null) {
            output["text"] = read.text;
          }
          context.recorder.tool(call.id, call.name, "completed", output);
          messages.push(toolMessage(call.id, output, context.limits.textBytes));
        } catch (error) {
          appendToolFailure(context, messages, call.id, call.name, error);
        }
        continue;
      }

      appendToolFailure(
        context,
        messages,
        call.id,
        call.name,
        invalidRequest(`The workflow does not expose a native tool named \`${call.name}\`.`, { field: "tool" }),
      );
    }
  }
}

async function runAcp(
  context: RunContext,
  frozen: FrozenInspection,
  finish: ResultFactory,
): Promise<WorkflowResult> {
  const source = context.options.proposalSource ?? null;
  if (source === null) {
    return finish(
      "task_failed",
      "ACP proposal-only mode is unavailable; no agent task was requested and no CAD command was applied.",
      "UNSUPPORTED_CAPABILITY",
    );
  }
  let staleRetried = false;
  for (;;) {
    assertActive(context);
    context.meter.assertTime();
    context.meter.reserveAgentTask();
    context.recorder.step("decide", "running");
    context.recorder.stage("plan", "Request a proposal-only ACP task", "running");
    const reply = await source.requestProposal(
      {
        instructions: context.input.instructions,
        task: acpTask(context, frozen),
        document: context.document,
        selection: context.input.selection,
        capabilities: frozen.capabilities,
        requirements: frozen.requirements,
      },
      context.signal,
    );
    assertActive(context);
    context.recorder.stage("plan", "Request a proposal-only ACP task", "completed");
    context.recorder.step("decide", "completed");
    context.recorder.step("apply", "running");

    let proposal;
    try {
      proposal = parseAcpProposal(reply.text, context.document);
    } catch (error) {
      const wire = asProtocolError(error);
      const details = wire.value.details;
      const isStaleSource =
        wire.value.code === "INVALID_REQUEST" &&
        details["field"] === "proposal" &&
        isObject(details["current"]) &&
        !staleRetried;
      if (isStaleSource) {
        staleRetried = true;
        context.recorder.stage("inspect", "Re-inspect after a stale ACP proposal", "running");
        const re = await context.options.cad.inspect({ design: true });
        assertActive(context);
        context.document = cloneStamp(re.stamp);
        context.recorder.stage("inspect", "Re-inspect after a stale ACP proposal", "completed");
        continue;
      }
      return finish(
        "task_failed",
        `The ACP proposal is invalid: ${wire.message}`,
        "INVALID_PROPOSAL",
        { changes: context.changes },
      );
    }

    context.meter.reserveCad(proposal.commands.length);
    assertActive(context);
    const callId = `acp-${context.meter.usage.agentTasks}`;
    context.recorder.tool(callId, "agent.proposal", "running", { commands: proposal.commands.length });
    let write: CadWriteResult;
    try {
      write = await applyProposal(context.options.cad, { commands: proposal.commands });
    } catch (error) {
      context.recorder.tool(callId, "agent.proposal", "failed", protocolOutput(error));
      throw error;
    }
    // Keep a write that committed while cancellation arrived, then let the
    // handler publish its document and changes before any terminal.
    const outcome = await handleWrite(context, callId, "agent.proposal", write);
    if (outcome.kind === "terminal") {
      return outcome.result;
    }
    if (context.appliedAny && context.checksPassed) {
      return validateAndFinish(context, finish);
    }
  }
}

async function handleWrite(
  context: RunContext,
  callId: string,
  toolName: string,
  write: CadWriteResult,
  allowReconcile = true,
): Promise<Outcome> {
  const outcome = write.outcome;
  switch (outcome.state) {
    case "applied": {
      context.appliedAny = true;
      context.changes = context.changes === "unknown" ? "unknown" : "applied";
      adoptAfter(context, write);
      if (outcome.after === null) {
        context.document = cloneStamp(context.options.cad.stamp);
      }
      context.recorder.step("apply", "completed");
      context.recorder.step("check", "running");
      context.recorder.stage("apply", "Apply ordered native commands through the guarded adapter", "completed");
      context.recorder.stage("check", "Check the fixed requirements with exact native evidence", "running");
      const checks = await evaluateFixedChecks(context.options.cad, context.input.requirements);
      context.checks = checks;
      context.checksPassed = requiredChecksPassed(checks);
      if (context.checksPassed) {
        context.recorder.stage("check", "Check the fixed requirements with exact native evidence", "completed");
        context.recorder.step("check", "completed");
        context.recorder.tool(callId, toolName, "completed", writeOutput(write, checks));
        return { kind: "continue", output: writeOutput(write, checks) };
      }
      context.recorder.stage("check", "Check the fixed requirements with exact native evidence", "failed");
      context.recorder.step("check", "failed");
      context.recorder.tool(callId, toolName, "failed", writeOutput(write, checks));
      return checkFailure(context, write);
    }
    case "pending": {
      if (!allowReconcile) {
        context.changes = "unknown";
        return { kind: "terminal", result: buildResult(context, "outcome_unknown", "A native mutation is still pending and could not be reconciled.", "OUTCOME_UNKNOWN", { changes: "unknown" }) };
      }
      context.recorder.tool(callId, toolName, "running", writeOutput(write));
      const reconciled = await context.options.cad.reconcile();
      if (reconciled === null) {
        context.changes = "unknown";
        return {
          kind: "terminal",
          result: buildResult(context, "outcome_unknown", "A native mutation is pending with no reconcilable result.", "OUTCOME_UNKNOWN", {
            changes: "unknown",
          }),
        };
      }
      if (reconciled.outcome.state === "pending") {
        context.changes = "unknown";
        return {
          kind: "terminal",
          result: buildResult(context, "outcome_unknown", "The original native mutation is still running; no replacement was sent.", "OUTCOME_UNKNOWN", {
            changes: "unknown",
          }),
        };
      }
      return await handleWrite(context, callId, toolName, reconciled, false);
    }
    case "command_failed": {
      adoptAfter(context, write);
      context.recorder.tool(callId, toolName, "failed", writeOutput(write));
      if (rollbackRestored(write)) {
        context.recorder.step("apply", "pending");
        const checks = await evaluateFixedChecks(context.options.cad, context.input.requirements);
        context.checks = checks;
        context.checksPassed = requiredChecksPassed(checks);
        if (!context.checksPassed) {
          return checkFailure(context, write);
        }
        return { kind: "continue", output: writeOutput(write, checks) };
      }
      const code = outcome.error?.code ?? "INTERNAL_ERROR";
      if (code === "UNSUPPORTED_CAPABILITY") {
        return {
          kind: "terminal",
          result: buildResult(
            context,
            "task_failed",
            outcome.error?.message ?? "The native command or capability is unsupported.",
            "UNSUPPORTED_CAPABILITY",
          ),
        };
      }
      if (code === "OUTCOME_UNKNOWN") {
        context.changes = "unknown";
        return {
          kind: "terminal",
          result: buildResult(context, "outcome_unknown", outcome.error?.message ?? "The native outcome is unknown.", "OUTCOME_UNKNOWN", {
            changes: "unknown",
          }),
        };
      }
      if (code === "CANCELLED") {
        return {
          kind: "terminal",
          result: buildResult(context, "cancelled", outcome.error?.message ?? "The write was cancelled.", "CANCELLED"),
        };
      }
      if (code === "RESOURCE_EXHAUSTED") {
        return {
          kind: "terminal",
          result: buildResult(context, "budget_exhausted", outcome.error?.message ?? "A CAD resource was exhausted.", "BUDGET_EXHAUSTED"),
        };
      }
      const noWrite =
        outcome.completed === 0 ||
        (outcome.after !== null && documentStampEquals(outcome.after, outcome.before));
      if (!noWrite) {
        context.changes = (outcome.completed ?? 0) > 0 || outcome.rollback.restored === false ? "applied" : "unknown";
      }
      const message = outcome.error?.message ?? "The native command failed without a usable rollback report.";
      return {
        kind: "terminal",
        result: buildResult(context, "task_failed", `The native command failed: ${message}`, "COMMAND_FAILED"),
      };
    }
    case "expectation_failed": {
      // The contract says an expectation mismatch leaves the model built.
      adoptAfter(context, write);
      context.changes = "applied";
      context.recorder.tool(callId, toolName, "failed", writeOutput(write));
      const message = outcome.error?.message ?? "A native expectation check failed after the design was built.";
      return {
        kind: "terminal",
        result: buildResult(context, "task_failed", `Native expectation mismatch: ${message}`, "EXPECTATION_MISMATCH"),
      };
    }
    case "stale": {
      const output: JsonObject = {
        ...writeOutput(write),
        replan: true,
        note: "The design changed before this guarded write. It was not applied or retried; re-inspect and replan against the new stamp.",
        inspected: context.document,
      };
      context.recorder.tool(callId, toolName, "failed", output);
      context.checksPassed = false;
      context.recorder.step("inspect", "running");
      context.recorder.stage("inspect", "Re-inspect after a stale document stamp", "running");
      const re = await context.options.cad.inspect({ design: true });
      assertActive(context);
      context.document = cloneStamp(re.stamp);
      output["inspected"] = context.document;
      context.recorder.stage("inspect", "Re-inspect after a stale document stamp", "completed");
      context.recorder.step("inspect", "completed");
      context.recorder.step("apply", "pending");
      return { kind: "continue", output };
    }
    case "cancelled": {
      adoptAfter(context, write);
      if (context.appliedAny || (outcome.completed ?? 0) > 0) {
        context.changes = "applied";
      } else if (outcome.completed === null && outcome.after === null) {
        context.changes = "unknown";
      }
      return {
        kind: "terminal",
        result: buildResult(context, "cancelled", "The turn was cancelled before the native write completed.", "CANCELLED"),
      };
    }
    case "unknown": {
      context.changes = "unknown";
      return {
        kind: "terminal",
        result: buildResult(
          context,
          "outcome_unknown",
          "The native mutation's effect could not be established; no further modeling is allowed until it is reconciled.",
          "OUTCOME_UNKNOWN",
          { changes: "unknown" },
        ),
      };
    }
  }
}

function checkFailure(context: RunContext, write: CadWriteResult): Outcome {
  const failed = failedCheckSummary(context.checks);
  if (context.meter.tryReserveRepair()) {
    context.recorder.stage("repair", "Repair the failed fixed-requirement check", "failed");
    context.recorder.step("apply", "pending");
    context.recorder.step("check", "pending");
    return { kind: "continue", output: writeOutput(write, context.checks) };
  }
  const ids = failedCheckIds(context.checks);
  return {
    kind: "terminal",
    result: buildResult(
      context,
      "quality_failed",
      `Fixed stage checks failed after ${context.meter.usage.repairs} repair attempt(s): ${ids.join(", ") || failed}.`,
      "QUALITY_FAILED",
    ),
  };
}

async function validateAndFinish(
  context: RunContext,
  finish: ResultFactory,
): Promise<WorkflowResult> {
  if (!context.checksPassed || !requiredChecksPassed(context.checks)) {
    context.recorder.step("check", "failed");
    return finish(
      "quality_failed",
      `Fixed stage checks did not pass: ${failedCheckSummary(context.checks) || "no exact check result"}.`,
      "QUALITY_FAILED",
    );
  }
  const gate = context.options.validation ?? null;
  if (gate === null) {
    return finish(
      "task_failed",
      "Native stage checks passed, but snapshot validation is not installed in this runtime revision (#13); no verified success is claimed.",
      "VALIDATION_UNAVAILABLE",
    );
  }
  context.meter.reserveValidation();
  assertActive(context);
  const source = cloneStamp(context.document);
  context.recorder.step("validate", "running");
  context.recorder.stage("validate", "Hand the native snapshot to host validation", "running");
  const outcome = await gate.submit(
    {
      sessionId: context.input.sessionId,
      turnId: context.input.turnId,
      workflowRunId: context.input.workflowRunId,
      document: source,
      profile: context.input.profile,
      requirements: context.input.requirements,
      checks: context.checks,
    },
    context.signal,
  );
  assertActive(context);

  if (outcome.state === "passed") {
    const current = await context.options.cad.inspect({ design: false });
    assertActive(context);
    context.document = cloneStamp(current.stamp);
    if (!documentStampEquals(source, current.stamp)) {
      context.recorder.stage("validate", "Hand the native snapshot to host validation", "failed");
      context.recorder.step("validate", "failed");
      return finish(
        "quality_failed",
        "The live design changed after the validated snapshot; that report is historical evidence, not approval of current content.",
        "STALE_DOCUMENT",
        { validationRunId: outcome.runId, report: outcome.report },
      );
    }
    context.recorder.stage("validate", "Hand the native snapshot to host validation", "completed");
    context.recorder.step("validate", "completed");
    return finish("validated_success", outcome.reason || "Required checks passed for the current native snapshot.", "VALIDATED_SUCCESS", {
      validationRunId: outcome.runId,
      report: outcome.report,
    });
  }
  context.recorder.stage("validate", "Hand the native snapshot to host validation", "failed");
  context.recorder.step("validate", "failed");
  switch (outcome.state) {
    case "quality_failed":
      return finish("quality_failed", outcome.reason, "QUALITY_FAILED", {
        validationRunId: outcome.runId,
        report: outcome.report,
      });
    case "cancelled":
      return finish("cancelled", outcome.reason || "Host validation was cancelled.", "CANCELLED", {
        validationRunId: outcome.runId,
        report: outcome.report,
      });
    case "unknown":
      return finish("outcome_unknown", outcome.reason || "The host validation outcome is unknown.", "OUTCOME_UNKNOWN", {
        changes: "unknown",
        validationRunId: outcome.runId,
        report: outcome.report,
      });
    case "infrastructure_error":
      return finish("task_failed", `Validation infrastructure error: ${outcome.reason}`, "VALIDATION_INFRASTRUCTURE_ERROR", {
        validationRunId: outcome.runId,
        report: outcome.report,
      });
  }
}

type ResultFactory = (
  state: WorkflowTerminalState,
  reason: string,
  reasonCode: string,
  extra?: Partial<WorkflowResult>,
) => WorkflowResult;

function buildResult(
  context: RunContext,
  state: WorkflowTerminalState,
  reason: string,
  reasonCode: string,
  extra: Partial<WorkflowResult> = {},
): WorkflowResult {
  return {
    state,
    reason,
    reasonCode,
    document: cloneStamp(context.document),
    changes: extra.changes ?? context.changes,
    answer: extra.answer ?? context.answer,
    validationRunId: extra.validationRunId ?? null,
    report: extra.report ?? null,
    checks: extra.checks ?? [...context.checks],
    stages: extra.stages ?? context.recorder.stages,
    plan: extra.plan ?? context.recorder.planSteps,
    tools: extra.tools ?? context.recorder.tools,
    budgets: extra.budgets ?? context.meter.usage,
  };
}

function finishFromError(context: RunContext, error: unknown): WorkflowResult {
  const wire = error instanceof ProtocolError ? error : asProtocolError(error);
  const code = wire.value.code;
  if (context.signal.aborted || code === "CANCELLED") {
    return buildResult(context, "cancelled", "The turn was cancelled.", "CANCELLED");
  }
  if (code === "RESOURCE_EXHAUSTED") {
    const resource = typeof wire.value.details["resource"] === "string" ? wire.value.details["resource"] : "turn";
    const limit = wire.value.details["limit"];
    return buildResult(
      context,
      "budget_exhausted",
      `The ${resource} budget was exhausted (limit ${String(limit)}); no further work was started.`,
      "BUDGET_EXHAUSTED",
    );
  }
  if (code === "AUTH_REQUIRED") {
    return buildResult(context, "auth_required", wire.message, "AUTH_REQUIRED");
  }
  if (code === "TRANSPORT_ERROR") {
    return buildResult(context, "transport_error", wire.message, "TRANSPORT_ERROR");
  }
  if (code === "OUTCOME_UNKNOWN") {
    return buildResult(context, "outcome_unknown", wire.message, "OUTCOME_UNKNOWN", { changes: "unknown" });
  }
  if (code === "UNSUPPORTED_CAPABILITY") {
    return buildResult(context, "task_failed", wire.message, "UNSUPPORTED_CAPABILITY");
  }
  return buildResult(context, "task_failed", wire.message, code);
}

function assertActive(context: RunContext): void {
  if (context.signal.aborted) {
    throw cancelled("workflow");
  }
  context.meter.assertTime();
}

function appendToolFailure(
  context: RunContext,
  messages: ModelMessage[],
  callId: string,
  name: string,
  error: unknown,
): void {
  const output = protocolOutput(error);
  context.recorder.tool(callId, name, "failed", output);
  messages.push(toolMessage(callId, output, context.limits.textBytes));
}

function protocolOutput(error: unknown): JsonObject {
  const wire = error instanceof ProtocolError ? error : asProtocolError(error);
  return errorOutput({ code: wire.value.code, message: wire.value.message, details: wire.value.details });
}

function acpTask(context: RunContext, frozen: FrozenInspection): string {
  return [
    context.input.parts.join("\n"),
    "",
    "Return exactly one JSON object with this shape and no prose:",
    '{"source":{"documentId":"...","revision":"..."},"commands":[{"command":"...","params":{}}]}',
    `The source stamp must be the frozen design ${JSON.stringify(context.document)}.`,
    "The outer workflow validates this proposal, executes it through guarded native commands and keeps every command editable.",
    `Fixed requirements: ${JSON.stringify(frozen.requirements)}`,
  ].join("\n");
}

function cloneStamp(stamp: DocumentStamp): DocumentStamp {
  return { documentId: stamp.documentId, revision: stamp.revision };
}

function adoptAfter(context: RunContext, write: CadWriteResult): void {
  const after = write.outcome.after;
  if (after !== null) {
    context.document = cloneStamp(after);
  }
}

export { validateRequirements };
