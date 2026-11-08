/**
 * Process entry point. The runtime starts from `dist/src/main.js` with only its
 * declared production dependencies; it does not shell out to another Node, npx
 * or the development checkout.
 */

import { pathToFileURL } from "node:url";

import { SchemaBundle } from "./protocol.js";
import { RuntimeServer } from "./server.js";

export interface StartOptions {
  readonly schemaRoot?: string;
}

export function startRuntime(options: StartOptions = {}): RuntimeServer {
  const bundle = SchemaBundle.load(options.schemaRoot);
  const server = new RuntimeServer({
    input: process.stdin,
    output: process.stdout,
    bundle,
    diagnostic: (line) => {
      process.stderr.write(`${line.slice(0, 400)}\n`);
    },
    onStopped: (code) => {
      process.exitCode = code;
      // A runtime.shutdown response is already queued on stdout; let the pipe
      // drain before leaving. The same orderly path covers incompatible hello.
      setImmediate(() => process.exit(code));
    },
  });
  server.start();
  return server;
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
