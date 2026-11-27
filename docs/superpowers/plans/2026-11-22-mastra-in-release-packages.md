# Ship the built-in Mastra agent in Intcad's release packages

**Goal:** Every formal Intcad release package for a supported target carries the built-in Mastra
Code agent (its own Node, entry, dependencies and licences) and the Assistant uses it by default,
while external ACP agents stay usable.

**Architecture:** Reuse `packages/mastra-runtime/scripts/agent-package.mjs`,
`packaging/mastra/stage-agent.*` and the existing ACP host. The release workflow builds the agent
folder on each platform's own runner (the only way to get that system's Node and native modules),
stages it through the existing checked helper, and records from the built installer bytes that the
agent is present. `scripts/ci/package_contents.py` and `scripts/ci/release_content.py` gain a third
distribution kind, `mastra-agent`, which is required for supported targets and still refuses the
legacy `mastra/` workflow runtime and enterprise content.

**Tech Stack:** Rust (ui-egui agent picker), Bash/PowerShell packaging, Python release gates,
GitHub Actions.

**Spec:** the session objective (INT-1503 with INT-1473), `docs/mastra-packaging.md`.

## Global Constraints

- No new agent runtime, launcher or second CAD registry; reuse PR #178's folder, lock and checks.
- No runtime dependency on a user's `node`, `npm`, `npx` or `PATH`: only the packaged Node.
- Telemetry disabled; no vector store, no embedding model; agent data stays in the app data folder.
- Never turn off a check, delete a test or hide a failure to make CI green.
- Code scanning passing is not a redistribution licence.

---

### Task 1: External ACP agents stay usable beside the built-in agent

**Files:**
- Modify: `crates/ui-egui/src/agent_chat/agents.rs`
- Modify: `crates/ui-egui/src/agent_chat/mod.rs`, `connections.rs`, `settings.rs`
- Test: `crates/ui-egui/src/agent_chat/tests.rs`

**Interfaces:**
- Produces: `pub fn is_builtin_entry(app: &SolveApp, entry: &Entry) -> bool`
- `entries()` returns the built-in agent first, then detected agents, then configured ones.

- [ ] Step 1: Test that a configured external ACP agent is listed after the built-in one and can be
      chosen, and that showing the panel still starts only the built-in agent.
- [ ] Step 2: Run it; it fails because `entries()` returns the built-in agent alone.
- [ ] Step 3: Add `is_builtin_entry`, stop the early return, keep detection running, use the helper
      where "built-in" meant "the agent running now".
- [ ] Step 4: Run the agent_chat tests.

### Task 2: macOS universal app carries one agent folder per slice

**Files:**
- Modify: `packaging/macos/package.sh`
- Test: `packaging/tests/mastra_agent_test.py` (staging contract)

- [ ] Step 1: Accept `--with-mastra-agent` for `--arch universal`, staging each target into
      `Contents/Resources/mastra-agent/<triple>` from `SOLIDCRAFT_MASTRA_AGENT_DIR_AARCH64` and
      `_X86_64`; sign every slice's Node and native libraries; check both slices in the mounted image.
- [ ] Step 2: `bash -n` the script.

### Task 3: Release workflow builds and stages the agent on every supported runner

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] Step 1: `SOLIDCRAFT_WITH_MASTRA_AGENT: "1"`, `SOLIDCRAFT_DISTRIBUTION: "mastra-agent"` for the
      supported jobs; add pinned Node + npm setup and an agent build/smoke step per platform.
- [ ] Step 2: Pass `--built-in-agent` to the installed checks on Linux, macOS and Windows.
- [ ] Step 3: Keep arm64/x86/arm64 Windows packages native and say so.

### Task 4: The gate proves the agent from the built installers

**Files:**
- Modify: `scripts/ci/package_contents.py`, `scripts/ci/release_content.py`
- Test: `scripts/ci/test_package_contents.py`, `scripts/ci/test_release_content.py`

- [ ] Step 1: Add failing tests: a `mastra-agent` package with the agent is accepted; without the
      Node, entry or dependencies it fails; a legacy `mastra/runtime.json` still fails.
- [ ] Step 2: Implement `agent_markers` and the `mastra-agent` distribution.
- [ ] Step 3: `python3 -m unittest` for both test files.

### Task 5: Documentation and skill

**Files:** `docs/mastra-packaging.md`, `skills/solidcraft-packaging/SKILL.md`

- [ ] Step 1: Say the release ships the agent by default, what each platform carries, and what is
      still a human decision.
