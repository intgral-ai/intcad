// Mastra Code's ACP agent with what Intcad adds: the connection methods, a model list a person can
// read (only models one can talk to, newest first), sessions that follow a sign-in or sign-out
// without being opened again, a reason on a turn that fails, no card for the app's commands that
// only read, and in a chat on a project no change before a plan (plan.mjs). Also `session/close`,
// which Mastra Code's ACP agent does not have, and sessions that guess the user's next step
// (`nextStep`, session.mjs), which start read-only.
//
// `Base` is Mastra Code's `MastraCodeAcpAgent`. It is passed in so this file imports nothing of
// Mastra's and the tests can stand in for it.

import { AsyncLocalStorage } from 'node:async_hooks';

import { METHODS } from './connections.mjs';
import { classifyFailure } from './problems.mjs';
import { isChatModel, newestFirst, orderModelOptions } from './models.mjs';
import { PlanGate, planEntries } from './plan.mjs';
import { askingOnlyWhenNeeded } from './reads.mjs';
import { isNextStep } from './session.mjs';

/** Mastra Code's mode that only reads (Explore): its tools look at files and change nothing. */
const READ_ONLY_MODE = 'fast';

const INVALID_PARAMS = -32602;
const METHOD_NOT_FOUND = -32601;
const INTERNAL = -32603;
const BUSY = -32000;

/**
 * A message's pictures and documents (PDFs), apart from the rest. Mastra Code's ACP agent reads text
 * and text documents and refuses the turn on a picture or a binary document, while its session
 * sends files with a message (`sendMessage({ content, files })`, as its own terminal sends a pasted
 * picture). So these go as files: `{ data: base64, mediaType, filename? }`.
 */
export function filesOf(blocks) {
  const rest = [];
  const files = [];
  for (const block of blocks ?? []) {
    if (block?.type === 'image' && typeof block.data === 'string') files.push({ data: block.data, mediaType: block.mimeType });
    else if (block?.type === 'resource' && typeof block.resource?.blob === 'string') {
      const { blob, mimeType, uri } = block.resource;
      files.push({ data: blob, mediaType: mimeType ?? 'application/octet-stream', filename: nameOf(uri) });
    } else rest.push(block);
  }
  return { rest, files };
}

/** The file name at the end of a `file:` address, or undefined. */
function nameOf(uri) {
  const last = String(uri ?? '').split('/').pop();
  try {
    return decodeURIComponent(last) || undefined;
  } catch {
    return last || undefined;
  }
}

/** The files of the turn `prompt` is sending, for the one message of that turn (see `newSession`). */
const turnFiles = new AsyncLocalStorage();

/**
 * @param Base            Mastra Code's ACP agent class
 * @param RequestError    ACP's error class (`new RequestError(code, message, data)`)
 * @param connections     the Connections of this process
 * @param dates           () => Map of release dates by model id; dates learned from Mastra's catalog are added to it
 * @param reloadSignIns   () => makes Mastra read its sign-in file again
 * @param takesAttachments (model id, files) => false (possibly async) when the model cannot read those files
 * @param isNotAdmitted    pinned SDK predicate for a startup cancellation before signal admission
 */
export function intcadAgent({
  Base, RequestError, connections, dates, reloadSignIns, cad,
  takesAttachments = () => undefined, isNotAdmitted = () => false,
}) {
  return class IntcadAgent extends Base {
    #gate;
    #prompts = new Map();

    /**
     * The app's `execute` with a command that only reads runs without a card (reads.mjs), and in a
     * chat on a project the first change of a reply waits for a plan (plan.mjs).
     */
    constructor(connection, ...rest) {
      let self;
      const gate = new PlanGate(sessionId => Boolean(self?.sessions?.get(sessionId)?.plansFirst));
      super(askingOnlyWhenNeeded(connection, gate), ...rest);
      self = this;
      this.#gate = gate;
    }

    /**
     * The sign-ins are offered on the Connections page, through the methods below, not as ACP
     * sign-in methods. Pictures and documents are taken (`prompt`): the app sends them only to an
     * agent that says so. Also advertise `session/close` without changing the SDK's other capabilities.
     */
    async initialize(request) {
      const response = await super.initialize(request);
      const capabilities = response.agentCapabilities ?? {};
      return {
        ...response,
        agentCapabilities: {
          ...capabilities,
          promptCapabilities: { ...capabilities.promptCapabilities, image: true, embeddedContext: true },
          sessionCapabilities: { ...capabilities.sessionCapabilities, close: {} },
        },
        authMethods: [],
      };
    }

    /**
     * `session/close`. Mastra Code's ACP agent keeps every session, a whole runtime with its storage,
     * workers and folder, until the connection closes; the app's guesses of the next step open one
     * each. The session is taken off the list first, so a prompt or a second close on it is refused,
     * then ended as Mastra Code ends each session when the connection closes.
     */
    async closeSession(params) {
      const sessionId = params?.sessionId;
      const entry = typeof sessionId === 'string' ? this.sessions.get(sessionId) : undefined;
      if (!entry) throw new RequestError(INVALID_PARAMS, 'Unknown ACP session', { sessionId });
      await this.#end(sessionId, entry);
      return {};
    }

    /**
     * What Mastra Code's `dispose` does for each session, for one: a turn under way is cancelled
     * and aborted, the session stops listening, and its runtime is cleaned up. (The plan gate keeps
     * a few flags per session and has no way to forget one; they stay.)
     */
    async #end(sessionId, entry) {
      this.sessions.delete(sessionId);
      this.#abortPendingPrompts(sessionId);
      for (const turn of entry.turns ?? []) turn.cancelled = true;
      if (entry.state) {
        entry.state.cancelled = true;
        entry.session.abort();
        entry.state?.resolve('aborted');
      }
      entry.unsubscribe?.();
      await entry.cleanup?.();
    }

    configOptions(entry) {
      const options = super.configOptions(entry);
      const model = options.find(option => option.id === 'model');
      if (model) {
        // Mastra Code lists the model in use even when nothing connected serves it. A session can
        // stay on such a model (see #settle); it is then not one to pick, so it is not listed.
        const order = listed => orderModelOptions(listed, dates(), model.currentValue, id => connections.isCustom(id));
        const usable = entry.usable?.length ? order(model.options.filter(option => entry.usable.includes(option.value))) : [];
        model.options = usable.length ? usable : order(model.options);
      }
      return options;
    }

    async extMethod(method, params) {
      const run = Object.hasOwn(METHODS, method) ? METHODS[method] : null;
      if (!run) throw new RequestError(METHOD_NOT_FOUND, `Method not found: ${method}`);
      try {
        return await run(connections, params ?? {});
      } catch (error) {
        if (error?.code === 'INVALID') throw new RequestError(INVALID_PARAMS, error.message);
        if (error?.code === 'BUSY') throw new RequestError(BUSY, 'busy');
        throw new RequestError(INTERNAL, error instanceof Error ? error.message : String(error));
      }
    }

    /**
     * Mastra Code opens a session on the model it used last, or on its own default, even when nothing
     * connected can serve it. A new session starts on a model that can answer. A guess of the next
     * step starts in the mode that only reads.
     */
    async newSession(request) {
      const response = await super.newSession(request);
      const entry = this.sessions.get(response.sessionId);
      if (!entry) return response;
      const nextStep = isNextStep(request);
      // A guess only looks at the design: in the read-only mode Mastra sends the model none of the
      // tools that write, run commands or reach MCP servers. Switched before the reply, so the app
      // never sees the session in another mode. Without that mode there is no safe guess, and the
      // session is ended rather than left open in one that writes.
      if (nextStep) {
        try {
          if (!entry.modes?.some(mode => mode?.id === READ_ONLY_MODE)) throw new RequestError(INTERNAL, `this agent has no read-only mode (${READ_ONLY_MODE}) for a next-step session`);
          await entry.session.mode.switch({ modeId: READ_ONLY_MODE });
        } catch (error) {
          await this.#end(response.sessionId, entry);
          throw error;
        }
      }
      cad?.bind(request, entry);
      // INT-1473: the project folder the app handed over (session.mjs) is where the file tools may
      // also go. Set here, on the session's own thread: Mastra Code opens a new thread for the
      // session after building it, and the thread starts with no such folder of its own.
      if (entry.project) await entry.session.state.set({ sandboxAllowedPaths: [entry.project] });
      // Plan first (plan.mjs): a change the gate held is declined with its reason, which is what the
      // model reads in place of Mastra's "not approved by the user". Without a way to give the
      // reason, nothing is held: a bare decline would read as the user's refusal. A guess of the next
      // step changes nothing, and has no project anyway.
      const session = entry.session;
      // The files of a turn (`prompt`) go with the first message that turn sends, which is Mastra
      // Code's own `sendMessage` of the user's words, and with no other: a turn cancelled before
      // it sends leaves nothing for the next one.
      if (typeof session?.sendMessage === 'function') {
        // Keep only MIME types, not a second copy of the session's files. The SDK can replay a
        // submitted attachment from its history, including after an interrupted answer.
        const history = new Set();
        const send = session.sendMessage.bind(session);
        session.sendMessage = async input => {
          const turn = turnFiles.getStore();
          if (turn?.entry !== entry) return send(input);
          const { signal, sessionId, files } = turn;
          signal.throwIfAborted();
          if (turn.submitted) return send(input);
          // The SDK queues model selection and prompts together. Check only now, when a
          // preceding selection has run; if refresh yields, read the model again afterwards.
          let model;
          let usable;
          let supported;
          const types = Array.from(new Set([...history, ...files.map(file => file.mediaType)]), mediaType => ({ mediaType }));
          do {
            signal.throwIfAborted();
            model = session.model.get() ?? '';
            usable = !entry.usable?.length || entry.usable.includes(model) ||
              await this.#usableNow(sessionId, entry, model, signal);
            signal.throwIfAborted();
            if (model !== (session.model.get() ?? '')) continue;
            supported = usable && types.length ? takesAttachments(model, types) : undefined;
            if (supported?.then) supported = await beforeAbort(signal, () => supported);
          } while (model !== (session.model.get() ?? ''));
          signal.throwIfAborted();
          turn.model = model;
          if (!usable) {
            throw new RequestError(INTERNAL, `no connection serves ${model}`, { intcad: { kind: 'model', prefix: connections.prefixOf(model, { known: true }) } });
          }
          if (supported === false) {
            throw new RequestError(INTERNAL, `${model} cannot read the pictures or documents in this chat`, { intcad: { kind: 'attachments', prefix: connections.prefixOf(model) } });
          }
          this.#gate.prompted(sessionId);
          turn.submitted = true;
          turn.files = [];
          let mayBeRetained = true;
          try {
            return await send(files.length ? { ...input, files } : input);
          } catch (error) {
            // The pinned Session raises this before agent.sendSignal, including when Stop
            // changes its abort generation during thread/subscription setup. Other outcomes
            // cannot prove non-admission, so keep their MIME types conservatively.
            mayBeRetained = !isNotAdmitted(error);
            throw error;
          } finally {
            if (mayBeRetained) for (const file of files) history.add(file.mediaType);
          }
        };
      }
      if (entry.project && !nextStep && typeof session?.respondToToolApproval === 'function') {
        const respond = session.respondToToolApproval.bind(session);
        const gate = this.#gate;
        session.respondToToolApproval = answer => {
          const reason = answer?.decision === 'decline' ? gate.reasonFor(answer.toolCallId) : undefined;
          return respond(reason ? { ...answer, declineContext: { ...(answer.declineContext ?? {}), reason } } : answer);
        };
        entry.plansFirst = true;
      }
      this.#showTasks(entry, response.sessionId);
      await this.#settle(entry, true);
      return {
        ...response,
        configOptions: this.configOptions(entry),
        ...(response.models ? { models: { ...response.models, currentModelId: entry.session.model.get() ?? '' } } : {}),
        // Mastra Code worked the reply's modes out before the switch above.
        ...(nextStep && response.modes ? { modes: { ...response.modes, currentModeId: entry.session.mode.get() } } : {}),
      };
    }

    /**
     * The agent's task list, shown as the reply's plan. Mastra Code's ACP agent leaves its
     * `task_updated` event out; each one is sent here as an ACP `plan` update, which the app shows
     * with a dot per step, and told to the plan gate, for which a task list is the plan (plan.mjs).
     */
    #showTasks(entry, sessionId) {
      if (typeof entry.session?.subscribe !== 'function') return;
      const gate = this.#gate;
      const unsubscribe = entry.session.subscribe(event => {
        if (event?.type !== 'task_updated') return;
        gate.listed(sessionId, event.tasks);
        const entries = planEntries(event.tasks);
        if (!entries.length) return;
        this.connection.sessionUpdate({ sessionId, update: { sessionUpdate: 'plan', entries } }).catch(error => process.stderr.write(`[intcad] could not show the task list: ${error}\n`));
      });
      if (typeof unsubscribe === 'function') {
        const previous = entry.unsubscribe;
        entry.unsubscribe = () => {
          unsubscribe();
          previous?.();
        };
      }
    }

    /**
     * Makes a session's model list what can really be used now. (Mastra Code's own list keeps the
     * model in use even when no provider is configured for it.) With `move`, a session on a model
     * nothing connected can serve goes to the newest one it may use. Without it the session keeps
     * its model and the user picks: the newest model may be one that is paid for by use, and nobody
     * chose it.
     */
    async #settle(entry, move, signal) {
      const catalog = await entry.controller.listAvailableModels();
      if (signal?.aborted) return;
      // Mastra's live catalog knows models that came out after the registry file it ships with.
      let learned = false;
      for (const model of catalog) {
        if (typeof model.releaseDate === 'string' && model.releaseDate && dates().get(model.id) !== model.releaseDate) {
          dates().set(model.id, model.releaseDate);
          learned = true;
        }
      }
      if (learned) connections.tell();
      const usable = [...new Set(catalog.filter(model => model.hasApiKey).map(model => model.id))];
      if (usable.length === 0) return;
      entry.usable = usable;
      entry.models = usable.map(id => ({ modelId: id, name: id }));
      if (move && !usable.includes(entry.session.model.get() ?? '')) await entry.session.model.switch(usable.filter(id => connections.isCustom(id) || isChatModel(id)).sort(newestFirst(dates()))[0] ?? usable[0]);
    }

    /**
     * A session's list can be older than the keys and sign-ins (opened while they changed, or after
     * a refresh that failed). Before a turn on a model it does not list is refused, the list is
     * worked out once more: whether the model can be used now. If it can, the app is told the new
     * list. A list that cannot be worked out leaves the turn refused as before.
     */
    async #usableNow(sessionId, entry, model, signal) {
      return beforeAbort(signal, () => this.#refreshForPrompt(sessionId, entry, model, signal));
    }

    async #refreshForPrompt(sessionId, entry, model, signal) {
      try {
        entry.controller.invalidateAvailableModelsCache?.();
        await this.#settle(entry, false, signal);
      } catch (error) {
        process.stderr.write(`[intcad] could not check the models of a session again: ${error}\n`);
        return false;
      }
      if (signal.aborted || !entry.usable?.includes(model)) return false;
      try {
        await this.connection.sessionUpdate({ sessionId, update: { sessionUpdate: 'config_option_update', configOptions: this.configOptions(entry) } });
      } catch (error) {
        // The turn can be answered all the same; the app learns the list with the next refresh.
        process.stderr.write(`[intcad] could not tell the app the models of a session: ${error}\n`);
      }
      return true;
    }

    /**
     * A turn that fails says why in a word the Assistant has a sentence for, and for which provider.
     * Its pictures and documents go to the model as files (`filesOf`).
     */
    async prompt(request) {
      const { sessionId } = request;
      const entry = this.sessions.get(sessionId);
      const controller = new AbortController();
      const prompts = this.#prompts.get(sessionId) ?? new Set();
      this.#prompts.set(sessionId, prompts);
      prompts.add(controller);
      const { rest, files } = filesOf(request.prompt);
      const turn = { entry, sessionId, files, signal: controller.signal, submitted: false };
      const early = Promise.withResolvers();
      const stopped = () => {
        if (turn.submitted) return;
        turn.files = [];
        early.resolve({ stopReason: 'cancelled' });
      };
      controller.signal.addEventListener('abort', stopped, { once: true });
      try {
        const sent = files.length ? { ...request, prompt: rest } : request;
        // Keep the SDK's queue intact. Its cancel marks its own queued callback before it can
        // replace entry.state; the per-turn signal independently guards our sendMessage hook.
        const run = () => Promise.race([super.prompt(sent), early.promise]);
        return await turnFiles.run(turn, () => (cad ? cad.prompt(sent, entry, run) : run()));
      } catch (error) {
        if (controller.signal.aborted) return { stopReason: 'cancelled' };
        if (error instanceof RequestError && error.data?.intcad) throw error;
        const model = turn.model ?? entry?.session.model.get() ?? '';
        const prefix = connections.prefixOf(model);
        const message = error instanceof Error ? error.message : String(error);
        const kind = classifyFailure(`${message} ${typeof error?.data === 'string' ? error.data : ''}`);
        if (kind === 'auth') connections.expired(prefix);
        const data = error?.data && typeof error.data === 'object' ? { ...error.data } : {};
        throw new RequestError(typeof error?.code === 'number' ? error.code : INTERNAL, message, { ...data, intcad: { kind, prefix } });
      } finally {
        controller.signal.removeEventListener('abort', stopped);
        prompts.delete(controller);
        if (!prompts.size) this.#prompts.delete(sessionId);
      }
    }

    async cancel(notification) {
      // Record Stop even before the SDK's queued callback or this wrapper's refresh completes.
      // The SDK still owns interruption of an already submitted message and its pending tools.
      this.#abortPendingPrompts(notification.sessionId);
      await super.cancel(notification);
    }

    #abortPendingPrompts(sessionId) {
      for (const controller of this.#prompts.get(sessionId) ?? []) controller.abort();
    }

    /**
     * After a sign-in, a sign-out or new keys: every open session finds out again which models it
     * may use and tells the app its new list. Without this a session keeps the list it was opened
     * with. Only after a sign-out (`signedOut`) does a session whose model is gone move to another
     * by itself; after anything else it keeps its model until the user picks one.
     */
    async refreshSessions({ signedOut = false } = {}) {
      reloadSignIns();
      for (const [sessionId, entry] of this.sessions) {
        try {
          // Mastra keeps the list it worked out last; this is its own call for "what can be used has changed".
          entry.controller.invalidateAvailableModelsCache?.();
          await this.#settle(entry, signedOut);
          await this.connection.sessionUpdate({ sessionId, update: { sessionUpdate: 'config_option_update', configOptions: this.configOptions(entry) } });
        } catch (error) {
          process.stderr.write(`[intcad] could not refresh the models of a session: ${error}\n`);
        }
      }
    }
  };
}

async function beforeAbort(signal, run) {
  signal.throwIfAborted();
  let abort;
  const cancelled = new Promise((_, reject) => {
    abort = () => reject(signal.reason);
    signal.addEventListener('abort', abort, { once: true });
  });
  try {
    return await Promise.race([run(), cancelled]);
  } finally {
    signal.removeEventListener('abort', abort);
  }
}
