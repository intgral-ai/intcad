#!/usr/bin/env node
// Intcad's built-in agent. The app starts this file and talks ACP with it over stdin and stdout.
//
// It is Mastra Code's ACP agent (its SDK, unchanged) with the parts in this folder added. Where the
// SDK is installed is not this file's business: SOLIDCRAFT_MASTRA_SDK names the `node_modules`
// folder that holds `@mastra/code-sdk` and `@agentclientprotocol/sdk`; without it the folder beside
// this one is used. README.md says which versions this was written against.

import http from 'node:http';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { Readable, Writable } from 'node:stream';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { intcadAgent } from './agent.mjs';
import { holdCallbackPorts } from './callback.mjs';
import { Connections, KEYS } from './connections.mjs';
import { releaseDates } from './models.mjs';

// Mastra Code is a coding agent: it reads a person's own agent files from their home folder (their
// instructions for agents, their MCP servers, its own settings there). None of that is meant for
// Intcad's assistant or for the model it talks to, so when the app names a data folder the agent
// gets a home of its own inside it. This has to happen before anything of Mastra's is loaded.
if (process.env.MASTRA_APP_DATA_DIR) {
  const home = join(process.env.MASTRA_APP_DATA_DIR, 'home');
  mkdirSync(home, { recursive: true });
  process.env.HOME = home;
  process.env.USERPROFILE = home;
}

const here = dirname(fileURLToPath(import.meta.url));
const sdkRoot = resolve(process.env.SOLIDCRAFT_MASTRA_SDK || join(here, '..', 'node_modules'));
// The files are named in full: an absolute path does not go through a package's export map, and a
// bare name would be looked up from this folder, where the SDK need not be.
const load = path => import(pathToFileURL(join(sdkRoot, path)).href);

if (!existsSync(join(sdkRoot, '@mastra', 'code-sdk', 'package.json'))) {
  process.stderr.write(`[intcad] Mastra Code's SDK is not in ${sdkRoot}. Set SOLIDCRAFT_MASTRA_SDK to the node_modules folder that holds it.\n`);
  process.exit(1);
}

const [{ AgentSideConnection, RequestError, ndJsonStream }, { MastraCodeAcpAgent }, { createAcpSession }, storage, gateway, customSource, { openUrlInBrowser }] =
  await Promise.all([
    load('@agentclientprotocol/sdk/dist/acp.js'),
    load('@mastra/code-sdk/dist/acp/agent.js'),
    load('@mastra/code-sdk/dist/acp/runtime.js'),
    load('@mastra/code-sdk/dist/auth/storage.js'),
    load('@mastra/code-sdk/dist/agents/mastracode-gateway.js'),
    load('@mastra/code-sdk/dist/agents/custom-provider-source.js'),
    load('@mastra/code-sdk/dist/utils/open-url.js'),
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

// A key reaches Mastra through this process's environment, and only the app puts one there
// (`_intcad/connections/setup`). One that happens to be in the environment the app was started from
// is not the user's choice here, so it is taken out.
for (const [id] of KEYS) for (const name of keyEnv(id)) delete process.env[name];

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
  openUrl: url => openUrlInBrowser(url),
  holdPorts: onCallback => holdCallbackPorts(http, onCallback),
  dates: () => dates,
  changed: snapshot => {
    agent?.connection.extNotification('_intcad/connections/changed', snapshot).catch(error => process.stderr.write(`[intcad] could not tell the app: ${error}\n`));
  },
  refresh: async () => {
    await agent?.refreshSessions();
  },
});
// Until the app hands over its custom addresses there are none: Mastra's own settings file is not read for them.
customSource.setCustomProvidersSource(() => []);

const IntcadAgent = intcadAgent({ Base: MastraCodeAcpAgent, RequestError, connections, dates: () => dates, reloadSignIns: () => gateway.reloadAuthStorage() });

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
    agent = new IntcadAgent(connection, request => createAcpSession(request));
    return agent;
  }, ndJsonStream(Writable.toWeb(process.stdout), Readable.toWeb(process.stdin))).closed;
} finally {
  await dispose();
}
