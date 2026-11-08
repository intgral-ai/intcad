/**
 * Protocol 1.0 stdio runtime server.
 *
 * Responsibilities: bootstrap negotiation, one session, one active turn,
 * event sequencing, HostAi call correlation, cancellation, reset and orderly
 * shutdown. The runtime owns normalized conversation state in memory only; a
 * replacement process has no session, history or continuation to resume.
 */

import type { Readable, Writable } from "node:stream";
import { randomUUID } from "node:crypto";

import type { Agent } from "@mastra/core/agent";
import { createStep, createWorkflow } from "@mastra/core/workflows";
import { z } from "zod/v4";

import { monotonicClock, type Clock } from "./clock.js";
import {
  asProtocolError,
  cancelled,
  invalidRequest,
  methodNotFound,
  ProtocolError,
  resourceExhausted,
  timeout,
  unsupportedCapability,
  type ErrorCode,
  type RetryAdvice,
} from "./errors.js";
import { DEFAULT_LIMITS, isObject, limitsValid, narrowerLimits, type Budgets, type Limits } from "./limits.js";
import { SessionHistory } from "./history.js";
import { createHostAiAgent, ContinuationBranch, toMastraMessages } from "./host-ai/language-model.js";
import {
  HostAiModelClient,
  type HostAiTransport,
  type ModelMessage,
  type ModelGenerateResult,
} from "./host-ai/model-client.js";
import { AGENT_RUN_CAPABILITY } from "./host-ai/agent-client.js";
import {
  decodeFrameBytes,
  isId,
  scopesEqual,
  turnScope,
  runtimeScope,
  validateDataValue,
  type EventFrame,
  type Frame,
  type JsonObject,
  type RequestFrame,
  type ResponseFrame,
  type Scope,
} from "./protocol.js";
import { NdjsonTransport } from "./transport.js";

export const RUNTIME_VERSION = "0.1.0";
export const PROTOCOL_VERSION = "1.0";
export const WORKFLOW_IDS = Object.freeze(["native-modeling"]);

const RESULT_DEFINITIONS: Readonly<Record<string, string>> = Object.freeze({
  "runtime.hello": "RuntimeHelloResult",
  "session.start": "SessionStartResult",
  "turn.start": "TurnStartResult",
  "turn.cancel": "TurnCancelResult",
  "request.cancel": "RequestCancelResult",
  "session.reset": "SessionResetResult",
  "runtime.shutdown": "RuntimeShutdownResult",
  "connection.describe": "ConnectionDescribeResult",
  "model.generate": "ModelGenerateResult",
  "agent.run": "AgentRunResult",
  "validation.submit": "ValidationSubmitResult",
  "validation.status": "ValidationStatusResult",
  "validation.report": "ValidationReportResult",
  "validation.cancel": "ValidationCancelResult",
});

class TurnCancelled extends Error {
  constructor() {
    super("turn cancelled");
    this.name = "TurnCancelled";
  }
}

class BudgetExhausted extends Error {
  constructor() {
    super("turn budget exhausted");
    this.name = "BudgetExhausted";
  }
}

interface PendingCall {
  readonly id: string;
  readonly method: string;
  readonly scope: Scope;
  readonly control: boolean;
  readonly resolve: (value: JsonObject) => void;
  readonly reject: (error: ProtocolError) => void;
  readonly timer: NodeJS.Timeout;
  readonly removeAbort: () => void;
}

interface SessionState {
  readonly id: string;
  readonly workflowId: string;
  readonly instructions: string;
  readonly bridge: JsonObject | null;
  readonly history: SessionHistory;
  readonly branch: ContinuationBranch;
}

type TerminalState =
  | "validated_success"
  | "quality_failed"
  | "task_failed"
  | "auth_required"
  | "budget_exhausted"
  | "cancelled"
  | "transport_error"
  | "outcome_unknown";

interface TurnOutcome {
  readonly type: "text" | "tool-calls";
  readonly text: string;
  readonly toolCalls: readonly JsonObject[];
  readonly finishReason: string;
}

interface TurnState {
  readonly sessionId: string;
  readonly turnId: string;
  readonly workflowRunId: string;
  readonly requestId: string;
  readonly binding: JsonObject;
  readonly document: { readonly documentId: string; readonly revision: string };
  readonly budgets: Budgets;
  readonly abort: AbortController;
  readonly startedAt: number;
  readonly deadline: number;
  readonly userMessage: ModelMessage;
  cancelled: boolean;
  retired: boolean;
  finished: boolean;
  modelCalls: number;
  outcome: TurnOutcome | null;
}

export interface RuntimeServerOptions {
  readonly input: Readable;
  readonly output: Writable;
  readonly bundle: import("./protocol.js").SchemaBundle;
  readonly diagnostic?: (line: string) => void;
  readonly limits?: Limits;
  readonly instanceId?: string;
  readonly runtimeVersion?: string;
  readonly onStopped?: (code: number) => void;
  readonly clock?: Clock;
}

export class RuntimeServer {
  readonly #transport: NdjsonTransport;
  readonly #diagnostic: (line: string) => void;
  readonly #defaultLimits: Limits;
  readonly #instanceId: string;
  readonly #runtimeVersion: string;
  readonly #onStopped: ((code: number) => void) | undefined;
  readonly #clock: Clock;
  readonly #pending = new Map<string, PendingCall>();
  readonly #tombstones = new Set<string>();
  readonly #seenIncoming = new Set<string>();
  readonly #incomingOrder: string[] = [];
  readonly #bundle: import("./protocol.js").SchemaBundle;
  #limits: Limits;
  #phase: "starting" | "ready" | "stopping" | "stopped" | "transport_error" = "starting";
  #negotiated = false;
  #sequence = 0;
  #nextRequest = 0;
  #session: SessionState | null = null;
  #turn: TurnState | null = null;
  #readyEmitted = false;
  #hostSequence = 0;

  constructor(options: RuntimeServerOptions) {
    this.#diagnostic = options.diagnostic ?? ((line) => process.stderr.write(`${line}\n`));
    this.#defaultLimits = options.limits ?? DEFAULT_LIMITS;
    this.#limits = this.#defaultLimits;
    this.#bundle = options.bundle;
    this.#instanceId = options.instanceId ?? randomUUID().replaceAll("-", "");
    this.#runtimeVersion = options.runtimeVersion ?? RUNTIME_VERSION;
    this.#onStopped = options.onStopped;
    this.#clock = options.clock ?? monotonicClock;
    this.#transport = new NdjsonTransport(
      options.input,
      options.output,
      { frameBytes: this.#limits.frameBytes, queueBytes: this.#limits.queueBytes, controlLaneBytes: this.#limits.controlLaneBytes },
      {
        onFrame: (bytes) => this.#onFrame(bytes),
        onViolation: (reason) => this.#violation(reason),
        onEnd: (partial) => this.#onEnd(partial),
      },
    );
  }

  start(): void {
    this.#transport.start();
  }

  get instanceId(): string {
    return this.#instanceId;
  }

  get phase(): string {
    return this.#phase;
  }

  /** Used by tests and by the process entry point to force an orderly stop. */
  stop(code = 0): void {
    if (this.#phase === "stopped" || this.#phase === "transport_error") {
      return;
    }
    this.#phase = "stopped";
    this.#retireTurn(this.#turn);
    this.#turn = null;
    this.#rejectAll(new ProtocolError("CANCELLED", "The runtime is stopping."));
    this.#transport.end();
    this.#onStopped?.(code);
  }

  #onFrame(bytes: Buffer): void {
    if (this.#phase === "stopping" || this.#phase === "stopped" || this.#phase === "transport_error") {
      return;
    }
    const phase = this.#negotiated ? "negotiated" : "bootstrap";
    const decoded = decodeFrameBytes(bytes, phase, "host", this.#limits, this.#bundle);
    if (decoded.kind === "transport_error") {
      this.#violation(decoded.reason);
      return;
    }
    if (decoded.kind === "request_error") {
      this.#respondError(decoded.header.requestId, decoded.header.scope, new ProtocolError(decoded.code, "The request was rejected."));
      return;
    }
    const frame = decoded.frame;
    if (frame.kind === "request") {
      if (this.#seenIncoming.has(frame.requestId)) {
        this.#violation("duplicate requestId for the transport lifetime");
        return;
      }
      this.#seenIncoming.add(frame.requestId);
      this.#incomingOrder.push(frame.requestId);
      if (this.#incomingOrder.length > this.#limits.requestLedgerEntries) {
        this.#violation("request ledger exhausted");
        return;
      }
      this.#handleRequest(frame);
    } else if (frame.kind === "response") {
      this.#handleResponse(frame);
    } else {
      this.#handleHostEvent(frame);
    }
  }

  #handleRequest(request: RequestFrame): void {
    try {
      switch (request.method) {
        case "runtime.hello":
          this.#handleHello(request);
          return;
        case "session.start":
          this.#handleSessionStart(request);
          return;
        case "turn.start":
          this.#handleTurnStart(request);
          return;
        case "turn.cancel":
          this.#handleTurnCancel(request);
          return;
        case "request.cancel":
          this.#handleRequestCancel(request);
          return;
        case "session.reset":
          this.#handleSessionReset(request);
          return;
        case "runtime.shutdown":
          this.#handleShutdown(request);
          return;
        default:
          this.#respondError(request.requestId, request.scope, methodNotFound(request.method));
      }
    } catch (error) {
      const wire = asProtocolError(error);
      this.#respondError(request.requestId, request.scope, wire);
    }
  }

  #handleHello(request: RequestFrame): void {
    if (this.#negotiated) {
      this.#respondError(request.requestId, request.scope, invalidRequest("runtime.hello was already negotiated."));
      return;
    }
    if (!scopesEqual(request.scope, runtimeScope())) {
      this.#respondError(request.requestId, request.scope, invalidRequest("runtime.hello uses the runtime scope."));
      return;
    }
    const versions = request.params["supportedVersions"];
    const offered = Array.isArray(versions) ? versions.filter((value): value is string => typeof value === "string") : [];
    if (!offered.includes(PROTOCOL_VERSION)) {
      this.#respondError(
        request.requestId,
        request.scope,
        new ProtocolError("INCOMPATIBLE_VERSION", "No exact protocol version is common.", "never", { supported: [PROTOCOL_VERSION] }),
      );
      this.#stopAfterFlush(0);
      return;
    }
    const hostLimits = request.params["limits"] as unknown as Limits;
    this.#limits = narrowerLimits(hostLimits);
    if (!limitsValid(this.#limits)) {
      this.#respondError(request.requestId, request.scope, invalidRequest("The offered limits are not internally consistent.", { field: "limits" }));
      return;
    }
    this.#transport.setLimits({
      frameBytes: this.#limits.frameBytes,
      queueBytes: this.#limits.queueBytes,
      controlLaneBytes: this.#limits.controlLaneBytes,
    });
    this.#negotiated = true;
    this.#phase = "ready";
    this.#respondOk(request.requestId, request.scope, {
      selectedVersion: PROTOCOL_VERSION,
      runtimeInstanceId: this.#instanceId,
      runtimeVersion: this.#runtimeVersion,
      capabilities: {
        workflowIds: [...WORKFLOW_IDS],
        hostModelCalls: true,
        hostAgentTasks: AGENT_RUN_CAPABILITY.hostAgentTasks,
        guardedExecute: false,
        guardedBatch: false,
        snapshotValidation: false,
      },
      limits: this.#limits,
    });
    if (!this.#readyEmitted) {
      this.#readyEmitted = true;
      this.#emit("runtime.ready", runtimeScope(), null, { runtimeInstanceId: this.#instanceId });
    }
  }

  #handleSessionStart(request: RequestFrame): void {
    if (!this.#negotiated) {
      this.#violation("a request arrived before runtime.hello");
      return;
    }
    if (this.#session !== null) {
      this.#respondError(request.requestId, request.scope, resourceExhausted("sessions", this.#limits.sessions));
      return;
    }
    const workflowId = request.params["workflowId"];
    const instructions = request.params["instructions"];
    const bridge = request.params["bridge"];
    if (typeof workflowId !== "string" || typeof instructions !== "string") {
      this.#respondError(request.requestId, request.scope, invalidRequest("session.start parameters are invalid."));
      return;
    }
    if (!WORKFLOW_IDS.includes(workflowId)) {
      this.#respondError(request.requestId, request.scope, unsupportedCapability(`workflow:${workflowId}`));
      return;
    }
    if (bridge !== null && !isObject(bridge)) {
      this.#respondError(request.requestId, request.scope, invalidRequest("The bridge reference is invalid."));
      return;
    }
    this.#session = {
      id: request.scope.sessionId as string,
      workflowId,
      instructions,
      bridge: bridge as JsonObject | null,
      history: new SessionHistory(),
      branch: new ContinuationBranch(this.#limits.continuationTtlSeconds),
    };
    this.#respondOk(request.requestId, request.scope, { state: "ready", workflowId });
  }

  #handleTurnStart(request: RequestFrame): void {
    if (!this.#negotiated || this.#session === null) {
      this.#respondError(request.requestId, request.scope, invalidRequest("There is no active session."));
      return;
    }
    if (this.#turn !== null) {
      this.#respondError(request.requestId, request.scope, resourceExhausted("activeTurns", this.#limits.activeTurns));
      return;
    }
    if (request.scope.sessionId !== this.#session.id) {
      this.#respondError(request.requestId, request.scope, invalidRequest("The turn scope names an unknown session."));
      return;
    }
    const binding = request.params["binding"];
    if (!isObject(binding) || binding["workflowId"] !== this.#session.workflowId) {
      this.#respondError(request.requestId, request.scope, invalidRequest("The turn binding does not match the session workflow."));
      return;
    }
    const document = binding["document"];
    if (!isObject(document) || typeof document["documentId"] !== "string" || typeof document["revision"] !== "string") {
      this.#respondError(request.requestId, request.scope, invalidRequest("The turn binding has no valid document stamp."));
      return;
    }
    const budgets = request.params["budgets"] as unknown as Budgets;
    const parts = request.params["parts"];
    if (!Array.isArray(parts)) {
      this.#respondError(request.requestId, request.scope, invalidRequest("turn.start needs content parts."));
      return;
    }
    for (const part of parts) {
      if (!isObject(part)) {
        this.#respondError(request.requestId, request.scope, invalidRequest("A content part is invalid."));
        return;
      }
      if (part["type"] === "image") {
        this.#respondError(request.requestId, request.scope, unsupportedCapability("images"));
        return;
      }
      if (part["type"] === "document") {
        this.#respondError(request.requestId, request.scope, unsupportedCapability("documents"));
        return;
      }
    }
    const text = parts
      .filter((part): part is JsonObject => isObject(part))
      .filter((part) => part["type"] === "text" && typeof part["text"] === "string")
      .map((part) => part["text"] as string)
      .join("\n");
    const userMessage: ModelMessage = {
      role: "user",
      content: [{ type: "text", text }],
      toolCalls: [],
      toolCallId: null,
    };
    const historyBlock = this.#session.history.blockedBy([userMessage], this.#limits);
    if (historyBlock !== null) {
      this.#respondError(request.requestId, request.scope, resourceExhausted(historyBlock, this.#limits[historyBlock]));
      return;
    }
    const now = this.#clock.now();
    const turn: TurnState = {
      sessionId: request.scope.sessionId as string,
      turnId: request.scope.turnId as string,
      workflowRunId: request.scope.workflowRunId as string,
      requestId: request.requestId,
      binding,
      document: { documentId: document["documentId"], revision: document["revision"] },
      budgets,
      abort: new AbortController(),
      startedAt: now,
      deadline: now + budgets.wallTimeSeconds * 1000,
      userMessage,
      cancelled: false,
      retired: false,
      finished: false,
      modelCalls: 0,
      outcome: null,
    };
    this.#turn = turn;
    this.#respondOk(request.requestId, request.scope, { state: "accepted" });
    queueMicrotask(() => {
      void this.#runTurn(turn);
    });
  }

  #handleTurnCancel(request: RequestFrame): void {
    const turn = this.#turn;
    if (
      turn !== null &&
      turn.turnId === request.scope.turnId &&
      turn.sessionId === request.scope.sessionId &&
      turn.workflowRunId === request.scope.workflowRunId
    ) {
      if (!turn.cancelled) {
        turn.cancelled = true;
        this.#cancelPendingForTurn(turn);
        turn.abort.abort();
      }
      this.#respondOk(request.requestId, request.scope, { state: "cancelling" });
      this.#finishTurn(turn, "cancelled", "The turn was cancelled.");
      return;
    }
    this.#respondOk(request.requestId, request.scope, { state: "terminal" });
  }

  #handleRequestCancel(request: RequestFrame): void {
    const target = request.params["targetRequestId"];
    if (typeof target !== "string" || !target.startsWith("h:")) {
      this.#respondError(
        request.requestId,
        request.scope,
        invalidRequest("request.cancel may only target a host-originated call.", { field: "targetRequestId" }),
      );
      return;
    }
    this.#respondOk(request.requestId, request.scope, { state: "terminal" });
  }

  #handleSessionReset(request: RequestFrame): void {
    const session = this.#session;
    if (session === null || request.scope.sessionId !== session.id) {
      this.#respondError(request.requestId, request.scope, invalidRequest("session.reset names an unknown session."));
      return;
    }
    const nextSessionId = request.params["nextSessionId"];
    const instructions = request.params["instructions"];
    if (!isId(nextSessionId) || typeof instructions !== "string") {
      this.#respondError(request.requestId, request.scope, invalidRequest("session.reset parameters are invalid."));
      return;
    }
    if (this.#turn !== null) {
      this.#retireTurn(this.#turn);
      this.#turn = null;
    }
    this.#session = {
      id: nextSessionId,
      workflowId: session.workflowId,
      instructions,
      bridge: session.bridge,
      history: new SessionHistory(),
      branch: new ContinuationBranch(this.#limits.continuationTtlSeconds),
    };
    this.#respondOk(request.requestId, request.scope, { state: "ready", sessionId: nextSessionId });
  }

  #handleShutdown(request: RequestFrame): void {
    this.#respondOk(request.requestId, request.scope, { state: "stopping" });
    this.#stopAfterFlush(0);
  }

  #handleResponse(response: ResponseFrame): void {
    const pending = this.#pending.get(response.requestId);
    if (pending === undefined) {
      if (this.#tombstones.has(response.requestId)) {
        this.#tombstones.delete(response.requestId);
        return;
      }
      this.#violation("response names an unknown requestId");
      return;
    }
    this.#pending.delete(response.requestId);
    clearTimeout(pending.timer);
    pending.removeAbort();
    if (!scopesEqual(pending.scope, response.scope)) {
      pending.reject(new ProtocolError("TRANSPORT_ERROR", "A response did not echo the request scope."));
      this.#violation("response scope does not match its request");
      return;
    }
    if (response.outcome === "error") {
      const code = response.error.code as ErrorCode;
      pending.reject(new ProtocolError(code, response.error.message, response.error.retry as RetryAdvice, response.error.details));
      return;
    }
    const definition = RESULT_DEFINITIONS[pending.method];
    if (definition === undefined) {
      pending.reject(new ProtocolError("INTERNAL_ERROR", "No result definition exists for this method."));
      return;
    }
    const schemaRef = `#/$defs/${definition}`;
    if (!this.#bundle.validate(schemaRef, response.result) || !validateDataValue(schemaRef, response.result, this.#limits)) {
      pending.reject(new ProtocolError("MALFORMED_RESPONSE", "HostAi returned a result that does not match the protocol."));
      return;
    }
    pending.resolve(response.result);
  }

  #handleHostEvent(event: EventFrame): void {
    if (event.sequence <= this.#hostSequence) {
      this.#violation("host event sequence is duplicate or decreasing");
      return;
    }
    if (this.#hostSequence === 0 && event.sequence !== 1) {
      this.#violation("host event sequence must start at 1");
      return;
    }
    this.#hostSequence = event.sequence;
    const session = this.#session;
    if (session === null || event.scope.sessionId !== session.id) {
      return;
    }
    if (event.event === "model.delta") {
      // Buffered delivery was negotiated; genuine deltas were never offered.
      return;
    }
    // connection.state and agent.progress are host projections. #7 records no
    // additional durable state and never invents progress from them.
  }

  #respondOk(requestId: string, scope: Scope, result: JsonObject): void {
    if (this.#phase === "stopping" || this.#phase === "stopped" || this.#phase === "transport_error") {
      return;
    }
    this.#write({ protocolVersion: PROTOCOL_VERSION, kind: "response", requestId, scope, outcome: "ok", result });
  }

  #respondError(requestId: string, scope: Scope, error: ProtocolError): void {
    if (this.#phase === "stopping" || this.#phase === "stopped" || this.#phase === "transport_error") {
      return;
    }
    this.#write({ protocolVersion: PROTOCOL_VERSION, kind: "response", requestId, scope, outcome: "error", error: error.toWire() });
  }

  #emit(event: string, scope: Scope, requestId: string | null, data: JsonObject): void {
    if (this.#phase === "stopping" || this.#phase === "stopped" || this.#phase === "transport_error") {
      return;
    }
    if (this.#sequence >= 2_147_483_647) {
      this.#violation("event sequence exhausted");
      return;
    }
    this.#sequence += 1;
    this.#write({ protocolVersion: PROTOCOL_VERSION, kind: "event", requestId, scope, sequence: this.#sequence, event, data });
  }

  #emitTurn(turn: TurnState, event: string, data: JsonObject): void {
    if (turn.finished || turn.retired) {
      return;
    }
    this.#emit(event, turnScope(turn.sessionId, turn.turnId, turn.workflowRunId), turn.requestId, data);
  }

  #write(frame: Frame): void {
    try {
      this.#transport.write(frame);
    } catch (error) {
      if (error instanceof ProtocolError && error.value.code === "RESOURCE_EXHAUSTED") {
        if (frame.kind === "request") {
          const pending = this.#pending.get(frame.requestId);
          if (pending !== undefined) {
            this.#pending.delete(frame.requestId);
            clearTimeout(pending.timer);
            pending.removeAbort();
            pending.reject(error);
          }
          return;
        }
        this.#violation(error.message);
        return;
      }
      this.#violation(error instanceof Error ? error.message : "transport write failed");
    }
  }

  async #runTurn(turn: TurnState): Promise<void> {
    try {
      if (turn.cancelled) {
        throw new TurnCancelled();
      }
      if (turn.modelCalls >= turn.budgets.modelCalls) {
        throw new BudgetExhausted();
      }
      const session = this.#session;
      if (session === null || session.id !== turn.sessionId || turn.retired) {
        return;
      }
      const workflow = this.#workflow(session, turn);
      const run = await workflow.createRun();
      const result = await run.start({ inputData: {} });
      if (result.status !== "success") {
        throw new Error(`workflow ${result.status}`);
      }
      if (turn.cancelled) {
        throw new TurnCancelled();
      }
      if (this.#clock.now() > turn.deadline) {
        throw new BudgetExhausted();
      }
      this.#appendHistory(session, turn);
      const outcome = turn.outcome;
      if (outcome === null) {
        this.#finishTurn(turn, "task_failed", "The workflow produced no modeling answer.");
      } else if (outcome.type === "tool-calls") {
        this.#emitTurn(turn, "workflow.stage", {
          stageId: "dispatch",
          title: "Native tool dispatch is not available in this runtime revision.",
          state: "failed",
        });
        this.#finishTurn(turn, "task_failed", "The model proposed a tool call. Native dispatch lands with issue #10; no CAD command was executed.");
      } else {
        this.#finishTurn(turn, "task_failed", "The model produced no native modeling result.");
      }
    } catch (error) {
      this.#finishFromError(turn, error);
    }
  }

  #workflow(session: SessionState, turn: TurnState) {
    const stepResult = z.object({ ok: z.boolean() });
    const inspect = createStep({
      id: "inspect",
      inputSchema: z.object({}),
      outputSchema: stepResult,
      execute: async () => {
        this.#checkCancelled(turn);
        this.#emitTurn(turn, "workflow.stage", { stageId: "inspect", title: "Inspect the frozen turn", state: "running" });
        this.#checkCancelled(turn);
        this.#emitTurn(turn, "workflow.stage", { stageId: "inspect", title: "Inspect the frozen turn", state: "completed" });
        return { ok: true };
      },
    });
    const generate = createStep({
      id: "generate",
      inputSchema: stepResult,
      outputSchema: stepResult,
      execute: async () => {
        this.#checkCancelled(turn);
        this.#emitTurn(turn, "workflow.stage", { stageId: "model", title: "Ask HostAi for one model step", state: "running" });
        const remainingSeconds = Math.max(1, Math.floor((turn.deadline - this.#clock.now()) / 1000));
        const client = new HostAiModelClient(
          this.#hostTransport(),
          {
            sessionId: session.id,
            turnId: turn.turnId,
            workflowRunId: turn.workflowRunId,
            connectionId: String(turn.binding["connectionId"]),
            modelId: String(turn.binding["modelId"]),
            remainingSeconds,
          },
          this.#limits,
          () => {
            turn.modelCalls += 1;
          },
        );
        if (turn.modelCalls > turn.budgets.modelCalls) {
          throw new BudgetExhausted();
        }
        const agent = createHostAiAgent({
          client,
          connectionId: String(turn.binding["connectionId"]),
          modelId: String(turn.binding["modelId"]),
          timeoutSeconds: Math.min(this.#limits.callDefaultSeconds, remainingSeconds),
          instructions: session.instructions,
          tools: [],
          signal: turn.abort.signal,
          limits: this.#limits,
          branch: session.branch,
        }) as Agent;
        const messages: ModelMessage[] = [...session.history.messages, turn.userMessage];
        const result = await agent.generateLegacy(toMastraMessages(messages) as Parameters<Agent["generateLegacy"]>[0], { maxSteps: 1 });
        this.#checkCancelled(turn);
        if (this.#clock.now() > turn.deadline) {
          throw new BudgetExhausted();
        }
        const steps = Array.isArray(result.steps) ? result.steps : [];
        if (steps.length > 1) {
          throw new ProtocolError("INTERNAL_ERROR", "Mastra executed more than one model step.");
        }
        const toolCalls = Array.isArray(result.toolCalls)
          ? result.toolCalls.map((call) => ({
              id: String(call.toolCallId),
              name: String(call.toolName),
              arguments: isObject(call.args) ? call.args : {},
            }))
          : [];
        turn.outcome = {
          type: toolCalls.length > 0 ? "tool-calls" : "text",
          text: typeof result.text === "string" ? result.text : "",
          toolCalls,
          finishReason: String(result.finishReason),
        };
        this.#emitTurn(turn, "workflow.stage", { stageId: "model", title: "Ask HostAi for one model step", state: "completed" });
        if (toolCalls.length === 0 && turn.outcome.text.length > 0) {
          this.#emitTurn(turn, "workflow.text", { text: turn.outcome.text, channel: "answer" });
        }
        return { ok: true };
      },
    });
    return createWorkflow({ id: "native-modeling", inputSchema: z.object({}), outputSchema: stepResult }).then(inspect).then(generate).commit();
  }

  #hostTransport(): HostAiTransport {
    return {
      call: (method, scope, params, timeoutSeconds, signal) => this.#callHost(method, scope, params, timeoutSeconds, signal),
    };
  }

  #callHost(method: string, scope: Scope, params: JsonObject, timeoutSeconds: number, signal: AbortSignal): Promise<JsonObject> {
    return new Promise<JsonObject>((resolve, reject) => {
      if (signal.aborted) {
        reject(cancelled(method));
        return;
      }
      if (this.#pending.size >= this.#limits.pendingRequests) {
        reject(resourceExhausted("pendingRequests", this.#limits.pendingRequests));
        return;
      }
      this.#nextRequest += 1;
      const requestId = `r:${this.#nextRequest.toString(36)}-${randomUUID().replaceAll("-", "").slice(0, 12)}`;
      const timer = setTimeout(() => {
        this.#pending.delete(requestId);
        this.#addTombstone(requestId);
        reject(timeout(method));
      }, Math.max(1, timeoutSeconds) * 1000);
      const onAbort = (): void => {
        if (!this.#pending.has(requestId)) {
          return;
        }
        this.#pending.delete(requestId);
        clearTimeout(timer);
        this.#addTombstone(requestId);
        reject(cancelled(method));
      };
      signal.addEventListener("abort", onAbort, { once: true });
      const pending: PendingCall = {
        id: requestId,
        method,
        scope,
        control: false,
        resolve,
        reject,
        timer,
        removeAbort: () => signal.removeEventListener("abort", onAbort),
      };
      this.#pending.set(requestId, pending);
      this.#write({
        protocolVersion: PROTOCOL_VERSION,
        kind: "request",
        requestId,
        scope,
        method,
        params,
      });
    });
  }

  #cancelPendingForTurn(turn: TurnState): void {
    for (const pending of [...this.#pending.values()]) {
      if (pending.control) {
        continue;
      }
      if (pending.scope.sessionId !== turn.sessionId || pending.scope.turnId !== turn.turnId || pending.scope.workflowRunId !== turn.workflowRunId) {
        continue;
      }
      this.#pending.delete(pending.id);
      clearTimeout(pending.timer);
      pending.removeAbort();
      this.#addTombstone(pending.id);
      pending.reject(cancelled(pending.method));
      this.#sendRequestCancel(pending.id, pending.scope);
    }
  }

  #sendRequestCancel(targetRequestId: string, scope: Scope): void {
    if (this.#phase === "stopping" || this.#phase === "stopped" || this.#phase === "transport_error") {
      return;
    }
    this.#nextRequest += 1;
    const requestId = `r:cancel-${this.#nextRequest.toString(36)}`;
    const timer = setTimeout(() => {
      this.#pending.delete(requestId);
    }, Math.max(1, this.#limits.cancelAckSeconds) * 1000);
    this.#pending.set(requestId, {
      id: requestId,
      method: "request.cancel",
      scope,
      control: true,
      resolve: () => undefined,
      reject: () => undefined,
      timer,
      removeAbort: () => undefined,
    });
    this.#write({
      protocolVersion: PROTOCOL_VERSION,
      kind: "request",
      requestId,
      scope,
      method: "request.cancel",
      params: { targetRequestId },
    });
  }

  #addTombstone(requestId: string): void {
    this.#tombstones.add(requestId);
    if (this.#tombstones.size > this.#limits.requestLedgerEntries) {
      this.#violation("cancelled request ledger exhausted");
    }
  }

  #checkCancelled(turn: TurnState): void {
    if (turn.cancelled || turn.retired) {
      throw new TurnCancelled();
    }
  }

  #appendHistory(session: SessionState, turn: TurnState): void {
    const outcome = turn.outcome;
    if (outcome === null || outcome.type === "tool-calls") {
      // An unresolved tool call never becomes history: the outer workflow must
      // supply its matching result before another user turn.
      return;
    }
    const assistant: ModelMessage = {
      role: "assistant",
      content: outcome.text.length === 0 ? [] : [{ type: "text", text: outcome.text }],
      toolCalls: outcome.toolCalls.map((call) => ({
        id: String(call["id"]),
        name: String(call["name"]),
        arguments: isObject(call["arguments"]) ? call["arguments"] : {},
      })),
      toolCallId: null,
    };
    session.history.appendRound([turn.userMessage, assistant], this.#limits);
  }

  #finishFromError(turn: TurnState, error: unknown): void {
    if (turn.retired || turn.finished) {
      return;
    }
    if (turn.cancelled || error instanceof TurnCancelled) {
      this.#finishTurn(turn, "cancelled", "The turn was cancelled.");
      return;
    }
    if (error instanceof BudgetExhausted) {
      this.#finishTurn(turn, "budget_exhausted", "The turn budget was exhausted.");
      return;
    }
    if (this.#clock.now() > turn.deadline) {
      this.#finishTurn(turn, "budget_exhausted", "The turn wall-clock budget was exhausted.");
      return;
    }
    const wire = asProtocolError(error);
    if (wire.value.code === "AUTH_REQUIRED") {
      this.#finishTurn(turn, "auth_required", wire.value.message);
      return;
    }
    if (wire.value.code === "CANCELLED") {
      this.#finishTurn(turn, "cancelled", wire.value.message);
      return;
    }
    if (wire.value.code === "RESOURCE_EXHAUSTED") {
      this.#finishTurn(turn, "budget_exhausted", wire.value.message);
      return;
    }
    this.#emitTurn(turn, "workflow.stage", { stageId: "model", title: "Ask HostAi for one model step", state: "failed" });
    this.#finishTurn(turn, "task_failed", wire.value.message);
  }

  #finishTurn(turn: TurnState, state: TerminalState, reason: string): void {
    if (turn.finished || turn.retired) {
      return;
    }
    turn.finished = true;
    this.#emit("turn.finished", turnScope(turn.sessionId, turn.turnId, turn.workflowRunId), turn.requestId, {
      state,
      reason,
      document: turn.document,
      changes: "none",
      validationRunId: null,
      report: null,
    });
    if (this.#turn === turn) {
      this.#turn = null;
    }
  }

  #retireTurn(turn: TurnState | null): void {
    if (turn === null) {
      return;
    }
    turn.retired = true;
    turn.cancelled = true;
    this.#cancelPendingForTurn(turn);
    turn.abort.abort();
  }

  #rejectAll(error: ProtocolError): void {
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.removeAbort();
      pending.reject(error);
    }
    this.#pending.clear();
  }

  #stopAfterFlush(code: number): void {
    this.#phase = "stopping";
    this.#retireTurn(this.#turn);
    this.#turn = null;
    this.#rejectAll(new ProtocolError("CANCELLED", "The runtime is stopping."));
    this.#transport.end();
    this.#onStopped?.(code);
  }

  #violation(reason: string): void {
    if (this.#phase === "transport_error") {
      return;
    }
    this.#phase = "transport_error";
    this.#diagnostic(`protocol violation: ${reason}`);
    this.#rejectAll(new ProtocolError("TRANSPORT_ERROR", reason));
    this.#turn = null;
    this.#session = null;
    this.#transport.closeInput();
    this.#transport.end();
    this.#onStopped?.(2);
  }

  #onEnd(partial: boolean): void {
    if (this.#phase === "stopped" || this.#phase === "transport_error") {
      return;
    }
    if (partial) {
      this.#violation("EOF in the middle of a frame");
      return;
    }
    this.#phase = "stopped";
    this.#retireTurn(this.#turn);
    this.#turn = null;
    this.#rejectAll(new ProtocolError("CANCELLED", "The host closed the transport."));
    this.#onStopped?.(0);
  }
}

export { HostAiModelClient, type ModelGenerateResult };
