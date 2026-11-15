# Packaged Mastra runtime

Status: **packaging scaffolding only — the runtime artifact is NOT approved, and bundling is
opt-in.** Four shipped `@mastra/core/dist/ee-*` files carry the Mastra Enterprise License (DQ-7),
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
| Linux `.deb`, `.rpm`, `.tar.gz`, AppImage, snap | `/usr/lib/solidcraft/mastra` (inside the image/sandbox: the same FHS path) | `usr/bin/solidcraft` → `../lib/solidcraft/mastra` |
| Flatpak | `/app/lib/solidcraft/mastra` | `/app/bin/solidcraft` → `../lib/solidcraft/mastra` |
| macOS `.app` (single architecture) | `SolidCraft.app/Contents/Resources/mastra` | `Contents/MacOS/SolidCraft` → `../Resources/mastra` |
| macOS `.app` (universal) | `Contents/Resources/mastra/aarch64-apple-darwin` and `.../x86_64-apple-darwin` | the matching slice by host target |
| Windows MSI and portable zip | `mastra\` beside `solidcraft.exe` | beside the executable |

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
