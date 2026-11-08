/**
 * Deterministic workflow harness: a real McpClient + CadAdapter over the
 * in-memory MCP peer, plus scripted HostAi model steps, events, requirements
 * and a host validation handoff. No test reaches an app, network or model.
 */

import type { TestContext } from "node:test";

import { ProtocolError } from "../../src/errors.js";
import type {
  ModelGenerateRequest,
  ModelGenerateResult,
  ToolCall,
  ToolDefinition,
} from "../../src/host-ai/model-client.js";
import { DEFAULT_LIMITS, type Budgets, type Limits } from "../../src/limits.js";
import type { JsonObject } from "../../src/protocol.js";
import { CadAdapter, type DocumentStamp } from "../../src/solidcraft/cad-adapter.js";
import { McpClient } from "../../src/solidcraft/mcp-client.js";
import {
  runNativeModelingWorkflow,
  type CandidateRef,
  type CheckRequirement,
  type FixedRequirements,
  type ModelStep,
  type NativeWorkflowInput,
  type PlanStep,
  type ProfileRef,
  type ProposalSource,
  type StageEvent,
  type TextChannel,
  type ToolEvent,
  type ValidationPort,
  type ValidationReport,
  type ValidationReportResult,
  type ValidationStatusResult,
  type ValidationSubmitResult,
  type WorkflowEvents,
  type WorkflowOptions,
  type WorkflowResult,
} from "../../src/workflows/index.js";
import {
  FakeMcpPeer,
  defaultCommands,
  defaultTools,
  guardedReply,
  textReply,
  type ToolReply,
} from "./mcp-peer.js";

export interface NativeParameter {
  expression: string;
  value: number;
}

export interface NativeBody {
  name: string;
  volume_mm3: number;
  bbox: { min: number[]; max: number[] };
  faces: number;
}

export interface NativeFeature {
  id: string;
  name: string;
  type: string;
  error: string | null;
}

export interface NativeState {
  parameters: Map<string, NativeParameter>;
  bodies: NativeBody[];
  timeline: NativeFeature[];
}

export function boxState(): NativeState {
  return {
    parameters: new Map([
      ["width", { expression: "20", value: 20 }],
      ["depth", { expression: "10", value: 10 }],
      ["height", { expression: "5", value: 5 }],
    ]),
    bodies: [],
    timeline: [],
  };
}

export function parameterCheck(
  name: string,
  value: number,
  absTolerance = 1e-9,
  checkId = "parameters.required",
): CheckRequirement {
  return {
    checkId,
    kind: "parameters.required",
    required: true,
    parameters: { requirements: [{ name, value, absTolerance }] },
  };
}

export function parameterRequirements(
  entries: readonly { name: string; value: number; absTolerance?: number }[],
  checkId = "parameters.required",
): CheckRequirement {
  return {
    checkId,
    kind: "parameters.required",
    required: true,
    parameters: {
      requirements: entries.map((entry) => ({
        name: entry.name,
        value: entry.value,
        absTolerance: entry.absTolerance ?? 1e-9,
      })),
    },
  };
}

export function volumeCheck(expected: number, checkId = "geometry.exact"): CheckRequirement {
  return {
    checkId,
    kind: "geometry.exact",
    required: true,
    parameters: {
      measurements: [
        {
          command: "measure",
          params: {},
          resultPointer: "/total/volume_mm3",
          expected,
          absTolerance: 1e-9,
          relTolerance: 1e-9,
        },
      ],
    },
  };
}

export function fixedRequirements(...checks: CheckRequirement[]): FixedRequirements {
  return { checks };
}

export const PROFILE: ProfileRef = {
  profileId: "native-workflow-fixture",
  version: "1.0.0",
  digest: `sha256:${"b".repeat(64)}`,
};

export const DEFAULT_BUDGETS: Budgets = {
  modelCalls: 20,
  agentTasks: 4,
  cadCommands: 64,
  repairs: 2,
  validationRuns: 3,
  wallTimeSeconds: 1_200,
};

export function toolCall(id: string, name: string, args: JsonObject): ToolCall {
  return { id, name, arguments: args };
}

export function replyToolCalls(calls: readonly ToolCall[], text = ""): ModelGenerateResult {
  return {
    content: text.length === 0 ? [] : [{ type: "text", text }],
    toolCalls: [...calls],
    finishReason: "tool_calls",
    usage: null,
    continuation: null,
    delivery: "buffered",
  };
}

export function replyText(text: string): ModelGenerateResult {
  return {
    content: [{ type: "text", text }],
    toolCalls: [],
    finishReason: "stop",
    usage: null,
    continuation: null,
    delivery: "buffered",
  };
}

export function decisionEdit(reason = "The requested edit is unambiguous."): ToolCall {
  return toolCall("call-decision", "record_decision", { decision: "edit", reason });
}

export function decisionAsk(reason = "Which dimension is protected?"): ToolCall {
  return toolCall("call-decision", "record_decision", { decision: "ask", reason });
}

export function decisionDecline(reason = "The task is out of scope."): ToolCall {
  return toolCall("call-decision", "record_decision", { decision: "decline", reason });
}

export function applyCall(id: string, commands: readonly { command: string; params?: JsonObject }[]): ToolCall {
  return toolCall(id, "apply_native_commands", { commands });
}

export class ScriptedModel implements ModelStep {
  readonly requests: ModelGenerateRequest[] = [];
  readonly #replies: ModelGenerateResult[];
  readonly #onReply: ((index: number, reply: ModelGenerateResult) => void) | undefined;
  #index = 0;

  constructor(
    replies: readonly ModelGenerateResult[],
    onReply?: (index: number, reply: ModelGenerateResult) => void,
  ) {
    this.#replies = [...replies];
    this.#onReply = onReply;
  }

  async generate(request: ModelGenerateRequest, signal: AbortSignal): Promise<ModelGenerateResult> {
    if (signal.aborted) {
      throw new Error("scripted model received an aborted signal");
    }
    // Snapshot the request: the workflow mutates its conversation array after
    // each round, while a real HostAi call serializes one exact message list.
    this.requests.push({
      ...request,
      messages: request.messages.map((message) => ({
        ...message,
        content: message.content.map((block) => ({ ...block })),
        toolCalls: message.toolCalls.map((call) => ({ ...call, arguments: { ...call.arguments } })),
      })),
      tools: request.tools.map((definition) => ({ ...definition })),
    });
    const reply = this.#replies[this.#index];
    const index = this.#index;
    this.#index += 1;
    if (reply === undefined) {
      throw new Error(`the scripted model has no reply for model call ${this.#index}`);
    }
    this.#onReply?.(index, reply);
    return reply;
  }
}

export interface ValidationSubmission {
  readonly candidate: CandidateRef;
  readonly profile: ProfileRef;
  readonly requestId: string;
}

type FixedState = Exclude<ValidationSubmitResult["state"], "queued" | "running">;

/**
 * Deterministic host validation state machine for workflow tests. It exercises
 * the exact `validation.submit/status/report` order; report bytes are built
 * from the submitted candidate and profile, never from model text.
 */
export class FixedValidation implements ValidationPort {
  readonly submissions: ValidationSubmission[] = [];
  readonly #state: FixedState | ((submission: ValidationSubmission, index: number) => FixedState);
  readonly #runs = new Map<string, { submission: ValidationSubmission; statusCalls: number }>();
  #next = 0;

  readonly onStatus: (() => void) | undefined;
  readonly tamper: ((report: ValidationReport) => ValidationReport) | undefined;

  constructor(
    state: FixedState | ((submission: ValidationSubmission, index: number) => FixedState) = "passed",
    onStatus?: () => void,
    tamper?: (report: ValidationReport) => ValidationReport,
  ) {
    this.#state = state;
    this.onStatus = onStatus;
    this.tamper = tamper;
  }

  get requests(): readonly ValidationSubmission[] {
    return this.submissions;
  }

  async submit(candidate: CandidateRef, profile: ProfileRef, requestId: string): Promise<ValidationSubmitResult> {
    const submission = { candidate, profile, requestId };
    this.submissions.push(submission);
    this.#next += 1;
    const runId = `v-${this.#next}`;
    this.#runs.set(runId, { submission, statusCalls: 0 });
    return { runId, state: "queued", candidate, profile };
  }

  async status(runId: string): Promise<ValidationStatusResult> {
    const run = this.#runs.get(runId);
    if (run === undefined) {
      throw new ProtocolError("NOT_FOUND", "No such test run.", "never");
    }
    run.statusCalls += 1;
    this.onStatus?.();
    const index = this.submissions.indexOf(run.submission);
    const state = run.statusCalls === 1 ? "running" : typeof this.#state === "function" ? this.#state(run.submission, index) : this.#state;
    return {
      runId,
      state,
      candidate: run.submission.candidate,
      profile: run.submission.profile,
      updatedAt: "2026-11-07T12:00:00.000Z",
      reportAvailable: state !== "running",
    };
  }

  async report(runId: string): Promise<ValidationReportResult> {
    const run = this.#runs.get(runId);
    if (run === undefined) {
      throw new ProtocolError("NOT_FOUND", "No such test run.", "never");
    }
    const index = this.submissions.indexOf(run.submission);
    const state = typeof this.#state === "function" ? this.#state(run.submission, index) : this.#state;
    const checkState = state === "passed" ? "passed" : state === "quality_failed" ? "failed" : "not_run";
    const report: ValidationReport = {
      schemaVersion: "1.0",
      runId,
      candidateId: run.submission.candidate.candidateId,
      candidateDigest: run.submission.candidate.candidateDigest,
      manifestDigest: run.submission.candidate.manifestDigest,
      profile: run.submission.profile,
      runner: { runnerId: "local-solidcraft", version: "1.0.0", engineVersion: "0.0.0-test" },
      state,
      startedAt: "2026-11-07T12:00:00.000Z",
      finishedAt: "2026-11-07T12:00:01.000Z",
      checks: [
        {
          checkId: "input.integrity",
          required: true,
          state: checkState,
          expected: {},
          actual: {},
          evidence: [{ artifactId: `evidence-${runId}`, digest: `sha256:${"a".repeat(64)}`, sizeBytes: 1, mediaType: "application/json" }],
        },
      ],
      error: state === "passed" || state === "quality_failed" ? null : { code: "CANCELLED", message: "The test run is terminal." },
    };
    return {
      report: this.tamper?.(report) ?? report,
      reference: { artifactId: `report-${runId}`, digest: `sha256:${"b".repeat(64)}`, sizeBytes: 1, mediaType: "application/json" },
    };
  }

  async cancel(runId: string): Promise<{ runId: string; state: FixedState }> {
    return { runId, state: "cancelled" };
  }
}

export function passedValidation(_runId?: string): FixedValidation {
  return new FixedValidation("passed");
}

export class ScriptedProposalSource implements ProposalSource {
  readonly requests: string[] = [];
  readonly #texts: string[];

  constructor(texts: readonly string[]) {
    this.#texts = [...texts];
  }

  async requestProposal(request: { readonly task: string }, signal: AbortSignal): Promise<{ readonly text: string }> {
    if (signal.aborted) {
      throw new Error("scripted proposal source received an aborted signal");
    }
    this.requests.push(request.task);
    const text = this.#texts.shift();
    if (text === undefined) {
      throw new Error("the scripted proposal source has no reply");
    }
    return { text };
  }
}

export interface RecordedEvents {
  readonly events: WorkflowEvents;
  readonly stages: StageEvent[];
  readonly plans: PlanStep[][];
  readonly texts: { text: string; channel: TextChannel }[];
  readonly tools: ToolEvent[];
}

export function recordEvents(): RecordedEvents {
  const stages: StageEvent[] = [];
  const plans: PlanStep[][] = [];
  const texts: { text: string; channel: TextChannel }[] = [];
  const tools: ToolEvent[] = [];
  return {
    stages,
    plans,
    texts,
    tools,
    events: {
      stage: (stageId, title, state) => stages.push({ stageId, title, state }),
      plan: (steps) => plans.push(steps.map((step) => ({ ...step }))),
      text: (text, channel) => texts.push({ text, channel }),
      tool: (callId, name, state, output) => tools.push({ callId, name, state, output }),
    },
  };
}

export class WorkflowHarness {
  readonly peer: FakeMcpPeer;
  readonly client: McpClient;
  readonly adapter: CadAdapter;
  state: NativeState;
  readonly events = recordEvents();
  readonly stamp: DocumentStamp = { documentId: "doc-A", revision: "41" };
  readonly writes: { name: string; args: JsonObject }[] = [];
  readonly stamps: DocumentStamp[] = [];
  model: ScriptedModel | null = null;
  #connected = false;
  #history: NativeState[] = [];

  // Configurable failure injections, all disabled by default.
  staleOnce = false;
  unknownOnce = false;
  pendingOnce = false;
  expectationFailedOnce = false;
  partialFailureOnce = false;
  duringWrite: (() => void) | null = null;
  #nextCandidate = 0;

  constructor(t: TestContext, state: NativeState = boxState()) {
    this.state = state;
    this.peer = new FakeMcpPeer({ tools: defaultTools(true), commands: defaultCommands(), documentId: "doc-A", revision: 41n });
    this.peer.commands.set("solid.fail", {
      id: "solid.fail",
      input: { type: "object", additionalProperties: false, properties: {} },
    });
    this.peer.setHandler("inspect_design", () => textReply(this.designView()));
    this.peer.setHandler("measure", () => textReply(this.measureView()));
    this.peer.execute((args) => this.onExecute(args));
    this.peer.batch((args) => this.onBatch(args));
    this.client = new McpClient({
      channel: this.peer,
      bridge: { name: "solidcraft-app", bridgeId: "bridge-1" },
      turn: { sessionId: "s-1", turnId: "t-1", workflowRunId: "w-1" },
      mcpSessionId: "mcp-session-fixture",
      requestTimeoutMs: 1_000,
      handshakeTimeoutMs: 1_000,
    });
    this.adapter = new CadAdapter({
      client: this.client,
      turn: {
        bridgeId: "bridge-1",
        sessionId: "s-1",
        turnId: "t-1",
        workflowRunId: "w-1",
        document: this.stamp,
      },
      writeTimeoutMs: 1_000,
      readTimeoutMs: 1_000,
      pendingTimeoutMs: 1_000,
    });
    t.after(() => this.adapter.dispose());
  }

  get writeCount(): number {
    return this.writes.length;
  }

  designView(): JsonObject {
    return {
      name: "analytic-fixture",
      document: this.peer.stamp(),
      params: [...this.state.parameters.entries()].map(([name, parameter]) => ({
        name,
        expression: parameter.expression,
        unit: "mm",
        value: parameter.value,
        error: null,
      })),
      timeline: this.state.timeline.map((feature) => ({
        ...feature,
        suppressed: false,
        rolled_back: false,
        warning: null,
      })),
      bodies: this.state.bodies.map((body) => ({
        ...body,
        area_mm2: 0,
        closed: true,
      })),
      sketches: [],
      selection: [],
      undo_depth: 0,
    };
  }

  measureView(): JsonObject {
    const volume = this.state.bodies.reduce((total, body) => total + body.volume_mm3, 0);
    return {
      body_count: this.state.bodies.length,
      bodies: this.state.bodies.map((body) => ({ name: body.name, volume_mm3: body.volume_mm3 })),
      total: { volume_mm3: volume },
    };
  }

  onExecute(args: JsonObject): ToolReply {
    const command = typeof args["command"] === "string" ? args["command"] : "";
    if (command === "document.revision") {
      return textReply(`{\n  "document_id": "${this.peer.documentId}",\n  "revision": ${this.peer.revision.toString()}\n}`);
    }
    if (command === "document.snapshot.capture") {
      this.#nextCandidate += 1;
      const serial = this.#nextCandidate.toString(16).padStart(64, "0");
      return {
        body: {
          ok: true,
          applied: [0],
          completed: 1,
          candidate: {
            candidateId: `cand-${this.#nextCandidate}`,
            source: this.peer.stamp(),
            candidateDigest: `sha256:${serial}`,
            manifestDigest: `sha256:${serial.split("").reverse().join("")}`,
            artifact: {
              artifactId: `artifact-cand-${this.#nextCandidate}`,
              digest: `sha256:${"c".repeat(64)}`,
              sizeBytes: 1024,
              mediaType: "application/zip",
            },
          },
          documentStamp: this.peer.stamp(),
        },
      };
    }
    if (command === "edit.undo") {
      this.writes.push({ name: "execute", args });
      this.stamps.push(this.expectedStamp(args));
      const undone = this.applyCommand(command, args["params"]);
      if (!undone.ok) {
        return textReply(undone.message, true);
      }
      this.peer.revision += 1n;
      return guardedReply({ ...undone.result, documentStamp: this.peer.stamp() });
    }
    this.#history.push(this.snapshot());
    this.writes.push({ name: "execute", args });
    this.stamps.push(this.expectedStamp(args));
    this.duringWrite?.();
    const applied = this.applyCommand(command, args["params"]);
    if (!applied.ok) {
      return textReply(applied.message, true);
    }
    this.peer.revision += 1n;
    return guardedReply({ ...applied.result, documentStamp: this.peer.stamp() });
  }

  onBatch(args: JsonObject): ToolReply {
    this.writes.push({ name: "batch", args });
    this.stamps.push(this.expectedStamp(args));
    this.duringWrite?.();
    if (this.staleOnce) {
      this.staleOnce = false;
      // Simulate an intervening user edit after the workflow inspected.
      this.peer.revision += 1n;
      return guardedReply(
        {
          error: {
            code: "STALE_DOCUMENT",
            message: "The design changed before the guarded batch.",
            retry: "never",
            details: { expected: args["expectedDocument"], current: this.peer.stamp() },
          },
        },
        true,
      );
    }
    if (this.unknownOnce) {
      this.unknownOnce = false;
      return guardedReply(
        {
          outcomeUnknown: true,
          error: { code: "OUTCOME_UNKNOWN", message: "The batch effect cannot be established.", retry: "never", details: {} },
        },
        true,
      );
    }
    if (this.pendingOnce) {
      this.pendingOnce = false;
      const commands = Array.isArray(args["commands"]) ? args["commands"] : [];
      const applied: number[] = [];
      for (const entry of commands) {
        const step = entry as JsonObject;
        const command = typeof step["command"] === "string" ? step["command"] : "";
        const params = step["params"] === undefined ? {} : (step["params"] as JsonObject);
        const result = this.applyCommand(command, params);
        if (!result.ok) {
          return guardedReply({ ok: false, failed: { error: result.message }, documentStamp: this.peer.stamp() });
        }
        applied.push(applied.length);
      }
      this.peer.revision += 1n;
      const original = {
        ok: true,
        completed: commands.length,
        applied,
        results: [],
        documentStamp: this.peer.stamp(),
      };
      this.peer.pending(() => textReply({ pending: null, done: true, result: original }));
      return textReply("still running: the guarded batch is being applied", false);
    }
    if (this.partialFailureOnce) {
      this.partialFailureOnce = false;
      const commands = Array.isArray(args["commands"]) ? (args["commands"] as JsonObject[]) : [];
      const applied: number[] = [];
      for (const [index, entry] of commands.entries()) {
        const step = entry as JsonObject;
        const command = typeof step["command"] === "string" ? step["command"] : "";
        const params = step["params"] === undefined ? {} : (step["params"] as JsonObject);
        const result = this.applyCommand(command, params);
        if (!result.ok) {
          this.peer.revision += 1n;
          return guardedReply(
            {
              ok: false,
              completed: index,
              applied,
              not_run: commands.slice(index + 1).map((_, offset) => index + 1 + offset),
              failed: { index, error: result.message },
              rolled_back: false,
              documentStamp: this.peer.stamp(),
            },
            true,
          );
        }
        applied.push(index);
      }
      this.peer.revision += 1n;
      return guardedReply({ ok: true, completed: commands.length, applied, results: [], documentStamp: this.peer.stamp() });
    }
    if (this.expectationFailedOnce) {
      this.expectationFailedOnce = false;
      this.peer.revision += 1n;
      return guardedReply({
        ok: true,
        completed: 1,
        applied: [0],
        expectation: {
          ok: false,
          measured: { volume_mm3: 0 },
          expected: { volume_mm3: 1 },
          mismatch: ["volume_mm3"],
        },
        documentStamp: this.peer.stamp(),
      });
    }
    const commands = args["commands"];
    if (!Array.isArray(commands)) {
      return guardedReply({ error: { code: "INVALID_REQUEST", message: "no commands", retry: "never", details: {} } }, true);
    }
    const containsUndo = commands.some((entry) => (entry as JsonObject)["command"] === "edit.undo");
    if (!containsUndo) {
      this.#history.push(this.snapshot());
    }
    const applied: number[] = [];
    for (const [index, entry] of commands.entries()) {
      const step = entry as JsonObject;
      const command = typeof step["command"] === "string" ? step["command"] : "";
      const params = step["params"] === undefined ? {} : (step["params"] as JsonObject);
      const result = this.applyCommand(command, params);
      if (!result.ok) {
        const previous = this.#history.pop();
        if (previous !== undefined) {
          this.state = previous;
        }
        return guardedReply(
          {
            ok: false,
            completed: index,
            applied,
            not_run: commands.slice(index + 1).map((_, offset) => index + 1 + offset),
            failed: { index, error: result.message },
            rolled_back: true,
            documentStamp: this.peer.stamp(),
          },
          true,
        );
      }
      applied.push(index);
    }
    this.peer.revision += 1n;
    return guardedReply({
      ok: true,
      completed: commands.length,
      applied,
      results: [],
      documentStamp: this.peer.stamp(),
    });
  }

  applyCommand(command: string, rawParams: unknown): { ok: true; result: JsonObject } | { ok: false; message: string } {
    const params = rawParams !== undefined && typeof rawParams === "object" && rawParams !== null ? (rawParams as JsonObject) : {};
    if (command === "solid.box") {
      const length = numberParam(params, "length");
      const width = numberParam(params, "width");
      const height = numberParam(params, "height");
      if (length === null || width === null || height === null) {
        return { ok: false, message: "solid.box needs length, width and height." };
      }
      const name = `Body${this.state.bodies.length + 1}`;
      this.state.bodies.push({
        name,
        volume_mm3: length * width * height,
        bbox: { min: [0, 0, 0], max: [length, width, height] },
        faces: 6,
      });
      this.state.timeline.push({
        id: `f${this.state.timeline.length + 1}`,
        name: "Box",
        type: "box",
        error: null,
      });
      return { ok: true, result: { body: name, volume_mm3: length * width * height } };
    }
    if (command === "parameters.change") {
      const name = typeof params["name"] === "string" ? params["name"] : "";
      if (name.length === 0) {
        return { ok: false, message: "parameters.change needs a name." };
      }
      const expression = params["expression"];
      const value = typeof expression === "number" ? expression : parseExpression(expression);
      if (value === null) {
        return { ok: false, message: "parameters.change needs a numeric expression." };
      }
      this.state.parameters.set(name, { expression: String(expression), value });
      return { ok: true, result: { name, value } };
    }
    if (command === "edit.undo") {
      const previous = this.#history.pop();
      if (previous === undefined) {
        return { ok: false, message: "nothing to undo" };
      }
      this.state = previous;
      return { ok: true, result: { undone: true } };
    }
    if (command === "solid.fail") {
      return { ok: false, message: "the fixture command failed" };
    }
    return { ok: false, message: `unknown fixture command \`${command}\`` };
  }

  snapshot(): NativeState {
    return {
      parameters: new Map(
        [...this.state.parameters.entries()].map(([name, parameter]) => [name, { ...parameter }]),
      ),
      bodies: this.state.bodies.map((body) => ({ ...body, bbox: { min: [...body.bbox.min], max: [...body.bbox.max] } })),
      timeline: this.state.timeline.map((feature) => ({ ...feature })),
    };
  }

  expectedStamp(args: JsonObject): DocumentStamp {
    const raw = args["expectedDocument"];
    if (typeof raw === "object" && raw !== null) {
      const value = raw as JsonObject;
      if (typeof value["documentId"] === "string" && typeof value["revision"] === "string") {
        return { documentId: value["documentId"], revision: value["revision"] };
      }
    }
    throw new Error("the workflow did not send expectedDocument");
  }

  async run(
    overrides: Partial<NativeWorkflowInput> & {
      readonly modelReplies?: readonly ModelGenerateResult[];
      readonly validation?: ValidationPort | null;
      readonly proposalSource?: ProposalSource | null;
      readonly signal?: AbortSignal;
      readonly limits?: Limits;
      readonly onModelReply?: (index: number, reply: ModelGenerateResult) => void;
    } = {},
  ): Promise<WorkflowResult> {
    const {
      modelReplies = [],
      validation = passedValidation(),
      proposalSource = null,
      signal = new AbortController().signal,
      limits = DEFAULT_LIMITS,
      onModelReply,
      ...inputOverrides
    } = overrides;
    const model = new ScriptedModel(modelReplies, onModelReply);
    this.model = model;
    if (!this.#connected) {
      await this.client.connect();
      this.#connected = true;
    }
    const options: WorkflowOptions = {
      model,
      cad: this.adapter,
      events: this.events.events,
      signal,
      validation,
      proposalSource,
      limits,
    };
    return runNativeModelingWorkflow(input(inputOverrides), options);
  }
}

export function input(overrides: Partial<NativeWorkflowInput> = {}): NativeWorkflowInput {
  return {
    sessionId: "s-1",
    turnId: "t-1",
    workflowRunId: "w-1",
    connectionId: "byok-1",
    modelId: "model-a",
    instructions: "Use native features and fixed checks.",
    parts: ["Make the requested native model change."],
    selection: [],
    requirements: fixedRequirements(volumeCheck(1_000)),
    profile: PROFILE,
    document: { documentId: "doc-A", revision: "41" },
    budgets: DEFAULT_BUDGETS,
    mode: "byok",
    ...overrides,
  };
}

function numberParam(params: JsonObject, key: string): number | null {
  const value = params[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function parseExpression(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof value !== "string") {
    return null;
  }
  const match = /^\s*(-?[0-9]+(?:\.[0-9]+)?)/u.exec(value);
  return match === null ? null : Number(match[1]);
}

export type { ToolDefinition };
