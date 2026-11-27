#!/usr/bin/env node
/**
 * Compare Intcad's built-in agent with Mastra Code's own `mastracode --acp`, by running both.
 *
 *   node compare-official-acp.mjs --ours <mastra-agent folder> --out <folder>
 *        [--official <folder>] [--only <text>] [--sign-in] [--refresh]
 *
 * Both are started with the folder's own Node, against the same stand-in model service
 * (agent-smoke.mjs), with nothing of the person running it: a new home, a new data folder, no key,
 * no account. Mastra Code's is the release that goes with the SDK version Intcad's agent is
 * pinned to (agent-package/package.json). It is installed from the npm registry into
 * `<out>/official`, unless `--official` names a folder that has it already.
 *
 * Each question is put to three programs:
 *   official      `mastracode --acp` as a person's own Mastra Code starts: it is given a home
 *                 folder and nothing else;
 *   official+app  the same program with what an app can do from outside before it starts it:
 *                 environment variables, and files in its data folder;
 *   ours          Intcad's entry, as the app starts it.
 * A difference then reads as one of three things: Mastra Code's entry does it by itself, an app
 * gets it by setting something before the start, or it takes code inside the process.
 *
 * A guard is loaded into every agent (`node --import`). It refuses every connection that is not
 * to this computer and writes down each place the agent tried to reach and each file it wrote.
 * Before a message is sent the run checks that the data folder holds no sign-in file and that the
 * model in use is the stand-in's, and sends nothing otherwise.
 *
 * `--sign-in` also starts a ChatGPT sign-in in each and watches how long it waits (macOS and
 * Linux, where a stand-in for the system's browser opener can be put first on PATH; no browser is
 * opened, the sign-in is never finished, and it holds local ports 1455 and 1457 for about two
 * minutes). `--refresh` keeps a session open for six minutes to see who asks for the providers'
 * lists again.
 *
 * Writes `<out>/observed.md` (the table), `<out>/observed.json` and `<out>/raw/` (every ACP
 * message, every request to the stand-in model, the guard's notes, standard error), and prints
 * the table. It says what these programs did on the system it was run on, and nothing more.
 */

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, chmodSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import net from "node:net";
import { homedir, tmpdir } from "node:os";
import { delimiter, dirname, join, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { listing, modelService, ownEnvironment, start, turnOf } from "./agent-smoke.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const smoke = join(here, "agent-smoke.mjs");
/** The `mastracode` release published with `@mastra/code-sdk` 1.19.1 and `@mastra/core` 1.84.0 (2026-11-17). */
const MASTRACODE = "0.50.1";
const WAYS = ["official", "official+app", "ours"];
// Made up, and shaped like no key: only where they turn up matters.
const ADDRESS_KEY = "address-key-made-up-for-the-comparison";
const ENV_KEY = "environment-key-made-up-for-the-comparison";
const HANDED_KEY = "handed-key-made-up-for-the-comparison";
const CONTROL = "control-value-of-the-comparison";
const HOME_NOTE = "A-NOTE-FROM-THE-PERSONS-HOME-FOLDER";

const GUARD = String.raw`// Loaded into an agent by compare-official-acp.mjs (node --import). It refuses every connection
// that is not to this computer, and writes down every try to reach a place and each file the
// agent wrote, with the seconds since the start, to $INTCAD_GUARD_OUT.<pid>.
import dns from 'node:dns';
import fs from 'node:fs';
import net from 'node:net';
import { syncBuiltinESMExports } from 'node:module';
const out = process.env.INTCAD_GUARD_OUT + '.' + process.pid;
const append = fs.appendFileSync;
const began = Date.now();
const seen = new Set();
const note = (kind, what) => {
  if (kind === 'write' && seen.has(what)) return;
  seen.add(what);
  try { append(out, kind + '\t' + Math.round((Date.now() - began) / 1000) + '\t' + what + '\n'); } catch {}
};
note('guard', 'present');
const local = host => host === undefined || host === 'localhost' || host === '::1' || host === '0.0.0.0' || /^127\./.test(host) || /^::ffff:127\./.test(host);
const connect = net.Socket.prototype.connect;
net.Socket.prototype.connect = function (...args) {
  const o = typeof args[0] === 'object' && args[0] !== null ? (Array.isArray(args[0]) ? args[0][0] : args[0]) : { port: args[0], host: typeof args[1] === 'string' ? args[1] : undefined };
  const host = o?.host ?? o?.hostname;
  if (o?.path || local(host)) return connect.apply(this, args);
  note('refused', host + ':' + o?.port);
  process.nextTick(() => this.destroy(Object.assign(new Error('connect ECONNREFUSED ' + host + ' (refused by the comparison)'), { code: 'ECONNREFUSED' })));
  return this;
};
const lookup = dns.lookup;
dns.lookup = function (host, ...rest) {
  if (local(host)) return lookup.call(this, host, ...rest);
  note('refused', String(host));
  const done = rest[rest.length - 1];
  if (typeof done === 'function') process.nextTick(() => done(Object.assign(new Error('getaddrinfo ENOTFOUND ' + host + ' (refused by the comparison)'), { code: 'ENOTFOUND' })));
};
const realFetch = globalThis.fetch;
globalThis.fetch = function (input, ...rest) {
  try {
    const u = new URL(typeof input === 'string' ? input : input?.url ?? String(input));
    if (!local(u.hostname)) note('fetch', u.origin + u.pathname);
  } catch {}
  return realFetch.call(this, input, ...rest);
};
const path = p => { try { return typeof p === 'string' ? p : p instanceof URL ? p.pathname : Buffer.isBuffer(p) ? p.toString() : null; } catch { return null; } };
const wrap = (object, name, flag = null) => {
  const real = object[name];
  if (typeof real !== 'function') return;
  object[name] = function (p, ...rest) {
    const s = path(p);
    if (s && s !== out && (flag === null || /[wa+]/.test(String(rest[flag] ?? 'r')))) note('write', s);
    return real.call(this, p, ...rest);
  };
};
for (const name of ['writeFileSync', 'writeFile', 'appendFile', 'createWriteStream', 'mkdirSync', 'mkdir', 'renameSync', 'rename', 'copyFileSync', 'copyFile']) wrap(fs, name);
wrap(fs, 'openSync', 0);
wrap(fs, 'open', 0);
for (const name of ['writeFile', 'appendFile', 'mkdir', 'rename', 'copyFile']) wrap(fs.promises, name);
wrap(fs.promises, 'open', 0);
fs.appendFileSync = function (p, ...rest) { const s = path(p); if (s && s !== out) note('write', s); return append.call(this, p, ...rest); };
syncBuiltinESMExports();
`;

const fail = (message) => {
  console.error(`error: ${message}`);
  process.exit(2);
};
const slug = (text) => text.replace(/[^a-z0-9+]+/giu, "-").replace(/^-|-$/gu, "");
const sha256 = (file) => createHash("sha256").update(readFileSync(file)).digest("hex");
const pause = (ms) => new Promise((done) => setTimeout(done, ms));
const versionOf = (modules, name) => JSON.parse(readFileSync(join(modules, ...name.split("/"), "package.json"), "utf8")).version;
/** Mastra Code's own data folder when nothing names one (`getAppDataDir` of its SDK). */
const defaultData = (home) => (process.platform === "darwin" ? join(home, "Library", "Application Support", "mastracode") : process.platform === "win32" ? join(home, "AppData", "Roaming", "mastracode") : join(home, ".local", "share", "mastracode"));
const files = (folder) => (existsSync(folder) ? listing(folder).split("\n").filter(Boolean).map((line) => line.replace(/ \d+ [\d.]+$/u, "")) : []);
/** The files under a folder that hold a text. */
function holding(folder, text) {
  const found = [];
  const walk = (dir) => {
    for (const entry of existsSync(dir) ? readdirSync(dir, { withFileTypes: true }) : []) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (statSync(path).size < 50e6 && readFileSync(path).includes(text)) found.push(path);
    }
  };
  walk(folder);
  return found;
}
const freePort = () => new Promise((done) => {
  const server = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = server.address();
    server.close(() => done(port));
  });
});
const portFree = (port) => new Promise((done) => {
  const server = net.createServer().once("error", () => done(false)).listen(port, "127.0.0.1", () => server.close(() => done(true)));
});

async function main(args) {
  const flag = (name, takesValue = true) => {
    const index = args.indexOf(name);
    if (index < 0) return undefined;
    return takesValue ? args.splice(index, 2)[1] : (args.splice(index, 1), true);
  };
  const [oursFolder, outGiven, officialGiven, only] = [flag("--ours"), flag("--out"), flag("--official"), flag("--only")];
  const [withSignIn, withRefresh] = [flag("--sign-in", false), flag("--refresh", false)];
  if (!oursFolder || !outGiven || args.length > 0) fail("usage: compare-official-acp.mjs --ours <mastra-agent folder> --out <folder> [--official <folder>] [--only <text>] [--sign-in] [--refresh]");
  const out = resolve(outGiven);
  const ours = resolve(oursFolder);
  const node = join(ours, process.platform === "win32" ? "node.exe" : "node");
  const entry = join(ours, "agent", "main.mjs");
  for (const file of [node, entry]) if (!existsSync(file)) fail(`${file} is not there`);
  mkdirSync(join(out, "raw"), { recursive: true });
  const work = join(out, "work");
  const guard = join(out, "guard.mjs");
  writeFileSync(guard, GUARD);

  // ---- Mastra Code's own command line, at the release that goes with the pinned SDK ------------
  const pin = JSON.parse(readFileSync(join(here, "..", "agent-package", "package.json"), "utf8")).dependencies["@mastra/code-sdk"];
  const official = resolve(officialGiven ?? join(out, "official"));
  if (!existsSync(join(official, "node_modules", "mastracode", "package.json"))) {
    mkdirSync(official, { recursive: true });
    writeFileSync(join(official, "package.json"), `${JSON.stringify({ name: "official-mastracode-for-comparison", private: true, dependencies: { mastracode: MASTRACODE }, overrides: { "@mastra/code-sdk": pin } }, null, 2)}\n`);
    const installed = spawnSync("npm", ["install", "--ignore-scripts", "--no-audit", "--no-fund"], { cwd: official, stdio: ["ignore", "ignore", "inherit"], shell: process.platform === "win32" });
    if (installed.status !== 0) fail(`npm could not install mastracode@${MASTRACODE} into ${official}`);
  }
  const cli = join(official, "node_modules", "mastracode", "dist", "cli.js");
  const [theirs, mine] = [join(official, "node_modules"), join(ours, "node_modules")];
  const adapted = JSON.parse(readFileSync(join(here, "..", "agent-package", "adapted-from.json"), "utf8"));
  const sdkFiles = (modules) => files(join(modules, "@mastra", "code-sdk", "dist")).filter((file) => file.endsWith(".js"));
  const identity = {
    mastracode: versionOf(theirs, "mastracode"),
    officialSdk: versionOf(theirs, "@mastra/code-sdk"),
    officialCore: versionOf(theirs, "@mastra/core"),
    ourSdk: versionOf(mine, "@mastra/code-sdk"),
    ourCore: versionOf(mine, "@mastra/core"),
    officialSessionBuilder: sha256(join(theirs, ...adapted.package.split("/"), adapted.file)),
    adaptedFrom: adapted.sha256,
    sdkFilesCompared: 0,
    sdkFilesDifferent: [],
  };
  if (identity.officialSdk !== pin) fail(`the official tree has @mastra/code-sdk ${identity.officialSdk}, not the pinned ${pin}`);
  for (const file of sdkFiles(theirs)) {
    const other = join(mine, "@mastra", "code-sdk", "dist", file);
    identity.sdkFilesCompared += 1;
    if (!existsSync(other) || sha256(other) !== sha256(join(theirs, "@mastra", "code-sdk", "dist", file))) identity.sdkFilesDifferent.push(file);
  }

  // ---- One agent, started one of the three ways ----------------------------------------------
  const realHome = homedir();
  const rows = [];
  /**
   * Starts an agent for a question. `options`: `model` (false: no stand-in is handed over),
   * `status` (what the stand-in answers), `address` (in place of the stand-in's own), `env`,
   * `home(folder)` (puts files into the person's home), `keys` (handed to ours), `capabilities`,
   * `permission`.
   */
  async function open(question, way, options = {}) {
    const base = join(work, slug(question), slug(way));
    const [home, project, appData, raw] = [join(base, "home"), join(base, "project"), join(base, "data"), join(out, "raw", slug(question), slug(way))];
    for (const folder of [home, project, raw]) mkdirSync(folder, { recursive: true });
    options.home?.(home);
    const requests = [];
    const keep = (file, value) => {
      if (file === "model-requests.jsonl") requests.push(value.body);
      appendFileSync(join(raw, file), `${JSON.stringify(value)}\n`);
    };
    const service = await modelService({ keep, status: options.status });
    const env = { ...ownEnvironment(appData, { HOME: home, USERPROFILE: home }), INTCAD_GUARD_OUT: join(raw, "guard"), ...options.env };
    let data = appData;
    if (way === "official") {
      delete env.MASTRA_APP_DATA_DIR;
      data = defaultData(home);
    } else if (way === "official+app") {
      mkdirSync(join(appData, "home"), { recursive: true });
      Object.assign(env, { HOME: join(appData, "home"), USERPROFILE: join(appData, "home"), MASTRA_TELEMETRY_DISABLED: "1", MASTRA_OFFLINE: "1", MASTRACODE_SIGNALS_SOCKET_ROOT: join(appData, "signals") });
    }
    const custom = [{ name: "Stand-in", url: options.address ?? `http://127.0.0.1:${service.port}/v1`, apiKey: ADDRESS_KEY, models: ["hello-1"] }];
    // Mastra Code's own way to be told of a model service with no terminal is its settings file.
    if (way !== "ours" && options.model !== false) {
      mkdirSync(data, { recursive: true });
      writeFileSync(join(data, "settings.json"), JSON.stringify({ customProviders: custom }));
    }
    const agent = start({ command: node, args: ["--import", pathToFileURL(guard).href, ...(way === "ours" ? [entry] : [cli, "--acp"])], env, cwd: project, permission: options.permission, keep });
    const run = { question, way, agent, service, requests, base, home, project, data, raw, env };
    try {
      run.initialize = await agent.call("initialize", { protocolVersion: 1, clientCapabilities: options.capabilities ?? {} }, 60000);
      if (way === "ours" && options.model !== false) run.setup = await agent.call("_intcad/connections/setup", { keys: options.keys ?? {}, custom });
    } catch (error) {
      await close(run);
      throw error;
    }
    return run;
  }
  /** Opens a session, moves it to the stand-in's model, and refuses to go on unless that is the model in use. */
  async function session(run) {
    const opened = await run.agent.call("session/new", { cwd: run.project, mcpServers: [] }, 120000);
    const model = () => opened.configOptions?.find((option) => option.id === "model");
    run.openedOn = model()?.currentValue ?? "";
    run.listed = model()?.options?.map((option) => option.value) ?? [];
    const standIn = run.listed.find((value) => value.includes("stand-in"));
    if (standIn && run.openedOn !== standIn) {
      const changed = await run.agent.call("session/set_config_option", { sessionId: opened.sessionId, configId: "model", value: standIn });
      opened.configOptions = changed.configOptions;
    }
    run.modelInUse = model()?.currentValue ?? "";
    const signIns = files(run.base).filter((file) => /(^|\/)auth\.json$/u.test(file));
    if (!run.modelInUse.includes("stand-in") || signIns.length > 0) throw new Error(`not alone with the stand-in (model in use: ${run.modelInUse || "none"}, sign-in files: ${signIns.length}); nothing was sent`);
    run.sessionId = opened.sessionId;
    return opened;
  }
  async function prompt(run, text) {
    const [from, before] = [run.agent.notes.length, run.service.seen.requests];
    const reply = await run.agent.call("session/prompt", { sessionId: run.sessionId, prompt: [{ type: "text", text }] });
    return { stop: reply.stopReason, askedStandIn: run.service.seen.requests > before, ...turnOf(run.agent.notes, from) };
  }
  /** Stops the agent and reads what the guard wrote down. */
  async function close(run) {
    if (run.closed) return run;
    run.closed = true;
    await run.agent.stop();
    run.service.close();
    writeFileSync(join(run.raw, "stderr.txt"), run.agent.stderr());
    writeFileSync(join(run.raw, "files.txt"), `${files(run.base).join("\n")}\n`);
    const notes = readdirSync(run.raw).filter((file) => file.startsWith("guard.")).flatMap((file) => readFileSync(join(run.raw, file), "utf8").split("\n").filter(Boolean).map((line) => line.split("\t")));
    run.guardPresent = notes.some(([kind]) => kind === "guard");
    run.reached = [...new Set(notes.filter(([kind]) => kind === "refused").map(([, , what]) => what.replace(/:\d+$/u, "")))];
    run.fetched = notes.filter(([kind]) => kind === "fetch").map(([, seconds, what]) => `${what} at ${seconds} s`);
    // Anything written outside the run's own folders, the system's temporary folder aside.
    const inside = [run.base, run.raw, tmpdir(), "/tmp", "/private/tmp", "/private/var/folders", "/dev/"];
    run.wroteOutside = [...new Set(notes.filter(([kind]) => kind === "write").map(([, , what]) => what).filter((path) => !inside.some((folder) => path.startsWith(folder))))];
    if (run.wroteOutside.some((path) => path.startsWith(realHome + sep) && !path.startsWith(run.base))) throw new Error(`an agent wrote into the home folder of the person running this (${run.wroteOutside[0]}); stopped`);
    if (!run.guardPresent) throw new Error("the guard was not loaded into an agent; stopped");
    return run;
  }

  /** Puts one question to the three programs. `ask(run)` returns what was seen, with `key`: the part that is compared. */
  async function question(text, options, ask) {
    if (only && !text.includes(only)) return;
    const row = { question: text, seen: {} };
    for (const way of WAYS) {
      let run;
      try {
        run = await open(text, way, typeof options === "function" ? options(way) : options);
        row.seen[way] = await ask(run);
      } catch (error) {
        row.seen[way] = { key: "failed", says: `could not be seen: ${String(error.message).slice(0, 200)}` };
      }
      if (run) {
        await close(run);
        if (run.wroteOutside.length > 0) row.seen[way].wroteOutside = run.wroteOutside;
      }
    }
    const [a, b, c] = WAYS.map((way) => row.seen[way].key);
    row.reading = a === "failed" || b === "failed" || c === "failed" ? "not all three were seen" : a === c ? "Mastra Code's entry does the same by itself" : b === c ? "an app gets it by what it sets before the start" : "it takes code inside the process";
    rows.push(row);
    console.error(`asked: ${text}`);
  }

  // ---- The questions -------------------------------------------------------------------------

  await question("What sign-in does it offer the app before any session?", { model: false, capabilities: { auth: { terminal: true } } }, async (run) => {
    const methods = (run.initialize.authMethods ?? []).map((method) => `${method.id} (${method.type === "terminal" ? "in a terminal" : "in the browser"})`);
    let connections;
    try {
      const list = await run.agent.call("_intcad/connections/list", {}, 20000);
      connections = `${list.plans.length} plans, ${list.keys.length} keys and custom addresses over \`_intcad/connections/*\``;
    } catch (error) {
      connections = `\`_intcad/connections/list\`: ${error.message}`;
    }
    return { key: `${methods.length}|${connections.includes("plans")}`, says: `ACP sign-in methods: ${methods.join(", ") || "none"}. ${connections}.`, methods };
  });

  await question("What does a new session do when nothing is connected?", { model: false }, async (run) => {
    try {
      const opened = await run.agent.call("session/new", { cwd: run.project, mcpServers: [] }, 120000);
      const model = opened.configOptions?.find((option) => option.id === "model");
      return { key: `opens on ${model?.currentValue}`, says: `It opens, on \`${model?.currentValue}\`, with ${model?.options?.length ?? 0} model(s) listed and nothing connected that serves any.` };
    } catch (error) {
      return { key: "refuses", says: `It refuses: ${error.message}` };
    }
  });

  await question("How is a model service handed over with no terminal, and where is its key kept?", {}, async (run) => {
    await session(run);
    const turn = await prompt(run, "hi");
    const kept = holding(run.base, ADDRESS_KEY).map((file) => file.slice(run.base.length + 1));
    const how = run.way === "ours" ? "Over ACP, by `_intcad/connections/setup`." : "By `settings.json` in its data folder, written before the start.";
    return { key: `${run.way === "ours" ? "acp" : "file"}|${kept.length > 0}`, says: `${how} The stand-in answered: ${turn.askedStandIn && turn.stop === "end_turn" ? "yes" : "no"}. The address's key is ${kept.length > 0 ? `in ${kept.join(", ")}` : "in no file"}.`, openedOn: run.openedOn, listed: run.listed };
  });

  await question("Which model is a session on when it opens, with one model service connected?", {}, async (run) => {
    await session(run);
    return { key: String(run.openedOn.includes("stand-in")), says: `It opens on \`${run.openedOn}\`; listed: ${run.listed.map((value) => `\`${value}\``).join(", ")}.` };
  });

  await question("Does a session make a vector store?", {}, async (run) => {
    await session(run);
    await prompt(run, "hi");
    await close(run);
    const vectors = files(run.base).filter((file) => /vectors/u.test(file));
    const models = run.reached.filter((host) => /huggingface|hf\.co/u.test(host));
    return { key: String(vectors.length > 0), says: vectors.length > 0 ? `Yes: ${vectors.map((file) => `\`${file.split("/").pop()}\``).join(", ")}.${models.length > 0 ? ` It tried to reach ${models.join(", ")}.` : ""}` : "No vector database was made." };
  });

  await question("Where does it keep what it writes?", {}, async (run) => {
    await session(run);
    await prompt(run, "hi");
    await close(run);
    const count = (folder) => files(folder).filter((file) => !file.endsWith("/")).length;
    const [given, standard] = [count(join(run.base, "data")), count(defaultData(run.home))];
    const elsewhere = count(run.home) - standard;
    return { key: standard > 0 ? "in the home folder" : "in the data folder", says: `${given} file(s) in the data folder it was given, ${standard} in Mastra Code's own folder under the person's home (\`${defaultData("~").replace(/\\/gu, "/")}\`), ${elsewhere} elsewhere in that home.` };
  });

  await question("Does it reach anything outside this computer without being asked?", {}, async (run) => {
    await session(run);
    await prompt(run, "hi");
    await close(run);
    return { key: run.reached.sort().join(","), says: run.reached.length > 0 ? `It tried: ${run.reached.sort().join(", ")} (refused by the guard).` : "It tried nothing.", fetched: run.fetched };
  });

  await question("Does it read the person's own agent files from the home folder?", {
    home: (home) => {
      mkdirSync(join(home, ".claude"), { recursive: true });
      writeFileSync(join(home, ".claude", "CLAUDE.md"), `${HOME_NOTE}\n`);
      mkdirSync(join(home, ".mastracode"), { recursive: true });
      writeFileSync(join(home, ".mastracode", "mcp.json"), JSON.stringify({ mcpServers: { "home-server": { command: process.execPath, args: [smoke, "--mcp-server", join(home, "mcp-calls.log")] } } }));
    },
  }, async (run) => {
    await session(run);
    await prompt(run, "hi");
    const told = run.requests.some((body) => JSON.stringify(body.messages ?? []).includes(HOME_NOTE) || JSON.stringify(body.system ?? "").includes(HOME_NOTE));
    const tool = run.requests.some((body) => (body.tools ?? []).some((item) => /measure_part/u.test((item.function ?? item).name ?? "")));
    return { key: `${told}|${tool}`, says: `The model was sent the note from \`~/.claude/CLAUDE.md\`: ${told ? "yes" : "no"}. The MCP server of \`~/.mastracode/mcp.json\` was started and its tool offered to the model: ${tool ? "yes" : "no"}.` };
  });

  await question("Does a command the model runs see a provider's key?", (way) => ({ env: { OPENAI_API_KEY: ENV_KEY, INTCAD_COMPARE_CONTROL: CONTROL }, keys: way === "ours" ? { openai: HANDED_KEY } : {} }), async (run) => {
    await session(run);
    const turn = await prompt(run, `run the command: ${process.platform === "win32" ? "set" : "env"}`);
    const result = run.requests.flatMap((body) => body.messages ?? []).filter((message) => message.role === "tool").map((message) => JSON.stringify(message.content)).join("\n");
    const [fromEnvironment, handed, control] = [result.includes(ENV_KEY), result.includes(HANDED_KEY), result.includes(CONTROL)];
    const saved = run.setup?.keys?.find((key) => key.id === "openai")?.saved;
    return {
      key: control ? String(fromEnvironment || handed) : "failed",
      says: control ? `The command's output had the key from the agent's environment: ${fromEnvironment ? "yes" : "no"}${run.way === "ours" ? `; the key the app handed over (saved: ${saved}): ${handed ? "yes" : "no"}` : ""}. (A control variable was in the output, so the command did print its environment.)` : `could not be seen: the command's output had no control variable (${turn.tools.join(", ") || "no tool"})`,
    };
  });

  const refusals = [["401", () => 401], ["429", () => 429], ["404", () => 404]];
  for (const [name, status] of refusals) {
    await question(`What does a turn say when the model service answers ${name}?`, { status }, async (run) => {
      await session(run);
      try {
        const turn = await prompt(run, "hi");
        return { key: "no error", says: `No error: the turn ended with \`${turn.stop}\` and the text "${turn.answer}".` };
      } catch (error) {
        return { key: String(error.data?.intcad?.kind ?? "text only"), says: `Error ${error.code}: "${String(error.message).slice(0, 160)}"${error.data?.intcad ? `, with the kind \`${error.data.intcad.kind}\`` : ", with no kind"}.` };
      }
    });
  }
  const closedPort = await freePort();
  await question("What does a turn say when nothing listens at the model service's address?", { address: `http://127.0.0.1:${closedPort}/v1` }, async (run) => {
    await session(run);
    try {
      const turn = await prompt(run, "hi");
      return { key: "no error", says: `No error: the turn ended with \`${turn.stop}\` and the text "${turn.answer}".` };
    } catch (error) {
      return { key: String(error.data?.intcad?.kind ?? "text only"), says: `Error ${error.code}: "${String(error.message).slice(0, 160)}"${error.data?.intcad ? `, with the kind \`${error.data.intcad.kind}\`` : ", with no kind"}.` };
    }
  });

  await question("Where are the lease files of its sessions?", {}, async (run) => {
    const shared = "/tmp/mc";
    const before = existsSync(shared) ? readdirSync(shared) : [];
    await session(run);
    await prompt(run, "hi");
    // Mastra Code writes a session's lease about ten seconds after the agent starts.
    await pause(20000);
    const added = (existsSync(shared) ? readdirSync(shared) : []).filter((name) => !before.includes(name));
    const own = files(run.base).filter((file) => /(^|\/)signals\//u.test(file));
    return { key: added.length > 0 ? "shared" : own.length > 0 ? "own" : "none", says: added.length > 0 ? `${added.length} new entr${added.length === 1 ? "y" : "ies"} in \`${shared}\`, which every Mastra Code of this user shares (left there when the agent ended).` : own.length > 0 ? `In its own folder: \`${own[0].split("/").slice(0, -1).join("/")}\`.` : `None was written in the first 20 seconds (\`${shared}\` got no new entry).` };
  });

  if (withRefresh) {
    await question("Who asks for the providers' lists again while a session stays open (six minutes)?", {}, async (run) => {
      await session(run);
      await prompt(run, "hi");
      await pause(6 * 60 * 1000);
      await close(run);
      const later = readdirSync(run.raw).filter((file) => file.startsWith("guard.")).flatMap((file) => readFileSync(join(run.raw, file), "utf8").split("\n").filter(Boolean).map((line) => line.split("\t"))).filter(([kind, seconds]) => kind === "fetch" && Number(seconds) > 60).map(([, seconds, what]) => `${what} at ${seconds} s`);
      return { key: String(later.length > 0), says: later.length > 0 ? `After the first minute it asked ${later.length} time(s), such as: ${later.slice(0, 4).join("; ")}.` : "After the first minute it asked for nothing.", later };
    });
  }

  if (withSignIn) await signIn();

  /** A ChatGPT sign-in is started and never finished: who opens the browser, and how long the wait is. */
  async function signIn() {
    const text = "How long does a ChatGPT sign-in wait for the browser, and who gives up?";
    if (only && !text.includes(only)) return;
    const row = { question: text, seen: {} };
    rows.push(row);
    const opener = process.platform === "darwin" ? "open" : "xdg-open";
    const reason = process.platform === "win32" ? "on Windows the browser opener cannot be replaced from outside" : !(await portFree(1455)) || !(await portFree(1457)) ? "local port 1455 or 1457 is in use" : null;
    for (const way of WAYS) {
      if (reason) {
        row.seen[way] = { key: "failed", says: `not run: ${reason}` };
        continue;
      }
      const bin = join(work, slug(text), slug(way), "bin");
      const log = join(work, slug(text), slug(way), "opened.log");
      mkdirSync(bin, { recursive: true });
      writeFileSync(join(bin, opener), `#!/bin/sh\nprintf 'home=%s url=%s\\n' "$HOME" "$1" >> "${log}"\n`);
      chmodSync(join(bin, opener), 0o755);
      const path = `${bin}${delimiter}${process.env.PATH ?? ""}`;
      // The stand-in must be what the name resolves to, or no sign-in is started.
      const tried = spawnSync(opener, ["https://example.invalid/check"], { env: { PATH: path, HOME: "check" }, stdio: "ignore" });
      if (tried.status !== 0 || !existsSync(log) || !readFileSync(log, "utf8").includes("example.invalid/check")) {
        row.seen[way] = { key: "failed", says: "not run: the stand-in for the browser opener was not the one found on PATH" };
        continue;
      }
      let run;
      try {
        run = await open(text, way, { model: false, env: { PATH: path } });
        const began = Date.now();
        const seconds = () => Math.round((Date.now() - began) / 1000);
        const opened = () => readFileSync(log, "utf8").split("\n").filter((line) => line && !line.includes("example.invalid"));
        if (way === "ours") {
          await run.agent.call("_intcad/connections/login", { id: "openai-codex" }, 30000);
          await pause(75000);
          const list = await run.agent.call("_intcad/connections/list", {}, 20000);
          const waiting = `phase \`${list.login?.phase ?? "none"}\``;
          await run.agent.call("_intcad/connections/cancel", {}, 20000);
          await pause(1000);
          row.seen[way] = { key: "waits", says: `Started over \`_intcad/connections/login\`. The opener was started ${opened().length} time(s)${opened()[0] ? `, with ${opened()[0].split(" ")[0].replace(run.base, "<run>")}` : ""}. After ${seconds()} s it had not given up (${waiting}). After Cancel, ports 1455 and 1457 were free: ${(await portFree(1455)) && (await portFree(1457)) ? "yes" : "no"}.` };
        } else {
          let outcome;
          try {
            await run.agent.call("authenticate", { methodId: "openai-codex" }, 150000);
            outcome = "it answered without an error";
          } catch (error) {
            outcome = `it gave up with: "${String(error.message).slice(0, 160)}"`;
          }
          row.seen[way] = { key: `gives up`, says: `Started over ACP's \`authenticate\`. The opener was started ${opened().length} time(s)${opened()[0] ? `, with ${opened()[0].split(" ")[0].replace(run.base, "<run>")}` : ""}. After ${seconds()} s ${outcome}.` };
        }
      } catch (error) {
        row.seen[way] = { key: "failed", says: `could not be seen: ${String(error.message).slice(0, 200)}` };
      }
      if (run) await close(run);
    }
    const [a, b, c] = WAYS.map((way) => row.seen[way].key);
    row.reading = [a, b, c].includes("failed") ? "not all three were seen" : a === c ? "Mastra Code's entry does the same by itself" : b === c ? "an app gets it by what it sets before the start" : "it takes code inside the process";
    console.error(`asked: ${text}`);
  }

  // ---- What was seen ---------------------------------------------------------------------------
  const cell = (text) => String(text).replace(/\|/gu, "\\|").replace(/\n/gu, " ");
  const table = [
    `# Mastra Code's own \`mastracode --acp\` and Intcad's entry, run side by side`,
    "",
    `Run on ${process.platform} ${process.arch} with Node ${spawnSync(node, ["--version"], { encoding: "utf8" }).stdout.trim()}, ${new Date().toISOString().slice(0, 10)}. Every agent had the guard loaded and reached nothing outside this computer.`,
    "",
    `- Official: \`mastracode\` ${identity.mastracode} with \`@mastra/code-sdk\` ${identity.officialSdk} and \`@mastra/core\` ${identity.officialCore}.`,
    `- Ours: Intcad's entry with \`@mastra/code-sdk\` ${identity.ourSdk} and \`@mastra/core\` ${identity.ourCore}.`,
    `- The SDK's JavaScript files are the same in both trees: ${identity.sdkFilesDifferent.length === 0 ? `yes, all ${identity.sdkFilesCompared}` : `no, ${identity.sdkFilesDifferent.length} of ${identity.sdkFilesCompared} differ (${identity.sdkFilesDifferent.slice(0, 5).join(", ")})`}.`,
    `- The official session builder (\`${adapted.file}\`) is the file Intcad's \`session.mjs\` was adapted from: ${identity.officialSessionBuilder === identity.adaptedFrom ? "yes" : "no"} (sha256 \`${identity.officialSessionBuilder.slice(0, 16)}…\`).`,
    "",
    "- **official**: started as a person's own Mastra Code starts, with a home folder and nothing else.",
    "- **official+app**: the same program, with a data folder, a home inside it, `MASTRA_TELEMETRY_DISABLED`, `MASTRA_OFFLINE` and `MASTRACODE_SIGNALS_SOCKET_ROOT` set by whoever starts it.",
    "- **ours**: Intcad's entry, given the person's home folder and a data folder, as the app gives them.",
    "",
    "| Question | official | official+app | ours | Reading |",
    "|---|---|---|---|---|",
    ...rows.map((row) => `| ${cell(row.question)} | ${WAYS.map((way) => cell(row.seen[way].says)).join(" | ")} | ${cell(row.reading)} |`),
    "",
  ].join("\n");
  writeFileSync(join(out, "observed.md"), table);
  writeFileSync(join(out, "observed.json"), `${JSON.stringify({ system: `${process.platform} ${process.arch}`, identity, rows }, null, 2)}\n`);
  console.log(table);
  return rows.some((row) => Object.values(row.seen).some((seen) => seen.key === "failed")) ? 1 : 0;
}

process.exit(await main(process.argv.slice(2)));
