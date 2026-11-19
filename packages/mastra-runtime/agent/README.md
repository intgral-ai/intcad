# The built-in agent

`main.mjs` is the program the app starts as its built-in agent. It is Mastra Code's ACP agent, from
its SDK and unchanged, with what the Assistant's Connections page needs added in this folder:

- `connections.mjs`: which plans are signed in, which keys and custom addresses the app handed
  over, the one sign-in that may be under way, and the methods the page's buttons end in;
- `callback.mjs`: how the ChatGPT browser sign-in waits for the browser without a time limit;
- `agent.mjs`: the methods on the ACP agent, a model list a person can read, sessions that follow a
  sign-in or sign-out, and the reason on a turn that fails;
- `models.mjs`, `problems.mjs`: the order of the model list, and the kinds of failure;
- `secrets.mjs`: what keeps a key out of the commands a model runs and out of what is printed;
- `session.mjs`: the session a `session/new` opens. It is Mastra Code's own ACP session builder
  (Apache-2.0), adapted so that the session has no vector store: nothing is embedded, and no
  embedding model is needed. `../agent-package/adapted-from.json` names the SDK file it follows;
  when a new SDK changes that file, the folder's build fails until the two are compared again.

The tests are `../test/connections.test.ts`. They run these files against stand-ins: no test signs
in anywhere, opens a browser or reads a key.

This folder is not part of the runtime artifact that `scripts/runtime-artifact.mjs` builds and
checks; putting the agent into the installed package is a later change.

## Running it

It needs Node 22.19 or later and Mastra Code's SDK. It was written against:

| Package | Version |
| -- | -- |
| `@mastra/code-sdk` | 1.19.1 |
| `@agentclientprotocol/sdk` | 0.21.1 |

The SDK is not installed by this repository. `SOLIDCRAFT_MASTRA_SDK` names the `node_modules`
folder that holds the two packages; the app is told where the agent is with
`SOLIDCRAFT_MASTRA_AGENT` (the path of an executable that starts `main.mjs`).

```sh
mkdir sdk && cd sdk && npm init -y
npm install @mastra/code-sdk@1.19.1 @agentclientprotocol/sdk@0.21.1
SOLIDCRAFT_MASTRA_SDK="$PWD/node_modules" node <repo>/packages/mastra-runtime/agent/main.mjs
```

The SDK's files are loaded by their full path under that folder, so nothing depends on where this
folder is relative to it.

## What it adds to ACP

Extension methods, all from the app to the agent. Each answers with the snapshot, and the agent
sends the same snapshot as the notification `_intcad/connections/changed` whenever it changes.

| Method | Params | What it does |
| -- | -- | -- |
| `_intcad/connections/setup` | `{ keys: { <key id>: <key> }, custom: [{ name, url, apiKey?, models }], words?: { callback } }` | Replaces every key and custom address, and reads Mastra's sign-ins from its file again. A custom address that cannot be used is left out, the rest is taken. `words.callback` is the sentence the browser shows after the ChatGPT sign-in page has called back. |
| `_intcad/connections/list` | `{}` | Nothing; answers with the snapshot. |
| `_intcad/connections/login` | `{ id }` | Starts a plan sign-in in the background. Error `-32000` `busy` if one is under way. |
| `_intcad/connections/code` | `{ id, code }` | Hands over the code the browser showed (Claude). With no sign-in waiting for one it changes nothing and is not an error. |
| `_intcad/connections/reopen` | `{}` | Opens the sign-in address in the browser again. |
| `_intcad/connections/cancel` | `{}` | Ends the sign-in under way. |
| `_intcad/connections/logout` | `{ id }` | Signs out of a plan. |

```json
{
  "plans":  [{ "id": "openai-codex", "name": "ChatGPT Plus/Pro (Codex Subscription)", "state": "in", "account": "a@example.com", "prefix": "openai" }],
  "keys":   [{ "id": "anthropic", "name": "Anthropic", "saved": false, "shadowed": false, "prefix": "anthropic" }],
  "custom": [{ "id": "my-llm", "name": "my-llm", "url": "http://localhost:11434/v1", "models": ["llama3.2"], "keyed": false, "prefix": "mastracode/my-llm" }],
  "login":   { "id": "anthropic", "phase": "code", "url": "https://…", "userCode": null, "error": null },
  "problem": null,
  "dates":   { "openai/gpt-6.2-sol": "2026-11-17" }
}
```

- `plans`: `openai-codex`, `anthropic`, `kimi-for-coding`, `xai`, `github-copilot`, those of them the
  SDK knows. `state` is `in`, `out`, or `expired` (a turn was refused for lack of a valid sign-in;
  it stays until the next sign-in or sign-out).
- `keys`: `anthropic`, `openai`, `google`, `openrouter`, `deepseek`. `shadowed`: a signed-in plan
  serves the same models, and the plan is what Mastra uses.
- `prefix`: what the ids of that connection's models start with, followed by `/`. A custom
  address's has two parts.
- `custom[].id`: formed from the name (lower case, a hyphen for every run of anything that is not a
  Latin letter or a digit; `custom` when nothing is left). Two names that give the same id are both
  kept, the later with `-2`, `-3` … after it. Mastra is handed the id as the address's name, so its
  model ids are `mastracode/<id>/<model>`.
- `login.phase`: `starting`; `browser` (the address is open, nothing to type); `device` (`userCode`
  is typed on that page); `code` (the page will show a code that is pasted back). `login.error` is
  `incomplete` after a pasted code that has not the shape of one.
- `problem.kind`, for a sign-in that could not finish: `port`, `offline`, `failed`.
- `dates`: release dates of the models of connected providers, from Mastra's registry.

A session's `model` option lists only models one can talk to, newest first. After a sign-in, a
sign-out or `setup`, every open session gets a `config_option_update` with its new list, and is moved
to another model if the one it used is gone. A new session starts on a model that something
connected can serve.

A `session/prompt` that fails carries
`data: { "intcad": { "kind": "auth" | "quota" | "rate" | "network" | "model" | "other", "prefix" } }`.

## Keys

A key reaches Mastra through this process's environment and nothing else: `setup` sets the
variable Mastra reads for that provider, and no key is written to a file. Mastra's own key store
(a plain file in its data folder) is never used. When the agent starts it removes those variables
from the environment it inherited, so a key that happens to be set where the app was started is
not used behind the user's back.

Three more things follow from the key being in the environment:

- Mastra Code starts the commands a model runs (its command tool) with a copy of the agent's
  environment. `secrets.mjs` takes every provider's key variable out of that copy, so a command
  such as `env` shows none. If Mastra's sandbox is not the one this was written for, the agent
  does not start, where it would otherwise start unguarded.
- The variables removed at start are every provider's that Mastra knows, not only the five the
  page offers: a key for another provider in the environment the app was started from would
  otherwise serve models with no row on the page.
- ACP's SDK prints a request it answers with an error, and the app keeps the agent's last lines.
  The parameters of a request are not printed (`params: '[not shown]'`): they can be a key, a
  pasted code, or what the user wrote.

Plan sign-ins are different: Mastra keeps them itself, in `auth.json` in its data folder.

A custom address without a key is given the word `none` in the key's place, because Mastra counts
an address without a key as not connected; a service that needs no key ignores it.

## What it does not do by itself

The entry sets Mastra's own switch `MASTRA_TELEMETRY_DISABLED` before anything of Mastra's is
loaded, so no usage is reported.

## What it keeps, and where

The app names a data folder with `MASTRA_APP_DATA_DIR`. Mastra keeps its plan sign-ins and its
own data there, apart from a Mastra Code the user may have installed. The agent also takes a home
folder of its own inside it (`home/`): Mastra Code reads a person's own agent files from their
home folder (instructions for agents, MCP servers, its own settings), and none of that is meant for
Intcad's assistant or for the model it talks to. Without `MASTRA_APP_DATA_DIR` (run by hand)
nothing of this is changed. The browser is the exception: it is started with the user's own home
folder, so it is the browser they use, with the profile they are signed in with.

## The ChatGPT sign-in does not time out

That sign-in ends with the browser calling a local address (port 1455, or 1457). Mastra Code's own
listener for the call gives up after 60 seconds, and a call that arrives later is lost. Intcad
waits until the user has signed in or cancels, without changing Mastra's files:

1. `callback.mjs` takes both ports before the sign-in starts.
2. Mastra Code, finding both taken, still builds the sign-in address (pointing at 1457) and then
   waits, without a limit, for the address the browser was sent back to, as pasted by hand.
3. The listener receives the browser's call and hands that address over. Only the call that
   carries the `state` Mastra put into the sign-in address is taken; any other request to the
   ports is refused and the wait goes on, and none can end the agent.

The page the browser then shows sends the user back to the app. It does not say "signed in":
the code is exchanged after it.

`cancel`, the end of the sign-in, or the end of the process frees the ports. If another program
holds 1457, the sign-in cannot be received and fails with `problem.kind` `port`.

## What was checked, and what was not

Run for real with the SDK above, an empty home folder, a stand-in model service and a stand-in for
the browser opener: start, the snapshot, a custom address, a session, a streamed answer, keys in
memory with the session's list updated, each kind of failed turn, the ChatGPT sign-in still waiting
after 70 seconds, cancel freeing the ports, and the first step of the other sign-ins. Also: with
two keys handed over and two more in the environment the agent was started from, the stand-in
model had the agent run `env` (one permission request, allowed), and neither the command's
output nor the request that carried it back held any of them; a refused request was printed
without its parameters; a request the listener cannot read and a call with another `state` were
each answered 400 with the sign-in still waiting.

The sign-in address points at port 1457 where Mastra Code's own points at 1455. Asked without
signing in (2026-11-18), OpenAI's sign-in service treats the two alike (it goes on to its sign-in
page) and refuses an address with any other port.

Not checked: any sign-in completed with a real account through this agent, a turn answered by a
real provider, and the Kimi plan's model prefix.
