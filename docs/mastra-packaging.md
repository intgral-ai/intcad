# Packaged Mastra runtime

Status: **the workflow runtime (`mastra/`) is packaging scaffolding only — the artifact is NOT
approved, and bundling it stays opt-in and refused. The built-in agent (`mastra-agent/`) is what
a release carries; see "The built-in agent in release packages".** Four shipped `@mastra/core/dist/ee-*` files carry the Mastra Enterprise License (DQ-7),
so `policy.approvals` is empty and the admission gate fails closed. Ordinary release packaging
leaves the workflow out unless `--with-mastra` / `SOLIDCRAFT_WITH_MASTRA=1` is passed; the owner
must decide that release default (DQ-7/DQ-4) before a package may include it. No installed-package,
signature or clean-machine acceptance is claimed here. Legal review in #15, the fork checks in #17
and real target machines are prerequisites for shipping this directory inside a release.

## Installed layout

Every supported package carries an application-relative `mastra/` directory. The host resolves it
from the running program, never from the development checkout, `PATH`, `npx` or the user's Node.

| Package | Installed location | Resolved from |
|---|---|---|
| Linux `.deb`, `.rpm`, `.tar.gz`, AppImage, snap | `/usr/lib/intcad/mastra` (inside the image/sandbox: the same FHS path) | `usr/bin/intcad` → `../lib/intcad/mastra` |
| Flatpak | `/app/lib/intcad/mastra` | `/app/bin/intcad` → `../lib/intcad/mastra` |
| macOS `.app` (single architecture) | `Intcad.app/Contents/Resources/mastra` | `Contents/MacOS/Intcad` → `../Resources/mastra` |
| macOS `.app` (universal) | `Contents/Resources/mastra/aarch64-apple-darwin` and `.../x86_64-apple-darwin` | the matching slice by host target |
| Windows MSI and portable zip | `mastra\` beside `intcad.exe` | beside the executable |

The directory contains `runtime.json`, `dist/src/main.js`, production `node_modules/`, the
contract bundle under `schema/mastra-runtime/v1/`, retained `licenses/` and `notices/`, and the
pinned `node`/`node.exe`. There is one source of Node: the pinned executable in this directory.
The package does not use a second Node repository, a user Node installation or a shell.

## What `runtime.json` must say

`runtime.json` is the contract's strict `RuntimeReceipt`. When the panel connects, the host:

1. reads it as an exact object (unknown or missing fields fail);
2. requires schema and protocol `1.0`, the host target triple and the pinned Node version;
3. requires the runtime version, full repository revision, contract digest, policy digest and
   source-records digest that this host embedded at build time from one reviewed revision;
4. requires the entry point, `node_modules/`, contract bundle and retained notices to exist;
5. refuses an inventory digest that `packaging/mastra/policy.json` has not approved.

On the first turn, before modeling, it hashes `inventory.json`, `manifest.json` and every file
listed in `artifactDigests` by SHA-256 (the contract manifest must match `contractDigest` too).
The multi-hundred-megabyte pass is deliberately deferred off startup and runs once per backend,
not on every prompt.

Any mismatch is an actionable error before modeling begins. The approval check is the legal gate:
the current policy has no approvals, so a real artifact is refused with `UNAPPROVED_ARTIFACT` until
#15 records a reviewed approval. A stale or damaged installation is also refused rather than
repaired.

## Packaging path

`packaging/mastra/stage.sh` (macOS/Linux) and `stage.ps1` (Windows) run
`cargo xtask runtime-artifact --mode artifact` for the exact target, revision, contract, policy
and source-record digests, then `validate_runtime.py` checks the receipt, required tree,
non-inclusion **and the reviewed policy's approvals** before copying. Replacing the xtask gate with
`SOLIDCRAFT_RUNTIME_ARTIFACT_CHECK` requires the explicit `SOLIDCRAFT_MASTRA_STAGE_TEST=1` guard;
without it the staging helper refuses to run.

Bundling is opt-in in every package script:

- Linux: `packaging/linux/package.sh --with-mastra` or `SOLIDCRAFT_WITH_MASTRA=1`;
- macOS: `packaging/macos/package.sh --with-mastra` or `SOLIDCRAFT_WITH_MASTRA=1`;
- Windows: `packaging/windows/package.ps1 -WithMastra` or `SOLIDCRAFT_WITH_MASTRA=1`.

The nonpublishing release workflow pins `SOLIDCRAFT_WITH_MASTRA: "0"`, so ordinary packaging keeps
its previous contents. Turning it on for a release needs the owner's explicit decision plus an
approved artifact for every target. The supported matrix is:

- Windows `x86_64-pc-windows-msvc`;
- macOS `aarch64-apple-darwin` and `x86_64-apple-darwin`;
- Linux `x86_64-unknown-linux-gnu` (glibc).

A macOS universal app needs both per-slice artifacts:
`SOLIDCRAFT_MASTRA_ARTIFACT_AARCH64` and `SOLIDCRAFT_MASTRA_ARTIFACT_X86_64`. A single-architecture
app uses `SOLIDCRAFT_MASTRA_ARTIFACT`. The nested Apple-silicon Node is signed with the
`allow-jit` and `allow-unsigned-executable-memory` entitlements; the Windows SignPath configuration
includes `mastra/node.exe`, not just the root executables.

Windows x86/arm64, Linux arm64/riscv64, FreeBSD and wasm build packages without the built-in
workflow; they do not advertise it in 1.0. Signing and notarization hooks remain in the existing
package scripts. This work does not sign, notarize or publish anything.

## Codex: discovered, not bundled

Codex ACP is the user's own installation. Discovery searches the same places the direct agent
picker uses (`codex-acp` and its `.cmd`/`.bat` shims on Windows) and reports four distinct states:
ready, missing executable (with the install command), unusable/incompatible (with the startup
problem) or authentication required. The panel uses localized messages for these states. The
package contains no Codex binary, adapter, `auth.json` or credential; the staging helpers and tests
reject any such file, including at the staging boundary. A missing, broken or signed-out Codex is
reported before a task, not after a model call.

The panel still does not expose an ACP agent's sign-in and permission flow for Mastra turns
(#4), so Codex discovery is implemented and tested here while the agent-task path remains gated
outside packaging.

## Behaviour preserved

The AI master switch and the panel's explicit execution choice still decide whether anything
starts. The runtime is a child in its own process group; Stop, panel disconnect, runtime failure
and application exit use the existing bounded process-tree cleanup. A stopped or exited turn
leaves no workflow writes and no orphaned runtime or helper process.

Model calls are **buffered**: `model.generate` is one HostAi request per Mastra Agent step with no
hidden retry, tool execution or provider fallback. The protocol advertises `delivery: buffered`
and does not claim token streaming. Workflow stage, plan, text and tool events are progress
projections, not model-token deltas.

## Diagnostics

- The panel shows a localized startup error naming the missing file, the mismatched identity or
  the unapproved inventory. The packaging CLI prints the machine-readable `UNAPPROVED_ARTIFACT`
  code; the panel avoids exposing that code as user-facing prose.
- Settings → AI no longer requires a runtime command when the installed package carries
  `mastra/runtime.json`; a source checkout can still use the development command while debugging.
- Ordinary builds without Git history compile with the unbound revision sentinel and can never
  match an installed runtime. A strict release/Mastra build (`SOLIDCRAFT_WITH_MASTRA=1` or
  `SOLIDCRAFT_RELEASE=1`) still requires a real 40-hex revision or
  `SOLIDCRAFT_GIT_REVISION_OVERRIDE`.
- The release machine can run the same gate by hand:

```sh
cargo xtask runtime-artifact --mode policy
SOLIDCRAFT_MASTRA_ARTIFACT=target/mastra-runtime-artifact \
SOLIDCRAFT_MASTRA_INPUTS=target/mastra-runtime-inputs \
  packaging/mastra/stage.sh --target x86_64-unknown-linux-gnu --dest /tmp/mastra-check
```

Until #15 approves an artifact, the second command is expected to stop with `UNAPPROVED_ARTIFACT`
and create no destination. That is the gate working, not a packaging bug. Ordinary package builds
without `--with-mastra` do not call it.

## Still required before acceptance

- #15: legal review of the four enterprise-license files (or a different `@mastra/core` entry
  that does not import them) and of the packages without separate licence files, plus a reviewed
  `policy.approvals` entry. This is the reason the current artifact cannot ship.
- Owner decision on the release default (DQ-7/DQ-4): whether official packages include the Mastra
  workflow at all. The code keeps ordinary packaging off until that decision and an approval exist.
- #17: real fork runners, required checks and nonpublishing release evidence for the same
  revision.
- Real Windows x86_64, macOS aarch64/x86_64 and Linux x86_64 glibc machines: installed-package
  launch, signature/notarization verification, non-inclusion and a native modeling plus local
  validation turn. None of those are claimed by this repository-only change.

## The built-in agent in release packages

The Assistant's built-in agent (`packages/mastra-runtime/agent/`, Mastra Code's ACP agent with
Intcad's additions) is a second thing beside the workflow runtime above, with a folder of its
own. Nothing in the sections above applies to it, and it does not go through
`cargo xtask runtime-artifact`: that gate admits the `mastra/` artifact and has no approval.

**A release carries the agent.** `release.yml` sets `SOLIDCRAFT_WITH_MASTRA_AGENT=1` and
`SOLIDCRAFT_DISTRIBUTION=mastra-agent`; each supported package builds the agent's folder on its own
runner from the pinned lock with that system's pinned Node, has the package script check it, and
proves from the built installer bytes that the folder is whole. The long-lived `SOLIDCRAFT_WITH_MASTRA`
(the legacy `mastra/` workflow runtime) stays `0`: that artifact is still unapproved.

The agent is the Assistant's agent. The agents found on the computer and the ones the user added
are still listed after it (`crates/ui-egui/src/agent_chat/agents.rs`): a release package brings a
working agent, it does not take an external ACP agent away.

`.github/workflows/agent-test-packages.yml` stays: it is the hand-started lane for one system's
package before a release, not the way a release gets the agent.

### Where it is, and how the app finds it

| Package | Folder | Target |
|---|---|---|
| Linux `.deb`, `.rpm`, `.tar.gz`, AppImage | `usr/lib/intcad/mastra-agent` | `x86_64-unknown-linux-gnu` |
| Flatpak, Arch, Snap (x86_64) | what the Linux tarball they repack carries | `x86_64-unknown-linux-gnu` |
| macOS `.app`, universal | `Contents/Resources/mastra-agent/aarch64-apple-darwin` and `.../x86_64-apple-darwin` | both Apple targets |
| Windows MSI and portable zip, x64 | `mastra-agent\` beside `intcad.exe` | `x86_64-pc-windows-msvc` |
| Linux aarch64, FreeBSD, Windows x86/arm64, web | none (native) | — |

The folder holds a Node program (`node`, `node.exe`), the agent's entry files (`agent/*.mjs`),
`node_modules/` with Mastra Code's SDK, the modelling skills, `licenses/` and `agent.json` (what it
was built from and what was left out). When no `SOLIDCRAFT_MASTRA_AGENT` is set, the app looks for
the folder beside itself by the rules of `mastra/`, including the per-CPU folder of a universal
app (`crates/ui-egui/src/agent_chat/agents.rs`). The folder is what says the package carries the
agent: with it the built-in agent is offered first and started when the Assistant opens. A folder
that is damaged does not bring the old agent list back; the agent fails to start and the Assistant
shows that. Architectures the agent is not built for carry none and say so in `contents.json`.

### What the agent does by itself, and what it does not

Whatever would go on the network, report, or write files without being asked is off:

- **No usage reporting.** Mastra reports feature use to PostHog unless `MASTRA_TELEMETRY_DISABLED`
  is set. The app starts the agent with it set, and the entry sets it for itself before anything
  of Mastra's is loaded.
- **No refresh written to disk.** Mastra refreshes the providers' lists every five minutes and
  writes them into its cache and into its own package folder. The entry switches that off with
  Mastra's `MASTRA_OFFLINE`. (In a folder that cannot be written the refreshed list was never used
  anyway: tried, the list in memory and the saved refresh time stayed as they were.)
- **No embedding model.** The agent's sessions have no vector store (`agent/session.mjs`), so
  Mastra's memory embeds nothing, and `@mastra/fastembed` with `onnxruntime-node` and the model
  it downloads from Hugging Face are not in the folder. Mastra's memory still observes a long
  conversation and puts what it observed in place of the old messages, and its `recall` tool
  browses the conversation; it does not search by meaning.
- **Nothing written into the package.** The package scripts make the folder read-only. What the
  agent keeps (sign-ins, its database, its settings, the lease files of its sessions) is in the
  app's settings folder under `mastra/`.

One thing still goes out unasked. When a session opens, Mastra Code reads two public catalogs
into memory, `https://models.dev/api.json` and
`https://api.netlify.com/api/v1/ai-gateway/providers`: that is where the models a session offers
come from, so the list is current whenever the network is there (with a made-up key for one
provider, 15 of the 369 models listed were not in the list the SDK was released with). It reads
them once for each new session; in a seven-minute session they were not read again. With the
network taken away the same session listed one model. No switch of Mastra's for this was found,
and none is set. Everything else on the network is what a person asks for: a model's service when
a message is sent, a sign-in page, an MCP server. The agent refuses a model name that is not in
its list.

`agent/session.mjs` is adapted from Mastra Code's own session builder (Apache-2.0), because that
builder has no option for the vector store; `vector: false` is what switches it off, and it is not
a value Mastra documents. The build fails when the SDK's builder is no longer the file the
adaptation was made from (`agent-package/adapted-from.json`).

### How the folder is made

The release workflow does this on each runner; a person does the same by hand:

```sh
# aarch64 macOS universal app: one folder per CPU
node packages/mastra-runtime/scripts/agent-package.mjs build --target aarch64-apple-darwin --out target/mastra-agent/aarch64-apple-darwin
node packages/mastra-runtime/scripts/agent-package.mjs build --target x86_64-apple-darwin --out target/mastra-agent/x86_64-apple-darwin
SOLIDCRAFT_MASTRA_AGENT_DIR_AARCH64=target/mastra-agent/aarch64-apple-darwin \
SOLIDCRAFT_MASTRA_AGENT_DIR_X86_64=target/mastra-agent/x86_64-apple-darwin \
  packaging/macos/package.sh --arch universal --skip-build --with-mastra-agent

# Linux x86_64 (or SOLIDCRAFT_WITH_MASTRA_AGENT=1)
node packages/mastra-runtime/scripts/agent-package.mjs build --target x86_64-unknown-linux-gnu --out target/mastra-agent/x86_64-unknown-linux-gnu
packaging/linux/package.sh --with-mastra-agent

# Windows x64
node packages/mastra-runtime/scripts/agent-package.mjs build --target x86_64-pc-windows-msvc --out target\mastra-agent\x86_64-pc-windows-msvc
pwsh packaging/windows/package.ps1 -Arch x64 -WithMastraAgent
```

A package for an architecture the agent is not built for (`--without-mastra-agent`,
`-WithMastraAgent:$false`, `SOLIDCRAFT_WITH_MASTRA_AGENT=0`) carries none and must not claim one.

`build` needs the pinned Node and npm (`.nvmrc`, `packageManager`) and the npm registry. It
installs `packages/mastra-runtime/agent-package/package-lock.json` for the target with
`npm ci --ignore-scripts` into a new folder (that lock is outside the npm workspace, so the
workspace's lock and the inventory that records it are not touched), and then:

1. takes Mastra's enterprise-licensed content out (`remove-enterprise.mjs`): the directories named
   `ee`, the `ee-*` chunk as CommonJS, the source maps and two type files are deleted, and the ESM
   chunk, which other files import by name, is replaced under its own name by a stand-in generated
   from its export list, with none of its code and every enterprise feature reading as off;
2. drops what Node never loads: source maps, type declaration files, native binaries of other
   systems;
3. leaves out whole packages that only an on-demand feature reaches (the list, with the importing
   file of each, is `LEFT_OUT` in the script and is copied into `agent.json`): browser automation
   tools, the DuckDB trace store, voice transcription, Postgres storage, the type-generation
   command line of `@mastra/mcp` (what brings `tsx` and `esbuild`), and embedding on this
   computer;
4. adds the entry files, the Node program that ran the script, and the licence texts of every
   package that is left (`licenses/THIRD-PARTY-LICENSES.txt`, `licenses/NODE-LICENSE.txt`);
5. copies the skill sets the agent carries from the repository into `skills/<set>/`, with a stamp
   (`skill-set.json`: every file with its SHA-256, and the set's `version`, the hash of the whole).
   The sets are named in `packages/mastra-runtime/agent/skills.mjs`; today there is one,
   `modelling-skills`: `skills/solidcraft-modeling`, `-editing`, `-checking` and `-mechanisms`,
   copied as they are, front matter and all. Hidden files and Python caches are not copied.

### The skills, at start

Mastra Code looks for skills in `.mastracode/skills`, `.claude/skills` and `.agents/skills`, under
the session's folder and under its home (`buildSkillPaths` in the SDK's
`dist/agents/workspace.js`). The agent's home is its own (`MASTRA_APP_DATA_DIR/home`), so before
anything of Mastra's is loaded the entry makes `home/.agents/skills` hold each set as the package
has it: a skill folder whose files are the same is left alone (about 1 ms), one that differs (an
older package, or the model wrote into it) is replaced whole, and one an earlier set put there and
this one does not carry is removed. A note per set (`.agents/skills/.<set>.json`) says which
folders are the set's; nothing else in the home is touched, so a skill beside the set (another
set's, such as the one INT-1477 plans) stays as it is. Not into the session's folder: Mastra Code
skips a link there that leads out of it, and the session's folder may one day be the user's
project. Run by hand without `MASTRA_APP_DATA_DIR`, the agent installs nothing. Run out of the
repository, it installs the repository's own folders.

Mastra loads a skill only when the `name` in its SKILL.md is its folder's name, and refuses to pick
between two skills of one name found in two of the folders it reads: by its code (`@mastra/core`
1.84.0; not tried), the `skill` tool then fails for that name and `skill_search` for every query. A skill name is therefore in one set only (the build fails otherwise), and a
skill of the same name must not be put in the session's folder or in the home's other two folders.
Mastra Code lists every skill whose front matter does not say `user-invocable: false` as a
`skill/<name>` command of the session, so the four modelling skills are offered as commands.

### What stops a package build

Each of these fails the build, and no package is made:

- the lock file has an entry without one version, an npm registry address or a sha512 checksum,
  or the folder holds a package the lock file does not name at that version (`npm ci` checks
  every archive against the lock's checksum when it installs it);
- anything named `ee`, a source map naming such a source, or text of the enterprise licence
  header or licence check is left (the stand-in, byte for byte as the script writes it, is the
  one allowed `ee-*` file);
- a package states a licence the repository refuses (GPL and the other copyleft families) or
  none;
- a file looks like a secret or a stored sign-in: key shapes of the services the agent talks to,
  a private key with its key material, `auth.json`, databases, `.env`, an agent's own folder. The
  scan runs over the folder and then over the whole staged package, and prints paths and kinds,
  never what it found;
- Mastra Code's session builder is not the file `agent/session.mjs` was adapted from;
- the folder is for another target, is not whole, or (macOS, Linux) can be written;
- a skill of a set is missing, its stamp is missing or names other skills, a copy is not what its
  stamp says (a file changed, missing, added, or one a copy does not carry, such as a hidden
  file), `skills/` holds a folder of no set, or a copy is not the repository's folder as it is
  now: the check compares each copy with `skills/<name>` of the checkout it runs from, so a
  package built before a skill changed fails;
- the agent, started with the folder's own Node and entry as the package carries them, does not
  pass its eight cases against stand-ins (`agent-smoke.mjs`): three turns; a command after a
  permission request; a tool of an MCP server; the skills of every set listed as `skill/<name>`
  commands and in what the model is told, each installed in the agent's home as packaged, and
  each SKILL.md read by the model through Mastra's `skill` tool; a refused command, which does not
  run; a refused file write, which writes nothing; a turn cancelled while a permission request is
  open; and a turn cancelled while the answer streams. After each refusal or cancel nothing is
  left behind and the session answers the next message. No vector store may have been made.

A package script takes the folder only through `packaging/mastra/stage-agent.sh` (`.ps1`), which
runs the folder's check first and copies nothing when it fails.

### What a passing package shows, and what it does not

It holds no file of Mastra's under a directory named `ee` and no `ee-*` chunk of Mastra's;
`@mastra/core` is carried changed in that way, and the stand-in's first lines say so. The licences
counted in `agent.json` are what each package states in its `package.json`; nobody has reviewed
them, some packages ship no licence file, and no one at Mastra has approved or cleared anything.
`scripts/runtime-artifact.mjs` treats any `@mastra/core` file named `ee-*` as enterprise by its
name, so it would flag the stand-in if the agent were ever put through that script.

### What was run, and where

On macOS (Apple silicon, Node 22.23.4, npm 11.19.0):

- The folder was built (261 MB in 10,716 files with Node, from 1,013 MB of packages as installed;
  349 package copies), its check passed, and `agent-smoke.mjs` passed on it. With the entry of
  before this change the same run fails on the vector store, as it should.
- `packaging/macos/package.sh --arch aarch64 --with-mastra-agent` made an ad-hoc signed disk
  image of 232 MB; the same build without the opt-in gave 143 MB and no `mastra-agent` folder.
  The script's own steps passed: the folder's check before staging, the secret scan over the whole
  app, and the read-only check and the smoke run on the folder inside the mounted image.
- The app copied out of that image was started with no network beyond the machine's own address
  and no keychain. It offered the built-in agent alone, started the Node and the entry inside
  itself with no `SOLIDCRAFT_MASTRA_*` variable and with Mastra's reporting switch set, and after
  a stand-in address was added one message was answered by the stand-in; no file in the app's
  agent folder changed. With the folder's Node taken away the Assistant still offered the
  built-in agent alone and showed that it could not start. `packaging/assistant-check.py
  --built-in-agent` found the built-in agent alone and started on the whole copy (its picture of
  the app's frame was not taken in that run: the window was not on screen).
- A conversation of about 100,000 tokens in English, and one in Chinese, against a stand-in that
  also answered Mastra's memory: from the third or fourth turn the model's requests carried the
  observations and stopped growing, the `recall` tool answered, nothing failed, and no vector
  database was made.
- Under a watcher of the agent's own connections and file writes: with Mastra's reporting
  switch unset, its usage-event function tried `us.posthog.com`; with it set, nothing. (In a
  short session of the entry as it was before, with the switch unset, no such attempt was seen
  either: Mastra's code sends an event only for some features.) In a seven-minute session of
  the agent as built, with the network open and its folder writable, the only places it tried
  outside this machine were the two catalogs named above, once, in the session's first second,
  and no file in its folder changed. The watcher sees the sockets and fetches of the agent's own Node, not
  other programs.

The skill sets (2026-11-21, Linux x86_64, Node 22.23.4, with `@mastra/code-sdk` 1.19.1 installed
from the pinned lock, not by `build`): the agent's entry and `skills/` as `build` lays them out,
beside that install, passed all eight smoke cases; in the skills case each of the four skills was
a `skill/<name>` command, was named in the model's first request, and was read with no permission
request. With `skills/` taken away the same case failed. The install took 20 ms at the first start
and about 1 ms after. `build` itself was not run with the skills.

For Linux x86_64 and Windows x64 the folder was built on that Mac with a placeholder in place of
the Node program, to see the build and the check work for those targets: 151 MB and 150 MB
without Node (Node itself is about 125 MB and 87 MB). Nothing of those folders was run. `packaging/linux/package.sh --with-mastra-agent`, `packaging/windows/package.ps1
-WithMastraAgent`, the WiX harvest and the workflow have not been run anywhere; the PowerShell
changes were not even parsed. The first run of the workflow is the evidence for those systems.

### What the release gate proves, and what it does not

`scripts/ci/package_contents.py` records, from each built installer, whether `mastra-agent/` is
there, whether its Node program is there, and which of its required files are missing;
`scripts/ci/release_content.py` refuses a release that declares `mastra-agent` and ships a
supported package without it whole. The installed checks (`packaging/linux/installed-check*.sh`,
`packaging/macos/installed-check.sh`, `tools/windows-assistant-smoke.ps1`) open the Assistant in the
installed copy, require the built-in agent first, require it to start out of the package and to
start again after it stopped.

None of that is a licence: the scans, the lock, the enterprise removal and the smoke tests are
technical checks. The open points below remain decisions for the owner and legal review, and a
release that ships the agent must record them.

### Open before the release carries it

- **Redistribution rights.** The lock's packages declare licences, but no one has reviewed the
  exact retained files, the transitive licences and the notices, and no one at Mastra has cleared
  them. A green scan is not permission to redistribute. Owner and legal review decide; this is a
  release blocker until recorded.
- **The three systems.** Nothing has run on Windows or Linux. On Windows, file modes do not make
  a folder read-only: an MSI installs it where a user cannot write, a portable zip unpacks it
  writable. With Mastra's refresh off, nothing was seen to write into a writable folder (macOS,
  seven minutes); that has not been watched on Windows.
- **The two catalogs.** Whether reading them at each new session is acceptable, or the list
  should come from somewhere else, is not decided.
- **A leftover list.** A provider list that an earlier version left in the agent's data folder
  (`home/.cache/mastra`) is still copied once into a folder that can be written.
- **Signing.** The macOS script signs the folder's Node and native libraries with the app; with a
  Developer ID and the hardened runtime that has not been tried. The Windows SignPath
  configuration does not name `mastra-agent\node.exe`.
- **No inventory.** There is no per-file provenance record of this folder as the workflow
  runtime has in `packaging/mastra/inventory.json`. Whether the release may carry the agent on the
  checks above, or only through that gate, is undecided.
- **Skills as commands.** Whether the modelling skills should be offered as `skill/<name>`
  commands is open (PRD section 9; the recommendation is no). Their front matter is left as it
  is, so Mastra Code offers them; not offering them takes either a filter in the app or
  `user-invocable: false` in each SKILL.md.
- **A skill of the same name elsewhere.** Mastra refuses to pick between two skills of one name in
  two of the folders it reads. Nothing puts a second copy there today; if the session's folder
  becomes a project with its own skills, a project skill named like a built-in one makes that
  skill unusable, and skill search with it, rather than taking its place.
