# Windows Packaged Assistant Startup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the v0.1.3 Windows x64 packaged Assistant initialize and perform a metered CAD operation on a clean install.

**Architecture:** Normalize only the verified packaged Node launch paths at the Rust spawn boundary. Add a separate Windows packaging step for the approved application-local Microsoft runtime, then exercise the resulting installed desktop and verify package contents before publication.

**Tech Stack:** Rust, Windows x64 Node 22.23.4, PowerShell, WiX, Python release-evidence tooling and a clean Windows 11 VM.

**Spec:** [Windows packaged Assistant startup](../specs/2026-11-26-windows-packaged-agent-startup-design.md)

## Global Constraints

- Start from the current protected `main` candidate (`37ee0c896400b99e1150695c38721d2032a0900f` at diagnosis, recheck before branching). Use an isolated task worktree and do not copy in the unrelated uncommitted SDK deduplication prototype.
- No change to the Node/npm/Cargo or solidcore pins, format 51, package matrix, desktop-only paid authority, private ACP protocol, external CLI refusal, or acceptance requirements.
- No first-run download, system-wide runtime install, or runtime from System32, WinSxS, PATH, or a third-party redistribution.
- Microsoft licensing and the exact redistributable input must be reviewed before shipping any new binary. If the input, local loader, or permitted terms cannot be verified, stop; do not accept a modified guest as proof.

## Review Focus

- Install path has spaces; canonical Windows form and subprocess argument semantics must both work.
- Canonical-to-launch conversion cannot redirect to another executable or entry, including paths the ordinary Windows namespace interprets differently.
- A hosted CI runner with global CRT must not mask missing or incorrect bundled bytes.
- The portable x64 package must carry the same runtime as the MSI, but no Agent-free package may start carrying it.
- A live provider reply is not evidence of user approval, an actual parameter change, or billing settlement.

### Task 1: Start the validated packaged process on Windows

**Files:** `crates/ui-egui/src/agent_chat/acp_client.rs` and its existing tests; `tools/windows-assistant-smoke.ps1` if its installed startup assertion needs tightening.

**Interfaces:** Keep `AcpBackend::packaged_agent(argv) -> Result<(PathBuf, PathBuf), String>` as the canonical validation step. Convert its returned paths only at `start_packaged` through a private `packaged_launch_path(path: &Path) -> Result<String, String>` helper.

- [ ] Reproduce the baseline with the installed Node on Windows: the canonical `\\?\` entry fails before ACP `initialize`; the normal entry reaches module loading. Keep this as diagnostic evidence, not acceptance.
- [ ] Add Windows tests in `acp_client.rs` using an actual fixture directory with spaces. A verified drive path must be absolute without a verbatim prefix when passed to Node. Confirm both launch paths canonicalize to the originally validated files. Keep the existing rejection of an out-of-package entry and missing SDK; add a rejection test for a path whose ordinary spelling does not round-trip. Unsupported namespace paths fail closed, not by stripping a prefix indiscriminately.
- [ ] Run the focused tests on Windows against baseline; require failure at the launch-path expectation, not at fixture setup. Implement the conversion using Windows path prefix components and a canonical round-trip equality check. Keep successful non-Windows launch arguments unchanged and retain strict UTF-8 conversion (no lossy paths).
- [ ] Run `cargo test --locked --profile ci -p solidcraft-ui-egui --lib -- agent_chat::acp_client::` on Windows and Linux, plus crate Clippy and fmt. Expected: all selected tests pass, with a nonzero test count on both platforms. Commit the scoped launch fix.

### Task 2: Carry and verify the Microsoft x64 runtime

**Files:** `packaging/windows/package.ps1`, `packaging/windows/installed-check.ps1`, `scripts/ci/{package_contents,test_package_contents}.py`, `docs/mastra-packaging.md`, Microsoft notice and a checked-in provenance record under `packaging/windows/`. Keep existing WiX harvesting and ZIP staging unless the actual package test proves a gap.

**Interfaces:** One reviewed Windows runtime receipt names the redistribution source/toolset version, AMD64 file basenames, file versions and expected SHA-256, and the applicable Microsoft terms/list. Stage reviewed files next to `mastra-agent\node.exe`, with the notice in `mastra-agent\licenses\`. The release content check requires their names only for Windows x64 MSI/ZIP packages that declare the built-in Agent. Stage, installed and portable readback checks independently verify the actual DLL bytes; the content inventory alone does not.

- [ ] Select the licensed Microsoft source, confirm the redistribution rights for the exact product/toolset and file list, and determine the minimal load set in a disposable test. Start with the observed `VCRUNTIME140.dll` import. Verify machine type, Microsoft signature, exact hash/version and that the packaged libSQL loads that copy on a clean guest without a global runtime. Check the file's servicing/upgrade ownership. Stop on missing legal or loader evidence.
- [ ] Add RED package-content fixtures for missing runtime or notice and for runtime present in an Agent-free package. Preserve existing tests for Agent absence on x86/arm64. Add Windows staging checks for a wrong hash and PE machine type, plus a positive exact-byte receipt check. Run `python3 -m unittest discover -s scripts/ci -p test_package_contents.py` and the Windows staging checks against the old layout; expected: the new checks catch the missing runtime.
- [ ] Add `Stage-AgentVCRuntime -AgentDirectory <dir>` in `packaging/windows/package.ps1` or a Windows helper sourced only there. Validate the reviewed bytes before copying, apply it only in the existing x64 Agent stage, and keep the same staged files for both MSI and ZIP. Missing or unapproved inputs fail the build with a diagnostic, not an online fallback or a weaker assembly check.
- [ ] Make installed x64 validation compare the runtime with the reviewed receipt, then run the existing network-isolated installed desktop startup, stop and reconnection smoke. On the clean guest, explicitly assert the runtime DLL's loaded path in the actual installed Node child (not simply that Node happens to start). Test MSI and portable readback and add the Windows x64 Agent files to the existing package contents evidence without reducing other checks.
- [ ] Run the same contents suite GREEN, existing packaging tests, exact MSI and ZIP build/readback and clean-guest startup. Review the new binary records and required notices; commit the packaging change separately.

### Task 3: Rebuild, accept and publish through the protected flow

**Interfaces:** Uses production launch behavior from Task 1 and the verified Windows x64 package layout from Task 2. Release evidence must belong to the new frozen source and its own run/attempt, not `37ee0c89`.

- [ ] Review both commits; follow the existing task/promotion PR gates, full candidate gate and approved merge. Freeze new `main`; build staging and match descriptors, checksums and embedded staging config.
- [ ] On exact staging packages, do Linux first then Windows. Use Ask mode; verify the native **Allow once** card precedes the write. Independently read `int1481_acceptance_marker = 13 mm` via native Parameters without issuing a billed Control readback. Require one bridge admission and terminal `succeeded → charged`. Check Agent stop/restart, offline and sign-out refusals with no new mutation or admission, CAD save/reopen and the existing external CLI negative. Verify macOS mounted image/both slices and CI smoke (real Mac and Gatekeeper remain **NOT TESTED**).
- [ ] Leave tags untouched until installed gates pass and the release executor confirms the unpublished-tag exception still covers the new source. Run one full formal tag attempt; verify exactly 14 matching production pairs, actual bytes/notices and exact human approvals. Preserve the independent protected signing approval, verified 51-asset draft and separate human publication.
- [ ] Verify public assets and website, reconcile `main → develop` and hand off to the Migration Agent per the existing release runbook.
