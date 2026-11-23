# Release readiness: 0.1.3 (INT-1503)

Owner: release manager. Written 2026-11-23. This file is a decision record, not release evidence
for a candidate: no candidate exists yet. Every claim below names the command, file or run that
shows it; anything unverified says `UNKNOWN`, `BLOCKED` or `NOT TESTED`.

**Corrections after the GPT-6.2-Sol final review (2026-11-23).** Three things in this document were
wrong and are fixed below: E1 was described backwards (the shipped 0.1.2 client asks for
`SHA256SUMS.txt` in the release it *fetches*, so what matters is the *new* release's naming, not
0.1.2's); the update path is not "a constant with no override" (`SOLIDCRAFT_UPDATE_FEED` exists, but
its asset URLs must be GitHub release URLs); and the first `release.yml` candidate run was cancelled
and produced no artifacts. The "state after the CTO's decisions" paragraphs in §6 and §9 describe
earlier moments in the day and are superseded by §12.4.1 and §13.

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

**State on 2026-11-23, evening, after the merge chain:** #204, #218, #219 and #212 are **merged**;
`origin/develop` is `be9cb2c16` (carries the cookie fix, the FreeBSD fix, INT-1473 and both skill
sets). #214 is the one open release PR: its three self-inflicted CI failures (skill anchors, the
3600-character limit, and the stage-agent test that only built one skill set) are fixed on its
branch. #213 still fails a Windows check; PP's UI has no PR. The 0.1.3 version commit is prepared
on `int1503-version-013`, and the release.yml candidate branch is described in §12.4.1.

| # | Sev | Blocker | Evidence |
|---|---|---|---|
| B1 | P0 | **No candidate artifact.** `release.yml` still builds without the agent (`SOLIDCRAFT_WITH_MASTRA: "0"` sets only the workflow runtime; nothing sets `SOLIDCRAFT_WITH_MASTRA_AGENT`), and #214, which adds it, is an unmerged draft. The rehearsal in §12 shows the local path works, so this is now a merge-and-run question, not an unknown | `release.yml:29`, `packaging/linux/package.sh:26`, PR #214 state, §12 |
| B2 | P0, fix in #219 | **FreeBSD package build fails, and `package-builds` requires it.** `apps/solidcraft/src/main.rs` called `egui_winit::set_emit_empty_paste_events` unconditionally while `apps/solidcraft/Cargo.toml` declares `egui-winit` only for Windows/Linux/macOS. #219 puts the dependency's `cfg` on the call; `release.yml` run 45230019033 on that branch is building FreeBSD as this is written | run 45112837415; #219 |
| B3 | P1 | **Login PR #213 has a failing required check on Windows.** `Windows app startup` → `tools/windows-resize-smoke.ps1`: `WM_NCHITTEST failed (Win32 error 1460) at (20,330) within the 2 s limit` after the window was moved to 20,20–1004,640. The same job passes on `develop` `1c3102f` (run 45087015186) and on `56e805f` (run 45035941855), so it is a regression in #213's range, not a flake observed twice | run 45109730998 job 137479303661 |
| B4 | P1, code merged | **Skills need a package that carries the agent.** #204 merged into `develop` (`41089b60`), so the code that installs the four sets is in; a *release* still needs #214, which puts the agent folder into the packages | #204 merged, §12, §12.1 |
| B5 / B10 | P1 | **The Assistant has no slash menu for `skill/<name>`.** With a working key and a live session, typing `/` or `/skill` draws no command menu, and no code in the app or the Assistant page consumes ACP `available_commands_update`. The agent registers the four commands (ACP evidence) and they work when typed as text, but "displayed as a command" is not implemented — an app-side feature, not packaging | §12.5 |
| B6 | P1 | **The Mastra workflow runtime (`mastra/`) is still not in any package.** Only the agent folder ships. `--with-mastra` stays opt-in and fails closed until the runtime artifact is approved (DQ-7/#15), so "Mastra in the package" is true for the agent and not for the workflow runtime | `packaging/linux/package.sh:101-118`, `docs/mastra-packaging.md` |
| B8 | P0 | **Every package build fails on a dependency advisory.** `Dependency advisories` fails with RUSTSEC-2026-0594: `cookie 0.18.2` via `wry 0.57.2` (private-cookie nonce reuse after `fork()`, which breaks AES-GCM). `release.yml` cannot pass for any branch until it is fixed; #218 moves it to 0.18.3 | run 45149031980 job 137606035774 |
| B9 | P1, PR open | **PP's UI is PR #221** (draft, `BEHIND`, 29 files, +1823/−224: the Assistant moves to the bottom centre, one message box, native clipping in `assistant_clip.rs`). Two red checks when reviewed: a `develop`-wide `lint` breakage inherited from #212 (fixed in #222) and `ui_containers` ("Use ui_kit::Container for outer hosts"), which is PP's code to fix. Integration still needs the visual pass | PR #221; §12.11 |
| E1 | P2, fix landed (not a blocker) | **The 0.1.2 → 0.1.3 upgrade needs `SHA256SUMS.txt` in the 0.1.3 release.** The shipped 0.1.2 client (`65af7346`, `install.rs` `SUMS`) looks for `SHA256SUMS.txt` in the release it fetches, so what matters is the name 0.1.3 publishes — and `packaging/checksums.sh` writes it. My earlier entry had this backwards. The branch change (accept `SHA256SUMS` too) only makes future clients tolerant of a hand-made list; it cannot change an installed binary. Verified against the tag: `git show 65af7346:crates/ui-egui/src/update/install.rs` has `pub const SUMS: &str = "SHA256SUMS.txt"` | `install.rs` `SUMS_LEGACY`; §7 |
| B11 | P1, tooling added | **A release needs unversioned asset aliases or those users cannot update.** `asset_name()` asks for `intcad-windows-x64.msi`, `intcad-macos-universal.dmg`, `intcad-linux-x86_64.{AppImage,deb,rpm,flatpak,snap}` and `intcad-windows-<arch>-portable.zip` without a version; `release.yml` produces only versioned names. Publishing them as-is would leave every MSI/DMG/AppImage/deb/rpm/Flatpak/Snap/portable-zip install unable to find its package. `packaging/aliases.sh` (4 tests, wired into packaging-lint) writes them, and `docs/beta-release.md` runs it before the list | `update.rs` `asset_name()`; §12.4.9 |
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

Verdict: **NO-GO** at the time of writing: no release.yml candidate had been accepted, B9 (PP's UI) is
missing from the scope, and Gate B has not run. E1 is **not** one of the reasons — see the corrected
row above.

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
- Does **not** prove: the one-click update path end to end. There *is* a feed override
  (`SOLIDCRAFT_UPDATE_FEED`, `update.rs:711`), but every asset URL in that feed must start with
  `https://github.com/intgral-ai/intcad/releases/` (`is_release_url`, `update.rs:43`), so a local
  payload needs DNS and TLS interception in the guest, and no release is authorised to point at.
  That path is covered only by the unit and scenario tests. Also untested here: preferences
  continuity (the 0.1.2 run created no preferences file, so there was nothing to carry), and
  Windows/macOS 0.1.3.

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

## 12.4.1 The real release.yml candidate (2026-11-23, run 45241269258)

The first package build in this release that comes from `release.yml` itself rather than a local
script, and the first one whose workflow bundles the agent.

| Item | Value |
|---|---|
| Branch | `int1503-rc-candidate`, a **local candidate** branch. First attempt: `develop` `3751144fc1` + #214 `054d51e4d`, SHA `3ffacdb1`, run 45241269258 — **cancelled** once `develop` moved on and the branch was rebuilt, so it produced no artifacts. |
| Current SHA | `14a6c95ae527b2f584035448513ae98455e2f709` = `develop` `be9cb2c16` (carries #218 cookie, #219 FreeBSD, #212 agentic and the `context-skills` set) + #214 `b71d51641` (packaging, agent, and the stage-agent test fix) + the 0.1.3 version commit |
| How started | `gh workflow run release.yml --ref int1503-rc-candidate` → run **45248432170** |
| Why not `release-gate` | it only starts for a `main`-target PR or a merge group, and this branch is not a promotion |
| Workflow settings inherited from #214 | `SOLIDCRAFT_WITH_MASTRA_AGENT: "1"`, `SOLIDCRAFT_DISTRIBUTION: "mastra-agent"`, per-arch overrides (`x86_64` only), legacy `mastra/` still `"0"` |
| What it produces | the nonpublishing packages for every target, `build.json`/`contents.json` per package, and the content scan that checks the agent is inside |
| Note on the first attempt | that run was cancelled and its artifacts were never uploaded; the watch script's "no artifact matches" lines are that run, not a failure of the current one |

This branch is **not** a release branch and must not be merged: it exists to prove that the merged
tree plus #214 produces installable packages that carry the agent. The real candidate is the
promotion PR `release/v0.1.3` once #212/#214 land through their own reviews.

### 12.4.3 The release.yml candidate, verified on Linux (2026-11-23)

The artifacts of run 45248432170 for SHA `14a6c95ae527b2f584035448513ae98455e2f709` were downloaded
and checked. This is the first *CI-built* package this release has installed and run.

| Item | Value |
|---|---|
| `build.json` `sourceRevision` | `14a6c95ae527b2f584035448513ae98455e2f709` |
| Linux tarball | 222,072,327 B, sha256 `35f114b267946d381b77be7c3ee11e91f426fd8bb19e7aa6a75bb89a381dae7f`; 13,656 entries, **13,605 under `mastra-agent`** |
| Windows MSI | 161,941,799 B, sha256 `f467db171b45b474f230ff3af13c7b05ff84da1c2d3abdb28e6e553a6de36550` |
| Also produced | Linux deb/rpm/AppImage(+zsync, plus the two legacy zsync aliases), Windows portable zip and standalone exe |
| `contents.json`, both platforms | `agent.present: true`, `node: true`, `missing: []`, skills: `modelling-skills/{solidcraft-checking,editing,mechanisms,modeling}` **and** `context-skills/intcad-context` |
| Checksum list, whole candidate | `packaging/checksums.sh` over the flattened set with `--require` per channel asset: 10 files, 1015 B, every hash OK |
| The guard works | with `--require intcad-macos-universal.dmg` and no DMG present it **refused**: "the list would omit these release assets: intcad-macos-universal.dmg" and wrote nothing |

**Installed and run from that tarball on a clean Ubuntu 24.04 VM** (`…rc013`):

```
bin/intcad-cli --version -> intcad-cli 0.1.3
bin/intcad --version     -> intcad 0.1.3
lib/intcad/mastra-agent/node --version -> v22.23.4
launch, open Assistant -> agent runs: /tmp/ci/…/mastra-agent/node …/agent/main.mjs
app data folder         -> intcad-context, solidcraft-checking, solidcraft-editing,
                           solidcraft-mechanisms, solidcraft-modeling   (five sets)
```

Evidence: `~/.local/state/intcad-e2e/runs/intcad-e2e-int1503-rc-rehearsal-3a92c4b6125c-rc013/20261123T205537Z-596622/screenshots/ci013-running.png`.

**Still not in this candidate:** the macOS DMG (the cancelled slice; `gh run rerun --failed` was
queued, see §12.4.2), a real-model turn with this exact build (the key was removed after §12.5), the
slash menu (B10), PP's UI (B9) and login (Gate B).

### 12.4.4 The same candidate's Windows MSI, installed (2026-11-23)

The Windows package of run 45248432170 (`intcad-0.1.3-windows-x64.msi`, 161,941,799 B, sha256
`f467db171b45b474f230ff3af13c7b05ff84da1c2d3abdb28e6e553a6de36550`) installed on a Windows 11 24H2 VM
(`…win013`), in-guest hash checked first.

```
msiexec /i … /qn /norestart -> exit 0
C:\Program Files\Intcad: intcad.exe 233,363,456; intcad-cli.exe 158,796,288; six notice files
agent_dir=True   agent_files=10739   agent_node=True (node.exe)
  skill  context-skills/intcad-context
  skill  modelling-skills/solidcraft-checking
  skill  modelling-skills/solidcraft-editing
  skill  modelling-skills/solidcraft-mechanisms
  skill  modelling-skills/solidcraft-modeling
intcad-cli --version -> intcad-cli 0.1.3 (exit 0)
launch -> window v0.1.3, Dx12, control server on 127.0.0.1:7878
```

So the Windows installer from CI carries the agent folder with its bundled `node.exe` and all five
skill sets in the install directory — the gap B7 recorded for my locally built MSI (`HasMastraAgent=0`)
is closed for the CI package.

Running it closed that gap too: with the Assistant opened (Ctrl+Shift+A), the packaged agent started
and copied its sets into the app's own data folder.

```
node      C:\Program Files\Intcad\mastra-agent\node.exe        (the packaged node, not a system one)
skills    %LOCALAPPDATA%\Intcad\mastra\home\.agents\skills\
          intcad-context, solidcraft-checking, solidcraft-editing,
          solidcraft-mechanisms, solidcraft-modeling
stamp     .modelling-skills.json present
```

So on Windows, from the CI-built MSI: install → start → Assistant → agent runs from the install
directory → five skill sets land in the user's data folder.

Not shown for this Windows build: a real-model turn, the `installed-check.ps1` result CI produces per
architecture, and the same on a machine without WebView2.

Evidence: `~/.local/state/intcad-e2e/windows/runs/intcad-win-int1503-rc-rehearsal-3a92c4b6125c-win013/20261123T210203Z-622930/screenshots/ciwin-agent.png`.

### 12.4.5 Two more #214 checks, fixed (2026-11-23, late)

While the macOS slice was being rebuilt, #214's own CI surfaced two more failures. Both were in the
PR, not in the packages:

| Check | Failure | Fix on the branch |
|---|---|---|
| `Defaults line check` | the body said `Changes a default: yes — … kept behind the packaging switch …`, which the checker's grammar rejects; it wants `yes — <what>, old behaviour kept behind preference <name>` | description now: `Changes a default: yes — the release packages carry and start the built-in Mastra agent, old behaviour kept behind preference ai_features` |
| `Format and repository checks` | `scripts/ci/test_package_contents.py` called `tarfile.open(..., "w:zst")`; Python writes zstd tars only from 3.14 and the runner has 3.12 → `tarfile.CompressionError: unknown compression type 'zst'` before the code under test ran | a helper writes the archive with `tarfile` when it can, falls back to the `zstd` command, and skips with a reason when neither exists; `python3 -B -m unittest scripts/ci/test_package_contents.py` → 19 tests, OK |

Pushed as `f628596aa4`. #214 then showed 5 pass / 2 pending / 1 skipping and no failures, with a
merge watcher waiting for the last two.

### 12.4.6 Windows upgrade with the published 0.1.2, then the CI 0.1.3 (2026-11-23)

Gate E's Windows half, done with real release bytes in both directions:

```
uninstall the 0.1.3 already on the VM        -> exit 0
install published intcad-0.1.2-windows-x64.msi (sha256 b8d8037b…) -> exit 0
    C:\Program Files\Intcad\intcad-cli.exe --version -> intcad-cli 0.1.2
    C:\Program Files\Intcad\mastra-agent exists -> False   (0.1.2 carried no agent, as published)
install the CI intcad-0.1.3-windows-x64.msi (sha256 f467db17…) over it -> exit 0
    intcad-cli 0.1.3
    uninstall entries: exactly one, "Intcad 0.1.3"
    mastra-agent present, 10,739 files, five skill sets
    %LOCALAPPDATA%\Intcad still present
```

So a user on the published 0.1.2 can install this 0.1.3 MSI and ends with one product at 0.1.3 that
carries the agent; the old install is replaced, not duplicated, and the user data folder survives.
That is the MSI half of Gate E. The one-click path is still untestable before publication (§10).

### 12.4.7 The candidate's AppImage, the channel the updater replaces (2026-11-23)

The AppImage is the one Linux channel the in-app updater replaces in place (`Method::AppImage`), so
its startup matters for Gate E even before the update itself can be tested.

```
APPIMAGE_EXTRACT_AND_RUN=1 ~/ai/intcad.AppImage --version -> intcad 0.1.3
launch with APPIMAGE=~/ai/intcad.AppImage -> window, Vulkan, control 127.0.0.1:7879
Assistant -> agent runs from inside the mounted image:
  /tmp/appimage_extracted_…/usr/lib/intcad/mastra-agent/node …/agent/main.mjs
data folder -> intcad-context, solidcraft-checking, solidcraft-editing,
               solidcraft-mechanisms, solidcraft-modeling
```

Evidence: `~/.local/state/intcad-e2e/runs/intcad-e2e-int1503-rc-rehearsal-3a92c4b6125c-rc013/20261123T205537Z-596622/screenshots/ci013-appimage.png`.

With §12.4.3 (tarball), §12.4.4/§12.4.6 (MSI install + upgrade) and this, the candidate has been
installed and started on both target platforms from three different package formats.

### 12.4.8 The candidate is complete: the macOS DMG exists

After `gh run rerun --failed`, run 45248432170 finished **success** with both slices built and the
universal package produced:

```
macOS build x86_64      success
macOS build aarch64     success
macOS universal (ad-hoc only)  success
intcad-0.1.3-macos-universal.dmg      489,745,075 B
intcad-cli-0.1.3-macos-universal.zip  114,899,408 B
contents.json (dmg) -> agent.present true, node true,
  nodeTargets [aarch64-apple-darwin, x86_64-apple-darwin],
  skills: the four modelling sets + context-skills/intcad-context; missing []
```

So the 0.1.3 candidate now has every asset the release promises: Linux tar/deb/rpm/AppImage and the
two legacy zsync aliases, Windows MSI/portable zip/standalone exe, the universal DMG, and the macOS
CLI zip. `packaging/checksums.sh` over that flattened set writes a 14-file list under both names and
verifies every hash.

**Checked again from the DMG itself, not from CI's report** (`7z l` on the 489 MB image):

```
21501 files, 5743 folders
Intcad.app/Contents/MacOS/intcad  and  …/MacOS/Intcad
mastra-agent/aarch64-apple-darwin/node      mastra-agent/x86_64-apple-darwin/node
agent/main.mjs  ×2  (one per slice)
both slices carry context-skills/intcad-context and the four modelling skills' SKILL.md
```

So the universal image really holds a per-architecture agent with its own Node and all five sets —
the same thing `contents.json` claimed, confirmed on the bytes.

What the DMG does **not** prove: first launch through Gatekeeper (ad-hoc signature only), a real
Intel Mac, or the universal app being started by a person — CI mounts it, copies the app out and
opens the Assistant in a sandbox with the network taken away. Those stay NOT TESTED.

### 12.4.9 The complete asset set, with the aliases the updater needs (2026-11-23)

The macOS DMG closed the last missing build (§12.4.8). Assembling the whole set then exposed a
release-process gap: the updater resolves several assets **by an unversioned name**.

| Written by `release.yml` | Asked for by `asset_name()` |
|---|---|
| `intcad-0.1.3-linux-x86_64.AppImage` | `intcad-linux-x86_64.AppImage` |
| `intcad-0.1.3-linux-x86_64.deb` / `.rpm` | `intcad-linux-x86_64.deb` / `.rpm` |
| `intcad-0.1.3-windows-x64.msi` | `intcad-windows-x64.msi` |
| `intcad-0.1.3-windows-x64-portable.zip` | `intcad-windows-x64-portable.zip` |
| `intcad-0.1.3-macos-universal.dmg` | `intcad-macos-universal.dmg` |
| `intcad-0.1.3-linux-x86_64.tar.gz` | `intcad-0.1.3-linux-x86_64.tar.gz` (version kept) |
| `Intcad-0.1.3-windows-x64.exe` | `Intcad-0.1.3-windows-x64.exe` (version kept) |

`packaging/aliases.sh dist 0.1.3` wrote the six aliases for this candidate (AppImage, deb, rpm, DMG,
MSI, portable zip — the Flatpak/Snap/Arch derived formats are built by their own jobs), and the list
over the whole set then passed:

```
aliased intcad-linux-x86_64.AppImage, intcad-linux-x86_64.deb, intcad-linux-x86_64.rpm,
        intcad-macos-universal.dmg, intcad-windows-x64.msi, intcad-windows-x64-portable.zip
checksums.sh --require <tar> --require <msi> --require <dmg> -> 18 files, every hash OK
```

`checksums.sh` also stopped listing `build.json`/`contents.json`: they are per-package evidence, not
downloads. Tests: `aliases_test.py` 4, `checksums_test.py` 11, both run by `packaging-lint.yml`.

### 12.4.10 Secret scan of the candidate itself

The release must not carry credentials. Checked on the real Linux tarball (the MSI install shows the
same agent folder, and CI's `agent-package.mjs scan` runs at staging):

```
grep -rIl -E 'sk-[A-Za-z0-9]{20,}|ghp_…|AKIA…|BEGIN … PRIVATE KEY' over the agent's own files
  -> no match (only library directories whose names contain auth/token/credential, e.g.
     google-auth-library, @aws-sdk/credential-provider-*, which are package folders, not data)
grep -rIl -E '/home/hyjing|/Users/hyjing|C:\\Users\\hyjing' over agent/ -> no match
agent/secrets.mjs is the module that deletes provider keys from the environment before Mastra
  starts (docs: "the agent's entry drops every provider key from its environment")
agent.json holds target, node version, sdk versions and the enterprise removal note only
```

Not a substitute for rights clearance or for #7/#14's audits; it is the "no accidental credential in
the artifact" check this role can run.

### 12.11 PP's UI PR arrived (2026-11-23, late)

PR #221 "Assistant moves to the bottom centre of the window: one message box, the conversation opens
above it (INT-1500)", 29 files, +1823/−224: `assistant_clip.rs` (native clipping), `assistant_web.rs`,
`agent_chat/{mod,panel,web,connections,settings}.rs`, the checked-in Assistant page
(`packages/assistant-web/dist/*`, `inventory.json`), `packages/mastra-runtime/agent/agent.mjs`, and
`tools/windows-assistant-smoke.ps1`. Draft and `BEHIND` against `develop`.

Two red checks, reviewed and reported on the PR:

| Check | Cause | Owner |
|---|---|---|
| `lint` | **not PP's**: `develop` has been red since #212 added `context-skills` while `packaging/tests/mastra_agent_test.py` still built only `modelling-skills`; every PR inherits it | fixed in #222 (test asks `SKILL_SETS`; test-only `--skills-root`), verified 8/8 where `develop` fails 2 |
| `Format and repository checks` | `cargo xtask ui-containers`: "Use ui_kit::Container for outer hosts" — the new outer host draws its own frame | PP |

Release-side consequence: the scope item is no longer missing, but it cannot join the candidate until
that check is fixed and the branch is ready. When it is, the visual pass (Linux and Windows at 100 %
and 150 %: themes, fonts, DPI, panel resize, clipping, basic CAD) is the acceptance this role owes.

## 12.5 A real model credential: slash list, real turn, skill used (Linux, 2026-11-23)

The CTO supplied a DeepSeek API key for acceptance. It was used only in the throwaway VM, kept in
a 0600 file, never echoed into logs and never committed. Cost: three short requests on a 3 USD
credit.

| Step | Result |
|---|---|
| Key reachable from the guest | `curl https://api.deepseek.com/models` → HTTP 200 |
| Saved through the app's own **Add key → DeepSeek → Save** | `Key saved`, `In use`, with Update/Remove: **the system keychain path works** |
| Why the first Save did nothing | the guest's `gnome-keyring` ran without an unwritten login collection; `dbus` showed only `…/collection/session`, and the app is `keyring`-backed. After unlocking a login keyring (`gnome-keyring-daemon --unlock`, collections then show `…/collection/login`) the same Save worked. That is the environment, not the app: a desktop session normally has an unlocked login keyring |
| Connection state | `Connection DeepSeek` / `Ready`, model `DeepSeek Flash` at the composer |
| **Real turn** | "Reply with the single word: ready" → the answer "ready" rendered in the Assistant |
| **Skill used by the real model** | "Read the solidcraft-modeling skill with your skill tool…" → the turn shows a `Skill  solidcraft-modeling` card and an accurate summary of what that skill is for. So the four installed skills are not only discoverable but usable from the packaged agent on a real provider |
| **Slash list** | typing `/` or `/skill` shows the composer with the text and **no command menu**; Tab opened the attach menu ("Choose a file…", "Attach view", "Attach selection", "Pick context"). `rg` over the app and the Assistant page finds no consumer of ACP `available_commands_update`, so the app never receives or draws a skill menu: the four `skill/<name>` commands exist at agent level only. Typing `/skill…` as text still worked, but that is the model reading the word, not a picked command |

Evidence: `key-12-saved.png` (key saved), `key-15-turn.png` (real answer), `key-18-skillturn.png`
(skill card and answer), `key-16-skillmenu.png` (`/skill` typed, no menu) in
`~/.local/state/intcad-e2e/runs/intcad-e2e-int1503-rc-rehearsal-3a92c4b6125c-rel012/20261123T075538Z-2614506/screenshots/`.

**Consequence for the release:** Gate C's "slash command correctly displayed" **fails as a product
gap** — the commands are registered but the Assistant has no slash menu for them. This is B10, and
it is an app-side feature (a command menu fed by `available_commands_update`), not a packaging
defect. Gate C's other two halves (installed, discoverable, callable) now have real-model evidence.

## 12.4.2 A cancelled macOS slice removed the DMG from the candidate

The run 45248432170 candidate has **no macOS package**. `macOS build x86_64` was cancelled by the
runner before its steps ran (`steps: []`, 19:25 → 19:40), and `macOS universal` then reported
`skipped` because it needs both slices. The aarch64 slice succeeded and Linux, FreeBSD and Arch
succeeded; Windows x64/x86/arm64, Linux x86_64 and Web were still running when this was written.

Consequence: this candidate cannot be a release, even if every remaining job passes, because the DMG
is a required asset. Recovery: when the run finishes, `gh run rerun <id> --failed` retries the
cancelled slice (and then the skipped universal job). That is a runner-capacity failure, not a code
failure, and it is why the macOS row stays NOT TESTED rather than PASS.

The gate behaved correctly here: the cancelled slice did **not** silently produce a package. The
first pass ended `failure`, and the only red jobs were `macOS build x86_64` (cancelled) and
`All nonpublishing package builds` (failure) — the aggregate in `release.yml` fails any job whose
result is not `success`, so a cancelled slice cannot pass as a package. `gh run rerun --failed` then
retried the cancelled slice and was queued.

**Second finding, in CI's evidence rather than in a package:** none of the installed checks look at
the agent. `rg 'mastra|agent' packaging/linux/installed-check.sh packaging/windows/installed-check.ps1
packaging/macos/installed-check.sh` returns nothing, so the installed checks prove the app and the
Assistant page, not the agent folder. What covers the agent in CI is the staging smoke inside
`package.sh` and the package content scan; the installed behaviour of the agent (starts, installs the
four skills, answers) is only proven by my VM runs in §12.1/§12.5. That is why those runs matter and
why a packaged candidate still needs them.

## 12.6 What the final review changed (GPT-6.2-Sol, high)

Recorded so the next reader knows which parts of this document were corrected and why.

| Finding | Was | Now |
|---|---|---|
| Checksum list could omit a channel's asset | one run per platform, no completeness check | `packaging/checksums.sh --require <name>` per channel asset, whole-name match, list verified before it replaces anything and both published names after; 10 tests |
| Top-level symlinks were silently left out | no check | refused with a message |
| A failing `find` could shorten the list silently | read from a process substitution | names read into a temporary file first |
| E1 described backwards | "0.1.2 cannot update because 0.1.2 published `SHA256SUMS`" | the shipped 0.1.2 asks for `SHA256SUMS.txt` in the release it **fetches**, so 0.1.3's naming is what matters; the branch fallback is tolerance for future clients only |
| Feed override denied | "a constant with no override" | `SOLIDCRAFT_UPDATE_FEED` exists; asset URLs must be GitHub release URLs, which is what blocks a local payload |
| A cancelled run read as evidence | run 45241269258 implied as the candidate | it was cancelled with no artifacts; §12.4.1 names the current SHA |

Still unchanged and still open: PP's UI has no PR (B9), login Gate B unrun (#213), slash menu missing
(B10), macOS untested, one-click update only testable after a stable release exists.

## 12.7 What CI itself verified for the candidate (run 45248432170, SHA `14a6c95ae`)

Beyond what I installed by hand, the workflow's own steps passed for this candidate:

| Job | Steps that passed |
|---|---|
| `Linux x86_64` | set up the pinned Node; build the agent folder and run it against stand-ins; build AppImage/deb/rpm/tar; check the packages; **install the packages on bare systems and open the Assistant** (ubuntu 22.04, 24.04 x11/wayland, debian 13, fedora 44, plus the AppImage); keep the logs and pictures; bind package bytes to the checkout |
| `Windows x64 (unsigned)` | build the agent folder and run it against stand-ins; build and check the MSI and portable zip; **install the MSI and open the Assistant in the installed app**; keep the logs and pictures; bind package bytes |
| `FreeBSD x86_64`, `Linux aarch64`, `macOS build aarch64`, `Windows x86/arm64`, `Flatpak`, `Arch`, `Snap`, `Web` | build/check steps succeeded |
| `macOS build x86_64` | cancelled on the first pass; rebuilt on rerun (still running when this was written) |
| `All nonpublishing package builds` | failed on the first pass (because the cancelled slice) — the expected, honest result |
| `macOS universal` | skipped until its two slices exist |

So the CI installed checks (which do **not** look at the agent, §12.4.2) prove the app and the
Assistant page on six Linux package/desktop combinations and on the Windows MSI; the agent's
installed behaviour is what my VM runs in §12.4.3/§12.4.4 add.

## 12.8 The reproducible candidate run, as a script

The candidate is no longer assembled by hand. `~/.cache/int1503/rc-final.sh` does the whole run and
logs to `~/.cache/int1503/rc-final.log`:

1. wait until #214 is `MERGED`;
2. reset the candidate worktree to `origin/develop` (so the agent packaging is the merged code, not a
   local merge), set `[workspace.package] version = 0.1.3` and `cargo update --workspace --offline`
   when it is not already there, commit, force-push with lease to `int1503-rc-candidate`;
3. print the two agent settings the branch's `release.yml` carries, dispatch `release.yml` on that
   SHA, watch it, and if the pass is not `success` list the red jobs, `gh run rerun --failed` once and
   watch again;
4. download `release-linux-x86_64`, `release-windows-x64`, `release-macos` and the three
   `package-evidence-*` artifacts;
5. flatten them and run `packaging/checksums.sh --require` for the tarball, the MSI **and** the
   universal DMG, so an incomplete set is refused rather than published.

That is the reproducible release flow this document owes the next person: the same five steps a
release manager runs by hand, in one command, with the refusal behaviour tested in
`packaging/tests/checksums_test.py`.

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

## 14. Decisions resolved, and what is still open

Resolved by the CTO on 2026-11-23 (see §13): version 0.1.3; PP's UI in scope; FreeBSD fixed rather
than dropped (#219 merged); the updater name change approved (#217); publication authorised in
principle once the gates pass, with Mac-hardware items explicitly excluded.

Still open, and not decidable from here:

1. **PP's UI (B9).** No PR exists, so the scope item the CTO requires cannot be reviewed or shipped.
2. **Login Gate B.** #213 is still a draft and its `Windows app startup` check fails; the whole of
   Gate B is unrun. Login development is not this role's work.
3. **B10, the slash menu.** The Assistant has no consumer of `available_commands_update`; the four
   `skill/<name>` commands work as typed text only. This needs an app-side feature and a decision on
   whether 0.1.3 ships without it.
4. **macOS.** Requires CI's DMG build to finish (§12.4.2) and, for Gatekeeper, notarization and
   native Intel, a real Mac.
5. **One-click update.** Only testable against a published stable release (§10, Gate E).

## 15. Go/No-Go, 2026-11-23 (evidence to that moment)

Verdict: **NO-GO.** Not because the packages are broken — the 0.1.3 candidate builds, installs,
starts, carries the agent and installs five skill sets on both Linux and Windows — but because three
required things do not exist yet and one is unverified.

| Gate | Status | Evidence / what is missing |
|---|---|---|
| A. Build and package | **PARTIAL** | run 45248432170: Linux x86_64/aarch64, Windows x64/x86/arm64, FreeBSD, Arch, Flatpak, Snap, Web and macOS aarch64 all succeeded; **macOS x86_64 was cancelled and its universal DMG skipped**, so the asset set is incomplete. `build.json` binds every package to `14a6c95ae…`. |
| A. Content | **PASS for Linux + Windows** | `contents.json` on both: `agent.present`, `node`, five skill sets, `missing: []`; the Linux tarball has 13,605 agent entries and the Windows MSI 10,739 agent files including `node.exe` (§12.4.3, §12.4.4) |
| A. Checksums | **PASS** | `checksums.sh --require` produced a 10-file, both-names list with every hash OK, and refused the set that lacks the DMG (§12.4.3) |
| B. Login | **FAIL / NOT RUN** | #213 draft, `Windows app startup` red; no first-login, session, expiry, logout or network-failure run exists |
| C. Skills | **PARTIAL PASS** | installed and used on Linux with a real model (§12.5); installed and started on Windows (§12.4.4); agent smoke 8/8 with the four `skill/<name>` commands; **slash menu missing (B10)** |
| D. UI | **PARTIAL** | Assistant page renders on Linux and Windows, themes and DPI not exercised, PP's UI absent (B9) |
| E. Upgrade | **PARTIAL** | 0.1.2 → 0.1.3 in place passes on Linux tar and Windows MSI (one ARP entry, `intcad-cli 0.1.3`); the one-click path is untestable before publication (§10) |
| F. Evidence | **PASS for what is claimed** | every row above names a run, file or screenshot; nothing is marked PASS without one |

Since this verdict, the macOS DMG exists and every promised asset is present (§12.4.8, §12.4.9), the
alias gap that would have blocked MSI/DMG/AppImage updates is closed (B11), and the CI Windows MSI was
installed, upgraded from the published 0.1.2 and run with its agent (§12.4.6). What has **not**
changed: #214 is still unmerged, so no accepted candidate comes from a merged `develop` SHA; PP's UI
has no PR; Gate B has not run; B10 is open.

To turn this into GO: land #214, rebuild from the merged `develop` SHA and repeat the Linux/Windows
installs on that exact SHA (`~/.cache/int1503/rc-final.sh` does the rebuild, the package run, the
download and the `--require` checksum list), then add PP's UI and the login run — or record them as
explicitly approved exceptions — and publish only after that.
