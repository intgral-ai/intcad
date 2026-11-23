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
  `mastra-agent`, proved from the built installers: the folder present, its Node present, none of
  its required files missing, at least one `skills/<set>/<skill>/SKILL.md`, no legacy `mastra/`
  runtime, no enterprise content or Node outside the folder, no unchecked artifact. `mastra` stays
  refused; a derived package must match its source.
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

## Release workflow evidence

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

## Still open (release blockers, not code)

- **Redistribution rights** for the exact retained files, transitive licences and notices: owner
  and legal review. A green scan is not permission to redistribute.
- **Real machines**: Windows and macOS ran on CI runners only; a person installing the MSI/DMG and
  a real-account conversation, CAD read and approved CAD edit on each platform remain.
- **FreeBSD** is broken by #174's `egui_winit` call, not by the agent; it needs its own fix.
- **#212** (project context, INT-1473) is a separate draft and stays out of this change.
