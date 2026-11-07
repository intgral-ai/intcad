/**
 * The Mastra language-model seam backed by SolidCraft HostAi.
 *
 * One `doGenerate` call becomes exactly one `model.generate` request. The
 * adapter never retries, never executes returned tools and never asks for
 * incremental delivery: the host's HTTP interface is buffered in 1.0.
 */

import { Agent, type MastraLegacyLanguageModel, type ToolsInput } from "@mastra/core/agent";

import { invalidRequest, unsupportedCapability } from "../errors.js";
import { DEFAULT_LIMITS, isObject, type Limits } from "../limits.js";
import { canonicalize } from "../jcs.js";
import type { JsonObject } from "../protocol.js";
import { utf8Bytes } from "../protocol.js";
import {
  HostAiModelClient,
  type ContentBlock,
  type ModelGenerateRequest,
  type ModelMessage,
  type ToolCall,
  type ToolDefinition,
  type Usage,
} from "./model-client.js";

interface PromptPart {
  readonly type?: string;
  readonly text?: string;
  readonly toolCallId?: string;
  readonly toolName?: string;
  readonly args?: unknown;
  readonly result?: unknown;
  readonly image?: unknown;
  readonly data?: unknown;
}

interface PromptMessage {
  readonly role?: string;
  readonly content?: string | readonly PromptPart[];
}

interface V1Tool {
  readonly type?: string;
  readonly name?: string;
  readonly description?: string;
  readonly parameters?: unknown;
}

interface V1Mode {
  readonly type?: string;
  readonly tools?: readonly V1Tool[];
}

interface V1Options {
  readonly prompt?: readonly PromptMessage[];
  readonly mode?: V1Mode;
  readonly maxTokens?: number;
  readonly temperature?: number;
  readonly responseFormat?: { readonly type?: string };
  readonly abortSignal?: AbortSignal;
}

function textBlock(text: string): ContentBlock {
  return { type: "text", text };
}

function parseToolArguments(args: unknown): JsonObject {
  if (isObject(args)) {
    return args;
  }
  if (typeof args !== "string" || args.length === 0) {
    return {};
  }
  try {
    const value: unknown = JSON.parse(args);
    if (isObject(value)) {
      return value;
    }
  } catch {
    // The caller supplied a malformed provider argument string.
  }
  throw invalidRequest("A tool call argument string is not a JSON object.");
}

function promptMessages(prompt: readonly PromptMessage[]): ModelMessage[] {
  const messages: ModelMessage[] = [];
  for (const message of prompt) {
    const role = message.role;
    if (role === "system") {
      const text = typeof message.content === "string" ? message.content : "";
      messages.push({ role, content: [textBlock(text)], toolCalls: [], toolCallId: null });
      continue;
    }
    const parts = Array.isArray(message.content) ? message.content : [];
    if (role === "user") {
      const content: ContentBlock[] = [];
      for (const part of parts) {
        if (part.type === "text" && typeof part.text === "string") {
          content.push(textBlock(part.text));
        } else if (part.type === "image") {
          throw unsupportedCapability("images");
        } else if (part.type === "file") {
          throw unsupportedCapability("documents");
        }
      }
      messages.push({ role, content, toolCalls: [], toolCallId: null });
      continue;
    }
    if (role === "assistant") {
      const content: ContentBlock[] = [];
      const toolCalls: ToolCall[] = [];
      for (const part of parts) {
        if (part.type === "text" && typeof part.text === "string") {
          content.push(textBlock(part.text));
        } else if (part.type === "tool-call" && typeof part.toolCallId === "string" && typeof part.toolName === "string") {
          toolCalls.push({ id: part.toolCallId, name: part.toolName, arguments: parseToolArguments(part.args) });
        }
      }
      messages.push({ role, content, toolCalls, toolCallId: null });
      continue;
    }
    if (role === "tool") {
      for (const part of parts) {
        if (part.type === "tool-result" && typeof part.toolCallId === "string") {
          const result = part.result === undefined ? null : part.result;
          messages.push({
            role: "tool",
            content: [textBlock(typeof result === "string" ? result : JSON.stringify(result))],
            toolCalls: [],
            toolCallId: part.toolCallId,
          });
        }
      }
      continue;
    }
    throw invalidRequest("Mastra produced a prompt role this runtime cannot translate.");
  }
  return messages;
}

function toolDefinitions(mode: V1Mode | undefined): ToolDefinition[] {
  if (mode?.type !== undefined && mode.type !== "regular") {
    throw unsupportedCapability("structuredOutput");
  }
  const definitions: ToolDefinition[] = [];
  for (const tool of mode?.tools ?? []) {
    if (tool.type !== "function" || typeof tool.name !== "string") {
      throw unsupportedCapability("provider-defined-tools");
    }
    if (!isObject(tool.parameters)) {
      throw invalidRequest("A Mastra tool has a non-object JSON Schema.");
    }
    // The AI SDK adds its dialect marker while normalizing a function tool. The
    // host's discovered schema is the authority; the marker is not part of it.
    const { $schema: _dialect, ...inputSchema } = tool.parameters;
    definitions.push({
      name: tool.name,
      description: typeof tool.description === "string" ? tool.description : "",
      inputSchema,
    });
  }
  return definitions;
}

/**
 * Continuation handles belong to the runtime and are scoped to the normalized
 * history branch that produced them. A call after a mismatch restarts with
 * `continuation: null` instead of replaying provider-private state.
 */
export class ContinuationBranch {
  #handle: string | null = null;
  #history: readonly string[] = [];
  #expiresAt = 0;
  readonly #ttlMillis: number;

  constructor(ttlSeconds: number = DEFAULT_LIMITS.continuationTtlSeconds) {
    this.#ttlMillis = ttlSeconds * 1000;
  }

  take(messages: readonly ModelMessage[], now = Date.now()): string | null {
    if (this.#handle === null) {
      return null;
    }
    if (now >= this.#expiresAt) {
      this.clear();
      return null;
    }
    const current = messages.map((message) => canonicalize(message));
    if (current.length < this.#history.length || this.#history.some((value, index) => current[index] !== value)) {
      this.clear();
      return null;
    }
    return this.#handle;
  }

  store(handle: string | null, messagesBeforeCall: readonly ModelMessage[], now = Date.now()): void {
    if (handle === null) {
      this.clear();
      return;
    }
    this.#handle = handle;
    this.#history = messagesBeforeCall.map((message) => canonicalize(message));
    this.#expiresAt = now + this.#ttlMillis;
  }

  clear(): void {
    this.#handle = null;
    this.#history = [];
    this.#expiresAt = 0;
  }

  get handle(): string | null {
    return this.#handle;
  }
}

export interface HostAiModelConfig {
  readonly client: HostAiModelClient;
  readonly connectionId: string;
  readonly modelId: string;
  readonly timeoutSeconds: number;
  readonly maxOutputTokens?: number | null;
  readonly temperature?: number | null;
  readonly reasoningEffort?: string | null;
  readonly signal?: AbortSignal;
  readonly limits?: Limits;
  readonly branch?: ContinuationBranch;
}

function mastraFinishReason(reason: "stop" | "tool_calls" | "length" | "refusal" | "unknown"): "stop" | "tool-calls" | "length" | "content-filter" | "unknown" {
  switch (reason) {
    case "stop":
      return "stop";
    case "tool_calls":
      return "tool-calls";
    case "length":
      return "length";
    case "refusal":
      return "content-filter";
    case "unknown":
      return "unknown";
  }
}

function mastraUsage(usage: Usage | null): { promptTokens: number; completionTokens: number } {
  return {
    promptTokens: usage?.inputTokens ?? 0,
    completionTokens: usage?.outputTokens ?? 0,
  };
}

/**
 * Convert normalized protocol messages into the AI SDK v4 message shape that
 * `generateLegacy` consumes. This is a boundary translation only: tool ids and
 * arguments are preserved, and a tool result must name its preceding call.
 */
export function toMastraMessages(messages: readonly ModelMessage[]): unknown[] {
  const toolNames = new Map<string, string>();
  for (const message of messages) {
    for (const call of message.toolCalls) {
      toolNames.set(call.id, call.name);
    }
  }
  return messages.map((message) => {
    if (message.role === "system") {
      return { role: "system", content: message.content.map((block) => block.text).join("\n") };
    }
    if (message.role === "user") {
      return { role: "user", content: message.content.map((block) => ({ type: "text", text: block.text })) };
    }
    if (message.role === "assistant") {
      const content: unknown[] = message.content.map((block) => ({ type: "text", text: block.text }));
      for (const call of message.toolCalls) {
        content.push({
          type: "tool-call",
          toolCallId: call.id,
          toolName: call.name,
          args: call.arguments,
        });
      }
      return { role: "assistant", content };
    }
    const text = message.content.map((block) => block.text).join("");
    let result: unknown = text;
    try {
      result = JSON.parse(text);
    } catch {
      // A plain-text tool result stays a string.
    }
    return {
      role: "tool",
      content: [
        {
          type: "tool-result",
          toolCallId: message.toolCallId,
          toolName: message.toolCallId === null ? "tool" : (toolNames.get(message.toolCallId) ?? "tool"),
          result,
        },
      ],
    };
  });
}

export function createHostAiLanguageModel(config: HostAiModelConfig): MastraLegacyLanguageModel {
  const limits = config.limits ?? DEFAULT_LIMITS;
  const branch = config.branch ?? new ContinuationBranch(limits.continuationTtlSeconds);
  const model: MastraLegacyLanguageModel = {
    specificationVersion: "v1",
    provider: "solidcraft-hostai",
    modelId: config.modelId,
    defaultObjectGenerationMode: undefined,
    supportsImageUrls: false,
    supportsStructuredOutputs: false,
    async doGenerate(rawOptions: Parameters<MastraLegacyLanguageModel["doGenerate"]>[0]) {
      const options = rawOptions as unknown as V1Options;
      if (options.responseFormat?.type === "json") {
        throw unsupportedCapability("structuredOutput");
      }
      const messages = promptMessages(options.prompt ?? []);
      const tools = toolDefinitions(options.mode);
      const continuation = branch.take(messages);
      const request: ModelGenerateRequest = {
        messages,
        tools,
        options: {
          delivery: "buffered",
          timeoutSeconds: config.timeoutSeconds,
          maxOutputTokens: config.maxOutputTokens ?? null,
          temperature: config.temperature ?? null,
          reasoningEffort: config.reasoningEffort ?? null,
          outputSchema: null,
        },
        continuation,
      };
      const signal = options.abortSignal ?? config.signal ?? new AbortController().signal;
      const result = await config.client.generate(request, signal);
      branch.store(result.continuation, messages);
      const text = result.content.map((block) => block.text).join("");
      return {
        ...(text.length === 0 ? {} : { text }),
        toolCalls: result.toolCalls.map((call) => ({
          toolCallType: "function" as const,
          toolCallId: call.id,
          toolName: call.name,
          args: JSON.stringify(call.arguments),
        })),
        finishReason: mastraFinishReason(result.finishReason),
        usage: mastraUsage(result.usage),
        rawCall: { rawPrompt: options.prompt ?? [], rawSettings: { delivery: "buffered" } },
        rawResponse: { headers: {} },
      };
    },
    async doStream(): Promise<never> {
      throw unsupportedCapability("model.generate.streaming");
    },
  };
  return model;
}

export interface HostAiAgentConfig extends HostAiModelConfig {
  readonly id?: string;
  readonly name?: string;
  readonly instructions: string;
  readonly tools: readonly ToolDefinition[];
}

/**
 * Create an actual Mastra Agent whose only capability is one HostAi model call.
 * Tool declarations carry no `execute`: a returned tool call is a proposal for
 * the outer workflow, never a hidden CAD action.
 */
export function createHostAiAgent(config: HostAiAgentConfig): Agent {
  const model = createHostAiLanguageModel(config);
  const tools: Record<string, { description: string; parameters: JsonObject }> = {};
  for (const definition of config.tools) {
    tools[definition.name] = { description: definition.description, parameters: definition.inputSchema };
  }
  return new Agent({
    id: config.id ?? "solidcraft-native-modeling",
    name: config.name ?? "SolidCraft native modeling",
    instructions: config.instructions,
    model,
    maxRetries: 0,
    tools: tools as unknown as ToolsInput,
  });
}

/** Bound the model-call text before a Mastra prompt can grow without limit. */
export function messagesWithinLimits(messages: readonly ModelMessage[], limits: Limits = DEFAULT_LIMITS): boolean {
  return (
    messages.length <= limits.modelMessages &&
    messages.every((message) => message.content.every((block) => utf8Bytes(block.text) <= limits.textBytes))
  );
}
