/**
 * Shared types for the native SolidCraft modeling workflow.
 *
 * The workflow adapts the pinned Vector Studio authoring stages to native
 * commands. It owns the model/tool rounds and drives #9's guarded CAD port;
 * it never implements the host validation report gate (#13).
 */

import type { Clock } from "../clock.js";
import type {
  ModelGenerateRequest,
  ModelGenerateResult,
  ContentBlock,
  ModelMessage,
  ToolDefinition,
} from "../host-ai/model-client.js";
import type { Budgets, Limits } from "../limits.js";
import type { JsonObject } from "../protocol.js";
import type {
  CadAdapter,
  CadCapabilities,
  CadInspection,
  CadReadResult,
  CadWriteResult,
  DocumentStamp,
} from "../solidcraft/cad-adapter.js";

export type WorkflowTerminalState =
  | "validated_success"
  | "quality_failed"
  | "task_failed"
  | "auth_required"
  | "budget_exhausted"
  | "cancelled"
  | "transport_error"
  | "outcome_unknown";

export type ChangeState = "none" | "applied" | "rolled_back" | "unknown";

export type StageState = "running" | "completed" | "failed";
export type PlanStepState = "pending" | "running" | "completed" | "failed";
export type ToolState = "running" | "completed" | "failed";
export type TextChannel = "answer" | "progress";

/** Stable stage identity; a repeated round reuses the same stage id. */
export interface StageEvent {
  readonly stageId: string;
  readonly title: string;
  readonly state: StageState;
}

export interface PlanStep {
  readonly id: string;
  readonly title: string;
  readonly state: PlanStepState;
}

export interface ToolEvent {
  readonly callId: string;
  readonly name: string;
  readonly state: ToolState;
  readonly output: JsonObject | null;
}

export interface WorkflowEvents {
  stage(stageId: string, title: string, state: StageState): void;
  plan(steps: readonly PlanStep[]): void;
  text(text: string, channel: TextChannel): void;
  tool(callId: string, name: string, state: ToolState, output: JsonObject | null): void;
}

export interface ParameterRequirement {
  readonly name: string;
  readonly value: number;
  readonly absTolerance: number;
}

export interface ExactMeasurement {
  /** A read-only native MCP tool that returns exact geometry evidence. */
  readonly command: string;
  readonly params: JsonObject;
  /** JSON Pointer (`/total/volume_mm3`) or dotted path (`total.volume_mm3`). */
  readonly resultPointer: string;
  readonly expected: number;
  readonly absTolerance: number;
  readonly relTolerance: number;
}

export type CheckKind = "parameters.required" | "geometry.exact" | string;

export interface CheckRequirement {
  readonly checkId: string;
  readonly kind: CheckKind;
  readonly required: boolean;
  readonly parameters: JsonObject;
}

/** Fixed before planning; the model can read these but never change them. */
export interface FixedRequirements {
  readonly checks: readonly CheckRequirement[];
}

export interface ProfileRef {
  readonly profileId: string;
  readonly version: string;
  readonly digest: string;
}

export interface NativeWorkflowInput {
  readonly sessionId: string;
  readonly turnId: string;
  readonly workflowRunId: string;
  readonly connectionId: string;
  readonly modelId: string;
  readonly instructions: string;
  readonly parts: readonly string[];
  readonly selection: readonly JsonObject[];
  readonly requirements: FixedRequirements;
  readonly profile: ProfileRef;
  readonly document: DocumentStamp;
  readonly budgets: Budgets;
  /** BYOK consumes model tool calls; ACP validates a proposal-only task result. */
  readonly mode?: "byok" | "acp";
}

export interface CheckResult {
  readonly checkId: string;
  readonly required: boolean;
  readonly state: "passed" | "failed" | "error" | "unsupported" | "not_run";
  readonly expected: JsonObject;
  readonly actual: JsonObject;
  readonly detail: string;
}

export interface BudgetUsage {
  readonly modelCalls: number;
  readonly agentTasks: number;
  readonly cadCommands: number;
  readonly repairs: number;
  readonly validationRuns: number;
}

export interface WorkflowResult {
  readonly state: WorkflowTerminalState;
  readonly reason: string;
  /** Machine-readable reason distinct from the protocol terminal state. */
  readonly reasonCode: string;
  readonly document: DocumentStamp;
  readonly changes: ChangeState;
  readonly answer: string;
  readonly validationRunId: string | null;
  readonly report: JsonObject | null;
  readonly checks: readonly CheckResult[];
  readonly stages: readonly StageEvent[];
  readonly plan: readonly PlanStep[];
  readonly tools: readonly ToolEvent[];
  readonly budgets: BudgetUsage;
}

/** The #7 single-step HostAi seam: one generate call is one model invocation. */
export interface ModelStep {
  generate(request: ModelGenerateRequest, signal: AbortSignal): Promise<ModelGenerateResult>;
}

/**
 * #9's adapter surface used by the workflow. `Pick` keeps the real guarded
 * adapter as the only implementation of write identity and reconciliation.
 */
export type CadPort = Pick<
  CadAdapter,
  | "stamp"
  | "requiresReconciliation"
  | "capabilities"
  | "inspect"
  | "readTool"
  | "execute"
  | "batch"
  | "probe"
  | "reconcile"
  | "cancel"
>;

export interface ValidationRequest {
  readonly sessionId: string;
  readonly turnId: string;
  readonly workflowRunId: string;
  readonly document: DocumentStamp;
  readonly profile: ProfileRef;
  readonly requirements: FixedRequirements;
  readonly checks: readonly CheckResult[];
}

export interface ValidationOutcome {
  readonly state: "passed" | "quality_failed" | "infrastructure_error" | "cancelled" | "unknown";
  readonly runId: string | null;
  readonly report: JsonObject | null;
  readonly reason: string;
}

/**
 * Host-owned snapshot/validation handoff (#13). The workflow only consumes a
 * host decision; it does not verify report bytes or LLM success text.
 */
export interface ValidationHandoff {
  submit(request: ValidationRequest, signal: AbortSignal): Promise<ValidationOutcome>;
}

export interface ProposalRequest {
  readonly instructions: string;
  readonly task: string;
  readonly document: DocumentStamp;
  readonly selection: readonly JsonObject[];
  readonly capabilities: CadCapabilities;
  readonly requirements: FixedRequirements;
}

/** `agent.run` in proposal-only mode (#4); returns the agent's text reply. */
export interface ProposalSource {
  requestProposal(request: ProposalRequest, signal: AbortSignal): Promise<{ readonly text: string }>;
}

export interface WorkflowOptions {
  readonly model: ModelStep;
  readonly cad: CadPort;
  readonly events: WorkflowEvents;
  readonly signal: AbortSignal;
  readonly clock?: Clock;
  readonly limits?: Limits;
  /** Absent while #13 is not installed; a passed stage check is then not success. */
  readonly validation?: ValidationHandoff | null;
  /** Absent while #4 is not installed; ACP mode then refuses without writes. */
  readonly proposalSource?: ProposalSource | null;
}

export interface PreparedTurn {
  readonly frozen: FrozenInspection;
  readonly messages: readonly ModelMessage[];
  readonly tools: readonly ToolDefinition[];
}

export interface FrozenInspection {
  readonly stamp: DocumentStamp;
  readonly design: JsonObject | null;
  readonly parameters: Readonly<Record<string, number | null>>;
  readonly capabilities: CadCapabilities;
  readonly requirements: FixedRequirements;
  readonly selection: readonly JsonObject[];
  readonly summary: string;
}

export type Content = ContentBlock;
export type Message = ModelMessage;
