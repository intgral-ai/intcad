// The built-in agent's connections (`agent/*.mjs`), run against stand-ins for Mastra: no test signs
// in anywhere, opens a browser or reads a key from this computer. The modules are plain JavaScript
// outside `src/`, so they are loaded by address from the compiled test's own place.

import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";

type Json = Record<string, any>;
// Compiled, this file is in `dist/test/`; the modules stay where they are written.
const agentFolder = import.meta.url.includes("/dist/test/") ? "../../agent/" : "../agent/";
const load = (name: string): Promise<Json> => import(new URL(agentFolder + name, import.meta.url).href);
const { Connections, METHODS, customId } = await load("connections.mjs");
const { holdCallbackPorts } = await load("callback.mjs");
const { isChatModel, orderModelOptions, releaseDates } = await load("models.mjs");
const { classifyFailure, classifySignInFailure } = await load("problems.mjs");
const { intcadAgent } = await load("agent.mjs");

const PROVIDERS = [
  { id: "openai-codex", name: "ChatGPT Plus/Pro (Codex Subscription)" },
  { id: "anthropic", name: "Anthropic (Claude Pro/Max)" },
  { id: "kimi-for-coding", name: "Kimi For Coding" },
  { id: "xai", name: "xAI (Grok)" },
  { id: "github-copilot", name: "GitHub Copilot" },
];
const ENV_NAMES: Record<string, string[]> = {
  anthropic: ["ANTHROPIC_API_KEY"],
  openai: ["OPENAI_API_KEY"],
  google: ["GOOGLE_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY"],
  openrouter: ["OPENROUTER_API_KEY"],
  deepseek: ["DEEPSEEK_API_KEY"],
};
const DATES = new Map([
  ["openai/gpt-6.2-sol", "2026-11-17"],
  ["openai/gpt-5", "2025-08-07"],
  ["anthropic/claude-fable-5-2", "2026-11-09"],
  ["deepseek/deepseek-chat", "2025-01-01"],
]);

/** A stand-in for Mastra's sign-in storage: what `login` does is decided by each test. */
function world(overrides: Json = {}) {
  const signedIn = new Set<string>(overrides.signedIn ?? []);
  const told: Json[] = [];
  const opened: string[] = [];
  const written: string[] = [];
  const env: Record<string, string> = { PATH: "/usr/bin", GOOGLE_GENERATIVE_AI_API_KEY: "left over" };
  const held: { close: () => void; closed: boolean; callback: (url: string) => void }[] = [];
  let custom: Json[] | null = null;
  let refreshed = 0;
  const auth = {
    isLoggedIn: (id: string) => signedIn.has(id),
    getActiveAccount: (id: string) => (signedIn.has(id) ? { label: `${id}@example.com` } : undefined),
    logout: (id: string) => void signedIn.delete(id),
    setStoredApiKey: (id: string) => void written.push(id),
    login: async (id: string, callbacks: Json) => {
      await overrides.login?.(id, callbacks);
      signedIn.add(id);
    },
  };
  const connections = new Connections({
    auth: () => auth,
    providers: () => overrides.providers ?? PROVIDERS,
    keyEnv: (id: string) => ENV_NAMES[id] ?? [],
    env,
    setCustom: (list: Json[]) => void (custom = list),
    customPrefix: (id: string) => `mastracode/${id}`,
    openUrl: (url: string) => void opened.push(url),
    holdPorts:
      overrides.holdPorts ??
      (async (callback: (url: string) => void) => {
        const ports = { closed: false, callback, close: () => void (ports.closed = true) };
        held.push(ports);
        return ports;
      }),
    dates: () => DATES,
    changed: (snapshot: Json) => void told.push(snapshot),
    refresh: async () => void (refreshed += 1),
  });
  return { connections, signedIn, told, opened, written, env, held, custom: () => custom, refreshed: () => refreshed };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 5));

test("with nothing connected the snapshot lists the five plans and five keys, each as not connected", () => {
  const { connections } = world();
  const snapshot = connections.list();
  assert.deepEqual(snapshot.plans.map((plan: Json) => [plan.id, plan.state, plan.prefix]), [
    ["openai-codex", "out", "openai"],
    ["anthropic", "out", "anthropic"],
    ["kimi-for-coding", "out", "kimi-code-plan-cn"],
    ["xai", "out", "xai"],
    ["github-copilot", "out", "github-copilot"],
  ]);
  assert.deepEqual(snapshot.keys.map((key: Json) => [key.id, key.saved]), [
    ["anthropic", false],
    ["openai", false],
    ["google", false],
    ["openrouter", false],
    ["deepseek", false],
  ]);
  assert.deepEqual([snapshot.custom, snapshot.login, snapshot.problem, snapshot.dates], [[], null, null, {}]);
});

test("a sign-in Mastra does not know is left out of the list", () => {
  const { connections } = world({ providers: PROVIDERS.slice(0, 2) });
  assert.deepEqual(connections.list().plans.map((plan: Json) => plan.id), ["openai-codex", "anthropic"]);
});

test("keys go into the process environment and nowhere else, and a removed key leaves it", async () => {
  const { connections, env, written, refreshed } = world();
  let snapshot = await connections.setup({ keys: { anthropic: " sk-ant-example ", google: "g-example" }, custom: [] });
  assert.equal(env.ANTHROPIC_API_KEY, "sk-ant-example");
  // The first name the registry lists gets the key; a value left under another of its names goes.
  assert.equal(env.GOOGLE_API_KEY, "g-example");
  assert.equal(env.GOOGLE_GENERATIVE_AI_API_KEY, undefined);
  assert.deepEqual(snapshot.keys.filter((key: Json) => key.saved).map((key: Json) => key.id), ["anthropic", "google"]);
  assert.ok(!JSON.stringify(snapshot).includes("sk-ant-example"), "the snapshot never carries a key");
  assert.deepEqual(written, [], "Mastra's own key store, a plain file, is never written");
  assert.equal(refreshed(), 1);
  assert.equal(snapshot.dates["anthropic/claude-fable-5-2"], "2026-11-09");
  snapshot = await connections.setup({ keys: {}, custom: [] });
  assert.equal(env.ANTHROPIC_API_KEY, undefined);
  assert.equal(env.PATH, "/usr/bin");
  assert.deepEqual(snapshot.dates, {});
});

test("a key is shadowed while a signed-in plan serves the same models", async () => {
  const { connections } = world({ signedIn: ["openai-codex"] });
  const snapshot = await connections.setup({ keys: { openai: "sk-example", deepseek: "sk-d" }, custom: [] });
  assert.deepEqual(snapshot.keys.filter((key: Json) => key.saved).map((key: Json) => [key.id, key.shadowed]), [
    ["openai", true],
    ["deepseek", false],
  ]);
  assert.equal(snapshot.plans[0].account, "openai-codex@example.com");
  assert.equal(snapshot.dates["openai/gpt-6.2-sol"], "2026-11-17");
  // A sign-in that was refused is still what Mastra uses, so the key stays behind it until the user signs out.
  connections.expired("openai");
  assert.equal(connections.list().keys[1].shadowed, true);
  assert.equal((await connections.logout({ id: "openai-codex" })).keys[1].shadowed, false);
});

test("custom addresses are handed to Mastra from memory, with the key kept out of the snapshot", async () => {
  const { connections, custom } = world();
  const snapshot = await connections.setup({ custom: [{ name: "My LLM", url: "http://localhost:11434/v1", apiKey: "secret-key", models: ["llama3.2", " qwen3 "] }] });
  assert.deepEqual(snapshot.custom, [{ id: "my-llm", name: "My LLM", url: "http://localhost:11434/v1", models: ["llama3.2", "qwen3"], keyed: true, prefix: "mastracode/my-llm" }]);
  assert.equal(connections.prefixOf("mastracode/my-llm/llama3.2"), "mastracode/my-llm");
  assert.equal(connections.prefixOf("openai/gpt-5"), "openai");
  assert.equal(connections.prefixOf("someone/else/model"), "someone");
  assert.ok(!JSON.stringify(snapshot).includes("secret-key"));
  // Mastra is given the id as the name, so the provider part it forms is the id.
  assert.deepEqual(custom(), [{ name: "my-llm", url: "http://localhost:11434/v1", models: ["llama3.2", "qwen3"], apiKey: "secret-key" }]);
  assert.equal(customId("  Über LLM 2 "), "ber-llm-2");
  assert.equal(customId("本地模型"), "custom");
});

test("a custom address without a name, an http address or a model is refused; names that give one id each get their own", async () => {
  const { connections } = world();
  for (const entry of [
    { name: "", url: "http://x/v1", models: ["m"] },
    { name: "a", url: "ftp://x", models: ["m"] },
    { name: "a", url: "http://x/v1", models: [] },
  ]) {
    await assert.rejects(connections.setup({ custom: [entry] }), { code: "INVALID" });
  }
  const same = await connections.setup({ custom: [{ name: "A b", url: "http://x/v1", models: ["m"] }, { name: "a-b", url: "http://y/v1", models: ["m"] }, { name: "本地模型", url: "http://z/v1", models: ["m"] }, { name: "另一个", url: "http://w/v1", models: ["m"] }] });
  assert.deepEqual(same.custom.map((entry: Json) => [entry.name, entry.id, entry.prefix]), [
    ["A b", "a-b", "mastracode/a-b"],
    ["a-b", "a-b-2", "mastracode/a-b-2"],
    ["本地模型", "custom", "mastracode/custom"],
    ["另一个", "custom-2", "mastracode/custom-2"],
  ]);
  assert.equal(connections.prefixOf("mastracode/a-b-2/m"), "mastracode/a-b-2");
  await assert.rejects(connections.setup({ keys: { openai: "x".repeat(5000) } }), { code: "INVALID" });
});

test("a browser sign-in holds the local ports, opens the browser, waits without a limit and ends when the browser calls back", async () => {
  let finish: (value?: unknown) => void = () => {};
  const { connections, told, opened, held, refreshed } = world({
    login: async (_id: string, callbacks: Json) => {
      assert.equal(callbacks.authMode, "browser");
      callbacks.onAuth({ url: "https://auth.example/authorize?state=s1" });
      // What Mastra does when its ports are taken: wait for the address the browser was sent back to.
      const address = await callbacks.onManualCodeInput();
      assert.equal(address, "http://localhost:1457/auth/callback?code=c1&state=s1");
      await new Promise((resolve) => (finish = resolve));
    },
  });
  const started = connections.login({ id: "openai-codex" });
  assert.deepEqual(started.login, { id: "openai-codex", phase: "starting", url: null, userCode: null, error: null });
  await settle();
  assert.deepEqual(connections.list().login, { id: "openai-codex", phase: "browser", url: "https://auth.example/authorize?state=s1", userCode: null, error: null });
  assert.deepEqual(opened, ["https://auth.example/authorize?state=s1"]);
  // Nothing in here counts time: however long the browser takes, the sign-in is still under way.
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.equal(connections.list().login.phase, "browser");
  assert.equal(held.length, 1);
  assert.equal(held[0]!.closed, false);
  connections.reopen();
  assert.equal(opened.length, 2);
  held[0]!.callback("http://localhost:1457/auth/callback?code=c1&state=s1");
  await settle();
  finish();
  await settle();
  const last = told.at(-1)!;
  assert.equal(last.login, null);
  assert.equal(last.problem, null);
  assert.equal(last.plans[0].state, "in");
  assert.equal(held[0]!.closed, true, "the ports are given back once the sign-in is over");
  assert.equal(refreshed(), 1, "open sessions learn of the new models");
});

test("cancelling a sign-in frees the ports at once, leaves no problem, and a new one can start", async () => {
  const { connections, held, told, signedIn } = world({
    login: (_id: string, callbacks: Json) => callbacks.onManualCodeInput(),
  });
  connections.login({ id: "openai-codex" });
  await settle();
  assert.throws(() => connections.login({ id: "xai" }), { code: "BUSY" });
  const snapshot = connections.cancel();
  assert.deepEqual([snapshot.login, snapshot.problem], [null, null]);
  assert.equal(held[0]!.closed, true);
  await settle();
  assert.equal(told.at(-1)!.problem, null, "the sign-in that was cancelled reports nothing afterwards");
  assert.equal(signedIn.size, 0);
  connections.login({ id: "openai-codex" });
  await settle();
  assert.equal(held.length, 2);
  connections.cancel();
});

test("a sign-in that shows a code in the browser takes it pasted, and refuses one that is not whole", async () => {
  const { connections, told } = world({
    login: async (_id: string, callbacks: Json) => {
      callbacks.onAuth({ url: "https://claude.example/authorize" });
      assert.equal(await callbacks.onPrompt({ message: "Paste the authorization code:" }), "abc#def");
    },
  });
  connections.login({ id: "anthropic" });
  await settle();
  assert.equal(connections.list().login.phase, "code");
  assert.equal(connections.code({ id: "anthropic", code: "abc" }).login.error, "incomplete");
  assert.equal(connections.list().login.phase, "code", "the sign-in is still waiting");
  assert.throws(() => connections.code({ id: "xai", code: "abc#def" }), { code: "INVALID" });
  connections.code({ id: "anthropic", code: " abc#def " });
  await settle();
  assert.equal(told.at(-1)!.plans[1].state, "in");
  assert.equal(told.at(-1)!.login, null);
});

test("a device sign-in shows the code to type, taken from the sentence when it is only there, and the company address is not asked for", async () => {
  let finish: (value?: unknown) => void = () => {};
  const { connections } = world({
    login: async (_id: string, callbacks: Json) => {
      assert.equal(await callbacks.onPrompt({ message: "GitHub Enterprise URL/domain (blank for github.com)" }), "");
      callbacks.onAuth({ url: "https://github.com/login/device", instructions: "Enter code: ABCD-1234" });
      await new Promise((resolve) => (finish = resolve));
    },
  });
  connections.login({ id: "github-copilot" });
  await settle();
  assert.deepEqual(connections.list().login, { id: "github-copilot", phase: "device", url: "https://github.com/login/device", userCode: "ABCD-1234", error: null });
  finish();
  await settle();
  assert.equal(connections.list().plans[4].state, "in");
});

test("a sign-in whose address already carries the code needs nothing typed", async () => {
  const { connections } = world({
    login: async (_id: string, callbacks: Json) => {
      callbacks.onAuth({ url: "https://x.example/device?user_code=WXYZ-9876", userCode: "WXYZ-9876" });
      await new Promise(() => {});
    },
  });
  connections.login({ id: "xai" });
  await settle();
  assert.equal(connections.list().login.phase, "browser");
  assert.equal(connections.list().login.userCode, null);
  connections.cancel();
});

test("a sign-in that cannot finish says why: the port, the network, or anything else", async () => {
  const port = world({ holdPorts: async () => Promise.reject(Object.assign(new Error("port 1457 is in use"), { code: "PORT_BUSY" })) });
  port.connections.login({ id: "openai-codex" });
  await settle();
  assert.deepEqual(port.connections.list().problem, { id: "openai-codex", kind: "port" });
  assert.equal(port.connections.list().login, null);

  const offline = world({ login: async () => Promise.reject(new TypeError("fetch failed", { cause: Object.assign(new Error("getaddrinfo"), { code: "ENOTFOUND" }) })) });
  offline.connections.login({ id: "xai" });
  await settle();
  assert.deepEqual(offline.connections.list().problem, { id: "xai", kind: "offline" });

  const other = world({ login: async () => Promise.reject(new Error("Token exchange failed")) });
  other.connections.login({ id: "anthropic" });
  await settle();
  assert.deepEqual(other.connections.list().problem, { id: "anthropic", kind: "failed" });
  // The next attempt starts clean.
  other.connections.cancel();
  assert.equal(other.connections.list().problem, null);
});

test("signing out, and a refused sign-in showing as expired until the next sign-in or sign-out", async () => {
  const { connections, told, refreshed } = world({ signedIn: ["openai-codex", "anthropic"] });
  connections.expired("openai");
  assert.equal(told.at(-1)!.plans[0].state, "expired");
  connections.expired("deepseek");
  assert.equal(told.length, 1, "a provider no plan serves changes nothing");
  const snapshot = await connections.logout({ id: "openai-codex" });
  assert.equal(snapshot.plans[0].state, "out");
  assert.equal(snapshot.plans[1].state, "in");
  assert.equal(refreshed(), 1);
  await assert.rejects(connections.logout({ id: "nobody" }), { code: "INVALID" });
});

test("the two local ports are held until closed, a callback is handed over, and a taken port is said to be busy", async () => {
  const got: string[] = [];
  const base = 41455;
  const ports = await holdCallbackPorts(http, (url: string) => void got.push(url), [base, base + 2]);
  // Mastra asks the holder to step aside before it gives the port up; the holder stays.
  assert.equal((await fetch(`http://127.0.0.1:${base}/cancel`)).status, 200);
  assert.equal((await fetch(`http://127.0.0.1:${base + 2}/nothing`)).status, 404);
  assert.equal((await fetch(`http://127.0.0.1:${base + 2}/auth/callback?state=s`)).status, 404, "a call without a code is not the sign-in");
  const response = await fetch(`http://127.0.0.1:${base + 2}/auth/callback?code=c1&state=s1`);
  assert.equal(response.status, 200);
  assert.match(await response.text(), /go back to Intcad/);
  assert.deepEqual(got, [`http://localhost:${base + 2}/auth/callback?code=c1&state=s1`]);
  await assert.rejects(holdCallbackPorts(http, () => {}, [base + 4, base + 2]), { code: "PORT_BUSY" });
  ports.close();
  // Free again at once; and the first port only has to be out of Mastra's reach, so another holder of it is fine.
  const again = await holdCallbackPorts(http, () => {}, [base, base + 2]);
  const second = await holdCallbackPorts(http, () => {}, [base, base + 6]);
  again.close();
  second.close();
});

test("only models one can talk to are listed, newest first, and the one in use always stays", () => {
  assert.equal(isChatModel("openai/gpt-6.2-sol"), true);
  for (const id of ["openai/gpt-image-2", "openai/text-embedding-3-small", "openai/gpt-realtime-2.1", "openai/chatgpt-image-latest", "google/veo-3"]) {
    assert.equal(isChatModel(id), false, id);
  }
  const options = ["openai/gpt-5", "openai/text-embedding-3-small", "openai/gpt-6.2-sol", "openai/gpt-image-2", "openai/undated-b", "openai/undated-a"].map((value) => ({ value, name: value }));
  assert.deepEqual(orderModelOptions(options, DATES, "openai/gpt-5").map((option: Json) => option.value), ["openai/gpt-6.2-sol", "openai/gpt-5", "openai/undated-a", "openai/undated-b"]);
  assert.deepEqual(orderModelOptions(options, DATES, "openai/gpt-image-2").map((option: Json) => option.value)[0], "openai/gpt-6.2-sol");
  assert.ok(orderModelOptions(options, DATES, "openai/gpt-image-2").some((option: Json) => option.value === "openai/gpt-image-2"));
  assert.deepEqual([...releaseDates({ providers: { openai: { modelReleaseDates: { "gpt-5": "2025-08-07", bad: 5 } }, none: {} } })], [["openai/gpt-5", "2025-08-07"]]);
});

test("a failed turn is put into one of the kinds the Assistant has words for", () => {
  const cases: [string, string][] = [
    ["Incorrect API key provided: sk-****. You can find your API key at …", "auth"],
    ["401 Unauthorized", "auth"],
    ["Could not find API key process.env.OPENAI_API_KEY for model id openai/gpt-5.6-sol", "auth"],
    ["OAuth token has expired. Please sign in again.", "auth"],
    ["You exceeded your current quota, please check your plan and billing details.", "quota"],
    ["429 insufficient_quota", "quota"],
    ["Rate limit reached for requests", "rate"],
    ["429 Too Many Requests", "rate"],
    ["TypeError: fetch failed", "network"],
    ["getaddrinfo ENOTFOUND api.example.com", "network"],
    ["The model `gpt-9` does not exist or you do not have access to it.", "model"],
    ["404 Not Found", "model"],
    ["Something unexpected happened", "other"],
  ];
  for (const [message, kind] of cases) assert.equal(classifyFailure(message), kind, message);
  assert.equal(classifySignInFailure(Object.assign(new Error("x"), { code: "PORT_BUSY" })), "port");
});

test("the agent answers the connection methods, refuses unknown ones, orders the model list, and says why a turn failed", async () => {
  class RequestError extends Error {
    code: number;
    data: unknown;
    constructor(code: number, message: string, data?: unknown) {
      super(message);
      this.code = code;
      this.data = data;
    }
  }
  const updates: Json[] = [];
  let model = "openai/gpt-5";
  let invalidated = 0;
  const entry = {
    models: [] as Json[],
    controller: { invalidateAvailableModelsCache: () => void (invalidated += 1), listAvailableModels: async () => [{ id: "anthropic/claude-fable-5-2", hasApiKey: true }, { id: "openai/gpt-5", hasApiKey: false }, { id: "openai/gpt-image-2", hasApiKey: true }] },
    session: { model: { get: () => model, switch: async (next: string) => void (model = next) } },
  };
  class Base {
    connection = { sessionUpdate: async (update: Json) => void updates.push(update) };
    sessions = new Map([["s1", entry]]);
    async initialize() {
      return { protocolVersion: 1, authMethods: [{ id: "openai-codex" }] };
    }
    configOptions(of: Json) {
      return [{ id: "model", currentValue: model, options: of.models.map((item: Json) => ({ value: item.modelId, name: item.name })) }, { id: "mode", currentValue: "build", options: [] }];
    }
    async prompt(): Promise<Json> {
      throw new RequestError(-32603, "401 Unauthorized: token expired");
    }
    async newSession(): Promise<Json> {
      // As Mastra Code does: the model in use is listed though nothing connected serves it.
      entry.models = [{ modelId: "openai/gpt-5", name: "openai/gpt-5 (provider not configured)" }];
      return { sessionId: "s1", models: { currentModelId: model, availableModels: [] }, configOptions: [] };
    }
  }
  const { connections, told } = world({ signedIn: ["openai-codex"] });
  let reloaded = 0;
  const Agent = intcadAgent({ Base, RequestError, connections, dates: () => DATES, reloadSignIns: () => void (reloaded += 1) });
  const agent = new Agent();

  assert.deepEqual((await agent.initialize({})).authMethods, [], "the sign-ins are on the Connections page, not ACP sign-in methods");
  assert.deepEqual(Object.keys(METHODS), ["setup", "list", "login", "code", "reopen", "cancel", "logout"].map((name) => `_intcad/connections/${name}`));
  assert.equal((await agent.extMethod("_intcad/connections/list", {})).plans.length, 5);
  await assert.rejects(agent.extMethod("_intcad/connections/nothing", {}), { code: -32601 });
  await assert.rejects(agent.extMethod("toString", {}), { code: -32601 });
  await assert.rejects(agent.extMethod("_intcad/connections/login", { id: "nobody" }), { code: -32602 });

  // The plan is refused in the middle of a turn: the error says so, for which provider, and the plan shows as expired.
  await assert.rejects(agent.prompt({ sessionId: "s1" }), (error: Json) => {
    assert.deepEqual(error.data, { intcad: { kind: "auth", prefix: "openai" } });
    return error.code === -32603;
  });
  assert.equal(told.at(-1)!.plans[0].state, "expired");

  // Mastra opens a session on its default model though nothing connected serves it: the session starts on one that can answer.
  const opened = await agent.newSession({});
  assert.equal(opened.models.currentModelId, "anthropic/claude-fable-5-2");
  assert.equal(opened.configOptions[0].currentValue, "anthropic/claude-fable-5-2");
  model = "openai/gpt-5";

  // The model in use is gone after a change: the session moves to the newest one it may use and tells the app.
  await agent.refreshSessions();
  assert.equal(reloaded, 1);
  assert.equal(invalidated, 1, "Mastra is told that what can be used has changed");
  assert.equal(model, "anthropic/claude-fable-5-2");
  assert.deepEqual(updates.at(-1), {
    sessionId: "s1",
    update: { sessionUpdate: "config_option_update", configOptions: [{ id: "model", currentValue: "anthropic/claude-fable-5-2", options: [{ value: "anthropic/claude-fable-5-2", name: "anthropic/claude-fable-5-2" }] }, { id: "mode", currentValue: "build", options: [] }] },
  });
});
