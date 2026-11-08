/**
 * Client for the host-injected `solidcraft-app` MCP bridge.
 *
 * The bridge is provisioned by the desktop host for one chat turn. This module
 * speaks ordinary MCP JSON-RPC (newline-delimited) over the injected channel,
 * discovers the tool list and each command's schemas through the existing
 * interface, and preserves the turn's association. It never sends a turn id:
 * the host owns the trusted association and a tool argument cannot manufacture
 * another turn's authority.
 *
 * There is deliberately no command catalogue here. Tool definitions come from
 * `tools/list`; command parameters and results come from `describe_command`
 * (`engine.describe`). Numeric JSON-RPC transport ids are mapped bijectively to
 * protocol `Id` strings (`mcp-<n>`) so a `WriteOutcome` can retain the original
 * request identity when a `pending` query is made with another transport id.
 */

import { randomUUID } from "node:crypto";
import type { Readable, Writable } from "node:stream";

import { Ajv2020 } from "ajv/dist/2020.js";

import {
  ERROR_CODES,
  invalidRequest,
  ProtocolError,
  resourceExhausted,
  type ErrorCode,
} from "../errors.js";
import { isObject } from "../limits.js";
import { isId, type JsonObject } from "../protocol.js";
import { parseStrictJson, StrictJsonError } from "../strict-json.js";

/** MCP revision this client asks for first. */
export const MCP_PROTOCOL_VERSION = "2025-06-18";
/** Revisions whose `initialize`/`tools` lifecycle this client understands. */
export const MCP_SUPPORTED_VERSIONS: readonly string[] = Object.freeze([
  "2025-06-18",
  "2025-03-26",
  "2024-11-05",
]);
/** The `tools/list` + `tools/call` + `pending` surface must fit the server's own 16 MiB line. */
export const DEFAULT_MCP_FRAME_BYTES = 16 * 1024 * 1024;
/** Bounded response ledger, matching the protocol's cancelled-id ledger intent. */
export const MCP_REQUEST_LEDGER_ENTRIES = 4096;
const DEFAULT_MAX_DEPTH = 64;

/** The host provisions this opaque reference; the runtime never resolves it itself. */
export interface McpBridgeRef {
  readonly name: "solidcraft-app";
  readonly bridgeId: string;
}

/** The chat turn this bridge belongs to. Immutable for the connection's lifetime. */
export interface McpTurnAssociation {
  readonly sessionId: string;
  readonly turnId: string;
  readonly workflowRunId: string;
}

/**
 * The host-owned duplex channel to the bridge. One `send` call is one JSON-RPC
 * message; the client appends the framing LF. `receive` installs the inbound
 * listener and returns its unsubscribe function.
 */
export interface McpBridgeChannel {
  send(line: string): void;
  receive(onLine: (line: string) => void, onClose: (reason: string) => void): () => void;
  close(reason?: string): void;
}

/** A discovered MCP tool definition, including its honest effect annotations. */
export interface McpTool {
  readonly name: string;
  readonly description: string;
  readonly inputSchema: JsonObject;
  readonly readOnly: boolean;
  readonly destructive: boolean;
}

/** One normalized `tools/call` reply. `value` prefers `structuredContent`. */
export interface McpToolResult {
  readonly tool: string;
  readonly wireRequestId: string;
  readonly isError: boolean;
  readonly text: string | null;
  readonly structured: JsonObject | null;
  readonly value: JsonObject | null;
}

/** A `describe_command` reply: the authoritative command shape. */
export interface DiscoveredCommand {
  readonly id: string;
  readonly label: string;
  readonly paramsDoc: string;
  readonly inputSchema: JsonObject | null;
  readonly outputSchema: JsonObject | null;
}

/** A reserved JSON-RPC request identity and its protocol `Id` form. */
export interface McpReservedRequest {
  readonly jsonId: number;
  readonly wireId: string;
}

export interface McpClientOptions {
  readonly channel: McpBridgeChannel;
  readonly bridge: McpBridgeRef;
  readonly turn: McpTurnAssociation;
  /** Tests may pin the identity; production generates a random one. */
  readonly mcpSessionId?: string;
  readonly clientName?: string;
  readonly clientVersion?: string;
  readonly handshakeTimeoutMs?: number;
  readonly requestTimeoutMs?: number;
  readonly maxFrameBytes?: number;
  readonly diagnostic?: (line: string) => void;
}

export interface McpConnectionInfo {
  readonly protocolVersion: string;
  readonly serverName: string;
  readonly mcpSessionId: string;
  readonly tools: readonly McpTool[];
}

/** A JSON-RPC error reply from the bridge (an unknown tool or bad arguments). */
export class McpRpcError extends Error {
  readonly code: number;
  readonly data: unknown;

  constructor(code: number, message: string, data: unknown) {
    super(message);
    this.name = "McpRpcError";
    this.code = code;
    this.data = data;
  }
}

interface PendingCall {
  readonly method: string;
  readonly resolve: (value: JsonObject) => void;
  readonly reject: (error: Error) => void;
  readonly timer: NodeJS.Timeout;
}

/** Adapter from Node streams (a child process's pipes) to the channel shape. */
export interface StreamChannelOptions {
  /** Inbound line ceiling, enforced before a newline is seen. */
  readonly maxLineBytes?: number;
}

/**
 * Adapter from Node streams (a child process's pipes) to the channel shape.
 *
 * The input must be a byte stream (no `setEncoding`): complete lines are
 * decoded with `fatal: true`, so invalid UTF-8 closes the bridge instead of
 * becoming U+FFFD. A line longer than `maxLineBytes` is refused before it is
 * buffered in full.
 */
export function streamChannel(
  input: Readable,
  output: Writable,
  options: StreamChannelOptions = {},
): McpBridgeChannel {
  const maxLineBytes = options.maxLineBytes ?? DEFAULT_MCP_FRAME_BYTES;
  const lineListeners = new Set<(line: string) => void>();
  const closeListeners = new Set<(reason: string) => void>();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let closed = false;
  let buffered: Buffer = Buffer.alloc(0);

  const finish = (reason: string): void => {
    if (closed) {
      return;
    }
    closed = true;
    buffered = Buffer.alloc(0);
    for (const listener of [...closeListeners]) {
      listener(reason);
    }
    closeListeners.clear();
  };

  input.on("data", (chunk: Buffer | string) => {
    if (closed) {
      return;
    }
    const bytes = typeof chunk === "string" ? Buffer.from(chunk, "utf8") : chunk;
    if (buffered.length === 0 && bytes.indexOf(0x0a) === -1 && bytes.length > maxLineBytes) {
      finish(`the SolidCraft bridge sent more than ${maxLineBytes} bytes without a line break`);
      return;
    }
    buffered = buffered.length === 0 ? bytes : Buffer.concat([buffered, bytes]);
    for (;;) {
      if (closed) {
        return;
      }
      const newline = buffered.indexOf(0x0a);
      if (newline === -1) {
        if (buffered.length > maxLineBytes) {
          finish(`the SolidCraft bridge sent more than ${maxLineBytes} bytes without a line break`);
        }
        return;
      }
      if (newline > maxLineBytes) {
        finish(`the SolidCraft bridge sent a line longer than ${maxLineBytes} bytes`);
        return;
      }
      const lineBytes = buffered.subarray(0, newline);
      buffered = buffered.subarray(newline + 1);
      let line: string;
      try {
        line = decoder.decode(lineBytes);
      } catch {
        finish("the SolidCraft bridge sent invalid UTF-8");
        return;
      }
      line = line.replace(/\r$/u, "");
      if (line.length > 0) {
        for (const listener of [...lineListeners]) {
          listener(line);
        }
      }
    }
  });
  input.on("end", () => finish("the SolidCraft bridge closed its output"));
  input.on("error", (error: Error) => finish(`SolidCraft bridge input failed: ${error.message}`));
  output.on("error", (error: Error) => finish(`SolidCraft bridge output failed: ${error.message}`));

  return {
    send(line: string): void {
      if (closed) {
        throw new ProtocolError("TRANSPORT_ERROR", "The SolidCraft bridge is closed.");
      }
      output.write(`${line}\n`);
    },
    receive(onLine, onClose) {
      lineListeners.add(onLine);
      closeListeners.add(onClose);
      return () => {
        lineListeners.delete(onLine);
        closeListeners.delete(onClose);
      };
    },
    close(reason = "closed by the runtime"): void {
      finish(reason);
      input.pause();
      output.end();
    },
  };
}

function capabilityError(capability: string, message: string): ProtocolError {
  return new ProtocolError("UNSUPPORTED_CAPABILITY", message, "never", { capability });
}

function isReadOnly(tool: JsonObject): boolean {
  const annotations = tool["annotations"];
  return isObject(annotations) && annotations["readOnlyHint"] === true;
}

function isDestructive(tool: JsonObject): boolean {
  const annotations = tool["annotations"];
  return isObject(annotations) && annotations["destructiveHint"] === true;
}

/** Bounded, actionable text for one failed JSON Schema validation. */
function schemaFailureMessage(label: string, errors: readonly unknown[]): string {
  const parts: string[] = [];
  for (const error of errors.slice(0, 3)) {
    if (!isObject(error)) {
      continue;
    }
    const path = typeof error["instancePath"] === "string" && error["instancePath"].length > 0 ? error["instancePath"] : "(root)";
    const message = typeof error["message"] === "string" ? error["message"] : "is not valid";
    const missing = isObject(error["params"]) && typeof error["params"]["missingProperty"] === "string"
      ? `: ${error["params"]["missingProperty"]}`
      : "";
    parts.push(`${path} ${message}${missing}`);
  }
  return `${label} does not match its discovered schema${parts.length > 0 ? ` (${parts.join("; ")})` : ""}.`;
}

/** Keys that belong to the execution envelope, never to CAD parameters. */
const RESERVED_PARAMETER_KEYS: readonly string[] = Object.freeze([
  "expectedDocument",
  "documentStamp",
  "sessionId",
  "turnId",
  "workflowRunId",
  "bridgeId",
]);

export class McpClient {
  readonly #channel: McpBridgeChannel;
  readonly #bridge: McpBridgeRef;
  readonly #turn: McpTurnAssociation;
  readonly #diagnostic: (line: string) => void;
  readonly #handshakeTimeoutMs: number;
  readonly #requestTimeoutMs: number;
  readonly #maxFrameBytes: number;
  readonly #mcpSessionId: string;
  readonly #clientName: string;
  readonly #clientVersion: string;
  readonly #ajv: Ajv2020;
  readonly #pending = new Map<number, PendingCall>();
  readonly #tombstones = new Set<number>();
  readonly #tools = new Map<string, McpTool>();
  readonly #commands = new Map<string, DiscoveredCommand>();
  readonly #commandValidators = new Map<string, CompiledValidator | null>();
  readonly #toolValidators = new Map<string, CompiledValidator | null>();
  #nextRequestId = 0;
  #unsubscribe: (() => void) | null = null;
  #connected = false;
  #closed = false;
  #closeReason: string | null = null;
  #protocolVersion: string | null = null;
  #serverName = "unknown";

  constructor(options: McpClientOptions) {
    const { bridge, turn } = options;
    if (bridge.name !== "solidcraft-app" || !isId(bridge.bridgeId)) {
      throw invalidRequest("The SolidCraft bridge reference is invalid.", { field: "bridge" });
    }
    if (!isId(turn.sessionId) || !isId(turn.turnId) || !isId(turn.workflowRunId)) {
      throw invalidRequest("The chat turn association is invalid.", { field: "turn" });
    }
    const sessionId = options.mcpSessionId ?? `mcp-session-${randomUUID()}`;
    if (!isId(sessionId)) {
      throw invalidRequest("The MCP session identity is invalid.", { field: "mcpSessionId" });
    }
    this.#channel = options.channel;
    this.#bridge = { name: "solidcraft-app", bridgeId: bridge.bridgeId };
    this.#turn = { sessionId: turn.sessionId, turnId: turn.turnId, workflowRunId: turn.workflowRunId };
    this.#mcpSessionId = sessionId;
    this.#diagnostic = options.diagnostic ?? (() => undefined);
    this.#handshakeTimeoutMs = options.handshakeTimeoutMs ?? 10_000;
    this.#requestTimeoutMs = options.requestTimeoutMs ?? 300_000;
    this.#maxFrameBytes = options.maxFrameBytes ?? DEFAULT_MCP_FRAME_BYTES;
    this.#clientName = options.clientName ?? "solidcraft-mastra-runtime";
    this.#clientVersion = options.clientVersion ?? "0.1.0";
    this.#ajv = new Ajv2020({ strict: false, allErrors: true, validateFormats: false });
    this.#unsubscribe = this.#channel.receive(
      (line) => this.#onLine(line),
      (reason) => this.#fail(reason),
    );
  }

  get bridgeId(): string {
    return this.#bridge.bridgeId;
  }

  get mcpSessionId(): string {
    return this.#mcpSessionId;
  }

  get association(): McpTurnAssociation {
    return this.#turn;
  }

  get serverName(): string {
    return this.#serverName;
  }

  get protocolVersion(): string | null {
    return this.#protocolVersion;
  }

  get isClosed(): boolean {
    return this.#closed;
  }

  /** Perform the MCP handshake and discover the tool list. */
  async connect(): Promise<McpConnectionInfo> {
    this.#assertOpen();
    if (this.#connected) {
      return {
        protocolVersion: this.#protocolVersion ?? MCP_PROTOCOL_VERSION,
        serverName: this.#serverName,
        mcpSessionId: this.#mcpSessionId,
        tools: [...this.#tools.values()],
      };
    }
    this.#assertOpen();
    let initialized: JsonObject;
    try {
      initialized = await this.#request(
        "initialize",
        {
          protocolVersion: MCP_PROTOCOL_VERSION,
          capabilities: {},
          clientInfo: { name: this.#clientName, version: this.#clientVersion },
        },
        this.#handshakeTimeoutMs,
      );
    } catch (error) {
      this.close("the MCP handshake failed");
      throw error;
    }
    const version = initialized["protocolVersion"];
    if (typeof version !== "string" || !MCP_SUPPORTED_VERSIONS.includes(version)) {
      const shown = typeof version === "string" ? version : "missing";
      this.close("unsupported MCP version");
      throw capabilityError(`mcp-protocol:${shown}`, `The SolidCraft bridge negotiated an unsupported MCP version (${shown}).`);
    }
    const serverInfo = initialized["serverInfo"];
    if (isObject(serverInfo) && typeof serverInfo["name"] === "string") {
      this.#serverName = serverInfo["name"].slice(0, 128);
    }
    this.#protocolVersion = version;
    this.#channel.send(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized", params: {} }));
    let listed: JsonObject;
    try {
      listed = await this.#request("tools/list", {}, this.#handshakeTimeoutMs);
    } catch (error) {
      this.close("the MCP tool discovery failed");
      throw error;
    }
    const tools = listed["tools"];
    if (!Array.isArray(tools)) {
      this.#fail("tools/list did not return a tools array");
      throw new ProtocolError("TRANSPORT_ERROR", "The SolidCraft bridge returned an invalid tool list.");
    }
    const seen = new Set<string>();
    for (const entry of tools) {
      if (!isObject(entry) || typeof entry["name"] !== "string" || entry["name"].length === 0 || !isObject(entry["inputSchema"])) {
        this.#fail("tools/list returned a malformed tool definition");
        throw new ProtocolError("TRANSPORT_ERROR", "The SolidCraft bridge returned a malformed tool definition.");
      }
      const name = entry["name"];
      if (seen.has(name)) {
        this.#fail("tools/list repeated a tool name");
        throw new ProtocolError("TRANSPORT_ERROR", "The SolidCraft bridge repeated a tool name.");
      }
      seen.add(name);
      const description = typeof entry["description"] === "string" ? entry["description"] : "";
      this.#tools.set(name, {
        name,
        description: description.slice(0, 2048),
        inputSchema: entry["inputSchema"],
        readOnly: isReadOnly(entry),
        destructive: isDestructive(entry),
      });
    }
    this.#connected = true;
    this.#diagnostic(`mcp: connected to ${this.#serverName} (${version}) with ${this.#tools.size} tools`);
    return {
      protocolVersion: version,
      serverName: this.#serverName,
      mcpSessionId: this.#mcpSessionId,
      tools: [...this.#tools.values()],
    };
  }

  /** Tools discovered through `tools/list` (empty before `connect`). */
  tools(): readonly McpTool[] {
    return [...this.#tools.values()];
  }

  tool(name: string): McpTool | null {
    return this.#tools.get(name) ?? null;
  }

  hasTool(name: string): boolean {
    return this.#tools.has(name);
  }

  /** Reserve an id even when a request is refused before it is sent. */
  reserveRequest(): McpReservedRequest {
    if (this.#nextRequestId >= MCP_REQUEST_LEDGER_ENTRIES) {
      throw resourceExhausted("requestLedgerEntries", MCP_REQUEST_LEDGER_ENTRIES);
    }
    this.#nextRequestId += 1;
    const jsonId = this.#nextRequestId;
    return { jsonId, wireId: `mcp-${jsonId}` };
  }

  /** The protocol `Id` for a JSON-RPC transport id (bijective for this connection). */
  wireIdFor(jsonId: number): string | null {
    return Number.isSafeInteger(jsonId) && jsonId >= 1 && jsonId <= this.#nextRequestId ? `mcp-${jsonId}` : null;
  }

  /** Call one discovered tool. A JSON-RPC error is thrown; `isError` is returned. */
  async callTool(
    name: string,
    args: JsonObject,
    options: { readonly reservation?: McpReservedRequest; readonly timeoutMs?: number } = {},
  ): Promise<McpToolResult> {
    if (!this.#tools.has(name)) {
      throw capabilityError(`mcp-tool:${name}`, `The SolidCraft bridge does not expose the \`${name}\` tool.`);
    }
    const reservation = options.reservation ?? this.reserveRequest();
    const result = await this.#request(
      "tools/call",
      { name, arguments: args },
      options.timeoutMs ?? this.#requestTimeoutMs,
      reservation,
    );
    return this.#normalizeToolResult(name, reservation.wireId, result);
  }

  /** A command already discovered on this connection, without a new call. */
  cachedCommand(id: string): DiscoveredCommand | null {
    return this.#commands.get(id) ?? null;
  }

  /** `describe_command`: the bridge's authoritative parameter/result schemas. */
  async describeCommand(id: string): Promise<DiscoveredCommand> {
    const cached = this.#commands.get(id);
    if (cached !== undefined) {
      return cached;
    }
    let reply: McpToolResult;
    try {
      reply = await this.callTool("describe_command", { id });
    } catch (error) {
      if (error instanceof ProtocolError) {
        throw error;
      }
      throw capabilityError(`command:${id}`, `The SolidCraft bridge could not describe command \`${id}\`.`);
    }
    if (reply.isError) {
      const detail = reply.text !== null ? reply.text.slice(0, 240) : "it is not known to this bridge";
      throw capabilityError(`command:${id}`, `The SolidCraft bridge cannot describe command \`${id}\`: ${detail}`);
    }
    const value = reply.value;
    if (!isObject(value) || value["id"] !== id || typeof value["label"] !== "string" || typeof value["params"] !== "string") {
      throw capabilityError(`command-schema:${id}`, `The SolidCraft bridge returned a malformed description for command \`${id}\`.`);
    }
    const input = value["input"];
    const output = value["output"];
    if ((input !== null && !isObject(input)) || (output !== null && !isObject(output))) {
      throw capabilityError(`command-schema:${id}`, `The SolidCraft bridge returned an unusable schema for command \`${id}\`.`);
    }
    const command: DiscoveredCommand = {
      id,
      label: value["label"],
      paramsDoc: value["params"].slice(0, 4096),
      inputSchema: input === null ? null : (input as JsonObject),
      outputSchema: output === null ? null : (output as JsonObject),
    };
    this.#commands.set(id, command);
    return command;
  }

  /**
   * Validate arguments against the discovered tool schema (`tools/list` is
   * authoritative; this is not a second copy of any command catalogue).
   */
  assertToolArguments(name: string, args: JsonObject): void {
    const tool = this.#tools.get(name);
    if (tool === undefined) {
      throw capabilityError(`mcp-tool:${name}`, `The SolidCraft bridge does not expose the \`${name}\` tool.`);
    }
    const validator = this.#validatorFor(`tool:${name}`, tool.inputSchema, this.#toolValidators);
    if (validator === null) {
      throw capabilityError(`tool-schema:${name}`, `The SolidCraft bridge's \`${name}\` tool schema cannot be used for validation.`);
    }
    const checked = validator.check(args);
    if (!checked.ok) {
      throw invalidRequest(schemaFailureMessage(`Arguments for \`${name}\``, checked.errors), {
        field: "arguments",
      });
    }
  }

  /**
   * Validate CAD parameters against `describe_command`'s input schema. A
   * missing schema permits only an exact read with no arguments, or is refused
   * for a write: arguments are never guessed.
   */
  assertCommandArguments(
    command: DiscoveredCommand,
    params: JsonObject,
    options: { readonly allowMissingSchema: boolean },
  ): void {
    if (!isObject(params)) {
      throw invalidRequest("Command parameters must be an object.", { field: "params" });
    }
    for (const key of Object.keys(params)) {
      if (RESERVED_PARAMETER_KEYS.includes(key)) {
        throw invalidRequest(
          `\`${key}\` belongs to the execution envelope, not to command parameters.`,
          { field: "params" },
        );
      }
    }
    if (command.inputSchema === null) {
      if (options.allowMissingSchema && Object.keys(params).length === 0) {
        return;
      }
      throw capabilityError(
        `command-schema:${command.id}`,
        `The SolidCraft bridge describes no input schema for command \`${command.id}\`; its arguments cannot be guessed.`,
      );
    }
    const validator = this.#validatorFor(`command:${command.id}`, command.inputSchema, this.#commandValidators);
    if (validator === null) {
      throw capabilityError(
        `command-schema:${command.id}`,
        `The SolidCraft bridge's schema for command \`${command.id}\` cannot be used for validation.`,
      );
    }
    const checked = validator.check(params);
    if (!checked.ok) {
      throw invalidRequest(schemaFailureMessage(`Parameters for \`${command.id}\``, checked.errors), {
        field: "params",
      });
    }
  }

  /** The bridge's `pending` tool, on this same connection (never a new one). */
  async pending(timeoutMs: number = 60_000): Promise<McpToolResult> {
    return this.callTool("pending", {}, { timeoutMs });
  }

  /** Diagnostic `running` query; it cannot prove a lost request's result. */
  async running(timeoutMs: number = 30_000): Promise<McpToolResult> {
    return this.callTool("running", {}, { timeoutMs });
  }

  /** Close the connection. Outstanding calls fail with TRANSPORT_ERROR. */
  close(reason: string = "closed by the runtime"): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#closeReason = reason;
    this.#unsubscribe?.();
    this.#unsubscribe = null;
    for (const [jsonId, pending] of this.#pending) {
      this.#pending.delete(jsonId);
      clearTimeout(pending.timer);
      pending.reject(new ProtocolError("TRANSPORT_ERROR", `The SolidCraft bridge closed: ${reason}`));
    }
    try {
      this.#channel.close(reason);
    } catch {
      // The host may already have torn the channel down.
    }
  }

  #assertOpen(): void {
    if (this.#closed) {
      throw new ProtocolError(
        "TRANSPORT_ERROR",
        `The SolidCraft bridge is closed${this.#closeReason === null ? "" : `: ${this.#closeReason}`}`,
      );
    }
  }

  #fail(reason: string): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#closeReason = reason;
    this.#unsubscribe?.();
    this.#unsubscribe = null;
    for (const [jsonId, pending] of this.#pending) {
      this.#pending.delete(jsonId);
      clearTimeout(pending.timer);
      pending.reject(new ProtocolError("TRANSPORT_ERROR", `The SolidCraft bridge was lost: ${reason}`));
    }
  }

  #request(
    method: string,
    params: JsonObject,
    timeoutMs: number,
    reservation?: McpReservedRequest,
  ): Promise<JsonObject> {
    this.#assertOpen();
    const reserved = reservation ?? this.reserveRequest();
    if (this.#pending.has(reserved.jsonId)) {
      throw new ProtocolError("INTERNAL_ERROR", "An MCP request identity was reused.");
    }
    return new Promise<JsonObject>((resolve, reject) => {
      const timer = setTimeout(() => {
        if (!this.#pending.delete(reserved.jsonId)) {
          return;
        }
        this.#rememberTombstone(reserved.jsonId);
        reject(
          new ProtocolError("TIMEOUT", `The SolidCraft bridge did not answer \`${method}\`.`, "same_submission", {
            operation: method,
          }),
        );
      }, timeoutMs);
      this.#pending.set(reserved.jsonId, { method, resolve, reject, timer });
      try {
        this.#channel.send(JSON.stringify({ jsonrpc: "2.0", id: reserved.jsonId, method, params }));
      } catch (error) {
        this.#pending.delete(reserved.jsonId);
        clearTimeout(timer);
        reject(error instanceof Error ? error : new ProtocolError("TRANSPORT_ERROR", "The SolidCraft bridge refused the request."));
      }
    });
  }

  #rememberTombstone(jsonId: number): void {
    this.#tombstones.add(jsonId);
    if (this.#tombstones.size > MCP_REQUEST_LEDGER_ENTRIES) {
      this.#fail("the cancelled-request ledger is exhausted");
    }
  }

  #onLine(line: string): void {
    if (this.#closed) {
      return;
    }
    if (Buffer.byteLength(line, "utf8") > this.#maxFrameBytes) {
      this.#fail("the SolidCraft bridge sent an oversized frame");
      return;
    }
    let value: unknown;
    try {
      value = parseStrictJson(line, DEFAULT_MAX_DEPTH);
    } catch (error) {
      this.#fail(error instanceof StrictJsonError ? error.message : "the SolidCraft bridge sent invalid JSON");
      return;
    }
    if (!isObject(value)) {
      this.#fail("the SolidCraft bridge sent a non-object frame");
      return;
    }
    if (typeof value["method"] === "string") {
      // The SolidCraft server sends no requests today. Answer `ping` and refuse
      // anything else without inventing a capability.
      const id = value["id"];
      if (id === undefined || id === null) {
        return;
      }
      if (value["method"] === "ping") {
        this.#channel.send(JSON.stringify({ jsonrpc: "2.0", id, result: {} }));
      } else {
        this.#channel.send(
          JSON.stringify({ jsonrpc: "2.0", id, error: { code: -32601, message: "method not found" } }),
        );
      }
      return;
    }
    const id = value["id"];
    if (typeof id !== "number" || !Number.isSafeInteger(id)) {
      this.#fail("the SolidCraft bridge sent a response without a usable id");
      return;
    }
    const pending = this.#pending.get(id);
    if (pending === undefined) {
      if (this.#tombstones.has(id)) {
        return;
      }
      this.#fail("the SolidCraft bridge answered an unknown request id");
      return;
    }
    this.#pending.delete(id);
    clearTimeout(pending.timer);
    const failure = value["error"];
    if (isObject(failure)) {
      const code = typeof failure["code"] === "number" ? failure["code"] : -32603;
      const message = typeof failure["message"] === "string" ? failure["message"] : "the SolidCraft bridge refused the call";
      pending.reject(new McpRpcError(code, message.slice(0, 2048), failure["data"]));
      return;
    }
    const result = value["result"];
    if (!isObject(result)) {
      pending.reject(new ProtocolError("TRANSPORT_ERROR", "The SolidCraft bridge returned a non-object result."));
      return;
    }
    pending.resolve(result);
  }

  #normalizeToolResult(tool: string, wireRequestId: string, result: JsonObject): McpToolResult {
    const structured = isObject(result["structuredContent"]) ? result["structuredContent"] : null;
    let text: string | null = null;
    const content = result["content"];
    if (Array.isArray(content)) {
      for (const block of content) {
        if (isObject(block) && block["type"] === "text" && typeof block["text"] === "string") {
          text = block["text"];
          break;
        }
      }
    }
    let value: JsonObject | null = structured;
    if (value === null && text !== null) {
      try {
        const parsed = parseStrictJson(text, DEFAULT_MAX_DEPTH);
        value = isObject(parsed) ? parsed : null;
      } catch {
        value = null;
      }
    }
    return {
      tool,
      wireRequestId,
      isError: result["isError"] === true,
      text,
      structured,
      value,
    };
  }

  #validatorFor(
    key: string,
    schema: JsonObject,
    cache: Map<string, CompiledValidator | null>,
  ): CompiledValidator | null {
    const cached = cache.get(key);
    if (cached !== undefined) {
      return cached;
    }
    try {
      const compiled = this.#ajv.compile({ ...schema });
      const validator: CompiledValidator = {
        check(value: unknown) {
          const ok = compiled(value) === true;
          return { ok, errors: ok ? [] : (compiled.errors ?? []) };
        },
      };
      cache.set(key, validator);
      return validator;
    } catch {
      cache.set(key, null);
      return null;
    }
  }
}

interface CompiledValidator {
  check(value: unknown): { readonly ok: boolean; readonly errors: readonly unknown[] };
}

/** True when a wire error code is one this runtime understands. */
export function isKnownErrorCode(code: string): code is ErrorCode {
  return (ERROR_CODES as readonly string[]).includes(code);
}
