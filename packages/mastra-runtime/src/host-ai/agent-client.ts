/**
 * `agent.run` is a separate proposal-only ACP operation. The ACP task executor
 * is #4; until it lands this runtime advertises `hostAgentTasks: false` and
 * refuses without sending a request. It never emulates ACP with a model call.
 */

import { unsupportedCapability } from "../errors.js";
import type { JsonObject } from "../protocol.js";

export interface AgentRunRequest {
  readonly connectionId: string;
  readonly modelId: string;
  readonly task: readonly JsonObject[];
  readonly document: { readonly documentId: string; readonly revision: string };
  readonly requirements: JsonObject | null;
}

export interface AgentRunResult {
  readonly agentSessionId: string;
  readonly state: "completed";
  readonly content: readonly JsonObject[];
  readonly stopReason: string;
  readonly usage: JsonObject | null;
}

export const AGENT_RUN_CAPABILITY = Object.freeze({
  hostAgentTasks: false,
  reason: "ACP task execution lands with issue #4.",
});

export class HostAiAgentClient {
  readonly #available: boolean;

  constructor(available = false) {
    this.#available = available;
  }

  get available(): boolean {
    return this.#available;
  }

  async run(_request: AgentRunRequest): Promise<AgentRunResult> {
    if (!this.#available) {
      throw unsupportedCapability("agent.run");
    }
    // The #4 executor owns the ACP session, authentication and proposal checks.
    throw unsupportedCapability("agent.run");
  }
}
