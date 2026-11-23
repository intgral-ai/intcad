# Release readiness: 0.1.3 (INT-1503)

Owner: release manager. Written 2026-11-23. This file is a decision record, not release evidence
for a candidate: no candidate exists yet. Every claim below names the command, file or run that
shows it; anything unverified says `UNKNOWN`, `BLOCKED` or `NOT TESTED`.

## 1. Baseline

| Item | Value | How it was checked |
|---|---|---|
| Integration branch | `origin/develop` = `1c3102f977e1dbfb8b3df009f6d07166385bd844` (2026-11-23 00:08:31 UTC) | `git ls-remote origin refs/heads/develop`; `gh api repos/intgral-ai/intcad/branches/develop` |
| Last public release | `v0.1.2`, tag `65af7346fbac9b892c84161d29347e405d557a2f`, published 2026-11-16 | `gh api repos/intgral-ai/intcad/releases/tags/v0.1.2` |
| `v0.1.2` assets | `intcad-0.1.2-{linux-x86_64.tar.gz, macos-universal.dmg, windows-x64.msi}`, the unversioned `intcad-{macos-universal.dmg, windows-x64.msi}` aliases, `SHA256SUMS` | same call |
| Version in the tree | `0.1.2` (`[workspace.package] version`, `Cargo.toml:8`) | `rg -n '^version' Cargo.toml` |
| Candidate version | **UNKNOWN — owner decision.** INT-1503 names both `0.1.3` (first, without login) and `0.1.4` (later, with login) and says which carries what was not settled | INT-1503 body |
| Free space | 520 GB available on `/home`; IntCAD tree 6.1 GB | `df -B1 .`; `du -sx --block-size=1 /home/hyjing/Code/intgral/intcad` |

**What the published `v0.1.2` asset actually contains** (downloaded from the release, hashes checked
with `sha256sum --check SHA256SUMS`, then listed locally):

- Linux tarball: 49 entries, `bin/intcad` (213,022,728 bytes) and `bin/intcad-cli`
  (138,988,736 bytes), icons, desktop and AppStream files. **No `mastra`, `mastra-agent`, `node` or
  `.agents` entry of any kind.**
- Windows MSI: two CABs holding 8 files in total — `SolidcraftExe` (229,128,704 bytes, the app),
  `SolidcraftCliExe` (156,839,424 bytes), the licence and third-party notices, and the WebView
  platform notices. **No runtime folder.**
- So the next release's Mastra and agent work changes what the MSI/tarball contain, not just the
  app's behaviour, and the checksum list must be regenerated for it (`packaging/checksums.sh`).

`v0.1.2` is the only reference point for the upgrade path: a 0.1.3 installer must be able to update
an installed 0.1.2, and the 0.1.2 client's own update check is what has to work (see §6, E1).

## 2. Scope and owners

| Workstream | Owner | State | Integration condition |
|---|---|---|---|
| Login, accounts, capability authorization (INT-1497) | Huaiyuan | PR #213, draft, head `22f427c8c0de3aec42299edc8c70056d8c86a870` | CI green on the promotion SHA, then the Gate B checks |
| Modelling skills in the built-in agent (#198, INT-1473 prerequisite) | Tianwei | PR #204, open, head `b2cb0ef0e0317c9273f771ade89e1c8892331778` | Review comments closed, Text-to-CAD as a second set, installed-app evidence |
| Agentic behaviour over the right rail (INT-1473) | Tianwei | PR #212, open, head `31eae3957c35f78381ef9b1a4cab70b09b6465f9`, based on #204's branch | #204 merged first, then rebase onto `develop` |
| UI changes | PP | **No PR exists.** No open PR is authored by `pp`; the only `paipeline` PR that is not a sync is #189, the verification harness (`.agents/skills/verify-intcad/**`), not a UI change | PR opened, reviewed, visual regression run |
| Release engineering, packages, RC, acceptance | this role | This branch `int1503-release-readiness` from `develop` (PR #217; `release/` is reserved for a `release/vX.Y.Z` promotion by the branch-admission policy, which rejected the first branch name) | — |
| Version scope, promotion, publishing | CTO | — | Explicit approval before any tag or GitHub Release |

Intcad ships the same three platforms as `v0.1.2` (Linux x86_64, Windows x64, macOS universal).
The workflow also builds Linux aarch64, Flatpak, Arch, Snap, FreeBSD, Windows x86/arm64 and web,
and `package-builds` requires **all** of them, so a FreeBSD failure blocks the stable path (§6, B2).

## 3. Packaging audit

The Assistant's page is embedded in the executable, so there is no separate front-end payload to
lose. The parts that can go missing are Mastra, the skills and the checksum list.

| Resource | Comes from | Build step into the package | Installed location | Found at runtime by | Proof it works |
|---|---|---|---|---|---|
| `intcad` / `intcad-cli` | `apps/solidcraft`, `apps/solidcraft-cli` | `packaging/{linux,macos,windows}/package.sh|ps1` | tarball `bin/`, MSI `C:\Program Files\Intcad`, DMG `.app/Contents/MacOS` | the OS, PATH from the installer | for `v0.1.2`: the DMG mounted and the copied-out app ran `--version` in CI (run 43859919979); installed-and-started VM runs exist at other commits only (§4). **No candidate: NOT TESTED** |
| Assistant page (HTML/JS/CSS/fonts) | `packages/assistant-web/dist` (checked in) | compiled in: `apps/solidcraft/src/assistant_web.rs:70-75` (`include_bytes!`) | inside the executable | the app's own custom scheme | `packaging-lint.yml` job `assistant-page` rebuilds the page from the pinned Mastra commit and fails unless it matches byte for byte |
| Web view | system GTK 3 + WebKitGTK 4.1 (Linux), Evergreen WebView2 (Windows), WKWebView (macOS) | nothing is bundled; the deb/rpm declare the Linux libraries | system | `wry` | the mechanisms exist and have not been run for a candidate: `packaging/linux/installed-check.sh`, `packaging/windows/installed-check.ps1`, `packaging/macos/installed-check.sh`, the Snap content-snap check and the Arch `.PKGINFO` check. **Windows installers do not carry WebView2** (`packages/assistant-web/README.md:161`) |
| Mastra runtime | `packages/mastra-runtime` | `packaging/mastra/stage.sh` → `mastra/` in the package, **opt-in** | beside the executable | relative path from the executable | **Not in any published package**, shown by listing the `v0.1.2` tarball and MSI (§1). `release.yml:29` sets `SOLIDCRAFT_WITH_MASTRA: "0"`; `packaging/linux/package.sh:26` and `packaging/macos/package.sh:85` default to 0. PR #214 changes this |
| Built-in agent folder (`mastra-agent/`) | `packages/mastra-runtime/agent` + `node` binary | `packaging/mastra/stage-agent.sh` → `mastra-agent/`, opt-in | beside the executable | the app starts `node.exe agent/main.mjs` there | `agent-smoke.mjs` + `agent-package.mjs scan` inside the package step; PR #214 only |
| Four modelling skills | `skills/solidcraft-{modeling,editing,checking,mechanisms}` | `agent-package.mjs build` copies them to `skills/modelling-skills/` with `skill-set.json` | at first start into `<MASTRA_APP_DATA_DIR>/home/.agents/skills` | Mastra Code's own skill discovery | PR #204 only. **No installed-app evidence yet**; the smoke test proves discovery in a package-shaped folder |
| Icons, version, AppStream | `assets/platforms/**`, `[workspace.package] version` | `icons` and `version` jobs; `build.json` per package | in the package and its metadata | the OS / `build.json` | `packaging-lint.yml` XML/desktop/AppStream checks; `scripts/ci/test_gate.py` |
| Update check | `crates/ui-egui/src/update/**` | compiled in | inside the executable | GitHub Releases API | unit and scenario tests; **cannot work against `v0.1.2` as published** (§6, E1) |

**The four skills, as the package would carry them** (checked on `develop`, and again in #204's
tree):

| Skill | Source | Front matter `name` | Installed to (with #204) |
|---|---|---|---|
| `solidcraft-modeling` | `skills/solidcraft-modeling/SKILL.md` | `solidcraft-modeling` (matches the folder, as Mastra requires) | `<MASTRA_APP_DATA_DIR>/home/.agents/skills/solidcraft-modeling` |
| `solidcraft-editing` | `skills/solidcraft-editing/SKILL.md` | `solidcraft-editing` | same root, `solidcraft-editing` |
| `solidcraft-checking` | `skills/solidcraft-checking/SKILL.md` | `solidcraft-checking` | same root, `solidcraft-checking` |
| `solidcraft-mechanisms` | `skills/solidcraft-mechanisms/SKILL.md` | `solidcraft-mechanisms` | same root, `solidcraft-mechanisms` |

Runtime requirements: the agent needs Node 22.19 or later (`packages/mastra-runtime/agent/README.md:27`;
the packaged workspace pins `>=22.23.4 <23`), is started from `agent/main.mjs` beside the app, and
installs the set only when `MASTRA_APP_DATA_DIR` is set — run by hand without it, nothing is
installed. `packages/mastra-runtime/agent/skills.mjs`, the registry and the installer exist only in
#204: `develop` has no code that puts a skill anywhere, so Gate C has nothing to test until #204
and #214 are in the candidate.

Three answers therefore matter for this release: Mastra and the agent must actually be in the
package (#214), the four skills must be installed and called from an installed build (#204, #212),
and the checksum asset must carry the name the client fetches (§6).

## 4. CI and E2E audit

What exists and what it proves:

| Layer | Runs where | Proves | Does not prove |
|---|---|---|---|
| `packaging-lint.yml` | GitHub-hosted Ubuntu | workflow syntax, shell/PowerShell parse, desktop/AppStream metadata, the Assistant page rebuild, packaging regressions, and (new, this branch) the release checksum list | that a package installs |
| `release.yml` (`packages` in `release-gate.yml`) | GitHub-hosted, per platform | the package builds, its descriptor (`build.json`) and contained files | that it starts, or that a runtime resource is found |
| `installed-check.sh` / `installed-check.ps1` / `assistant-check.py` | inside those jobs | the installed app opens the Assistant page with the network taken away | real hardware, signing, SmartScreen or Gatekeeper |
| VM acceptance (`/home/hyjing/Code/intgral/intcad/.dev/e2e/bin/intcad-e2e`) | local Linux and Windows VMs | a real desktop session: the app starts, the Assistant opens and closes, screenshots are kept | **the published artifact**: `accept` builds the checkout and copies a stripped executable on Linux, or a GUI-only MSI on Windows. There is no `--artifact`/`--no-build` option (checked with `intcad-e2e --help` and the wrapper) |
| macOS | GitHub-hosted `macos-15` | the DMG mounts, both slices are present, the copied-out app runs `--version` (v0.1.2 run 43859919979) | quarantined first launch, notarization, native Intel hardware |

Local VM status on this tree: `intcad-e2e --source <this worktree> --task rc-audit status` and
`doctor` report both task VMs **not created**; no build exists for the baseline SHA; the golden
images and the Linux builder are present. Nothing was booted, because there is no candidate to
test (§6, E2).

Evidence that exists for older commits (not transferable to a new candidate):
`~/.local/state/intcad-e2e/runs/intcad-e2e-int1454-ui-c28767d6c085-int1454/20261118T212022Z-681395/REPORT.md`
(Linux, 8/8 PASS, direct executable, `42f4d670`), and
`~/.local/state/intcad-e2e/windows/runs/intcad-win-int1488-06d740216329-int1488/20261118T053729Z-1459234/REPORT.md`
(Windows, 8/8 PASS, GUI-only MSI, `4a78ac48`). Both are older than the baseline and were not
re-run.

## 5. PR status and review of the skill work

**#204 (read first, as agreed).** Reviewed at `b2cb0ef0e` against #198 and the review request in
[PR comment](https://github.com/intgral-ai/intcad/pull/204#issuecomment-7067826763). Findings, all
reported to the author rather than fixed here because the code is his:

1. **Merge gate, not a regression:** `intcad-text-to-cad` must be a second entry in `SKILL_SETS`
   with the same build, install and drift checks. Today `agent/skills.mjs` lists only
   `modelling-skills`; the test merely preserves a hand-made Text-to-CAD folder.
2. **Merge gate:** the requested installed-app evidence does not exist. The PR states no screenshot
   was taken and the app was not checked; the smoke run used a package-shaped folder.
3. **Defect:** `agent/skills.mjs:131` calls `readdirSync` outside its error handler, so a packaged
   `skills` path that is a file or unreadable aborts the agent before Mastra starts. The repository
   rule is "never crash".
4. **Defect:** `scripts/agent-package.mjs` validates names, hashes and version but not
   `stamp.set`; startup skips a set whose stamp names another set, so a stamp-only mistake passes
   the package check and ships an agent with no skills.
5. **Smell:** `agent-smoke.mjs` re-implements the package/checkout layout that `agent-package.mjs`
   already computes.

**#212** is based on #204's branch (`baseRefOid` = `b2cb0ef0e`), so it cannot merge before #204 and
will need a rebase onto `develop` afterwards. Its known issues are listed in the PR; two of them
(a corrupt PDF poisoning the rest of a conversation; a design switch not reaching the agent until a
new chat) are user-visible and belong in the release notes if #212 ships.

**#214** is the release-engineering PR that makes packages carry the built-in agent. It is not mine
to merge, and it overlaps the packaging files I would otherwise edit, so this branch deliberately
does not touch `release.yml`, `release-gate.yml` or the packaging scripts. Its last package run
(45112837415) built `77914e22`, a merge/test SHA rather than the PR head: Linux x86_64/aarch64,
macOS universal, Windows x64/x86/arm64 and Arch all passed there, FreeBSD failed.

## 6. Blockers

Severity follows the release brief's section 7: P0 blocks, P1 blocks unless the release owner
explicitly excepts it, P2 may be deferred.

**State on 2026-11-23 after the CTO's decisions:** #204 merged (`41089b60`). Three PRs are open and
in the merge chain: #218 (cookie 0.18.3 for RUSTSEC-2026-0594; its `Dependency advisories` job is
already green, and its branch was updated onto the new `develop`), #212 (INT-1473, retargeted to
`develop` and updated to `6c7b81ab`), and #219 (FreeBSD compile fix, with `release.yml` run
45230019033 building the FreeBSD job). The 0.1.3 version commit is prepared on
`int1503-version-013`. #214 still needs its own two fixes; #213 still fails a Windows check; PP's
UI has no PR.

| # | Sev | Blocker | Evidence |
|---|---|---|---|
| B1 | P0 | **No candidate artifact.** `release.yml` still builds without the agent (`SOLIDCRAFT_WITH_MASTRA: "0"` sets only the workflow runtime; nothing sets `SOLIDCRAFT_WITH_MASTRA_AGENT`), and #214, which adds it, is an unmerged draft. The rehearsal in §12 shows the local path works, so this is now a merge-and-run question, not an unknown | `release.yml:29`, `packaging/linux/package.sh:26`, PR #214 state, §12 |
| B2 | P0, fix in #219 | **FreeBSD package build fails, and `package-builds` requires it.** `apps/solidcraft/src/main.rs` called `egui_winit::set_emit_empty_paste_events` unconditionally while `apps/solidcraft/Cargo.toml` declares `egui-winit` only for Windows/Linux/macOS. #219 puts the dependency's `cfg` on the call; `release.yml` run 45230019033 on that branch is building FreeBSD as this is written | run 45112837415; #219 |
| B3 | P1 | **Login PR #213 has a failing required check on Windows.** `Windows app startup` → `tools/windows-resize-smoke.ps1`: `WM_NCHITTEST failed (Win32 error 1460) at (20,330) within the 2 s limit` after the window was moved to 20,20–1004,640. The same job passes on `develop` `1c3102f` (run 45087015186) and on `56e805f` (run 45035941855), so it is a regression in #213's range, not a flake observed twice | run 45109730998 job 137479303661 |
| B4 | P1, code merged | **Skills need a package that carries the agent.** #204 merged into `develop` (`41089b60`), so the code that installs the four sets is in; a *release* still needs #214, which puts the agent folder into the packages | #204 merged, §12, §12.1 |
| B5 | P1 | **The slash list is not seen in the Assistant.** The agent registers the four `skill/<name>` commands (ACP-level, `agent-smoke.mjs`, §12), but drawing them needs a model signed in: the panel shows the Connections page, and no credential is available to this role | §12, §12.1 |
| B6 | P1 | **The Mastra workflow runtime (`mastra/`) is still not in any package.** Only the agent folder ships. `--with-mastra` stays opt-in and fails closed until the runtime artifact is approved (DQ-7/#15), so "Mastra in the package" is true for the agent and not for the workflow runtime | `packaging/linux/package.sh:101-118`, `docs/mastra-packaging.md` |
| B8 | P0 | **Every package build fails on a dependency advisory.** `Dependency advisories` fails with RUSTSEC-2026-0594: `cookie 0.18.2` via `wry 0.57.2` (private-cookie nonce reuse after `fork()`, which breaks AES-GCM). `release.yml` cannot pass for any branch until it is fixed; #218 moves it to 0.18.3 | run 45149031980 job 137606035774 |
| B9 | P0 (scope) | **PP's UI has no PR.** The CTO requires it in 0.1.3, and there is no branch or PR to review, so this scope item cannot start from the release side | open PR list, 2026-11-23 |
| E1 | P1, fixed on this branch | **The update check could not upgrade `v0.1.2`.** `install.rs` required an asset named `SHA256SUMS.txt`; `v0.1.2` publishes `SHA256SUMS` only, so `run()` stopped before downloading. The client now accepts either name and still verifies the hash; the release process writes both | release asset list; `install.rs` `SUMS_LEGACY`; the new test in §7 |
| B7 | P1 | **The Windows candidate has no agent.** A Windows 0.1.3 exe and MSI now build here (§12.4), but the agent folder for Windows must be built on Windows (its `node.exe` has to run for the version check), and this VM has no Node; `HasMastraAgent=0` was used. The Windows agent is what `agent-test-packages.yml` builds on Windows runners and what #214's gate checks | §12.4, `agent-package.mjs` line 473 |
| E2 | P1 | **No installed-artifact acceptance path for a candidate.** The VM harness builds from source; `accept` cannot take a package. Linux/Windows installed checks exist in CI and in `packaging/*/installed-check.*`, but they have not been run for a 0.1.3 candidate | §4; `intcad-e2e --help` |
| E3 | P2 | Portable-ZIP updates replace only `intcad.exe` and `intcad-cli.exe`, so a `mastra-agent/` folder from #214 would be left stale or absent | `install.rs:302` `apply()`, `Method::PortableZip` |
| E4 | P2 | `.github/workflows/verify-beta-installers.yml` hard-codes `C:\Program Files\SolidCraft` and `SolidCraft.app/Contents/MacOS/SolidCraft`, so it cannot verify an Intcad release | lines 40 and 73 of that workflow |
| E5 | P2 | `packaging/windows/package.ps1:186` tests `$CanRun` and `$HostArch`, which the script never sets, so the `intcad-cli --version` smoke there never runs (it is silently skipped) | the two lines; no assignment anywhere in the file |
| E6 | P2 | PP's UI work has no PR, so its integration risk is unknown rather than assessed | open PR list |

## 7. Fixes made on this branch

One release-engineering defect is closed here, because it is in this role's ownership, touches no
file another workstream is editing, and can be verified without a release:

- `packaging/checksums.sh` + `packaging/tests/checksums_test.py`: write the release checksum list
  deterministically (sorted, every hash checked) under **both** `SHA256SUMS.txt` (what the updater,
  `packaging/web-channels.sh` and `packaging/flatpak-site.sh` fetch) and `SHA256SUMS` (what earlier
  hand-made releases published). `docs/beta-release.md` step 5 now uses it, and
  `packaging-lint.yml` runs the test. This makes a correct list reproducible and removes
  the release-side cause of E1 for future releases.

The other half of E1 is a client change in `crates/ui-egui/src/update/install.rs`: the update check
now reads `SHA256SUMS.txt` and falls back to `SHA256SUMS`, the name `v0.1.2` published, and still
refuses anything whose hash does not match. Regression test:
`a_release_that_lists_the_sums_under_the_old_name_still_updates` in
`crates/ui-egui/src/update/install_tests.rs`, written first (it failed to compile with `SUMS_LEGACY`
missing). Passing locally on 2026-11-23 with a private target directory:
`cargo test --profile ci -p solidcraft-ui-egui --lib --locked a_release_that_lists_the_sums_under_the_old_name_still_updates`
→ 1 passed; the whole `update::install` group → 10 passed; `cargo clippy --profile ci -p
solidcraft-ui-egui --all-targets -- -D warnings` → no issues. The PR's `Native Linux selected
regression gate` job does **not** cover this test for this change set (it selects `xtask` only).
The update owner can veto this single change without touching the checksum script.

Against the real release: the two `v0.1.2` assets downloaded in §1, run through
`packaging/checksums.sh`, produce lines byte-identical to the published `SHA256SUMS` for the same
files, and both generated names are identical (`diff` of the published lines: empty;
`cmp SHA256SUMS SHA256SUMS.txt`: equal).

Verification on this machine: `python3 -B -m unittest packaging/tests/checksums_test.py` → 6
tests, OK (`nightly_test.py` fails only without `jq` on `PATH`, and passes with it; the other
suites below are unchanged). Re-ran, unchanged and passing: `nightly_test.py` (12), `apprun_test.py`,
`flatpak_test.py`, `web_channels_test.py`, `mastra_runtime_test.py`, `mastra_agent_test.py`,
`scripts/ci/test_gate.py`. `windows_icon_test.py` and the workflow's PowerShell parse need
PowerShell, which this machine does not have: they run in `packaging-lint.yml` and are NOT TESTED
here. `actionlint .github/workflows/packaging-lint.yml` → exit 0.

## 8. Integration order once the workstreams land

1. Merge #204 (skills) after the two merge gates in §5 are closed; then rebase #212 onto
   `develop` and merge it. Both keep their own checks.
2. Merge #214 (packages carry the agent), which also fixes the FreeBSD compile failure's
   neighbourhood; confirm `package-builds` is green on the merged SHA, including FreeBSD.
3. Merge #213 (login) after its Windows check passes; the FreeBSD fix from B2 must already be in
   `develop` or #213's own CI will inherit the same failure.
4. Merge PP's UI PR when it exists, after a visual pass on Linux and Windows at 100 % and 150 %.
5. Cut the version commit, take the candidate SHA, run the three-platform package build, then
   install and accept that exact SHA on all three platforms (§9 acceptance matrix).
6. Only then ask the CTO for promotion and publication. This branch does none of that.

## 9. Acceptance matrix for the candidate (not yet run)

Rows marked *rehearsal* are the §12 tree (`develop` + #204 + this branch), not a versioned
candidate: they show the path works, and they must be repeated for the candidate SHA.

| Platform | Build | Package | Startup | Login | Mastra | Skills + commands | UI | Update | Evidence |
|---|---|---|---|---|---|---|---|---|---|
| Linux x86_64 | 0.1.3 candidate built (Ubuntu 24.04, 5m25s) | 0.1.3 tar 462 MB, sha256 `e30b1cf8…`, checksum list written | PASS (0.1.3 window, sample design) | NOT TESTED | 0.1.3 agent ships, starts from the install prefix, installs the four skills; workflow runtime absent (B6) | 0.1.3 agent smoke 8/8 with the four `skill/<name>` commands; slash list not seen (B5) | Assistant page renders | one-click path NOT TESTED (feed URL is a constant); in-place 0.1.2→0.1.3 PASS, design saved by 0.1.2 opens in 0.1.3 | §12.3 |
| Windows x64 | 0.1.3 exe + cli built (cargo-xwin, 3m40s + 3m36s) | 0.1.3 MSI built in the guest (WiX 5.0.2), 100.7 MB, sha256 `894e04fb…` | PASS (window `v0.1.3`, sample design) | NOT TESTED | **no agent** (B7) | NOT TESTED | rail + Assistant page render | one-click NOT TESTED; in-place MSI 0.1.2→0.1.3 PASS (one ARP entry, `intcad-cli 0.1.3`) | §12.4 |
| macOS universal | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | CI-only DMG mount, no DMG for a candidate yet |

Verdict: **NO-GO**. B1–B4 alone are enough; E1 means even a perfect 0.1.3 package would not be
reachable by one-click update from 0.1.2.

## 10. How each gate will be run

Each line names the command or the person; nothing here has been run for a candidate yet. A gate
row is only PASS with the artifact, the SHA and the evidence path recorded beside it.

**Gate A, build and package.** `release.yml` on the candidate SHA (through `release-gate` for a
`main`-target PR). PASS needs: every job in `package-builds` green including FreeBSD, `build.json`
naming the candidate SHA for each target, Mastra and the agent folder present in each installer
(the `package_contents` scan, once #214 lands), and `packaging/checksums.sh dist/<platform>` run
over the downloaded assets with every hash OK.

**Gate B, login.** The functional part belongs to the login workstream on real accounts and real
WorkOS/Railway configuration; this role only verifies what the installed package does. Map to PRD
§9 and record per platform:

1. Anonymous or offline: CAD, completion, save/export, undo/redo work — installed app, network
   off.
2. Anonymous cannot reach a controlled capability through any entry: GUI, `intcad-cli mcp`,
   control channel, CLI. Each must return a recognizable error, never open a browser.
3. Email + password sign-up, sign-in, email verification, password reset, sign-out, account
   switch, deletion request: real email account, system browser, and AuthKit shows no Google or
   GitHub entry.
4. Signed-in, online, authorized: the existing controlled capabilities work with the user's own
   key.
5. A version the server no longer authorizes loses only the controlled capabilities.
6. A long MCP/control session cannot run forever on the authorization it started with.
7. Network loss mid-task stops further scheduling within the agreed limit.
8. Sign-out, account switch and authorization withdrawal: queued, awaiting-approval and late
   results cannot write; finished work stays.
9. Network return does not replay a write of unknown state.
10. Authorization traffic and logs carry no model key, full token, project content, prompt or
    parameter.
11. The three platforms behave the same, and nothing unimplemented is presented as available.

**Gate C, skills.** In the installed app, on a clean profile: the four skill folders exist under
`<MASTRA_APP_DATA_DIR>/home/.agents/skills` (and `intcad-text-to-cad` beside them once #204 adds
its set); each `SKILL.md` is byte-identical to the package's copy (the `skill-set.json` stamp);
`skill_search` finds them; one request per skill makes the agent read and use it; and the
`skill/<name>` commands the Assistant actually shows are recorded as a screenshot. A file on disk
is not evidence that the skill can be called.

**Gate D, UI.** Installed app at 100 % and 150 % on Linux and Windows (and a Mac if a machine is
available): every built-in theme, font and control size, no unexpected background or clipping, the
Assistant web view positioned correctly, panel resize and move, and a basic CAD operation on each.

**Gate E has a sequencing constraint that cannot be engineered away.** The one-click path can only be
exercised against a release GitHub serves as `releases/latest`, and the client refuses a
`prerelease` (it uses `releases/latest`, and `Release::parse` rejects `prerelease: true`). A local
feed is possible (`SOLIDCRAFT_UPDATE_FEED`), but every asset URL in it must sit under
`https://github.com/intgral-ai/intcad/releases/`, so a local payload needs DNS and TLS interception
in the guest. Practical consequence: the first real test of the one-click update is **after** the
stable release exists — install the published 0.1.2, let it find 0.1.3, and let it download, verify
and apply. Because a published release cannot be replaced, that test comes last, and its failure
would mean a follow-up patch release rather than a re-tag. Everything before it (feed fetch, asset
selection, checksum naming, verification) is covered by unit/scenario tests and by #218/#217.

**Gate E, upgrade.** Install the published `v0.1.2`, then update to the candidate with the
one-click path and record: the update check finds the release, the download matches the checksum
list, the package applies, the app restarts, a design from 0.1.2 opens, preferences survive, and
the login and skill state matches the product design.

What the installer already guarantees, from the source rather than a run: the MSI keeps one
`UpgradeCode` with `MajorUpgrade AllowSameVersionUpgrades="yes"`
(`assets/platforms/windows/solidcraft.wxs:25,31`), so a new MSI replaces the old install instead of
sitting beside it, and a downgrade is refused; the scope is `perMachine`, so the install needs
administrator rights. The tarball, deb, rpm, Flatpak and Snap are never replaced in place
(`install.rs` `method()`), so for those the record is a reinstall or a package-manager upgrade,
and the one-click path only has to be proven for the AppImage, the MSI, the portable zip and the
DMG. With E1 fixed on this branch the 0.1.2
client can read a new release's list only if that release also carries `SHA256SUMS` (the 0.1.2
binary knows that name); the release must therefore publish both names, which `checksums.sh` does.

**Gate F, evidence.** Keep: the `release-gate` run URL for the candidate SHA, the package artefact
names and hashes, `build.json` per target, installed-check logs and screenshots, the VM E2E
`REPORT.md` paths, the known-issues list and every NOT TESTED or BLOCKED row. No row is PASS
without its file.

## 11. Published-artifact rehearsal (Linux, 2026-11-23)

Done to prove the acceptance route before a candidate exists, using the **published** `v0.1.2`
tarball, not a source build. Task `rel012`, VM
`intcad-e2e-release-readiness-nov23-378d722a8f6a-rel012` (Ubuntu 24.04.5, XFCE, X11, llvmpipe).

| Step | Result |
|---|---|
| Extract `intcad-0.1.2-linux-x86_64.tar.gz` in the guest | `bin/intcad`, `bin/intcad-cli` |
| `bin/intcad-cli --version` / `bin/intcad --version` | `intcad-cli 0.1.2` / `intcad 0.1.2`, exit 0 |
| Launch `bin/intcad --control 7879 --sample` on `DISPLAY=:0` | window 1440×900, `Vulkan · llvmpipe`, control server on 127.0.0.1:7879 |
| `ui.inspect` | one document, `Plate` body, face hover, viewport 1440×866 |
| `ui.click` on handle `titlebar:assistant` | Assistant panel opens |
| Whole-VM screenshot | Assistant page rendered: "No agent found / Set up an assistant", agent list, composer |
| `engine.execute` `sketch.create`, `sketch.rectangle.two_point`, `solid.extrude {distance:20}` | `body: Body1`, `ms:17.9`, no warning; the solid appears and the browser lists 2 bodies |

Evidence (this machine):
`~/.local/state/intcad-e2e/runs/intcad-e2e-release-readiness-nov23-378d722a8f6a-rel012/20261123T064236Z-2372381/screenshots/rel012-assistant.png`
and `rel012-extrude.png`.

Two method findings that change how Gate C/D evidence must be taken:

1. **`ui.screenshot` cannot see the Assistant page.** It captures egui's own frame, so the native
   web view (WebKitGTK/WebView2/WKWebView) comes out as a black rectangle — the repository's own
   passing run has the same black `app-frame.png` while its `screen.png` shows the page. A black
   panel in a control screenshot is **not** a UI failure. Use the whole-VM screenshot
   (`intcad-e2e shot`) for anything inside the Assistant, or `packaging/assistant-check.py`.
2. **No rebuild is needed to test a package.** Extracting the release tarball in a fresh VM and
   launching it by hand with the harness's environment (display, `SOLIDCRAFT_CONTROL_DIR`,
   `--control`, `--sample`) exercises the real published binary; `intcad-e2e accept` cannot,
   because it builds the checkout first. This is the path to use for the candidate; making it a
   scripted command is still open (E2), since the launch flags and the web-view capture had to be
   worked out by hand here.

Limits: Linux X11 on software rendering only; no Windows or macOS equivalent was run, and nothing
here proves login, Mastra or the skills, which `v0.1.2` does not carry.

## 12. Candidate-shaped package rehearsal (Linux, 2026-11-23)

Built to answer the two questions no document can: does the package carry the agent and the four
skills, and does the agent that ships start and expose them. Tree: `develop` `1c3102f` + #204
`b2cb0ef` + this branch, merged in a separate worktree `worktrees/int1503-rc-rehearsal` (nothing
pushed, nothing merged).

| Step | Result |
|---|---|
| `node packages/mastra-runtime/scripts/agent-package.mjs build --target x86_64-unknown-linux-gnu` | 782 npm packages, 10,733 files, 275,950,178 bytes, skills set `modelling-skills` = `sha256:0d7b9a61…`, no enterprise content, licence inventory written; needed Node 22.23.4 exactly, which this host lacks (see limits) |
| `packaging/linux/package.sh --skip-build --formats tar --with-mastra-agent` | agent staged (291 MB), `check` and secret `scan` pass |
| Tarball | 210.9 MB, **13,597 `mastra-agent` entries**; the four SKILL.md and `skill-set.json` at `lib/intcad/mastra-agent/skills/modelling-skills/` |
| Extracted on clean Ubuntu 24.04.5 (VM `…int1503-rc-rehearsal…rel012`) | the packaged `node` runs: `v22.23.4`; skills folder present |
| Start the packaged agent with `MASTRA_APP_DATA_DIR` | installs all four into `<data>/home/.agents/skills/` plus the `.modelling-skills.json` note; log: `[intcad] skills: installed solidcraft-checking, solidcraft-editing, solidcraft-mechanisms, solidcraft-modeling` |
| `agent-smoke.mjs --only "skill sets"` against the **packaged** folder | `commands: skill/solidcraft-checking, skill/solidcraft-editing, skill/solidcraft-mechanisms, skill/solidcraft-modeling`; all four told to the model, `installedAsPackaged: true`, `read: true`, `permissionsAsked: 0`; `{"summary":"all passed"}` exit 0 |

That is the first evidence in this release that the shipped agent carries the four skills, installs
them, and offers the four `skill/<name>` commands — the part #204's PR could only show from a
package-shaped folder. What it does **not** show is the Assistant drawing those commands in its
slash menu, which needs the app (next rows) and is still open.

**Limits found here, all build-host properties rather than package defects:**

- `package.sh`'s own smoke cannot run on this NixOS host: the bundled Node needs
  `libstdc++.so.6`, which NixOS does not expose to it (`error while loading shared libraries`).
  The same case passes in the Ubuntu VM.
- The app binary built on this host is linked to Nix's loader
  (`[Requesting program interpreter: /nix/store/…-glibc-2.44-25/lib/ld-linux-x86-64.so.2]`), so it
  cannot run on Ubuntu; the package's `bin/intcad` has to come from an Ubuntu build (`podman` with
  the E2E builder image, or CI). The agent folder itself is portable.

### 12.1 The app on the same installed shape

The app binary has to come from an Ubuntu build, so it was built in the E2E builder container
(`podman run … intcad-e2e-builder:24.04 cargo build --release -p solidcraft -p solidcraft-cli`,
10m47s) and placed in the installed layout: `bin/intcad` beside `lib/intcad/mastra-agent/`.

| Step | Result |
|---|---|
| `bin/intcad --version` on Ubuntu 24.04.5 | `intcad 0.1.2` |
| Launch, open the Assistant | window 1440×900, Vulkan/llvmpipe, control server up |
| Assistant entry | moved from the application bar to the right rail (`rail:assistant`) — the app had found the packaged agent, which is what switches that layout on; `v0.1.2` has no rail and only `titlebar:assistant` |
| Click `rail:assistant` | the panel opens and the page renders: "Not connected: sign in here", model sign-ins and API keys |
| After the click | the app is running the packaged agent: `…/mastra-agent/node …/mastra-agent/agent/main.mjs` (pid 3117) |
| Agent's own data folder | `~/pkgrun/data/intcad/mastra/home/.agents/skills/` holds `solidcraft-{checking,editing,mechanisms,modeling}` and `.modelling-skills.json`, 100K/12K/40K/28K |
| Second control port in the app log | `127.0.0.1:37059` — the agent's own channel, opened after the panel did |

The same run repeated from the final tarball rather than a hand-made layout: the 461.8 MB
`intcad-0.1.2-linux-x86_64.tar.gz` (sha256 `c8dcfabfcad3e6f98d9a7cb6bf0aa7fbcaec5da51423418fd809f91bd2a1d519`,
built with the Ubuntu binaries and `SHA256SUMS.txt` written by `packaging/checksums.sh`) extracted in
a fresh VM, `bin/intcad-cli --version` → `intcad-cli 0.1.2`, `bin/intcad --version` → `intcad 0.1.2`,
bundled `node --version` → `v22.23.4`, then launched from `bin/intcad`: the agent started from
inside the extracted tree (`/tmp/final/intcad-0.1.2-linux-x86_64/lib/intcad/mastra-agent/node …/agent/main.mjs`)
and the four skills appeared under the app's data folder. The tarball still says `0.1.2`: the
candidate needs its version bump before any of this is a release.

Reproduce (Linux x86_64, from a worktree containing `develop` + #204 + #214's agent plumbing):

```
node packages/mastra-runtime/scripts/agent-package.mjs build --target x86_64-unknown-linux-gnu \
  --out target/mastra-agent/x86_64-unknown-linux-gnu --node <node 22.23.4>
packaging/linux/package.sh --skip-build --formats tar --with-mastra-agent   # after an Ubuntu build
packaging/checksums.sh dist/
```

Evidence: `~/.local/state/intcad-e2e/runs/intcad-e2e-int1503-rc-rehearsal-3a92c4b6125c-rel012/20261123T075538Z-2614506/screenshots/rel012-pkg-assistant.png`
and `rel012-final-tar.png`.

So the installed shape starts, finds the packaged agent, and that agent installs the four skills in
the app's own data folder — Gate C's install half, end to end, from an installed layout rather than
a test folder.

**Still open for Gate C:** the slash list in the Assistant's composer. The panel here shows the
Connections page because no model is signed in, and the composer is not drawn in that state, so
`skill/<name>` entries cannot be seen without a model credential (an API key or a plan sign-in) or
a stand-in model service. The ACP-level proof above (four `skill/<name>` commands, all read) is the
agent side; the drawing is the only part left, and it needs a credential this role does not hold.
The same run also cannot show a real turn, for the same reason.

### 12.2 Windows: the published MSI, installed and launched (2026-11-23)

Same idea as §11 but on Windows, to prove the Windows acceptance route with a real package instead
of a source build. VM `intcad-win-…-win012` (Windows 11 Enterprise 24H2, WebView2 present).

| Step | Result |
|---|---|
| Transfer the published `intcad-0.1.2-windows-x64.msi` over SSH | 108,363,776 bytes; in-guest SHA-256 `b8d8037be8f36c00d69e8bc9a1ca8cdc05346c1dc0a2badd0d891aeccbc828fd`, the release's own digest |
| `msiexec /i … /qn /norestart` | exit 0 |
| `C:\Program Files\Intcad` | exactly the MSI's files: `intcad.exe` 229,128,704; `intcad-cli.exe` 156,839,424; `ASSISTANT-THIRD-PARTY-NOTICES.txt`; `LICENSE-APACHE`, `LICENSE-MIT`, `NOTICE`, `THIRD-PARTY-LICENSES.txt`, `WEB-VIEW-PLATFORM-NOTICES.txt` |
| `intcad-cli.exe --version` from the install folder | `intcad-cli 0.1.2`, exit 0 |
| Launch | window 1024×768; log: `Dx12, MSAA 4x`, `control server listening on 127.0.0.1:7878` |
| Assistant | panel opens; the **WebView2 page renders**: "No agent found / Set up an assistant", "Not installed", the composer "Ask the agent to change the design…" |

Evidence: `~/.local/state/intcad-e2e/windows/runs/intcad-win-int1503-rc-rehearsal-3a92c4b6125c-win012/20261123T080432Z-2631759/screenshots/win012-assistant.png`
and `win012-assistant-clean.png`.

**A method finding, not a product defect:** a process started over SSH lands in the SSH session, so
its window never appears on the console desktop; the app looks "not started" while its control
server answers. The harness launches through a scheduled task with `-LogonType Interactive`, and
that is what put the window on screen. The first, SSH-started instance was killed by hand, which
left one "Closed unexpectedly" record in the app's crash folder — the report names the killed
instance's start time, and no new record appeared for the session launched properly.

**Local tooling drift, now repaired:** the Windows guest scripts in `.dev/e2e/guest/windows`
named `C:\Program Files\SolidCraft\solidcraft.exe`, so the harness could not launch an installed
**Intcad** MSI. `run-app.cmd` and `run-app.ps1` now try `C:\Program Files\Intcad\intcad.exe`
first and fall back to the old path. Verified in the guest: after stopping the app, running the
patched `run-app.cmd` produced one `intcad` process and the app's own log lines (`Dx12, MSAA 4x`,
`control server listening on 127.0.0.1:7878`). The `.ps1` uses the same resolution but was not
executed (it blocks until the app exits). `.dev/e2e` is excluded through `.git/info/exclude`, so
this is a local repair, not part of any PR.

**Still open for Windows:** a machine without WebView2 (the app is expected to keep the built-in
conversation there), SmartScreen/unsigned first launch, the upgrade from 0.1.2 to a versioned
candidate, and anything the Assistant shows once a model is signed in.

### 12.3 A versioned candidate: 0.1.3 (Linux, 2026-11-23)

The first artifact in this release that calls itself 0.1.3. Built in the rehearsal worktree only;
nothing was pushed, tagged or published, and the release branch still says 0.1.2.

| Step | Result |
|---|---|
| Version bump | `[workspace.package] version = "0.1.3"`; `cargo update --workspace --offline` changed exactly the 25 workspace crates in `Cargo.lock` and no dependency version (26 insertions, 26 deletions) |
| Build | in the Ubuntu 24.04 E2E builder container, `cargo build --locked --release -p solidcraft -p solidcraft-cli`, 5m25s |
| Package | the real `packaging/linux/package.sh --skip-build --formats tar --with-mastra-agent`, run inside `ubuntu:24.04` with the stock Node 22.23.4, so the script's own agent checks and smoke ran |
| Agent checks | `check` and the secret `scan`: `problemCount: 0`; skills set `sha256:0d7b9a61…` |
| **Agent smoke, all 8 cases** | `{"summary":"all passed","cases":8}` — three turns, a command after a permission request, an MCP tool, the skill sets (four `skill/<name>` commands, every SKILL.md read), a refused command, a refused write, and both cancellation cases |
| Script's own CLI check | `intcad-cli 0.1.3` |
| Artifact | `intcad-0.1.3-linux-x86_64.tar.gz`, 462 MB, sha256 `e30b1cf86fa1c4c0b39bb42ef16333c3d8256cfe01f7c44f61d887ddf1edf134`, 13,597 `mastra-agent` entries, the four `SKILL.md` and `skill-set.json` |
| Checksum list | `packaging/checksums.sh` wrote `SHA256SUMS.txt` and `SHA256SUMS`, and its `sha256sum --check` passed on the artifact |

**Upgrade from the published 0.1.2, in a clean Ubuntu 24.04 VM** (`…rel012`, same user data folder
throughout):

1. Extracted the published `0.1.2` tarball into `~/apps/intcad`, ran it → `intcad 0.1.2`, opened the
   sample plate, saved it through the control channel as `~/upg/from-012.sc` (45,543 bytes), then
   `app.quit` → `quitting: true` and the process exited.
2. Replaced the install tree in place with the 0.1.3 tree → `~/apps/intcad/bin/intcad --version`
   → `intcad 0.1.3`.
3. Launched 0.1.3 on that saved design → `ui.documents` lists `from-012`; the window shows `v0.1.3`
   with the plate and its sketches.
4. Opened the Assistant → the agent starts from the new prefix
   (`~/apps/intcad/lib/intcad/mastra-agent/node …/agent/main.mjs`) and installs the four skills into
   the same `XDG_DATA_HOME/intcad/mastra/home/.agents/skills`.

Evidence: `~/.local/state/intcad-e2e/runs/intcad-e2e-int1503-rc-rehearsal-3a92c4b6125c-rel012/20261123T075538Z-2614506/screenshots/win-upgrade-013.png`
(the filename is a mistake in the run's label; it is the Linux 0.1.3 window).

**What this proves and what it does not**

- Proves: a versioned 0.1.3 package is buildable from this tree with the agent and the four skills,
  it installs, starts, opens a design saved by 0.1.2, and its agent works.
- Does **not** prove: the one-click update path (feed → download → checksum → apply). The feed URL is
  a constant (`crates/ui-egui/src/update.rs` `LATEST_API`) with no override, and publishing a
  prerelease is not authorised, so that path can only be exercised by the existing unit and scenario
  tests plus the E1 name fix. Also untested here: preferences continuity (the 0.1.2 run created no
  preferences file, so there was nothing to carry), and Windows/macOS 0.1.3.

### 12.4 Windows 0.1.3: built, upgraded from the published 0.1.2 (2026-11-23)

The Windows build that failed in the previous turn now works, without touching any other branch:

| Step | Result |
|---|---|
| Toolchain | rustup 1.99.1 + `x86_64-pc-windows-msvc` std; nix's `clang` has no `clang-cl`, so a two-line shim runs it as `--driver-mode=cl` and adds LLVM's split C headers (`/imsvc …/clang/19/include`); `llvm-lib` and `lld-link` from LLVM 19/21; `cargo-xwin` |
| Build | `cargo-xwin xwin build --release --target x86_64-pc-windows-msvc`: `intcad.exe` 222.1 MB (3m40s), `intcad-cli.exe` 139.1 MB (3m36s); only `LNK4099` warnings about xwin's CRT PDBs |
| MSI | built **in the VM** with WiX 5.0.2 (installed there), PDB-free `wix build` of `assets/platforms/windows/solidcraft.wxs`, `-d Version=0.1.3 -d DisplayVersion=0.1.3 -d HasMastraAgent=0` → `Intcad-0.1.3-windows-x64.msi`, 100,716,544 bytes, sha256 `894e04fb735e71c7a2a95a882effeecccc8bd6bee2bde853ce884a79e7c8feb2`; `packaging/checksums.sh` wrote `SHA256SUMS.txt` |
| Upgrade | `msiexec /i … /qn` over the installed published 0.1.2 → exit 0; `C:\Program Files\Intcad` holds the new `intcad.exe` 232,881,664 and `intcad-cli.exe` 145,908,736; `intcad-cli 0.1.3`; **one** uninstall entry (`Intcad 0.1.3`), so the old install was replaced, not duplicated; `%APPDATA%\Intcad` still present |
| Launch | window `v0.1.3` with the sample plate and the new right rail; the Assistant panel opens and its WebView2 page renders (Connections) |

Evidence: `~/.local/state/intcad-e2e/windows/runs/intcad-win-int1503-rc-rehearsal-3a92c4b6125c-win012/20261123T080432Z-2631759/screenshots/win013-later.png`
and `win013-assistant.png`.

**Not in this package:** the agent folder (B7). `HasMastraAgent=0` because the Windows agent must be
built on Windows: `agent-package.mjs` runs the given `node.exe` to check its version, and this VM
has no Node. On a real via-CI package the agent comes from `agent-test-packages.yml`, and #214's
content gate is what proves it is there.

**Slash list, again:** opening the composer and typing `/` on Windows (no agent) and on Linux (agent
present) did not show a `skill/…` menu; the panel sits on the Connections page until a model is
signed in. The four commands are registered by the agent (ACP evidence above), but drawing them
still needs a model credential.

**Crash dialogs seen in these runs** are from force-stopping the app between installs (the report
names the killed start time); they are not product crashes.

## 13. Decisions from the CTO (2026-11-23)

Recorded as given: **version 0.1.3**; **PP's UI must be in this release**; everything else is
approved except what needs real Mac hardware. In practice that means:

- 0.1.3 is the batch version; the version commit is prepared on `int1503-version-013`
  (`Cargo.toml` + the 25 workspace entries in `Cargo.lock`) and will be carried by the
  `release/v0.1.3` promotion PR once the scope is on `develop`.
- The FreeBSD fix (#219) and the updater name fix (#217) are approved to land.
- Publication is authorised in principle, but only after the gates pass. The Mac-dependent items
  (native Intel hardware, Gatekeeper/quarantine first launch, real-Mac V2/V7 login runs) stay
  NOT TESTED and are not exceptions to the rest.
- **PP's UI still has no PR**, so the release cannot be complete: this is the one scope item that
  cannot be started from here.

## 14. Decisions still open / carried

1. **Version and package plan:** one 0.1.3 with login, or 0.1.3 now without it and 0.1.4 later
   (INT-1503 raises both)? This decides whether #213 blocks the first package.
2. **B2 (FreeBSD):** fix the one-line guard, or drop FreeBSD from `package-builds` for this
   release? Dropping it is a gate change and needs an explicit decision.
3. **B3 (Windows regression in #213):** who owns the Windows hit-test regression, and whether a
   re-run is allowed to rule out flakiness before the owner investigates.
4. **E1:** approve a small change to `crates/ui-egui/src/update/install.rs` to accept `SHA256SUMS`
   as well as `SHA256SUMS.txt`, so an installed 0.1.2 can update; or accept that updates are manual
   for this release.
5. **PP's UI PR:** when it opens, whether it must be in this release.
6. **Publication:** nothing is tagged or published without explicit approval.
