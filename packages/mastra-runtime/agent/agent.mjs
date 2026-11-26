// Mastra Code's ACP agent with what Intcad adds: the connection methods, a model list a person can
// read (only models one can talk to, newest first), sessions that follow a sign-in or sign-out
// without being opened again, a reason on a turn that fails, no card for the app's commands that
// only read, and in a chat on a project no change before a plan (plan.mjs).
//
// `Base` is Mastra Code's `MastraCodeAcpAgent`. It is passed in so this file imports nothing of
// Mastra's and the tests can stand in for it.

import { AsyncLocalStorage } from 'node:async_hooks';

import { METHODS } from './connections.mjs';
import { classifyFailure } from './problems.mjs';
import { isChatModel, newestFirst, orderModelOptions } from './models.mjs';
import { PlanGate, planEntries } from './plan.mjs';
import { askingOnlyWhenNeeded } from './reads.mjs';

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
 * @param takesAttachments (model id, files) => false when the model cannot read those files (models.mjs)
 */
export function intcadAgent({ Base, RequestError, connections, dates, reloadSignIns, cad, takesAttachments = () => undefined }) {
  return class IntcadAgent extends Base {
    #gate;

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
     * agent that says so.
     */
    async initialize(request) {
      const response = await super.initialize(request);
      const capabilities = response.agentCapabilities ?? {};
      return {
        ...response,
        agentCapabilities: { ...capabilities, promptCapabilities: { ...capabilities.promptCapabilities, image: true, embeddedContext: true } },
        authMethods: [],
      };
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
     * connected can serve it. A new session starts on a model that can answer.
     */
    async newSession(request) {
      const response = await super.newSession(request);
      const entry = this.sessions.get(response.sessionId);
      if (!entry) return response;
      cad?.bind(request, entry);
      // INT-1473: the project folder the app handed over (session.mjs) is where the file tools may
      // also go. Set here, on the session's own thread: Mastra Code opens a new thread for the
      // session after building it, and the thread starts with no such folder of its own.
      if (entry.project) await entry.session.state.set({ sandboxAllowedPaths: [entry.project] });
      // Plan first (plan.mjs): a change the gate held is declined with its reason, which is what the
      // model reads in place of Mastra's "not approved by the user". Without a way to give the
      // reason, nothing is held: a bare decline would read as the user's refusal.
      const session = entry.session;
      // The files of a turn (`prompt`) go with the first message that turn sends, which is Mastra
      // Code's own `sendMessage` of the user's words, and with no other: a turn cancelled before
      // it sends leaves nothing for the next one.
      if (typeof session?.sendMessage === 'function') {
        const send = session.sendMessage.bind(session);
        session.sendMessage = input => {
          const turn = turnFiles.getStore();
          const files = turn?.files ?? [];
          if (turn) turn.files = [];
          return send(files.length ? { ...input, files } : input);
        };
      }
      if (entry.project && typeof session?.respondToToolApproval === 'function') {
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
      return { ...response, configOptions: this.configOptions(entry), ...(response.models ? { models: { ...response.models, currentModelId: entry.session.model.get() ?? '' } } : {}) };
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
    async #settle(entry, move) {
      const catalog = await entry.controller.listAvailableModels();
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
    async #usableNow(sessionId, entry, model) {
      try {
        entry.controller.invalidateAvailableModelsCache?.();
        await this.#settle(entry, false);
      } catch (error) {
        process.stderr.write(`[intcad] could not check the models of a session again: ${error}\n`);
        return false;
      }
      if (!entry.usable?.includes(model)) return false;
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
      const entry = this.sessions.get(request.sessionId);
      // A session is not moved off a model nothing serves by itself (see #settle), so a turn can be
      // asked for on one. No provider is asked: the Assistant says to pick another model.
      const using = entry?.session.model.get() ?? '';
      if (entry?.usable?.length && !entry.usable.includes(using) && !(await this.#usableNow(request.sessionId, entry, using))) {
        throw new RequestError(INTERNAL, `no connection serves ${using}`, { intcad: { kind: 'model', prefix: connections.prefixOf(using, { known: true }) } });
      }
      const { rest, files } = filesOf(request.prompt);
      // A model that cannot read them is not sent them: the Assistant says so instead.
      if (files.length && takesAttachments(using, files) === false) {
        throw new RequestError(INTERNAL, `${using} reads no pictures or documents`, { intcad: { kind: 'attachments', prefix: connections.prefixOf(using) } });
      }
      this.#gate.prompted(request.sessionId);
      try {
        const sent = files.length ? { ...request, prompt: rest } : request;
        return await turnFiles.run({ files }, () => (cad ? cad.prompt(sent, entry, () => super.prompt(sent)) : super.prompt(sent)));
      } catch (error) {
        const model = entry?.session.model.get() ?? '';
        const prefix = connections.prefixOf(model);
        const message = error instanceof Error ? error.message : String(error);
        const kind = classifyFailure(`${message} ${typeof error?.data === 'string' ? error.data : ''}`);
        if (kind === 'auth') connections.expired(prefix);
        const data = error?.data && typeof error.data === 'object' ? { ...error.data } : {};
        throw new RequestError(typeof error?.code === 'number' ? error.code : INTERNAL, message, { ...data, intcad: { kind, prefix } });
      }
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
