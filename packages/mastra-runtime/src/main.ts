/**
 * Process entry point. The runtime starts from `dist/src/main.js` with only its
 * declared production dependencies; it does not shell out to another Node, npx
 * or the development checkout.
 *
 * Host provisioning additions for the native workflow (#10):
 * - `--bridge-fd <fd>` wraps one inherited duplex file descriptor as the
 *   SolidCraft MCP bridge. The host owns the descriptor; the runtime opens one
 *   MCP session per turn and never closes the lane itself before shutdown.
 * - `--requirements-file <path>` supplies the host-frozen fixed check profiles
 *   for this launch (`{"profiles": {"<id>": {"checks": [...]}}}` or a direct
 *   mapping). #13 will replace this launch seam with host profile/report
 *   delivery; without it the runtime uses no extra checks and never claims
 *   validated success.
 */

import { readFileSync } from "node:fs";
import { Socket } from "node:net";
import { pathToFileURL } from "node:url";

import { isObject } from "./limits.js";
import { SchemaBundle } from "./protocol.js";
import { RuntimeServer } from "./server.js";
import { bridgeLane } from "./bridge-lane.js";
import type { McpBridgeChannel } from "./solidcraft/mcp-client.js";
import type { FixedRequirements, ProfileRef } from "./workflows/index.js";

export interface StartOptions {
  readonly schemaRoot?: string;
  readonly bridgeFd?: number;
  readonly requirementsFile?: string;
}

type RequirementTable = Readonly<Record<string, FixedRequirements>>;

export function startRuntime(options: StartOptions = {}): RuntimeServer {
  const diagnostic = (line: string): void => {
    process.stderr.write(`${line.slice(0, 400)}\n`);
  };
  const bridgeFd = options.bridgeFd ?? argument("bridge-fd");
  const bridgeCapabilities = capabilities(argument("bridge-capabilities"));
  const requirementsFile = options.requirementsFile ?? argument("requirements-file");
  const requirements = loadRequirements(requirementsFile, diagnostic);

  let closeBridge: () => void = () => undefined;
  let cadBridge: (() => McpBridgeChannel) | undefined;
  if (bridgeFd !== null) {
    const fd = Number(bridgeFd);
    if (!Number.isInteger(fd) || fd < 0) {
      throw new Error("--bridge-fd must be a nonnegative integer");
    }
    const socket = new Socket({ fd, readable: true, writable: true });
    socket.on("error", (error: Error) => diagnostic(`bridge: ${error.message.slice(0, 200)}`));
    const lane = bridgeLane(socket, socket);
    closeBridge = () => {
      lane.close("the runtime stopped");
      socket.destroy();
    };
    cadBridge = () => lane.channel();
  }

  const bundle = SchemaBundle.load(options.schemaRoot);
  const server = new RuntimeServer({
    input: process.stdin,
    output: process.stdout,
    bundle,
    diagnostic,
    ...(cadBridge === undefined ? {} : { cadBridge }),
    bridgeCapabilities,
    fixedRequirements: async (profile: ProfileRef) => requirements[profile.profileId] ?? { checks: [] },
    onStopped: (code) => {
      closeBridge();
      process.exitCode = code;
      // A runtime.shutdown response is already queued on stdout; let the pipe
      // drain before leaving. The same orderly path covers incompatible hello.
      setImmediate(() => process.exit(code));
    },
  });
  server.start();
  return server;
}

function capabilities(value: string | null): { guardedExecute: boolean; guardedBatch: boolean } {
  const words = (value ?? "").split(",").map((word) => word.trim());
  return { guardedExecute: words.includes("execute"), guardedBatch: words.includes("batch") };
}

function argument(name: string): string | null {
  const args = process.argv.slice(2);
  const prefix = `--${name}=`;
  for (const [index, value] of args.entries()) {
    if (value.startsWith(prefix)) {
      return value.slice(prefix.length);
    }
    if (value === `--${name}`) {
      return args[index + 1] ?? null;
    }
  }
  return null;
}

function loadRequirements(path: string | null, diagnostic: (line: string) => void): RequirementTable {
  if (path === null) {
    return {};
  }
  const raw: unknown = JSON.parse(readFileSync(path, "utf8"));
  const profiles = isObject(raw) && isObject(raw["profiles"]) ? raw["profiles"] : raw;
  if (!isObject(profiles)) {
    throw new Error("the requirements file must contain a profile mapping");
  }
  const table: Record<string, FixedRequirements> = {};
  for (const [profileId, value] of Object.entries(profiles)) {
    if (!isObject(value) || !Array.isArray(value["checks"])) {
      diagnostic(`requirements: profile ${profileId.slice(0, 64)} has no check list; using no extra checks`);
      continue;
    }
    table[profileId] = { checks: value["checks"] as FixedRequirements["checks"] };
  }
  return table;
}

const entry = process.argv[1];
if (entry !== undefined && import.meta.url === pathToFileURL(entry).href) {
  try {
    startRuntime();
  } catch (error) {
    const message = error instanceof Error ? error.message : "runtime failed to start";
    process.stderr.write(`solidcraft-mastra-runtime: ${message.slice(0, 400)}\n`);
    process.exitCode = 1;
    setImmediate(() => process.exit(1));
  }
}
