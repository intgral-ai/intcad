# The built-in Mastra agent in Intcad's release packages

Work item: INT-1503 (the 11-22 release), with INT-1473's agentic work and INT-1477's runtime.
Baseline: `origin/develop` `56e805ffc39f61cea04e5da9cedb56497c28aaac` (PR #178, squash
`9ce777903`, merged).

## Root cause: why the agent only reached test packages

Four separate things held it back, and all four had to change together:

1. **The release switched it off in one place and had no way to switch it on.** `release.yml`
   set `SOLIDCRAFT_WITH_MASTRA: "0"` (the *legacy* `mastra/` workflow runtime) and never set
   `SOLIDCRAFT_WITH_MASTRA_AGENT` at all, so `packaging/{linux,macos,windows}` never staged
   `mastra-agent/`. Only `.github/workflows/agent-test-packages.yml` built it, by hand, per
   system, from an earlier commit.
2. **The release gate required a package set that could not carry the agent.**
   `SOLIDCRAFT_DISTRIBUTION: "native"` plus `scripts/ci/release_content.py` failed any package
   whose bytes contained `mastra`, `node_modules` or a Node program. Turning the agent on
   without changing the gate would have failed the release, and turning the gate off would have
   removed the only proof of what a package contains.
3. **The agent's folder had never been built on the runners.** The lock installs that system's
   Node and native modules, so an agent built on macOS cannot be shipped in a Windows MSI. The
   test workflow built it on its own runners; `release.yml` had no Node setup or build step.
4. **The macOS app had no place for a universal agent.** `package.sh --with-mastra-agent`
   refused `--arch universal`, and the universal DMG is the only macOS package the release
   makes.

## What changed

- `crates/ui-egui/src/agent_chat/agents.rs`: `entries()` lists the built-in agent **first** and
  then the agents found on the computer and the ones the user added, instead of returning the
  built-in agent alone. New `is_builtin_entry` / `builtin_current` answer "is this conversation's
  agent the package's one", so its Connections page, key handover, data folder and start stay
  its own while an external ACP agent keeps working.
- `mod.rs`, `connections.rs`, `settings.rs`, `web.rs`: use those two helpers; discovery runs
  again; Settings lists the other agents and keeps the execution choice when they exist; a
  carried agent has no Remove/Edit/Make-default, is not "the agent started last", and the page
  is told the connections only for it.
- `packaging/linux/package.sh`, `packaging/macos/package.sh`, `packaging/windows/package.ps1`:
  staging is the release's default (`--without-mastra-agent` / `-WithMastraAgent:$false` to
  leave it out); macOS universal stages one folder per CPU from
  `SOLIDCRAFT_MASTRA_AGENT_DIR_{AARCH64,X86_64}`, signs every slice's Node and native libraries,
  and checks/runs the host slice from the mounted image; the agent's licences and the read-only
  checks still run.
- `.github/workflows/release.yml`: `SOLIDCRAFT_WITH_MASTRA_AGENT=1`,
  `SOLIDCRAFT_DISTRIBUTION=mastra-agent`, pinned Node 22.23.4 / npm 11.19.0 on every supported
  runner, the agent built from `packages/mastra-runtime/agent-package/package-lock.json` and run
  against stand-ins before packaging. `SOLIDCRAFT_WITH_MASTRA` stays `0`. Other architectures
  declare `native`.
- `.github/workflows/release-gate.yml`: declares `SOLIDCRAFT_DISTRIBUTION: "mastra-agent"` and
  passes it to the decision.
- `scripts/ci/package_contents.py`, `scripts/ci/release_content.py`: a third distribution,
  `mastra-agent`, proved from the built installers: the folder present, its Node present, none of
  its required files missing, at least one `skills/<set>/<skill>/SKILL.md`, no legacy `mastra/`
  runtime, no enterprise content outside the folder, no unchecked artifact. `mastra` stays
  refused; a derived package (Flatpak/Arch/Snap) must match its source.
- `packaging/linux/installed-check-inside.sh`, `packaging/macos/installed-check.sh`,
  `tools/windows-assistant-smoke.ps1`, `packaging/assistant-check.py`: the installed copy must
  offer the built-in agent first, start it out of the package, report its connections, stop and
  start again.
- `docs/mastra-packaging.md`, `docs/beta-evidence.md`, `skills/solidcraft-packaging/SKILL.md`:
  the release default, the layout per platform, what the gate proves, and what it does not.
- Tests: `crates/ui-egui/src/agent_chat/agents.rs`, `agent_chat/tests.rs`,
  `scripts/ci/test_package_contents.py`, `scripts/ci/test_release_content.py`.

## Local evidence

- `cargo test --profile ci -p solidcraft-ui-egui --lib -- agent_chat` — 376 passed,
  3 ignored.
- `cargo clippy --profile ci -p solidcraft-ui-egui --all-targets -- -D warnings` — clean.
- `python3 scripts/ci/test_package_contents.py` / `test_release_content.py` — 17 + 19 passed.
- `packaging/tests/mastra_agent_test.py` — 8 passed.
- `node packages/mastra-runtime/scripts/agent-package.mjs build --target
  x86_64-unknown-linux-gnu` with Node 22.23.4 — 291 MB, 10,716 files, 349 packages, 0 problems.
- `node packages/mastra-runtime/scripts/agent-smoke.mjs` — 8 of 8 cases pass, on the folder the
  release stages and again on the folder unpacked from `intcad-0.1.2-linux-x86_64.tar.gz`, in
  `fedora:latest` with **no system Node** (`command -v node` empty).
- `packaging/linux/package.sh --skip-build --formats tar` with the pinned Node — produced
  `intcad-0.1.2-linux-x86_64.tar.gz` (461 MB) with `lib/intcad/mastra-agent/` at mode `555`.
- `scripts/ci/package_contents.py --distribution mastra-agent` on that tarball —
  `agent.present`, `agent.node`, no missing file, all four modelling skills, `markers: []`,
  `agentMarkers: []`, `unchecked: []`.
- `packaging/assistant-check.py --built-in-agent` in `fedora:latest` with the tarball's app —
  the Agent's page and the built-in agent came up, the agent started out of the package and
  reported its connections, and stopped.
- `actionlint` on both workflows — clean; `shellcheck` on the shell scripts — clean.

## Still open (release blockers, not code)

- Redistribution rights for the exact retained files, transitive licences and notices: owner
  and legal review, not a scan.
- Windows MSI and macOS DMG: CI runners only until real machines install them.
- The modelling skills reach the agent through PR #204 (draft); the gate now refuses a release
  whose agent carries none.
- Real-account conversation, a real CAD read and a real approved edit on each platform.
