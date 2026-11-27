// What the built-in agent runs without asking the user (INT-1473, decided 2026-11-22). Every other
// tool call shows the app a card first.
//
// - With a project, the file tools that only read (`FENCED_READS`). Each reaches files only through
//   the fenced filesystem (fence.mjs), so a read beside the project is refused, not asked about.
// - In every session, the tools of the app's own MCP server that only look at the design
//   (`APP_READS`), and its `execute` when the command only reads (`commandReadsOnly`). The app's
//   gate still checks every request they send (crates/ui-egui/src/agent_chat/gate.rs) and asks in
//   the panel before anything that names a file.
// - In every session, Mastra Code's task tools (`TASK_TOOLS`): the task list is the plan.
//
// Everything else asks: a change to a file, a command, `lsp_inspect`, the web tools (a fetch can
// carry what was read to anywhere), and the app's tools and commands that change the design or
// write files.

/** The file tools that only read. */
export const FENCED_READS = ["view", "find_files", "search_content", "file_stat"];

/**
 * The name the app gives its MCP server (`BRIDGE_NAME`, crates/ui-egui/src/agent_chat/acp.rs).
 * Mastra Code names a server's tool `<server>_<tool>` (`mcp/manager.js` of SDK 1.19.1).
 */
export const APP_SERVER = "intcad-app";

/**
 * The app's tools that only look: those its server marks read-only (`Effect::ReadOnly`,
 * crates/mcp/src/tools.rs), less `crash_reports`, which the app keeps from the agent.
 */
export const APP_READS = ["list_commands", "describe_command", "inspect_design", "measure", "body_topology", "running", "pending", "skill_reference"];

/**
 * Mastra Code's task tools (SDK 1.19.1). They change only the agent's own task list, which the
 * agent shows the user as the reply's plan (agent.mjs) and which is the plan a change waits for
 * (plan.mjs). Mastra Code lists them as never asking, but its session asks for any tool without a
 * rule: in the end-to-end run each raised a card.
 */
export const TASK_TOOLS = ["task_write", "task_update", "task_complete", "task_check"];

/** Commands that only read besides the families `commandReadsOnly` names. */
const READ_COMMANDS = ["document.inspect", "document.revision", "engine.commands", "file.slicers", "render.backends", "render.settings.get", "render.status", "view.pick", "view.status_items"];

/**
 * Whether the engine command `id` only reads the design: inspection, lists and measurements. The
 * engine's own rule (`command_reads_only`, crates/engine/src/control_pending.rs), plus `view.pick`,
 * which the window answers. Compared as the engine resolves an id: trimmed, ASCII case ignored.
 */
export function commandReadsOnly(id) {
  if (typeof id !== "string") return false;
  const command = id.trim().replace(/[A-Z]/gu, (letter) => letter.toLowerCase());
  return command.startsWith("inspect.") || command.startsWith("model.") || command.endsWith(".inspect") || command.endsWith(".list") || READ_COMMANDS.includes(command);
}

/** The permission rules a session starts with. With no rule for a category, every tool in it asks. */
export function permissionRules(project) {
  const tools = { ask_user: "deny", request_access: "deny" };
  for (const name of APP_READS) tools[`${APP_SERVER}_${name}`] = "allow";
  for (const name of TASK_TOOLS) tools[name] = "allow";
  if (project) for (const name of FENCED_READS) tools[name] = "allow";
  return { categories: {}, tools };
}

/**
 * Whether a permission request is for the app's `execute` with a command that only reads.
 * Mastra Code's request names the tool in `title` and gives its arguments, as JSON text, in
 * `rawInput` (`acp/event-mapper.js` of SDK 1.19.1). Anything else, or arguments that do not parse,
 * is asked about.
 */
export function readsWithoutAsking(request) {
  const call = request?.toolCall;
  if (call?.title !== `${APP_SERVER}_execute`) return false;
  let args = call.rawInput;
  if (typeof args === "string") {
    try {
      args = JSON.parse(args);
    } catch {
      return false;
    }
  }
  return commandReadsOnly(args?.command);
}

/**
 * The connection the agent speaks to the app through, answering itself the permission requests
 * `readsWithoutAsking` takes, with the request's own "allow once". Mastra decides approval by a
 * tool's name alone, so a per-command rule for `execute` lives here. With a `gate` (plan.mjs), a
 * change it holds is answered with the request's own "reject once": the user is not asked. Every
 * other request, and everything else the agent sends, goes to the app unchanged.
 */
export function askingOnlyWhenNeeded(connection, gate) {
  if (connection === null || typeof connection !== "object") return connection;
  return new Proxy(connection, {
    get(target, key) {
      if (key === "requestPermission") {
        return async (request) => {
          const once = request?.options?.find((option) => option?.kind === "allow_once");
          if (once && readsWithoutAsking(request)) return { outcome: { outcome: "selected", optionId: once.optionId } };
          if (gate?.holds(request)) {
            const reject = request.options?.find((option) => option?.kind === "reject_once");
            return reject ? { outcome: { outcome: "selected", optionId: reject.optionId } } : { outcome: { outcome: "cancelled" } };
          }
          return target.requestPermission(request);
        };
      }
      const value = Reflect.get(target, key, target);
      return typeof value === "function" ? value.bind(target) : value;
    },
  });
}
