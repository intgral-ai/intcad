/**
 * Protocol 1.0 types, the repository schema bundle loader and the explicit
 * validation layers that JSON Schema cannot express.
 *
 * The bundle under `schema/mastra-runtime/v1/` is read from the repository
 * revision, verified against `manifest.json`, and never copied into this
 * package. The shape evaluator is locked ajv Draft 2020-12. Framing, UTF-8,
 * duplicate keys, depth, byte limits, scope/direction, ledger and history
 * rules are enforced by this module and the server.
 */

import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { Ajv2020 } from "ajv/dist/2020.js";

import { invalidRequest } from "./errors.js";
import { DEFAULT_LIMITS, isObject, LIMIT_KEYS, type Budgets, type Limits } from "./limits.js";
import { parseStrictJson, StrictJsonError } from "./strict-json.js";

export type Sender = "host" | "runtime";
export type Phase = "bootstrap" | "negotiated";
export type JsonObject = Record<string, unknown>;

export interface Scope {
  readonly sessionId: string | null;
  readonly turnId: string | null;
  readonly workflowRunId: string | null;
}

export interface RequestFrame {
  readonly protocolVersion: string;
  readonly kind: "request";
  readonly requestId: string;
  readonly scope: Scope;
  readonly method: string;
  readonly params: JsonObject;
  readonly extensions?: JsonObject;
}

export interface ResponseOkFrame {
  readonly protocolVersion: string;
  readonly kind: "response";
  readonly requestId: string;
  readonly scope: Scope;
  readonly outcome: "ok";
  readonly result: JsonObject;
  readonly extensions?: JsonObject;
}

export interface ResponseErrorFrame {
  readonly protocolVersion: string;
  readonly kind: "response";
  readonly requestId: string;
  readonly scope: Scope;
  readonly outcome: "error";
  readonly error: {
    readonly code: string;
    readonly message: string;
    readonly retry: string;
    readonly details: JsonObject;
  };
  readonly extensions?: JsonObject;
}

export type ResponseFrame = ResponseOkFrame | ResponseErrorFrame;

export interface EventFrame {
  readonly protocolVersion: string;
  readonly kind: "event";
  readonly requestId: string | null;
  readonly scope: Scope;
  readonly sequence: number;
  readonly event: string;
  readonly data: JsonObject;
  readonly extensions?: JsonObject;
}

export type Frame = RequestFrame | ResponseFrame | EventFrame;

export type DecodeResult =
  | { readonly kind: "frame"; readonly frame: Frame }
  | { readonly kind: "request_error"; readonly code: "INVALID_REQUEST" | "METHOD_NOT_FOUND"; readonly header: RequestFrame }
  | { readonly kind: "transport_error"; readonly reason: string };

export const RUNTIME_SCOPE: Scope = Object.freeze({ sessionId: null, turnId: null, workflowRunId: null });

const HOST_METHODS = new Set([
  "runtime.hello",
  "session.start",
  "turn.start",
  "turn.cancel",
  "session.reset",
  "runtime.shutdown",
]);
const RUNTIME_METHODS = new Set([
  "connection.describe",
  "model.generate",
  "agent.run",
  "validation.submit",
  "validation.status",
  "validation.report",
  "validation.cancel",
]);
const HOST_EVENTS = new Set(["connection.state", "agent.progress", "model.delta"]);
const RUNTIME_EVENTS = new Set(["runtime.ready", "workflow.stage", "workflow.text", "workflow.plan", "workflow.tool", "turn.finished"]);

export function knownMethod(method: string): boolean {
  return HOST_METHODS.has(method) || RUNTIME_METHODS.has(method) || method === "request.cancel";
}

export function scopeAllowed(methodOrEvent: string, scope: Scope): boolean {
  const turn = scope.sessionId !== null && scope.turnId !== null && scope.workflowRunId !== null;
  const session = scope.sessionId !== null && scope.turnId === null && scope.workflowRunId === null;
  const runtime = scope.sessionId === null && scope.turnId === null && scope.workflowRunId === null;
  if (methodOrEvent === "runtime.hello" || methodOrEvent === "runtime.shutdown" || methodOrEvent === "runtime.ready") {
    return runtime;
  }
  if (methodOrEvent === "session.start" || methodOrEvent === "session.reset") {
    return session;
  }
  if (methodOrEvent === "request.cancel") {
    return runtime || session || turn;
  }
  if (methodOrEvent === "connection.describe" || methodOrEvent === "validation.status" || methodOrEvent === "validation.report" || methodOrEvent === "validation.cancel" || methodOrEvent === "connection.state") {
    return session || turn;
  }
  return turn;
}

function directionAllowed(method: string, sender: Sender): boolean {
  if (method === "request.cancel") {
    return true;
  }
  return sender === "host" ? HOST_METHODS.has(method) : RUNTIME_METHODS.has(method);
}

function eventDirectionAllowed(event: string, sender: Sender): boolean {
  return sender === "host" ? HOST_EVENTS.has(event) : RUNTIME_EVENTS.has(event);
}

const ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;
const REQUEST_ID_PATTERN = /^[hr]:[A-Za-z0-9._:-]{0,126}$/u;

export function isId(value: unknown): value is string {
  return typeof value === "string" && ID_PATTERN.test(value);
}

export function isRequestId(value: unknown): value is string {
  return typeof value === "string" && REQUEST_ID_PATTERN.test(value);
}

export function readScope(value: unknown): Scope | null {
  if (!isObject(value)) {
    return null;
  }
  const keys = Object.keys(value).sort();
  if (keys.length !== 3 || keys[0] !== "sessionId" || keys[1] !== "turnId" || keys[2] !== "workflowRunId") {
    return null;
  }
  const { sessionId, turnId, workflowRunId } = value;
  const nullableId = (item: unknown): item is string | null => item === null || isId(item);
  if (!nullableId(sessionId) || !nullableId(turnId) || !nullableId(workflowRunId)) {
    return null;
  }
  if (sessionId === null && (turnId !== null || workflowRunId !== null)) {
    return null;
  }
  if (turnId === null && workflowRunId !== null) {
    return null;
  }
  return { sessionId, turnId, workflowRunId };
}

export function scopesEqual(left: Scope, right: Scope): boolean {
  return left.sessionId === right.sessionId && left.turnId === right.turnId && left.workflowRunId === right.workflowRunId;
}

export function runtimeScope(): Scope {
  return { ...RUNTIME_SCOPE };
}

export function sessionScope(sessionId: string): Scope {
  return { sessionId, turnId: null, workflowRunId: null };
}

export function turnScope(sessionId: string, turnId: string, workflowRunId: string): Scope {
  return { sessionId, turnId, workflowRunId };
}

export interface SchemaCase {
  readonly id: string;
  readonly path: string;
  readonly kind: "frame" | "data" | "transcript" | "bytes";
  readonly schemaRef: string | null;
  readonly phase: string;
  readonly sender: Sender;
  readonly shapeExpected: "valid" | "invalid" | "not_applicable";
  readonly handlingExpected: "accept" | "invalid_request" | "method_not_found" | "transport_error" | "transcript";
  readonly errorCode: string | null;
}

interface ManifestEntry {
  readonly path: string;
  readonly sizeBytes: number;
  readonly digest: string;
}

interface Manifest {
  readonly schemaVersion: string;
  readonly protocolVersion: string;
  readonly files: readonly ManifestEntry[];
}

function sha256(bytes: Buffer): string {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

function safeRelativePath(path: string): boolean {
  if (path.length === 0 || path.includes("\\") || path.includes("\0") || path.includes(":")) {
    return false;
  }
  return path.split("/").every((part) => part.length > 0 && part !== "." && part !== "..");
}

function findBundleRoot(): string {
  const override = process.env["SOLIDCRAFT_MASTRA_SCHEMA"];
  if (override !== undefined && override.length > 0) {
    return resolve(override);
  }
  const moduleDirectory = dirname(fileURLToPath(import.meta.url));
  const starts = [process.cwd(), moduleDirectory];
  for (const start of starts) {
    let current = resolve(start);
    for (;;) {
      const candidate = join(current, "schema", "mastra-runtime", "v1");
      if (existsSync(join(candidate, "manifest.json"))) {
        return candidate;
      }
      const parent = dirname(current);
      if (parent === current) {
        break;
      }
      current = parent;
    }
  }
  throw new Error("schema/mastra-runtime/v1/manifest.json was not found from the current directory");
}

interface FileEntry {
  readonly path: string;
  readonly bytes: Buffer;
}

function inventory(root: string): FileEntry[] {
  const files: FileEntry[] = [];
  const walk = (directory: string): void => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const absolute = join(directory, entry.name);
      const name = relative(root, absolute).split(sep).join("/");
      if (!safeRelativePath(name)) {
        throw new Error(`unsafe bundle path ${JSON.stringify(name)}`);
      }
      if (entry.isSymbolicLink()) {
        throw new Error(`symlink in schema bundle: ${name}`);
      }
      if (entry.isDirectory()) {
        walk(absolute);
      } else if (entry.isFile()) {
        if (name !== "manifest.json") {
          files.push({ path: name, bytes: readFileSync(absolute) });
        }
      } else {
        throw new Error(`non-file in schema bundle: ${name}`);
      }
    }
  };
  walk(root);
  files.sort((left, right) => Buffer.compare(Buffer.from(left.path, "utf8"), Buffer.from(right.path, "utf8")));
  return files;
}

export interface BundleSummary {
  readonly root: string;
  readonly bundleDigest: string;
  readonly files: number;
  readonly cases: number;
}

export class SchemaBundle {
  readonly root: string;
  readonly bundleDigest: string;
  readonly summary: BundleSummary;
  readonly #ajv: Ajv2020;
  readonly #schema: JsonObject;
  readonly #validators = new Map<string, (value: unknown) => boolean>();
  readonly #cases: readonly SchemaCase[];

  private constructor(root: string, manifestBytes: Buffer, schema: JsonObject, cases: readonly SchemaCase[], files: number) {
    this.root = root;
    this.bundleDigest = sha256(manifestBytes);
    this.#schema = schema;
    this.#cases = cases;
    this.summary = { root, bundleDigest: this.bundleDigest, files, cases: cases.length };
    this.#ajv = new Ajv2020({ strict: false, allErrors: false, validateFormats: false });
    this.#ajv.addSchema(schema, String(schema["$id"]));
  }

  static load(root?: string): SchemaBundle {
    const bundleRoot = root === undefined ? findBundleRoot() : resolve(root);
    const manifestPath = join(bundleRoot, "manifest.json");
    if (!existsSync(manifestPath)) {
      throw new Error(`schema bundle manifest is missing at ${manifestPath}`);
    }
    const manifestBytes = readFileSync(manifestPath);
    const manifest = parseStrictJson(manifestBytes.toString("utf8"), DEFAULT_LIMITS.jsonDepth) as Manifest;
    if (manifest.schemaVersion !== "1.0" || manifest.protocolVersion !== "1.0") {
      throw new Error("schema bundle has an unsupported version");
    }
    const files = inventory(bundleRoot);
    const byPath = new Map(files.map((entry) => [entry.path, entry.bytes]));
    let previous: string | null = null;
    for (const entry of manifest.files) {
      if (!safeRelativePath(entry.path) || entry.path === "manifest.json") {
        throw new Error(`unsafe manifest path ${JSON.stringify(entry.path)}`);
      }
      if (previous !== null && !(Buffer.from(previous, "utf8") < Buffer.from(entry.path, "utf8"))) {
        throw new Error(`manifest paths are not bytewise sorted: ${entry.path}`);
      }
      previous = entry.path;
      const bytes = byPath.get(entry.path);
      if (bytes === undefined) {
        throw new Error(`manifest names a missing file: ${entry.path}`);
      }
      if (bytes.length !== entry.sizeBytes) {
        throw new Error(`manifest size mismatch for ${entry.path}`);
      }
      if (sha256(bytes) !== entry.digest) {
        throw new Error(`manifest digest mismatch for ${entry.path}`);
      }
    }
    if (byPath.size !== manifest.files.length) {
      throw new Error("schema bundle contains an unlisted artifact");
    }
    const schemaBytes = byPath.get("protocol.schema.json");
    const casesBytes = byPath.get("cases.json");
    if (schemaBytes === undefined || casesBytes === undefined) {
      throw new Error("schema bundle is missing protocol.schema.json or cases.json");
    }
    if (parseStrictJson(manifestBytes.toString("utf8"), DEFAULT_LIMITS.jsonDepth) === null) {
      throw new Error("invalid manifest");
    }
    const schema = parseStrictJson(schemaBytes.toString("utf8"), DEFAULT_LIMITS.jsonDepth);
    const catalog = parseStrictJson(casesBytes.toString("utf8"), DEFAULT_LIMITS.jsonDepth);
    if (!isObject(schema) || !isObject(catalog) || !Array.isArray(catalog["cases"])) {
      throw new Error("schema bundle contents are invalid");
    }
    const cases = catalog["cases"];
    const parsedCases: SchemaCase[] = [];
    let previousId: string | null = null;
    for (const item of cases) {
      if (!isObject(item)) {
        throw new Error("catalog case is not an object");
      }
      const candidate = item as unknown as SchemaCase;
      if (previousId !== null && !(previousId < candidate.id)) {
        throw new Error(`catalog cases are not sorted: ${candidate.id}`);
      }
      previousId = candidate.id;
      parsedCases.push(candidate);
    }
    return new SchemaBundle(bundleRoot, manifestBytes, schema, parsedCases, files.length);
  }

  get cases(): readonly SchemaCase[] {
    return this.#cases;
  }

  get schema(): JsonObject {
    return this.#schema;
  }

  validate(schemaRef: string, value: unknown): boolean {
    let validator = this.#validators.get(schemaRef);
    if (validator === undefined) {
      if (!schemaRef.startsWith("#/$defs/")) {
        throw new Error(`non-local schema reference ${schemaRef}`);
      }
      const id = String(this.#schema["$id"]);
      const compiled = this.#ajv.compile({ $ref: `${id}${schemaRef}` });
      validator = (candidate: unknown): boolean => compiled(candidate) === true;
      this.#validators.set(schemaRef, validator);
    }
    return validator(value);
  }

  read(path: string): Buffer {
    if (!safeRelativePath(path)) {
      throw new Error(`unsafe bundle path ${JSON.stringify(path)}`);
    }
    return readFileSync(join(this.root, path));
  }
}

export function decodeFrameBytes(bytes: Buffer, phase: Phase, sender: Sender, limits: Limits, bundle: SchemaBundle): DecodeResult {
  if (bytes.length > limits.frameBytes) {
    return { kind: "transport_error", reason: "frame exceeds frameBytes" };
  }
  if (bytes.length === 0 || bytes[bytes.length - 1] !== 0x0a) {
    return { kind: "transport_error", reason: "frame is not LF terminated" };
  }
  const payload = bytes.subarray(0, bytes.length - 1);
  if (payload.includes(0x0a)) {
    return { kind: "transport_error", reason: "frame contains more than one line" };
  }
  if (payload[payload.length - 1] === 0x0d) {
    return { kind: "transport_error", reason: "frame uses CRLF" };
  }
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(payload);
  } catch {
    return { kind: "transport_error", reason: "frame is not valid UTF-8" };
  }
  let value: unknown;
  try {
    value = parseStrictJson(text, limits.jsonDepth);
  } catch (error) {
    return { kind: "transport_error", reason: error instanceof StrictJsonError ? error.message : "frame is not strict JSON" };
  }
  return decodeValue(value, phase, sender, limits, bundle);
}

export function decodeValue(value: unknown, phase: Phase, sender: Sender, limits: Limits, bundle: SchemaBundle): DecodeResult {
  if (!isObject(value)) {
    return { kind: "transport_error", reason: "frame is not an object" };
  }
  const kind = value["kind"];
  if (kind === "response") {
    return decodeResponse(value, phase, sender, limits, bundle);
  }
  if (kind === "request") {
    return decodeRequest(value, phase, sender, limits, bundle);
  }
  if (kind === "event") {
    return decodeEvent(value, phase, sender, limits, bundle);
  }
  return { kind: "transport_error", reason: "frame kind is not request, response or event" };
}

function onlyKeys(object: JsonObject, allowed: readonly string[]): boolean {
  const allowedSet = new Set(allowed);
  return Object.keys(object).every((key) => allowedSet.has(key));
}

function protocolVersionOk(value: unknown, phase: Phase): value is string {
  return typeof value === "string" && (phase === "bootstrap" || value === "1.0");
}

function responseRequestIdSender(sender: Sender): Sender {
  return sender === "host" ? "runtime" : "host";
}

function requestIdPrefix(value: string): Sender | null {
  if (value.startsWith("h:")) {
    return "host";
  }
  if (value.startsWith("r:")) {
    return "runtime";
  }
  return null;
}

function decodeResponse(value: JsonObject, phase: Phase, sender: Sender, limits: Limits, bundle: SchemaBundle): DecodeResult {
  const outcome = value["outcome"];
  const allowed =
    outcome === "ok"
      ? ["protocolVersion", "kind", "scope", "requestId", "outcome", "result", "extensions"]
      : ["protocolVersion", "kind", "scope", "requestId", "outcome", "error", "extensions"];
  if (!onlyKeys(value, allowed)) {
    return { kind: "transport_error", reason: "response has undeclared fields" };
  }
  if (!protocolVersionOk(value["protocolVersion"], phase)) {
    return { kind: "transport_error", reason: "response protocolVersion is not 1.0" };
  }
  const scope = readScope(value["scope"]);
  if (scope === null) {
    return { kind: "transport_error", reason: "response scope is invalid" };
  }
  const requestId = value["requestId"];
  if (!isRequestId(requestId) || requestIdPrefix(requestId) !== responseRequestIdSender(sender)) {
    return { kind: "transport_error", reason: "response requestId has the wrong sender prefix" };
  }
  if (outcome === "ok") {
    if (!isObject(value["result"])) {
      return { kind: "transport_error", reason: "response result is not an object" };
    }
  } else if (outcome === "error") {
    if (!isObject(value["error"])) {
      return { kind: "transport_error", reason: "response error is not an object" };
    }
  } else {
    return { kind: "transport_error", reason: "response outcome is invalid" };
  }
  if (value["extensions"] !== undefined && !isObject(value["extensions"])) {
    return { kind: "transport_error", reason: "extensions must be an object" };
  }
  const schemaRef = phase === "bootstrap" ? "#/$defs/BootstrapFrame" : "#/$defs/Frame";
  if (!bundle.validate(schemaRef, value)) {
    return { kind: "transport_error", reason: "response does not match the protocol schema" };
  }
  return { kind: "frame", frame: value as unknown as ResponseFrame };
}

function decodeRequest(value: JsonObject, phase: Phase, sender: Sender, limits: Limits, bundle: SchemaBundle): DecodeResult {
  if (!onlyKeys(value, ["protocolVersion", "kind", "scope", "requestId", "method", "params", "extensions"])) {
    return { kind: "transport_error", reason: "request has undeclared fields" };
  }
  if (!protocolVersionOk(value["protocolVersion"], phase)) {
    return { kind: "transport_error", reason: "request protocolVersion is not 1.0" };
  }
  const scope = readScope(value["scope"]);
  if (scope === null) {
    return { kind: "transport_error", reason: "request scope is invalid" };
  }
  const requestId = value["requestId"];
  if (!isRequestId(requestId) || requestIdPrefix(requestId) !== sender) {
    return { kind: "transport_error", reason: "request requestId has the wrong sender prefix" };
  }
  const method = value["method"];
  if (typeof method !== "string" || !isObject(value["params"])) {
    return { kind: "transport_error", reason: "request method or params is invalid" };
  }
  if (value["extensions"] !== undefined && !isObject(value["extensions"])) {
    return { kind: "transport_error", reason: "extensions must be an object" };
  }
  const header: RequestFrame = {
    protocolVersion: String(value["protocolVersion"]),
    kind: "request",
    requestId,
    scope,
    method,
    params: value["params"],
    ...(value["extensions"] === undefined ? {} : { extensions: value["extensions"] as JsonObject }),
  };
  if (!knownMethod(method)) {
    return { kind: "request_error", code: "METHOD_NOT_FOUND", header };
  }
  if (phase === "bootstrap" && method !== "runtime.hello") {
    return { kind: "transport_error", reason: "bootstrap accepts only runtime.hello" };
  }
  if (!scopeAllowed(method, scope) || !directionAllowed(method, sender)) {
    return { kind: "transport_error", reason: "request scope or direction is invalid" };
  }
  const schemaRef = phase === "bootstrap" ? "#/$defs/BootstrapFrame" : "#/$defs/Frame";
  if (!bundle.validate(schemaRef, value)) {
    return { kind: "request_error", code: "INVALID_REQUEST", header };
  }
  const problem = validateFrameData(header, limits);
  if (problem !== null) {
    return { kind: "request_error", code: "INVALID_REQUEST", header };
  }
  return { kind: "frame", frame: header };
}

function decodeEvent(value: JsonObject, phase: Phase, sender: Sender, limits: Limits, bundle: SchemaBundle): DecodeResult {
  if (phase !== "negotiated") {
    return { kind: "transport_error", reason: "events are not allowed during bootstrap" };
  }
  if (!onlyKeys(value, ["protocolVersion", "kind", "scope", "sequence", "event", "requestId", "data", "extensions"])) {
    return { kind: "transport_error", reason: "event has undeclared fields" };
  }
  if (value["protocolVersion"] !== "1.0") {
    return { kind: "transport_error", reason: "event protocolVersion is not 1.0" };
  }
  const scope = readScope(value["scope"]);
  if (scope === null) {
    return { kind: "transport_error", reason: "event scope is invalid" };
  }
  const event = value["event"];
  if (typeof event !== "string") {
    return { kind: "transport_error", reason: "event name is invalid" };
  }
  const requestId = value["requestId"];
  if (requestId !== null && !isId(requestId)) {
    return { kind: "transport_error", reason: "event requestId is invalid" };
  }
  const sequence = value["sequence"];
  if (typeof sequence !== "number" || !Number.isInteger(sequence) || sequence < 1 || sequence > 2_147_483_647) {
    return { kind: "transport_error", reason: "event sequence is invalid" };
  }
  if (!isObject(value["data"])) {
    return { kind: "transport_error", reason: "event data is not an object" };
  }
  if (value["extensions"] !== undefined && !isObject(value["extensions"])) {
    return { kind: "transport_error", reason: "extensions must be an object" };
  }
  if (!scopeAllowed(event, scope) || !eventDirectionAllowed(event, sender)) {
    return { kind: "transport_error", reason: "event scope or direction is invalid" };
  }
  if (!bundle.validate("#/$defs/Frame", value)) {
    return { kind: "transport_error", reason: "event does not match the protocol schema" };
  }
  const frame = value as unknown as EventFrame;
  if (validateFrameData(frame, limits) !== null) {
    return { kind: "transport_error", reason: "event exceeds a byte or list limit" };
  }
  return { kind: "frame", frame };
}

/** Encode one frame as a single LF-terminated UTF-8 line. */
export function encodeFrame(frame: Frame): Buffer {
  return Buffer.from(`${JSON.stringify(frame)}\n`, "utf8");
}

export function utf8Bytes(text: string): number {
  return Buffer.byteLength(text, "utf8");
}

function jsonBytes(value: unknown): number {
  return utf8Bytes(JSON.stringify(value));
}

function textBlocks(blocks: unknown, limits: Limits): boolean {
  if (!Array.isArray(blocks)) {
    return true;
  }
  for (const block of blocks) {
    if (isObject(block) && block["type"] === "text" && typeof block["text"] === "string") {
      if (utf8Bytes(block["text"]) > limits.textBytes) {
        return false;
      }
    }
  }
  return true;
}

function validateMessages(messages: readonly unknown[]): boolean {
  const pending = new Set<string>();
  const allCalls = new Set<string>();
  for (const item of messages) {
    if (!isObject(item)) {
      return false;
    }
    const role = item["role"];
    const toolCalls = item["toolCalls"];
    const toolCallId = item["toolCallId"];
    if (!Array.isArray(toolCalls)) {
      return false;
    }
    if (role !== "assistant" && toolCalls.length > 0) {
      return false;
    }
    if (role === "tool") {
      if (typeof toolCallId !== "string" || !pending.delete(toolCallId)) {
        return false;
      }
    } else {
      if (toolCallId !== null || (pending.size > 0 && role === "user")) {
        return false;
      }
      for (const call of toolCalls) {
        if (!isObject(call) || typeof call["id"] !== "string" || allCalls.has(call["id"])) {
          return false;
        }
        allCalls.add(call["id"]);
        pending.add(call["id"]);
      }
    }
  }
  return pending.size === 0;
}

function uniqueIds(items: readonly unknown[], field: string, sorted: boolean): boolean {
  const seen = new Set<string>();
  let previous: string | null = null;
  for (const item of items) {
    if (!isObject(item) || typeof item[field] !== "string") {
      return false;
    }
    const id = item[field];
    if (seen.has(id) || (sorted && previous !== null && previous >= id)) {
      return false;
    }
    seen.add(id);
    previous = id;
  }
  return true;
}

function finiteNumber(value: unknown, nonnegative: boolean): boolean {
  return typeof value === "number" && Number.isFinite(value) && (!nonnegative || value >= 0);
}

function validateManifest(manifest: JsonObject, limits: Limits): boolean {
  const inputs = manifest["inputs"];
  if (!Array.isArray(inputs) || inputs.length === 0 || inputs.length > limits.snapshotInputs) {
    return false;
  }
  if (jsonBytes(manifest) > limits.manifestBytes) {
    return false;
  }
  let previous: string | null = null;
  const dependencyIds = new Set<string>();
  let designs = 0;
  let bytes = 0;
  for (const input of inputs) {
    if (!isObject(input)) {
      return false;
    }
    const path = input["path"];
    const kind = input["kind"];
    const dependencyId = input["dependencyId"];
    const size = input["sizeBytes"];
    if (typeof path !== "string" || path.length > 240 || path.includes("\\") || path.includes("\0")) {
      return false;
    }
    if (path.split("/").some((part) => part.length === 0 || part === "." || part === "..")) {
      return false;
    }
    if (previous !== null && !(previous < path)) {
      return false;
    }
    previous = path;
    if (typeof dependencyId !== "string" || dependencyIds.has(dependencyId)) {
      return false;
    }
    dependencyIds.add(dependencyId);
    if (typeof size !== "number" || !Number.isSafeInteger(size) || size < 0) {
      return false;
    }
    bytes += size;
    if (kind === "design") {
      designs += 1;
      if (!path.endsWith(".sc")) {
        return false;
      }
    }
  }
  return designs === 1 && bytes <= limits.snapshotBytes;
}

function validateChecks(map: JsonObject): boolean {
  const checks = map["checks"];
  if (!Array.isArray(checks)) {
    return true;
  }
  const profile = Object.hasOwn(map, "profileId");
  if (profile && checks.length === 0) {
    return false;
  }
  if (!uniqueIds(checks, "checkId", profile)) {
    return false;
  }
  if (!profile) {
    return true;
  }
  for (const check of checks) {
    if (!isObject(check)) {
      return false;
    }
    const kind = check["kind"];
    const parameters = check["parameters"];
    if (!isObject(parameters)) {
      return false;
    }
    if (kind === "input.integrity" || kind === "native.rebuild") {
      if (Object.keys(parameters).length !== 0) {
        return false;
      }
    } else if (kind === "parameters.required") {
      const requirements = parameters["requirements"];
      if (!Array.isArray(requirements) || requirements.length < 1 || requirements.length > 64) {
        return false;
      }
      if (requirements.some((item) => !isObject(item) || !finiteNumber(item["value"], false) || !finiteNumber(item["absTolerance"], true))) {
        return false;
      }
    } else if (kind === "geometry.exact") {
      const measurements = parameters["measurements"];
      if (!Array.isArray(measurements) || measurements.length < 1 || measurements.length > 64) {
        return false;
      }
      if (
        measurements.some(
          (item) =>
            !isObject(item) ||
            !finiteNumber(item["expected"], false) ||
            !finiteNumber(item["absTolerance"], true) ||
            !finiteNumber(item["relTolerance"], true),
        )
      ) {
        return false;
      }
    }
  }
  return true;
}

/** Explicit checks that survive JSON Schema: byte caps, identity and opaque-data scope. */
export function declaredChecks(value: unknown, key: string, limits: Limits): boolean {
  if ((key === "arguments" || key === "inputSchema" || key === "outputSchema") && value !== null) {
    return jsonBytes(value) <= limits.toolJsonBytes;
  }
  if (key === "params" || key === "parameters" || key === "expected" || key === "actual" || key === "measured" || key === "output" || key === "selection") {
    return true;
  }
  if (typeof value === "number") {
    if (key === "temperature") {
      return Number.isFinite(value) && value >= 0 && value <= 2;
    }
    if (key === "value" || key === "absTolerance" || key === "relTolerance") {
      return Number.isFinite(value) && (key === "value" || value >= 0);
    }
    if (!Number.isInteger(value) || value < 0 || value > 2_147_483_647) {
      return false;
    }
    if (key === "timeoutSeconds" && value === 0) {
      return false;
    }
    if (key === "maxOutputTokens" && (value < 1 || value > 131_072)) {
      return false;
    }
  } else if (typeof value === "string") {
    if (key === "reasoningEffort" && utf8Bytes(value) > 64) {
      return false;
    }
  } else if (Array.isArray(value)) {
    const cap =
      key === "messages"
        ? limits.modelMessages
        : key === "tools"
          ? limits.toolDefinitions
          : key === "toolCalls"
            ? limits.toolCalls
            : key === "steps"
              ? 64
              : key === "inputs"
                ? limits.snapshotInputs
                : key === "checks"
                  ? limits.checks
                  : key === "evidence"
                    ? limits.evidenceArtifacts
                    : Number.MAX_SAFE_INTEGER;
    if (value.length > cap) {
      return false;
    }
    for (const child of value) {
      if (!declaredChecks(child, "", limits)) {
        return false;
      }
    }
    if (key === "messages" && !validateMessages(value)) {
      return false;
    }
  } else if (isObject(value)) {
    for (const [childKey, child] of Object.entries(value)) {
      if (!declaredChecks(child, childKey, limits)) {
        return false;
      }
    }
    if (isObject(value["limits"])) {
      const limitsValue = value["limits"];
      for (const limitKey of LIMIT_KEYS) {
        if (!(limitKey in limitsValue)) {
          return false;
        }
      }
    }
    if (isObject(value["budgets"])) {
      const budgets = value["budgets"] as unknown as Budgets;
      if (
        budgets.modelCalls > 20 ||
        budgets.agentTasks > 4 ||
        budgets.cadCommands > 64 ||
        budgets.repairs > 2 ||
        budgets.validationRuns > 3 ||
        budgets.wallTimeSeconds > 1_200
      ) {
        return false;
      }
    }
    if (value["selectedVersion"] !== undefined && value["selectedVersion"] !== "1.0") {
      return false;
    }
    if (value["inputs"] !== undefined && !validateManifest(value, limits)) {
      return false;
    }
    if (!validateChecks(value)) {
      return false;
    }
    if (Array.isArray(value["steps"]) && !uniqueIds(value["steps"], "id", false)) {
      return false;
    }
    if (Array.isArray(value["toolCalls"]) && !uniqueIds(value["toolCalls"], "id", false)) {
      return false;
    }
  }
  return true;
}

function validateTextForFrame(frame: Frame, limits: Limits): boolean {
  if (frame.kind === "request") {
    if (frame.method === "turn.start" && !textBlocks(frame.params["parts"], limits)) {
      return false;
    }
    if (frame.method === "model.generate") {
      const messages = frame.params["messages"];
      if (Array.isArray(messages)) {
        for (const message of messages) {
          if (!isObject(message) || !textBlocks(message["content"], limits)) {
            return false;
          }
        }
      }
    }
    if (frame.method === "agent.run" && !textBlocks(frame.params["task"], limits)) {
      return false;
    }
  } else if (frame.kind === "event") {
    const data = frame.data;
    if (frame.event === "workflow.text" || frame.event === "model.delta") {
      if (typeof data["text"] === "string" && utf8Bytes(data["text"]) > limits.textBytes) {
        return false;
      }
    }
    if (frame.event === "agent.progress" && data["kind"] === "text") {
      const content = data["content"];
      if (isObject(content) && typeof content["text"] === "string" && utf8Bytes(content["text"]) > limits.textBytes) {
        return false;
      }
    }
  }
  return true;
}

function validateFrameData(frame: Frame, limits: Limits): string | null {
  const body = frame.kind === "request" ? frame.params : frame.kind === "event" ? frame.data : undefined;
  if (body !== undefined && !declaredChecks(body, "", limits)) {
    return "declared field limit";
  }
  if (!validateTextForFrame(frame, limits)) {
    return "text byte limit";
  }
  return null;
}

/** Explicit data-layer validation used by the conformance runner and reply decoder. */
export function validateDataValue(schemaRef: string, value: unknown, limits: Limits): boolean {
  if (!declaredChecks(value, "", limits)) {
    return false;
  }
  const definition = schemaRef.replace("#/$defs/", "");
  if (definition === "WireError") {
    if (!isObject(value) || typeof value["message"] !== "string" || utf8Bytes(value["message"]) > 2048) {
      return false;
    }
  }
  if (definition === "SnapshotManifest") {
    if (!isObject(value) || !validateManifest(value, limits)) {
      return false;
    }
  }
  if (definition === "CheckProfile") {
    if (!isObject(value) || !validateChecks(value)) {
      return false;
    }
  }
  if (definition === "ModelGenerateResult" || definition === "AgentRunResult") {
    if (!isObject(value) || !textBlocks(value["content"], limits)) {
      return false;
    }
  }
  return true;
}

/** Map a data definition to the method that owns its decoder, for text limits. */
export function checkProtocolShape(bundle: SchemaBundle, schemaRef: string, raw: Buffer, limits: Limits): boolean {
  let value: unknown;
  try {
    value = parseStrictJson(raw.toString("utf8"), limits.jsonDepth);
  } catch {
    return false;
  }
  return bundle.validate(schemaRef, value);
}

export function parseData(raw: Buffer, limits: Limits): unknown {
  return parseStrictJson(raw.toString("utf8"), limits.jsonDepth);
}

export function validateParsedData(bundle: SchemaBundle, schemaRef: string, value: unknown, limits: Limits): boolean {
  return bundle.validate(schemaRef, value) && validateDataValue(schemaRef, value, limits);
}

export function asRequest(value: unknown): RequestFrame {
  if (!isObject(value)) {
    throw invalidRequest("expected a request frame");
  }
  return value as unknown as RequestFrame;
}

export function isRelativeSafePath(path: string): boolean {
  return safeRelativePath(path) && !isAbsolute(path);
}

export type { Limits, Budgets };
