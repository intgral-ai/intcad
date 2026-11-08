/**
 * Inspect/freeze stage: read the bound document, selection, capabilities and
 * fixed requirements before any planning or mutation.
 */

import type { JsonObject } from "../protocol.js";
import { DEFAULT_LIMITS, type Limits } from "../limits.js";
import { parameterMap, replyValue, validateRequirements } from "./checks.js";
import { WORKFLOW_TOOLS } from "./proposals.js";
import type {
  CadPort,
  FrozenInspection,
  NativeWorkflowInput,
  PreparedTurn,
} from "./types.js";
import type { ModelMessage, ToolDefinition } from "../host-ai/model-client.js";
import type { DocumentStamp } from "../solidcraft/cad-adapter.js";

const SUMMARY_LIMIT = 64 * 1024;

export async function inspectAndFreeze(
  cad: CadPort,
  input: NativeWorkflowInput,
  limits: Limits = DEFAULT_LIMITS,
): Promise<FrozenInspection> {
  const requirements = validateRequirements(input.requirements, limits);
  const inspection = await cad.inspect({ design: true });
  const design = inspection.design === null ? null : replyValue(inspection.design);
  const capabilities = cad.capabilities();
  const frozen: FrozenInspection = {
    stamp: inspection.stamp,
    design,
    parameters: parameterMap(design),
    capabilities,
    requirements,
    selection: input.selection.map((value) => JSON.parse(JSON.stringify(value)) as JsonObject),
    summary: "",
  };
  return { ...frozen, summary: boundedSummary(frozen) };
}

export function initialMessages(
  input: NativeWorkflowInput,
  frozen: FrozenInspection,
  limits: Limits = DEFAULT_LIMITS,
): ModelMessage[] {
  const system = [
    input.instructions,
    [
      "You are the native SolidCraft modeling workflow. Work only through the declared tools.",
      "Read the frozen inspection, selection, capabilities and fixed requirements before planning.",
      "Record an explicit `record_decision` decision first. An `edit` decision permits planning but never bypasses guarded writes.",
      "Propose native SolidCraft commands with apply_native_commands. Each returned call is only a proposal; the guarded bridge decides whether it was applied.",
      "Never claim verified success from your own text. The host validates a native snapshot after this workflow's stage checks.",
      "Fixed requirements and tolerances are external. You cannot relax, add or remove them.",
      "A speculative candidate probe is unsupported; inspect read-only instead of editing then undoing.",
    ].join("\n"),
    `FROZEN TURN\n${frozen.summary}`,
  ].join("\n\n");
  return [
    { role: "system", content: [{ type: "text", text: clamp(system, limits.textBytes) }], toolCalls: [], toolCallId: null },
    {
      role: "user",
      content: [{ type: "text", text: input.parts.join("\n") }],
      toolCalls: [],
      toolCallId: null,
    },
  ];
}

export function prepareTurn(
  input: NativeWorkflowInput,
  frozen: FrozenInspection,
  limits: Limits = DEFAULT_LIMITS,
): PreparedTurn {
  const tools: readonly ToolDefinition[] = WORKFLOW_TOOLS;
  const messages = initialMessages(input, frozen, limits);
  return { frozen, messages, tools };
}

export function documentStampEquals(left: DocumentStamp, right: DocumentStamp): boolean {
  return left.documentId === right.documentId && left.revision === right.revision;
}

function boundedSummary(frozen: FrozenInspection): string {
  const design = frozen.design;
  const compact: JsonObject = {
    document: frozen.stamp,
    capabilities: frozen.capabilities,
    selection: frozen.selection,
    requirements: frozen.requirements,
    parameters: frozen.parameters,
    features: compactFeatures(design),
    bodies: compactBodies(design),
  };
  const text = JSON.stringify(compact, null, 2);
  if (text.length <= SUMMARY_LIMIT) {
    return text;
  }
  return JSON.stringify(
    {
      document: frozen.stamp,
      capabilities: frozen.capabilities,
      selection: frozen.selection,
      requirements: frozen.requirements,
      parameters: frozen.parameters,
      note: "The full design summary is too large for one model step; use inspect_design for details.",
    },
    null,
    2,
  );
}

function compactFeatures(design: JsonObject | null): unknown {
  const timeline = design?.["timeline"];
  if (!Array.isArray(timeline)) {
    return [];
  }
  return timeline.slice(0, 64).map((entry) => {
    if (typeof entry !== "object" || entry === null) {
      return entry;
    }
    const value = entry as JsonObject;
    return {
      id: value["id"],
      name: value["name"],
      type: value["type"],
      suppressed: value["suppressed"],
      rolled_back: value["rolled_back"],
      error: value["error"],
      warning: value["warning"],
    };
  });
}

function compactBodies(design: JsonObject | null): unknown {
  const bodies = design?.["bodies"];
  if (!Array.isArray(bodies)) {
    return [];
  }
  return bodies.slice(0, 64).map((entry) => {
    if (typeof entry !== "object" || entry === null) {
      return entry;
    }
    const value = entry as JsonObject;
    return {
      name: value["name"],
      volume_mm3: value["volume_mm3"],
      area_mm2: value["area_mm2"],
      bbox: value["bbox"],
      faces: value["faces"],
      error: value["error"],
    };
  });
}

function clamp(text: string, limit: number): string {
  return text.length <= limit ? text : text.slice(0, limit);
}
