#!/usr/bin/env node
/**
 * Run the built-in agent out of a `mastra-agent` folder against stand-ins, with no account.
 *
 *   node agent-smoke.mjs <mastra-agent folder>
 *   node agent-smoke.mjs --node <node> --entry <main.mjs> [--sdk <node_modules>]
 *
 * It starts the folder's own Node with the folder's entry, as the app does, and talks ACP with
 * it: `initialize`, a custom address handed over with `_intcad/connections/setup`, then
 *   - three turns in one session, answered by a stand-in model service;
 *   - a turn in which the model asks for a command, run after one permission request;
 *   - a turn in which the model asks for a tool of an MCP server handed over in `session/new`.
 * The model service is in this process, on a port the system picks; the MCP server is this file
 * started again with `--mcp-server`.
 *
 * Nothing of the person running it is used. The agent gets a new, empty data folder
 * (`MASTRA_APP_DATA_DIR`), which is also where Mastra keeps plan sign-ins, and the agent's entry
 * drops every provider key from its environment and takes a home of its own in that folder; no
 * keychain is read by the agent. Before the first message the run still checks that no key is
 * saved, no plan is signed in and the only models offered are the stand-in's, and stops if not.
 * After each turn it checks that the stand-in was the one asked, and after each case that the
 * session made no vector store (the agent's sessions have none: agent/session.mjs). Mastra's
 * usage reporting is off: the agent's entry switches it off itself.
 *
 * Prints one JSON line per case and exits 1 when any failed. It says the agent ran on the system
 * it was run on, and nothing about any other.
 */

import { spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const self = fileURLToPath(import.meta.url);
const MARK = "intcad-smoke";

// ---- The stand-in MCP server (this file, started by the agent) ----------------------------------

if (process.argv[2] === "--mcp-server") {
  const log = process.argv[3];
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
} else {
  process.exit(await main(process.argv.slice(2)));
}

// ---- The stand-in model service -----------------------------------------------------------------

/** An OpenAI-compatible service. The first path segment picks what it does: plain, command, mcp. */
function modelService() {
  const seen = { plain: 0, command: 0, mcp: 0 };
  const server = http.createServer((request, response) => {
    let body = "";
    request.on("data", (chunk) => (body += chunk));
    request.on("end", () => {
      const kind = request.url.split("/")[1];
      if (!(kind in seen)) {
        response.writeHead(404).end();
        return;
      }
      seen[kind] += 1;
      let wanted = {};
      try {
        wanted = JSON.parse(body || "{}");
      } catch {
        // an empty request is answered like any other
      }
      const id = `chatcmpl-${kind}-${seen[kind]}`;
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
      const messages = wanted.messages ?? [];
      const last = messages[messages.length - 1] ?? {};
      const tools = (wanted.tools ?? []).map((tool) => tool.function ?? tool);
      const wantedTool = kind === "command" ? tools.find((tool) => /execute_command|shell|bash|command/iu.test(tool.name ?? "")) : kind === "mcp" ? tools.find((tool) => /measure_part/u.test(tool.name ?? "")) : null;
      if (wantedTool && last.role !== "tool" && wanted.stream) {
        const args = kind === "command" ? { command: `echo ${MARK}` } : { unit: "mm" };
        for (const name of wantedTool.parameters?.required ?? []) args[name] ??= "check";
        response.writeHead(200, { "content-type": "text/event-stream" });
        response.write(chunk({ role: "assistant", content: null, tool_calls: [{ index: 0, id: `call_${kind}_${seen[kind]}`, type: "function", function: { name: wantedTool.name, arguments: JSON.stringify(args) } }] }));
        response.write(chunk({}, "tool_calls"));
        response.end("data: [DONE]\n\n");
        return;
      }
      if (last.role === "tool") {
        const result = typeof last.content === "string" ? last.content : JSON.stringify(last.content);
        say(result.includes(MARK) ? `The tool answered with ${MARK}.` : "The tool answered without the expected text.");
        return;
      }
      say(kind === "plain" ? `Hello from the stand-in model (request ${seen.plain}).` : "No such tool was offered.");
    });
  });
  return new Promise((ready) => server.listen(0, "127.0.0.1", () => ready({ port: server.address().port, seen, close: () => server.close() })));
}

// ---- Talking ACP with the agent -----------------------------------------------------------------

function start({ node, entry, sdk, data, cwd }) {
  // Only what a program needs to run on this system, never the caller's whole environment.
  const env = { PATH: process.env.PATH ?? "", MASTRA_APP_DATA_DIR: data, HOME: join(data, "home"), USERPROFILE: join(data, "home") };
  for (const name of ["SystemRoot", "SystemDrive", "windir", "ComSpec", "PATHEXT", "TEMP", "TMP", "TMPDIR", "LANG", "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE"]) {
    if (process.env[name]) env[name] = process.env[name];
  }
  if (sdk) env.SOLIDCRAFT_MASTRA_SDK = sdk;
  const child = spawn(node, [entry], { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  let pending = "";
  let stderr = "";
  let nextId = 0;
  const waiting = new Map();
  const notes = [];
  child.stderr.on("data", (data) => (stderr += data));
  child.on("error", (error) => (stderr += `could not start: ${error.message}\n`));
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
      if (message.id != null && (message.result !== undefined || message.error !== undefined) && waiting.has(message.id)) {
        waiting.get(message.id)(message);
        waiting.delete(message.id);
      } else if (message.method && message.id != null) {
        // The agent asks the app something: a permission is allowed once, anything else gets an empty answer.
        const option = message.params?.options?.find((item) => /allow/u.test(item.kind)) ?? message.params?.options?.[0];
        const result = message.method === "session/request_permission" ? { outcome: { outcome: "selected", optionId: option?.optionId } } : {};
        child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id: message.id, result })}\n`);
        notes.push(message);
      } else if (message.method) notes.push(message);
    }
  });
  const call = (method, params = {}, ms = 120000) =>
    new Promise((done, failed) => {
      const id = ++nextId;
      const timer = setTimeout(() => failed(new Error(`${method}: no answer in ${ms} ms`)), ms);
      waiting.set(id, (message) => {
        clearTimeout(timer);
        if (message.error) failed(new Error(message.error.message));
        else done(message.result);
      });
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  const stop = () =>
    new Promise((done) => {
      if (child.exitCode !== null || child.signalCode !== null) return done();
      child.once("exit", done);
      child.kill();
      setTimeout(() => child.kill("SIGKILL"), 4000).unref();
    });
  return { call, notes, stop, stderr: () => stderr };
}

/** What a turn left in the notes: the answer's text, the tools called, the permissions asked. */
function turnOf(notes, from) {
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

// ---- The cases ----------------------------------------------------------------------------------

async function main(args) {
  const flag = (name) => {
    const index = args.indexOf(name);
    return index < 0 ? undefined : args.splice(index, 2)[1];
  };
  let node = flag("--node");
  let entry = flag("--entry");
  const sdk = flag("--sdk");
  if (args.length === 1 && !node && !entry) {
    const folder = resolve(args[0]);
    node = join(folder, process.platform === "win32" ? "node.exe" : "node");
    entry = join(folder, "agent", "main.mjs");
  } else if (args.length !== 0 || !node || !entry) {
    console.error("usage: agent-smoke.mjs <mastra-agent folder>\n       agent-smoke.mjs --node <node> --entry <main.mjs> [--sdk <node_modules>]");
    return 2;
  }
  for (const file of [node, entry]) {
    if (!existsSync(file)) {
      console.error(`error: ${file} is not there`);
      return 2;
    }
  }
  const work = mkdtempSync(join(tmpdir(), "intcad-agent-smoke-"));
  const service = await modelService();
  const failures = [];
  const address = (kind) => ({ keys: {}, custom: [{ name: "Stand-in", url: `http://127.0.0.1:${service.port}/${kind}/v1`, apiKey: "stand-in", models: ["hello-1"] }] });

  /** One agent process for one case. `fn` gets `prompt(text)`, which sends only to the stand-in. */
  async function run(name, kind, mcpServers, fn) {
    const agent = start({ node, entry, sdk, data: join(work, name), cwd: work });
    const out = { case: name };
    try {
      await agent.call("initialize", { protocolVersion: 1, clientCapabilities: {} });
      const snapshot = await agent.call("_intcad/connections/setup", address(kind));
      const session = await agent.call("session/new", { cwd: work, mcpServers });
      const models = session.configOptions?.find((option) => option.id === "model")?.options?.map((option) => option.value) ?? [];
      // Nothing is sent unless the stand-in is the only thing the agent could talk to.
      const alone = snapshot.keys.every((key) => !key.saved) && snapshot.plans.every((plan) => plan.state === "out") && models.length > 0 && models.every((model) => model.startsWith("mastracode/stand-in/"));
      if (!alone) throw new Error(`not alone with the stand-in (keys saved: ${snapshot.keys.filter((key) => key.saved).length}, plans signed in: ${snapshot.plans.filter((plan) => plan.state !== "out").length}, models: ${models.length}); nothing was sent`);
      const prompt = async (text) => {
        const [from, before] = [agent.notes.length, service.seen[kind]];
        const reply = await agent.call("session/prompt", { sessionId: session.sessionId, prompt: [{ type: "text", text }] });
        return { stop: reply.stopReason, askedStandIn: service.seen[kind] > before, ...turnOf(agent.notes, from) };
      };
      out.ok = await fn(prompt, out);
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
    // A session without a vector store leaves no vector database in the agent's data folder.
    const vectors = existsSync(join(work, name)) ? readdirSync(join(work, name)).filter((file) => file.includes("vectors")) : [];
    if (vectors.length > 0) {
      out.ok = false;
      out.vectorStore = vectors;
    }
    if (!out.ok) failures.push(name);
    console.log(JSON.stringify(out));
  }

  await run("three turns", "plain", [], async (prompt, out) => {
    out.turns = [];
    for (const text of ["hi", "and again", "a third time"]) out.turns.push(await prompt(text));
    return out.turns.every((turn) => turn.stop === "end_turn" && turn.askedStandIn && /stand-in model/u.test(turn.answer));
  });
  await run("a command after a permission request", "command", [], async (prompt, out) => {
    Object.assign(out, await prompt("run the command"));
    return out.stop === "end_turn" && out.askedStandIn && out.tools.length === 1 && out.asked === 1 && out.answer.includes(MARK);
  });
  const calls = join(work, "mcp-calls.log");
  await run("a tool of an MCP server", "mcp", [{ name: "intcad-app", command: process.execPath, args: [self, "--mcp-server", calls], env: [] }], async (prompt, out) => {
    Object.assign(out, await prompt("measure the part"));
    out.serverSawCalls = existsSync(calls) ? readFileSync(calls, "utf8").trim().split("\n").length : 0;
    return out.stop === "end_turn" && out.askedStandIn && out.serverSawCalls === 1 && out.answer.includes(MARK);
  });

  service.close();
  try {
    rmSync(work, { recursive: true, force: true, maxRetries: 5 });
  } catch {
    // A file the system still holds (a database on Windows) stays in the temporary folder; the run's result stands.
  }
  console.log(JSON.stringify({ summary: failures.length === 0 ? "all passed" : `failed: ${failures.join(", ")}`, system: `${process.platform} ${process.arch}`, node }));
  return failures.length === 0 ? 0 : 1;
}
