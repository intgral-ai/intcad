// Mastra Code's ACP agent with what Intcad adds: the connection methods, a model list a person can
// read (only models one can talk to, newest first), sessions that follow a sign-in or sign-out
// without being opened again, and a reason on a turn that fails.
//
// `Base` is Mastra Code's `MastraCodeAcpAgent`. It is passed in so this file imports nothing of
// Mastra's and the tests can stand in for it.

import { METHODS } from './connections.mjs';
import { classifyFailure } from './problems.mjs';
import { isChatModel, newestFirst, orderModelOptions } from './models.mjs';

const INVALID_PARAMS = -32602;
const METHOD_NOT_FOUND = -32601;
const INTERNAL = -32603;
const BUSY = -32000;

/**
 * @param Base            Mastra Code's ACP agent class
 * @param RequestError    ACP's error class (`new RequestError(code, message, data)`)
 * @param connections     the Connections of this process
 * @param dates           () => Map of release dates by model id; dates learned from Mastra's catalog are added to it
 * @param reloadSignIns   () => makes Mastra read its sign-in file again
 */
export function intcadAgent({ Base, RequestError, connections, dates, reloadSignIns }) {
  return class IntcadAgent extends Base {
    /** The sign-ins are offered on the Connections page, through the methods below, not as ACP sign-in methods. */
    async initialize(request) {
      const response = await super.initialize(request);
      return { ...response, authMethods: [] };
    }

    configOptions(entry) {
      const options = super.configOptions(entry);
      const model = options.find(option => option.id === 'model');
      if (model) model.options = orderModelOptions(model.options, dates(), model.currentValue, id => connections.isCustom(id));
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
      await this.#settle(entry, true);
      return { ...response, configOptions: this.configOptions(entry), ...(response.models ? { models: { ...response.models, currentModelId: entry.session.model.get() ?? '' } } : {}) };
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

    /** A turn that fails says why in a word the Assistant has a sentence for, and for which provider. */
    async prompt(request) {
      const entry = this.sessions.get(request.sessionId);
      // A session is not moved off a model nothing serves by itself (see #settle), so a turn can be
      // asked for on one. No provider is asked: the Assistant says to pick another model.
      const using = entry?.session.model.get() ?? '';
      if (entry?.usable?.length && !entry.usable.includes(using)) {
        throw new RequestError(INTERNAL, `no connection serves ${using}`, { intcad: { kind: 'model', prefix: connections.prefixOf(using) } });
      }
      try {
        return await super.prompt(request);
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
