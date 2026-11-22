// Plan first (`agent/plan.mjs`), INT-1473 B2: in a chat on a project, the first change of a reply
// waits until the user has been shown a numbered plan, and the model is told why. No Mastra, no
// model, no network: the agent runs on a stand-in for Mastra Code's ACP agent.

import assert from "node:assert/strict";
import { test } from "node:test";

type Json = Record<string, any>;
// Compiled, this file is in `dist/test/`; the modules stay where they are written.
const compiled = import.meta.url.includes("/dist/test/");
const agentFolder = compiled ? "../../agent/" : "../agent/";
const load = (name: string): Promise<Json> => import(new URL(agentFolder + name, import.meta.url).href);
const { PLAN_FIRST, PlanGate, hasPlan, isChange, planEntries } = await load("plan.mjs");
const { askingOnlyWhenNeeded } = await load("reads.mjs");
const { intcadAgent } = await load("agent.mjs");

const ASK = [
  { optionId: "approve", name: "Allow", kind: "allow_once" },
  { optionId: "reject", name: "Reject", kind: "reject_once" },
];
let calls = 0;
/** A permission request as Mastra Code sends it (`acp/event-mapper.js` of SDK 1.19.1). */
const request = (title: string, args: unknown = {}, sessionId = "s1"): Json => ({ sessionId, toolCall: { toolCallId: `c${++calls}`, title, rawInput: JSON.stringify(args) }, options: ASK });
const fillet = (sessionId = "s1") => request("intcad-app_execute", { command: "solid.fillet", params: { radius: 3 } }, sessionId);

test("hasPlan: a numbered line in what the user was shown, in English or Chinese; not a number that starts a line", () => {
  for (const text of ["1. Set the thickness to 2.5 mm; check with measure.", "Plan:\n1) thickness\n2) hole", "1、把板厚改成 2.5 mm", "**1.** Move the hole", "- 1. Move the hole", "第1步：改厚度", "第一步：改厚度", "一、改厚度", "  2. Then the hole"]) {
    assert.equal(hasPlan(text), true, JSON.stringify(text));
  }
  for (const text of ["", "I'll read the project files first.", "2.5 mm is the lightest allowed.", "Version 1.19.1 of the SDK", "see step 1 below", "Thickness 1.5 → 2.5", undefined, null, 42]) {
    assert.equal(hasPlan(text), false, JSON.stringify(text));
  }
});

test("hasPlan: a plan written as a table or with its steps in one line counts too (the end-to-end run's model wrote both)", () => {
  for (const text of ["| Step | Change | Check |\n|---|---|---|\n| 1 | Thickness → 2.5 mm | measure |", "| **1.** | Move the hole | body_topology |", "方案同上:1. thickness→2.5mm,2. 线缆孔移到 (110, 40)", "First 1) set the thickness; then 2) move the cable hole."]) {
    assert.equal(hasPlan(text), true, JSON.stringify(text));
  }
  for (const text of ["| Requirement | Source |\n|---|---|\n| Outline 120 × 80 | requirements.md |", "| 120 | 80 |", "| 2.5 mm | ok |", "Thickness 1.5 → 2.5 mm, hole at (114, 40).", "see section 1. Then we look at 2.5 mm."]) {
    assert.equal(hasPlan(text), false, JSON.stringify(text));
  }
});

test("PlanGate: what the agent says after a tool call starts a line of its own, so a numbered line there is a plan", () => {
  // Joined without a break the numbered line is no line of its own, and the plan was missed.
  assert.equal(hasPlan("Reading the files." + "1. Thickness to 2.5 mm."), false);
  const gate = new PlanGate(() => true);
  gate.prompted("s1");
  gate.said("s1", "Reading the files.");
  gate.called("s1");
  gate.called("s1");
  gate.said("s1", "1. Thickness to 2.5 mm; check with measure.");
  assert.equal(gate.holds(fillet()), false);
  // A break before anything was said adds nothing.
  const quiet = new PlanGate(() => true);
  quiet.prompted("s1");
  quiet.called("s1");
  assert.equal(quiet.holds(fillet()), true);
});

test("isChange: the app's tools that change the design or write a file, and Mastra's file writes; not reads, looks, the shell or the web", () => {
  for (const r of [fillet(), request("intcad-app_batch", { commands: [] }), request("intcad-app_set_parameter", { name: "t", value: "2 mm" }), request("intcad-app_save"), request("intcad-app_export"), request("intcad-app_undo"), request("intcad-app_open"), request("intcad-app_new_design"), request("write_file"), request("string_replace_lsp"), request("delete_file")]) {
    assert.equal(isChange(r), true, r.toolCall.title);
  }
  for (const r of [request("intcad-app_execute", { command: "inspect.measure" }), request("intcad-app_inspect_design"), request("intcad-app_screenshot"), request("intcad-app_compare_picture"), request("intcad-app_measure"), request("execute_command", { command: "pdftotext a.pdf -" }), request("web_search"), request("skill"), request("other-server_execute", { command: "solid.fillet" }), {}, null]) {
    assert.equal(isChange(r), false, r?.toolCall?.title ?? JSON.stringify(r));
  }
});

test("PlanGate: in a chat on a project, a change before any plan is held once, with the reason; a plan lets it through", () => {
  const gate = new PlanGate((id: string) => id === "s1");
  gate.prompted("s1");
  const first = fillet();
  assert.equal(gate.holds(first), true);
  assert.equal(gate.reasonFor(first.toolCall.toolCallId), PLAN_FIRST);
  assert.equal(gate.reasonFor(first.toolCall.toolCallId), undefined, "the reason is given once");
  assert.equal(gate.reasonFor("never-held"), undefined);
  assert.equal(gate.holds(request("intcad-app_execute", { command: "inspect.measure" })), false, "a read is never held");
  // The plan arrives in pieces, as the text streams.
  gate.said("s1", "Plan:\n");
  gate.said("s1", "1");
  gate.said("s1", ". Fillet the edge; check with measure.");
  assert.equal(gate.holds(fillet()), false);
  assert.equal(gate.holds(fillet()), false, "later changes of the same reply");
  // A chat not on a project is never held.
  assert.equal(gate.holds(fillet("s2")), false);
  // The model is told what counts, and to call again in the same reply rather than wait.
  assert.match(PLAN_FIRST, /not refused/u);
  assert.match(PLAN_FIRST, /Record the plan now with task_write/u);
  assert.match(PLAN_FIRST, /Intcad shows that list to the user/u);
  assert.match(PLAN_FIRST, /your thinking is not shown/u);
  assert.match(PLAN_FIRST, /make this same call again right away, in the same reply/u);
  assert.match(PLAN_FIRST, /approves the change on its card/u);
  assert.match(PLAN_FIRST, /in the user's language/u);
  assert.match(PLAN_FIRST, /done only once its check has run and passed/u);
});

test("planEntries: a task list as an ACP plan, a step per task with text, status kept or pending", () => {
  assert.deepEqual(planEntries([{ id: "t1", content: " Thickness to 2.5 mm; check with measure ", status: "in_progress" }, { id: "t2", content: "Move the cable hole", status: "odd" }, { id: "t3", content: "" }, { id: "t4" }, null]), [
    { content: "Thickness to 2.5 mm; check with measure", priority: "medium", status: "in_progress" },
    { content: "Move the cable hole", priority: "medium", status: "pending" },
  ]);
  assert.deepEqual(planEntries(undefined), []);
  assert.equal(planEntries(Array.from({ length: 80 }, (_, i) => ({ content: `step ${i}` }))).length, 64);
});

test("PlanGate: a task list is a plan; an empty one is not; it carries over like a written plan", () => {
  const gate = new PlanGate(() => true);
  gate.prompted("s1");
  gate.listed("s1", []);
  assert.equal(gate.holds(fillet()), true, "an empty list is no plan");
  gate.listed("s1", [{ id: "t1", content: "Fillet the edge; check with measure", status: "pending" }]);
  assert.equal(gate.holds(fillet()), false);
  // A list written, then a question, then the user's OK: no second plan asked for.
  gate.prompted("s1");
  gate.listed("s1", [{ id: "t1", content: "Thickness to 2.5 mm" }, { id: "t2", content: "Move the hole" }]);
  gate.said("s1", "Shall I go ahead?");
  gate.prompted("s1");
  assert.equal(gate.holds(fillet()), false);
  gate.prompted("s1");
  assert.equal(gate.holds(fillet()), true, "a list from two messages ago, with changes since, is not this reply's plan");
});

test("PlanGate: a plan the last reply ended on, with no change after it, carries over to the user's answer; one with a change does not", () => {
  const gate = new PlanGate(() => true);
  // The plan, then a question, then the user's "go ahead".
  gate.prompted("s1");
  gate.said("s1", "1. Thickness to 2.5 mm.\n2. Move the cable hole.\nShall I go ahead?");
  gate.prompted("s1");
  assert.equal(gate.holds(fillet()), false, "the plan was shown and nothing changed after it");
  // That reply made a change: the next message needs a plan of its own.
  gate.prompted("s1");
  assert.equal(gate.holds(fillet()), true);
  // A reply that planned and changed at once: no carry either.
  gate.said("s1", "1. Fillet.");
  assert.equal(gate.holds(fillet()), false);
  gate.prompted("s1");
  assert.equal(gate.holds(fillet()), true);
});

test("askingOnlyWhenNeeded with a gate: a held change gets the request's own reject without reaching the app; the agent's words reach the gate and the app", async () => {
  const sent: Json[] = [];
  const app = { requestPermission: async (asked: Json) => (sent.push({ asked }), { outcome: { outcome: "selected", optionId: "approve" } }), sessionUpdate: async (update: Json) => void sent.push({ update }) };
  const gate = new PlanGate(() => true);
  const connection = askingOnlyWhenNeeded(app, gate);
  gate.prompted("s1");
  assert.deepEqual(await connection.requestPermission(fillet()), { outcome: { outcome: "selected", optionId: "reject" } });
  assert.deepEqual(sent, [], "the app was not asked");
  // Text, a tool call, then the plan: the tool call puts the plan on a line of its own.
  const before = { sessionId: "s1", update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "Reading the files." } } };
  const tool = { sessionId: "s1", update: { sessionUpdate: "tool_call", toolCallId: "t1", title: "view" } };
  const words = { sessionId: "s1", update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "1. Fillet the edge." } } };
  for (const update of [before, tool, words]) await connection.sessionUpdate(update);
  const allowed = fillet();
  assert.deepEqual(await connection.requestPermission(allowed), { outcome: { outcome: "selected", optionId: "approve" } });
  assert.deepEqual(sent, [{ update: before }, { update: tool }, { update: words }, { asked: allowed }]);
});

/** Mastra Code's ACP agent as far as the gate needs it: sessions, a session's approvals, a turn. */
function stand(project: string | null) {
  const declined: Json[] = [];
  const listeners: ((event: Json) => void)[] = [];
  const entry: Json = {
    project,
    state: null,
    controller: { listAvailableModels: async () => [] },
    session: {
      state: { set: async () => {} },
      model: { get: () => "anthropic/claude-fable-5" },
      respondToToolApproval(answer: Json) {
        declined.push(answer);
        return { accepted: true };
      },
      subscribe(listener: (event: Json) => void) {
        listeners.push(listener);
        return () => listeners.splice(listeners.indexOf(listener), 1);
      },
    },
    emit: (event: Json) => listeners.forEach((listener) => listener(event)),
  };
  class Base {
    connection: Json;
    sessions = new Map<string, Json>();
    constructor(connection: Json) {
      this.connection = connection;
    }
    configOptions() {
      return [];
    }
    async newSession() {
      this.sessions.set("s1", entry);
      return { sessionId: "s1" };
    }
    async prompt() {
      return { stopReason: "end_turn" };
    }
  }
  const asked: Json[] = [];
  const updates: Json[] = [];
  const app = { requestPermission: async (r: Json) => (asked.push(r), { outcome: { outcome: "selected", optionId: "approve" } }), sessionUpdate: async (u: Json) => void updates.push(u) };
  const Agent = intcadAgent({ Base, RequestError: Error, connections: {}, dates: () => new Map(), reloadSignIns: () => {} });
  return { agent: new Agent(app), entry, declined, asked, updates };
}

test("the agent: in a chat on a project the held change is declined with the plan reason; after a plan it reaches the user", async () => {
  const { agent, entry, declined, asked } = stand("/home/u/plate");
  await agent.newSession({});
  assert.equal(entry.plansFirst, true);
  await agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "Fix this part." }] });
  const held = fillet();
  const answer = await agent.connection.requestPermission(held);
  assert.equal(answer.outcome.optionId, "reject");
  assert.equal(asked.length, 0, "the user was not asked");
  // Mastra Code answers the approval as the event mapper does; the reason goes with the decline.
  entry.session.respondToToolApproval({ decision: "decline", toolCallId: held.toolCall.toolCallId });
  assert.deepEqual(declined.at(-1), { decision: "decline", toolCallId: held.toolCall.toolCallId, declineContext: { reason: PLAN_FIRST } });
  // A decline the gate did not make keeps Mastra's own wording.
  entry.session.respondToToolApproval({ decision: "decline", toolCallId: "the-users-own" });
  assert.deepEqual(declined.at(-1), { decision: "decline", toolCallId: "the-users-own" });
  await agent.connection.sessionUpdate({ sessionId: "s1", update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "1. Fillet the edge; check with measure." } } });
  await agent.connection.requestPermission(fillet());
  assert.equal(asked.length, 1, "after the plan the user is asked");
});

test("the agent: its task list reaches the app as the reply's plan, and is the plan the gate waits for", async () => {
  const { agent, entry, asked, updates } = stand("/home/u/plate");
  await agent.newSession({});
  await agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "Fix this part." }] });
  // Held without a plan.
  assert.equal((await agent.connection.requestPermission(fillet())).outcome.optionId, "reject");
  // Mastra Code's task tool runs and its event comes through the session.
  entry.emit({ type: "task_updated", tasks: [{ id: "t1", content: "Fillet the edge, 3 mm; check with measure", status: "pending" }] });
  assert.deepEqual(updates, [{ sessionId: "s1", update: { sessionUpdate: "plan", entries: [{ content: "Fillet the edge, 3 mm; check with measure", priority: "medium", status: "pending" }] } }]);
  await agent.connection.requestPermission(fillet());
  assert.equal(asked.length, 1, "after the list the user is asked");
  // An empty list shows nothing.
  entry.emit({ type: "task_updated", tasks: [] });
  assert.equal(updates.length, 1);
  // Other events are not the task list.
  entry.emit({ type: "message_update", id: "m1", event: { type: "text-delta", delta: "1. hi" } });
  assert.equal(updates.length, 1);
});

test("the agent: a chat without a project is never held", async () => {
  const { agent, entry, asked } = stand(null);
  await agent.newSession({});
  assert.equal(entry.plansFirst, undefined);
  await agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "Fillet it." }] });
  await agent.connection.requestPermission(fillet());
  assert.equal(asked.length, 1);
});
