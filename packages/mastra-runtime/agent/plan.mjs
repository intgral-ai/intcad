// Plan first (INT-1473 B2, decided 2026-11-22): in a chat on a project, the first change of a reply
// waits until the user has been shown a plan.
//
// Asked in words, the model did not do it: in ten runs of "fix this part" with the plan asked for in
// the brief and in the skill, no reply showed one before its first change. Mastra Code's own prompt
// says "Just do it" and "Don't announce what you're about to do", and the model plans in its
// thinking, which the user never sees. Held, it planned in its thinking again, several times, before
// it wrote one in the reply (the end-to-end runs: 4 to 11 held calls a run). So the plan it is asked
// for is its task list (decided 2026-11-22): `task_write`, which Mastra Code's own prompt asks for on
// a task of several steps, which runs without a card (reads.mjs), and which the agent shows the user
// as the chat's plan (`planEntries`, agent.mjs), ticked off as the work goes.
//
// What counts:
// - A change is a call of the app's tools that changes the design or writes a file, or of Mastra's
//   file tools that write. Reads, screenshots, the shell, web tools, skills and the task tools are
//   not held: they keep the card they have, or none.
// - A plan is a task list written since the user's last message. A plan the previous reply ended on,
//   with no change after it (it was waiting for the user's word), still counts, so "plan, then the
//   user's OK, then the change" asks for no second plan. One task is a plan: a single change needs
//   no more. A numbered line in the reply does not count: a numbered answer (the requirements, the
//   files read) would let the next change through with no plan shown.
// - Only chats on a project are held, as only they are told to plan (brief.mjs).
// A change before a plan is declined by the agent itself, the user not asked, and the model is told
// why (`PLAN_FIRST`, in place of Mastra's "not approved by the user").

import { APP_SERVER, readsWithoutAsking } from "./reads.mjs";

/** What the model is told when its change is held: what counts, and to call again in the same reply (the user still approves the change on its card). */
export const PLAN_FIRST =
  "Not run yet, and not refused: Intcad shows the user the plan before the first change, and none has been shown in this reply yet (your thinking is not shown). Record the plan now with task_write, in the user's language: one task per change, in order, each saying how it will be checked. Intcad shows that list to the user. Then make this same call again right away, in the same reply; the user still approves the change on its card. Mark a task done only once its check has run and passed.";

/** The app's tools that change the design or write a file (crates/mcp/src/tools.rs); `execute` only when its command does not just read. */
const APP_CHANGES = ["execute", "batch", "set_parameter", "undo", "redo", "new_design", "open", "save", "export"];

/** Mastra Code's file tools that write (SDK 1.19.1, `tool-names.js`). */
const FILE_CHANGES = ["write_file", "string_replace_lsp", "ast_smart_edit", "delete_file", "mkdir"];

/**
 * A task list (Mastra's `task_updated` event) as the entries of an ACP `plan` update, which the app
 * shows as the reply's plan, a dot per step that fills as the step is under way and done. A task with
 * no text is left out; at most 64, as the app shows.
 */
export function planEntries(tasks) {
  if (!Array.isArray(tasks)) return [];
  const statuses = new Set(["pending", "in_progress", "completed"]);
  return tasks
    .map((task) => ({ content: typeof task?.content === "string" ? task.content.trim() : "", status: statuses.has(task?.status) ? task.status : "pending" }))
    .filter((entry) => entry.content)
    .slice(0, 64)
    .map((entry) => ({ content: entry.content, priority: "medium", status: entry.status }));
}

/** Whether the permission request is for a change the plan must come before. */
export function isChange(request) {
  const title = request?.toolCall?.title;
  if (typeof title !== "string") return false;
  if (FILE_CHANGES.includes(title)) return true;
  if (!title.startsWith(`${APP_SERVER}_`)) return false;
  return APP_CHANGES.includes(title.slice(APP_SERVER.length + 1)) && !readsWithoutAsking(request);
}

/**
 * The state of every chat: whether the agent wrote a task list since the user's last message, and
 * whether the reply before it ended on one with no change after it. `onProject(sessionId)` says
 * whether the chat is on a project.
 */
export class PlanGate {
  #chats = new Map();
  #held = new Set();
  #onProject;

  constructor(onProject) {
    this.#onProject = onProject;
  }

  #chat(sessionId) {
    let chat = this.#chats.get(sessionId);
    if (!chat) this.#chats.set(sessionId, (chat = { tasks: false, changed: false, carried: false }));
    return chat;
  }

  /** The user sent a message: a reply that showed a plan and changed nothing after it carries the plan over. */
  prompted(sessionId) {
    const chat = this.#chat(sessionId);
    chat.carried = chat.tasks && !chat.changed;
    chat.tasks = false;
    chat.changed = false;
  }

  /** The agent's task list is now `tasks` (Mastra's `task_updated`): a list with a step in it is a plan shown. */
  listed(sessionId, tasks) {
    if (planEntries(tasks).length > 0) this.#chat(sessionId).tasks = true;
  }

  /**
   * Whether to hold this permission request back from the user: a change, in a chat on a project,
   * with no plan shown. One let through counts as a change of this reply, so the next reply needs a
   * plan of its own.
   */
  holds(request) {
    const sessionId = request?.sessionId;
    if (!isChange(request) || !this.#onProject(sessionId)) return false;
    const chat = this.#chat(sessionId);
    if (chat.carried || chat.tasks) {
      chat.changed = true;
      return false;
    }
    const id = request.toolCall?.toolCallId;
    if (typeof id === "string") this.#held.add(id);
    return true;
  }

  /** The reason to decline the call `toolCallId` with, once: `PLAN_FIRST` when it was held, else none. */
  reasonFor(toolCallId) {
    return this.#held.delete(toolCallId) ? PLAN_FIRST : undefined;
  }
}
