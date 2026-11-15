/**
 * Deterministic in-memory MCP peer used by the SolidCraft adapter tests.
 *
 * It speaks the same newline-delimited JSON-RPC 2.0 the real Rust server
 * speaks: `initialize`, `notifications/initialized`, `tools/list`,
 * `tools/call`, `ping`. Tool bodies mirror the real server's two result
 * carriers: plain text JSON (`text_result`) and `structuredContent` for
 * guarded replies. No test reaches a real app or a network.
 */

import type { JsonObject } from "../../src/protocol.js";
import type { McpBridgeChannel } from "../../src/solidcraft/mcp-client.js";

export interface CommandFixture {
  readonly id: string;
  readonly label?: string;
  readonly params?: string;
  readonly input: object | null;
  readonly output?: object | null;
}

export interface PeerTool {
  readonly name: string;
  readonly description?: string;
  readonly inputSchema: object;
  readonly readOnly?: boolean;
  readonly destructive?: boolean;
}

export interface ToolReply {
  /** Structured body; the peer adds it as `structuredContent` and pretty text. */
  readonly body?: JsonObject;
  /** Raw text content (used by the legacy `pending`/`execute` carriers). */
  readonly text?: string;
  readonly isError?: boolean;
}

export type ToolHandler = (args: JsonObject) => ToolReply | Promise<ToolReply>;

export interface RecordedRequest {
  readonly id: number | string | null;
  readonly method: string;
  readonly params: JsonObject;
}

export interface RecordedToolCall {
  readonly name: string;
  readonly args: JsonObject;
  readonly id: number | string | null;
  readonly sequence: number;
}

export interface FakePeerOptions {
  readonly tools?: readonly PeerTool[];
  readonly commands?: readonly CommandFixture[];
  readonly guarded?: boolean;
  readonly documentId?: string;
  readonly revision?: bigint;
}

export function deferred<T>(): { readonly promise: Promise<T>; readonly resolve: (value: T) => void } {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

export const STAMP_SCHEMA: object = {
  type: "object",
  additionalProperties: false,
  required: ["documentId", "revision"],
  properties: {
    documentId: { type: "string", minLength: 1, maxLength: 128, pattern: "^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$" },
    revision: { type: "string", pattern: "^(?:0|[1-9][0-9]{0,19})$" },
  },
};

function executeSchema(guarded: boolean): object {
  const properties: Record<string, unknown> = {
    command: { type: "string" },
    params: { type: "object" },
    verbose: { type: "boolean" },
    expect: {
      type: "object",
      additionalProperties: false,
      properties: {
        volume_mm3: { type: "number" },
        tol: { type: "number" },
        bodies: { type: "integer", minimum: 0 },
        bbox: {
          type: "object",
          additionalProperties: false,
          properties: {
            min: { type: "array", minItems: 3, maxItems: 3, items: { type: "number" } },
            max: { type: "array", minItems: 3, maxItems: 3, items: { type: "number" } },
          },
          required: ["min", "max"],
        },
      },
    },
  };
  if (guarded) {
    properties["expectedDocument"] = STAMP_SCHEMA;
  }
  return { type: "object", additionalProperties: false, required: ["command"], properties };
}

function batchSchema(guarded: boolean): object {
  const properties: Record<string, unknown> = {
    commands: {
      type: "array",
      minItems: 1,
      items: {
        type: "object",
        additionalProperties: false,
        required: ["command"],
        properties: { command: { type: "string" }, params: { type: "object" } },
      },
    },
    rollback: { type: "boolean" },
    verbose: { type: "boolean" },
    expect: { type: "object" },
  };
  if (guarded) {
    properties["expectedDocument"] = STAMP_SCHEMA;
  }
  return { type: "object", additionalProperties: false, required: ["commands"], properties };
}

export function defaultTools(guarded = true): PeerTool[] {
  return [
    { name: "execute", inputSchema: executeSchema(guarded) },
    { name: "batch", inputSchema: batchSchema(guarded) },
    {
      name: "describe_command",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["id"],
        properties: { id: { type: "string" } },
      },
      readOnly: true,
    },
    {
      name: "inspect_design",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        properties: { measure: { type: "boolean" }, sketches: { type: "boolean" } },
      },
      readOnly: true,
    },
    {
      name: "measure",
      inputSchema: { type: "object", additionalProperties: true },
      readOnly: true,
    },
    {
      name: "pending",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
      readOnly: true,
    },
    {
      name: "running",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
      readOnly: true,
    },
  ];
}

export function defaultCommands(): CommandFixture[] {
  return [
    { id: "document.revision", params: "→ document_id, revision", input: null },
    {
      id: "document.snapshot.capture",
      input: { type: "object", additionalProperties: false, properties: {} },
    },
    {
      id: "parameters.change",
      input: {
        type: "object",
        additionalProperties: false,
        required: ["name"],
        properties: { name: { type: "string" }, expression: { type: "string" } },
      },
    },
    {
      id: "solid.box",
      input: {
        type: "object",
        additionalProperties: false,
        required: ["length", "width", "height"],
        properties: {
          length: { type: "number" },
          width: { type: "number" },
          height: { type: "number" },
        },
      },
    },
    { id: "edit.undo", input: { type: "object", additionalProperties: false, properties: {} } },
    { id: "solid.unconverted", input: null },
    {
      id: "solid.mismatch",
      input: {
        type: "object",
        additionalProperties: false,
        required: ["radius"],
        properties: { radius: { type: "number" } },
      },
    },
  ];
}

/** A body pretty-printed the way the Rust `text_result` prints a value. */
export function pretty(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export function textReply(body: unknown, isError = false): ToolReply {
  return { text: typeof body === "string" ? body : pretty(body), isError };
}

export function guardedReply(body: JsonObject, isError = false): ToolReply {
  return { body, isError };
}

export class FakeMcpPeer implements McpBridgeChannel {
  readonly tools: PeerTool[];
  readonly commands = new Map<string, CommandFixture>();
  readonly requests: RecordedRequest[] = [];
  readonly toolCalls: RecordedToolCall[] = [];
  readonly handlers = new Map<string, ToolHandler>();
  documentId: string;
  revision: bigint;
  protocolVersion = "2025-06-18";
  maxConcurrentToolCalls = 0;
  closed = false;
  #inFlight = 0;
  #sequence = 0;
  #onLine: (line: string) => void = () => undefined;
  #onClose: (reason: string) => void = () => undefined;
  #closeReason: string | null = null;

  constructor(options: FakePeerOptions = {}) {
    this.tools = [...(options.tools ?? defaultTools(options.guarded ?? true))];
    for (const command of options.commands ?? defaultCommands()) {
      this.commands.set(command.id, command);
    }
    this.documentId = options.documentId ?? "doc-A";
    this.revision = options.revision ?? 41n;
    this.handlers.set("describe_command", (args) => this.#describe(args));
    this.handlers.set("pending", () => textReply({ pending: null, note: "nothing is running" }));
    this.handlers.set("running", () => textReply({ running: [] }));
    this.handlers.set("inspect_design", () => textReply({ name: "fixture", revision: Number(this.revision) }));
    this.handlers.set("measure", () => textReply({ body_count: 1, total: { volume_mm3: 1000 } }));
  }

  stamp(): { documentId: string; revision: string } {
    return { documentId: this.documentId, revision: this.revision.toString() };
  }

  advance(): { documentId: string; revision: string } {
    this.revision += 1n;
    return this.stamp();
  }

  setHandler(name: string, handler: ToolHandler): void {
    this.handlers.set(name, handler);
  }

  execute(handler: ToolHandler): void {
    this.setHandler("execute", handler);
  }

  batch(handler: ToolHandler): void {
    this.setHandler("batch", handler);
  }

  pending(handler: ToolHandler): void {
    this.setHandler("pending", handler);
  }

  toolCallsNamed(name: string): RecordedToolCall[] {
    return this.toolCalls.filter((call) => call.name === name);
  }

  /** Simulate the host bridge disappearing. */
  disconnect(reason = "simulated disconnect"): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.#closeReason = reason;
    this.#onClose(reason);
  }

  /** Inject an arbitrary inbound line (malformed-frame tests). */
  inject(line: string): void {
    this.#onLine(line);
  }

  send(line: string): void {
    if (this.closed) {
      throw new Error("peer is closed");
    }
    const message = JSON.parse(line) as JsonObject;
    const id = typeof message["id"] === "number" || typeof message["id"] === "string" ? message["id"] : null;
    const method = typeof message["method"] === "string" ? message["method"] : "";
    const params = message["params"] === undefined ? {} : (message["params"] as JsonObject);
    this.requests.push({ id, method, params });
    if (method.length === 0) {
      // A response to a server request; the fake peer sends none.
      return;
    }
    if (id === null) {
      return;
    }
    queueMicrotask(() => {
      void this.#dispatch(id, method, params);
    });
  }

  receive(onLine: (line: string) => void, onClose: (reason: string) => void): () => void {
    this.#onLine = onLine;
    this.#onClose = onClose;
    return () => {
      this.#onLine = () => undefined;
      this.#onClose = () => undefined;
    };
  }

  /** The channel's close, as the runtime calls it; no server-loss callback. */
  close(reason = "closed by the runtime"): void {
    this.closed = true;
    this.#closeReason = reason;
  }

  async #dispatch(id: number | string, method: string, params: JsonObject): Promise<void> {
    switch (method) {
      case "initialize":
        this.#respond(id, {
          protocolVersion: this.protocolVersion,
          capabilities: { tools: { listChanged: false } },
          serverInfo: { name: "solidcraft", version: "fixture" },
          instructions: "fixture",
        });
        return;
      case "tools/list":
        this.#respond(id, {
          tools: this.tools.map((tool) => ({
            name: tool.name,
            description: tool.description ?? `${tool.name} fixture`,
            inputSchema: tool.inputSchema,
            annotations: {
              title: tool.name,
              readOnlyHint: tool.readOnly ?? false,
              destructiveHint: tool.destructive ?? false,
              openWorldHint: false,
            },
          })),
        });
        return;
      case "ping":
        this.#respond(id, {});
        return;
      case "tools/call": {
        const name = typeof params["name"] === "string" ? params["name"] : "";
        const args = params["arguments"] === undefined ? {} : (params["arguments"] as JsonObject);
        this.#sequence += 1;
        this.toolCalls.push({ name, args, id, sequence: this.#sequence });
        this.#inFlight += 1;
        this.maxConcurrentToolCalls = Math.max(this.maxConcurrentToolCalls, this.#inFlight);
        try {
          const handler = this.handlers.get(name);
          if (handler === undefined) {
            this.#respondError(id, -32602, `unknown tool \`${name}\``);
            return;
          }
          let reply: ToolReply;
          try {
            reply = await handler(args);
          } catch (error) {
            this.#respondError(id, -32602, error instanceof Error ? error.message : "invalid arguments");
            return;
          }
          if (this.closed) {
            return;
          }
          if (reply.body !== undefined) {
            this.#respond(id, {
              content: [{ type: "text", text: reply.text ?? pretty(reply.body) }],
              structuredContent: reply.body,
              ...(reply.isError === true ? { isError: true } : {}),
            });
          } else {
            this.#respond(id, {
              content: [{ type: "text", text: reply.text ?? "" }],
              ...(reply.isError === true ? { isError: true } : {}),
            });
          }
        } finally {
          this.#inFlight -= 1;
        }
        return;
      }
      default:
        this.#respondError(id, -32601, `method \`${method}\` not found`);
    }
  }

  #describe(args: JsonObject): ToolReply {
    const id = typeof args["id"] === "string" ? args["id"] : "";
    const command = this.commands.get(id);
    if (command === undefined) {
      return textReply(`unknown command \`${id}\``, true);
    }
    return textReply({
      id: command.id,
      label: command.label ?? command.id,
      params: command.params ?? "fixture parameters",
      input: command.input,
      output: command.output ?? null,
    });
  }

  #respond(id: number | string, result: JsonObject): void {
    if (this.closed) {
      return;
    }
    this.#onLine(JSON.stringify({ jsonrpc: "2.0", id, result }));
  }

  #respondError(id: number | string, code: number, message: string): void {
    if (this.closed) {
      return;
    }
    this.#onLine(JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } }));
  }

  get closeReason(): string | null {
    return this.#closeReason;
  }
}
