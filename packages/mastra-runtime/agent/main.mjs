#!/usr/bin/env node
// Intcad's built-in agent. The app starts this file and talks ACP with it over stdin and stdout.
//
// It is Mastra Code's ACP agent (its SDK, unchanged) with the parts in this folder added. Where the
// SDK is installed is not this file's business: SOLIDCRAFT_MASTRA_SDK names the `node_modules`
// folder that holds `@mastra/code-sdk` and `@agentclientprotocol/sdk`; without it the folder beside
// this one is used. README.md says which versions this was written against.

import http from 'node:http';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import { Readable, Writable } from 'node:stream';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { intcadAgent } from './agent.mjs';
import { holdCallbackPorts } from './callback.mjs';
import { cadBridge } from './cad.mjs';
import { Connections, KEYS } from './connections.mjs';
import { deniedFolders, keepFilesFrom } from './fence.mjs';
import { attachmentReader, releaseDates } from './models.mjs';
import { keepKeysFromCommands, keyVariables, withoutParams } from './secrets.mjs';
import { sessionBuilder, sweepSessionFolders } from './session.mjs';
import { installSkillSets, setsToInstall } from './skills.mjs';

// ACP's SDK prints a request it refuses, parameters and all. See secrets.mjs.
const printError = console.error.bind(console);
console.error = (...args) => printError(...withoutParams(args));

// Mastra Code is a coding agent: it reads a person's own agent files from their home folder (their
// instructions for agents, their MCP servers, its own settings there). None of that is meant for
// Intcad's assistant or for the model it talks to, so when the app names a data folder the agent
// gets a home of its own inside it. This has to happen before anything of Mastra's is loaded.
const usersHome = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE };
const here = dirname(fileURLToPath(import.meta.url));
if (process.env.MASTRA_APP_DATA_DIR) {
  const home = join(process.env.MASTRA_APP_DATA_DIR, 'home');
  mkdirSync(home, { recursive: true });
  process.env.HOME = home;
  process.env.USERPROFILE = home;
  // Mastra Code keeps small lease files for its sessions under /tmp/mc, shared by every Mastra
  // Code of this user. Intcad's agent keeps its own, with the rest of its data.
  process.env.MASTRACODE_SIGNALS_SOCKET_ROOT = join(process.env.MASTRA_APP_DATA_DIR, 'signals');
  // Mastra Code's file tools reach the system's temporary folder by themselves, as a scratch space
  // (`DEFAULT_ALLOWED_PATHS` in its `agents/workspace.js`). The agent gets a temporary folder of its
  // own, so the user's is not that scratch space (INT-1473: files only in the project folder).
  // `/tmp` itself stays on Mastra Code's list on Linux and macOS.
  const scratch = join(process.env.MASTRA_APP_DATA_DIR, 'tmp');
  mkdirSync(scratch, { recursive: true });
  for (const name of ['TMPDIR', 'TMP', 'TEMP']) process.env[name] = scratch;
  // Each session works in a new folder under `sessions/` (session.mjs). Mastra Code looks for a git
  // repository from there upwards, and would read the agent files, MCP servers and hooks of one it
  // found (a data folder inside a checkout of the user's). Git stops at `sessions/`.
  const sessions = join(process.env.MASTRA_APP_DATA_DIR, 'sessions');
  mkdirSync(sessions, { recursive: true });
  // What an agent stopped before it cleaned up left there goes now.
  sweepSessionFolders(process.env.MASTRA_APP_DATA_DIR);
  process.env.GIT_CEILING_DIRECTORIES = [sessions, process.env.GIT_CEILING_DIRECTORIES].filter(Boolean).join(delimiter);
  // The skills the package carries (skills.mjs) go where Mastra Code looks in its home,
  // `.agents/skills`, before a session reads that folder. Not into the session's folder: Mastra
  // Code skips a link there that leads out of it. Run by hand, without a home of its own, the
  // agent installs nothing: the person's own folder is theirs.
  const log = line => process.stderr.write(`[intcad] ${line}\n`);
  const sets = setsToInstall(join(here, '..'), join(here, '..', '..', '..'), log);
  if (sets.length === 0) log('skills: the package carries no skill set');
  installSkillSets({ sets, home, log });
}

// Mastra reports feature use to its own service unless this is set (`@mastra/core`, and Mastra
// Code's analytics). Intcad's agent reports nothing: it is set here, before anything of Mastra's is
// loaded, so it holds however the agent was started.
process.env.MASTRA_TELEMETRY_DISABLED = '1';
// Mastra refreshes the providers' lists every five minutes and writes them into its cache and
// into its own package folder. An installed package's folder is not to be written, so that
// refresh is off (Mastra's own switch for it). The models a session offers do not come from
// there: Mastra Code reads the public catalogs (models.dev, Netlify) into memory when a session
// opens, and this switch leaves that as it is.
process.env.MASTRA_OFFLINE = '1';

const sdkRoot = resolve(process.env.SOLIDCRAFT_MASTRA_SDK || join(here, '..', 'node_modules'));
// The files are named in full: an absolute path does not go through a package's export map, and a
// bare name would be looked up from this folder, where the SDK need not be.
const load = path => import(pathToFileURL(join(sdkRoot, path)).href);

if (!existsSync(join(sdkRoot, '@mastra', 'code-sdk', 'package.json'))) {
  process.stderr.write(`[intcad] Mastra Code's SDK is not in ${sdkRoot}. Set SOLIDCRAFT_MASTRA_SDK to the node_modules folder that holds it.\n`);
  process.exit(1);
}

const [{ AgentSideConnection, RequestError, ndJsonStream }, { MastraCodeAcpAgent }, { createMastraCode }, settings, { resolveSubagentsSetting }, { withCleanupFailure }, storage, gateway, customSource, { openUrlInBrowser }, { LocalSandbox, LocalFilesystem }, { createTool }, { jsonSchema }, { RequestContext }, cadErrors, { modelSupportsAttachments }] =
  await Promise.all([
    load('@agentclientprotocol/sdk/dist/acp.js'),
    load('@mastra/code-sdk/dist/acp/agent.js'),
    load('@mastra/code-sdk/dist/index.js'),
    load('@mastra/code-sdk/dist/onboarding/settings.js'),
    load('@mastra/code-sdk/dist/agents/subagents/index.js'),
    load('@mastra/code-sdk/dist/acp/errors.js'),
    load('@mastra/code-sdk/dist/auth/storage.js'),
    load('@mastra/code-sdk/dist/agents/mastracode-gateway.js'),
    load('@mastra/code-sdk/dist/agents/custom-provider-source.js'),
    load('@mastra/code-sdk/dist/utils/open-url.js'),
    load('@mastra/core/dist/workspace/index.js'),
    load('@mastra/core/dist/tools/index.js'),
    load('ai/dist/index.js'),
    load('@mastra/core/dist/request-context/index.js'),
    load('@mastra/core/dist/error/index.js'),
    load('@mastra/core/dist/llm/index.js'),
  ]);

let registry = {};
try {
  registry = JSON.parse(readFileSync(join(sdkRoot, '@mastra', 'core', 'dist', 'provider-registry.json'), 'utf8'));
} catch (error) {
  // Without the registry the models have no dates and list by name; nothing else depends on it.
  process.stderr.write(`[intcad] no release dates: ${error}\n`);
}
const dates = releaseDates(registry);
const keyEnv = id => [registry.providers?.[id]?.apiKeyEnvVar ?? []].flat();
// Which models a message's pictures and documents are not sent to (models.mjs).
const takesAttachments = attachmentReader(registry, modelSupportsAttachments);

// A key reaches Mastra through this process's environment, and only the app puts one there
// (`_intcad/connections/setup`). One that happens to be in the environment the app was started from
// is not the user's choice here, so it is taken out: every provider's, not only the five the page
// offers, or Mastra would serve models on a key no row of the page shows.
const keyNames = keyVariables(registry);
for (const [id] of KEYS) for (const name of keyEnv(id)) keyNames.add(name);
for (const name of keyNames) delete process.env[name];
// And the keys the app does put there stay out of the commands a model runs.
keepKeysFromCommands(LocalSandbox, keyNames);
// And the file tools stay out of `/tmp`, which every program of the computer shares, and out of the
// session's own folder, whose agent files Mastra Code reads on every turn (fence.mjs).
keepFilesFrom(LocalFilesystem, deniedFolders(tmpdir()));

// The browser is the user's own: it is started with their home folder, not the agent's, so it is
// the browser they use and the profile they are signed in with. (The opener starts it before it
// returns, so the agent's home is back before anything of Mastra's runs again.)
const setHome = home => {
  for (const [name, value] of Object.entries(home)) {
    if (value === undefined) delete process.env[name];
    else process.env[name] = value;
  }
};
const openUrl = url => {
  const agentsHome = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE };
  setHome(usersHome);
  try {
    openUrlInBrowser(url);
  } finally {
    setHome(agentsHome);
  }
};

let agent = null;
const connections = new Connections({
  auth: () => gateway.getGlobalAuthStorage(),
  providers: () => storage.getOAuthProviders().map(provider => ({ id: provider.id, name: provider.name })),
  keyEnv,
  env: process.env,
  // Mastra counts an address without a key as not connected. One that needs none (a model on this
  // computer) is given a word in the key's place, which such a service ignores.
  setCustom: list => customSource.setCustomProvidersSource(() => list.map(entry => ({ ...entry, apiKey: entry.apiKey || 'none' }))),
  customPrefix: id => `${gateway.MASTRACODE_GATEWAY_ID}/${id}`,
  openUrl,
  holdPorts: onCallback => holdCallbackPorts(http, onCallback, { sentence: () => connections.callbackSentence() }),
  dates: () => dates,
  changed: snapshot => {
    agent?.connection.extNotification('_intcad/connections/changed', snapshot).catch(error => process.stderr.write(`[intcad] could not tell the app: ${error}\n`));
  },
  refresh: async why => {
    await agent?.refreshSessions(why);
  },
});
// Until the app hands over its custom addresses there are none: Mastra's own settings file is not read for them.
customSource.setCustomProvidersSource(() => []);

// Sessions as Mastra Code's ACP server opens them, without a vector store (session.mjs).
const createSession = cad => sessionBuilder({
  createMastraCode,
  loadSettings: settings.loadSettings,
  resolveDefaultThinkingLevel: settings.resolveDefaultThinkingLevel,
  resolveSubagentsSetting,
  withCleanupFailure,
  RequestError,
  cad,
});

// As Mastra Code's own `runAcpServer`, with the class above and the sign-in ended on the way out.
let disposing;
const dispose = () => (disposing ??= Promise.resolve().then(() => {
  connections.dispose();
  return agent?.dispose();
}));
const stop = () => dispose().then(() => process.exit(0), error => {
  process.stderr.write(`[acp] Shutdown failed: ${error}\n`);
  process.exit(1);
});
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
try {
  await new AgentSideConnection(connection => {
    const cad = cadBridge({ createTool, jsonSchema, RequestContext, errors: cadErrors, invoke: (method, params) => connection.extMethod(method, params) });
    const IntcadAgent = intcadAgent({ Base: MastraCodeAcpAgent, RequestError, connections, dates: () => dates, reloadSignIns: () => gateway.reloadAuthStorage(), cad, takesAttachments });
    agent = new IntcadAgent(connection, createSession(cad));
    return agent;
  }, ndJsonStream(Writable.toWeb(process.stdout), Readable.toWeb(process.stdin))).closed;
} finally {
  await dispose();
}
