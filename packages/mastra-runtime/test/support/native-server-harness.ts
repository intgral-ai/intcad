/**
 * Host-side process peer for the #10 server integration tests.
 *
 * Spawns the built `dist/src/main.js` with an inherited MCP bridge fd and a
 * host-frozen requirements file, speaks protocol 1.0 on stdin/stdout, and
 * serves the SolidCraft MCP bridge on fd 3 with the deterministic peer.
 */

import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type { Duplex } from "node:stream";

import type { TestContext } from "node:test";

import { DEFAULT_LIMITS } from "../../src/limits.js";
import { streamChannel, type McpBridgeChannel } from "../../src/solidcraft/mcp-client.js";
import type { JsonObject, Scope } from "../../src/protocol.js";
import {
  FakeMcpPeer,
  defaultCommands,
  defaultTools,
  guardedReply,
  textReply,
  type ToolReply,
} from "./mcp-peer.js";

const MAIN = fileURLToPath(new URL("../../src/main.js", import.meta.url));

export interface Frame {
  readonly protocolVersion: string;
  readonly kind: "request" | "response" | "event";
  readonly requestId?: string | null;
  readonly method?: string;
  readonly event?: string;
  readonly scope?: Scope;
  readonly outcome?: string;
  readonly result?: JsonObject;
  readonly error?: JsonObject;
  readonly data?: JsonObject;
  readonly sequence?: number;
  readonly params?: JsonObject;
}

export interface NativeParameter {
  expression: string;
  value: number;
}

export interface NativeBody {
  name: string;
  volume_mm3: number;
  bbox: { min: number[]; max: number[] };
  faces: number;
}

export interface NativeFeature {
  id: string;
  name: string;
  type: string;
}

export interface NativeState {
  parameters: Map<string, NativeParameter>;
  bodies: NativeBody[];
  timeline: NativeFeature[];
}

export function editBase(): NativeState {
  return {
    parameters: new Map([
      ["width", { expression: "20", value: 20 }],
      ["depth", { expression: "10", value: 10 }],
      ["height", { expression: "5", value: 5 }],
    ]),
    bodies: [],
    timeline: [],
  };
}

export function parameterCheck(name: string, value: number, absTolerance = 1e-9): JsonObject {
  return {
    checkId: "parameters.required",
    kind: "parameters.required",
    required: true,
    parameters: { requirements: [{ name, value, absTolerance }] },
  };
}

export function parameterChecks(
  entries: readonly { name: string; value: number; absTolerance?: number }[],
): JsonObject {
  return {
    checkId: "parameters.required",
    kind: "parameters.required",
    required: true,
    parameters: {
      requirements: entries.map((entry) => ({
        name: entry.name,
        value: entry.value,
        absTolerance: entry.absTolerance ?? 1e-9,
      })),
    },
  };
}

export function volumeCheck(expected: number): JsonObject {
  return {
    checkId: "geometry.exact",
    kind: "geometry.exact",
    required: true,
    parameters: {
      measurements: [
        {
          command: "measure",
          params: {},
          resultPointer: "/total/volume_mm3",
          expected,
          absTolerance: 1e-9,
          relTolerance: 1e-9,
        },
      ],
    },
  };
}

export interface NativePeerOptions {
  /** Omit --bridge-fd to exercise a configured bridge with no channel. */
  readonly bridge?: boolean;
}

export class NativeRecordedPeer {
  readonly child: ChildProcess;
  readonly frames: Frame[] = [];
  readonly stderr: string[] = [];
  readonly exit: Promise<number>;
  readonly mcp: FakeMcpPeer;
  readonly state: NativeState;
  readonly #tempDir: string;
  readonly #consumed = new Set<Frame>();
  readonly #waiters: {
    predicate: (frame: Frame) => boolean;
    resolve: (frame: Frame) => void;
    timer: NodeJS.Timeout;
  }[] = [];
  readonly #channel: McpBridgeChannel | null;
  #closed = false;

  constructor(
    t: TestContext,
    state: NativeState,
    profiles: Record<string, JsonObject>,
    options: NativePeerOptions = {},
  ) {
    this.state = state;
    this.#tempDir = mkdtempSync(join(tmpdir(), "solidcraft-mastra-native-"));
    const requirementsFile = join(this.#tempDir, "requirements.json");
    writeFileSync(requirementsFile, JSON.stringify({ profiles }), "utf8");
    const bridge = options.bridge ?? true;
    const args = [`--requirements-file=${requirementsFile}`];
    if (bridge) {
      args.push("--bridge-fd=3");
    }
    this.child = spawn(process.execPath, [MAIN, ...args], { stdio: ["pipe", "pipe", "pipe", "pipe"] });
    this.exit = new Promise<number>((resolve) => {
      this.child.once("exit", (code) => resolve(code ?? -1));
    });
    const bridgeStream = this.child.stdio[3];
    if (bridgeStream === null) {
      throw new Error("the runtime did not receive an MCP bridge fd");
    }
    const duplex = bridgeStream as Duplex;
    this.#channel = bridge ? streamChannel(duplex, duplex) : null;
    this.mcp = new FakeMcpPeer({
      tools: defaultTools(true),
      commands: defaultCommands(),
      documentId: "doc-native",
      revision: 41n,
    });
    this.mcp.setHandler("inspect_design", () => textReply(this.designView()));
    this.mcp.setHandler("measure", () => textReply(this.measureView()));
    this.mcp.execute((args) => this.onExecute(args));
    this.mcp.batch((args) => this.onBatch(args));
    if (this.#channel !== null) {
      this.#channel.receive(
        (line) => this.mcp.send(line),
        (reason) => {
          this.stderr.push(`bridge: ${reason}`);
        },
      );
      this.mcp.receive(
        (line) => this.#channel?.send(line),
        (reason) => this.#channel?.close(reason),
      );
    }

    this.child.stdin?.on("error", () => undefined);
    let buffer = "";
    this.child.stdout?.setEncoding("utf8");
    this.child.stdout?.on("data", (chunk: string) => {
      buffer += chunk;
      for (;;) {
        const newline = buffer.indexOf("\n");
        if (newline === -1) {
          break;
        }
        const line = buffer.slice(0, newline);
        buffer = buffer.slice(newline + 1);
        if (line.length > 0) {
          this.#onFrame(JSON.parse(line) as Frame);
        }
      }
    });
    this.child.stderr?.setEncoding("utf8");
    this.child.stderr?.on("data", (chunk: string) => {
      this.stderr.push(chunk);
    });
    t.after(() => {
      this.kill();
      this.#channel?.close("the test ended");
      rmSync(this.#tempDir, { recursive: true, force: true });
    });
  }

  get revision(): string {
    return this.mcp.revision.toString();
  }

  stamp(): { documentId: string; revision: string } {
    return { documentId: this.mcp.documentId, revision: this.revision };
  }

  designView(): JsonObject {
    return {
      name: "analytic-native-fixture",
      document: this.stamp(),
      params: [...this.state.parameters.entries()].map(([name, parameter]) => ({
        name,
        expression: parameter.expression,
        unit: "mm",
        value: parameter.value,
        error: null,
      })),
      timeline: this.state.timeline.map((feature) => ({
        ...feature,
        suppressed: false,
        rolled_back: false,
        error: null,
        warning: null,
      })),
      bodies: this.state.bodies.map((body) => ({ ...body, area_mm2: 0, closed: true })),
      sketches: [],
      selection: [],
      undo_depth: 0,
    };
  }

  measureView(): JsonObject {
    const volume = this.state.bodies.reduce((total, body) => total + body.volume_mm3, 0);
    return {
      body_count: this.state.bodies.length,
      bodies: this.state.bodies.map((body) => ({ name: body.name, volume_mm3: body.volume_mm3 })),
      total: { volume_mm3: volume },
    };
  }

  onExecute(args: JsonObject): ToolReply {
    const command = typeof args["command"] === "string" ? args["command"] : "";
    if (command === "document.revision") {
      return textReply(`{\n  "document_id": "${this.mcp.documentId}",\n  "revision": ${this.revision}\n}`);
    }
    const applied = this.applyCommand(command, args["params"]);
    if (!applied.ok) {
      return textReply(applied.message, true);
    }
    this.mcp.revision += 1n;
    return guardedReply({ ...applied.result, documentStamp: this.stamp() });
  }

  onBatch(args: JsonObject): ToolReply {
    const commands = Array.isArray(args["commands"]) ? (args["commands"] as JsonObject[]) : [];
    const applied: number[] = [];
    for (const [index, step] of commands.entries()) {
      const command = typeof step["command"] === "string" ? step["command"] : "";
      const params = step["params"] === undefined ? {} : (step["params"] as JsonObject);
      const result = this.applyCommand(command, params);
      if (!result.ok) {
        return guardedReply(
          {
            ok: false,
            completed: index,
            applied,
            not_run: commands.slice(index + 1).map((_, offset) => index + 1 + offset),
            failed: { index, error: result.message },
            rolled_back: true,
            documentStamp: this.stamp(),
          },
          true,
        );
      }
      applied.push(index);
    }
    this.mcp.revision += 1n;
    return guardedReply({
      ok: true,
      completed: commands.length,
      applied,
      results: [],
      documentStamp: this.stamp(),
    });
  }

  applyCommand(command: string, rawParams: unknown): { ok: true; result: JsonObject } | { ok: false; message: string } {
    const params = rawParams !== undefined && typeof rawParams === "object" && rawParams !== null ? (rawParams as JsonObject) : {};
    if (command === "solid.box") {
      const length = numberParam(params, "length");
      const width = numberParam(params, "width");
      const height = numberParam(params, "height");
      if (length === null || width === null || height === null) {
        return { ok: false, message: "solid.box needs length, width and height." };
      }
      const name = `Body${this.state.bodies.length + 1}`;
      this.state.bodies.push({
        name,
        volume_mm3: length * width * height,
        bbox: { min: [0, 0, 0], max: [length, width, height] },
        faces: 6,
      });
      this.state.timeline.push({ id: `f${this.state.timeline.length + 1}`, name: "Box", type: "box" });
      return { ok: true, result: { body: name, volume_mm3: length * width * height } };
    }
    if (command === "parameters.change") {
      const name = typeof params["name"] === "string" ? params["name"] : "";
      const expression = params["expression"];
      const value = typeof expression === "number" ? expression : parseExpression(expression);
      if (name.length === 0 || value === null) {
        return { ok: false, message: "parameters.change needs a name and numeric expression." };
      }
      this.state.parameters.set(name, { expression: String(expression), value });
      return { ok: true, result: { name, value } };
    }
    return { ok: false, message: `unknown fixture command \`${command}\`` };
  }

  #onFrame(frame: Frame): void {
    this.frames.push(frame);
    for (const waiter of [...this.#waiters]) {
      if (!this.#consumed.has(frame) && waiter.predicate(frame)) {
        this.#consumed.add(frame);
        clearTimeout(waiter.timer);
        this.#waiters.splice(this.#waiters.indexOf(waiter), 1);
        waiter.resolve(frame);
      }
    }
  }

  async waitFor(predicate: (frame: Frame) => boolean, timeoutMillis = 10_000): Promise<Frame> {
    const existing = this.frames.find((frame) => !this.#consumed.has(frame) && predicate(frame));
    if (existing !== undefined) {
      this.#consumed.add(existing);
      return existing;
    }
    return new Promise<Frame>((resolve, reject) => {
      const waiter = {
        predicate,
        resolve,
        timer: setTimeout(() => {
          this.#waiters.splice(this.#waiters.indexOf(waiter), 1);
          reject(
            new Error(
              `timed out waiting for frame; saw ${JSON.stringify(this.frames.map((frame) => frame.method ?? frame.event ?? frame.requestId))}; stderr ${this.stderr.join("").slice(0, 800)}`,
            ),
          );
        }, timeoutMillis),
      };
      this.#waiters.push(waiter);
    });
  }

  send(method: string, scope: Scope, params: JsonObject, requestId: string): void {
    this.child.stdin?.write(
      `${JSON.stringify({ protocolVersion: "1.0", kind: "request", requestId, scope, method, params })}\n`,
    );
  }

  async request(method: string, scope: Scope, params: JsonObject, requestId: string): Promise<Frame> {
    this.send(method, scope, params, requestId);
    return this.waitFor((frame) => frame.kind === "response" && frame.requestId === requestId);
  }

  replyOk(request: Frame, result: JsonObject): void {
    this.child.stdin?.write(
      `${JSON.stringify({ protocolVersion: "1.0", kind: "response", requestId: request.requestId, scope: request.scope, outcome: "ok", result })}\n`,
    );
  }

  replyText(request: Frame, text: string): void {
    this.replyOk(request, {
      content: [{ type: "text", text }],
      toolCalls: [],
      finishReason: "stop",
      usage: null,
      continuation: null,
      delivery: "buffered",
    });
  }

  replyTools(request: Frame, toolCalls: readonly JsonObject[], text = ""): void {
    this.replyOk(request, {
      content: text.length === 0 ? [] : [{ type: "text", text }],
      toolCalls: [...toolCalls],
      finishReason: "tool_calls",
      usage: null,
      continuation: null,
      delivery: "buffered",
    });
  }

  async closeInput(): Promise<number> {
    if (!this.child.stdin?.destroyed) {
      this.child.stdin?.end();
    }
    return this.exit;
  }

  kill(): void {
    if (!this.#closed) {
      this.#closed = true;
      if (!this.child.killed) {
        this.child.kill();
      }
    }
  }
}

export const RUNTIME_SCOPE: Scope = { sessionId: null, turnId: null, workflowRunId: null };
export const SESSION_SCOPE: Scope = { sessionId: "s-native", turnId: null, workflowRunId: null };

export function helloParams(): JsonObject {
  return { supportedVersions: ["1.0"], hostVersion: "0.40.0", limits: DEFAULT_LIMITS as unknown as JsonObject };
}

export function sessionParams(): JsonObject {
  return {
    workflowId: "native-modeling",
    instructions: "Use native SolidCraft features and fixed checks.",
    bridge: { name: "solidcraft-app", bridgeId: "bridge-native" },
  };
}

export function turnParams(
  turnId: string,
  workflowRunId: string,
  profileId: string,
  text: string,
  document: { documentId: string; revision: string },
  budgets: JsonObject = {
    modelCalls: 20,
    agentTasks: 4,
    cadCommands: 64,
    repairs: 2,
    validationRuns: 3,
    wallTimeSeconds: 1200,
  },
): JsonObject {
  return {
    binding: {
      workflowId: "native-modeling",
      connectionId: "byok-native",
      modelId: "model-native",
      document,
      profile: { profileId, version: "1.0.0", digest: `sha256:${"b".repeat(64)}` },
    },
    parts: [{ type: "text", text }],
    context: { selection: [], requirements: null },
    budgets,
  };
}

export function turnScope(turnId: string, workflowRunId: string): Scope {
  return { sessionId: "s-native", turnId, workflowRunId };
}

export function toolCall(id: string, name: string, args: JsonObject): JsonObject {
  return { id, name, arguments: args };
}

export function decisionEdit(): JsonObject {
  return toolCall("call-decision", "record_decision", { decision: "edit", reason: "The request is unambiguous." });
}

export function decisionAsk(): JsonObject {
  return toolCall("call-decision", "record_decision", { decision: "ask", reason: "Which dimension is protected?" });
}

export function applyCall(id: string, commands: readonly { command: string; params: JsonObject }[]): JsonObject {
  return toolCall(id, "apply_native_commands", { commands });
}

function numberParam(params: JsonObject, key: string): number | null {
  const value = params[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function parseExpression(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof value !== "string") {
    return null;
  }
  const match = /^\s*(-?[0-9]+(?:\.[0-9]+)?)/u.exec(value);
  return match === null ? null : Number(match[1]);
}
