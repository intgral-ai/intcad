/**
 * Guarded native CAD execution over the injected `solidcraft-app` MCP bridge.
 *
 * Responsibilities (contract sections 1.5 and 7):
 * - serialize mutations for one turn (exactly one unresolved mutation);
 * - bound read concurrency (at most four) and serialize reads behind a mutation;
 * - bind every write to the intended `DocumentStamp` in the execution envelope;
 * - normalize the real bridge facts into `WriteOutcome`, never guessing;
 * - reconcile a same-connection `pending` request without resending it;
 * - report `unknown` after a disconnect, and admit no further writes;
 * - stop scheduling new writes after cancellation;
 * - refuse a speculative candidate probe with `UNSUPPORTED_CAPABILITY` and
 *   zero writes (editing then undoing the live design is forbidden).
 */

import {
  cancelled,
  ProtocolError,
  redactedMessage,
  resourceExhausted,
  type ErrorCode,
  type RetryAdvice,
  type WireErrorValue,
} from "../errors.js";
import { isObject } from "../limits.js";
import { isId, type JsonObject } from "../protocol.js";
import {
  isKnownErrorCode,
  McpRpcError,
  type DiscoveredCommand,
  type McpClient,
  type McpReservedRequest,
  type McpToolResult,
} from "./mcp-client.js";

/** At most four reads at once (contract section 4). */
export const MAX_CONCURRENT_READS = 4;
/** The native batch cap; the adapter never splits a guarded batch. */
export const MAX_BATCH_STEPS = 10_000;

export type WriteState =
  | "applied"
  | "command_failed"
  | "expectation_failed"
  | "pending"
  | "stale"
  | "cancelled"
  | "unknown";

export interface DocumentStamp {
  readonly documentId: string;
  readonly revision: string;
}

export interface CadTurnAssociation {
  readonly bridgeId: string;
  readonly sessionId: string;
  readonly turnId: string;
  readonly workflowRunId: string;
  /** The host-frozen stamp at turn start; re-inspection advances the adapter's copy. */
  readonly document: DocumentStamp;
  /** Host cancellation closes write admission independently of the runtime. */
  readonly signal?: AbortSignal;
}

export interface Rollback {
  readonly requested: boolean;
  readonly attempted: boolean;
  readonly restored: boolean | null;
  readonly undone: number | null;
}

export interface Expectation {
  readonly ok: boolean;
  readonly measured: JsonObject;
  readonly expected: JsonObject;
  readonly mismatch: readonly string[];
}

export interface WriteRequest {
  readonly bridgeId: string;
  readonly mcpSessionId: string;
  readonly requestId: string;
}

/** Exactly the contract's section 7 write result. */
export interface WriteOutcome {
  readonly state: WriteState;
  readonly request: WriteRequest;
  readonly before: DocumentStamp;
  readonly after: DocumentStamp | null;
  readonly completed: number | null;
  readonly rollback: Rollback;
  readonly expectation: Expectation | null;
  readonly error: WireErrorValue | null;
}

/** Native evidence kept beside the normalized outcome (never a second wire shape). */
export interface CadWriteEvidence {
  readonly applied: readonly number[];
  readonly notRun: readonly number[];
  readonly failed: JsonObject | null;
  readonly results: readonly unknown[];
  readonly native: JsonObject | null;
  readonly expectationUnchecked: boolean;
  readonly pending: boolean;
}

export interface CadWriteResult {
  readonly outcome: WriteOutcome;
  readonly evidence: CadWriteEvidence;
}

export interface CadReadResult {
  readonly tool: string;
  readonly value: JsonObject | null;
  readonly text: string | null;
}

export interface CadInspection {
  readonly stamp: DocumentStamp;
  readonly design: CadReadResult | null;
}

export interface CadCapabilities {
  readonly tools: readonly string[];
  readonly guardedExecute: boolean;
  readonly guardedBatch: boolean;
  readonly describeCommand: boolean;
  readonly pending: boolean;
  readonly running: boolean;
  readonly maxReads: number;
  readonly maxBatchSteps: number;
}

export interface CadBatchStep {
  readonly command: string;
  readonly params?: JsonObject;
}

export interface CadWriteOptions {
  readonly expect?: JsonObject;
  readonly verbose?: boolean;
  /** Batch only; the native default is true. */
  readonly rollback?: boolean;
}

export interface CadAdapterOptions {
  readonly client: McpClient;
  readonly turn: CadTurnAssociation;
  readonly maxConcurrentReads?: number;
  /** The real bridge serves one command at a time; reads wait for a mutation. */
  readonly serializeReadsBehindMutation?: boolean;
  readonly writeTimeoutMs?: number;
  readonly readTimeoutMs?: number;
  readonly pendingTimeoutMs?: number;
  readonly diagnostic?: (line: string) => void;
}

interface PendingWriteContext {
  readonly reservation: McpReservedRequest;
  readonly before: DocumentStamp;
  readonly isBatch: boolean;
  readonly steps: number;
  readonly rollbackRequested: boolean;
  readonly expectRequested: boolean;
  readonly expect: JsonObject | null;
}

interface AppliedContext extends PendingWriteContext {
  readonly sent: boolean;
  readonly fromPending: boolean;
}

class UnresolvedMutation extends Error {
  constructor() {
    super("an earlier mutation is unresolved");
    this.name = "UnresolvedMutation";
  }
}

const REVISION_PATTERN = /^(?:0|[1-9][0-9]{0,19})$/u;
const MAX_REVISION = 18_446_744_073_709_551_615n;
const RETRY_ADVICE: readonly string[] = Object.freeze(["never", "read_only", "same_submission"]);

/** A canonical decimal revision string, not a JavaScript number. */
export function isCanonicalRevision(value: unknown): value is string {
  return typeof value === "string" && REVISION_PATTERN.test(value) && BigInt(value) <= MAX_REVISION;
}

/** Parse the wire/section 7 stamp; numbers and extra fields are refused. */
export function parseDocumentStamp(value: unknown): DocumentStamp | null {
  if (!isObject(value)) {
    return null;
  }
  const keys = Object.keys(value);
  if (keys.length !== 2 || !keys.includes("documentId") || !keys.includes("revision")) {
    return null;
  }
  const documentId = value["documentId"];
  const revision = value["revision"];
  if (!isId(documentId) || !isCanonicalRevision(revision)) {
    return null;
  }
  return { documentId, revision };
}

export function cloneStamp(stamp: DocumentStamp): DocumentStamp {
  return { documentId: stamp.documentId, revision: stamp.revision };
}

/**
 * Read a top-level nonnegative integer exactly as written. The native
 * `document.revision` reply is a JSON number that may exceed 2^53; ordinary
 * `JSON.parse` has already rounded it, so the raw text is authoritative.
 */
export function readTopLevelUnsignedInteger(text: string, key: string): string | null {
  let index = 0;
  const whitespace = (): void => {
    while (index < text.length && /\s/u.test(text[index] ?? "")) {
      index += 1;
    }
  };
  const readString = (): string | null => {
    if (text[index] !== '"') {
      return null;
    }
    index += 1;
    let value = "";
    while (index < text.length) {
      const character = text[index];
      index += 1;
      if (character === '"') {
        return value;
      }
      if (character === "\\") {
        const escape = text[index];
        index += 1;
        switch (escape) {
          case '"':
          case "\\":
          case "/":
            value += escape;
            break;
          case "b":
            value += "\b";
            break;
          case "f":
            value += "\f";
            break;
          case "n":
            value += "\n";
            break;
          case "r":
            value += "\r";
            break;
          case "t":
            value += "\t";
            break;
          case "u": {
            const digits = text.slice(index, index + 4);
            if (!/^[0-9A-Fa-f]{4}$/u.test(digits)) {
              return null;
            }
            value += String.fromCharCode(Number.parseInt(digits, 16));
            index += 4;
            break;
          }
          default:
            return null;
        }
        continue;
      }
      value += character;
    }
    return null;
  };
  const skipStringAt = (start: number): number => {
    let at = start + 1;
    while (at < text.length) {
      const character = text[at];
      if (character === "\\") {
        at += 2;
        continue;
      }
      if (character === '"') {
        return at + 1;
      }
      at += 1;
    }
    return -1;
  };
  const skipValue = (start: number): number => {
    const first = text[start] ?? "";
    if (first === '"') {
      return skipStringAt(start);
    }
    if (first === "{" || first === "[") {
      let at = start;
      let depth = 0;
      while (at < text.length) {
        const character = text[at];
        if (character === '"') {
          const end = skipStringAt(at);
          if (end < 0) {
            return -1;
          }
          at = end;
          continue;
        }
        if (character === "{" || character === "[") {
          depth += 1;
        } else if (character === "}" || character === "]") {
          depth -= 1;
          if (depth === 0) {
            return at + 1;
          }
        }
        at += 1;
      }
      return -1;
    }
    let at = start;
    while (at < text.length && !/[,\]}\s]/u.test(text[at] ?? "")) {
      at += 1;
    }
    return at;
  };

  whitespace();
  if (text[index] !== "{") {
    return null;
  }
  index += 1;
  for (;;) {
    whitespace();
    const character = text[index];
    if (character === "}") {
      return null;
    }
    if (character === ",") {
      index += 1;
      continue;
    }
    const name = readString();
    if (name === null) {
      return null;
    }
    whitespace();
    if (text[index] !== ":") {
      return null;
    }
    index += 1;
    whitespace();
    if (name === key) {
      if (text[index] === "-" || (text[index] ?? "").match(/[0-9]/u) === null) {
        return null;
      }
      const start = index;
      while (index < text.length && (text[index] ?? "").match(/[0-9]/u) !== null) {
        index += 1;
      }
      const digits = text.slice(start, index);
      whitespace();
      const next = text[index];
      if (next !== "," && next !== "}") {
        return null;
      }
      return digits;
    }
    const end = skipValue(index);
    if (end < 0) {
      return null;
    }
    index = end;
  }
}

function wireError(
  code: ErrorCode,
  message: string,
  details: JsonObject = {},
  retry: RetryAdvice = "never",
): WireErrorValue {
  return { code, message: boundedMessage(message), retry, details: normalizeDetails(code, details) };
}

function boundedMessage(message: string): string {
  return redactedMessage(message);
}

/** Keep only the detail fields the contract permits for each code. */
function normalizeDetails(code: ErrorCode, raw: unknown): JsonObject {
  const details = isObject(raw) ? raw : {};
  const id = (value: unknown): string | null => (isId(value) ? value : null);
  switch (code) {
    case "STALE_DOCUMENT": {
      const expected = parseDocumentStamp(details["expected"]);
      const currentRaw = details["current"];
      const current = currentRaw === null ? null : parseDocumentStamp(currentRaw);
      if (expected !== null && (currentRaw === null || current !== null)) {
        return { expected, current };
      }
      return {};
    }
    case "INVALID_REQUEST":
    case "METHOD_NOT_FOUND": {
      const out: JsonObject = {};
      if (typeof details["field"] === "string") {
        out["field"] = details["field"].slice(0, 128);
      }
      if (typeof details["method"] === "string") {
        out["method"] = details["method"].slice(0, 128);
      }
      return out;
    }
    case "UNSUPPORTED_CAPABILITY":
      return typeof details["capability"] === "string" ? { capability: details["capability"].slice(0, 256) } : {};
    case "AUTH_REQUIRED":
    case "CONNECTION_CHANGED":
    case "TASK_FAILED":
    case "INVALID_PROPOSAL": {
      const connectionId = id(details["connectionId"]);
      return connectionId === null ? {} : { connectionId };
    }
    case "PERMISSION_DENIED": {
      const permissionId = id(details["permissionId"]);
      return permissionId === null ? {} : { permissionId };
    }
    case "MODEL_ERROR":
    case "MALFORMED_RESPONSE": {
      const connectionId = id(details["connectionId"]);
      if (connectionId === null) {
        return {};
      }
      const httpStatus = details["httpStatus"];
      return Number.isInteger(httpStatus) && (httpStatus as number) >= 0 && (httpStatus as number) <= 2_147_483_647
        ? { connectionId, httpStatus }
        : { connectionId };
    }
    case "DEPENDENCY_CHANGED":
    case "INVALID_CANDIDATE": {
      const out: JsonObject = {};
      const candidateId = id(details["candidateId"]);
      const dependencyId = id(details["dependencyId"]);
      if (candidateId !== null) {
        out["candidateId"] = candidateId;
      }
      if (dependencyId !== null) {
        out["dependencyId"] = dependencyId;
      }
      return out;
    }
    case "PROFILE_UNSUPPORTED":
    case "REPORT_MISMATCH": {
      const out: JsonObject = {};
      const runId = id(details["runId"]);
      const profileId = id(details["profileId"]);
      if (runId !== null) {
        out["runId"] = runId;
      }
      if (profileId !== null) {
        out["profileId"] = profileId;
      }
      return out;
    }
    case "NOT_READY":
    case "NOT_FOUND": {
      const out: JsonObject = {};
      const runId = id(details["runId"]);
      const artifactId = id(details["artifactId"]);
      if (runId !== null) {
        out["runId"] = runId;
      }
      if (artifactId !== null) {
        out["artifactId"] = artifactId;
      }
      return out;
    }
    case "IDEMPOTENCY_CONFLICT":
    case "REQUEST_EXPIRED": {
      const submissionRequestId = id(details["submissionRequestId"]);
      return submissionRequestId === null ? {} : { submissionRequestId };
    }
    case "RESOURCE_EXHAUSTED": {
      const resource = details["resource"];
      const limit = details["limit"];
      return typeof resource === "string" && Number.isInteger(limit) && (limit as number) >= 0
        ? { resource: resource.slice(0, 128), limit }
        : {};
    }
    case "CANCELLED":
    case "TIMEOUT":
    case "TRANSPORT_ERROR":
    case "OUTCOME_UNKNOWN":
    case "INTERNAL_ERROR": {
      return typeof details["operation"] === "string" ? { operation: details["operation"].slice(0, 128) } : {};
    }
    default:
      return {};
  }
}

function normalizeWireError(value: unknown, fallback: string): WireErrorValue {
  if (!isObject(value)) {
    return wireError("INTERNAL_ERROR", fallback);
  }
  const code = typeof value["code"] === "string" && isKnownErrorCode(value["code"]) ? value["code"] : "INTERNAL_ERROR";
  const message = typeof value["message"] === "string" && value["message"].length > 0 ? value["message"] : fallback;
  const retry =
    typeof value["retry"] === "string" && RETRY_ADVICE.includes(value["retry"])
      ? (value["retry"] as RetryAdvice)
      : "never";
  return wireError(code, message, isObject(value["details"]) ? value["details"] : {}, retry);
}

function errorIsCancelled(error: unknown): boolean {
  return error instanceof ProtocolError && error.value.code === "CANCELLED";
}

function errorIsTimeout(error: unknown): boolean {
  return error instanceof ProtocolError && error.value.code === "TIMEOUT";
}

function errorIsTransport(error: unknown): boolean {
  return error instanceof ProtocolError && error.value.code === "TRANSPORT_ERROR";
}

function toWireError(error: unknown, operation: string): WireErrorValue {
  if (error instanceof ProtocolError) {
    return normalizeWireError(error.toWire(), `The ${operation} operation failed.`);
  }
  if (error instanceof McpRpcError) {
    return wireError("INVALID_REQUEST", error.message, { field: "arguments" });
  }
  if (error instanceof Error) {
    return wireError("INTERNAL_ERROR", error.message.length > 0 ? error.message : `The ${operation} operation failed.`, {
      operation,
    });
  }
  return wireError("INTERNAL_ERROR", `The ${operation} operation failed.`, { operation });
}

function isStillRunning(value: unknown): boolean {
  return typeof value === "string" && value.startsWith("still running:");
}

function normalizeExpectation(value: unknown): Expectation | null {
  if (!isObject(value) || typeof value["ok"] !== "boolean") {
    return null;
  }
  const measured = isObject(value["measured"]) ? value["measured"] : {};
  const expected = isObject(value["expected"]) ? value["expected"] : {};
  const mismatch = Array.isArray(value["mismatch"])
    ? value["mismatch"].filter((item): item is string => typeof item === "string").map((item) => item.slice(0, 512))
    : [];
  return { ok: value["ok"], measured, expected, mismatch };
}

function numberArray(value: unknown, limit: number): number[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const out: number[] = [];
  for (const item of value.slice(0, limit)) {
    if (typeof item === "number" && Number.isInteger(item) && item >= 0) {
      out.push(item);
    }
  }
  return out;
}

function hasExpectedDocument(schema: JsonObject): boolean {
  const properties = schema["properties"];
  return isObject(properties) && isObject(properties["expectedDocument"]);
}

function rollbackNone(requested: boolean): Rollback {
  return { requested, attempted: false, restored: null, undone: null };
}

function expectationUnchecked(request: JsonObject | null): Expectation {
  return {
    ok: false,
    measured: {},
    expected: request === null ? {} : request,
    mismatch: ["the bridge did not return an expectation result for the completed write"],
  };
}

function raceAbort<T>(promise: Promise<T>, signal: AbortSignal, operation: string): Promise<T> {
  if (signal.aborted) {
    return Promise.reject(cancelled(operation));
  }
  return new Promise<T>((resolve, reject) => {
    const onAbort = (): void => {
      signal.removeEventListener("abort", onAbort);
      reject(cancelled(operation));
    };
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener("abort", onAbort);
        reject(error);
      },
    );
  });
}

export class CadAdapter {
  readonly #client: McpClient;
  readonly #turn: CadTurnAssociation;
  readonly #diagnostic: (line: string) => void;
  readonly #maxReads: number;
  readonly #serializeReads: boolean;
  readonly #writeTimeoutMs: number;
  readonly #readTimeoutMs: number;
  readonly #pendingTimeoutMs: number;
  readonly #owned = new AbortController();
  readonly #signal: AbortSignal;
  readonly #documentId: string;
  #stamp: DocumentStamp;
  #unresolved: PendingWriteContext | null = null;
  #writeChain: Promise<void> | null = null;
  #reads = 0;
  #readFreed: Array<() => void> = [];
  #disposed = false;

  constructor(options: CadAdapterOptions) {
    this.#client = options.client;
    this.#turn = options.turn;
    this.#diagnostic = options.diagnostic ?? (() => undefined);
    this.#maxReads = Math.min(Math.max(1, options.maxConcurrentReads ?? MAX_CONCURRENT_READS), MAX_CONCURRENT_READS);
    this.#serializeReads = options.serializeReadsBehindMutation ?? true;
    this.#writeTimeoutMs = options.writeTimeoutMs ?? 360_000;
    this.#readTimeoutMs = options.readTimeoutMs ?? 300_000;
    this.#pendingTimeoutMs = options.pendingTimeoutMs ?? 90_000;
    if (!isId(options.turn.sessionId) || !isId(options.turn.turnId) || !isId(options.turn.workflowRunId)) {
      throw new ProtocolError("INVALID_REQUEST", "The chat turn association is invalid.", "never", { field: "turn" });
    }
    const association = options.client.association;
    if (
      options.client.bridgeId !== options.turn.bridgeId ||
      association.sessionId !== options.turn.sessionId ||
      association.turnId !== options.turn.turnId ||
      association.workflowRunId !== options.turn.workflowRunId
    ) {
      throw new ProtocolError(
        "INVALID_REQUEST",
        "The MCP connection belongs to another bridge or chat turn.",
        "never",
        { field: "turn" },
      );
    }
    const stamp = parseDocumentStamp(options.turn.document);
    if (stamp === null) {
      throw new ProtocolError("INVALID_REQUEST", "The turn binding's document stamp is invalid.", "never", {
        field: "document",
      });
    }
    this.#stamp = stamp;
    this.#documentId = stamp.documentId;
    this.#signal =
      options.turn.signal === undefined
        ? this.#owned.signal
        : AbortSignal.any([options.turn.signal, this.#owned.signal]);
  }

  /** The stamp the next mutation is bound to. */
  get stamp(): DocumentStamp {
    return cloneStamp(this.#stamp);
  }

  /** The chat turn this adapter serves (never accepted from a tool argument). */
  get association(): CadTurnAssociation {
    return this.#turn;
  }

  get requiresReconciliation(): boolean {
    return this.#unresolved !== null;
  }

  get isCancelled(): boolean {
    return this.#signal.aborted;
  }

  /** Discovered capabilities; no guessed fallbacks. */
  capabilities(): CadCapabilities {
    const execute = this.#client.tool("execute");
    const batch = this.#client.tool("batch");
    return {
      tools: this.#client.tools().map((tool) => tool.name),
      guardedExecute: execute !== null && hasExpectedDocument(execute.inputSchema),
      guardedBatch: batch !== null && hasExpectedDocument(batch.inputSchema),
      describeCommand: this.#client.hasTool("describe_command"),
      pending: this.#client.hasTool("pending"),
      running: this.#client.hasTool("running"),
      maxReads: this.#maxReads,
      maxBatchSteps: MAX_BATCH_STEPS,
    };
  }

  /**
   * Read-only exact inspection through a tool the bridge annotates as
   * read-only. A write-capable tool is refused rather than used as a read.
   */
  async readTool(name: string, args: JsonObject = {}): Promise<CadReadResult> {
    const tool = this.#client.tool(name);
    if (tool === null) {
      throw new ProtocolError("UNSUPPORTED_CAPABILITY", `The SolidCraft bridge does not expose the \`${name}\` tool.`, "never", {
        capability: `mcp-tool:${name}`,
      });
    }
    if (!tool.readOnly) {
      throw new ProtocolError(
        "UNSUPPORTED_CAPABILITY",
        `\`${name}\` can change the design; a guarded execute or batch is required.`,
        "never",
        { capability: `read-tool:${name}` },
      );
    }
    this.#client.assertToolArguments(name, args);
    return this.#withRead(async () => {
      const reply = await this.#client.callTool(name, args, { timeoutMs: this.#readTimeoutMs });
      if (reply.isError) {
        throw new ProtocolError("INTERNAL_ERROR", reply.text ?? `The \`${name}\` read failed.`, "read_only", {
          operation: name,
        });
      }
      return { tool: name, value: reply.value, text: reply.text };
    });
  }

  /**
   * The exact `document.revision` inspection (no arguments), preserving a u64
   * revision above 2^53 from the raw reply text.
   */
  async readDocumentStamp(): Promise<DocumentStamp> {
    const command = await this.#describeCommand("document.revision");
    this.#client.assertCommandArguments(command, {}, { allowMissingSchema: true });
    return this.#withRead(async () => {
      const reply = await this.#client.callTool(
        "execute",
        { command: "document.revision", params: {} },
        { timeoutMs: this.#readTimeoutMs },
      );
      if (reply.isError) {
        throw new ProtocolError(
          "INTERNAL_ERROR",
          reply.text ?? "The SolidCraft bridge could not read the document revision.",
          "read_only",
          { operation: "document.revision" },
        );
      }
      const value = reply.value;
      if (!isObject(value)) {
        throw new ProtocolError("INTERNAL_ERROR", "The document revision reply is missing.", "read_only", {
          operation: "document.revision",
        });
      }
      const documentId = value["document_id"];
      if (!isId(documentId)) {
        throw new ProtocolError("INTERNAL_ERROR", "The document revision reply names no valid design.", "read_only", {
          operation: "document.revision",
        });
      }
      const revision = reply.text === null ? null : readTopLevelUnsignedInteger(reply.text, "revision");
      if (revision === null || !isCanonicalRevision(revision)) {
        throw new ProtocolError("INTERNAL_ERROR", "The document revision could not be decoded losslessly.", "read_only", {
          operation: "document.revision",
        });
      }
      if (documentId !== this.#documentId) {
        throw new ProtocolError(
          "STALE_DOCUMENT",
          "The open design is not the one this chat turn pinned.",
          "never",
          { expected: cloneStamp(this.#stamp), current: { documentId, revision } },
        );
      }
      return { documentId, revision };
    });
  }

  /** Re-inspect the current design; adopts the fresh stamp for later writes. */
  async inspect(options: { readonly design?: boolean } = {}): Promise<CadInspection> {
    const [stamp, design] = await Promise.all([
      this.readDocumentStamp(),
      options.design === false ? Promise.resolve(null) : this.readTool("inspect_design", { measure: true, sketches: false }),
    ]);
    if (stamp.documentId !== this.#documentId) {
      throw new ProtocolError(
        "STALE_DOCUMENT",
        "Re-inspection returned another document; this turn cannot adopt it.",
        "never",
        { expected: cloneStamp(this.#stamp), current: stamp },
      );
    }
    if (this.#unresolved === null) {
      this.#stamp = cloneStamp(stamp);
    }
    return { stamp, design };
  }

  /** Adopt a stamp established by an explicit re-inspection. */
  adoptStamp(stamp: DocumentStamp): void {
    const parsed = parseDocumentStamp(stamp);
    if (parsed === null) {
      throw new ProtocolError("INVALID_REQUEST", "The inspected document stamp is invalid.", "never", {
        field: "document",
      });
    }
    if (parsed.documentId !== this.#documentId) {
      throw new ProtocolError(
        "STALE_DOCUMENT",
        "The inspected stamp belongs to another document; this turn cannot adopt it.",
        "never",
        { expected: cloneStamp(this.#stamp), current: parsed },
      );
    }
    if (this.#unresolved !== null) {
      throw new ProtocolError(
        "OUTCOME_UNKNOWN",
        "An earlier mutation is unresolved; inspect and reconcile it before adopting a new stamp.",
        "never",
        { operation: "write-admission" },
      );
    }
    this.#stamp = parsed;
  }

  /** One guarded command. */
  async execute(command: string, params: JsonObject = {}, options: CadWriteOptions = {}): Promise<CadWriteResult> {
    const reservation = this.#client.reserveRequest();
    if (this.#unresolved !== null) {
      return this.#refuseUnresolved(reservation, false, 1);
    }
    if (this.#signal.aborted) {
      return this.#cancelledOutcome(reservation, false, 1);
    }
    let prepared: JsonObject;
    try {
      this.#assertGuarded("execute");
      const descriptor = await this.#describeCommand(command);
      this.#client.assertCommandArguments(descriptor, params, { allowMissingSchema: false });
      const args: JsonObject = { command, params };
      if (options.verbose !== undefined) {
        args["verbose"] = options.verbose;
      }
      if (options.expect !== undefined) {
        args["expect"] = options.expect;
      }
      this.#client.assertToolArguments("execute", { ...args, expectedDocument: cloneStamp(this.#stamp) });
      prepared = args;
    } catch (error) {
      return this.#preSendFailure(reservation, error, false, 1, options.expect !== undefined);
    }
    return this.#runWrite(
      {
        reservation,
        before: cloneStamp(this.#stamp),
        isBatch: false,
        steps: 1,
        rollbackRequested: false,
        expectRequested: options.expect !== undefined,
        expect: options.expect ?? null,
      },
      () => prepared,
    );
  }

  /** One guarded batch: admitted and applied as one host operation. */
  async batch(steps: readonly CadBatchStep[], options: CadWriteOptions = {}): Promise<CadWriteResult> {
    const reservation = this.#client.reserveRequest();
    const count = Array.isArray(steps) ? steps.length : 0;
    if (this.#unresolved !== null) {
      return this.#refuseUnresolved(reservation, true, count);
    }
    if (this.#signal.aborted) {
      return this.#cancelledOutcome(reservation, true, count);
    }
    const rollbackRequested = options.rollback ?? true;
    let prepared: JsonObject;
    try {
      if (!Array.isArray(steps) || steps.length === 0) {
        throw new ProtocolError("INVALID_REQUEST", "A guarded batch needs at least one step.", "never", {
          field: "commands",
        });
      }
      if (steps.length > MAX_BATCH_STEPS) {
        throw resourceExhausted("batchSteps", MAX_BATCH_STEPS);
      }
      const normalized: JsonObject[] = [];
      for (const [index, step] of steps.entries()) {
        if (!isObject(step) || typeof step["command"] !== "string" || step["command"].length === 0) {
          throw new ProtocolError("INVALID_REQUEST", `Batch step ${index} needs a command id.`, "never", {
            field: "commands",
          });
        }
        for (const key of Object.keys(step)) {
          if (key !== "command" && key !== "params") {
            throw new ProtocolError(
              "INVALID_REQUEST",
              `Batch step ${index} has \`${key}\`; a step is exactly {command, params}.`,
              "never",
              { field: "commands" },
            );
          }
        }
        const rawParams = step["params"];
        if (rawParams !== undefined && !isObject(rawParams)) {
          throw new ProtocolError("INVALID_REQUEST", `Batch step ${index} parameters must be an object.`, "never", {
            field: "commands",
          });
        }
        normalized.push({ command: step["command"], params: rawParams ?? {} });
      }
      const commands = [...new Set(normalized.map((step) => String(step["command"])))];
      this.#assertGuarded("batch");
      const descriptors = await this.#discoverAll(commands);
      for (const step of normalized) {
        const descriptor = descriptors.get(String(step["command"]));
        if (descriptor === undefined) {
          throw new ProtocolError(
            "UNSUPPORTED_CAPABILITY",
            `The SolidCraft bridge cannot describe batch command \`${String(step["command"])}\`.`,
            "never",
            { capability: `command:${String(step["command"])}` },
          );
        }
        this.#client.assertCommandArguments(descriptor, step["params"] as JsonObject, { allowMissingSchema: false });
      }
      const args: JsonObject = { commands: normalized };
      if (options.rollback !== undefined) {
        args["rollback"] = options.rollback;
      }
      if (options.verbose !== undefined) {
        args["verbose"] = options.verbose;
      }
      if (options.expect !== undefined) {
        args["expect"] = options.expect;
      }
      this.#client.assertToolArguments("batch", { ...args, expectedDocument: cloneStamp(this.#stamp) });
      prepared = args;
    } catch (error) {
      return this.#preSendFailure(reservation, error, true, count, options.expect !== undefined);
    }
    return this.#runWrite(
      {
        reservation,
        before: cloneStamp(this.#stamp),
        isBatch: true,
        steps: steps.length,
        rollbackRequested,
        expectRequested: options.expect !== undefined,
        expect: options.expect ?? null,
      },
      () => prepared,
    );
  }

  /** A speculative candidate probe is unsupported in 1.0 and performs zero writes. */
  probe(): never {
    throw new ProtocolError(
      "UNSUPPORTED_CAPABILITY",
      "A speculative candidate probe is unsupported; inspect the live design read-only or run a guarded write.",
      "never",
      { capability: "speculative-candidate-probe" },
    );
  }

  /**
   * Reconcile the original mutation on the same connection; never resend it.
   *
   * While the outcome is unresolved, every new write is refused. 1.0 has no
   * in-turn escape from `unknown`: if this connection cannot establish the
   * effect, the turn ends `outcome_unknown` and the user starts a new turn and
   * re-inspects. A future version may add an explicit host acknowledgement.
   */
  async reconcile(): Promise<CadWriteResult | null> {
    const context = this.#unresolved;
    if (context === null) {
      return null;
    }
    return this.#queryPending(context);
  }

  /** The existing `pending` query on this connection (diagnostic). */
  async pendingStatus(): Promise<JsonObject | null> {
    const reply = await this.#client.pending(this.#pendingTimeoutMs);
    return reply.value;
  }

  /** The existing `running` query; diagnostic only, never proof of an effect. */
  async runningStatus(): Promise<JsonObject | null> {
    const reply = await this.#client.running(this.#readTimeoutMs);
    return reply.value;
  }

  /** Close write admission and stop scheduling new writes. */
  cancel(reason: string = "the turn is terminal"): void {
    if (!this.#signal.aborted) {
      this.#owned.abort(reason);
    }
    this.#diagnostic(`cad: cancelled (${reason.slice(0, 80)})`);
  }

  dispose(): void {
    if (this.#disposed) {
      return;
    }
    this.#disposed = true;
    this.cancel("the adapter was disposed");
    this.#client.close("the CAD adapter was disposed");
  }

  async #withRead<T>(body: () => Promise<T>): Promise<T> {
    if (this.#signal.aborted) {
      throw cancelled("cad-read");
    }
    if (this.#serializeReads && this.#writeChain !== null) {
      await raceAbort(this.#writeChain, this.#signal, "cad-read");
    }
    while (this.#reads >= this.#maxReads) {
      await raceAbort(this.#slotFreed(), this.#signal, "cad-read");
    }
    this.#reads += 1;
    try {
      return await body();
    } finally {
      this.#reads -= 1;
      for (const resolve of this.#readFreed.splice(0)) {
        resolve();
      }
    }
  }

  #slotFreed(): Promise<void> {
    return new Promise<void>((resolve) => {
      this.#readFreed.push(resolve);
    });
  }

  async #drainReads(): Promise<void> {
    while (this.#reads > 0) {
      await raceAbort(this.#slotFreed(), this.#signal, "cad-write");
    }
  }

  async #serializeWrite<T>(body: () => Promise<T>): Promise<T> {
    const previous = this.#writeChain;
    let release: () => void = () => undefined;
    const current = new Promise<void>((resolve) => {
      release = resolve;
    });
    this.#writeChain = previous === null ? current : previous.then(() => current);
    try {
      if (previous !== null) {
        await raceAbort(previous, this.#signal, "cad-write");
      }
      if (this.#signal.aborted) {
        throw cancelled("cad-write");
      }
      return await body();
    } finally {
      release();
    }
  }

  async #runWrite(context: PendingWriteContext, args: () => JsonObject): Promise<CadWriteResult> {
    let sent = false;
    let before = context.before;
    try {
      return await this.#serializeWrite(async () => {
        if (this.#signal.aborted) {
          throw cancelled("cad-write");
        }
        if (this.#unresolved !== null) {
          throw new UnresolvedMutation();
        }
        await this.#drainReads();
        before = cloneStamp(this.#stamp);
        const envelope: JsonObject = { ...args(), expectedDocument: cloneStamp(before) };
        sent = true;
        const reply = await this.#client.callTool(context.isBatch ? "batch" : "execute", envelope, {
          reservation: context.reservation,
          timeoutMs: this.#writeTimeoutMs,
        });
        return await this.#interpret(reply, { ...context, before, sent: true, fromPending: false });
      });
    } catch (error) {
      if (error instanceof UnresolvedMutation) {
        return this.#refuseUnresolved(context.reservation, context.isBatch, context.steps);
      }
      return await this.#failure(error, { ...context, before: sent ? before : cloneStamp(this.#stamp), sent, fromPending: false });
    }
  }

  #assertGuarded(tool: "execute" | "batch"): void {
    const found = this.#client.tool(tool);
    if (found === null) {
      throw new ProtocolError("UNSUPPORTED_CAPABILITY", `The SolidCraft bridge does not expose the \`${tool}\` tool.`, "never", {
        capability: `mcp-tool:${tool}`,
      });
    }
    if (!hasExpectedDocument(found.inputSchema)) {
      throw new ProtocolError(
        "UNSUPPORTED_CAPABILITY",
        `The SolidCraft bridge's \`${tool}\` tool cannot bind a document stamp; no unguarded fallback is used.`,
        "never",
        { capability: `guarded-${tool}` },
      );
    }
  }

  /**
   * Discovery is a read: it shares the four-slot gate with `readTool` and waits
   * behind an in-flight mutation, so a held read set plus a batch cannot exceed
   * the section 4 ceiling. Cached descriptors do not consume a slot.
   */
  async #describeCommand(id: string): Promise<DiscoveredCommand> {
    const cached = this.#client.cachedCommand(id);
    if (cached !== null) {
      return cached;
    }
    return this.#withRead(() => this.#client.describeCommand(id));
  }

  async #discoverAll(commands: readonly string[]): Promise<Map<string, DiscoveredCommand>> {
    const out = new Map<string, DiscoveredCommand>();
    let next = 0;
    const worker = async (): Promise<void> => {
      for (;;) {
        const index = next;
        next += 1;
        const command = commands[index];
        if (command === undefined) {
          return;
        }
        if (this.#signal.aborted) {
          throw cancelled("cad-discovery");
        }
        out.set(command, await this.#describeCommand(command));
      }
    };
    const workers = Array.from({ length: Math.min(this.#maxReads, commands.length) }, () => worker());
    await Promise.all(workers);
    return out;
  }

  #request(reservation: McpReservedRequest): WriteRequest {
    return {
      bridgeId: this.#client.bridgeId,
      mcpSessionId: this.#client.mcpSessionId,
      requestId: reservation.wireId,
    };
  }

  #evidence(
    body: JsonObject | null,
    state: WriteState,
    context: PendingWriteContext,
    expectationUnchecked: boolean,
    pending: boolean,
  ): CadWriteEvidence {
    const native = body;
    let applied =
      native !== null && state !== "pending" && state !== "unknown"
        ? numberArray(native["applied"], context.steps)
        : [];
    if (native !== null && state === "applied" && context.isBatch && applied.length === 0) {
      const completed = Number.isInteger(native["completed"]) ? (native["completed"] as number) : context.steps;
      applied = Array.from({ length: Math.max(0, Math.min(completed, context.steps)) }, (_, index) => index);
    }
    const notRun =
      native !== null
        ? numberArray(native["not_run"], context.steps)
        : context.isBatch && (state === "command_failed" || state === "stale")
          ? Array.from({ length: context.steps }, (_, i) => i)
          : [];
    const failed = native !== null && isObject(native["failed"]) ? native["failed"] : null;
    const results = native !== null && Array.isArray(native["results"]) ? native["results"] : [];
    return {
      applied,
      notRun,
      failed,
      results,
      native,
      expectationUnchecked,
      pending,
    };
  }

  #preSendFailure(
    reservation: McpReservedRequest,
    error: unknown,
    isBatch: boolean,
    steps: number,
    expectRequested: boolean,
  ): CadWriteResult {
    const context: PendingWriteContext = {
      reservation,
      before: cloneStamp(this.#stamp),
      isBatch,
      steps,
      rollbackRequested: isBatch,
      expectRequested,
      expect: null,
    };
    const state: WriteState = errorIsCancelled(error) ? "cancelled" : "command_failed";
    return {
      outcome: {
        state,
        request: this.#request(reservation),
        before: cloneStamp(context.before),
        after: cloneStamp(context.before),
        completed: 0,
        rollback: rollbackNone(isBatch),
        expectation: null,
        error: toWireError(error, isBatch ? "batch" : "execute"),
      },
      evidence: this.#evidence(null, state, context, expectRequested, false),
    };
  }

  #refuseUnresolved(reservation: McpReservedRequest, isBatch: boolean, steps: number): CadWriteResult {
    const context: PendingWriteContext = {
      reservation,
      before: cloneStamp(this.#stamp),
      isBatch,
      steps,
      rollbackRequested: isBatch,
      expectRequested: false,
      expect: null,
    };
    return {
      outcome: {
        state: "command_failed",
        request: this.#request(reservation),
        before: cloneStamp(context.before),
        after: null,
        completed: 0,
        rollback: rollbackNone(isBatch),
        expectation: null,
        error: wireError(
          "OUTCOME_UNKNOWN",
          "An earlier mutation is unresolved on this connection; reconcile it before another write.",
          { operation: "write-admission" },
        ),
      },
      evidence: this.#evidence(null, "command_failed", context, false, false),
    };
  }

  #cancelledOutcome(
    reservation: McpReservedRequest,
    isBatch: boolean,
    steps: number,
  ): CadWriteResult {
    const context: PendingWriteContext = {
      reservation,
      before: cloneStamp(this.#stamp),
      isBatch,
      steps,
      rollbackRequested: isBatch,
      expectRequested: false,
      expect: null,
    };
    return {
      outcome: {
        state: "cancelled",
        request: this.#request(reservation),
        before: cloneStamp(context.before),
        after: cloneStamp(context.before),
        completed: 0,
        rollback: rollbackNone(isBatch),
        expectation: null,
        error: wireError("CANCELLED", "The chat turn was cancelled before another write was admitted.", {
          operation: isBatch ? "batch" : "execute",
        }),
      },
      evidence: this.#evidence(null, "cancelled", context, false, false),
    };
  }

  async #failure(error: unknown, context: AppliedContext): Promise<CadWriteResult> {
    const operation = context.isBatch ? "batch" : "execute";
    if (errorIsCancelled(error)) {
      this.#unresolved = null;
      return {
        outcome: {
          state: "cancelled",
          request: this.#request(context.reservation),
          before: cloneStamp(context.before),
          after: context.sent ? null : cloneStamp(context.before),
          completed: context.sent ? null : 0,
          rollback: rollbackNone(context.rollbackRequested),
          expectation: null,
          error: toWireError(error, operation),
        },
        evidence: this.#evidence(null, "cancelled", context, context.expectRequested, false),
      };
    }
    if (error instanceof McpRpcError) {
      // JSON-RPC rejected the call before the server invoked the tool: zero writes.
      return this.#preSendFailure(context.reservation, error, context.isBatch, context.steps, context.expectRequested);
    }
    if (errorIsTimeout(error) && context.sent) {
      // A timeout is "still running", not a definite failure: ask `pending` on
      // this very connection, with the original request identity.
      return this.#pendingAdvisory(context);
    }
    if ((errorIsTransport(error) || !(error instanceof ProtocolError)) && context.sent) {
      return this.#unknown(context, wireError("OUTCOME_UNKNOWN", "The connection was lost after submission; its effect is unknown.", {
        operation,
      }));
    }
    return this.#preSendFailure(context.reservation, error, context.isBatch, context.steps, context.expectRequested);
  }

  async #interpret(reply: McpToolResult, context: AppliedContext): Promise<CadWriteResult> {
    const body = reply.structured ?? reply.value;
    if (body === null) {
      if (isStillRunning(reply.text)) {
        return this.#queryPending(context);
      }
      if (reply.isError) {
        // A tool-result error is a definite failed command: the bridge answered.
        this.#unresolved = null;
        return {
          outcome: {
            state: "command_failed",
            request: this.#request(context.reservation),
            before: cloneStamp(context.before),
            after: cloneStamp(context.before),
            completed: 0,
            rollback: rollbackNone(context.rollbackRequested),
            expectation: null,
            error: toWireError(
              new ProtocolError("INTERNAL_ERROR", reply.text ?? "The bridge refused the write.", "never", {
                operation: context.isBatch ? "batch" : "execute",
              }),
              context.isBatch ? "batch" : "execute",
            ),
          },
          evidence: this.#evidence(null, "command_failed", context, context.expectRequested, false),
        };
      }
      return this.#unknown(context, wireError("OUTCOME_UNKNOWN", "The bridge returned no readable write result.", {
        operation: context.isBatch ? "batch" : "execute",
      }));
    }
    return this.#interpretBody(body, context);
  }

  async #interpretBody(body: JsonObject, context: AppliedContext): Promise<CadWriteResult> {
    if (body["outcomeUnknown"] === true) {
      return this.#unknown(context, normalizeWireError(body["error"], "The bridge cannot establish what happened."));
    }
    if (body["pending"] === true || isStillRunning(body["error"])) {
      if (context.fromPending) {
        return this.#unknown(context, wireError("OUTCOME_UNKNOWN", "The original request is still marked pending.", {
          operation: context.isBatch ? "batch" : "execute",
        }));
      }
      return this.#queryPending(context);
    }
    if (body["notSent"] === true) {
      this.#unresolved = null;
      return {
        outcome: {
          state: "command_failed",
          request: this.#request(context.reservation),
          before: cloneStamp(context.before),
          after: cloneStamp(context.before),
          completed: 0,
          rollback: rollbackNone(context.rollbackRequested),
          expectation: null,
          error: toWireError(new ProtocolError("INTERNAL_ERROR", typeof body["error"] === "string" ? body["error"] : "The request was refused before it was sent."), context.isBatch ? "batch" : "execute"),
        },
        evidence: this.#evidence(body, "command_failed", context, context.expectRequested, false),
      };
    }
    if (isObject(body["error"])) {
      return this.#interpretWireError(body["error"], context);
    }
    if (body["documentStamp"] !== undefined) {
      return this.#interpretStamped(body, context);
    }
    if (typeof body["error"] === "string") {
      const state: WriteState = "command_failed";
      this.#unresolved = null;
      return {
        outcome: {
          state,
          request: this.#request(context.reservation),
          before: cloneStamp(context.before),
          after: cloneStamp(context.before),
          completed: 0,
          rollback: rollbackNone(context.rollbackRequested),
          expectation: null,
          error: toWireError(new ProtocolError("INTERNAL_ERROR", body["error"], "never", {
            operation: context.isBatch ? "batch" : "execute",
          }), context.isBatch ? "batch" : "execute"),
        },
        evidence: this.#evidence(body, state, context, context.expectRequested, false),
      };
    }
    // A pending result wrapper that the bridge returned without `pending: true`.
    if (body["done"] === true && isObject(body["result"])) {
      return this.#interpretBody(body["result"], { ...context, fromPending: true });
    }
    return this.#unknown(context, wireError("OUTCOME_UNKNOWN", "The guarded reply carried no document stamp.", {
      operation: context.isBatch ? "batch" : "execute",
    }));
  }

  async #interpretWireError(error: JsonObject, context: AppliedContext): Promise<CadWriteResult> {
    const code = typeof error["code"] === "string" ? error["code"] : "INTERNAL_ERROR";
    const normalized = normalizeWireError(error, "The SolidCraft bridge refused the write.");
    if (code === "STALE_DOCUMENT") {
      // `after` carries the app's current stamp (or null when the target is
      // closed) so the caller can re-inspect; the adapter never adopts it here.
      const details = isObject(error["details"]) ? error["details"] : {};
      const current = details["current"] === null ? null : parseDocumentStamp(details["current"]);
      this.#unresolved = null;
      return {
        outcome: {
          state: "stale",
          request: this.#request(context.reservation),
          before: cloneStamp(context.before),
          after: current,
          completed: 0,
          rollback: rollbackNone(context.rollbackRequested),
          expectation: null,
          error: normalized,
        },
        evidence: this.#evidence(null, "stale", context, false, false),
      };
    }
    if (code === "CANCELLED") {
      this.#unresolved = null;
      return {
        outcome: {
          state: "cancelled",
          request: this.#request(context.reservation),
          before: cloneStamp(context.before),
          after: context.sent ? null : cloneStamp(context.before),
          completed: context.sent ? null : 0,
          rollback: rollbackNone(context.rollbackRequested),
          expectation: null,
          error: normalized,
        },
        evidence: this.#evidence(null, "cancelled", context, context.expectRequested, false),
      };
    }
    if (code === "TIMEOUT") {
      return this.#queryPending(context);
    }
    if (code === "INVALID_REQUEST" || code === "METHOD_NOT_FOUND" || code === "UNSUPPORTED_CAPABILITY" || code === "RESOURCE_EXHAUSTED" || code === "NOT_FOUND") {
      this.#unresolved = null;
      return {
        outcome: {
          state: "command_failed",
          request: this.#request(context.reservation),
          before: cloneStamp(context.before),
          after: cloneStamp(context.before),
          completed: 0,
          rollback: rollbackNone(context.rollbackRequested),
          expectation: null,
          error: normalized,
        },
        evidence: this.#evidence(null, "command_failed", context, context.expectRequested, false),
      };
    }
    return this.#unknown(context, wireError("OUTCOME_UNKNOWN", normalized.message, {
      operation: context.isBatch ? "batch" : "execute",
    }));
  }

  #interpretStamped(body: JsonObject, context: AppliedContext): CadWriteResult {
    const after = parseDocumentStamp(body["documentStamp"]);
    if (after === null) {
      return this.#unknown(context, wireError("OUTCOME_UNKNOWN", "The guarded reply's document stamp is invalid.", {
        operation: context.isBatch ? "batch" : "execute",
      }));
    }
    const failed = body["ok"] === false || (body["failed"] !== undefined && body["failed"] !== null);
    const expectation = normalizeExpectation(body["expectation"]);
    const completed = this.#completed(body, context, failed);
    const rollback = context.isBatch
      ? {
          requested: context.rollbackRequested,
          attempted:
            failed &&
            context.rollbackRequested &&
            ((completed ?? 0) > 0 || (typeof body["undone"] === "number" && body["undone"] > 0)),
          restored: typeof body["rolled_back"] === "boolean" ? body["rolled_back"] : null,
          undone: Number.isInteger(body["undone"]) ? (body["undone"] as number) : null,
        }
      : rollbackNone(false);
    if (!failed) {
      if (context.expectRequested && expectation === null) {
        return this.#expectationFailed(context, after, completed, rollback, expectationUnchecked(context.expect), body);
      }
      if (expectation !== null && !expectation.ok) {
        return this.#expectationFailed(context, after, completed, rollback, expectation, body);
      }
      this.#stamp = after;
      this.#unresolved = null;
      return {
        outcome: {
          state: "applied",
          request: this.#request(context.reservation),
          before: cloneStamp(context.before),
          after,
          completed,
          rollback,
          expectation,
          error: null,
        },
        evidence: this.#evidence(body, "applied", context, false, false),
      };
    }
    const failure = isObject(body["failed"]) ? body["failed"] : null;
    const message =
      failure !== null && typeof failure["error"] === "string"
        ? failure["error"]
        : typeof body["error"] === "string"
          ? body["error"]
          : context.isBatch
            ? "A batch step failed."
            : "The command failed.";
    this.#stamp = after;
    this.#unresolved = null;
    return {
      outcome: {
        state: "command_failed",
        request: this.#request(context.reservation),
        before: cloneStamp(context.before),
        after,
        completed,
        rollback,
        expectation,
        error: wireError("INTERNAL_ERROR", message, { operation: context.isBatch ? "batch" : "execute" }),
      },
      evidence: this.#evidence(body, "command_failed", context, false, false),
    };
  }

  #completed(body: JsonObject, context: AppliedContext, failed: boolean): number | null {
    if (!context.isBatch) {
      return failed ? (body["documentStamp"] === undefined ? 0 : 1) : 1;
    }
    const raw = body["completed"];
    if (Number.isInteger(raw) && (raw as number) >= 0) {
      return Math.min(raw as number, context.steps);
    }
    return failed ? null : context.steps;
  }

  #expectationFailed(
    context: AppliedContext,
    after: DocumentStamp,
    completed: number | null,
    rollback: Rollback,
    expectation: Expectation,
    body: JsonObject,
  ): CadWriteResult {
    this.#stamp = after;
    this.#unresolved = null;
    return {
      outcome: {
        state: "expectation_failed",
        request: this.#request(context.reservation),
        before: cloneStamp(context.before),
        after,
        completed,
        rollback,
        expectation,
        error: null,
      },
      evidence: this.#evidence(body, "expectation_failed", context, body["expectation"] === undefined, false),
    };
  }

  #pendingAdvisory(context: AppliedContext): Promise<CadWriteResult> {
    return this.#queryPending(context);
  }

  async #queryPending(context: PendingWriteContext & { readonly sent?: boolean }): Promise<CadWriteResult> {
    this.#unresolved = context;
    let reply: McpToolResult;
    try {
      reply = await this.#client.pending(this.#pendingTimeoutMs);
    } catch (error) {
      return this.#unknown(context, wireError("OUTCOME_UNKNOWN", "The original request could not be reconciled on this connection.", {
        operation: context.isBatch ? "batch" : "execute",
      }), error);
    }
    const status = reply.value;
    if (status === null) {
      return this.#unknown(context, wireError("OUTCOME_UNKNOWN", "The bridge returned no pending status.", {
        operation: context.isBatch ? "batch" : "execute",
      }));
    }
    if (status["outcomeUnknown"] === true || status["lost"] === true) {
      return this.#unknown(context, wireError("OUTCOME_UNKNOWN", "The original request's effect could not be established.", {
        operation: context.isBatch ? "batch" : "execute",
      }));
    }
    if (status["done"] === true) {
      const original = status["result"];
      if (!isObject(original)) {
        return this.#unknown(context, wireError("OUTCOME_UNKNOWN", "The original reply is missing or unreadable.", {
          operation: context.isBatch ? "batch" : "execute",
        }));
      }
      return this.#interpretBody(original, { ...context, sent: context.sent ?? true, fromPending: true });
    }
    if (status["pending"] === null) {
      return this.#unknown(
        context,
        wireError("OUTCOME_UNKNOWN", "The bridge holds no original reply; its effect is not inferred from an empty pending slot.", {
          operation: context.isBatch ? "batch" : "execute",
        }),
      );
    }
    return {
      outcome: {
        state: "pending",
        request: this.#request(context.reservation),
        before: cloneStamp(context.before),
        after: null,
        completed: null,
        rollback: rollbackNone(context.rollbackRequested),
        expectation: null,
        error: null,
      },
      evidence: this.#evidence(null, "pending", context, false, true),
    };
  }

  #unknown(context: PendingWriteContext, error: WireErrorValue, cause?: unknown): CadWriteResult {
    this.#unresolved = context;
    if (cause !== undefined) {
      this.#diagnostic(`cad: unresolved write ${context.reservation.wireId}: ${error.message.slice(0, 120)}`);
    }
    return {
      outcome: {
        state: "unknown",
        request: this.#request(context.reservation),
        before: cloneStamp(context.before),
        after: null,
        completed: null,
        rollback: rollbackNone(context.rollbackRequested),
        expectation: null,
        error,
      },
      evidence: this.#evidence(null, "unknown", context, context.expectRequested, false),
    };
  }
}
