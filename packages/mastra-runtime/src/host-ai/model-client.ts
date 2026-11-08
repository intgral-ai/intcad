/**
 * The runtime side of `model.generate`: one HostAi request per call, buffered
 * delivery in 1.0, no hidden retry, no tool execution and no provider fallback.
 */

import { ProtocolError, invalidRequest, unsupportedCapability } from "../errors.js";
import { DEFAULT_LIMITS, isObject, type Limits } from "../limits.js";
import type { JsonObject, Scope } from "../protocol.js";
import { utf8Bytes } from "../protocol.js";

export interface ContentBlock {
  readonly type: "text";
  readonly text: string;
}

export interface ToolCall {
  readonly id: string;
  readonly name: string;
  readonly arguments: JsonObject;
}

export interface ToolDefinition {
  readonly name: string;
  readonly description: string;
  readonly inputSchema: JsonObject;
}

export interface ModelMessage {
  readonly role: "system" | "user" | "assistant" | "tool";
  readonly content: ContentBlock[];
  readonly toolCalls: ToolCall[];
  readonly toolCallId: string | null;
}

export interface ModelOptions {
  readonly delivery: "buffered";
  readonly timeoutSeconds: number;
  readonly maxOutputTokens: number | null;
  readonly temperature: number | null;
  readonly reasoningEffort: string | null;
  readonly outputSchema: null;
}

export interface ModelGenerateRequest {
  readonly messages: ModelMessage[];
  readonly tools: ToolDefinition[];
  readonly options: ModelOptions;
  readonly continuation: string | null;
}

export interface Usage {
  readonly inputTokens: number | null;
  readonly outputTokens: number | null;
  readonly totalTokens: number | null;
}

export interface ModelGenerateResult {
  readonly content: ContentBlock[];
  readonly toolCalls: ToolCall[];
  readonly finishReason: "stop" | "tool_calls" | "length" | "refusal" | "unknown";
  readonly usage: Usage | null;
  readonly continuation: string | null;
  readonly delivery: "buffered";
}

export interface HostCallContext {
  readonly sessionId: string;
  readonly turnId: string;
  readonly workflowRunId: string;
  readonly connectionId: string;
  readonly modelId: string;
  readonly remainingSeconds: number;
}

/** A transport call always resolves the whole reply; there is no delta callback here. */
export interface HostAiTransport {
  call(method: string, scope: Scope, params: JsonObject, timeoutSeconds: number, signal: AbortSignal): Promise<JsonObject>;
}

export function modelScope(context: HostCallContext): Scope {
  return { sessionId: context.sessionId, turnId: context.turnId, workflowRunId: context.workflowRunId };
}

function validateMessages(messages: readonly ModelMessage[], limits: Limits): void {
  if (messages.length === 0 || messages.length > limits.modelMessages) {
    throw invalidRequest("The model message list is empty or over the negotiated limit.", { field: "messages" });
  }
  for (const message of messages) {
    if (message.content.some((block) => utf8Bytes(block.text) > limits.textBytes)) {
      throw invalidRequest("A model text block exceeds textBytes.", { field: "messages" });
    }
  }
  if (messages.some((message) => message.role !== "assistant" && message.toolCalls.length > 0)) {
    throw invalidRequest("Only assistant messages may carry tool calls.", { field: "messages" });
  }
}

export function validateToolDefinitions(tools: readonly ToolDefinition[], limits: Limits): void {
  if (tools.length > limits.toolDefinitions) {
    throw invalidRequest("The tool definition list exceeds toolDefinitions.", { field: "tools" });
  }
  const names = new Set<string>();
  for (const tool of tools) {
    if (names.has(tool.name)) {
      throw invalidRequest("Tool names must be unique for one call.", { field: "tools" });
    }
    names.add(tool.name);
    if (utf8Bytes(JSON.stringify(tool.inputSchema)) > limits.toolJsonBytes) {
      throw invalidRequest("A tool schema exceeds toolJsonBytes.", { field: "tools" });
    }
  }
}

export function validateModelResult(value: unknown, limits: Limits): ModelGenerateResult {
  if (!isObject(value)) {
    throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned a non-object model result.");
  }
  if (value["delivery"] !== "buffered") {
    throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned a delivery this runtime did not negotiate.");
  }
  const content = value["content"];
  const toolCalls = value["toolCalls"];
  if (!Array.isArray(content) || !Array.isArray(toolCalls)) {
    throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned an invalid model result shape.");
  }
  if (content.length > 0 && content.some((block) => !isObject(block) || block["type"] !== "text" || typeof block["text"] !== "string")) {
    throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned non-text content in protocol 1.0.");
  }
  if (content.some((block) => typeof block["text"] === "string" && utf8Bytes(block["text"]) > limits.textBytes)) {
    throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned a text block over textBytes.");
  }
  if (toolCalls.length > limits.toolCalls) {
    throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned too many tool calls.");
  }
  const ids = new Set<string>();
  for (const call of toolCalls) {
    if (!isObject(call) || typeof call["id"] !== "string" || typeof call["name"] !== "string" || !isObject(call["arguments"])) {
      throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned an invalid tool call.");
    }
    if (ids.has(call["id"])) {
      throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned duplicate tool-call identities.");
    }
    ids.add(call["id"]);
  }
  const finishReason = value["finishReason"];
  if (finishReason !== "stop" && finishReason !== "tool_calls" && finishReason !== "length" && finishReason !== "refusal" && finishReason !== "unknown") {
    throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned an unknown finish reason.");
  }
  const continuation = value["continuation"];
  if (continuation !== null && typeof continuation !== "string") {
    throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned an invalid continuation handle.");
  }
  const usage = value["usage"];
  if (usage !== null && !isObject(usage)) {
    throw new ProtocolError("MALFORMED_RESPONSE", "HostAi returned an invalid usage value.");
  }
  return {
    content: content as ContentBlock[],
    toolCalls: toolCalls as ToolCall[],
    finishReason,
    usage: (usage as Usage | null) ?? null,
    continuation,
    delivery: "buffered",
  };
}

export class HostAiModelClient {
  readonly #transport: HostAiTransport;
  readonly #context: HostCallContext;
  readonly #limits: Limits;
  readonly #onCall: (() => void) | undefined;
  #calls = 0;

  constructor(transport: HostAiTransport, context: HostCallContext, limits: Limits = DEFAULT_LIMITS, onCall?: () => void) {
    this.#transport = transport;
    this.#context = context;
    this.#limits = limits;
    this.#onCall = onCall;
  }

  get calls(): number {
    return this.#calls;
  }

  async generate(request: ModelGenerateRequest, signal: AbortSignal): Promise<ModelGenerateResult> {
    if (request.options.delivery !== "buffered") {
      throw unsupportedCapability("model.generate.streaming");
    }
    if (request.options.outputSchema !== null) {
      throw unsupportedCapability("structuredOutput");
    }
    validateMessages(request.messages, this.#limits);
    validateToolDefinitions(request.tools, this.#limits);
    const timeoutSeconds = Math.min(request.options.timeoutSeconds, this.#context.remainingSeconds, this.#limits.callMaxSeconds);
    if (!Number.isInteger(timeoutSeconds) || timeoutSeconds < 1) {
      throw new ProtocolError("RESOURCE_EXHAUSTED", "No turn time remains for a model call.", "never", {
        resource: "wallTimeSeconds",
        limit: 0,
      });
    }
    this.#calls += 1;
    this.#onCall?.();
    const params: JsonObject = {
      connectionId: this.#context.connectionId,
      modelId: this.#context.modelId,
      messages: request.messages as unknown as JsonObject[],
      tools: request.tools as unknown as JsonObject[],
      options: {
        delivery: "buffered",
        timeoutSeconds,
        maxOutputTokens: request.options.maxOutputTokens,
        temperature: request.options.temperature,
        reasoningEffort: request.options.reasoningEffort,
        outputSchema: null,
      },
      continuation: request.continuation,
    };
    const result = await this.#transport.call("model.generate", modelScope(this.#context), params, timeoutSeconds, signal);
    return validateModelResult(result, this.#limits);
  }
}
