/**
 * Workflow-facing native tools and proposal validation.
 *
 * These declarations are the workflow's proposal surface over #9's discovered
 * command/schema interface. They are not a second command catalog: every
 * `apply_native_commands` step is validated again by the guarded adapter
 * against the bridge's own `describe_command` schemas before dispatch.
 */

import { invalidRequest } from "../errors.js";
import { isObject } from "../limits.js";
import type { JsonObject } from "../protocol.js";
import type { ModelMessage, ToolCall, ToolDefinition } from "../host-ai/model-client.js";
import { isCanonicalRevision, type DocumentStamp } from "../solidcraft/cad-adapter.js";
import type { CadBatchStep } from "../solidcraft/cad-adapter.js";

export const DECISION_TOOL = "record_decision";
export const APPLY_TOOL = "apply_native_commands";
export const PROBE_TOOL = "probe_cad";
export const INSPECT_TOOL = "inspect_design";
export const MEASURE_TOOL = "measure";
export const DESCRIBE_TOOL = "describe_command";

export const READ_TOOLS: ReadonlySet<string> = new Set([INSPECT_TOOL, MEASURE_TOOL, DESCRIBE_TOOL]);

export type DecisionKind = "edit" | "ask" | "decline";

export interface Decision {
  readonly decision: DecisionKind;
  readonly reason: string;
  readonly answer: string;
}

export interface ApplyProposal {
  readonly commands: readonly CadBatchStep[];
}

export const WORKFLOW_TOOLS: readonly ToolDefinition[] = Object.freeze([
  {
    name: DECISION_TOOL,
    description:
      "Record the explicit decision for this turn before any native edit. `edit` permits planning; it is not a permission bypass. " +
      "`ask` needs a clarification and `decline` refuses the task; both stop all further writes.",
    inputSchema: {
      type: "object",
      additionalProperties: false,
      required: ["decision", "reason"],
      properties: {
        decision: { type: "string", enum: ["edit", "ask", "decline"] },
        reason: { type: "string", minLength: 1, maxLength: 2048 },
        answer: { type: "string", maxLength: 16384 },
      },
    },
  },
  {
    name: APPLY_TOOL,
    description:
      "Propose ordered native SolidCraft commands. The workflow validates the declaration, reserves every command, applies one guarded " +
      "batch (or one guarded command) with the inspected expectedDocument and then checks the fixed requirements exactly. A returned call is a proposal, not proof of an edit.",
    inputSchema: {
      type: "object",
      additionalProperties: false,
      required: ["commands"],
      properties: {
        commands: {
          type: "array",
          minItems: 1,
          maxItems: 64,
          items: {
            type: "object",
            additionalProperties: false,
            required: ["command"],
            properties: {
              command: { type: "string", minLength: 1, maxLength: 256 },
              params: { type: "object" },
            },
          },
        },
        note: { type: "string", maxLength: 2048 },
      },
    },
  },
  {
    name: INSPECT_TOOL,
    description: "Read the live design exactly (parameters, timeline, bodies, selection). Read-only; no edit.",
    inputSchema: {
      type: "object",
      additionalProperties: false,
      properties: {
        measure: { type: "boolean" },
        sketches: { type: "boolean" },
      },
    },
  },
  {
    name: MEASURE_TOOL,
    description: "Measure exact native geometry. Read-only; display meshes are not evidence.",
    inputSchema: { type: "object", additionalProperties: true },
  },
  {
    name: DESCRIBE_TOOL,
    description: "Describe one discovered native command and its input/output schemas. Read-only.",
    inputSchema: {
      type: "object",
      additionalProperties: false,
      required: ["id"],
      properties: { id: { type: "string", minLength: 1, maxLength: 256 } },
    },
  },
  {
    name: PROBE_TOOL,
    description:
      "A speculative candidate-building probe is unsupported in 1.0. It returns UNSUPPORTED_CAPABILITY and performs zero writes; inspect read-only or apply guarded commands instead.",
    inputSchema: { type: "object", additionalProperties: true },
  },
]);

export function parseDecision(args: JsonObject): Decision {
  rejectExtraKeys(args, new Set(["decision", "reason", "answer"]), "decision");
  const decision = args["decision"];
  const reason = args["reason"];
  const answer = args["answer"];
  if (decision !== "edit" && decision !== "ask" && decision !== "decline") {
    throw invalidRequest("The decision must be edit, ask or decline.", { field: "decision" });
  }
  if (typeof reason !== "string" || reason.length === 0) {
    throw invalidRequest("A decision needs a reason.", { field: "reason" });
  }
  if (answer !== undefined && typeof answer !== "string") {
    throw invalidRequest("A decision answer must be text.", { field: "answer" });
  }
  return { decision, reason, answer: typeof answer === "string" ? answer : reason };
}

export function parseApplyProposal(args: JsonObject): ApplyProposal {
  rejectExtraKeys(args, new Set(["commands", "note"]), "commands");
  const raw = args["commands"];
  if (!Array.isArray(raw) || raw.length === 0 || raw.length > 64) {
    throw invalidRequest("A native proposal needs 1..64 commands.", { field: "commands" });
  }
  const commands: CadBatchStep[] = [];
  for (const [index, entry] of raw.entries()) {
    if (!isObject(entry) || typeof entry["command"] !== "string" || entry["command"].length === 0) {
      throw invalidRequest(`Native command ${index} has no command id.`, { field: "commands" });
    }
    rejectExtraKeys(entry, new Set(["command", "params"]), "commands");
    const params = entry["params"];
    if (params !== undefined && !isObject(params)) {
      throw invalidRequest(`Native command ${index} parameters must be an object.`, { field: "commands" });
    }
    commands.push({ command: entry["command"], params: params ?? {} });
  }
  return { commands };
}

export interface AcpProposal {
  readonly source: DocumentStamp;
  readonly commands: readonly CadBatchStep[];
}

/**
 * The #4 proposal-only agent contract: exactly `{source, commands}`.
 * External agent text is not evidence that any command was applied.
 */
export function parseAcpProposal(text: string, expected: DocumentStamp): AcpProposal {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw invalidRequest("The ACP proposal is not a JSON object.", { field: "proposal" });
  }
  if (!isObject(parsed)) {
    throw invalidRequest("The ACP proposal is not a JSON object.", { field: "proposal" });
  }
  rejectExtraKeys(parsed, new Set(["source", "commands"]), "proposal");
  const source = parsed["source"];
  if (
    !isObject(source) ||
    typeof source["documentId"] !== "string" ||
    typeof source["revision"] !== "string" ||
    !isCanonicalRevision(source["revision"])
  ) {
    throw invalidRequest("The ACP proposal has no valid document source stamp.", { field: "proposal" });
  }
  rejectExtraKeys(source, new Set(["documentId", "revision"]), "proposal");
  const commandsRaw = parsed["commands"];
  if (!Array.isArray(commandsRaw) || commandsRaw.length === 0 || commandsRaw.length > 64) {
    throw invalidRequest("The ACP proposal needs 1..64 commands.", { field: "proposal" });
  }
  if (source["documentId"] !== expected.documentId || source["revision"] !== expected.revision) {
    throw invalidRequest("The ACP proposal source stamp is stale.", {
      field: "proposal",
      expected,
      current: { documentId: source["documentId"], revision: source["revision"] },
    });
  }
  const commands: CadBatchStep[] = [];
  for (const [index, entry] of commandsRaw.entries()) {
    if (!isObject(entry) || typeof entry["command"] !== "string" || entry["command"].length === 0) {
      throw invalidRequest(`ACP command ${index} has no command id.`, { field: "proposal" });
    }
    rejectExtraKeys(entry, new Set(["command", "params"]), "proposal");
    const params = entry["params"];
    if (params !== undefined && !isObject(params)) {
      throw invalidRequest(`ACP command ${index} parameters must be an object.`, { field: "proposal" });
    }
    commands.push({ command: entry["command"], params: params ?? {} });
  }
  return {
    source: { documentId: source["documentId"], revision: source["revision"] },
    commands,
  };
}

export function assistantMessage(text: string, calls: readonly ToolCall[]): ModelMessage {
  return {
    role: "assistant",
    content: text.length === 0 ? [] : [{ type: "text", text }],
    toolCalls: [...calls],
    toolCallId: null,
  };
}

export function toolMessage(callId: string, value: JsonObject, limits = 262_144): ModelMessage {
  let text = JSON.stringify(value);
  if (text.length > limits) {
    text = JSON.stringify({
      truncated: true,
      originalBytes: text.length,
      note: "The native tool result exceeded the model text ceiling; inspect the design again with a narrower request.",
    });
  }
  return {
    role: "tool",
    content: [{ type: "text", text }],
    toolCalls: [],
    toolCallId: callId,
  };
}

export function errorOutput(error: { readonly code: string; readonly message: string; readonly details: JsonObject }): JsonObject {
  return { ok: false, code: error.code, message: error.message, details: error.details };
}

function rejectExtraKeys(value: JsonObject, allowed: ReadonlySet<string>, field: string): void {
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      throw invalidRequest(`Proposal field \`${key}\` is not allowed.`, { field });
    }
  }
}
