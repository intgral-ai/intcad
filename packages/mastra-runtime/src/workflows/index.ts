/**
 * Native SolidCraft modeling workflow (#10).
 *
 * `runNativeModelingWorkflow` consumes the #7 single-step model seam and #9's
 * guarded CAD adapter, and exposes a host snapshot-validation handoff (#13).
 * It never verifies a report or turns LLM text into success.
 */

export { runNativeModelingWorkflow } from "./workflow.js";
export { BudgetMeter } from "./budget.js";
export {
  evaluateFixedChecks,
  failedCheckIds,
  parameterMap,
  requiredChecksPassed,
  resolvePointer,
  validateRequirements,
} from "./checks.js";
export { inspectAndFreeze, prepareTurn } from "./inspect.js";
export {
  APPLY_TOOL,
  DECISION_TOOL,
  PROBE_TOOL,
  READ_TOOLS,
  WORKFLOW_TOOLS,
  parseAcpProposal,
  parseApplyProposal,
  parseDecision,
  type Decision,
} from "./proposals.js";
export type {
  BudgetUsage,
  CadPort,
  ChangeState,
  CheckRequirement,
  CheckResult,
  ExactMeasurement,
  FixedRequirements,
  FrozenInspection,
  ModelStep,
  NativeWorkflowInput,
  ParameterRequirement,
  PlanStep,
  PlanStepState,
  ProfileRef,
  ProposalRequest,
  ProposalSource,
  StageEvent,
  StageState,
  TextChannel,
  ToolEvent,
  ToolState,
  ValidationHandoff,
  ValidationOutcome,
  ValidationRequest,
  WorkflowEvents,
  WorkflowOptions,
  WorkflowResult,
  WorkflowTerminalState,
} from "./types.js";
