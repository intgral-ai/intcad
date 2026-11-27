// The built-in agent's connections (`agent/*.mjs`), run against stand-ins for Mastra: no test signs
// in anywhere, opens a browser or reads a key from this computer. The modules are plain JavaScript
// outside `src/`, so they are loaded by address from the compiled test's own place.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import net from "node:net";
import { tmpdir } from "node:os";
import { test } from "node:test";
import { Agent as CoreAgent } from "@mastra/core/agent";

type Json = Record<string, any>;

function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

// Compiled, this file is in `dist/test/`; the modules stay where they are written.
const agentFolder = import.meta.url.includes("/dist/test/") ? "../../agent/" : "../agent/";
const load = (name: string): Promise<Json> => import(new URL(agentFolder + name, import.meta.url).href);
const { Connections, METHODS, customId } = await load("connections.mjs");
const callbackModule = await load("callback.mjs");
const { holdCallbackPorts } = callbackModule;
const { attachmentReader, isChatModel, orderModelOptions, releaseDates } = await load("models.mjs");
const { classifyFailure, classifySignInFailure } = await load("problems.mjs");
const { intcadAgent } = await load("agent.mjs");
const { PlanGate } = await load("plan.mjs");
const { keepKeysFromCommands, keyVariables, withoutParams } = await load("secrets.mjs");

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
  const held: { close: () => void; closed: boolean; callback: (url: string) => boolean }[] = [];
  let custom: Json[] | null = null;
  let refreshed = 0;
  const whys: (Json | undefined)[] = [];
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
      (async (callback: (url: string) => boolean) => {
        const ports = { closed: false, callback, close: () => void (ports.closed = true) };
        held.push(ports);
        return ports;
      }),
    dates: () => DATES,
    changed: (snapshot: Json) => void told.push(snapshot),
    refresh: async (why?: Json) => void ((refreshed += 1), whys.push(why)),
    ...(overrides.limitMs ? { limitMs: overrides.limitMs } : {}),
  });
  return { connections, signedIn, told, opened, written, env, held, custom: () => custom, refreshed: () => refreshed, whys };
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

test("a custom address that cannot be used is left out and the rest is taken; names that give one id each get their own", async () => {
  const { connections, env } = world();
  // The app checks an address before it keeps it, so a bad one comes from a file changed by hand.
  // It must not cost the user their keys, nor print them (a refused request is printed by ACP's SDK).
  const mixed = await connections.setup({
    keys: { openai: "sk-kept" },
    custom: [
      { name: "", url: "http://x/v1", models: ["m"] },
      { name: "\uFEFF", url: "http://x/v1", models: ["m"] },
      { name: "a", url: "ftp://x", models: ["m"] },
      { name: "a", url: "http://x/v1", models: [] },
      { name: "kept", url: "http://x/v1", models: ["m"] },
    ],
    words: { callback: "回到 Intcad" },
  });
  assert.deepEqual(mixed.custom.map((entry: Json) => entry.name), ["kept"]);
  assert.equal(env.OPENAI_API_KEY, "sk-kept");
  assert.equal(connections.callbackSentence(), "回到 Intcad");
  assert.equal((await connections.setup({ words: { callback: 5 } })).custom.length, 0);
  assert.equal(connections.callbackSentence(), "", "a sentence that is not one is not shown");
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

test("a browser sign-in holds the local ports, opens the browser, waits for it and ends when the browser calls back", async () => {
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
  // However long the browser takes within the time limit (15 minutes), the sign-in is still under way.
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.equal(connections.list().login.phase, "browser");
  assert.equal(held.length, 1);
  assert.equal(held[0]!.closed, false);
  connections.reopen();
  assert.equal(opened.length, 2);
  // A call that is not this sign-in's (another `state`) is refused and the wait goes on: handed to
  // Mastra it would end the sign-in as failed.
  assert.equal(held[0]!.callback("http://localhost:1457/auth/callback?code=x&state=other"), false);
  await settle();
  assert.equal(connections.list().login.phase, "browser");
  assert.equal(held[0]!.callback("http://localhost:1457/auth/callback?code=c1&state=s1"), true);
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
  assert.equal(connections.code({ id: "anthropic", code: "x".repeat(5000) }).login.error, "incomplete");
  // A code for a sign-in that is not the one waiting changes nothing, and is not refused: ACP's SDK
  // prints what it refuses, and this is a code.
  assert.equal(connections.code({ id: "xai", code: "abc#def" }).login.id, "anthropic");
  connections.code({ id: "anthropic", code: " abc#def " });
  await settle();
  assert.equal(told.at(-1)!.plans[1].state, "in");
  assert.equal(told.at(-1)!.login, null);
  // The button pressed a second time, after the code was taken.
  assert.equal(connections.code({ id: "anthropic", code: "abc#def" }).login, null);
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
  const { connections, told, refreshed, whys } = world({ signedIn: ["openai-codex", "anthropic"] });
  connections.expired("openai");
  assert.equal(told.at(-1)!.plans[0].state, "expired");
  connections.expired("deepseek");
  assert.equal(told.length, 1, "a provider no plan serves changes nothing");
  const snapshot = await connections.logout({ id: "openai-codex" });
  assert.equal(snapshot.plans[0].state, "out");
  assert.equal(snapshot.plans[1].state, "in");
  assert.equal(refreshed(), 1);
  assert.deepEqual(whys, [{ signedOut: true }], "only a sign-out lets a session go to another model by itself");
  await assert.rejects(connections.logout({ id: "nobody" }), { code: "INVALID" });
});

test("the two local ports are held until closed, only the awaited callback is taken, and a taken port is said to be busy", async () => {
  const got: string[] = [];
  const base = 41455;
  let sentence = "";
  const awaited = (url: string) => {
    got.push(url);
    return url.includes("state=s1");
  };
  const ports = await holdCallbackPorts(http, awaited, { ports: [base, base + 2], sentence: () => sentence });
  // Mastra asks the holder to step aside before it gives the port up; the holder stays.
  assert.equal((await fetch(`http://127.0.0.1:${base}/cancel`)).status, 200);
  assert.equal((await fetch(`http://127.0.0.1:${base + 2}/nothing`)).status, 404);
  assert.equal((await fetch(`http://127.0.0.1:${base + 2}/auth/callback?state=s1`)).status, 400, "a call without a code is not the sign-in");
  // Anything on this computer can call the port. An address that cannot be read once ended the
  // agent's process; it is answered and the listener stays.
  const raw = await new Promise<string>((resolve, reject) => {
    const request = http.request({ host: "127.0.0.1", port: base + 2, path: "//", method: "GET" }, (response) => resolve(String(response.statusCode)));
    request.on("error", reject);
    request.end();
  });
  assert.equal(raw, "400");
  assert.equal((await fetch(`http://127.0.0.1:${base + 2}/auth/callback?code=x&state=other`)).status, 400, "another sign-in's call is refused, and the wait goes on");
  const response = await fetch(`http://127.0.0.1:${base + 2}/auth/callback?code=c1&state=s1`);
  assert.equal(response.status, 200);
  // The sign-in is not finished when the browser calls: the page promises nothing.
  const page = await response.text();
  assert.match(page, /Go back to Intcad/);
  assert.doesNotMatch(page, /Signed in/);
  assert.deepEqual(got, [`http://localhost:${base + 2}/auth/callback?code=x&state=other`, `http://localhost:${base + 2}/auth/callback?code=c1&state=s1`]);
  // In the user's language when the app gave the sentence, and never as markup.
  sentence = "<b>回到 Intcad</b>";
  const translated = await (await fetch(`http://127.0.0.1:${base + 2}/auth/callback?code=c2&state=s1`)).text();
  assert.match(translated, /&#60;b&#62;回到 Intcad&#60;\/b&#62;/);
  await assert.rejects(holdCallbackPorts(http, () => false, { ports: [base + 4, base + 2] }), { code: "PORT_BUSY" });
  ports.close();
  // Free again at once; and the first port only has to be out of Mastra's reach, so another holder of it is fine.
  const again = await holdCallbackPorts(http, () => false, { ports: [base, base + 2] });
  const second = await holdCallbackPorts(http, () => false, { ports: [base, base + 6] });
  again.close();
  second.close();
});

test("a sign-in left unfinished stops at the time limit, gives the ports back and says so (INT-1522)", async () => {
  const { connections, held, told } = world({
    limitMs: 40,
    login: (_id: string, callbacks: Json) => {
      callbacks.onAuth({ url: "https://auth.example/authorize?state=s1" });
      return callbacks.onManualCodeInput();
    },
  });
  connections.login({ id: "openai-codex" });
  await settle();
  assert.equal(connections.list().login.phase, "browser");
  await new Promise((resolve) => setTimeout(resolve, 120));
  assert.equal(held[0]!.closed, true, "the ports are free once the time is up");
  assert.equal(told.at(-1)!.login, null);
  assert.deepEqual(told.at(-1)!.problem, { id: "openai-codex", kind: "timeout" });
  // A new sign-in starts as usual afterwards, and the end of the agent's process gives its ports back too.
  connections.login({ id: "openai-codex" });
  await settle();
  assert.equal(held.length, 2);
  connections.dispose();
  assert.equal(held[1]!.closed, true);
  await new Promise((resolve) => setTimeout(resolve, 80));
  assert.equal(connections.list().problem, null, "a sign-in ended with the process is not timed out afterwards");
});

/** Listens on `port` of 127.0.0.1 with a server that is not a sign-in's: another program. */
async function foreignListener(port: number): Promise<net.Server> {
  const server = net.createServer((socket) => socket.end());
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve());
  });
  return server;
}

test("a taken port says who holds it: a sign-in of this app, of another Intcad, or which program (INT-1522)", async () => {
  const base = 42455;
  // Another program: named where the system lets the agent find it (lsof and ps on macOS and Linux).
  const foreign = await foreignListener(base + 2);
  const byProgram = await holdCallbackPorts(http, () => false, { ports: [base, base + 2] }).then(() => null, (error: Json) => error);
  foreign.close();
  assert.equal(byProgram?.code, "PORT_BUSY");
  assert.equal(byProgram?.port, base + 2);
  if (process.platform === "darwin") assert.deepEqual(byProgram?.holder, { kind: "program", name: "node" });
  else assert.ok(byProgram?.holder === null || byProgram?.holder?.name === "node", JSON.stringify(byProgram?.holder));

  // A sign-in of this same app (another design's agent: same parent), which never ends.
  const ours = await holdCallbackPorts(http, () => false, { ports: [base + 4, base + 6] });
  const byUs = await holdCallbackPorts(http, () => false, { ports: [base + 8, base + 6], retryMs: 300 }).then(() => null, (error: Json) => error);
  assert.equal(byUs?.code, "PORT_BUSY");
  assert.deepEqual(byUs?.holder, { kind: "here" });
  ours.close();

  // A sign-in of another Intcad: its agent has another parent.
  const entry = new URL(agentFolder + "callback.mjs", import.meta.url).href;
  const child = spawn(
    process.execPath,
    ["--input-type=module", "-e", `const http = await import("node:http"); const { holdCallbackPorts } = await import(${JSON.stringify(entry)}); await holdCallbackPorts(http, () => false, { ports: [${base + 10}, ${base + 12}] }); console.log("held");`],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  try {
    await new Promise<void>((resolve) => child.stdout!.once("data", () => resolve()));
    const byOther = await holdCallbackPorts(http, () => false, { ports: [base + 14, base + 12] }).then(() => null, (error: Json) => error);
    assert.equal(byOther?.code, "PORT_BUSY");
    assert.deepEqual(byOther?.holder, { kind: "intcad" });
  } finally {
    child.kill();
  }
});

test("the holder of a port is read from what lsof, netstat, ps and tasklist print, and named by its app (INT-1522)", () => {
  const { pidIn, programName } = callbackModule;
  assert.equal(pidIn("p15185\ncnode\nf33\n", 1457, "darwin"), 15185);
  assert.equal(pidIn("", 1457, "linux"), null);
  const netstat = "\r\nActive Connections\r\n\r\n  Proto  Local Address          Foreign Address        State           PID\r\n  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1000\r\n  TCP    127.0.0.1:1457         0.0.0.0:0              LISTENING       4242\r\n";
  assert.equal(pidIn(netstat, 1457, "win32"), 4242);
  assert.equal(pidIn(netstat, 1455, "win32"), null);
  assert.equal(programName("/Applications/Intcad.app/Contents/Resources/mastra-agent/node\n"), "Intcad");
  assert.equal(programName("/Applications/ChatGPT.app/Contents/Frameworks/ChatGPT Helper.app/Contents/MacOS/ChatGPT Helper"), "ChatGPT");
  assert.equal(programName("/usr/local/bin/codex"), "codex");
  assert.equal(programName("C:\\Program Files\\nodejs\\node.exe"), "node.exe");
  assert.equal(programName("codex\u0007\u202e"), "codex");
  assert.equal(programName(""), null);
});

test("a sign-in of another design of this app that is being cancelled is waited for briefly (INT-1522)", async () => {
  const base = 43455;
  const leaving = await holdCallbackPorts(http, () => false, { ports: [base, base + 2] });
  // The app cancels the other design's sign-in before it starts this one; that agent lets go a moment later.
  setTimeout(() => leaving.close(), 150);
  const taken = await holdCallbackPorts(http, () => false, { ports: [base + 4, base + 2], retryMs: 2000 });
  taken.close();
});

test("a port the system will not open is told apart from a taken one; the first port may be refused (INT-1522)", async () => {
  const base = 44455;
  // As `node:http`, with the system refusing to open one port (EACCES: a reserved range, security software).
  const refusing = (refused: number) => ({
    createServer: (handler: http.RequestListener) => {
      const server = http.createServer(handler);
      const listen = server.listen.bind(server) as (port: number, host: string) => http.Server;
      (server as Json).listen = (port: number, host: string) => {
        if (port !== refused) return listen(port, host);
        setImmediate(() => server.emit("error", Object.assign(new Error(`listen EACCES: permission denied 127.0.0.1:${port}`), { code: "EACCES" })));
        return server;
      };
      return server;
    },
  });
  // The first port only has to be out of Mastra's reach, and what keeps this process off it keeps Mastra off it too.
  const fine = await holdCallbackPorts(refusing(base), () => false, { ports: [base, base + 2] });
  fine.close();
  const refused = await holdCallbackPorts(refusing(base + 6), () => false, { ports: [base + 4, base + 6] }).then(() => null, (error: Json) => error);
  assert.equal(refused?.code, "LISTEN_FAILED");
  assert.equal(refused?.port, base + 6);
  assert.equal(refused?.errno, "EACCES");
  assert.equal(classifySignInFailure(refused), "listen");
});

test("a sign-in's problem carries who holds the port, or the port and the system's code (INT-1522)", async () => {
  const held = world({ holdPorts: async () => Promise.reject(Object.assign(new Error("port 1457 is in use"), { code: "PORT_BUSY", port: 1457, holder: { kind: "program", name: "Codex" } })) });
  held.connections.login({ id: "openai-codex" });
  await settle();
  assert.deepEqual(held.connections.list().problem, { id: "openai-codex", kind: "port", holder: { kind: "program", name: "Codex" } });

  const refused = world({ holdPorts: async () => Promise.reject(Object.assign(new Error("port 1457 could not be opened"), { code: "LISTEN_FAILED", port: 1457, errno: "EACCES" })) });
  refused.connections.login({ id: "openai-codex" });
  await settle();
  assert.deepEqual(refused.connections.list().problem, { id: "openai-codex", kind: "listen", port: 1457, code: "EACCES" });
});

test("a key is not in the environment of a command a model runs, nor in a refusal that is printed", () => {
  const names = keyVariables({ providers: { openai: { apiKeyEnvVar: "OPENAI_API_KEY" }, google: { apiKeyEnvVar: ["GOOGLE_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY"] }, none: {} } });
  assert.deepEqual([...names].sort(), ["GOOGLE_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY", "MASTRA_GATEWAY_API_KEY", "OPENAI_API_KEY"]);
  // As Mastra's local sandbox: the environment of every process it starts begins as a copy of the agent's.
  class Sandbox {
    env: Record<string, string> = { HOME: "/h", OPENAI_API_KEY: "sk-secret", MASTRA_GATEWAY_API_KEY: "gw-secret" };
    buildEnv(extra: Record<string, string> = {}) {
      return { PATH: "/usr/bin", ...this.env, ...extra };
    }
  }
  keepKeysFromCommands(Sandbox, names);
  const sandbox = new Sandbox();
  assert.deepEqual(sandbox.buildEnv({ EXTRA: "1", GOOGLE_API_KEY: "g-secret" }), { PATH: "/usr/bin", HOME: "/h", EXTRA: "1" });
  assert.equal(sandbox.env.OPENAI_API_KEY, "sk-secret", "what Mastra itself reads is untouched");
  // Another SDK, where the guard has nothing to hold on to: the agent does not start unguarded.
  assert.throws(() => keepKeysFromCommands(class {}, names), /would be given the keys/);

  // ACP's SDK prints `console.error("Error handling request", message, error)` for a request it refuses.
  const printed = withoutParams(["Error handling request", { jsonrpc: "2.0", id: 3, method: "_intcad/connections/setup", params: { keys: { openai: "sk-secret" } } }, { code: -32602, message: "bad" }]);
  assert.ok(!JSON.stringify(printed).includes("sk-secret"));
  assert.deepEqual(printed, ["Error handling request", { jsonrpc: "2.0", id: 3, method: "_intcad/connections/setup", params: "[not shown]" }, { code: -32602, message: "bad" }]);
  const failure = new Error("x");
  assert.deepEqual(withoutParams([failure, "text", null, 5]), [failure, "text", null, 5]);
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
  // A custom address's models were typed by the user. Neither their name nor the address's ("Olive"
  // has "live" in it, "audio lab" has "audio") is read for what the model can do.
  const typed = ["mastracode/olive/llama3.2", "mastracode/audio-lab/qwen2-audio", "openai/gpt-image-2"].map((value) => ({ value, name: value }));
  assert.deepEqual(orderModelOptions(typed, DATES, "", (id: string) => id.startsWith("mastracode/")).map((option: Json) => option.value), ["mastracode/audio-lab/qwen2-audio", "mastracode/olive/llama3.2"]);
});

test("a failed turn is put into one of the kinds the Assistant has words for", () => {
  const cases: [string, string][] = [
    ["Incorrect API key provided: sk-****. You can find your API key at …", "auth"],
    ["401 Unauthorized", "auth"],
    ["Could not find API key process.env.OPENAI_API_KEY for model id openai/gpt-5.6-sol", "auth"],
    ["OAuth token has expired. Please sign in again.", "auth"],
    ["You exceeded your current quota, please check your plan and billing details.", "quota"],
    ["429 insufficient_quota", "quota"],
    // DeepSeek's words for a key with no money left on it.
    ["Insufficient Balance", "quota"],
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
  let provider = async (_input: Json): Promise<void> => { throw new RequestError(-32603, "401 Unauthorized: token expired"); };
  const entry = {
    models: [] as Json[],
    controller: {
      invalidateAvailableModelsCache: () => void (invalidated += 1),
      listAvailableModels: async () => [
        { id: "anthropic/claude-fable-5-2", hasApiKey: true },
        { id: "anthropic/claude-newer-than-the-registry", hasApiKey: true, releaseDate: "2026-12-01" },
        { id: "openai/gpt-5", hasApiKey: false },
        { id: "openai/gpt-image-2", hasApiKey: true },
      ],
    },
    session: {
      model: { get: () => model, switch: async (next: string) => void (model = next) },
      sendMessage: (input: Json) => provider(input),
    },
  };
  class Base {
    connection = { sessionUpdate: async (update: Json) => void updates.push(update) };
    sessions = new Map([["s1", entry]]);
    async initialize() {
      return { protocolVersion: 1, authMethods: [{ id: "openai-codex" }] };
    }
    configOptions(of: Json) {
      // As Mastra Code does: the model in use is in the list whether or not anything serves it.
      const listed = of.models.map((item: Json) => ({ value: item.modelId, name: item.name }));
      if (!listed.some((item: Json) => item.value === model)) listed.push({ value: model, name: `${model} (provider not configured)` });
      return [{ id: "model", currentValue: model, options: listed }, { id: "mode", currentValue: "build", options: [] }];
    }
    async prompt(): Promise<Json> {
      await entry.session.sendMessage({ content: "" });
      return { stopReason: "end_turn" };
    }
    async newSession(): Promise<Json> {
      // As Mastra Code does: the model in use is listed though nothing connected serves it.
      entry.models = [{ modelId: "openai/gpt-5", name: "openai/gpt-5 (provider not configured)" }];
      return { sessionId: "s1", models: { currentModelId: model, availableModels: [] }, configOptions: [] };
    }
  }
  const { connections, told } = world({ signedIn: ["openai-codex"] });
  let reloaded = 0;
  const dates = new Map(DATES);
  const Agent = intcadAgent({ Base, RequestError, connections, dates: () => dates, reloadSignIns: () => void (reloaded += 1) });
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
  // A model that came out after the registry was written has its date from Mastra's catalog, so it lists first.
  const opened = await agent.newSession({});
  assert.equal(opened.models.currentModelId, "anthropic/claude-newer-than-the-registry");
  assert.deepEqual(opened.configOptions[0].options.map((option: Json) => option.value), ["anthropic/claude-newer-than-the-registry", "anthropic/claude-fable-5-2"]);
  assert.equal(dates.get("anthropic/claude-newer-than-the-registry"), "2026-12-01");
  model = "openai/gpt-5";

  // The model in use is gone after new keys or a finished sign-in: the session keeps it, gets the
  // list of what can be used (without the model nothing serves, which Mastra Code would list), and
  // tells the app. Nobody chose another model, so none is taken.
  await agent.refreshSessions();
  assert.equal(reloaded, 1);
  assert.equal(invalidated, 1, "Mastra is told that what can be used has changed");
  assert.equal(model, "openai/gpt-5");
  const newest = { value: "anthropic/claude-newer-than-the-registry", name: "anthropic/claude-newer-than-the-registry" };
  const listed = [newest, { value: "anthropic/claude-fable-5-2", name: "anthropic/claude-fable-5-2" }];
  assert.deepEqual(updates.at(-1), {
    sessionId: "s1",
    update: { sessionUpdate: "config_option_update", configOptions: [{ id: "model", currentValue: "openai/gpt-5", options: listed }, { id: "mode", currentValue: "build", options: [] }] },
  });

  // A turn asked for on that model reaches no provider: the Assistant is told to have another model picked.
  let asked = 0;
  provider = async (_input: Json) => {
    asked += 1;
  };
  await assert.rejects(agent.prompt({ sessionId: "s1" }), (error: Json) => {
    assert.deepEqual(error.data, { intcad: { kind: "model", prefix: "openai" } });
    return error.code === -32603;
  });
  assert.equal(asked, 0);
  // A custom address that is gone has no name left: no prefix is told, not Mastra's word for its gateway.
  model = "mastracode/gone/llama";
  await assert.rejects(agent.prompt({ sessionId: "s1" }), (error: Json) => {
    assert.deepEqual(error.data, { intcad: { kind: "model", prefix: "" } });
    return true;
  });
  assert.equal(asked, 0);
  model = "openai/gpt-5";

  // After a sign-out the session does move, to the newest model it may use (the one case the PRD names).
  await agent.refreshSessions({ signedOut: true });
  assert.equal(model, newest.value);
  assert.deepEqual(updates.at(-1), {
    sessionId: "s1",
    update: { sessionUpdate: "config_option_update", configOptions: [{ id: "model", currentValue: newest.value, options: listed }, { id: "mode", currentValue: "build", options: [] }] },
  });
});

test("a session's model list older than its keys is worked out again before a turn on a model it does not list is refused", async () => {
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
  let keyThere = true;
  let unreadable = false;
  let invalidated = 0;
  let asked = 0;
  const entry = {
    // Worked out before the DeepSeek key was saved; the refresh after it never reached this session.
    usable: ["openai/gpt-5"],
    models: [{ modelId: "openai/gpt-5", name: "openai/gpt-5" }],
    controller: {
      invalidateAvailableModelsCache: () => void (invalidated += 1),
      listAvailableModels: async () => {
        if (unreadable) throw new Error("the catalog could not be read");
        return [
          { id: "openai/gpt-5", hasApiKey: true },
          { id: "deepseek/deepseek-chat", hasApiKey: keyThere },
        ];
      },
    },
    session: {
      model: { get: () => "deepseek/deepseek-chat", switch: async () => assert.fail("nobody chose another model") },
      sendMessage: async (_input: Json) => void (asked += 1),
    },
  };
  class Base {
    connection = { sessionUpdate: async (update: Json) => void updates.push(update) };
    sessions = new Map([["s1", entry]]);
    async newSession() {
      return { sessionId: "s1" };
    }
    configOptions(of: Json) {
      return [{ id: "model", currentValue: of.session.model.get(), options: of.models.map((item: Json) => ({ value: item.modelId, name: item.name })) }];
    }
    async prompt(): Promise<Json> {
      await entry.session.sendMessage({ content: "" });
      return { stopReason: "end_turn" };
    }
  }
  const { connections } = world();
  const Agent = intcadAgent({ Base, RequestError, connections, dates: () => new Map(DATES), reloadSignIns: () => {} });
  const agent = new Agent();
  await agent.newSession({});
  keyThere = false;
  entry.usable = ["openai/gpt-5"];
  const refusedForTheModel = (error: Json) => {
    assert.deepEqual(error.data, { intcad: { kind: "model", prefix: "deepseek" } });
    return error.code === -32603;
  };

  // No key: refused as before, after one more look, and no provider is asked.
  await assert.rejects(agent.prompt({ sessionId: "s1" }), refusedForTheModel);
  assert.deepEqual({ asked, invalidated, told: updates.length }, { asked: 0, invalidated: 1, told: 0 });
  // The key is there but the list cannot be worked out: refused as before, not as some other failure.
  keyThere = true;
  unreadable = true;
  await assert.rejects(agent.prompt({ sessionId: "s1" }), refusedForTheModel);
  assert.deepEqual({ asked, told: updates.length }, { asked: 0, told: 0 });
  // The list can be worked out: the turn goes ahead on the model in use, and the app is told the new list.
  unreadable = false;
  assert.deepEqual(await agent.prompt({ sessionId: "s1" }), { stopReason: "end_turn" });
  assert.equal(asked, 1);
  assert.equal(updates.length, 1);
  assert.equal(updates[0]!.update.sessionUpdate, "config_option_update");
  const model = updates[0]!.update.configOptions[0];
  assert.equal(model.currentValue, "deepseek/deepseek-chat");
  assert.deepEqual(model.options.map((option: Json) => option.value), ["openai/gpt-5", "deepseek/deepseek-chat"]);
  // The next turn needs no second look.
  await agent.prompt({ sessionId: "s1" });
  assert.deepEqual({ asked, invalidated, told: updates.length }, { asked: 2, invalidated: 3, told: 1 });
});

async function attachmentAgent(
  deliver?: (input: Json, admit: () => void) => Promise<void>,
  isNotAdmitted?: (error: unknown) => boolean,
) {
  class RequestError extends Error {
    code: number;
    data: unknown;
    constructor(code: number, message: string, data?: unknown) {
      super(message);
      this.code = code;
      this.data = data;
    }
  }
  const sent: Json[] = [];
  const vision = "anthropic/claude-sonnet-4-5";
  const textOnly = "deepseek/deepseek-v4-pro";
  let model = vision;
  const list = async () => [vision, textOnly].map((id) => ({ id, hasApiKey: true }));
  const entry: Json = {
    controller: { listAvailableModels: list },
    session: {
      model: { get: () => model, switch: async () => {} },
      sendMessage: async (input: Json) => {
        const admit = () => { sent.push(input); };
        if (deliver) await deliver(input, admit);
        else admit();
      },
    },
    queue: Promise.resolve(),
    turns: new Set<Json>(),
  };
  // As Mastra Code's ACP agent: words and text documents only, a picture refused; turns queue, and
  // one cancelled before its place in the queue sends nothing.
  class Base {
    connection = { sessionUpdate: async () => {} };
    sessions = new Map<string, Json>();
    async initialize() {
      return { protocolVersion: 1, agentCapabilities: { loadSession: false, mcpCapabilities: { http: true, sse: false } }, authMethods: [{ id: "mastracode-login" }] };
    }
    async newSession() {
      this.sessions.set("s1", entry);
      return { sessionId: "s1" };
    }
    configOptions() {
      return [];
    }
    async prompt(request: Json): Promise<Json> {
      const content = request.prompt
        .map((block: Json) => {
          if (block.type === "text") return block.text;
          if (block.type === "resource" && "text" in block.resource) return block.resource.text;
          throw new RequestError(-32602, `Unsupported prompt content: ${block.type}`);
        })
        .join("\n");
      const turn = { cancelled: false };
      entry.turns.add(turn);
      const result = entry.queue.then(async () => {
        await new Promise((done) => setImmediate(done));
        if (turn.cancelled) return { stopReason: "cancelled" };
        await entry.session.sendMessage({ content });
        return { stopReason: "end_turn" };
      });
      entry.queue = result.then(() => {}, () => {});
      return result;
    }
    async cancel() {
      for (const turn of entry.turns) turn.cancelled = true;
    }
  }
  const { connections } = world();
  const takesAttachments = (id: string) => (id === textOnly ? false : undefined);
  const Agent = intcadAgent({ Base, RequestError, connections, dates: () => new Map(DATES), reloadSignIns: () => {}, takesAttachments, isNotAdmitted });
  const agent = new Agent();
  await agent.newSession({});
  return { agent, entry, sent, list, vision, textOnly, select: (id: string) => { model = id; } };
}

const picture = (data: string) => ({ type: "image", mimeType: "image/png", data });
const pdf = { type: "resource", resource: { uri: "file:///work/brief%20one.pdf", mimeType: "application/pdf", blob: "JVBERi0=" } };
const attachmentRefusal = (error: Json) => {
  assert.deepEqual(error.data, { intcad: { kind: "attachments", prefix: "deepseek" } });
  return error.code === -32603;
};

test("a message's pictures and PDFs go with its words, without leaking files into later messages (INT-1513)", async () => {
  const { agent, sent } = await attachmentAgent();

  // The app sends pictures and PDFs only to an agent that says it takes them.
  const init = await agent.initialize({});
  assert.deepEqual(init.agentCapabilities, { loadSession: false, mcpCapabilities: { http: true, sse: false }, promptCapabilities: { image: true, embeddedContext: true }, sessionCapabilities: { close: {} } });
  assert.deepEqual(init.authMethods, []);

  const note = { type: "resource", resource: { uri: "file:///work/note.txt", mimeType: "text/plain", text: "a note" } };
  // The picture and the PDF go as files with the words; a text document stays with the words.
  assert.deepEqual(await agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "what is this?" }, picture("AAAA"), pdf, note] }), { stopReason: "end_turn" });
  assert.deepEqual(sent.shift(), {
    content: "what is this?\na note",
    files: [
      { data: "AAAA", mediaType: "image/png" },
      { data: "JVBERi0=", mediaType: "application/pdf", filename: "brief one.pdf" },
    ],
  });
  // Two turns queued at once: each message carries its own picture.
  await Promise.all([agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "one" }, picture("ONE")] }), agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "two" }, picture("TWO")] })]);
  assert.deepEqual(sent.splice(0).map((input) => [input.content, (input.files ?? []).map((file: Json) => file.data)]), [["one", ["ONE"]], ["two", ["TWO"]]]);
  // A turn cancelled before it sends: its picture does not go with the next message.
  const cancelled = agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "never sent" }, picture("GONE")] });
  await agent.cancel({ sessionId: "s1" });
  assert.deepEqual(await cancelled, { stopReason: "cancelled" });
  await agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "words only" }] });
  assert.deepEqual(sent.splice(0), [{ content: "words only" }]);

});

test("refused files do not enter history or prevent a later text-only turn", async () => {
  const { agent, sent, select, textOnly } = await attachmentAgent();
  select(textOnly);
  await assert.rejects(agent.prompt({ sessionId: "s1", prompt: [picture("ONE"), picture("TWO"), pdf] }), attachmentRefusal);
  assert.equal(sent.length, 0);
  assert.deepEqual(await agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "words only" }] }), { stopReason: "end_turn" });
  assert.deepEqual(sent.splice(0), [{ content: "words only" }]);
});

test("a queued model switch is honored when checking the prompt's attachments", async () => {
  const { agent, entry, sent, select, textOnly, vision } = await attachmentAgent();
  entry.queue = entry.queue.then(() => select(textOnly));
  await assert.rejects(agent.prompt({ sessionId: "s1", prompt: [picture("NOW")] }), attachmentRefusal);
  assert.equal(sent.length, 0);
  entry.queue = entry.queue.then(() => select(vision));
  assert.deepEqual(await agent.prompt({ sessionId: "s1", prompt: [picture("NOW")] }), { stopReason: "end_turn" });
  assert.equal(sent.length, 1);
});

test("a text-only follow-up cannot resend retained pictures to an incompatible model", async () => {
  const { agent, sent, select, textOnly, vision } = await attachmentAgent();
  await agent.prompt({ sessionId: "s1", prompt: [picture("OLD")] });
  select(textOnly);
  const followUp = { sessionId: "s1", prompt: [{ type: "text", text: "and now?" }] };
  await assert.rejects(agent.prompt(followUp), attachmentRefusal);
  assert.equal(sent.length, 1, "no call that would include the earlier picture");
  select(vision);
  assert.deepEqual(await agent.prompt(followUp), { stopReason: "end_turn" });
  assert.deepEqual(sent.at(-1), { content: "and now?" });
});

test("Stop acknowledges a prompt held in model refresh without dispatching it afterwards", async () => {
  const { agent, entry, sent, list } = await attachmentAgent();
  entry.usable = ["openai/other"];
  const refreshStarted = deferred();
  const refreshed = deferred();
  entry.controller.listAvailableModels = async () => {
    refreshStarted.resolve();
    await refreshed.promise;
    return list();
  };
  const pending = agent.prompt({ sessionId: "s1", prompt: [picture("STOP")] });
  await refreshStarted.promise;
  await agent.cancel({ sessionId: "s1" });
  const result = await Promise.race([pending, new Promise((done) => setImmediate(() => done("still waiting")))]);
  refreshed.resolve();
  await pending;
  assert.deepEqual(result, { stopReason: "cancelled" });
  assert.equal(sent.length, 0);
  await agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "after Stop" }] });
  assert.deepEqual(sent, [{ content: "after Stop" }]);
});

test("session/close promptly cancels a queued attachment and never admits it", async () => {
  const { agent, entry, sent } = await attachmentAgent();
  const head = deferred();
  entry.queue = head.promise;
  const running = agent.prompt({ sessionId: "s1", prompt: [picture("QUEUED")] });
  await agent.closeSession({ sessionId: "s1" });
  try {
    assert.deepEqual(await Promise.race([running, settle().then(() => "still pending")]), { stopReason: "cancelled" });
  } finally {
    head.resolve();
    await running.catch(() => {});
  }
  assert.deepEqual(sent, []);
});

test("Stop acknowledges a queued prompt before a blocked queue head clears", async () => {
  const { agent, entry, sent } = await attachmentAgent();
  const queueHead = deferred();
  entry.queue = queueHead.promise;
  const cancelled = agent.prompt({ sessionId: "s1", prompt: [picture("ABANDONED")] });
  await agent.cancel({ sessionId: "s1" });
  const outcome = await Promise.race([cancelled, new Promise((done) => setImmediate(() => done("still waiting")))]);
  const followUp = agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "later" }, picture("LATER")] });
  queueHead.resolve();
  await cancelled;
  assert.deepEqual(await followUp, { stopReason: "end_turn" });
  assert.deepEqual(outcome, { stopReason: "cancelled" });
  assert.deepEqual(sent, [{ content: "later", files: [{ data: "LATER", mediaType: "image/png" }] }]);
});

test("only a confirmed SDK startup cancellation removes the picture from possible history", async () => {
  // The pinned local SDK's isSessionStartupCancelledError distinguishes this code from
  // post-admission aborts in Session.sendMessage. A generic AbortError proves nothing.
  for (const admitted of [false, true]) {
    const started = deferred();
    const release = deferred();
    const { agent, select, textOnly, sent } = await attachmentAgent(async (input, admit) => {
      if (!input.files) return admit();
      if (admitted) admit();
      started.resolve();
      await release.promise;
      throw Object.assign(new Error("stopped"), {
        name: "AbortError",
        ...(admitted ? {} : { code: "SESSION_STARTUP_CANCELLED" }),
      });
    }, (error) => error instanceof Error && "code" in error && error.code === "SESSION_STARTUP_CANCELLED");
    const stopped = agent.prompt({ sessionId: "s1", prompt: [picture("STOPPED")] });
    await started.promise;
    await agent.cancel({ sessionId: "s1" });
    release.resolve();
    assert.deepEqual(await stopped, { stopReason: "cancelled" });
    select(textOnly);
    const followUp = { sessionId: "s1", prompt: [{ type: "text", text: "after Stop" }] };
    if (admitted) {
      await assert.rejects(agent.prompt(followUp), attachmentRefusal);
      assert.equal(sent.length, 1);
    } else {
      assert.deepEqual(await agent.prompt(followUp), { stopReason: "end_turn" });
      assert.deepEqual(sent, [{ content: "after Stop" }]);
    }
  }
});

test("attachment capability accounts for the MIME type and chosen adapter (INT-1513)", () => {
  const registry = { models: { deepseek: ["deepseek-v4-pro", "deepseek-v4-flash"], anthropic: ["claude-sonnet-4-5"] } };
  // As Mastra's `modelSupportsAttachments`: no also for a model it does not know.
  const supports = (id: string) => ["deepseek/deepseek-v4-flash", "anthropic/claude-sonnet-4-5"].includes(id);
  let xaiOAuth = true;
  const reads = attachmentReader(registry, supports, (provider: string) => provider === "xai" && xaiOAuth);
  const [picture, pdf] = [[{ mediaType: "image/png" }], [{ mediaType: "application/pdf" }]];
  assert.equal(reads("deepseek/deepseek-v4-pro", picture), false);
  assert.equal(reads("deepseek/deepseek-v4-flash", picture), true);
  assert.equal(reads("deepseek/deepseek-v4-flash", [...picture, ...pdf]), false, "DeepSeek's adapter would leave the PDF out");
  assert.equal(reads("anthropic/claude-sonnet-4-5", pdf), true);
  assert.equal(reads("anthropic/claude-sonnet-9", picture), undefined, "newer than the catalog: sent, and the provider answers");
  assert.equal(reads("mastracode/stand-in/hello-1", pdf), undefined, "a custom address");
  assert.equal(reads("xai/grok-vision", pdf), false, "the xAI OAuth chat adapter cannot encode a PDF");
  assert.equal(reads("xai/grok-vision", picture), undefined, "pictures are not refused by that adapter");
  xaiOAuth = false;
  assert.equal(reads("xai/grok-vision", pdf), undefined, "the OAuth restriction does not get applied to the key route");
  assert.equal(reads("mastracode/stand-in/hello-1", [{ mediaType: "application/zip" }]), false, "the desktop only sends pictures and PDFs");
});

test("Copilot requires advertised vision for SDK-listed models on its signed-in route", async () => {
  const registry = { models: { "github-copilot": ["former"] } };
  let signedIn = true;
  let catalogFailed = false;
  let loads = 0;
  const reads = attachmentReader(registry, () => false, (provider: string) => provider === "github-copilot" && signedIn, async () => {
    loads++;
    if (catalogFailed) throw new Error("catalog unavailable");
    // The pinned SDK returns a boolean on every row: omitted upstream vision also becomes false.
    return [{ id: "text", supportsVision: false }, { id: "vision", supportsVision: true }, { id: "omitted-upstream", supportsVision: false }];
  });
  const png = [{ mediaType: "image/png" }];
  assert.equal(await reads("github-copilot/text", png), false);
  assert.equal(await reads("github-copilot/vision", png), true);
  assert.equal(await reads("github-copilot/omitted-upstream", png), false);
  assert.equal(await reads("github-copilot/former", png), undefined);
  assert.equal(await reads("mastracode/github-copilot/text", png), undefined);
  catalogFailed = true;
  assert.equal(await reads("github-copilot/text", png), undefined);
  signedIn = false;
  assert.equal(await reads("github-copilot/text", png), undefined);
  assert.equal(loads, 5);
});

// ---- The session builder (session.mjs): Mastra Code's ACP session, without a vector store ---------

const { LANGUAGE, NO_PROJECT, predictionToolGuard, sessionBuilder } = await load("session.mjs");
const { permissionRules } = await load("reads.mjs");

/**
 * A stand-in for what the entry loads from Mastra Code, recording what the builder does with it;
 * with `cad`, the design tools' bridge (cad.mjs) too.
 */
function fakeSdk(overrides: { failed?: Json[]; cad?: Json } = {}) {
  const calls: Json = { configs: [] as Json[], cleanup: [] as string[] };
  const step = (name: string) => () => {
    calls.cleanup.push(name);
  };
  class RequestError extends Error {
    static invalidParams(_data: unknown, message: string) {
      return new RequestError(`invalid params: ${message}`);
    }
    static internalError(_data: unknown, message: string) {
      return new RequestError(`internal error: ${message}`);
    }
  }
  const result = {
    controller: { listModes: () => ["build", "plan"], resolveWorkspace: async () => ({ skills: ["a skill"] }), getMastra: () => ({ stopWorkers: step("workers") }), stopIntervals: step("intervals") },
    session: {
      abort: step("abort"),
      thread: { detachFromCurrent: step("detach"), clearAndReleaseLock: step("thread lock") },
      state: { get: () => ({ thinkingLevel: undefined }) },
      mode: { get: () => "build" },
    },
    releaseThreadClaims: step("claims"),
    stopPluginSignalProviders: step("plugins"),
    githubSignals: { stopAllPolling: step("github") },
    threadScheduler: { stop: step("scheduler") },
    stopNotificationDispatch: step("dispatch"),
    mcpManager: { initInBackground: async () => ({ failed: overrides.failed ?? [] }), disconnect: step("mcp") },
    signalsPubSub: { close: step("pubsub") },
    storage: { close: step("storage") },
  };
  const sdk = {
    RequestError,
    verifyPredictionTools: predictionToolGuard(CoreAgent),
    createMastraCode: async (config: Json) => {
      calls.configs.push(config);
      return result;
    },
    loadSettings: () => ({ preferences: { subagentsEnabled: true } }),
    resolveDefaultThinkingLevel: () => ({ level: "medium" }),
    resolveSubagentsSetting: (enabled: boolean) => (enabled ? "the subagents" : undefined),
    withCleanupFailure: (error: unknown) => error,
    ...(overrides.cad ? { cad: overrides.cad } : {}),
  };
  return { sdk, calls };
}

test("a session is made as Mastra Code's ACP server makes it, with no vector store", async () => {
  const { sdk, calls } = fakeSdk();
  const runtime = await sessionBuilder(sdk)({
    cwd: "/project",
    mcpServers: [
      { name: "app", command: "intcad-cli", args: ["mcp"], env: [{ name: "A", value: "1" }] },
      { name: "remote", type: "http", url: "http://127.0.0.1:1/mcp", headers: [{ name: "x", value: "y" }] },
    ],
  });
  const config = calls.configs[0];
  // The one difference from Mastra Code's own builder: no vector store, so nothing is ever embedded.
  assert.equal(config.vector, false);
  assert.equal(config.cwd, "/project");
  // Intcad's (INT-1473): the rules of a session without a project (agent/reads.mjs), no
  // `request_access`, no hooks.
  assert.deepEqual(config.initialState, { projectPath: "/project", yolo: false, permissionRules: permissionRules(null) });
  assert.deepEqual(config.disabledTools, ["ask_user", "request_access"]);
  assert.deepEqual([config.disableEnvFile, config.disableMcp, config.disableHooks, config.unixSocketPubSub, config.planApprovalStartsGoal], [true, false, true, false, false]);
  assert.equal(config.subagents, "the subagents");
  assert.deepEqual(config.mcpServers, {
    app: { command: "intcad-cli", args: ["mcp"], env: { A: "1" }, cwd: "/project" },
    remote: { url: "http://127.0.0.1:1/mcp", headers: { x: "y" } },
  });
  assert.deepEqual(runtime.modes, ["build", "plan"]);
  assert.deepEqual(await runtime.getSkills(), ["a skill"]);
  assert.equal(runtime.getThinkingLevel(), "medium");
  // Ending it is done once however often it is asked, and closes the storage last.
  await Promise.all([runtime.cleanup(), runtime.cleanup()]);
  assert.equal(calls.cleanup.filter((name: string) => name === "storage").length, 1);
  assert.equal(calls.cleanup.at(-1), "storage");
  for (const name of ["claims", "abort", "detach", "plugins", "github", "scheduler", "dispatch", "thread lock", "mcp", "workers", "intervals", "pubsub"]) {
    assert.ok(calls.cleanup.includes(name), name);
  }
});

test("a session refuses a legacy SSE server and a repeated server name, and makes nothing", async () => {
  const { sdk, calls } = fakeSdk();
  const build = sessionBuilder(sdk);
  await assert.rejects(build({ cwd: "/p", mcpServers: [{ name: "old", type: "sse", url: "http://127.0.0.1:1/sse", headers: [] }] }), /Legacy SSE/u);
  const twice = { name: "app", command: "x", args: [], env: [] };
  await assert.rejects(build({ cwd: "/p", mcpServers: [twice, twice] }), /Duplicate MCP server name: app/u);
  assert.equal(calls.configs.length, 0);
});

test("a session whose MCP server does not connect is ended and says which one", async () => {
  const { sdk, calls } = fakeSdk({ failed: [{ name: "app", error: "no such program" }, { name: "the user's own", error: "ignored" }] });
  await assert.rejects(sessionBuilder(sdk)({ cwd: "/p", mcpServers: [{ name: "app", command: "x", args: [], env: [] }] }), /MCP server app: no such program$/u);
  assert.ok(calls.cleanup.includes("storage"), "the session was cleaned up");
});

// A session the app opens to guess the user's next step (docs/design/autocomplete-tests.md, G88).
test("G88 a guess of the next step is made with no MCP and no design tools, whatever the request names", async () => {
  const tools = { "intcad-app_inspect_design": { id: "intcad-app_inspect_design" } };
  const asked: Json[] = [];
  const cad = { tools: (request: Json) => (asked.push(request), tools) };
  const { sdk, calls } = fakeSdk({ cad });
  const build = sessionBuilder(sdk);
  const note = "The guessing note.";
  const servers = [{ name: "app", command: "intcad-cli", args: ["mcp"], env: [{ name: "A", value: "1" }] }];
  const meta = (intcad: Json) => ({ systemPrompt: { append: note }, intcad: { cadBridge: { version: 1, tools: [] }, ...intcad } });
  await build({ cwd: "/p", mcpServers: servers, _meta: meta({}) });
  // A project sent with a guess is not taken either (G87).
  await build({ cwd: "/p", mcpServers: servers, _meta: meta({ nextStep: true, project: tmpdir() }) });
  const [chat, guess] = calls.configs;
  // Without the flag, as before: the server named, MCP on, the design tools, the note with its lines.
  assert.deepEqual(chat.mcpServers, { app: { command: "intcad-cli", args: ["mcp"], env: { A: "1" }, cwd: "/p" } });
  assert.equal(chat.disableMcp, false);
  assert.equal(chat.extraTools, tools);
  assert.equal(chat.hostInstructions, `${note}\n\n${LANGUAGE}\n\n${NO_PROJECT}`);
  assert.equal(asked.length, 1, "the design tools were made for the chat alone");
  // The guess: no MCP, no design tools, the note alone; everything else as the chat's, the rules of
  // a session without a project too.
  assert.equal(Object.hasOwn(guess, "extraTools"), false);
  const rest = { ...chat };
  delete rest.extraTools;
  const { inputProcessors: processors, ...guessOptions } = guess;
  assert.equal(processors.length, 1);
  assert.deepEqual(guessOptions, { ...rest, mcpServers: {}, disableMcp: true, disablePlugins: true, hostInstructions: note });
  assert.deepEqual(guess.initialState.permissionRules, permissionRules(null));
  // The servers a guess names are not even checked: one Mastra Code's builder refuses is ignored.
  await build({ cwd: "/p", mcpServers: [{ name: "old", type: "sse", url: "http://127.0.0.1:1/sse", headers: [] }], _meta: meta({ nextStep: true }) });
  assert.deepEqual(calls.configs[2].mcpServers, {});
  assert.equal(asked.length, 1);
});

test("G88 the actual model request has no tools for a prediction, but ordinary chat keeps its tools", async () => {
  const { sdk, calls } = fakeSdk();
  const build = sessionBuilder(sdk);
  for (const nextStep of [true, false]) {
    await build({ cwd: "/p", mcpServers: [], _meta: { intcad: { nextStep } } });
    const seen: Json[] = [];
    const model: Json = {
      specificationVersion: "v2", provider: "stub", modelId: "offline", supportedUrls: {},
      async doGenerate(request: Json) {
        seen.push(request);
        return { content: [{ type: "text", text: "done" }], finishReason: "stop",
          usage: { inputTokens: 1, outputTokens: 1, totalTokens: 2 }, warnings: [] };
      },
    };
    const agent = new CoreAgent({
      id: `tool-guard-${nextStep}`, name: "boundary check", instructions: "Answer in words.", model: model as never,
      tools: { inspect_file: { id: "inspect_file", description: "fixture tool", execute: async () => "fixture" } },
      inputProcessors: calls.configs.at(-1).inputProcessors ?? [],
    });
    await agent.generate("Suggest a step.");
    assert.equal(seen.length, 1, "the model stub received one request");
    assert.deepEqual((seen[0]?.tools ?? []).map((tool: Json) => tool.name), nextStep ? [] : ["inspect_file"]);
  }
});

test("built-in predictions fail closed if Core leaves tools in its stream request", async () => {
  class IgnoresInputProcessors extends CoreAgent {
    constructor(config: Pick<ConstructorParameters<typeof CoreAgent>[0], "id" | "name" | "instructions" | "model"> & { tools: Json }) {
      super({ ...config, inputProcessors: [] });
    }
  }
  await predictionToolGuard(CoreAgent)();
  const { sdk, calls } = fakeSdk();
  const build = sessionBuilder({ ...sdk, verifyPredictionTools: predictionToolGuard(IgnoresInputProcessors) });
  await assert.rejects(build({ cwd: "/p", mcpServers: [], _meta: { intcad: { nextStep: true } } }), /tool-free predictions/u);
  assert.equal(calls.configs.length, 0, "do not create a prediction session if the guard fails");
  await build({ cwd: "/p", mcpServers: [] });
  assert.equal(calls.configs.length, 1, "ordinary chat does not depend on the guard");
});

// ---- session/close and the guesses' read-only mode (agent.mjs), G89 and G90 -----------------------

class AcpError extends Error {
  code: number;
  data: unknown;
  constructor(code: number, message: string, data?: unknown) {
    super(message);
    this.code = code;
    this.data = data;
  }
}

/** A session as Mastra Code's ACP agent keeps one, recording what is done to it. */
function openSession({ modes = ["build", "plan", "fast"], project = null as string | null } = {}) {
  let mode = "build";
  const listeners = new Set<(event: Json) => void>();
  const done = { switched: [] as string[], aborted: 0, cleanedUp: 0 };
  const entry: Json = {
    project,
    modes: modes.map((id) => ({ id, name: id })),
    state: null,
    turns: new Set<Json>(),
    controller: { listAvailableModels: async () => [] },
    session: {
      model: { get: () => "anthropic/claude-fable-5-2" },
      mode: { get: () => mode, switch: async ({ modeId }: Json) => void (done.switched.push(modeId), (mode = modeId)) },
      state: { set: async () => {} },
      abort: () => void (done.aborted += 1),
      respondToToolApproval: (answer: Json) => answer,
      subscribe: (listener: (event: Json) => void) => {
        listeners.add(listener);
        return () => void listeners.delete(listener);
      },
    },
    cleanup: async () => void (done.cleanedUp += 1),
  };
  return { entry, done, listening: () => listeners.size };
}

/**
 * The agent on a stand-in for Mastra Code's ACP agent (`dist/acp/agent.js` of SDK 1.19.1), as far as
 * these cases need it: `session/new` opens the next of `entries` as s1, s2 …, a turn runs until it is
 * aborted, and a session not on the list is unknown.
 */
function guessingAgent(entries: Json[]) {
  const next = [...entries];
  let opened = 0;
  class Base {
    connection: Json;
    sessions = new Map<string, Json>();
    constructor(connection: Json) {
      this.connection = connection;
    }
    getSession(sessionId: string): Json {
      const entry = this.sessions.get(sessionId);
      if (!entry) throw new AcpError(-32602, "Unknown ACP session", { sessionId });
      return entry;
    }
    async initialize() {
      return { protocolVersion: 1, agentInfo: { name: "mastracode" }, agentCapabilities: { loadSession: false, mcpCapabilities: { http: true, sse: false } }, authMethods: [{ id: "openai-codex" }] };
    }
    configOptions(entry: Json) {
      return [{ id: "mode", currentValue: entry.session.mode.get(), options: entry.modes.map((item: Json) => ({ value: item.id, name: item.name })) }];
    }
    async newSession() {
      const entry = next.shift();
      if (!entry) throw new Error("this test opens no more sessions");
      const sessionId = `s${++opened}`;
      entry.unsubscribe = entry.session.subscribe(() => {});
      const response = { sessionId, modes: { currentModeId: entry.session.mode.get(), availableModes: entry.modes }, configOptions: this.configOptions(entry) };
      this.sessions.set(sessionId, entry);
      return response;
    }
    async prompt(request: Json) {
      const entry = this.getSession(request.sessionId);
      const turn = { cancelled: false };
      entry.turns.add(turn);
      const state: Json = { cancelled: false };
      try {
        await entry.session.sendMessage?.({ content: request.prompt[0]?.text });
        const reason = await new Promise((resolve) => {
          state.resolve = resolve;
          entry.state = state;
        });
        return { stopReason: state.cancelled || reason === "aborted" ? "cancelled" : "end_turn" };
      } finally {
        if (entry.state === state) entry.state = null;
        entry.turns.delete(turn);
      }
    }
  }
  const Agent = intcadAgent({ Base, RequestError: AcpError, connections: world().connections, dates: () => new Map(DATES), reloadSignIns: () => {} });
  return new Agent();
}

const unknownSession = (error: Json) => error.code === -32602 && /Unknown ACP session/u.test(error.message);

test("G89 session/close is offered, ends the one session once with its turn, and a closed or unknown one is refused", async () => {
  const s1 = openSession();
  const s2 = openSession();
  const agent = guessingAgent([s1.entry, s2.entry]);
  const offered = await agent.initialize({ protocolVersion: 1 });
  assert.deepEqual(offered.agentCapabilities, { loadSession: false, mcpCapabilities: { http: true, sse: false }, promptCapabilities: { image: true, embeddedContext: true }, sessionCapabilities: { close: {} } });
  assert.deepEqual([offered.protocolVersion, offered.agentInfo, offered.authMethods], [1, { name: "mastracode" }, []]);
  await agent.newSession({ cwd: "/p", mcpServers: [] });
  await agent.newSession({ cwd: "/p", mcpServers: [] });
  assert.deepEqual([s1.listening(), s2.listening()], [2, 2], "Mastra Code's listener and the task list's");
  // A turn under way on s1.
  const running = agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "Fix this part." }] });
  await settle();
  const turns = [...s1.entry.turns];
  assert.ok(s1.entry.state && turns.length === 1, "the turn is running");

  assert.deepEqual(await agent.closeSession({ sessionId: "s1" }), {});
  assert.deepEqual(await running, { stopReason: "cancelled" });
  assert.ok(turns.every((turn: Json) => turn.cancelled), "the turn was cancelled");
  assert.deepEqual(s1.done, { switched: [], aborted: 1, cleanedUp: 1 });
  assert.equal(s1.listening(), 0, "no listener is left");
  assert.equal(agent.sessions.has("s1"), false);
  // Closed: a prompt and a second close are refused, as is a close of a session never opened.
  await assert.rejects(agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "Again." }] }), unknownSession);
  await assert.rejects(agent.closeSession({ sessionId: "s1" }), unknownSession);
  await assert.rejects(agent.closeSession({ sessionId: "nope" }), unknownSession);
  await assert.rejects(agent.closeSession({}), unknownSession);
  assert.equal(s1.done.cleanedUp, 1, "cleaned up once");
  // s2 as it was.
  assert.deepEqual(s2.done, { switched: [], aborted: 0, cleanedUp: 0 });
  assert.equal(s2.listening(), 2);
  assert.equal(agent.sessions.get("s2"), s2.entry);
});

test("G90 a guess of the next step is put in the read-only mode before the reply; without that mode it is refused and ended", async () => {
  // The guess is given a project, which the session builder never does (G87), to show that the
  // plan gate holds nothing of a guess whatever its entry says.
  const guess = openSession({ project: "/home/u/plate" });
  const chat = openSession();
  const lacking = openSession({ modes: ["build", "plan"] });
  const agent = guessingAgent([guess.entry, chat.entry, lacking.entry]);
  const asGuess = { cwd: "/p", mcpServers: [], _meta: { intcad: { nextStep: true } } };

  const reply = await agent.newSession(asGuess);
  assert.deepEqual(guess.done.switched, ["fast"]);
  assert.equal(reply.modes.currentModeId, "fast");
  assert.deepEqual(reply.modes.availableModes, guess.entry.modes);
  assert.equal(reply.configOptions.find((option: Json) => option.id === "mode").currentValue, "fast");
  assert.equal(guess.entry.plansFirst, undefined);
  // Without the flag the mode stays as Mastra Code opened it.
  const opened = await agent.newSession({ cwd: "/p", mcpServers: [], _meta: { intcad: { nextStep: "true" } } });
  assert.deepEqual(chat.done.switched, []);
  assert.equal(opened.modes.currentModeId, "build");
  assert.equal(opened.configOptions.find((option: Json) => option.id === "mode").currentValue, "build");
  // No read-only mode: the guess is refused, and ended rather than left open in a mode that writes.
  await assert.rejects(agent.newSession(asGuess), (error: Json) => error.code === -32603 && /no read-only mode/u.test(error.message));
  assert.deepEqual(lacking.done, { switched: [], aborted: 0, cleanedUp: 1 });
  assert.equal(lacking.listening(), 0);
  assert.deepEqual([...agent.sessions.keys()], ["s1", "s2"]);
});

test("prediction sessions never allocate plan state or subscribe to task updates", async (t) => {
  const guess = openSession();
  const chat = openSession();
  const prompted = t.mock.method(PlanGate.prototype, "prompted");
  const agent = guessingAgent([guess.entry, chat.entry]);
  guess.entry.session.sendMessage = async () => {};
  chat.entry.session.sendMessage = async () => {};
  await agent.newSession({ cwd: "/p", mcpServers: [], _meta: { intcad: { nextStep: true } } });
  await agent.newSession({ cwd: "/p", mcpServers: [] });
  const guessTurn = agent.prompt({ sessionId: "s1", prompt: [{ type: "text", text: "a guess" }] });
  const chatTurn = agent.prompt({ sessionId: "s2", prompt: [{ type: "text", text: "a chat" }] });
  await settle();
  const listeners = guess.listening();
  await Promise.all([agent.closeSession({ sessionId: "s1" }), agent.closeSession({ sessionId: "s2" }), guessTurn, chatTurn]);
  assert.deepEqual(prompted.mock.calls.map((call: Json) => call.arguments[0]), ["s2"]);
  assert.equal(listeners, 1, "only the SDK's own listener");
  assert.equal(guess.listening(), 0);
});
