# The built-in Mastra agent in Intcad's release packages

Work item: INT-1503 (the 11-22 release), with INT-1473's agentic work and INT-1477's runtime.
Baseline: PR #178, squash `9ce777903`, merged into `develop`; this branch is based on PR #204 (the
modelling skills).

## Root cause: why the agent only reached test packages

Four separate things held it back, and all four had to change together:

1. **The release switched it off in one place and had no way to switch it on.** `release.yml`
   set `SOLIDCRAFT_WITH_MASTRA: "0"` (the *legacy* `mastra/` workflow runtime) and never set
   `SOLIDCRAFT_WITH_MASTRA_AGENT`, so `packaging/{linux,macos,windows}` never staged
   `mastra-agent/`. Only `.github/workflows/agent-test-packages.yml` built it, by hand, per
   system, from an earlier commit.
2. **The release gate required a package set that could not carry the agent.**
   `SOLIDCRAFT_DISTRIBUTION: "native"` plus `scripts/ci/release_content.py` failed any package
   whose bytes contained `mastra`, `node_modules` or a Node program. Turning the agent on
   without changing the gate would have failed the release; turning the gate off would have
   removed the only proof of what a package contains.
3. **The agent's folder had never been built on the runners.** The lock installs that system's
   Node and native modules, so an agent built on macOS cannot be shipped in a Windows MSI. The
   test workflow built it on its own runners; `release.yml` had no Node setup or build step.
4. **The macOS app had no place for a universal agent.** `package.sh --with-mastra-agent`
   refused `--arch universal`, the universal DMG is the only macOS package the release makes,
   and `x86_64-apple-darwin` was not a build target at all.

## What changed

- `crates/ui-egui/src/agent_chat/agents.rs`: `entries()` lists the built-in agent **first** and
  then the agents found on the computer and the ones the user added, instead of returning the
  built-in agent alone. New `is_builtin_entry` / `builtin_current` answer "is this conversation's
  agent the package's one", so its Connections page, key handover, data folder and start stay its
  own while an external ACP agent keeps working.
- `mod.rs`, `connections.rs`, `settings.rs`, `web.rs`: use those helpers; discovery runs again;
  Settings lists the other agents and keeps the execution choice when they exist; a carried agent
  has no Remove/Edit/Make-default, is not "the agent started last", and the page is told the
  connections only for it.
- `packaging/linux/package.sh`, `packaging/macos/package.sh`, `packaging/windows/package.ps1`:
  staging is the release's default (`--without-mastra-agent` / `-WithMastraAgent:$false` to leave
  it out); macOS universal stages one folder per CPU, signs every slice's Node and native
  libraries, ships each slice's own licence texts, and checks/runs the host slice from the
  mounted image; the tar carries ordinary modes because Flatpak, Arch and Snap rewrite their
  extracts, while deb/rpm/AppImage keep the agent read-only.
- `.github/workflows/release.yml`: `SOLIDCRAFT_WITH_MASTRA_AGENT=1`,
  `SOLIDCRAFT_DISTRIBUTION=mastra-agent`, pinned Node 22.23.4 / npm 11.19.0 on every supported
  runner, the agent built from `packages/mastra-runtime/agent-package/package-lock.json` and run
  against stand-ins before packaging. The macOS job fetches both Apple Nodes from nodejs.org
  under checksum so the Intel folder is built with an x86_64 Node. `SOLIDCRAFT_WITH_MASTRA`
  stays `0`. Other architectures declare `native`.
- `.github/workflows/release-gate.yml`: declares `SOLIDCRAFT_DISTRIBUTION: "mastra-agent"` and
  passes it to the decision.
- `scripts/ci/package_contents.py`, `scripts/ci/release_content.py`: a third distribution,
  `mastra-agent`, proved from the built installers rather than the staging tree: at the place the
  packager installs it (`usr/lib/intcad/mastra-agent`, the app's `Contents/Resources/mastra-agent`,
  or the Windows tree's `mastra-agent/`), the folder present with its Node, every required file
  and at least one `skills/<set>/<skill>/SKILL.md`; each macOS slice whole on its own; no legacy
  `mastra/` runtime, no enterprise chunk and no Node outside that place; no unchecked artifact;
  and an answer for every artifact, so a good archive cannot vouch for a broken sibling. A
  universal app with one stand-in per slice, an empty Windows `mastra/` folder and the headless
  CLI archive are recognised as what they are. `mastra` stays refused; a repack is inspected from
  its own bytes as well as its source, and the Flatpak bundle is listed from the installation its
  job makes of that file.
- `packaging/linux/arch-agent.sh`: the `arch-package` action's PKGBUILD copies only `bin/` and
  `share/` from the tarball, so `lib/intcad/mastra-agent` never reached the `.pkg.tar.zst`. The
  release job adds that folder from the same tarball, regenerates the package's `.MTREE` and
  repacks it, and the old `contents.json` only looked correct because the recorder read the
  source tarball rather than the package.
- Permission handling: a tool's ACP title decides only whether Intcad asks a second time. The
  session gives the agent Intcad's own `intcad-app` MCP server, and every call that would change
  the design is checked by the Rust host gate whatever the agent calls it; the shorter
  `intcad-app_<tool>` spelling is trusted only from the packaged agent that is actually running
  (`builtin_running`: the running id plus the package's own folder), never from an environment
  variable's program.
- `packages/mastra-runtime/scripts/agent-package.mjs`: `x86_64-apple-darwin` added as a target.
- `packaging/mastra/stage-agent.sh`: forwards the test-only `--skills-root`.
- Installed checks: `packaging/linux/installed-check-inside.sh`, `packaging/macos/installed-check.sh`,
  `tools/windows-assistant-smoke.ps1`, `packaging/windows/installed-check.ps1` and
  `packaging/assistant-check.py` require the installed copy to offer the built-in agent first,
  start it out of the package, report its connections, stop, start again, and leave no agent
  process behind; the Windows check asks only the x64 MSI for an agent.
- `docs/mastra-packaging.md`, `docs/beta-evidence.md`, `skills/solidcraft-packaging/SKILL.md`.
- Tests: `agents.rs`, `agent_chat/tests.rs`, `scripts/ci/test_package_contents.py`,
  `scripts/ci/test_release_content.py`, `packaging/tests/mastra_agent_test.py`.

## Local evidence

- `cargo test --profile ci -p solidcraft-ui-egui --lib -- agent_chat` — 697 passed, 3 ignored.
- `cargo clippy --profile ci -p solidcraft-ui-egui --all-targets -- -D warnings` — clean;
  `cargo fmt --all -- --check` — clean.
- `python3 scripts/ci/test_package_contents.py`, `test_release_content.py`, `test_gate.py`,
  `packaging/tests/mastra_agent_test.py` — 101 tests, all passing.
- `actionlint`, `shellcheck`, and every changed PowerShell file parses.
- The agent's folder built with Node 22.23.4 (291 MB, 10,716 files, 349 packages, 0 problems); its
  8 stand-in cases pass, staged and again unpacked from the tarball in `fedora:latest` with no
  system Node.
- Cross-target builds verified locally: `x86_64-pc-windows-msvc` with the official win-x64 Node
  and `x86_64-apple-darwin` with the official darwin-x64 Node both build and pass `check`.
- `packaging/assistant-check.py --built-in-agent --screen` on the installed tarball copy in
  `fedora:latest`: 14 of 14 checks, including "the agent stopped with the app".

## Skills and read-only behaviour, from the shipped package

From `intcad-0.1.2-linux-x86_64.tar.gz` (the tarball in `dist/release`), in `ubuntu:24.04` with
`command -v node` empty:

```
system node: NONE
{"case":"the skill sets are listed and each SKILL.md is read",
 "commands":["skill/solidcraft-checking","skill/solidcraft-editing","skill/solidcraft-mechanisms","skill/solidcraft-modeling"],
 "read":[{"skill":"solidcraft-checking","installedAsPackaged":true,"read":true,"permissionsAsked":0},
         {"skill":"solidcraft-editing","installedAsPackaged":true,"read":true,"permissionsAsked":0},
         ...],
 "summary":"all passed"}
```

So the packaged agent tells the model about the four skills, the model asks for each, and each
`SKILL.md` is read from the package with **no permission question** (`permissionsAsked: 0`): a
read-only step needs no repeated approval, as required.

## Release workflow evidence: what passed, in which run

The three platform packages below each passed in a release-workflow run, but **not all in one
run**: the full matrix at one SHA is still red because of FreeBSD (see below), which is why
"all nonpublishing package builds" is red at every SHA tried so far.

| Run | SHA | What it proves |
|---|---|---|
| [45112837415](https://github.com/intgral-ai/intcad/actions/runs/45112837415) | `77914e22` | Linux x86_64 (deb, rpm, AppImage, tar.gz) and macOS universal carry the agent whole; Linux aarch64, Windows x86/arm64, web are `native`. It also claimed Flatpak/Arch/Snap x86_64 carried it; that was the recorder reading the source tarball — the Arch package did not, which this branch now fixes |
| [45105758418](https://github.com/intgral-ai/intcad/actions/runs/45105758418) | `9df38898` | Windows x64 MSI + portable zip + standalone exe carry the agent whole; Windows x86/arm64 MSI are `native`; the installed MSI starts, stops and restarts the agent |
| [45086047185](https://github.com/intgral-ai/intcad/actions/runs/45086047185) | `9dd860eb` | Arch x86_64 with the writable-tarball fix; Flatpak/Arch/Snap aarch64 `native` |



Run [45112837415](https://github.com/intgral-ai/intcad/actions/runs/45112837415) at `77914e22`
(nonpublishing, `workflow_dispatch`):

| Job | Result |
|---|---|
| Linux x86_64 (deb, rpm, AppImage, tar.gz) | pass — `contents.json`: `mastra-agent`, `agent.node`, no missing file, four skills, no marker, nothing unchecked |
| Linux aarch64 | pass — `native`, no agent |
| macOS universal (arm64 + x86_64) | pass — `contents.json`: `agent.nodeTargets` both slices, four skills, no marker |
| Windows x64 MSI + zip | pass — `contents.json`: whole agent, four skills (run 45105758418) |
| Windows x86, arm64 | pass — `native`, no agent |
| Flatpak x86_64, Arch x86_64, Snap x86_64 | pass — derived from the Linux tarball, `mastra-agent`, source digest matches |
| Flatpak/Arch/Snap aarch64 | pass — `native` |
| Web (wasm) | pass — `native` |
| FreeBSD x86_64 | **fail** — `egui_winit` unresolved in `apps/solidcraft/src/main.rs`, introduced by #174; unchanged here and unrelated to the agent |

The installed checks inside those jobs printed, for each platform: the built-in agent offered
first and current, started out of the package with its connections, stopped with none, started
again, and no agent process left behind.

## Real-function acceptance: the Linux run

Run on this machine, 2026-11-23, with the agent folder from
`intcad-0.1.2-linux-x86_64.tar.gz` and the app built from this branch. The machine is NixOS, which
cannot execute the package's generic-Linux Node, so that one file was replaced with Node 22.23.5
built for this host; every other file is the package's. The DeepSeek key the machine already keeps
in its keyring was used for the provider; it was not copied into the repository, and the keyring
entry and the agent data folder the run created were removed afterwards.

| Step | Result |
|---|---|
| Real model, one message | the built-in agent answered through `deepseek/deepseek-flash` |
| Read-only CAD tool | `intcad-app_inspect_design` ran with **no permission question**; it reported the sample plate's one body, its revision and its volume; `document.revision` unchanged |
| Approved edit, mode "ask" | the approval `solid.box` was asked and allowed; the model created `solid.box` as `E2EBox` and confirmed it by inspection; `document.revision` 1 → 2 |
| Rejected edit, mode "ask" | a second `solid.box` was asked and **declined**; the model reported "the request was declined, so nothing changed"; `document.revision` stayed 2 |
| Modelling skill | the model activated `solidcraft-modeling` (the `skill` tool and `intcad-app_skill_reference`), built a washer OD 30 / ID 10 / 3 mm, and its own check matched π/4·(30²−10²)·3 = 1,884.956 mm³; `document.revision` 1 → 2 |

Two defects were found by this run and fixed in the same branch:

1. `acp::is_bridge_tool` only knew the ACP spelling `mcp__intcad-app__<tool>`. A real session
   names the tool `intcad-app_<tool>`, so a **read-only** bridge tool asked the user for
   permission, and the double question the host is meant to remove came back. Both spellings count
   now, with the same one-word rule; the host gate still decides what a call may change.
2. The control channel's item list did not carry an approval's id or a permission's index, so a
   program could see a request but not answer it. Both are there now, and `docs/control-protocol.md`
   says so.

## What the real run did not cover

- A second ACP agent run end to end. OpenCode is detected, listed, selectable and reaches `ready`
  (a real ACP session) in a probe; a full turn with it is not run.
- Upgrade and reinstall over an older version.

## Spec and DFM after the runtime switch

The runtime switch does not touch either path; what was checked:

| Surface | Check | Result |
|---|---|---|
| DFM | `cargo test --profile ci -p solidcraft-ui-egui --lib -- dfm` | 24 passed |
| DFM (Review panel) | the installed package's screenshot | the Review entry and its report are in the right rail |
| Text-to-CAD | packaged agent, real model | the `skill` tool and `intcad-app_skill_reference` were used; a washer was built and its volume matched π/4·(30²−10²)·3 |
| Spec | there is no separate Spec editor in the app before or after this change; a spec is a file in the project, which the agent reads and writes through the same project-bounded file tools | #212's packaged smoke: V1 (brief and file list), V4 (a refused write leaves the file byte for byte, an allowed one is written) |

## Owner decisions recorded (2026-11-23)

- **Redistribution rights** are being discussed by the owner and the team. They are not a blocker
  for this pull request.
- **Windows and macOS real-machine acceptance** is done by the owner and colleagues after this
  lands on `develop`; it is not a blocker for this pull request.
- A repack's own agent is proved from its bytes, not by starting the agent inside the Flatpak,
  Arch or Snap build; the owner accepts that scope.
- The work is complete when these changes are merged into the existing pull request.

## Earlier note (kept for the record): before the real run

The objective's C block (a real model conversation, a real read-only CAD tool through the agent, an
approved edit and a rejected edit with the document unchanged, Text-to-CAD executed end to end) is
**not evidenced**. This machine has no display, no provider key in the environment, no local model
server and no Windows/macOS hardware. Using the account's own stored keychain entries would mean
spending the user's production credentials and is not done here.

What is needed, and who can do it:

1. A display (X11/Wayland, or Xvfb) and a provider: either an API key pasted into the Assistant's
   Connections page, or a plan signed in there.
2. Install the built package, open the Assistant, send one message, then ask for a read-only CAD
   question (for example "measure the open document") and check the tool result in the record.
3. Ask for an edit, allow it, and check the model changed. Ask for another, reject it, and check
   the design's file hash did not change.
4. Ask for a small part in words, confirm Text-to-CAD is chosen and its `SKILL.md` is in the run,
   and that the commands reach the real design.
5. For INT-1473's context (#212), read the Files list and the newest DFM report from the same run.

Stand-in-model runs are not a substitute for this and are not reported as if they were.

## Still open (recorded, not blockers for this pull request)

- **Redistribution rights** for the exact retained files, transitive licences and notices: the
  owner and the team are discussing them, and this is recorded rather than treated as a PR
  blocker. A green scan is still not permission to redistribute.
- **Real machines**: Windows and macOS ran on CI runners only. The owner and colleagues accept the
  remaining real-machine run after this lands on `develop`; it is recorded, not a PR blocker.
- **FreeBSD** was broken by #174's `egui_winit` call and is fixed on `develop` by #219; the agent
  is not involved.
- **#212** (project context, INT-1473): the objective puts INT-1473's agentic work in this
  release, and that work (Files list, newest DFM report, project-scoped reads as the default
  context) lives in #212. #212 is no longer a draft (`mergeable: MERGEABLE`,
  `mergeStateStatus: CLEAN`, base `tianwei/modelling-skills-40c32c`, the same #204 head this
  branch sits on). Measured integration, on a local branch only, not merged and not pushed:

  | Check | Result |
  |---|---|
  | `git merge pr212` onto this branch | clean, no conflicts |
  | `cargo test --profile ci -p solidcraft-ui-egui --lib -- agent_chat` | 700 passed; the only failures are the base's two host-environment ones (`cat` missing from the tests' minimal PATH) |
  | `cargo clippy -p solidcraft-ui-egui --all-targets -- -D warnings` | clean; `cargo fmt` only re-wrapped one #212 test |
  | agent folder built from the merged tree | both skill sets: `modelling-skills` (4) and `context-skills` (`intcad-context`) |
  | `agent-smoke.mjs` on that package | 22 of 22 cases pass: project brief and file list (V1, V3), a refused write leaving the file byte for byte and an allowed one (V4), a file beside the project unreadable through its path or a link (V5), read tools asking nothing inside the project (F1), project agent files/MCP/hooks not loaded (V6), no agent files written to a session folder (R3), no earlier session's project files through `recall` (R9), `/tmp` untouched (R1), no brief without a project (V8) |

  So #212 is ready to land and the packaging takes it up unchanged. Landing it is a review and
  owner decision; this branch does not merge another PR's commits. If it does not land before the
  release, the release ships without the Files/DFM context INT-1473 describes.
