# The built-in agent

`main.mjs` is the program the app starts as its built-in agent. It is Mastra Code's ACP agent, from
its SDK and unchanged, with what the Assistant's Connections page needs added in this folder:

- `connections.mjs`: which plans are signed in, which keys and custom addresses the app handed
  over, the one sign-in that may be under way, and the methods the page's buttons end in;
- `callback.mjs`: how the ChatGPT browser sign-in waits for the browser beyond Mastra Code's 60
  seconds, and whether an Intcad holds its port when it is taken;
- `agent.mjs`: the methods on the ACP agent, a model list a person can read, sessions that follow a
  sign-in or sign-out, and the reason on a turn that fails;
- `models.mjs`, `problems.mjs`: the order of the model list, and the kinds of failure;
- `secrets.mjs`: what keeps a key out of the commands a model runs and out of what is printed
  or logged;
- `signin.mjs`: what a failed sign-in leaves to go by: what the ChatGPT sign-in's token endpoint
  answered, and the sign-in log;
- `skills.mjs`: the skill sets the package carries (`modelling-skills`: the repository's
  `skills/solidcraft-modeling`, `-editing`, `-checking` and `-mechanisms`; `context-skills`:
  `../agent-skills/intcad-context`, written for this agent alone), how the build copies
  them with a stamp, and how the entry puts them in `home/.agents/skills` at start, where Mastra
  Code looks for skills (`docs/mastra-packaging.md`, "The skills, at start");
- `session.mjs`: the session a `session/new` opens. It is Mastra Code's own ACP session builder
  (Apache-2.0), adapted so that the session has no vector store: nothing is embedded, and no
  embedding model is needed. `../agent-package/adapted-from.json` names the SDK file it follows;
  when a new SDK changes that file, the folder's build fails until the two are compared again.
  It also hands Mastra what the app sends with a session: its note, and a project (below);
- `brief.mjs`: the project brief, the list of the project's files a session on a project is told;
- `reads.mjs`: what runs without asking the user (below), and the connection that answers itself
  the permission requests of the app's commands that only read;
- `plan.mjs`: plan first, the first change of a reply waiting for a plan the user has seen (below).

The tests are `../test/connections.test.ts`, `../test/skills.test.ts`, `../test/brief.test.ts`,
`../test/reads.test.ts` and `../test/plan.test.ts`. They run these files
against stand-ins: no test signs in anywhere, opens a browser or reads a key.

This folder is not part of the runtime artifact that `scripts/runtime-artifact.mjs` builds and
checks. A test package can carry the agent in a folder of its own, `mastra-agent`, built by
`scripts/agent-package.mjs`: see `docs/mastra-packaging.md`.

## Running it

It needs Node 22.19 or later and Mastra Code's SDK. It was written against:

| Package | Version |
| -- | -- |
| `@mastra/code-sdk` | 1.19.1 |
| `@agentclientprotocol/sdk` | 0.21.1 |

The two packages are pinned, with a lock file, in `../agent-package/`; nothing installs them
into this checkout. `SOLIDCRAFT_MASTRA_SDK` names the `node_modules` folder that holds them; the
app is told where the agent is with `SOLIDCRAFT_MASTRA_AGENT` (the path of an executable that
starts `main.mjs`), or finds it in the package it runs from.

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
- `problem.kind`, for a sign-in that could not finish: `port` (with `holder` when it is Intcad's:
  `{ kind: 'here' }` a sign-in of another design of the same app, `{ kind: 'intcad' }` one of
  another Intcad; none for another program), `listen` (the system would not open the port: `port`,
  `code`, such as `EACCES`), `timeout` (15 minutes passed), `offline`, `failed`, and for the
  ChatGPT sign-in's token endpoint `refused`, `region`, `plan` (the endpoint's answer names the
  region or the plan). `step`: how far it got: `open` (before the browser is opened), `wait`,
  `exchange` (with the token endpoint: its `status` and its error `code`, or for `offline` the
  system's code, `timeout` or `no-connection`), `store` (saving it: the file error's `code`).
  `log`: the path of the sign-in log, when the agent keeps one (below).
- `dates`: release dates of the models of connected providers, from Mastra's registry.

A session's `model` option lists only models one can talk to, newest first. After a sign-in, a
sign-out or `setup`, every open session gets a `config_option_update` with its new list. Only a
sign-out moves a session to another model when the one it used is gone. After a sign-in or a
`setup` the session keeps its model, which is then not in its list (Mastra Code would list it), until
the user picks one: the newest model may be one paid for by use, and nobody chose it. A
`session/prompt` on a model nothing connected serves is refused with kind `model` before any provider
is asked; its `prefix` is empty when the model's connection was a custom address that is gone. A new
session starts on a model that something connected can serve.

A `session/prompt` that fails carries
`data: { "intcad": { "kind": "auth" | "quota" | "rate" | "network" | "model" | "attachments" | "other", "prefix" } }`.

`initialize` declares `promptCapabilities.image` and `embeddedContext`, so the app sends a message's
pictures (`image` blocks) and PDFs (`resource` blocks with a `blob`). Mastra Code's own prompt handler
refuses both; `agent.mjs` takes them out and hands them to the session's `sendMessage` as files of
that turn's message (INT-1513). Just before submission, it checks the effective model and adapter
against the message and any attachment types the SDK may retain from earlier turns, even on a
text-only follow-up. A confirmed SDK cancellation before admission leaves no attachment history.
DeepSeek and xAI's OAuth chat adapter cannot take PDFs. On signed-in Copilot, a model listed by
the SDK must advertise vision to receive pictures. The SDK reports both explicit `false` and
omitted upstream vision as `false`. An unlisted model, a failed catalog lookup, or no sign-in
leaves the decision to the provider. Vision support does not establish PDF support.
An incompatible turn is refused with kind `attachments` before a provider is asked. Stop also
applies during model refresh or while a prompt is waiting in the SDK queue.

The desktop preflights the complete encoded prompt, including the JSON-RPC envelope and newline.
A built-in prompt is limited to 18 MiB, enough for one 12 MiB PDF and ordinary text. It is refused
locally if it does not fit; agent-to-desktop limits do not change.

Two more, for the app's guesses of the user's next step (2026-11-26, G87 to G90 of
`docs/design/autocomplete-tests.md`):

- `session/close`, offered as `agentCapabilities.sessionCapabilities.close`. Mastra Code's ACP agent
  has none, and keeps each session (a whole runtime, with its storage, workers and folder) until the
  connection closes. A closed session is ended as every session is then: its turn cancelled and
  aborted, its listeners removed, its runtime cleaned up. A prompt or a second close on it is
  refused, and so is a close of a session that is not open. The app closes only guessing sessions.
- A `session/new` with `_meta.intcad.nextStep: true` (only `true` counts; `isNextStep`, `session.mjs`)
  is a guess. It gets the app's note alone (no `LANGUAGE`, no brief, no `NO_PROJECT`, and no project
  even if one is sent), no MCP at all (`disableMcp`; the servers the request names are not started),
  no design tools (a `cadBridge` catalog is ignored), and no per-session plan tracking. An input
  processor clears model-visible tools at every step. Before the first guess the installed Core
  must also pass an offline stream check with a stub model; otherwise the session is refused.
  It is put in Mastra Code's read-only mode `fast` (Explore) before the reply as a second safeguard.
  An agent without that mode refuses the session and ends it.

### The note and the project (INT-1473)

`session/new` may carry two things of the app's own:

| Field | What the agent does with it |
| -- | -- |
| `_meta.systemPrompt.append` (or the standard `systemPrompt`) | The app's note. It becomes Mastra Code's host instructions, after its base prompt. |
| `_meta.intcad.project` | A project folder: the Files panel's folder, sent only when it is the shown design's project. It must be an absolute path to an existing folder, and not the root of a drive; anything else is no project. |

Every session, with a project or without, works in a new, empty folder of its own,
`<data>/sessions/s-<pid>-…` (`sessionFolder` in `session.mjs`), removed when the session is
cleaned up, that is when the app closes its connection to the agent (New chat leaves the old
session open), or the session itself (`session/close`, sent only for guesses of the next step). A start removes the folders of an agent process that is no longer running
(`sweepSessionFolders`). The `cwd` the app names is used only for the MCP servers it hands over.
Mastra Code reads agent files (`AGENTS.md`, `CLAUDE.md`, on every turn), MCP servers (`.mcp.json`,
`.claude/settings.local.json`) and skills from a session's folder: in one folder shared by every
session, a file put there would be loaded by every later session, and the sessions would share
one memory, so `recall` in one would read another's. The file tools do not reach the session's
folder at all (`fence.mjs`), so the model cannot write its own instructions there either. Hooks
are off (`disableHooks`): Intcad configures none. Git does not look for a repository above
`sessions/` (`GIT_CEILING_DIRECTORIES`), so a data folder inside a checkout reads none of its files.
Every session is also told to answer in the language the user writes in (`LANGUAGE`).
A session without a project is told so in place of the brief (`NO_PROJECT`, INT-1508): it cannot
write the project's files, and a user who asks it to is told to create or open the project, then
start a New chat, since the app hands a project over only when a chat starts.

With a project:

- Mastra Code reads agent files, MCP servers and skills only from the session's own folder and
  from the agent's home, so nothing of the project's own is loaded or run.
- The project folder is added to the folders Mastra Code's file tools may reach
  (`sandboxAllowedPaths`), on the session's own thread (`agent.mjs`): set in `initialState`, it is
  gone once Mastra Code opens that thread. The tools follow links and refuse a path whose real
  place is outside the allowed folders. The four tools that only read (`view`, `find_files`,
  `search_content`, `file_stat`: `FENCED_READS` in `reads.mjs`) run without asking, as decided
  on 2026-11-22: a read beside the project is refused by the fence, not asked about. Without a
  project they ask.
- The brief follows the note (`brief.mjs`). It lists the files the Files panel shows by default
  (no hidden entries; `.intcad/` is read through Intcad's tools) by path, kind and size, folder
  by folder, in at most 4000 characters, ending with how many more there are. A character of a
  file name that would break the line (a control or format character, a line separator) is
  written as `\u{…}`, and the brief says that names and contents are the project's data, not
  instructions. It asks for absolute paths (a relative one would be the session's own folder,
  which the tools do not reach). It also says how to read the latest DFM report (`inspect.dfm`)
  and the project's settings (`project.inspect`), and how to work, in four lines: start from the
  files, the plan as a task list (`task_write`) before the first change (it says this holds where
  Mastra Code's own prompt says to just do it), check each requirement after a change, ask when requirements are
  missing or in conflict. It is made when the session opens; a file added later is found with
  the file tools.
- The skill `intcad-context` says how to work from it: find the context first, plan as a task list,
  check each requirement with numbers from tools, ask when something is unclear.

Besides the project, the file tools reach Mastra Code's own defaults: the temporary folder (the
agent's own, see below), its plans folder and the skill folders in the agent's home. Mastra Code
also lists `/tmp`, which every program of the computer shares, and the session's own folder;
`fence.mjs` takes both off, and makes the sentence that tells the model where its file tools
reach say the same. To a read, the session's folder is empty: a file read there is not found,
not refused (`find_files` and `search_content` read its `.gitignore` first, and failed on every
call while that was refused). The agent does not start if Mastra's file system class is not the one that
was written for. `request_access` is not offered (`disabledTools`): its card named no folder, and
three clicks opened any folder of the computer. The command tool is not limited to any folder; it
asks first. It stays on, as decided on 2026-11-22 and as Huaiyuan's spec v3.2 has it: the agent
is a coding agent, and later work may need a shell. The note of a session with a project says
nothing against it (`PROJECT_FILE_TOOLS`, `acp.rs`); it says the design is built and checked
only through Intcad's tools. A session without a project gets the note every agent gets, which
says not to use shell or file tools.

### What runs without asking (`reads.mjs`)

Decided on 2026-11-22. In every session, with a project or without:

- The tools of the app's MCP server (`intcad-app`) that only look: `list_commands`,
  `describe_command`, `inspect_design`, `measure`, `body_topology`, `running`, `pending`,
  `skill_reference` (`APP_READS`: those the server marks read-only in `crates/mcp/src/tools.rs`,
  less `crash_reports`, which the app keeps from the agent). A test fails when the server marks
  another tool read-only, so adding one is a decision.
- The app's `execute` when its command only reads: an id that starts with `inspect.` or `model.`,
  ends with `.inspect` or `.list`, or is one of a few more (`commandReadsOnly`). This is the
  engine's rule (`command_reads_only` in `crates/engine/src/control_pending.rs`), plus
  `view.pick`; case is ignored, as the engine ignores it. Mastra decides whether to ask by a
  tool's name alone, so the agent answers these permission requests itself (`allow once`) and
  the app is not asked; it still shows the call (`askingOnlyWhenNeeded`, `agent.mjs`).

The app's gate (`crates/ui-egui/src/agent_chat/gate.rs`) still checks every request these tools
send, and asks in the panel before one that names a file. Everything else asks the user first: a
write, a command, `lsp_inspect`, the web tools (a fetch can carry what was read to anywhere), and
the app's tools and commands that change the design or write files (`batch` too, even when every
step only reads).

### Plan first (`plan.mjs`)

Decided on 2026-11-22 (INT-1473 B2). In a chat on a project, the first change of a reply waits until
the user has been shown a plan, and the plan is the agent's task list. Asked in words, in the brief
and in the skill, the model never wrote a plan: 0 of 10 real runs of "fix this part" showed one
before the first change. Mastra Code's own prompt says "Just do it" and "Don't announce what you're
about to do", and the model planned in its thinking, which the user never sees. Held and asked for
a written plan, it planned in its thinking again, 4 to 11 times a run, before writing one.

- The plan is Mastra Code's task list (`task_write`), which its own prompt asks for on a task of
  several steps. The agent sends each `task_updated` event, which Mastra Code's ACP agent leaves
  out, to the app as an ACP `plan` update: the app shows it as the reply's plan, a dot per step
  that fills as the step is under way and done. The task tools run without a card (`TASK_TOOLS`,
  `reads.mjs`): they change only that list. A numbered line in what the agent says is no plan: a
  numbered answer (the requirements as read) would let the next change through unplanned.
- A change is a call of the app's tools that changes the design or writes a file (`execute` with a
  command that does not only read, `batch`, `set_parameter`, `undo`, `redo`, `new_design`, `open`,
  `save`, `export`), or one of Mastra's file writes. Reads, screenshots, the shell, web tools,
  skills and the task tools are not held.
- A plan counts from the user's last message on. A plan the previous reply ended on, with no change
  after it, carries over to the next message, so "plan, then the user's OK, then the change" needs
  no second plan. A single change needs one task.
- A change before a plan is declined by the agent itself (the user is not asked) with
  `PLAN_FIRST` as the reason, which the model reads in place of Mastra's "not approved by the
  user": record the plan with `task_write`, then make the call again. The reason goes with the
  decline through Mastra's `declineContext` (`agent.mjs` wraps the session's
  `respondToToolApproval`); where that is not there, nothing is held.

Known and left as they are:
- A new skill the model writes into the agent's home (it asks first) is offered to later
  sessions: a start puts the packaged sets back and leaves other folders alone.
- A session keeps the project it was opened with when the user then opens another design or
  folder, until New chat.
- Mastra checks a write's path before it writes, so a link swapped in between by another program
  could redirect it.
- An Intcad tool call that changes the design asks the user first, and the card names the tool
  (`intcad-app_execute`), not what it does. The app lets other agents' calls of its tools through
  and leaves them to its gate (`is_bridge_tool`, `acp.rs`), but it does not recognise this
  agent's names for them (`intcad-app_<tool>`, not `mcp__intcad-app__<tool>`).
- A PDF that is not valid, once read (the model may read one without asking), makes every later turn of that conversation fail on the
  provider's side, until New chat: Mastra hands the file to the model as it is.

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

The entry sets two of Mastra's own switches before anything of Mastra's is loaded:
`MASTRA_TELEMETRY_DISABLED`, so no usage is reported, and `MASTRA_OFFLINE`, so Mastra's
five-minute refresh of the providers' lists, which writes into its cache and into the SDK's own
folder, does not run. What still goes out unasked: when a session opens, Mastra Code reads two
public catalogs into memory (`https://models.dev/api.json` and
`https://api.netlify.com/api/v1/ai-gateway/providers`), which is where the models a session offers
come from. No switch for that was found, and none is set.

## What it keeps, and where

The app names a data folder with `MASTRA_APP_DATA_DIR`. Mastra keeps its plan sign-ins and its
own data there, apart from a Mastra Code the user may have installed. The agent also takes a home
folder of its own inside it (`home/`): Mastra Code reads a person's own agent files from their
home folder (instructions for agents, MCP servers, its own settings), and none of that is meant for
Intcad's assistant or for the model it talks to. The lease files Mastra Code keeps for its
sessions, which it would put under `/tmp/mc` for every Mastra Code of the user, go to `signals/`
in the same folder (`MASTRACODE_SIGNALS_SOCKET_ROOT`). Its temporary folder is `tmp/` there too
(`TMPDIR`, `TMP`, `TEMP`): Mastra Code's file tools reach the temporary folder by themselves, and
the user's is not theirs to reach. The skill sets go to `home/.agents/skills`,
one folder per skill and a note per set, before Mastra is loaded (`skills.mjs`). Without
`MASTRA_APP_DATA_DIR` (run by hand) nothing of this is changed, and no skill is installed. The browser is the exception: it is started with the user's own home
folder, so it is the browser they use, with the profile they are signed in with.

## The ChatGPT sign-in waits for the browser

That sign-in ends with the browser calling a local address (port 1455, or 1457). Mastra Code's own
listener for the call gives up after 60 seconds, and a call that arrives later is lost. Intcad
waits until the user has signed in or cancels, or 15 minutes have passed (every sign-in has that
limit: one left open once held the ports for 16 hours), without changing Mastra's files:

1. `callback.mjs` takes both ports before the sign-in starts.
2. Mastra Code, finding both taken, still builds the sign-in address (pointing at 1457) and then
   waits, without a limit, for the address the browser was sent back to, as pasted by hand.
3. The listener receives the browser's call and hands that address over. Only the call that
   carries the `state` Mastra put into the sign-in address is taken; any other request to the
   ports is refused and the wait goes on, and none can end the agent.

The page the browser then shows sends the user back to the app. It does not say "signed in":
the code is exchanged after it.

`cancel`, the end of the sign-in, its time limit, or the end of the process frees the ports. The
app cancels a sign-in when its design is left and, before a sign-in starts, any still under way in
another design; the page cancels one when the user goes back from it. If 1457 is taken, the
sign-in cannot be received and fails with `problem.kind` `port`. Who holds it is asked first at
`/intcad-sign-in`, where Intcad's own listener answers `{ app: 'intcad', parent }` (the agent's
parent process, the app); one of the same app is waited for up to two seconds, since its cancel
may be on the way. Another program is not named. The first port may be refused by the system as well: what keeps this
process off it keeps Mastra Code off it too.

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

A ChatGPT sign-in with a real account was completed on 2026-11-27 on macOS in a copy built
locally from `develop` (seen by its sign-in file, not by this folder's tests). Not checked here: a
turn answered by a real provider, and the Kimi plan's model prefix.

## What a sign-in leaves to go by

Mastra Code's SDK keeps nothing of a failed exchange: it prints the HTTP status and throws "Token
exchange failed". The entry wraps `fetch` for the ChatGPT sign-in's token endpoint only
(`signin.mjs`): the answer's status, and for a refusal the provider's own error code (a short word
from the JSON, never its description) and whether it names the region or the plan, are handed to
the sign-in under way; the body of an answer that was taken is never read. With
`MASTRA_APP_DATA_DIR` set, each sign-in writes `logs/sign-in.log` there: the Node version and the
names of the proxy variables it was given, each step, what the endpoint answered, and every line
the agent printed meanwhile. Each line goes through `scrub` (`secrets.mjs`) first: the keys the
agent holds, every value in an address's query, a user and password in an address, token fields,
JWTs, e-mail addresses and any other run of 32 or more letters and digits are taken out. The file
starts over past 256 KiB.

The agent's requests follow a proxy only as Node does: with `NODE_USE_ENV_PROXY=1`, `HTTPS_PROXY`
or `HTTP_PROXY`, and `NO_PROXY`. The app passes the first and adds this computer's names to the
last when its own environment names a proxy (`proxy_env` in `acp_client.rs`). The operating
system's own proxy setting is not read.
