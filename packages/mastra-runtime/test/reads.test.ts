// What the built-in agent runs without asking (`agent/reads.mjs`), INT-1473: the permission rules a
// session starts with, the app's commands that only read, and the connection that answers those
// itself. No Mastra, no model, no network; the app's own sources are read for what they declare.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

type Json = Record<string, any>;
// Compiled, this file is in `dist/test/`; the modules stay where they are written.
const compiled = import.meta.url.includes("/dist/test/");
const agentFolder = compiled ? "../../agent/" : "../agent/";
const repository = compiled ? "../../../../" : "../../../";
const load = (name: string): Promise<Json> => import(new URL(agentFolder + name, import.meta.url).href);
const source = (path: string): string => readFileSync(new URL(repository + path, import.meta.url), "utf8");
const { APP_READS, APP_SERVER, FENCED_READS, askingOnlyWhenNeeded, commandReadsOnly, permissionRules, readsWithoutAsking } = await load("reads.mjs");

const ASK = [
  { optionId: "approve", name: "Allow", kind: "allow_once" },
  { optionId: "reject", name: "Reject", kind: "reject_once" },
];
/** A permission request as Mastra Code sends it (`acp/event-mapper.js` of SDK 1.19.1). */
const request = (title: string, args: unknown, options = ASK): Json => ({ sessionId: "s1", toolCall: { toolCallId: "c1", title, rawInput: typeof args === "string" ? args : JSON.stringify(args) }, options });

test("permissionRules: the app's tools that only look run unasked in every session, the four file reads only with a project, and no category is allowed", () => {
  const denied = { ask_user: "deny", request_access: "deny" };
  const looks = Object.fromEntries(["list_commands", "describe_command", "inspect_design", "measure", "body_topology", "running", "pending", "skill_reference"].map((name) => [`intcad-app_${name}`, "allow"]));
  assert.deepEqual(FENCED_READS, ["view", "find_files", "search_content", "file_stat"]);
  assert.deepEqual(permissionRules(null), { categories: {}, tools: { ...denied, ...looks } });
  assert.deepEqual(permissionRules("/a/project"), { categories: {}, tools: { ...denied, ...looks, view: "allow", find_files: "allow", search_content: "allow", file_stat: "allow" } });
  for (const tool of ["intcad-app_execute", "intcad-app_batch", "intcad-app_save", "intcad-app_set_parameter", "intcad-app_crash_reports", "execute_command", "write_file", "lsp_inspect", "web_search"]) {
    assert.equal(permissionRules("/a/project").tools[tool], undefined, tool);
  }
});

test("the app's server is the one the app names, and the tools taken as only looking are those its server marks read-only, less crash_reports", () => {
  assert.equal(/pub const BRIDGE_NAME: &str = "([^"]+)";/u.exec(source("crates/ui-egui/src/agent_chat/acp.rs"))?.[1], APP_SERVER);
  const effects = new Map([...source("crates/mcp/src/tools.rs").matchAll(/\btool\(\s*"([a-z_]+)",[\s\S]*?Effect::(\w+)/gu)].map((match) => [match[1], match[2]]));
  assert.ok(effects.size >= 20 && effects.get("execute") === "Files", `the tool list was read: ${[...effects].join(", ")}`);
  for (const name of APP_READS) assert.equal(effects.get(name), "ReadOnly", name);
  // A new read-only tool of the server is a decision: add it to APP_READS, or here.
  const readOnly = [...effects].filter(([, effect]) => effect === "ReadOnly").map(([name]) => name);
  assert.deepEqual(readOnly.filter((name) => !APP_READS.includes(name)), ["crash_reports"]);
});

test("commandReadsOnly: inspection, lists and measurements, as the engine resolves an id; nothing that changes the design or writes a file", () => {
  // The read-only commands of the engine's own test (crates/engine/src/control_pending.rs), and others of the same families.
  for (const id of ["sketch.inspect", "view.status_items", "inspect.measure", "model.edges", "document.inspect", "document.revision", "parameters.list", "render.settings.get", "file.slicers", "view.pick", " Inspect.Measure ", "MODEL.EDGES", "\tsketch.inspect\n"]) {
    assert.equal(commandReadsOnly(id), true, JSON.stringify(id));
  }
  for (const id of ["solid.fillet", "sketch.create", "solid.extrude", "file.save", "file.save_as", "file.export", "parameters.export", "doc.open", "edit.checkpoint", "diagnostics.crash_reports", "inspect", "xinspect.measure", "solid.inspector", "model", "", "   "]) {
    assert.equal(commandReadsOnly(id), false, JSON.stringify(id));
  }
  for (const value of [undefined, null, 42, {}, ["inspect.measure"]]) assert.equal(commandReadsOnly(value), false, JSON.stringify(value));
  // Only ASCII letters are folded, as the engine folds them: the Kelvin sign, which lowercases to
  // "k", names no command there, and is no "k" here.
  assert.equal(commandReadsOnly("RENDER.BACKENDS"), true);
  assert.equal(commandReadsOnly("render.bacKends"), false);
});

test("readsWithoutAsking: only the app's execute, with a command that only reads", () => {
  assert.equal(readsWithoutAsking(request("intcad-app_execute", { command: "inspect.measure" })), true);
  assert.equal(readsWithoutAsking(request("intcad-app_execute", { command: " Model.Edges ", params: { body: "Body1" } })), true);
  const asObject = request("intcad-app_execute", {});
  asObject.toolCall.rawInput = { command: "sketch.inspect" };
  assert.equal(readsWithoutAsking(asObject), true, "arguments given as an object");
  for (const [title, args] of [
    ["intcad-app_execute", { command: "solid.fillet", params: { radius: 3 } }],
    ["intcad-app_execute", { params: { command: "inspect.measure" } }],
    ["intcad-app_execute", "{not json"],
    ["intcad-app_execute", "null"],
    ["intcad-app_batch", { commands: [{ command: "inspect.measure" }] }],
    ["other-server_execute", { command: "inspect.measure" }],
    ["execute_command", { command: "inspect.measure" }],
    ["mcp__intcad-app__execute", { command: "inspect.measure" }],
  ] as const) {
    assert.equal(readsWithoutAsking(request(title, args)), false, `${title} ${JSON.stringify(args)}`);
  }
  for (const odd of [undefined, null, {}, { toolCall: null }, { toolCall: { title: "intcad-app_execute" } }]) assert.equal(readsWithoutAsking(odd), false, JSON.stringify(odd));
});

test("askingOnlyWhenNeeded: a read command is allowed once without reaching the app; everything else reaches it unchanged", async () => {
  class Connection {
    #sent: Json[] = [];
    get sent(): Json[] {
      return this.#sent;
    }
    async requestPermission(asked: Json): Promise<Json> {
      this.#sent.push({ asked });
      return { outcome: { outcome: "selected", optionId: "reject" } };
    }
    async sessionUpdate(update: Json): Promise<void> {
      this.#sent.push({ update });
    }
  }
  const app = new Connection();
  const connection = askingOnlyWhenNeeded(app);

  assert.deepEqual(await connection.requestPermission(request("intcad-app_execute", { command: "inspect.measure" })), { outcome: { outcome: "selected", optionId: "approve" } });
  assert.deepEqual(app.sent, [], "the app was not asked");

  const change = request("intcad-app_execute", { command: "solid.fillet" });
  assert.deepEqual(await connection.requestPermission(change), { outcome: { outcome: "selected", optionId: "reject" } });
  // A read command whose request offers no "allow once" is the app's to answer.
  const noOnce = request("intcad-app_execute", { command: "inspect.measure" }, [{ optionId: "reject", name: "Reject", kind: "reject_once" }]);
  await connection.requestPermission(noOnce);
  // The other methods still reach the app, called on it (its private fields work).
  await connection.sessionUpdate({ sessionUpdate: "agent_message_chunk" });
  assert.deepEqual(app.sent, [{ asked: change }, { asked: noOnce }, { update: { sessionUpdate: "agent_message_chunk" } }]);
  assert.equal(askingOnlyWhenNeeded(undefined), undefined, "no connection (tests that make the agent without one)");
});

test("the agent speaks to the app through that connection", async () => {
  const { intcadAgent } = await load("agent.mjs");
  class Base {
    connection: Json;
    constructor(connection: Json) {
      this.connection = connection;
    }
  }
  const asked: Json[] = [];
  const Agent = intcadAgent({ Base, RequestError: Error, connections: {}, dates: () => new Map(), reloadSignIns: () => {} });
  const agent = new Agent({ requestPermission: async (r: Json) => (asked.push(r), { outcome: { outcome: "cancelled" } }) });
  assert.deepEqual(await agent.connection.requestPermission(request("intcad-app_execute", { command: "model.edges" })), { outcome: { outcome: "selected", optionId: "approve" } });
  assert.deepEqual(await agent.connection.requestPermission(request("intcad-app_execute", { command: "solid.box" })), { outcome: { outcome: "cancelled" } });
  assert.equal(asked.length, 1);
});
