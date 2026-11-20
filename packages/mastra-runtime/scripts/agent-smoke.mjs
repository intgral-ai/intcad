#!/usr/bin/env node
/**
 * Run the built-in agent out of a `mastra-agent` folder against stand-ins, with no account.
 *
 *   node agent-smoke.mjs <mastra-agent folder> [--only <text>] [--raw <folder>]
 *   node agent-smoke.mjs --node <node> --entry <main.mjs> [--sdk <node_modules>] [--only <text>] [--raw <folder>]
 *
 * It starts the folder's own Node with the folder's entry, as the app does, and talks ACP with
 * it: `initialize`, a custom address handed over with `_intcad/connections/setup`, then
 *   - three turns in one session, answered by a stand-in model service;
 *   - a turn in which the model asks for a command, run after one permission request;
 *   - a turn in which the model asks for a tool of an MCP server handed over in `session/new`;
 *   - a command and a file write whose permission requests are refused;
 *   - a turn cancelled while a permission request is open, and one cancelled while the answer
 *     streams.
 * A refused or cancelled turn must leave nothing behind: the command's marker file is nowhere, no
 * file of the project folder changed, and the same session answers the next message. The allowed
 * command and the allowed file write are what show that a marker would be there otherwise.
 * The model service is in this process, on a port the system picks; the MCP server is this file
 * started again with `--mcp-server`. `--only` runs the cases whose name has the text; `--raw`
 * keeps every ACP message and every request to the model service, one folder per case.
 * `--only "no-close control"` instead runs a text-only held stream without cancelling it.
 * This control must fail because the server does not see the response close.
 * `--only "early-rejection control"` runs both cancellation cases with a nonexistent
 * session for the prompt, so an ACP error arrives before the awaited event.
 *
 * Nothing of the person running it is used. The agent gets a new, empty data folder
 * (`MASTRA_APP_DATA_DIR`), which is also where Mastra keeps plan sign-ins, and the agent's entry
 * drops every provider key from its environment and takes a home of its own in that folder; no
 * keychain is read by the agent. Before the first message the run chooses the stand-in's model
 * itself, whatever model the session started on, and checks that no key is saved, no plan is
 * signed in and the model in use is the stand-in's; it stops if not.
 * After each turn it checks that the stand-in was the one asked, and after each case that the
 * session made no vector store (the agent's sessions have none: agent/session.mjs). Mastra's
 * usage reporting is off: the agent's entry switches it off itself.
 *
 * Prints one JSON line per case and exits 1 when any failed. It says the agent ran on the system
 * it was run on, and nothing about any other.
 *
 * The stand-ins and the ACP client are exported: compare-official-acp.mjs runs the same ones
 * against Mastra Code's own `mastracode --acp`.
 */

import { spawn } from "node:child_process";
import { once } from "node:events";
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const self = fileURLToPath(import.meta.url);
export const MARK = "intcad-smoke";

// ---- The stand-in MCP server (this file, started by the agent) ----------------------------------

function mcpServer(log) {
  const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
  let pending = "";
  process.stdin.on("data", (data) => {
    pending += data;
    for (let end; (end = pending.indexOf("\n")) >= 0; ) {
      const line = pending.slice(0, end);
      pending = pending.slice(end + 1);
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        continue;
      }
      if (message.method === "initialize") {
        send({ jsonrpc: "2.0", id: message.id, result: { protocolVersion: message.params?.protocolVersion ?? "2025-06-18", capabilities: { tools: {} }, serverInfo: { name: "stand-in-app", version: "0.0.0" } } });
      } else if (message.method === "tools/list") {
        send({ jsonrpc: "2.0", id: message.id, result: { tools: [{ name: "measure_part", description: "Returns the bounding box of the open part.", inputSchema: { type: "object", properties: { unit: { type: "string" } }, required: [] } }] } });
      } else if (message.method === "tools/call") {
        appendFileSync(log, `${JSON.stringify(message.params)}\n`);
        send({ jsonrpc: "2.0", id: message.id, result: { content: [{ type: "text", text: `box 120 x 80 x 2 mm ${MARK}` }], isError: false } });
      } else if (message.id != null) send({ jsonrpc: "2.0", id: message.id, result: {} });
    }
  });
}

// ---- The stand-in model service -----------------------------------------------------------------

const textOf = (message) => (typeof message?.content === "string" ? message.content : Array.isArray(message?.content) ? message.content.map((part) => part?.text ?? "").join("\n") : "");

/**
 * An OpenAI-compatible service. What it answers follows the last message of the request:
 *   "run the command: <command>"                        asks for the command tool with that command;
 *   "write the file: <path>"                            asks for the file-writing tool, the text being MARK;
 *   "measure the part"                                  asks for the MCP server's `measure_part`;
 *   "answer slowly, then run the command: <command>"    holds the answer open before the command;
 *   "hold the answer open"                              holds a text-only answer open;
 *   a tool's result                                     says whether the result had MARK in it;
 *   anything else                                       a plain sentence.
 * `keep(file, value)` is told every request, for a run that keeps its raw output.
 */
export function modelService({ keep = () => {}, status = () => 200 } = {}) {
  const seen = { requests: 0 };
  let heldResponse;
  const server = http.createServer((request, response) => {
    let body = "";
    request.on("data", (chunk) => (body += chunk));
    request.on("end", () => {
      seen.requests += 1;
      let wanted = {};
      try {
        wanted = JSON.parse(body || "{}");
      } catch {
        // an empty request is answered like any other
      }
      keep("model-requests.jsonl", { n: seen.requests, url: request.url, body: wanted });
      const refused = status(request.url);
      if (refused !== 200) {
        response.writeHead(refused, { "content-type": "application/json" });
        response.end(JSON.stringify({ error: { message: `stand-in answer ${refused}`, type: "stand_in", code: String(refused) } }));
        return;
      }
      const id = `chatcmpl-${seen.requests}`;
      const model = wanted.model ?? "hello-1";
      const chunk = (delta, finish = null) => `data: ${JSON.stringify({ id, object: "chat.completion.chunk", created: 1, model, choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`;
      const say = (text) => {
        if (!wanted.stream) {
          response.writeHead(200, { "content-type": "application/json" });
          response.end(JSON.stringify({ id, object: "chat.completion", created: 1, model, choices: [{ index: 0, message: { role: "assistant", content: text }, finish_reason: "stop" }], usage: { prompt_tokens: 5, completion_tokens: 9, total_tokens: 14 } }));
          return;
        }
        response.writeHead(200, { "content-type": "text/event-stream" });
        response.write(chunk({ role: "assistant", content: "" }));
        response.write(chunk({ content: text }));
        response.write(chunk({}, "stop"));
        response.end("data: [DONE]\n\n");
      };
      const call = (tool, args) => {
        for (const name of tool.parameters?.required ?? []) args[name] ??= "check";
        response.write(chunk({ role: "assistant", content: null, tool_calls: [{ index: 0, id: `call_${seen.requests}`, type: "function", function: { name: tool.name, arguments: JSON.stringify(args) } }] }));
        response.write(chunk({}, "tool_calls"));
        response.end("data: [DONE]\n\n");
      };
      const messages = wanted.messages ?? [];
      const last = messages[messages.length - 1] ?? {};
      if (last.role === "tool") {
        say(textOf(last).includes(MARK) || JSON.stringify(last.content ?? "").includes(MARK) ? `The tool answered with ${MARK}.` : "The tool answered without the expected text.");
        return;
      }
      const tools = (wanted.tools ?? []).map((tool) => tool.function ?? tool);
      const find = (pattern) => tools.find((tool) => pattern.test(tool.name ?? ""));
      const asked = last.role === "user" && wanted.stream ? textOf(last) : "";
      const slow = /answer slowly, then run the command: ([^\n]+)/u.exec(asked);
      const command = slow ?? /run the command: ([^\n]+)/u.exec(asked);
      const write = /write the file: ([^\n]+)/u.exec(asked);
      // Mastra Code's own names for its tools, as it offers them (SDK 1.19.1).
      const commandTool = find(/^execute_command$/u);
      const writeTool = find(/^write_file$/u);
      if ((slow && commandTool) || asked === "hold the answer open") {
        heldResponse = response;
        response.writeHead(200, { "content-type": "text/event-stream" });
        response.write(chunk({ role: "assistant", content: "" }));
        response.write(chunk({ content: "The start of a slow answer. " }));
        // Only the client can end this response before teardown.
        return;
      }
      const [tool, args] = command && commandTool ? [commandTool, { command: command[1].trim() }] : write && writeTool ? [writeTool, { path: write[1].trim(), content: MARK }] : /measure the part/u.test(asked) && find(/measure_part/u) ? [find(/measure_part/u), { unit: "mm" }] : [];
      if (tool) {
        response.writeHead(200, { "content-type": "text/event-stream" });
        call(tool, args);
        return;
      }
      say(command || write || /measure the part/u.test(asked) ? "No such tool was offered." : `Hello from the stand-in model (request ${seen.requests}).`);
    });
  });
  const waitForHeldClose = async (ms) => {
    const response = heldResponse;
    if (!response) throw new Error("the stand-in did not hold a response open");
    // The server may already have seen 'close' before the ACP reply arrived.
    if (response.closed) return;
    try {
      await once(response, "close", { signal: AbortSignal.timeout(ms) });
    } catch (error) {
      if (error.name !== "AbortError") throw error;
      throw new Error(`the stand-in's held response did not close in ${ms} ms`);
    }
  };
  const close = () => {
    server.close();
    server.closeAllConnections();
  };
  return new Promise((ready) => server.listen(0, "127.0.0.1", () => ready({ port: server.address().port, seen, waitForHeldClose, close })));
}

// ---- Talking ACP with an agent ------------------------------------------------------------------

/** The environment a program needs to run on this system and nothing else of the caller's, with a data folder and a home in it. */
export function ownEnvironment(data, more = {}) {
  const env = { PATH: process.env.PATH ?? "", MASTRA_APP_DATA_DIR: data, HOME: join(data, "home"), USERPROFILE: join(data, "home") };
  for (const name of ["SystemRoot", "SystemDrive", "windir", "ComSpec", "PATHEXT", "TEMP", "TMP", "TMPDIR", "LANG", "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE"]) {
    if (process.env[name]) env[name] = process.env[name];
  }
  return { ...env, ...more };
}

/**
 * Starts an ACP agent (`command` with `args`) and speaks for the app. `permission(request)` answers
 * a permission request: its return value is the result, and `undefined` leaves the request open
 * (in `agent.open`) for the case to answer with `agent.answer`. Without it every request is allowed
 * once. `keep(file, value)` is told every message in both directions.
 */
export function start({ command, args, env, cwd, permission, keep = () => {} }) {
  const child = spawn(command, args, { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  let pending = "";
  let stderr = "";
  let nextId = 0;
  const waiting = new Map();
  const notes = [];
  const open = [];
  const watchers = new Set();
  const write = (message) => {
    keep("acp.jsonl", { to: "agent", message });
    child.stdin.write(`${JSON.stringify(message)}\n`);
  };
  const noted = (message) => {
    notes.push(message);
    for (const watcher of [...watchers]) watcher(message);
  };
  child.stderr.on("data", (data) => (stderr += data));
  child.on("error", (error) => (stderr += `could not start: ${error.message}\n`));
  child.stdin.on("error", () => {});
  child.stdout.on("data", (data) => {
    pending += data;
    for (let end; (end = pending.indexOf("\n")) >= 0; ) {
      const line = pending.slice(0, end);
      pending = pending.slice(end + 1);
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        continue;
      }
      keep("acp.jsonl", { to: "app", message });
      if (message.id != null && (message.result !== undefined || message.error !== undefined) && waiting.has(message.id)) {
        waiting.get(message.id)(message);
        waiting.delete(message.id);
      } else if (message.method && message.id != null) {
        // The agent asks the app something. A permission follows the case's rule; anything else gets an empty answer.
        if (message.method === "session/request_permission") {
          const allow = message.params?.options?.find((item) => /allow/u.test(item.kind)) ?? message.params?.options?.[0];
          const result = permission ? permission(message) : { outcome: { outcome: "selected", optionId: allow?.optionId } };
          if (result === undefined) open.push(message);
          else write({ jsonrpc: "2.0", id: message.id, result });
        } else write({ jsonrpc: "2.0", id: message.id, result: {} });
        noted(message);
      } else if (message.method) noted(message);
    }
  });
  const call = (method, params = {}, ms = 120000) =>
    new Promise((done, failed) => {
      const id = ++nextId;
      const timer = setTimeout(() => failed(new Error(`${method}: no answer in ${ms} ms`)), ms);
      waiting.set(id, (message) => {
        clearTimeout(timer);
        if (message.error) failed(Object.assign(new Error(message.error.message), { code: message.error.code, data: message.error.data }));
        else done(message.result);
      });
      write({ jsonrpc: "2.0", id, method, params });
    });
  /** The first message from the agent, already there or still to come, that `wanted` accepts. */
  const waitFor = (wanted, ms = 30000, from = 0, signal) =>
    new Promise((done, failed) => {
      const found = notes.slice(from).find(wanted);
      if (found) return done(found);
      signal?.throwIfAborted();
      const clear = () => {
        clearTimeout(timer);
        watchers.delete(watcher);
        signal?.removeEventListener("abort", abort);
      };
      const abort = () => {
        clear();
        failed(signal.reason);
      };
      const watcher = (message) => {
        if (!wanted(message)) return;
        clear();
        done(message);
      };
      const timer = setTimeout(() => {
        clear();
        failed(new Error(`the agent did not send what was waited for in ${ms} ms`));
      }, ms);
      watchers.add(watcher);
      signal?.addEventListener("abort", abort);
    });
  const stop = () =>
    new Promise((done) => {
      if (child.exitCode !== null || child.signalCode !== null) return done();
      child.once("exit", done);
      child.kill();
      setTimeout(() => child.kill("SIGKILL"), 4000).unref();
    });
  return { call, notify: (method, params) => write({ jsonrpc: "2.0", method, params }), answer: (id, result) => write({ jsonrpc: "2.0", id, result }), waitFor, notes, open, stop, stderr: () => stderr, pid: child.pid };
}

/** What a turn left in the notes: the answer's text, the tools called, the permissions asked. */
export function turnOf(notes, from) {
  let answer = "";
  const tools = [];
  let asked = 0;
  for (const note of notes.slice(from)) {
    const update = note.params?.update;
    if (update?.sessionUpdate === "agent_message_chunk") answer += update.content?.text ?? "";
    if (update?.sessionUpdate === "tool_call") tools.push(update.title ?? update.kind ?? "tool");
    if (note.method === "session/request_permission") asked += 1;
  }
  return { answer: answer.slice(0, 120), tools, asked };
}

/** Every entry under a folder with its size and the time it was last changed, as one text to compare. */
export function listing(folder) {
  const lines = [];
  const walk = (dir, prefix) => {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        lines.push(`${prefix}${entry.name}/`);
        walk(path, `${prefix}${entry.name}/`);
      } else {
        const stat = statSync(path);
        lines.push(`${prefix}${entry.name} ${stat.size} ${stat.mtimeMs}`);
      }
    }
  };
  if (existsSync(folder)) walk(folder, "");
  return lines.join("\n");
}

/** Where under `folder` a file of this name is, or null. */
export function findFile(folder, name) {
  if (!existsSync(folder)) return null;
  for (const entry of readdirSync(folder, { withFileTypes: true })) {
    const path = join(folder, entry.name);
    if (entry.isDirectory()) {
      const inside = findFile(path, name);
      if (inside) return inside;
    } else if (entry.name === name) return path;
  }
  return null;
}

const refuse = (message) => {
  const option = message.params?.options?.find((item) => /reject/u.test(item.kind));
  return option ? { outcome: { outcome: "selected", optionId: option.optionId } } : { outcome: { outcome: "cancelled" } };
};
const isPermission = (message) => message.method === "session/request_permission";

// ---- The cases ----------------------------------------------------------------------------------

async function main(args) {
  const flag = (name) => {
    const index = args.indexOf(name);
    return index < 0 ? undefined : args.splice(index, 2)[1];
  };
  let node = flag("--node");
  let entry = flag("--entry");
  const sdk = flag("--sdk");
  const only = flag("--only");
  const earlyRejection = only === "early-rejection control";
  const raw = flag("--raw");
  if (args.length === 1 && !node && !entry) {
    const folder = resolve(args[0]);
    node = join(folder, process.platform === "win32" ? "node.exe" : "node");
    entry = join(folder, "agent", "main.mjs");
  } else if (args.length !== 0 || !node || !entry) {
    console.error("usage: agent-smoke.mjs <mastra-agent folder> [--only <text>] [--raw <folder>]\n       agent-smoke.mjs --node <node> --entry <main.mjs> [--sdk <node_modules>] [--only <text>] [--raw <folder>]");
    return 2;
  }
  for (const file of [node, entry]) {
    if (!existsSync(file)) {
      console.error(`error: ${file} is not there`);
      return 2;
    }
  }
  const work = mkdtempSync(join(tmpdir(), "intcad-agent-smoke-"));
  const failures = [];
  let ran = 0;

  /**
   * One agent process, one model service and one project folder for one case. `fn` gets
   * `prompt(text)`, which sends only to the stand-in, and what it needs to look at the folders.
   */
  async function run(name, { mcpServers = [], permission } = {}, fn) {
    if (only && !name.includes(only)) return;
    ran += 1;
    const slug = name.replace(/[^a-z0-9]+/giu, "-");
    const keep = raw
      ? (file, value) => {
          mkdirSync(join(raw, slug), { recursive: true });
          appendFileSync(join(raw, slug, file), `${JSON.stringify(value)}\n`);
        }
      : undefined;
    const [data, project] = [join(work, slug, "data"), join(work, slug, "project")];
    mkdirSync(project, { recursive: true });
    writeFileSync(join(project, "part.txt"), "a file of the project\n");
    const service = await modelService({ keep });
    const agent = start({ command: node, args: [entry], env: ownEnvironment(data, sdk ? { SOLIDCRAFT_MASTRA_SDK: sdk } : {}), cwd: project, permission, keep });
    const out = { case: name };
    try {
      await agent.call("initialize", { protocolVersion: 1, clientCapabilities: {} });
      const snapshot = await agent.call("_intcad/connections/setup", { keys: {}, custom: [{ name: "Stand-in", url: `http://127.0.0.1:${service.port}/v1`, apiKey: "stand-in", models: ["hello-1"] }] });
      const session = await agent.call("session/new", { cwd: project, mcpServers });
      // The run chooses the stand-in's model itself, so it does not depend on which model a new session starts on.
      const model = () => session.configOptions?.find((option) => option.id === "model");
      const standIn = model()?.options?.map((option) => option.value).find((value) => value.startsWith("mastracode/stand-in/"));
      if (standIn && model().currentValue !== standIn) session.configOptions = (await agent.call("session/set_config_option", { sessionId: session.sessionId, configId: "model", value: standIn })).configOptions;
      const inUse = model()?.currentValue ?? "";
      // Nothing is sent unless the stand-in is the only thing the agent could talk to.
      const alone = snapshot.keys.every((key) => !key.saved) && snapshot.plans.every((plan) => plan.state === "out") && inUse.startsWith("mastracode/stand-in/");
      if (!alone) throw new Error(`not alone with the stand-in (keys saved: ${snapshot.keys.filter((key) => key.saved).length}, plans signed in: ${snapshot.plans.filter((plan) => plan.state !== "out").length}, model in use: ${inUse || "none"}); nothing was sent`);
      const send = (text) => agent.call("session/prompt", { sessionId: earlyRejection ? `${session.sessionId}-missing` : session.sessionId, prompt: [{ type: "text", text }] });
      const sendUntil = async (text, wanted, from) => {
        const turn = send(text);
        const waitingForEvent = new AbortController();
        try {
          const event = await Promise.race([
            agent.waitFor(wanted, 60000, from, waitingForEvent.signal),
            turn.then(() => { throw new Error("the prompt ended before the expected ACP event"); }),
          ]);
          return { turn, event };
        } finally {
          waitingForEvent.abort();
        }
      };
      const prompt = async (text) => {
        const [from, before] = [agent.notes.length, service.seen.requests];
        const reply = await send(text);
        return { stop: reply.stopReason, askedStandIn: service.seen.requests > before, ...turnOf(agent.notes, from) };
      };
      const before = listing(project);
      out.ok = await fn({ out, prompt, sendUntil, agent, service, sessionId: session.sessionId, project, unchanged: () => listing(project) === before, marker: (file) => findFile(join(work, slug), file) });
    } catch (error) {
      out.ok = false;
      out.failed = String(error.message).slice(0, 300);
      out.stderr = agent
        .stderr()
        .split("\n")
        .filter((line) => /Error|Cannot find|does not provide|ERR_|not found|could not start/iu.test(line))
        .slice(0, 5)
        .join(" | ")
        .slice(0, 800);
    }
    await agent.stop();
    service.close();
    // A session without a vector store leaves no vector database in the agent's data folder.
    const vectors = existsSync(data) ? readdirSync(data).filter((file) => file.includes("vectors")) : [];
    if (vectors.length > 0) {
      out.ok = false;
      out.vectorStore = vectors;
    }
    if (!out.ok) failures.push(name);
    console.log(JSON.stringify(out));
  }

  /** After a terminal reply, check same-session recovery before looking for side effects. */
  const nothingLeft = async (context, file) => {
    const { out } = context;
    out.next = await context.prompt("hi");
    out.markerFile = context.marker(file);
    out.projectUnchanged = context.unchanged();
    return out.markerFile === null && out.projectUnchanged && out.next.stop === "end_turn" && out.next.askedStandIn && /stand-in model/u.test(out.next.answer);
  };

  await run("three turns", {}, async ({ out, prompt }) => {
    const turns = (out.turns = []);
    for (const text of ["hi", "and again", "a third time"]) turns.push(await prompt(text));
    return turns.every((turn) => turn.stop === "end_turn" && turn.askedStandIn && /stand-in model/u.test(turn.answer));
  });

  await run("a command after a permission request", {}, async ({ out, prompt, marker }) => {
    const turn = Object.assign(out, await prompt(`run the command: echo ${MARK} > allowed-command.txt`));
    out.markerFile = marker("allowed-command.txt") !== null;
    return turn.stop === "end_turn" && turn.askedStandIn && turn.tools.length === 1 && turn.asked === 1 && marker("allowed-command.txt") !== null;
  });

  const calls = join(work, "mcp-calls.log");
  await run("a tool of an MCP server", { mcpServers: [{ name: "intcad-app", command: process.execPath, args: [self, "--mcp-server", calls], env: [] }] }, async ({ out, prompt }) => {
    const turn = Object.assign(out, await prompt("measure the part"));
    out.serverSawCalls = existsSync(calls) ? readFileSync(calls, "utf8").trim().split("\n").length : 0;
    return turn.stop === "end_turn" && turn.askedStandIn && out.serverSawCalls === 1 && turn.answer.includes(MARK);
  });

  await run("a refused command does not run", { permission: refuse }, async (context) => {
    const turn = (context.out.turn = await context.prompt(`run the command: echo ${MARK} > refused-command.txt`));
    return turn.stop === "end_turn" && turn.asked === 1 && (await nothingLeft(context, "refused-command.txt"));
  });

  let refuseWrites = true;
  await run("a refused file write writes nothing, an allowed one writes", { permission: (message) => (refuseWrites ? refuse(message) : { outcome: { outcome: "selected", optionId: message.params.options.find((item) => /allow/u.test(item.kind))?.optionId } }) }, async (context) => {
    const refused = (context.out.refused = await context.prompt(`write the file: ${join(context.project, "refused-write.txt")}`));
    if (!(refused.stop === "end_turn" && refused.asked === 1 && (await nothingLeft(context, "refused-write.txt")))) return false;
    refuseWrites = false;
    const allowed = (context.out.allowed = await context.prompt(`write the file: ${join(context.project, "allowed-write.txt")}`));
    const written = context.marker("allowed-write.txt");
    context.out.allowedFileWritten = written !== null;
    return allowed.stop === "end_turn" && allowed.asked === 1 && written !== null && readFileSync(written, "utf8").includes(MARK);
  });

  const control = (name) => earlyRejection ? `${name} (${only})` : name;
  await run(control("a turn cancelled while a permission request is open"), { permission: () => undefined }, async (context) => {
    const from = context.agent.notes.length;
    const { turn, event: request } = await context.sendUntil(`run the command: echo ${MARK} > cancelled-command.txt`, isPermission, from);
    context.agent.notify("session/cancel", { sessionId: context.sessionId });
    // ACP: after a cancel the app answers every open permission request as cancelled.
    context.agent.answer(request.id, { outcome: { outcome: "cancelled" } });
    const reply = await turn;
    context.out.stop = reply.stopReason;
    return reply.stopReason === "cancelled" && (await nothingLeft(context, "cancelled-command.txt"));
  });

  const noClose = only === "no-close control";
  await run(noClose ? only : control("a turn cancelled while the answer streams"), {}, async (context) => {
    const from = context.agent.notes.length;
    const { turn } = await context.sendUntil(
      noClose ? "hold the answer open" : `answer slowly, then run the command: echo ${MARK} > cancelled-stream.txt`,
      (message) => message.params?.update?.sessionUpdate === "agent_message_chunk",
      from,
    );
    if (noClose) {
      context.out.streamStarted = true;
      await Promise.race([
        context.service.waitForHeldClose(1500),
        turn.then(() => { throw new Error("the open control prompt ended unexpectedly"); }),
      ]);
      return true;
    }
    context.agent.notify("session/cancel", { sessionId: context.sessionId });
    const reply = await turn;
    context.out.stop = reply.stopReason;
    if (reply.stopReason !== "cancelled") return false;
    await context.service.waitForHeldClose(10000);
    context.out.modelRequestEndedByAgent = true;
    const left = await nothingLeft(context, "cancelled-stream.txt");
    context.out.permissionsAsked = context.agent.notes.slice(from).filter(isPermission).length;
    return context.out.permissionsAsked === 0 && left;
  });

  try {
    rmSync(work, { recursive: true, force: true, maxRetries: 5 });
  } catch {
    // A file the system still holds (a database on Windows) stays in the temporary folder; the run's result stands.
  }
  if (ran === 0) failures.push(`no case has "${only}" in its name`);
  console.log(JSON.stringify({ summary: failures.length === 0 ? "all passed" : `failed: ${failures.join(", ")}`, cases: ran, system: `${process.platform} ${process.arch}`, node }));
  return failures.length === 0 ? 0 : 1;
}

// Only when this file is the program that was started: compare-official-acp.mjs imports it.
if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(self)) {
  if (process.argv[2] === "--mcp-server") mcpServer(process.argv[3]);
  else process.exit(await main(process.argv.slice(2)));
}
