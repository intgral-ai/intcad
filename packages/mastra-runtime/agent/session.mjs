// The session the agent opens for the app.
//
// This is Mastra Code's own ACP session builder (`createAcpSession`, `src/acp/runtime.ts` of
// @mastra/code-sdk 1.19.1, Apache-2.0), adapted for one difference: the session has no vector
// store. Mastra Code's builder takes no option for that, and with a vector store its memory
// embeds text with a model run on this computer (`@mastra/fastembed`, downloaded on first use),
// which Intcad's package does not carry. Without one, Mastra's memory still observes a long
// conversation and recalls from it; it does so without vectors (`retrieval: true`, no embedder).
//
// `vector: false` is what switches it off: Mastra Code makes its default store only when the
// option is absent, and treats a false one as it treats the case it has itself, a session on
// storage handed in from outside. The option is documented for a vector store to use instead,
// not for `false`, so the build checks that the SDK's file is still the one this was written
// against (`../agent-package/adapted-from.json`, checked by `scripts/agent-package.mjs`). When
// that check fails after a new version of the SDK, compare its builder with this one again.
//
// And for INT-1473, two things the app hands over in `session/new` reach Mastra:
// - The app's note (`_meta.systemPrompt.append`, or the standard `systemPrompt`) becomes Mastra
//   Code's host instructions. Without this the built-in agent never saw it.
// - A project folder (`_meta.intcad.project`) is returned as `project`, for agent.mjs to add to the
//   folders the file tools may reach (`sandboxAllowedPaths`, on the session's thread: one set in
//   `initialState` is gone once Mastra Code opens that thread), and its brief (brief.mjs) follows
//   the note. The session works in a new empty folder of its own (`sessionFolder`), not in the
//   project, so Mastra Code reads none of the project's agent files, MCP servers, hooks or skills:
//   those it takes only from the session's folder and its home.
//
// And a session the app opens to guess the user's next step (`_meta.intcad.nextStep: true`,
// `isNextStep`) is a lean one: the app's note alone, no project, no MCP at all and no design tools,
// whatever the request names. A guess only reads, and the app checks and applies it itself.

import { mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";

import { projectBrief, projectRoot } from "./brief.mjs";
import { permissionRules } from "./reads.mjs";

/**
 * The folder a session works in: a new, empty one of its own under `<data>/sessions/`, named
 * after this process, removed when the session is cleaned up (when the app closes its connection
 * to the agent; New chat leaves the old session open). The app names one folder for every session
 * (`agent-chat/work`), and Mastra Code reads agent files (`AGENTS.md`), MCP servers (`.mcp.json`)
 * and hooks from a session's folder. In a shared folder, one file put there would be loaded, or
 * run, by every later session; and sessions of one folder share one memory, so `recall` in one
 * would read another's. (The file tools do not reach the folder at all: fence.mjs.) Run by hand,
 * without a data folder, the session works in the folder it is given.
 */
export function sessionFolder(given, data = process.env.MASTRA_APP_DATA_DIR) {
  if (!data) return { folder: given, remove: () => {} };
  const root = join(data, "sessions");
  mkdirSync(root, { recursive: true });
  const folder = mkdtempSync(join(root, `s-${process.pid}-`));
  return { folder, remove: () => rmSync(folder, { recursive: true, force: true, maxRetries: 3 }) };
}

/**
 * Removes the session folders an agent that is no longer running left behind (it was stopped
 * before it cleaned up). A folder names the process that made it; one of a process still running
 * is left alone. Never throws.
 */
export function sweepSessionFolders(data, alive = isRunning) {
  const root = join(data, "sessions");
  let entries = [];
  try {
    entries = readdirSync(root, { withFileTypes: true });
  } catch {
    return [];
  }
  const removed = [];
  for (const entry of entries) {
    const pid = /^s-(\d+)-/u.exec(entry.name)?.[1];
    if (!entry.isDirectory() || !pid || Number(pid) === process.pid || alive(Number(pid))) continue;
    try {
      rmSync(join(root, entry.name), { recursive: true, force: true, maxRetries: 3 });
      removed.push(entry.name);
    } catch {
      // still held by the system: a later start tries again
    }
  }
  return removed;
}

function isRunning(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

/** The longest note taken from the app, in characters: its note and custom instructions are far shorter. */
const NOTE_LIMIT = 16000;

/** The app's note for the session, from whichever route it took; empty when it sent none. */
export function noteOf(request) {
  const meta = request?._meta?.systemPrompt?.append;
  const standard = Array.isArray(request?.systemPrompt?.content)
    ? request.systemPrompt.content.filter(part => part?.type === "text" && typeof part.text === "string").map(part => part.text).join("\n")
    : "";
  const note = typeof meta === "string" && meta.trim() ? meta : standard;
  return note.trim().slice(0, NOTE_LIMIT);
}

/** What every session of the built-in agent is told after the app's note. */
export const LANGUAGE = "Answer in the language the user writes in.";

/**
 * What a session without a project is told in place of the brief (INT-1508): the app hands a
 * project over only when a chat starts, so the way to one is a New chat.
 */
export const NO_PROJECT =
  "This chat has no project folder, so you cannot write the project's files. When the user asks to change project files (the Spec: requirements, design intent), tell them to create or open the project, then start a New chat.";

/**
 * Whether the app opened this session to guess the user's next step. Only `true` counts: anything
 * else is an ordinary chat.
 */
export function isNextStep(request) {
  return request?._meta?.intcad?.nextStep === true;
}

/**
 * What the app hands a session over its own: the note, and the project folder with its brief, or
 * null. The instructions are the note, then `LANGUAGE`, then the brief, or `NO_PROJECT`. A guess of
 * the next step gets the note alone and no project: the note says what to answer, and the lines
 * about the language and New chat are for a chat with a person.
 */
export function hostOf(request) {
  const note = noteOf(request);
  if (isNextStep(request)) return { project: null, instructions: note };
  const project = projectRoot(request?._meta?.intcad?.project);
  const brief = project ? projectBrief(project) : NO_PROJECT;
  return { project, instructions: [note, LANGUAGE, brief].filter(Boolean).join("\n\n") };
}

const noPredictionTools = {
  id: 'intcad-prediction-no-tools',
  processInputStep: () => ({ tools: {}, activeTools: [], toolChoice: 'none' }),
};

/** Check the installed Core once in this process, with no provider or credentials. */
export function predictionToolGuard(CoreAgent) {
  let verified;
  return () => (verified ??= (async () => {
    let requests = 0;
    const probe = new CoreAgent({
      id: 'intcad-prediction-tool-check',
      name: 'Intcad prediction tool check',
      instructions: 'Reply with text.',
      tools: { inspect_file: { id: 'inspect_file', description: 'Offline probe tool', execute: () => 'offline' } },
      inputProcessors: [noPredictionTools],
      model: {
        specificationVersion: 'v2', provider: 'intcad-offline', modelId: 'tool-check', supportedUrls: {},
        async doStream(request) {
          const empty = Array.isArray(request.tools) ? request.tools.length === 0 : request.tools === undefined;
          if (!empty || (request.toolChoice && request.toolChoice.type !== 'none')) {
            // `consumeStream` can report errors as chunks rather than rejecting.
            requests = Number.NaN;
          } else {
            requests += 1;
          }
          return {
            stream: new ReadableStream({
              start(controller) {
                controller.enqueue({ type: 'stream-start', warnings: [] });
                controller.enqueue({ type: 'finish', finishReason: 'stop', usage: { inputTokens: 1, outputTokens: 0, totalTokens: 1 } });
                controller.close();
              },
            }),
          };
        },
      },
    });
    const response = await probe.stream('Check the tool boundary.');
    await response.consumeStream();
    if (requests !== 1) throw new Error('the probe did not make exactly one tool-free model request');
  })());
}

/** Returns the function the ACP agent calls for `session/new`, using the installed SDK. */
export function sessionBuilder(sdk) {
  const { RequestError } = sdk;
  return async function createSession(request) {
    // Intcad's: a guess of the next step has no MCP servers, whatever the request names, so they
    // are neither checked nor started.
    const nextStep = isNextStep(request);
    if (nextStep) {
      try {
        await sdk.verifyPredictionTools();
      } catch (error) {
        throw new Error('The built-in agent could not verify tool-free predictions.', { cause: error });
      }
    }
    const names = new Set();
    const servers = nextStep ? [] : request.mcpServers.map(server => {
      if ('type' in server && server.type === 'sse') throw RequestError.invalidParams(undefined, 'Legacy SSE MCP servers are unsupported; use HTTP or stdio');
      if (names.has(server.name)) throw RequestError.invalidParams(undefined, `Duplicate MCP server name: ${server.name}`);
      names.add(server.name);
      const config = 'command' in server
        ? { command: server.command, args: server.args, env: Object.fromEntries(server.env.map(item => [item.name, item.value])), cwd: request.cwd }
        : { url: server.url, headers: Object.fromEntries(server.headers.map(item => [item.name, item.value])) };
      return [server.name, config];
    });
    const settings = sdk.loadSettings();
    const host = hostOf(request);
    // Intcad's: the session's own new folder (sessionFolder), not the one the app named.
    const own = sessionFolder(request.cwd);
    let result;
    try {
      result = await sdk.createMastraCode({
        cwd: own.folder,
        initialState: { projectPath: own.folder, yolo: false, permissionRules: permissionRules(host.project) },
        ...(host.instructions ? { hostInstructions: host.instructions } : {}),
        mcpServers: Object.fromEntries(servers),
        // Intcad's: no design tools for a guess, even when the request carries their catalog.
        ...(sdk.cad && !nextStep ? { extraTools: sdk.cad.tools(request) } : {}),
        // ACP clients can answer permission requests, but have no free-text tool response method.
        // Intcad's: no `request_access` either. Its card names no folder, and after three clicks
        // any folder of the computer was open to the file tools (INT-1473: files only in the project).
        disabledTools: ['ask_user', 'request_access'],
        planApprovalStartsGoal: false,
        unixSocketPubSub: false,
        // Intcad's: a guess loads no MCP server at all, also none from the agent's home.
        disableMcp: nextStep,
        // Fast mode alone still exposes inspection tools. Replace the effective tool set
        // on every model step, regardless of the SDK's workspace and controller defaults.
        ...(nextStep ? {
          inputProcessors: [noPredictionTools],
          disablePlugins: true,
        } : {}),
        // Intcad's: no hooks. Intcad configures none, and a hook is a command run without asking.
        disableHooks: true,
        // Project environment files must not mutate other ACP sessions.
        disableEnvFile: true,
        subagents: sdk.resolveSubagentsSetting(settings.preferences.subagentsEnabled),
        // Intcad's difference: no vector store, so no embedding model.
        vector: false,
      });
    } catch (error) {
      own.remove();
      throw error;
    }
    let cleanupPromise;
    const runtime = {
      controller: result.controller,
      session: result.session,
      modes: result.controller.listModes(),
      // The project folder this session was handed (INT-1473), or null.
      project: host.project,
      getSkills: async () => (await result.controller.resolveWorkspace({ session: result.session }))?.skills,
      getThinkingLevel: () => result.session.state.get().thinkingLevel ?? sdk.resolveDefaultThinkingLevel(settings, result.session.mode.get()).level,
      cleanup: () => (cleanupPromise ??= (async () => {
        try {
          result.releaseThreadClaims();
        } catch {
          // a claim that is already gone
        }
        await Promise.allSettled([
          Promise.resolve().then(() => result.session.abort()),
          Promise.resolve().then(() => result.session.thread.detachFromCurrent()),
          Promise.resolve().then(() => result.stopPluginSignalProviders()),
          Promise.resolve().then(() => result.githubSignals?.stopAllPolling()),
          Promise.resolve().then(() => result.threadScheduler.stop()),
        ]);
        let dispatchTimeout;
        await Promise.race([
          Promise.resolve().then(() => result.stopNotificationDispatch()).catch(() => {}),
          new Promise(done => {
            dispatchTimeout = setTimeout(done, 2000);
            dispatchTimeout.unref();
          }),
        ]).finally(() => clearTimeout(dispatchTimeout));
        await Promise.allSettled([
          Promise.resolve().then(() => result.session.thread.clearAndReleaseLock()),
          Promise.resolve().then(() => result.mcpManager?.disconnect()),
          Promise.resolve().then(() => result.controller.getMastra()?.stopWorkers()),
          Promise.resolve().then(() => result.controller.stopIntervals()),
        ]);
        await Promise.resolve().then(() => result.signalsPubSub?.close?.()).catch(() => {});
        try {
          await result.storage.close();
        } finally {
          try {
            own.remove();
          } catch (error) {
            process.stderr.write(`[intcad] a session's folder stays behind: ${error}\n`);
          }
        }
      })()),
    };
    try {
      const failed = (await result.mcpManager?.initInBackground())?.failed.filter(server => names.has(server.name)) ?? [];
      if (failed.length) throw RequestError.internalError(undefined, failed.map(server => `MCP server ${server.name}: ${server.error ?? 'connection failed'}`).join('; '));
      return runtime;
    } catch (error) {
      try {
        await runtime.cleanup?.();
      } catch (cleanupError) {
        throw sdk.withCleanupFailure(error, cleanupError);
      }
      throw error;
    }
  };
}
