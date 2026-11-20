// The connections of the built-in agent: which plans are signed in, which keys and custom addresses
// the app has handed over, and the one sign-in that may be under way. Everything the Assistant's
// Connections page shows comes from `snapshot()`, and every button on it ends in one method here.
//
// Nothing in this file talks to Mastra directly: what it needs is passed in (`deps`), so the tests
// run it against stand-ins and no test signs in anywhere.

import { classifySignInFailure } from './problems.mjs';

/** The plan sign-ins offered, in the order the page lists them, with the provider part of their model ids. */
export const PLANS = [
  ['openai-codex', 'openai'],
  ['anthropic', 'anthropic'],
  ['kimi-for-coding', 'kimi-code-plan-cn'],
  ['xai', 'xai'],
  ['github-copilot', 'github-copilot'],
];

/** The providers a key of one's own can be given for, in the order the page lists them. */
export const KEYS = [
  ['anthropic', 'Anthropic'],
  ['openai', 'OpenAI'],
  ['google', 'Google'],
  ['openrouter', 'OpenRouter'],
  ['deepseek', 'DeepSeek'],
];

// The one sign-in that ends with a code the browser shows and the user pastes back.
const PASTED = new Set(['anthropic']);
// The one sign-in whose browser calls a local address: its ports are held here (see callback.mjs).
const CALLED_BACK = new Set(['openai-codex']);

const MAX_KEY = 4096;

/**
 * A custom address's id, from its name: lower case, and a hyphen for every run of anything else.
 * Mastra forms the provider part of a model id the same way from the name it is given, so it is
 * given this id as the name and the two always agree. A name with no Latin letter or digit in it
 * (one written in Chinese, say) has nothing to form an id from and is called `custom`.
 */
export function customId(name) {
  return String(name).trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'custom';
}

function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  // Cancelling may reject it before anything waits on it; that is not an error to report.
  promise.catch(() => {});
  return { promise, resolve, reject };
}

function invalid(message) {
  return Object.assign(new Error(message), { code: 'INVALID' });
}

// A device sign-in's code, where the provider gives it only inside a sentence ("Enter code: ABCD-1234").
function codeIn(instructions) {
  return /code:?\s*([A-Z0-9]{4,}(?:-[A-Z0-9]{4,})*)/i.exec(String(instructions ?? ''))?.[1] ?? null;
}

export class Connections {
  #deps;
  #keys = new Map();
  #custom = [];
  #login = null;
  #problem = null;
  #expired = new Set();
  #callbackSentence = '';

  /**
   * @param deps.auth        () => Mastra's sign-in storage (`isLoggedIn`, `login`, `logout`, `getActiveAccount`)
   * @param deps.providers   () => the sign-ins Mastra knows: `[{ id, name }]`
   * @param deps.keyEnv      (key id) => the environment variable names Mastra reads that provider's key from
   * @param deps.env         the process environment (keys live here and nowhere else)
   * @param deps.setCustom   (list) => makes Mastra use these custom addresses instead of its settings file
   * @param deps.customPrefix (custom id) => the provider part of the ids Mastra gives that address's models
   * @param deps.openUrl     (url) => opens the user's browser
   * @param deps.holdPorts   (onCallback) => Promise<{ close() }>, see callback.mjs; `onCallback` answers whether the call was the one waited for
   * @param deps.dates       () => Map of release dates by model id
   * @param deps.changed     (snapshot) => tells the app
   * @param deps.refresh     () => Promise: every open session re-reads which models it may use
   */
  constructor(deps) {
    this.#deps = deps;
  }

  snapshot() {
    const auth = this.#deps.auth();
    const known = new Map(this.#deps.providers().map(provider => [provider.id, provider.name]));
    const plans = PLANS.filter(([id]) => known.has(id)).map(([id, prefix]) => ({
      id,
      name: known.get(id),
      state: this.#expired.has(id) ? 'expired' : auth.isLoggedIn(id) ? 'in' : 'out',
      account: auth.getActiveAccount?.(id)?.label ?? null,
      prefix,
    }));
    const served = new Set(plans.filter(plan => plan.state === 'in').map(plan => plan.prefix));
    // Mastra uses a plan's sign-in for as long as it holds one, refused or not: the key waits behind it.
    const held = new Set(plans.filter(plan => plan.state !== 'out').map(plan => plan.prefix));
    const keys = KEYS.map(([id, name]) => ({ id, name, saved: this.#keys.has(id), shadowed: this.#keys.has(id) && held.has(id), prefix: id }));
    const custom = this.#custom.map(entry => ({
      id: entry.id,
      name: entry.name,
      url: entry.url,
      models: [...entry.models],
      keyed: Boolean(entry.apiKey),
      prefix: this.#deps.customPrefix(entry.id),
    }));
    for (const key of keys) if (key.saved) served.add(key.prefix);
    const dates = {};
    for (const [model, date] of this.#deps.dates()) if (served.has(model.slice(0, model.indexOf('/')))) dates[model] = date;
    // A custom address's models have no dates: nothing knows when they came out.
    const login = this.#login && { id: this.#login.id, phase: this.#login.phase, url: this.#login.url, userCode: this.#login.userCode, error: this.#login.error };
    return { plans, keys, custom, login, problem: this.#problem, dates };
  }

  #tell() {
    this.#deps.changed(this.snapshot());
  }

  /** Something the snapshot is made from changed outside this class (the release dates): the app is told. */
  tell() {
    this.#tell();
  }

  /**
   * Replaces every key and custom address. Keys go into this process's environment and nowhere
   * else. A custom address that cannot be used (the app checks them first, so this is one from a
   * file changed by hand) is left out and the rest is taken: one bad entry does not cost the keys.
   */
  async setup(params) {
    const keys = new Map();
    for (const [id] of KEYS) {
      const key = params?.keys?.[id];
      if (key === undefined || key === null || key === '') continue;
      if (typeof key !== 'string' || key.length > MAX_KEY) throw invalid(`keys.${id} must be a string of at most ${MAX_KEY} characters`);
      keys.set(id, key.trim());
    }
    const custom = [];
    const seen = new Set();
    for (const entry of params?.custom ?? []) {
      const name = typeof entry?.name === 'string' ? entry.name.trim() : '';
      const url = typeof entry?.url === 'string' ? entry.url.trim() : '';
      const models = Array.isArray(entry?.models) ? entry.models.filter(model => typeof model === 'string' && model.trim()).map(model => model.trim()) : [];
      if (!name || !/^https?:\/\//i.test(url) || models.length === 0) continue;
      // Two names can give one id ("My LLM" and "my-llm", or any two without Latin letters). The
      // later one gets a number, in the order the app keeps them, so both stay usable.
      let id = customId(name);
      for (let n = 2; seen.has(id); n += 1) id = `${customId(name)}-${n}`;
      seen.add(id);
      custom.push({ id, name, url, models, ...(typeof entry.apiKey === 'string' && entry.apiKey ? { apiKey: entry.apiKey } : {}) });
    }
    for (const [id] of KEYS) {
      const [first, ...others] = this.#deps.keyEnv(id);
      for (const name of [first, ...others]) delete this.#deps.env[name];
      if (keys.has(id) && first) this.#deps.env[first] = keys.get(id);
    }
    this.#keys = keys;
    this.#custom = custom;
    const sentence = params?.words?.callback;
    this.#callbackSentence = typeof sentence === 'string' && sentence.length <= 400 ? sentence : '';
    // Mastra gets the id as the name: its model ids are then `<its gateway>/<id>/<model>`.
    this.#deps.setCustom(custom.map(entry => ({ name: entry.id, url: entry.url, models: [...entry.models], ...(entry.apiKey ? { apiKey: entry.apiKey } : {}) })));
    await this.#deps.refresh();
    this.#tell();
    return this.snapshot();
  }

  list() {
    return this.snapshot();
  }

  /** Starts a sign-in and returns at once; how it goes is told through `changed`. */
  login(params) {
    const id = params?.id;
    if (!PLANS.some(([plan]) => plan === id) || !this.#deps.providers().some(provider => provider.id === id)) throw invalid(`unknown sign-in: ${id}`);
    if (this.#login) throw Object.assign(new Error('busy'), { code: 'BUSY' });
    const login = { id, phase: 'starting', url: null, userCode: null, error: null, abort: new AbortController(), pasted: null, ports: null };
    this.#login = login;
    this.#problem = null;
    this.#tell();
    void this.#run(login);
    return this.snapshot();
  }

  async #run(login) {
    const { id } = login;
    try {
      const callbacks = {
        signal: login.abort.signal,
        onProgress: () => {},
        onAuth: info => {
          if (this.#login !== login) return;
          const userCode = info.userCode ?? codeIn(info.instructions);
          login.url = info.url;
          login.userCode = userCode && !String(info.url).includes(userCode) ? userCode : null;
          login.phase = login.userCode ? 'device' : PASTED.has(id) ? 'code' : 'browser';
          this.#deps.openUrl(info.url);
          this.#tell();
        },
        // Two sign-ins ask something: Claude for the code its page shows, which the user pastes;
        // GitHub Copilot for a company address, which Intcad does not offer and answers with none.
        onPrompt: () => {
          if (!PASTED.has(id)) return Promise.resolve('');
          login.pasted = deferred();
          return login.pasted.promise;
        },
      };
      if (CALLED_BACK.has(id)) {
        const calledBack = deferred();
        login.abort.signal.addEventListener('abort', () => calledBack.reject(new Error('Login cancelled')));
        login.ports = await this.#deps.holdPorts(url => {
          // Mastra puts a `state` in the address it opens, and the browser brings it back. A call
          // with another one is not this sign-in's: it is refused here and the wait goes on, where
          // handing it to Mastra would end the sign-in as failed.
          const expected = login.url ? new URL(login.url).searchParams.get('state') : null;
          if (expected && new URL(url).searchParams.get('state') !== expected) return false;
          calledBack.resolve(url);
          return true;
        });
        if (this.#login !== login) return login.ports.close();
        callbacks.authMode = 'browser';
        callbacks.onManualCodeInput = () => calledBack.promise;
      }
      await this.#deps.auth().login(id, callbacks);
      if (this.#login !== login) return;
      this.#login = null;
      this.#expired.delete(id);
      await this.#deps.refresh();
    } catch (error) {
      // Cancelled: the user asked for it, there is nothing to report.
      if (this.#login !== login) return;
      this.#login = null;
      this.#problem = { id, kind: classifySignInFailure(error) };
    } finally {
      login.ports?.close();
      this.#tell();
    }
  }

  /**
   * The code the browser showed. One that has not the shape of a code is refused and the sign-in
   * goes on. With no sign-in waiting for one (the button pressed twice, the first code already
   * taken) there is nothing to do, and nothing to refuse either.
   */
  code(params) {
    const login = this.#login;
    if (!login || login.id !== params?.id || !login.pasted) return this.snapshot();
    const code = typeof params.code === 'string' ? params.code.trim() : '';
    if (code.length > MAX_KEY || !/^\S+#\S+$/.test(code)) {
      login.error = 'incomplete';
    } else {
      login.error = null;
      login.pasted.resolve(code);
    }
    this.#tell();
    return this.snapshot();
  }

  reopen() {
    if (this.#login?.url) this.#deps.openUrl(this.#login.url);
    return this.snapshot();
  }

  cancel() {
    const login = this.#login;
    this.#login = null;
    this.#problem = null;
    if (login) {
      login.abort.abort();
      login.pasted?.reject(new Error('Login cancelled'));
      login.ports?.close();
    }
    this.#tell();
    return this.snapshot();
  }

  async logout(params) {
    const id = params?.id;
    if (!PLANS.some(([plan]) => plan === id)) throw invalid(`unknown sign-in: ${id}`);
    this.#deps.auth().logout(id);
    this.#expired.delete(id);
    this.#problem = null;
    // The one change after which a session may go to another model by itself (agent.mjs).
    await this.#deps.refresh({ signedOut: true });
    this.#tell();
    return this.snapshot();
  }

  /**
   * The connection part of a model id: the longest prefix of a connection that it starts with (a
   * custom address's has two parts), else what stands before the first slash.
   */
  prefixOf(model) {
    const prefixes = [...PLANS.map(([, prefix]) => prefix), ...KEYS.map(([id]) => id), ...this.#custom.map(entry => this.#deps.customPrefix(entry.id))];
    const match = prefixes.filter(prefix => model.startsWith(`${prefix}/`)).sort((a, b) => b.length - a.length)[0];
    return match ?? model.slice(0, Math.max(model.indexOf('/'), 0));
  }

  /** Whether a model is one of a custom address: its name was typed by the user, who knows what it is. */
  isCustom(model) {
    return this.#custom.some(entry => model.startsWith(`${this.#deps.customPrefix(entry.id)}/`));
  }

  /** What the browser shows after the ChatGPT sign-in page has called back, in the user's language; empty when the app gave none. */
  callbackSentence() {
    return this.#callbackSentence;
  }

  /** A turn was refused for lack of a valid sign-in: the plan that serves that provider shows as expired. */
  expired(prefix) {
    const auth = this.#deps.auth();
    const plan = PLANS.find(([id, served]) => served === prefix && auth.isLoggedIn(id));
    if (!plan || this.#expired.has(plan[0])) return;
    this.#expired.add(plan[0]);
    this.#tell();
  }

  /** Ends a sign-in that is under way, when the process is about to go. */
  dispose() {
    const login = this.#login;
    this.#login = null;
    login?.abort.abort();
    login?.pasted?.reject(new Error('Login cancelled'));
    login?.ports?.close();
  }
}

export const METHODS = {
  '_intcad/connections/setup': (connections, params) => connections.setup(params),
  '_intcad/connections/list': connections => connections.list(),
  '_intcad/connections/login': (connections, params) => connections.login(params),
  '_intcad/connections/code': (connections, params) => connections.code(params),
  '_intcad/connections/reopen': connections => connections.reopen(),
  '_intcad/connections/cancel': connections => connections.cancel(),
  '_intcad/connections/logout': (connections, params) => connections.logout(params),
};
