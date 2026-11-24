# Release readiness: 0.1.3 (INT-1503)

Owner: release manager. Written 2026-11-23. This file is a decision record, not release evidence
for a candidate: no candidate exists yet. Every claim below names the command, file or run that
shows it; anything unverified says `UNKNOWN`, `BLOCKED` or `NOT TESTED`.

**Current verdict: NO-GO.** [§15.3](#153-current-release-readiness-closed-source-cutover) records
the current gate status. [§8.1](#81-closed-source-release-addendum) governs integration and
publication. Earlier results in this report are historical, not final-candidate acceptance.

**Corrections after the GPT-6.2-Sol final review (2026-11-23).** Three things in this document were
wrong and are fixed below: E1 was described backwards (the shipped 0.1.2 client asks for
`SHA256SUMS.txt` in the release it *fetches*, so what matters is the *new* release's naming, not
0.1.2's); the update path is not "a constant with no override" (`SOLIDCRAFT_UPDATE_FEED` exists, but
its asset URLs must be GitHub release URLs); and the first `release.yml` candidate run (45241269258)
was cancelled **after** uploading seven artifacts from the slices that had finished (both Linux
architectures, Web and their evidence), so "no artifacts" was wrong — what it lacked was the three
platforms the candidate needs. The "state after the CTO's decisions" paragraphs in §6 and §9 describe
earlier moments in the day and are superseded by §12.4.1 and §13.

## 0. Evidence index

Every claim in this file points at one of these. Nothing below is a plan; each row is a run, a file
or a measurement that exists on the machine or on GitHub.

| Evidence | What it proves |
|---|---|
| `gh api repos/intgral-ai/intcad/releases/tags/v0.1.2` | the last published release and its six assets, including `SHA256SUMS` (no `.txt`) |
| v0.1.2 tarball + MSI, hashes from `sha256sum --check SHA256SUMS` | the baseline package contents: 49 entries / 8 MSI files, **no** agent or Mastra runtime |
| run 45230019033 (FreeBSD on #219) | the FreeBSD compile fix works; the job is `success` |
| run 45248432170 (candidate, SHA `14a6c95ae`) | the first `release.yml` build carrying the agent: every platform `success` after the macOS rerun, DMG included |
| `build.json` / `contents.json` of that run | `sourceRevision` = the candidate SHA; `agent.present`, `node`, five skill folders in two sets, `missing: []` on Linux, Windows and macOS |
| `7z l` of the DMG | both Darwin slices carry `node`, `agent/main.mjs` and all five `SKILL.md` |
| `checksums.sh --require` on the flattened set | 18-line list (16 files plus the two legacy `.zsync` names) under both names, every hash OK; refuses a set that omits the DMG. The set is the three downloaded channels, not every artifact `release.yml` builds |
| Ubuntu VM `…rc013` | CI tarball and AppImage installed and run; agent starts; five sets installed; 0.1.2 → CI 0.1.3 upgrade opens the design 0.1.2 saved |
| Windows VM `…win013` | CI MSI installed; agent runs from `Program Files`; five sets in `%LOCALAPPDATA%`; published 0.1.2 → CI 0.1.3 upgrade leaves one ARP entry |
| real DeepSeek turn (screenshots `key-*`) | a real model answers, and reads `solidcraft-modeling`; typing `/` or `/skill` shows no menu (B10) |
| `cargo test -p xtask`, `cargo check --target wasm32`, `python3 -m unittest packaging/tests/*` | the branch's own checks: skill anchors, 3600-char limit, wasm cfg, 15 checksum tests, 8 alias tests, stage-agent 8/8 |
| PR #217, #218, #219, #222 | the release-engineering changes and their CI |

Not evidence, and marked so wherever it appears: a cancelled run, a skipped dependent job, a document
that describes what should happen, and any result from a commit other than the one named.

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
| Login, accounts, capability authorization (INT-1497) | Huaiyuan | PR #224, open, head `0519d3aba6e047aa2fd6c530f556b5d3ee111240`; #213 closed at `22f427c8` | CI green on the promotion SHA, then the Gate B checks |
| Modelling skills in the built-in agent (#198, INT-1473 prerequisite) | Tianwei | PR #204, open, head `b2cb0ef0e0317c9273f771ade89e1c8892331778` | Review comments closed, Text-to-CAD as a second set, installed-app evidence |
| Agentic behaviour over the right rail (INT-1473) | Tianwei | PR #212, open, head `31eae3957c35f78381ef9b1a4cab70b09b6465f9`, based on #204's branch | #204 merged first, then rebase onto `develop` |
| UI changes | PP | **PR #221**, draft and `BEHIND` (head `54424aa3`): the Assistant moves to the bottom centre, one message box, native clipping; 29 files, +1823/−224. `ui_containers` red (its own), `lint` red (the `develop`-wide #212 breakage, fixed in #222) | `ui_containers` green, marked ready, visual regression run |
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
branch. #213 still fails a Windows check; PP's UI is #221, draft and `BEHIND` with its own
`ui_containers` failure. The 0.1.3 version commit is prepared
on `int1503-version-013`, and the release.yml candidate branch is described in §12.4.1.

| # | Sev | Blocker | Evidence |
|---|---|---|---|
| B1 | P0 | **No candidate artifact.** `release.yml` still builds without the agent (`SOLIDCRAFT_WITH_MASTRA: "0"` sets only the workflow runtime; nothing sets `SOLIDCRAFT_WITH_MASTRA_AGENT`), and #214, which adds it, is an unmerged draft. The rehearsal in §12 shows the local path works, so this is now a merge-and-run question, not an unknown | `release.yml:29`, `packaging/linux/package.sh:26`, PR #214 state, §12 |
| B2 | P0, fix in #219 | **FreeBSD package build fails, and `package-builds` requires it.** `apps/solidcraft/src/main.rs` called `egui_winit::set_emit_empty_paste_events` unconditionally while `apps/solidcraft/Cargo.toml` declares `egui-winit` only for Windows/Linux/macOS. #219 puts the dependency's `cfg` on the call; `release.yml` run 45230019033 on that branch is building FreeBSD as this is written | run 45112837415; #219 |
| B3 | P1 | **Login PR #213 has a failing required check on Windows.** `Windows app startup` → `tools/windows-resize-smoke.ps1`: `WM_NCHITTEST failed (Win32 error 1460) at (20,330) within the 2 s limit` after the window was moved to 20,20–1004,640. The same job passes on `develop` `1c3102f` (run 45087015186) and on `56e805f` (run 45035941855), so it is a regression in #213's range, not a flake observed twice | run 45109730998 job 137479303661 |
| B4 | P1, code merged | **Skills need a package that carries the agent.** #204 merged into `develop` (`41089b60`), so the code that installs the four sets is in; a *release* still needs #214, which puts the agent folder into the packages | #204 merged, §12, §12.1 |
| B5 / B10 | P1, reported as #223 | **The Assistant has no slash menu for `skill/<name>`.** With a working key and a live session, typing `/` or `/skill` draws no command menu, and no code in the app or the Assistant page consumes ACP `available_commands_update`. The agent registers the four commands (ACP evidence) and they work when typed as text, but "displayed as a command" is not implemented — an app-side feature, not packaging. Filed with the reproduction and the exact missing arm (`acp.rs:784` has no `available_commands_update` case) as **#223** | §12.5, #223 |
| B6 | P1 | **The Mastra workflow runtime (`mastra/`) is still not in any package.** Only the agent folder ships. `--with-mastra` stays opt-in and fails closed until the runtime artifact is approved (DQ-7/#15), so "Mastra in the package" is true for the agent and not for the workflow runtime | `packaging/linux/package.sh:101-118`, `docs/mastra-packaging.md` |
| B8 | P0 | **Every package build fails on a dependency advisory.** `Dependency advisories` fails with RUSTSEC-2026-0594: `cookie 0.18.2` via `wry 0.57.2` (private-cookie nonce reuse after `fork()`, which breaks AES-GCM). `release.yml` cannot pass for any branch until it is fixed; #218 moves it to 0.18.3 | run 45149031980 job 137606035774 |
| B9 | P1, PR open | **PP's UI is PR #221** (draft, `BEHIND`, 29 files, +1823/−224: the Assistant moves to the bottom centre, one message box, native clipping in `assistant_clip.rs`). Two red checks when reviewed: a `develop`-wide `lint` breakage inherited from #212 (fixed in #222) and `ui_containers` ("Use ui_kit::Container for outer hosts"), which is PP's code to fix. Integration still needs the visual pass | PR #221; §12.11 |
| E1 | P2, fix landed (not a blocker) | **The 0.1.2 → 0.1.3 upgrade needs `SHA256SUMS.txt` in the 0.1.3 release.** The shipped 0.1.2 client (`65af7346`, `install.rs` `SUMS`) looks for `SHA256SUMS.txt` in the release it fetches, so what matters is the name 0.1.3 publishes — and `packaging/checksums.sh` writes it. My earlier entry had this backwards. The branch change (accept `SHA256SUMS` too) only makes future clients tolerant of a hand-made list; it cannot change an installed binary. Verified against the tag: `git show 65af7346:crates/ui-egui/src/update/install.rs` has `pub const SUMS: &str = "SHA256SUMS.txt"` | `install.rs` `SUMS_LEGACY`; §7 |
| B11 | P1, tooling added | **A release needs unversioned asset aliases or those users cannot update.** `asset_name()` asks for `intcad-windows-x64.msi`, `intcad-macos-universal.dmg`, `intcad-linux-x86_64.{AppImage,deb,rpm,flatpak,snap}` and `intcad-windows-<arch>-portable.zip` without a version; `release.yml` produces only versioned names. Publishing them as-is leaves MSI/DMG/AppImage/deb/rpm/portable-zip installs unable to find their package; Flatpak and a store Snap are updated by their package managers rather than by reading these aliases, and the aliases are still published for them. `packaging/aliases.sh` (8 tests, wired into packaging-lint) writes them as copies, and `docs/beta-release.md` runs it before the list | `update.rs` `asset_name()`; §12.4.9 |
| B7 | P1 | **The Windows candidate has no agent.** A Windows 0.1.3 exe and MSI now build here (§12.4), but the agent folder for Windows must be built on Windows (its `node.exe` has to run for the version check), and this VM has no Node; `HasMastraAgent=0` was used. The Windows agent is what `agent-test-packages.yml` builds on Windows runners and what #214's gate checks | §12.4, `agent-package.mjs` line 473 |
| E2 | P1 | **No installed-artifact acceptance path for a candidate.** The VM harness builds from source; `accept` cannot take a package. Linux/Windows installed checks exist in CI and in `packaging/*/installed-check.*`, but they have not been run for a 0.1.3 candidate | §4; `intcad-e2e --help` |
| E3 | P2 | Portable-ZIP updates replace only `intcad.exe` and `intcad-cli.exe`, so a `mastra-agent/` folder from #214 would be left stale or absent | `install.rs:302` `apply()`, `Method::PortableZip` |
| E4 | P2 | `.github/workflows/verify-beta-installers.yml` hard-codes `C:\Program Files\SolidCraft` and `SolidCraft.app/Contents/MacOS/SolidCraft`, so it cannot verify an Intcad release | lines 40 and 73 of that workflow |
| E5 | P2 | `packaging/windows/package.ps1:186` tests `$CanRun` and `$HostArch`, which the script never sets, so the `intcad-cli --version` smoke there never runs (it is silently skipped) | the two lines; no assignment anywhere in the file |
| E6 | P1, PR open | **PP's UI is PR #221** (§12.11): draft, `BEHIND`, `ui_containers` red. Integration risk is now reviewable, and the visual pass is the acceptance this role owes once it is ready | PR #221 |

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
`cmp SHA256SUMS SHA256SUMS.txt`: equal). Re-run with the hardened script (2026-11-23, after the
review fixes): `intcad-0.1.2-linux-x86_64.tar.gz` → `d6de276e94fed11e…`, the published hash, and
`intcad-0.1.2-windows-x64.msi` → `b8d8037be8f36c00…`, also the published one; `sha256sum --check`
on both names: `OK`.

Verification on this machine: `python3 -B -m unittest packaging/tests/checksums_test.py` → 13
tests, OK (it was 6 when §7 was first written; the failure-path cases were added after review) (`nightly_test.py` fails only without `jq` on `PATH`, and passes with it; the other
suites below are unchanged). Re-ran, unchanged and passing: `nightly_test.py` (12), `apprun_test.py`,
`flatpak_test.py`, `web_channels_test.py`, `mastra_runtime_test.py`, `mastra_agent_test.py`,
`scripts/ci/test_gate.py`. `windows_icon_test.py` and the workflow's PowerShell parse need
PowerShell, which this machine does not have: they run in `packaging-lint.yml` and are NOT TESTED
here. `actionlint .github/workflows/packaging-lint.yml` → exit 0.

## 8. Integration order once the workstreams land

Historical proposal. Use §8.1 for current prerequisites, gate order and approvals.

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

### 8.1 Closed-source release addendum

**Final CTO decision: ship and verify v0.1.3 from the existing public development repo;
move development only after release acceptance.** This replaces the pre-release freeze,
PR migration and private-repo build plan. The scope in §13.1 and all product gates still
apply. The historical Gate E text does not defer upgrade testing until publication.

| Repository | Role |
|---|---|
| `intgral-ai/intcad` | Current public development and build repo: continue `main`, `develop`, active PRs and the v0.1.3 promotion branch. Do not publish the product Release here. Preserve source and historical releases. |
| `intgral-ai/intcad-client` | Future independent private development repo. Initialization, remaining active PR migration and development cutover occur after release acceptance, not during 0.1.3. |
| `intgral-ai/intcad-releases` | Public product distribution and update origin, without confidential source or evidence. |
| `intcad-server` | Unchanged; retain the approved login and authorization service. |

Keep the current GitHub-hosted runner environment unchanged. Do not introduce AWS runners
or private CI costs as a release dependency. The Release Manager keeps a separate local-only
migration ledger as the post-release handoff template; it is not a live freeze and must not
be added to this public repository.

#### Owner handoffs

| Owner | Handoff required before freeze |
|---|---|
| Login Agent (INT-1497) | Name the approved integration PR and exact head, the approved PRD and staging test setup, and provide green Windows startup/resize evidence plus the authorization scenarios and expected results for Gate B. Own the client and service behavior, account setup, version admission, and revocation. |
| Closed-source Migration Agent | Deliver the public-safe PRs in `intgral-ai/intcad`: URL inventory, updater/package mapping, signing contract and approved key identity, tested v0.1.2 route, package/rights audit, and cross-repo publishing from the accepted public build. Keep credentials and internal audit files out of the public repo. Coordinate private-repo initialization and CI readiness after release. Own the implementation; the Release Manager does not build parallel versions. |
| Feature owners and Release Manager | PP owns Assistant visual behavior, Tianwei owns Skills/agent changes, and the Release Manager tracks #223 to a reviewed owner and fix. Continue UI, packaging and isolated tests now. The Release Manager owns the integrated gate, artifact identity, package acceptance and the decision record. |

Before each overlapping merge, compare reviewed heads against `develop` and each other, record
the intended owner and resolution, and have affected owners review the resolution. Start with
§15.2's Assistant host/bridge and Windows/Linux smoke overlaps. Also agree on ownership for
update client/config, packaging workflows, website/download links and notice generation before
accepting migration PRs. The login version-admission policy is separate from updater version
selection; neither can substitute for the other. No conflict resolution may weaken authorization,
signature enforcement, or packaging checks.

#### Landed foundations

GitHub's merged PR records and `git merge-base --is-ancestor` identify the following
commits in `develop` snapshot `6032271178140b356e4a7ed2e9bb0f1352d81048`. This proves
the integrations, not installed behavior. Verify them again against the final checkout.

| PR | Landed revision |
|---|---|
| #204 — Skills | `41089b606b89dd44f6b852db77d8664b21b991cd` |
| #212 — Assistant agent behavior | `be9cb2c163c46529a9a4b3472f6a5c69168390f8` |
| #217 — release tooling | `7bc8992c125d67437d5cf4f93be6fe9875bcee87` |
| #218 — cookie advisory | `3751144fc129d0ae3b28777875d69ada2f8b1892` |
| #219 — FreeBSD fix | `ef2273e92d575667e6f556a92e0905ac1888065b` |
| #222 — packaging lint | `7abe4f5e3cb249bacaf2d081d1b11198b4b080c0` |
| #189 — verification harness | `6032271178140b356e4a7ed2e9bb0f1352d81048` |

#### Execution and approval checklist

- [ ] **Continue current development and integrate in `intgral-ai/intcad`.** No branch or PR
  transfer, no pre-release source freeze. Keep the active product PRs and owners; resolve the
  reviewed integration dependencies on public `develop`. Bring in the Migration Agent's
  independent distribution, update trust and license work only after review. Don't push
  confidential code or internal material to the public fork, and leave the unsafe old RC
  automation stopped.
  The designated signing owner must also confirm the production public-key identity and protected
  signing environment before freeze. Do not paste credentials or commit a private key.
- [ ] **Freeze one candidate.** Record a reviewed public `develop` snapshot and the candidate's full SHA.
  Prove the landed integration commits are ancestors of the tested checkout. Under the current
  promotion workflow, that checkout is the `release/v0.1.3` main-target PR merge SHA or the main
  merge-group SHA, not a branch tip. For a promotion PR, record the head SHA carrying the
  required GitHub check separately from the tested merge/group SHA that binds receipts and
  packages. Take accepted bytes from the `packages` job of that same
  `release-gate` run, which calls `release.yml`. Pin the repository, workflow, run ID and attempt.
  Use the current GitHub-hosted jobs. Require the complete 14 package/evidence pairs from the
  [stable release-gate target map](../.github/workflows/release-gate.yml).
  If a partial rerun cannot prove every package came from the pinned attempt, start a clean
  eligible run. Artifact API metadata alone does not record the attempt; in the historical
  rehearsal, all 28 artifact IDs matched upload logs from the corresponding attempt's jobs.
  Verify `build.json`, `contents.json`, target and contract identity and every payload digest
  before assembly. Keep evidence namespaced and reject duplicate or missing payload names.
  Verify the signed checksum data and aliases against the assembled bytes. A run or artifact
  mismatch stops assembly; a changed revision requires a new candidate and new acceptance.
- [ ] **Run Required Checks, Geometry Matrix, Release Gate and three-platform acceptance.**
  Use the existing GitHub-hosted runners for CI and cross-platform builds. Record all
  required-check results on their correct PR heads and Geometry Matrix receipts at the
  tested candidate merge SHA. Both identities belong in the acceptance record.
  The public ruleset API currently reports strict `ci` on `develop` and strict `release-gate`
  on `main`; recheck before integration and promotion. Record all 16 `action-matrix-*`
  shards from `ci.yml`, the determinism job and the Release Gate's kernel-rig result at
  the candidate checkout. Do not substitute a nightly report from another SHA. Bind
  every real install/launch row to the exact artifact digest. Keep test fixtures separate from
  production. Do not create a public stable release to get pre-release test evidence. A
  nonpublishing diagnostic build does not substitute for a required `release-gate` result.
- [ ] **Verify remaining gates and distribution readiness.** Finish online authorization,
  actual packaged runtime and rights checks, package hygiene and Minisign. Prove the public
  source revision matches the accepted asset manifest and rehearse its publisher against an
  approved nonproduction target. With anonymous checks, prove the website and planned
  distribution channel are reachable. Exercise staging or an authorized rehearsal that matches
  the reviewed production wiring, including cross-repo publishing and update rejection. Test the
  exact new client without relying on the development repo for downloads or updates.
  Verify the v0.1.2 route and recovery instructions before publication, including the approved
  fallback for unchanged clients that cannot discover the new distribution origin.
- [ ] **Obtain final publication approval.** Record the CTO's express approval for the pinned
  SHA, artifact digests, destination and version. Publish only those accepted assets to the
  `intgral-ai/intcad-releases` repository, never the source repository.
- [ ] **Verify and hand off.** Without credentials or cached downloads, fetch the published
  checksum list and Minisign signature and verify them against the approved public key,
  release tag and accepted manifest. Then install and launch the public bytes on all three
  OSes. Separately start from the
  unchanged 0.1.2 packages and exercise each supported anonymous production upgrade route end
  to end: discovery, download, apply and restart, or the approved user-facing fallback where
  in-app update is unsupported. Also check that 0.1.3 uses the new origin for subsequent update
  detection, signed update verification and installed-client online authorization. Record
  results in §15.3. If a check fails, stop handoff and escalate to the CTO and
  owners; do not silently replace released bytes, re-tag, or delete the release.
- [ ] **After release and production acceptance, hand off development.** Reconcile `main` and
  `develop`, then pin both final migration SHAs. Refresh the ledger of remaining active PRs.
  Migration Agent initializes private development and verifies permissions, protections and
  basic CI. Owners migrate only their continuing branches and recreate PRs parent-first,
  preserving stack edges and Linear traceability. AWS/long-term CI cost governance is post-release
  work. Once all work is accounted for, obtain CTO approval for development cutover and public
  repo archival. Do not detach the fork, change visibility, rewrite history or delete historical
  releases. Archival does not revoke prior source or license grants.

### 8.2 Signed draft release (plan A), after the gates

The CTO reports that production signing and the independent distribution infrastructure
are configured. The release path after the gates is:

1. **Version and notes.** `release/v0.1.3` carries the workspace version 0.1.3 (our own
   `Cargo.lock` entries; `cbc` and `dpi` stay third-party 0.1.2) and
   `docs/release-notes-0.1.3.md`, the only notes path the publisher accepts at the source
   SHA. Prepared locally at `809fe61b65` on develop `6032271178`; it must be updated with
   the landed features and re-gated before it is a candidate.
2. **Formal Release Gate.** A ready, same-repository `release/v0.1.3` PR to `main` (or a
   stable main merge group) must produce the `stable-release-handoff-a<attempt>` with the
   actual SHA, run/attempt and all 14 package/evidence pairs. Standalone or diagnostic
   dispatches do not qualify.
3. **Agent rights and installed approval.** The owner must record
   `RELEASE_AGENT_LEGAL_APPROVED` and `RELEASE_AGENT_INSTALLED_APPROVED` as
   `<source SHA>:<run ID>:<attempt>` for the exact candidate; a page smoke or a content
   scan is neither rights clearance nor installed-agent acceptance.
4. **Stage the signed draft** from protected `main` with `tag`, `source_sha`, `run_id`,
   `run_attempt` and `notes_file`. A reviewer must approve the `release-signing` job
   before it receives credentials; that approval is separate from publication. The
   workflow creates only a draft, rejects an absent asset digest, reads every asset and
   both checksum lists back, and verifies the signature.
5. **Publication** of that verified draft is a separate owner decision and action.
   Final policy (owner decision): multiple authorized approvers, **one approval is enough**.
   `Huaiyuan-Jing` and `paipeline` may each approve independently, including approving a
   workflow they triggered themselves; admin bypass stays disabled and no agent may approve
   on a human's behalf. The verified-draft summary must still record the exception warning
   when a one-reviewer environment was used.

Nothing here authorizes merging the promotion PR, approving the signing job or
publishing. The source repository stays public and CI stays on GitHub-hosted runners
for this release.

**Review status of the signing path (GPT-6.2 Sol High).** Client trust, gate binding,
independent signature verification against the tracked key, draft-only, read-back, secrets
handling and the notes path pass by inspection. `can_admins_bypass` handling is fixed (missing
field accepted with a warning, `false` accepted, `true` refused, three tests). Two items remain:
the tag→SHA recheck still uses a **local** tag and a stale main checkout, so remote tag drift
after checkout is missed; and the new "one-person release environment with a warning" weakens
separation of duties unless the owner explicitly opts in. **Owner decision (recorded): the
exception is allowed only when an explicit repository/environment switch enables it; the
default stays strict (independent reviewer, no self-review), and the warning must appear in
the verified draft summary so it is auditable.** Local Python gate/publish tests pass on the branch.

**Migration branch status for this path (`5ca2f08edb`, not yet a PR).** The production
public key is now embedded at
`crates/ui-egui/src/update/trusted-release-key.pub` (one 56-character base64 line, no
placeholder), `PUBLIC_KEY` is a plain `&str` from that file so verification cannot be
compiled out, and a test parses it as a minisign key. The publisher's
`scripts/ci/release_handoff.py` verifies the signature it makes against the **tracked**
key (`git show <sha>:crates/ui-egui/src/update/trusted-release-key.pub`, decoded as an
`Ed…` key, `minisign -Vm`) rather than against the secret, and the signing step refuses to
run without `MINISIGN_SECRET_KEY` and writes the secret key only to a temp file removed by
a trap. Local gate/publish-content/publish-verify Python tests pass on that branch.
This is not landed, not reviewed to completion and not a candidate; the key must still be
merged before the freeze as the CTO requires.

### 8.3 Integration conflict map (simulated, not merged)

`git merge-tree` over the current PR heads chained onto develop `6032271178`, in the planned
order #226 → #214 → #224 → #221 → #229:

| Step | Result |
|---|---|
| #226 `int225-ci-race` | clean |
| #214 `feat/int1503-mastra-release-skills` | clean |
| #224 `task/int1497-v013-integration` | **conflicts**: `crates/engine/src/validation/tests.rs`, `packaging/linux/installed-check-inside.sh`, `tools/windows-assistant-smoke.ps1` |
| #221 `ui/int-1500-centre-chat` | clean |
| #229 `int223-slash-menu` | **conflicts**: `crates/ui-egui/src/agent_chat/mod.rs`, `packages/assistant-web/{dist/assistant.js,dist/style.css,inventory.json}`, `packages/assistant-web/src/App.tsx` |

Both conflicting PRs are already based on the same develop, so the real merge will need a
local resolution on the PR branch (rebase onto the merged `develop`), not a GitHub merge
button. The bundled `dist/` must be rebuilt with the pinned generator after the page sources
merge; never merge generated files by hand.

**Stacked resolution prepared for #229.** Rebasing the menu onto #221's head (`9903e8e788`)
conflicted in `agent_chat/mod.rs` (adjacent field blocks — both kept) and
`packages/assistant-web/src/App.tsx` (kept `{header}` and inserted the menu before
`<ComposerRing>`). Regenerating `dist/` with the pinned generator in a clean `TMPDIR` passes
`--check`. The combined tree also needed `#![recursion_limit = "256"]` in
`crates/ui-egui/src/lib.rs`: one `json!` web-state value with both #221's and the menu's fields
exceeded the default macro limit (`cargo check -p solidcraft-ui-egui --profile ci` failed, then
passed). Pushed as `9ca2ddaff2`; Assistant browser suite on that tree: **21 passed**.

**Promotion commit applies cleanly.** `git merge-tree` of `release/v0.1.3`'s single commit
`994b280dda` (version 0.1.3, notes, format 51) on top of the planned integration
(develop + #226 + #214 + #221) returns `rc=0`, and on top of #229's stacked head also
`rc=0`. So rebasing the promotion branch after the feature merges is a no-conflict step;
the promotion PR can then be opened against `main` to run the formal gate.

**Composed candidate dry run.** A local branch `candidate-dryrun` merges, in order,
`#229`'s stacked head (`9ca2ddaff2`, which already contains #221), `#226` (`c12cd1ba70`),
`#214` (`feat/int1503-mastra-release-skills`) and the promotion commit `994b280dda`. All four
merges were clean (no conflicts beyond the ones already resolved on the #229 stack). On that
tree: `scripts/test-format-pending.sh` OK, `aliases/checksums/nightly` packaging tests OK,
`test_gate`/`test_release_content`/`test_package_contents` OK, workspace version `0.1.3`,
`docs/release-notes-0.1.3.md` present and zero pending format files.
`cargo check -p solidcraft --profile ci` on that composed tree: **exit 0**
(`Finished ci profile … in 1m 51s`). #224 is not in this dry run; it is the remaining
integration.

**Integration conflict found and handed to owners (#214 vs merged #230).** Rebasing #214 onto
`e8c4b1be2d` (which contains #226, #231, #221, #230) merges cleanly except
`.github/workflows/release-gate.yml`, where the merged gate's `DISTRIBUTION` check was kept
(it refuses a declaration substituting for inspection). Two tests then disagree with the
merged receipt contract: `test_a_derived_package_needs_a_cleaned_source_scan` fails with
`release-flatpak-x86_64: derived content evidence does not cover the built package bytes;
missing solidcraft.flatpak` because the merged `problems()` requires every package, including
a derived Flatpak/Snap/Arch repack, to record its own artifact digest in `checked`, while
#214 records only `derivedFrom`/`sourceArtifact`; and
`test_aggregate_no_longer_appends_unconditional_mastra_failures` still expects the older
`SOLIDCRAFT_DISTRIBUTION` string. The local rebased branch is kept but **not pushed**: the
packaging owner decides between recording derived artifacts in `checked` (preferred) or asking
the gate owner for a documented exemption.

**Production account configuration is not wired into the formal release build — BLOCKED.**
`docs/account/desktop-configuration.md` (on #224's branch) says the three values
`INTCAD_WORKOS_PUBLIC_CLIENT_ID`, `INTCAD_AUTHKIT_VERIFICATION_ORIGIN` and
`INTCAD_PRODUCT_HTTPS_ORIGIN` are compiled in with `option_env!`, that no production values are
checked in, and that the release builder must set them together. Measured on the same branch:
`release.yml` contains **no** `vars.`/`secrets.`/`INTCAD_*` reference (`rg` output empty),
`staging-packages.yml` covers only the staging tuple, and `repos/.../actions/variables` is empty.
A formal gate run therefore builds packages with no account configuration; the sign-in entry
fails closed. The Login Agent has been asked to add a reviewed production-config path (for
example a protected environment/repo variable surfaced as a `production_account_config` input,
fail closed when absent), record `accountConfig` evidence for the candidate without secrets, and
provide a verification of the final binary plus the Gate B staging plan.

## 9. Acceptance matrix for the candidate (not yet run)

Rows marked *rehearsal* are the §12 tree (`develop` + #204 + this branch), not a versioned
candidate: they show the path works, and they must be repeated for the candidate SHA.

| Platform | Build | Package | Startup | Login | Mastra | Skills + commands | UI | Update | Evidence |
|---|---|---|---|---|---|---|---|---|---|
| Linux x86_64 | 0.1.3 candidate built (Ubuntu 24.04, 5m25s) | 0.1.3 tar 462 MB, sha256 `e30b1cf8…`, checksum list written | PASS (0.1.3 window, sample design) | NOT TESTED | 0.1.3 agent ships, starts from the install prefix, installs the four modelling skills and the context skill; workflow runtime absent (B6) | 0.1.3 agent smoke 8/8 with the four `skill/<name>` commands; slash list not seen (B5) | Assistant page renders | one-click path NOT TESTED (feed URL is a constant); in-place 0.1.2→0.1.3 PASS, design saved by 0.1.2 opens in 0.1.3 | §12.3 |
| Windows x64 | 0.1.3 exe + cli built (cargo-xwin, 3m40s + 3m36s) | 0.1.3 MSI built in the guest (WiX 5.0.2), 100.7 MB, sha256 `894e04fb…` | PASS (window `v0.1.3`, sample design) | NOT TESTED | **no agent** (B7) | NOT TESTED | rail + Assistant page render | one-click NOT TESTED; in-place MSI 0.1.2→0.1.3 PASS (one ARP entry, `intcad-cli 0.1.3`) | §12.4 |
| macOS universal | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | NOT TESTED | CI-only DMG mount, no DMG for a candidate yet |

Verdict: **NO-GO** at the time of writing: no release.yml candidate had been accepted, B9 (PP's UI) is
missing from the scope, and Gate B has not run. E1 is **not** one of the reasons — see the corrected
row above.

## 10. How each gate will be run

Use §§8.1 and 15.3 for current release controls. The historical Gate E sequencing below
does not authorize testing the migration only after publication. A gate row is only PASS
with the artifact, the SHA and the evidence path recorded beside it.

**Gate A, build and package.** `release.yml` on the candidate SHA (through `release-gate` for a
`main`-target PR). PASS needs: every job in `package-builds` green including FreeBSD, `build.json`
  naming the candidate SHA for each target, the built-in Mastra agent and bundled Node
  (`mastra-agent/`) present in each applicable desktop installer
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

**Gate C, skills.** In the installed app, on a clean profile: the five skill folders exist under
`<MASTRA_APP_DATA_DIR>/home/.agents/skills` (and `intcad-text-to-cad` beside them once #204 adds
its set); each `SKILL.md` is byte-identical to the package's copy (the `skill-set.json` stamp);
`skill_search` finds them; one request per skill makes the agent read and use it; and the
`skill/<name>` commands the Assistant actually shows are recorded as a screenshot. A file on disk
is not evidence that the skill can be called.

**Gate D, UI.** Installed app at 100 % and 150 % on Linux, Windows and macOS:
every built-in theme, font and control size, no unexpected background or clipping, the
Assistant web view positioned correctly, panel resize and move, and a basic CAD operation on each.

**Historical Gate E (obsolete for 0.1.3).** Do not follow its checksum naming, discovery or
publication-order instructions; §§8.1 and 15.3 define the current migration and fallback
sequence. Its old constraint was that the one-click path can only be
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
them, and offers the four modelling `skill/<name>` commands — the part #204's PR could only show from a
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
| Branch | `int1503-rc-candidate`, a **local candidate** branch. First attempt: `develop` `3751144fc1` + #214 `054d51e4d`, SHA `3ffacdb1`, run 45241269258 — **cancelled** once `develop` moved on and the branch was rebuilt. It uploaded seven artifacts from the slices that finished (both Linux architectures, Web, and their evidence); the Windows and macOS jobs had started and were cancelled before uploading, so it is not a candidate. |
| Current SHA | `14a6c95ae527b2f584035448513ae98455e2f709` = `develop` `be9cb2c16` (carries #218 cookie, #219 FreeBSD, #212 agentic and the `context-skills` set) + #214 `b71d51641` (packaging, agent, and the stage-agent test fix) + the 0.1.3 version commit |
| How started | `gh workflow run release.yml --ref int1503-rc-candidate` → run **45248432170** |
| Why not `release-gate` | it only starts for a `main`-target PR or a merge group, and this branch is not a promotion |
| Workflow settings inherited from #214 | `SOLIDCRAFT_WITH_MASTRA_AGENT: "1"`, `SOLIDCRAFT_DISTRIBUTION: "mastra-agent"`, per-arch overrides (`x86_64` only), legacy `mastra/` still `"0"` |
| What it produces | the nonpublishing packages for every target, `build.json`/`contents.json` per package, and the content scan that checks the agent is inside |
| Note on the first attempt | that run was cancelled after uploading the seven artifacts above; its Windows and macOS jobs were cancelled before they uploaded, so it has no such package and cannot stand in for the candidate. Run 45248432170 itself is **attempt 2**: the first attempt lost the `macOS build x86_64` slice, the rerun rebuilt it and the whole run finished `success` |

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
directory → five skill folders in two sets land in the user's data folder.

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
    mastra-agent present, 10,739 files, five skill folders in two sets
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
MSI, portable zip — this download held Linux x86_64, Windows x64 and macOS only; Flatpak, Snap,
Arch, aarch64 and the Windows x86/arm64 builds come from their own artifacts and were not in it),
and the list over the downloaded set then passed:

```
aliased intcad-linux-x86_64.AppImage, intcad-linux-x86_64.deb, intcad-linux-x86_64.rpm,
        intcad-macos-universal.dmg, intcad-windows-x64.msi, intcad-windows-x64-portable.zip
checksums.sh --require <tar> --require <msi> --require <dmg> -> 18 files, every hash OK
```

`checksums.sh` also stopped listing `build.json`/`contents.json`: they are per-package evidence, not
downloads. Tests: `aliases_test.py` 8, `checksums_test.py` 15, both run by `packaging-lint.yml`.

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

### 12.4.11 Linux Gate E with the CI artifact (2026-11-23, late)

Redone with the **CI-built** tarball rather than the rehearsal one, in a clean Ubuntu 24.04 VM
(`…rc013`), same user data folder throughout:

```
install published intcad-0.1.2-linux-x86_64.tar.gz
  (sha256 d6de276e94fed11e172db214b702141aa9d3e193c47ea9782765607534c6854b)   -> intcad 0.1.2
launch, save the sample as ~/upgci/from-012.sc (45,561 bytes), app.quit  -> "0.1.2 exited"
replace the tree with the CI candidate's tarball                        -> intcad 0.1.3
launch on that design -> ui.documents = [{"name": "from-012", "dirty": false}]
open the Assistant    -> agent runs: ~/apps/intcad/lib/intcad/mastra-agent/node …/agent/main.mjs
data folder           -> intcad-context, solidcraft-{checking,editing,mechanisms,modeling}
```

Both platforms now have the upgrade with release bytes: Linux here, Windows in §12.4.6. What neither
shows is the one-click path, which needs a published release (§10).

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
| Checksum list could omit a channel's asset | one run per platform, no completeness check | `packaging/checksums.sh --require <name>` per channel asset, whole-name match, list verified before it replaces anything, both published names written together, and a failed rename removes both rather than leaving mismatched lists; 15 tests, including the failure paths (subfolder, symlink list, symlinked `.new`, hard-linked `.new`, missing require, newline in a name) |
| Top-level symlinks were silently left out | no check | refused with a message; both scans (`-type f` and the nonregular entries) write to a temporary file first, so a failing `find` stops the run instead of shortening the list |
| A failing `find` could shorten the list silently | read from a process substitution | names read into a temporary file first |
| E1 described backwards | "0.1.2 cannot update because 0.1.2 published `SHA256SUMS`" | the shipped 0.1.2 asks for `SHA256SUMS.txt` in the release it **fetches**, so 0.1.3's naming is what matters; the branch fallback is tolerance for future clients only |
| Feed override denied | "a constant with no override" | `SOLIDCRAFT_UPDATE_FEED` exists; asset URLs must be GitHub release URLs, which is what blocks a local payload |
| A cancelled run read as evidence | run 45241269258 implied as the candidate | it was cancelled; its seven artifacts cover only the finished Linux/Web slices, and its Windows/macOS jobs stopped before uploading; §12.4.1 names the current SHA |

Still unchanged and still open: PP's UI (#221) is not ready, login Gate B unrun (#213), slash menu missing
(B10), macOS untested, one-click update only testable after a stable release exists.

## 12.7 What CI itself verified for the candidate (run 45248432170, SHA `14a6c95ae`)

Beyond what I installed by hand, the workflow's own steps passed for this candidate:

| Job | Steps that passed |
|---|---|
| `Linux x86_64` | set up the pinned Node; build the agent folder and run it against stand-ins; build AppImage/deb/rpm/tar; check the packages; **install the packages on bare systems and open the Assistant** (ubuntu 22.04, 24.04 x11/wayland, debian 13, fedora 44, plus the AppImage); keep the logs and pictures; bind package bytes to the checkout |
| `Windows x64 (unsigned)` | build the agent folder and run it against stand-ins; build and check the MSI and portable zip; **install the MSI and open the Assistant in the installed app**; keep the logs and pictures; bind package bytes |
| `FreeBSD x86_64`, `Linux aarch64`, `macOS build aarch64`, `Windows x86/arm64`, `Flatpak`, `Arch`, `Snap`, `Web` | build/check steps succeeded |
| `macOS build x86_64` | cancelled on the first pass; rebuilt on the rerun, which is why the run is attempt 2 and its conclusion is `success` |
| `All nonpublishing package builds` | failed on the first pass (because the cancelled slice) — the expected, honest result |
| `macOS universal` | skipped until its two slices exist |

So the CI installed checks (which do **not** look at the agent, §12.4.2) prove the app and the
Assistant page on six Linux package/desktop combinations and on the Windows MSI; the agent's
installed behaviour is what my VM runs in §12.4.3/§12.4.4 add.

## 12.8 The reproducible candidate run, as a script

**Historical only: do not run this script for 0.1.3.** See §§8.1 and 15.2 for its replacement
requirements and known failure modes.

The candidate is no longer assembled by hand. `~/.cache/int1503/rc-chain.sh` does the whole run and
logs to `~/.cache/int1503/rc-chain.log`:

1. wait until #214 is `MERGED`;
2. reset the candidate worktree to `origin/develop` (so the agent packaging is the merged code, not a
   local merge), set `[workspace.package] version = 0.1.3` and `cargo update --workspace --offline`
   when it is not already there, commit, force-push with lease to `int1503-rc-candidate`;
3. print the two agent settings the branch's `release.yml` carries, dispatch `release.yml` on that
   SHA, watch it, and if the pass is not `success` list the red jobs, `gh run rerun --failed` once and
   watch again;
4. download every channel artifact (`release-linux-{x86_64,aarch64}`, `release-windows-{x64,x86,arm64}`,
   `release-macos`, `release-flatpak-{x86_64,aarch64}`, `release-snap-{x86_64,aarch64}`) into one
   folder, and the evidence artifacts — named `package-evidence-release-<package id>`, not
   `package-evidence-<platform>` — elsewhere;
5. write the unversioned aliases with `packaging/aliases.sh`, then run `packaging/checksums.sh
   --require` for **every name `asset_name()` returns** plus the versioned tarballs: both Linux
   architectures' `tar.gz`, the unversioned `AppImage`/`deb`/`rpm`/`flatpak`/`snap`, all three
   versioned MSIs and their unversioned aliases, all three portable zips, the versioned standalone
   exe and both DMG names. It does **not** require the CLI zip or the `.zsync` files, because no
   client fetches those by name; they are still in the list because the list covers every file in
   the folder. It **exits non-zero** if a required name is missing, so a short list stops the run
   instead of being logged and carried on.

That is the reproducible release flow this document owes the next person: the same five steps a
release manager runs by hand, in one command, with the refusal behaviour tested in
`packaging/tests/checksums_test.py`.

## 13. Decisions from the CTO (2026-11-23)

Historical decisions. The current plan is public integration and release, then migration
after production acceptance, with separate publication and later cutover/archival approvals (§8.1).
The earlier Mac and UI exception language is not an authorization to waive either gate.

Recorded as given: **version 0.1.3**; **PP's UI must be in this release**; everything else is
approved except what needs real Mac hardware. In practice that means:

- 0.1.3 is the batch version; the version commit is prepared on `int1503-version-013`
  (`Cargo.toml` + the 25 workspace entries in `Cargo.lock`) and will be carried by the
  `release/v0.1.3` promotion PR once the scope is on `develop`.
- The FreeBSD fix (#219) and the updater name fix (#217) are approved to land.
- Publication is authorised in principle, but only after the gates pass. The Mac-dependent items
  (native Intel hardware, Gatekeeper/quarantine first launch, real-Mac V2/V7 login runs) stay
  NOT TESTED and are not exceptions to the rest.
- **PP's UI is PR #221** (draft, `BEHIND`, `ui_containers` red), so the release cannot be complete
  until it is ready or the CTO explicitly excepts it.

## 13.1 Scope handed over on 2026-11-23 (after the CTO discussion)

Tianwei's and PP's PRs were handed to this role to merge and prepare for packaging. The scope was
decided explicitly:

- **0.1.3 keeps its current scope plus PP's UI (#221).** The RC is cut from `develop` once #217,
  #221, #189 and #214 are merged.
- **Tianwei's stress-estimate series goes to 0.1.4, not 0.1.3.** #169–#208 are fifteen stacked
  drafts based on each other; only #169 has ever had a CI run (its `ci` job is green), and the
  children are based on feature branches, so no child has been built by CI at all. The chain also
  adds external `gmsh`/`CalculiX` runtime tools, which needs its own packaging and dependency audit.
  Merging starts only after the 0.1.3 candidate SHA has been dispatched, so the chain cannot leak
  into 0.1.3.
- **PP's #221 is merged here** (its `ui_containers` failure fixed in `87f9c7c630`: the Assistant's
  centre layer is `ui_kit::Container`, not a raw `egui::Area`) after a green CI; the visual pass on
  Linux and Windows follows.
- **PP's #189 (the verification harness) is merged here**; it is tooling (`.agents/skills/verify-intcad`,
  `.github/workflows/assistant-ui.yml`, `scripts/ci/affected.py`), not product behaviour.
- **PP's #133, #139, #153 and #154 are not in 0.1.3.** The first three conflict with `develop`;
  #154 renames the installers and programs (249 files). Each gets its own conflict resolution and
  review round, then its own decision.
- **A personal acceptance instance**: once the required PRs are on `develop`, this role builds
  `intcad` from that SHA and opens it on the host for the user to verify by hand, while the
  packaging run proceeds in CI.

### 13.1.1 The personal acceptance instance (2026-11-24 02:15Z)

Built from the #221 head `f4a7cf179b` (develop `7abe4f5e3c` + PP's UI + the `ui_containers` fix):

```
worktree  /home/hyjing/Code/intgral/intcad/worktrees/int1503-preview
build     cargo build -p solidcraft --bin intcad   -> intcad 0.1.2 (dev profile)
launch    ~/.cache/int1503/launch-preview.sh --sample
profile   ~/.cache/int1503/preview-profile   (own data/config/cache; the installed 0.1.2 is untouched)
window    XWayland on purpose: the Assistant is a WebKitGTK child window, and a native Wayland
          window shows a diagnostic instead of the page. The app prefers native Wayland on niri
          because xwayland-satellite breaks window management, so this instance trades that for
          a visible Assistant.
control   ui.documents -> [{"name": "Sample Plate"}];  ui.agent_chat -> panel state (minimized:false)
log       no "native Wayland one ... diagnostic" line, so the page is in an X11 child window
```

## 14. Decisions resolved, and what is still open

Historical snapshot. Use §15.3 for current blockers. This section does not authorize
publication, a UI/login/Mac exception, or moving pre-release upgrade evidence after publication.

Resolved by the CTO on 2026-11-23 (see §13): version 0.1.3; PP's UI in scope; FreeBSD fixed rather
than dropped (#219 merged); the updater name change approved (#217); publication authorised in
principle once the gates pass, with Mac-hardware items explicitly excluded.

Still open, and not decidable from here:

1. **PP's UI (#221, B9).** Draft, `BEHIND`, `ui_containers` red; the scope item the CTO requires
   cannot be integrated until that check is green and the PR is marked ready.
2. **Login Gate B.** #213 is closed; #224 carries INT-1497 and its `Windows app startup` check fails; the whole of
   Gate B is unrun. Login development is not this role's work.
3. **B10, the slash menu.** The Assistant has no consumer of `available_commands_update`; the four
   `skill/<name>` commands work as typed text only. This needs an app-side feature and a decision on
   whether 0.1.3 ships without it.
4. **macOS.** Requires CI's DMG build to finish (§12.4.2) and, for Gatekeeper, notarization and
   native Intel, a real Mac.
5. **One-click update.** Only testable against a published stable release (§10, Gate E).

## 15. Go/No-Go, 2026-11-23 (evidence to that moment)

Historical rehearsal verdict only. The old GO instructions below are superseded by §8.1:
do not restart `rc-chain.sh`, waive the UI or login requirement, or use rehearsal bytes.

Verdict: **NO-GO.** Not because the packages are broken — the 0.1.3 candidate builds, installs,
starts, carries the agent and installs five skill folders on both Linux and Windows — but because three
required things do not exist yet and one is unverified.

| Gate | Status | Evidence / what is missing |
|---|---|---|
| A. Build and package | **PASS** | run 45248432170 (attempt 2) finished `success`: every job, including `macOS x86_64` on the rerun and the universal DMG (489,745,075 B). `build.json` binds every package to `14a6c95ae…`. aarch64/Flatpak/Snap/Arch/FreeBSD/Web were built by the run but not downloaded or installed here. |
| A. Content | **PASS for Linux + Windows** | `contents.json` on both: `agent.present`, `node`, five skill sets, `missing: []`; the Linux tarball has 13,605 agent entries and the Windows MSI 10,739 agent files including `node.exe` (§12.4.3, §12.4.4) |
| A. Checksums | **PASS for what was downloaded** | `checksums.sh --require` produced an 18-line list (16 files plus the two legacy `.zsync` names) under both names with every hash OK, and refused a set that lacks the DMG (§12.4.3, §12.5). It proves the tarball, MSI and DMG are listed, not the full published set |
| B. Login | **FAIL / NOT RUN** | #213 draft, `Windows app startup` red; no first-login, session, expiry, logout or network-failure run exists |
| C. Skills | **PARTIAL PASS** | installed and used on Linux with a real model (§12.5); installed and started on Windows (§12.4.4); agent smoke 8/8 with the four `skill/<name>` commands; **slash menu missing (B10)** |
| D. UI | **PARTIAL** | Assistant page renders on Linux and Windows, themes and DPI not exercised, PP's UI absent (#221 not ready) |
| E. Upgrade | **PARTIAL** | 0.1.2 → 0.1.3 in place passes on Linux tar and Windows MSI (one ARP entry, `intcad-cli 0.1.3`); the one-click path is untestable before publication (§10) |
| F. Evidence | **PASS for what is claimed** | every row above names a run, file or screenshot; nothing is marked PASS without one |

Since this verdict, the macOS DMG exists and every asset for the three installed platforms is present
(§12.4.8, §12.4.9), the
alias gap that would have blocked MSI/DMG/AppImage updates is closed (B11), and the CI Windows MSI was
installed, upgraded from the published 0.1.2 and run with its agent (§12.4.6). What has **not**
changed: #214 is still unmerged, so no accepted candidate comes from a merged `develop` SHA; PP's UI
(#221) is not ready; Gate B has not run; B10 is open.

To turn this into GO: land #214, rebuild from the merged `develop` SHA and repeat the Linux/Windows
installs on that exact SHA (`~/.cache/int1503/rc-chain.sh` does the rebuild, the package run, the
download, the aliases and the `--require` checksum list, and exits non-zero if the list is
incomplete), then add PP's UI and the login run — or record them as
explicitly approved exceptions — and publish only after that.

### 15.1 Current decision: Assistant visual acceptance blocks 0.1.3

**NO-GO.** The personal Linux/XWayland preview (`f4a7cf179b`, §13.1.1) showed an oversized
Assistant composer and a dark rectangle with a partially clipped status line after collapse.
This is pre-merge preview evidence, not an installed-package result. The local #221 merge watcher,
the dependent #214 merge watcher, and `rc-chain.sh` are on hold; do not restart this automatic
path until the fix and the visual acceptance evidence are recorded.

| Check | Status | Evidence / required follow-up |
|---|---|---|
| Composer size | **FAIL in Linux preview** | With the same bundled page in WebKitGTK, changing only the root size from 125% to 100% changed the input text from 17.5 px to 14 px and the composer height from about 109 px to 88 px. The external-agent size rule is shared across platforms. |
| Exposed backing after collapse | **REPRODUCED in Linux X11 VM** | At 125% app scale, a pixel outside the rounded panel but inside the WebView rectangle was RGB (13,13,13). Closing the Assistant changed the same pixel to RGB (28,28,28), the colour of the neighbouring viewport. |
| Partially clipped status | **OBSERVED in Linux/XWayland preview; not reproduced in XFCE/X11** | The first fixture produced `Connection failed · OpenCode`. A follow-up kept synthetic ACP connected and reproduced the original missing-`intcad-cli` notice in the compact row. Its text ellipsized horizontally; no vertically clipped glyphs were found in the sampled frames. |
| macOS visual acceptance | **NOT TESTED** | The macOS host uses a transparent WKWebView and a separate clipping container, but shares the page's sizing and folding logic. Verify open, collapse and resize on macOS; the Linux result does not count as a Mac pass. |

The user saw the clipped notice after expanding the conversation and then collapsing it.
A later screen update appeared to restore the row, but the action and timing are unknown.
Capture the transition as well as the settled state: compare a no-input replay with separate
replays that move the pointer or resize the window after collapse. This is a timing clue,
not proof of a repaint defect or an explanation for the exposed backing.

The isolated `ui221-repro` VM initially could not run the local NixOS build (glibc 2.44 vs
Ubuntu 24.04's 2.39). The Ubuntu 24.04 E2E builder has now produced a standalone visual-test
binary from clean source `f4a7cf179bd358db94931623e97a2d34f59aeb73`, with verified SHA-256
`57f993c3074001af4cbb55811a1eaa84b9771472dcb130db024669ed97b36761`. The deployed guest binary
matched that hash. It is not an installer or a release candidate.

Local evidence lives in the `ui221-repro` run's `diagnostic/` directory: screenshots
`10-fractional-expanded.png`, `11-fractional-collapsed.png`, and
`12-fractional-assistant-closed.png`, with the corresponding X11 geometry logs. At 125% app
scale the WebView shrank from 650×375 to 650×284 physical pixels, held its bottom edge at the
same position, and matched `ui.rects`. This is evidence against a window-size mismatch in
that transition, not an explanation for the clipped notice in the XWayland screenshot.

The same binary's `acp-followup/` evidence shows a second backing symptom at 960×750 and 125%
app scale: the compact notice had returned, but the expanded backing still covered the
viewport. In the no-input replay, the last sample with extra backing was at 390 ms and the
first with the viewport restored was at 405 ms. Pointer-move and resize replays also cleared
before either input at 650 ms. A separate untimed replay kept extra backing in its settled
screenshot. The samples were roughly 15 ms apart in the first 500 ms, not a capture of every
presented frame. The collapsed panel's own rectangular backing remained after the extra area
cleared. These are separate observations; neither proves the cause of vertical clipping.

To clear the visual gate, use the merged candidate's installed packages to check consistent
composer sizing across agent types, fully visible or fully hidden status text, and no exposed
backing rectangle or stale content. Repeat collapse and resize at normal and scaled DPI in light
and dark themes. Existing login, slash-menu, update, and publication-approval gates still apply.

### 15.2 Parallel release audits

- **#221 native shape fix does not build yet — BLOCKED locally.** The E2E build of
  `27bdaa577e` failed to compile `apps/solidcraft/src/assistant_web.rs`: `connect_map_event`
  needs `gtk::prelude::WidgetExtManual`, and the import was `WidgetExt` (unused). This is
  exactly the class the isolated geometry driver hid. Fix and a real
  `cargo check -p solidcraft --profile ci` are in progress; no native acceptance exists yet.
- **#221 native shape — first measured run: corners fixed, top-edge band still backed.**
  Exact binary `a2b88840ebc8d8f4abc9b39d04e8949c0ad81416ab8ec63d0e2a0b66ef2d4076`
  (source `a5753cedec`) in the `ui221-backing` VM at 125%, 960×750, collapsed height 137
  (same geometry as the recorded baseline): `(802,639)` now `(28,28,28)` and `(800,473)`
  now `(133,133,133)`, both equal to the hidden reference — before the fix both were
  `(13,13,13)`. The whole empty region still fails: **494 of 1300** baseline-empty pixels
  differ with the opaque `(13,13,13)`, concentrated in the child's top rows from `y=471`
  (e.g. `(319,471)` expected CAD `(28,28,28)`). Rows from `473` down are correct. This is
  a real remaining defect, not a pass.
  **Second measured run** (binary `38390a1f41b55ad01a296e86ca1cf1e4c5a99cb41dde0126ca0bad0b981a29f1`
  from `a58265778e`, same VM, geometry and starting state): both named corners pass, and the
  empty-region mismatches fell from **494 to 31** of 1300. The remaining 31 are a 1-px band
  along the **top-right rounded arc** (`(783,472)`, `(784,472)`, `(786,473)`, `(789,474)`,
  `(792,476)` …), all still `(13,13,13)` where CAD should show: the arc's partially painted
  pixels are inside the mask.
  **Third measured run — PASSES the settled-state native check** (binary
  `2058a119a8927041efd3ba2a51fd5e8a34c4421f8af0b6d0a389a0a587a5b521` from `9903e8e788`,
  same VM/geometry/state, `SOLIDCRAFT_ASSISTANT_SHAPE_TRACE=1`): **0 of 1300** empty pixels
  differ from the hidden reference, `(802,639)` = `(28,28,28)` and `(800,473)` =
  `(133,133,133)`, and the trace shows the native child shaped for the page report
  (`native=650x171 page=Some((520.0, 136.0, 2)) fits=true rectangles=Some(35) shaped=true`).
  Independent visual confirmation on the same captures (GPT-6.2 Sol High): the dark
  rectangle and corners are gone, the top band is clean, and the status row, ellipsis,
  chevron, composer and rounded corners show no clipping or mask artefact.
  **Not measured:** the expand→click→collapse transition, 100%/150%, popup click and
  dismissal, first-key focus, click-through outside the shape, and wide↔narrow resize. The
  transition replay was attempted twice with the offline OpenCode fixture and both times
  stopped at the layout wait (`height 137` while the fixture's notice never appeared), so
  it is recorded as NOT RUN rather than inferred from the settled captures.
- **#221 composer sizing — local browser fix reviewed, not landed.** `b002647acb` held the
  centred input at 14 px and the composer at 86 px for both agent modes in the same browser
  viewport. Independent Spec and Quality review approved; controller rerun had 12 passes and
  one pytest-plugin deprecation warning. This is not installed acceptance. Native backing work
  continues separately; vertical clipping stays unverified.
- **#223 slash commands — IN PROGRESS, Release Manager owns it.** The issue was open and
  unassigned; the Release Manager took it for 0.1.3 ([issue note](https://github.com/intgral-ai/intcad/issues/223#issuecomment-7122373320)).
  Implementation is in local branch `int223-slash-menu` from `develop` `6032271178`.
  A read-only review at `b002647acb` confirms the packaged agent emits
  `available_commands_update` (`agent-smoke.mjs`), while the client's `acp.rs` ignores it.
  Handoff: scope displayed commands to the active ACP session. In the bundled-browser
  regression, `/` and `/skill` show the advertised command, selecting it keeps it in the
  draft, and an old session's late update cannot repopulate a new session. An empty list
  clears the current session's menu. Coordinate the composer and bundle with PP and the
  Rust state/event bridge with the Login Agent; leave authorization unchanged. No code
  change or acceptance run for #223 yet. Ownership and acceptance were requested on
  [the existing issue](https://github.com/intgral-ai/intcad/issues/223#issuecomment-7121993793).
- **PR CI timeout is a runner/mirror flake, not a code failure.** #226's `CI` on `c12cd1ba70`
  was cancelled at exactly the 30-minute job bound: the `Native Linux selected regression gate`
  log shows `Run sudo apt-get update` starting at `07:41:09` and the next line at `08:09:07`
  (cleanup), i.e. ~28 minutes of silence inside `apt-get update`. The same job took **3 min 39 s**
  on the successful run at `ba52b76d4c`. Re-running is the right response; a later infrastructure
  change should add bounded retries/timeout to that step so a slow mirror fails fast instead of
  burning the job's whole budget. Evidence it is per-runner: in the same attempt, `Native
  Linux Clippy` finished its apt step at `08:36:42` while the gate job had been in apt since
  `08:15:08`. A separate PR (#231) now bounds every `apt-get update` in `ci.yml` and
  `release.yml` to three 180 s attempts.
- **`develop` required CI is red — BLOCKED.** The latest `CI` run on merged `develop`
  (`6032271178`) is [run 45299000127](https://github.com/intgral-ai/intcad/actions/runs/45299000127),
  conclusion `failure`. Its `Native Linux selected regression gate` failed the
  `known-failing` check with a regression not on the list:
  `solidcraft-cli::script_roundtrip::cli_status_and_cancel_from_a_second_process_do_not_reconcile_a_live_submit`
  (`apps/solidcraft-cli/tests/script_roundtrip.rs:660`) with
  `error: REPORT_MISMATCH: a different terminal report already exists for this run`, which
  looks like leftover run state rather than a product regression; the owner must decide
  whether the test's isolation is wrong or the runner state must be cleaned. Filed with
  the logs and the owner question as [#225](https://github.com/intgral-ai/intcad/issues/225).
  The Release Manager owns the fix. Local branch `int225-ci-race`: the regression test
  reproduced the exact CI error first, then `crates/engine/src/validation` adopts the
  winning verifiable terminal report (and its state) when a writer published only the
  report; a different terminal state still returns `REPORT_MISMATCH` (a guard test covers
  that). Focused run: `56 passed, 3594 filtered out` on `validation::tests`;
  `cargo clippy -p solidcraft-engine --all-targets --profile ci -- -D warnings` reported no
  issues; `cargo fmt --all -- --check` and `git diff --check` were clean. Published as
  [#226](https://github.com/intgral-ai/intcad/pull/226). Hosted CI passed
  (`known-failing: 17 ran, 17 pass … no regression`), **but** the PR's selected gate need not
  include that CLI test, so a local run of it is still required.
  An independent review then found that adopting *any* winning terminal report can bind a
  terminal state to a report a worker may still replace (a provisional `passed` report that
  becomes `cancelled`). The fix was narrowed: adopt only a **cancelled** report when the
  cancellation marker exists, and a new guard test proves a provisional result report still
  returns `REPORT_MISMATCH`. Pushed as `dc71fadad3`; focused run after the narrowing:
  **57 passed**, and a local run of the originating CLI test itself now **passes**:
  `cargo test -p solidcraft-cli --profile ci --test script_roundtrip --
  cli_status_and_cancel_from_a_second_process_do_not_reconcile_a_live_submit --exact`
  → `1 passed, 7 filtered out (1 suite, 0.12s)`. That is the path the hosted gate had
  failed on, so the fix is proven against the original failure rather than by a subset. Two other failures in that job are
  listed known failures (SMLib variable fillet `Unsupported`, the dialog variable round
  status 1001). The `ci` verifier then failed with `rust: real job result is not success`
  and `rust: missing receipt`. Until `ci` is green at a reviewed `develop`, no release-gate
  candidate can pass; this is not caused by the 0.1.3 feature PRs and is reported, not fixed
  here, because the file belongs to its owner.
- **RC provenance — BLOCKED.** `45248432170` attempt 2 built rehearsal SHA `14a6c95ae`, not
  a descendant of the current merged `develop` snapshot `6032271178`. Rehashing the cached
  Linux x64, Windows x64 and macOS payloads against their descriptors validates those
  bytes, not a new candidate. Do not resume the old `rc-chain.sh` unchanged: it can continue
  after failed Git preparation or a failed run, omits four package channels, and can
  overwrite same-named evidence while flattening. Require the landed #217/#189/#221/#214
  commits to be ancestors of one reviewed `develop` snapshot, then bind the run ID, workflow,
  SHA, final success, and all 14 package/evidence pairs to the same candidate. Verify each
  manifest and payload hash before flattening, keep evidence namespaced, and reject duplicate
  payload names. The four PRs are the packaging and UI prerequisites, not the whole release
  scope: verify the approved login and slash-command fixes are also present before treating an
  RC as satisfying those feature gates.
- **#214 package builds — green on the author's head.** `Package builds (nonpublishing)`
  run 45332120863 on `5e94f155fa` (which contains the reviewed commits) concluded `success`
  with no failed job. The branch is still `BEHIND` develop and needs an update before merge.
- **#214 package verdict — local fix reviewed, not landed.** At `6717ef201c`, the
  workflow's real package-evidence step inspected a fixture tarball with a complete agent
  file layout except Node. The report named `mastra-agent/node` as missing, but the step
  exited `0`: its final failure check was inside the native-distribution branch. Local
  commit `bad89b5c4f` on `int1503-package-verdict` makes the failure check apply to both
  branches. Review exposed a second problem: the inspector missed the agent in
  `0.1.3-beta.1` archive wrappers. That failed in real workflow tests for Linux tar and
  Windows portable ZIP before `83a38fbf97` fixed wrapper recognition. Stable Linux and
  both prerelease fixtures now pass with a Node entry and explicitly fail without one.
  Native packages still reject runtime markers and unreadable archives.
  The scoped review passed those cases and found that an invalid numeric prerelease
  identifier with a leading zero was still recognised. The `beta.01` archive regression
  failed before `020da5b3d2` aligned that rule with the workflow's version check.
  The local gate, release-content and package-content suites passed 73, 27 and 28 tests
  respectively, and `actionlint` passed for `release.yml`. These tests do not launch a
  package or replace hosted CI and the separate release gate. Scoped re-review passed.
  Final review found a case newly reachable after the inspector learned prerelease
  roots: a legal `preview-cli-build` version makes the app's portable ZIP contain
  `-cli-` in its filename. `carries_no_agent()` mistakes it for a CLI-only archive.
  A verified mixed fixture (complete MSI, ZIP missing Node) yielded no problems from
  the aggregate check. Restrict the CLI exemption to the actual CLI archive name and
  prove both the standalone and aggregate paths reject the broken ZIP.
  **Local follow-up:** `ee17261b82` restricts the exemption to the canonical macOS headless
  CLI ZIP. Independent scoped review confirmed both verdict paths reject the missing-Node
  package. Controller rerun: gate 74/74, release-content 30/30, package-contents 28/28, and
  `actionlint` on `release.yml` exited zero. The standalone regression uses a real ZIP and
  a fixture for the MSI file listing. It does not prove a real MSI install or hosted CI.
  The stack is not on #214; integration and real-package evidence are still blocked.
  **Landed on the PR:** the four reviewed commits (including `ee17261b82`) were
  rebased onto the branch and pushed; the author then added `e4fac55e57` on top. New CI
  is running on that head.
- **Gate B — BLOCKED / NOT RUN.** #213 is now closed; #224 carries INT-1497. The previously
  [tested Windows job](https://github.com/intgral-ai/intcad/actions/runs/45109730998/job/137479303661) passed
  startup, then timed out in `tools/windows-resize-smoke.ps1`: after moving the window to
  `(20,20)–(1004,640)`, `WM_NCHITTEST` at `(20,330)` did not return `HTLEFT` within 2 s
  (Win32 error 1460). Assistant smoke was skipped, not failed. The latest completed green
  `develop` job and the older package rehearsal do not include the login change. The newer
  [#224 login integration PR](https://github.com/intgral-ai/intcad/pull/224), head `0519d3aba6`,
  has a separate [failed Windows startup job](https://github.com/intgral-ai/intcad/actions/runs/45314675673/job/138179044898).
  Direct job logs now confirm `windows-resize-smoke.ps1` hit `WM_NCHITTEST` Win32 error
  1460 at `(1,360)` within the 2 s limit and the process was still running at cleanup.
  This is separate evidence for #224, not proof that both failures share a cause. On an
  approved installed candidate, record SHA, installer hashes, a clean profile, offline CAD
  smoke and anonymous controlled-request refusals separately from live staging login.
  The login owner still owns the CI failure and live staging readiness.
  `git merge-tree` on `0519d3aba6` found no conflict with `6032271178` alone, but conflicts
  with the #221 head in the Assistant host, bridge and Windows smoke, and with the #214 local
  patch stack in the bare-system Linux smoke and Windows smoke. These are integration
  conflicts, not license to discard either branch's behaviour. Recheck on the reviewed heads
  and run the affected smokes on the resolved integration SHA.
- **#224 Windows smoke, measured on 2026-11-24 (head `6b0da01b59`).** The resize smoke now
  records the delay instead of dying on it: `WM_NCHITTEST` at `(1,360)` missed the 2 s window
  and **answered after 3918 ms on the second attempt**; every later resize assertion passed
  (drag height +80, maximize with the screen edge as client, restore). The same job then
  failed in the Assistant smoke: after the page grew to `536,172–915,633`, its native edges
  differed from `agent_chat:web` by `0, 0, 0, 7 px` and the loop ended with
  `control read timed out during ui.inspect`.
  **Regression evidence:** the green `develop` Windows job (`138141577913`, run 45299000127)
  shows `PASS: assistant web view`, with **zero** "native page edges differ" lines and no
  hit-test timeout. So the ~4 s UI-thread delay and the 7 px bottom mismatch are introduced
  in #224's range, not load noise.
  **Cause and fix.** The 7 px came from publishing `agent_chat:web` with the panel's
  pre-inset rect while the native child is placed after the custom title bar's resize-zone
  inset: the child was placed correctly, the published rect was 7 px taller (`panel.rs`
  vs `web::placement`). `web::placement` now publishes the rect the native view is actually
  given (`5580bf2976`), with layout, placement and the 2 px tolerance unchanged;
  `cargo check -p solidcraft-ui-egui --profile ci` compiled 233 crates cleanly. The ~4 s
  delay matches DX12 surface reconfiguration on the window thread after maximize
  (`WindowEvent::Resized → surface.configure`), unchanged by this PR, with no account or
  WebView2 work in that probe; it is recorded, not called a product regression.
  **Re-run result (attempt 2, head `5580bf2976`): `Windows app startup` PASSES.**
  `maximized : WM_NCHITTEST at (1),(360) answered after 4261 ms, attempt 2 (2000 ms missed)`,
  then `PASS: resize border` and `PASS: assistant web view` with no edge-difference line.
  The other required contexts were still running; this clears the Windows smoke, not Gate B's
  real sign-in/authorization evidence. A new simulation at local
  `b002647acb` and `ee17261b82` still finds those conflicts with `0519d3aba6`, but no conflict
  when either is tested against `6032271178` or when the two are tested against each other.
- **macOS — NOT TESTED.** Follow the [Mac acceptance runbook](release-acceptance-macos-0.1.3.md)
  for the exact reviewed DMG. An ad-hoc CI launch is not a clean Gatekeeper assessment or
  native Intel acceptance.

### 15.3 Current release readiness: closed-source cutover

**NO-GO.** No frozen candidate SHA, run/attempt or acceptance manifest is recorded. The
`develop` snapshot is `6032271178140b356e4a7ed2e9bb0f1352d81048`. The Migration Agent has
handed off local commit `5f7a3462075fc4a83b38d0ce97f4666d58c1fde1`, not a landed PR or a
candidate. The accepted candidate must come from `intgral-ai/intcad` and its existing
GitHub-hosted CI. `intcad-client` status, initialization and CI readiness are deferred until
after production acceptance and do not block v0.1.3. The migration ledger is kept only in
the Release Manager's local state, never in this public repository.
Baseline checkout
evidence still has old `intgral-ai/intcad` release URLs in the updater fixtures and `PUBLIC_KEY`
set to `None` in `crates/ui-egui/src/update/install.rs`. The baseline `release.yml` is a
nonpublishing build, not proof of the new production signing or publication path.

Anonymous readiness probe (no credentials, redirects followed): `https://intgral.ai/` returned
HTTP 200. Both `https://github.com/intgral-ai/intcad-releases` and its GitHub API
`releases/latest` returned HTTP 404. This does not tell us whether the tentative repository is
absent or private. The homepage's `#download` section says the installers are coming soon
and has no download links in the returned HTML; an accessible homepage is not an accessible
product distribution path.

Legacy upgrade inputs are ready for testing: the published 0.1.2 Linux tarball, Windows MSI
and universal DMG match a freshly downloaded anonymous `SHA256SUMS`. The DMG is newly
downloaded (302,524,433 bytes), not copied from CI. Full digests and local locations are in
`~/.local/state/intcad-release-readiness/v0.1.3/legacy-baseline.Nyw4DRC8/README.md`.
This identifies the old installers only; no cutover, install or migration verdict follows.

The published 0.1.2 Linux binary itself was also inspected (`bin/intcad` from that tarball,
SHA-256 prefix `a5ec6ca6e4618bf9`): its embedded strings are
`https://api.github.com/repos/intgral-ai/intcad/releases/latest`,
the asset allowlist prefix `https://github.com/intgral-ai/intcad/releases/`, and
`SHA256SUMS.txt`. So the unchanged 0.1.2 client can only read a list named
`SHA256SUMS.txt` under the old repository, while every published release carries only
`SHA256SUMS`. Its in-app update is therefore already unable to verify an update today;
this is a fact about the shipped bytes, not a projection from source. All three stable
releases carry `SHA256SUMS` only (v0.1.0–v0.1.2 asset API); no stable release carries
`SHA256SUMS.txt`, so the unchanged client's one-click update cannot succeed regardless of
the migration. No signature
requirement was found in those strings, matching `PUBLIC_KEY = None` at tag `v0.1.2`.

The Release Manager notified the owners to continue in the existing repository and lifted
the earlier push-pause request. The Login Agent owns Gate B and integration, and the Migration
Agent owns distribution, signing, migration and rights evidence. Neither notice authorizes
merging, publication or archival. Migration delivery review is below.

Runtime scope for the current packaging integration is the built-in Mastra agent, including
bundled Node and required Skills (`mastra-agent/`, #214). The legacy workflow runtime
(`mastra/`, B6) is a separate opt-in deliverable, disabled in the rehearsal. Before freeze,
check this scope against the approved migration and product handoff. If the separate runtime
is required, both its inclusion and operation are blocking until independently evidenced and
cleared for redistribution; an agent smoke is not a substitute.

For each result, record candidate SHA, workflow run/attempt, artifact name and SHA-256, host and
OS version, scenario and expected/observed results, evidence link or path, and reviewer. Include
the approved staging configuration or policy revision when it affects the result. Redact
credentials, tokens, prompts and private projects. An earlier rehearsal, file listing, headless
smoke or green CI does not replace real installed-client acceptance.

| Gate | Current verdict | Evidence required for PASS |
|---|---|---|
| Integration, build and package | BLOCKED | Landed PR map in `intgral-ai/intcad`; Required Checks on their correct heads; Geometry Matrix and Release Gate receipts plus complete verified assets for the tested candidate merge SHA from §8.1. #214 must reject incomplete payloads. |
| Linux install and launch | NOT TESTED | Clean supported Linux host; install and launch the accepted desktop package, exercise core CAD and record the actual bytes. Test AppImage update and the documented reinstall/package-manager path where applicable. |
| Windows install and launch | NOT TESTED | Real Windows MSI install and launch; test the applicable portable path, plus window resize, Assistant and core CAD. |
| macOS install and launch | NOT TESTED | Exact universal DMG; real Finder install and launch and the [macOS runbook](release-acceptance-macos-0.1.3.md), including native slice evidence. Rosetta or CI-only launch does not cover a missing native cell. |
| Login, authorization and offline CAD | BLOCKED / NOT RUN | Gate B on all three installed clients: approved sign-in/session paths, denied anonymous or unauthorized controlled requests, version admission, revocation and late-write refusal, and ordinary offline CAD including save/export. #224's Windows failure is not a staging result. |
| Mastra runtime, Skills and slash commands | BLOCKED / NOT RUN | Identify the exact approved runtime variant in each installer, observe it starting from the installed app and completing a real turn. Verify the required bundled Skills can be discovered and invoked and #223's slash menu lists and runs the commands. Earlier `mastra-agent/` evidence does not establish that the separate legacy `mastra/` workflow runtime shipped. |
| Assistant UI | BLOCKED | Installed bottom-centre composer size, clipping, backing, focus and resize evidence on each OS. Use the visual plan and keep the reproduced backing and reported half-line clipping separate. |
| Public distribution and new-client independence | BLOCKED | `intgral-ai/intcad-releases` is the required destination; it returned 404 anonymously. Test anonymous website, release notes, prompts, API and asset links, including redirects. With the source endpoints unavailable to the candidate, it must still discover and download updates and related resources. |
| Minisign acceptance and refusal | NOT TESTED | On installed supported auto-update paths, a valid signed payload verifies and applies with the approved embedded public key. Separately show that a wrong signature, absent signature and tampered installer each stop before the installer is launched or the existing install is modified. Record each refusal and the positive control. |
| Published v0.1.2 migration | NOT TESTED | Start with unchanged public 0.1.2 packages. The public old repo may still serve historical assets after archival, but it will not serve 0.1.3. Prove the supported route under those conditions: discovery or approved fallback, install/restart, design and preference retention, and subsequent discovery in the new channel. An in-place local install alone does not prove discovery. |
| Redistribution and notices | BLOCKED / NOT RUN | Approved shipped-file inventory covering Rust, npm/Node, runtime and assets; resolve applicable `needs-human` items. Inspect the actual packages for the applicable third-party license texts and copyright notices. A Cargo-only audit is insufficient. |
| Package security and hygiene | NOT TESTED | Inspect and scan actual payloads and installed trees, including nested archives. No unapproved credentials, source, debug data, sourcemaps or internal docs. Any approved exception must identify the exact file and approval; missing coverage is not PASS. |
| Cross-repo publication | BLOCKED / NOT RUN | Bind the exact `intgral-ai/intcad` candidate and accepted assets to an authorized nonproduction rehearsal of the public distribution workflow, with scoped credentials and fail-closed read-back. Private CI is post-release work. |
| Production publication and anonymous handoff | NOT RUN | CTO publication approval; then fresh anonymous download/install/launch, independent production 0.1.2 upgrade or approved fallback, and new-client detection, signature and authorization checks. |

### 15.3.1 What has to happen for a GO

In order, with the owner who must act. Nothing here is approved yet.

1. Clear the red `develop` gate [#225](https://github.com/intgral-ai/intcad/issues/225) so `ci`
   can pass at a reviewed snapshot (owner of `script_roundtrip.rs`).
2. Land #214 with the reviewed `ee17261b82` (author applies or approves it) and #221 with
   the reviewed sizing `b002647acb` plus the shape fix after native acceptance.
3. Land #223 (Release Manager, in progress) and obtain PP's visual sign-off on the final
   Assistant, including the native backing evidence at 125% and 150%.
4. Green Gate B from the Login Agent on #224: real staging sign-in, version admission,
   revocation and offline CAD on installed clients; the Windows `WM_NCHITTEST` 1460 failure
   must be fixed, not waived.
5. Land the reviewed Migration Agent fixes (publisher provenance, checksum/signature contract,
   key identity, distribution destination, notices) and prove cross-repo publication in an
   authorized nonproduction rehearsal.
6. Cut the promotion PR, record the tested merge SHA and all 14 package/evidence pairs,
   install that exact Linux/Windows/macOS artifact set, and complete the acceptance table.
7. Publish to `intgral-ai/intcad-releases` only with CTO approval; then verify anonymous
   download, signature, update discovery and install on the published bytes.

**Local E2E harness defect found while installing the integration build (release infrastructure).**
`intcad-e2e` derives the VM name from the worktree and task and truncates it to 60 characters;
its SSH identity check then compares the truncated expected name with the guest hostname and
refuses ("refusing to act on another VM"). Task `int1503-integ` produced a 61-character name and
the deploy failed before any product check. Retrying with the short task `i3int` gives 57
characters and reuses the same build artifact. Fix belongs in the harness (compare the truncated
name or reject over-long names with a clear error message), not in the release docs.

### 15.4 Integration preview (local, not a candidate)

A local integration branch proves the reviewed pieces combine, without claiming candidate
status. Worktree `int1503-integration-preview`, branch of the same name, built from the
reviewed `develop` snapshot `6032271178` plus two merge commits:

- `3b04fbece5` merges the reviewed #214 packaging verdict fix `ee17261b82`.
- `4ce096fc37` merges the reviewed #221 composer sizing fix `b002647acb`.

Both merges used `ort` with no conflicts. On that tree:

| Check | Result |
|---|---|
| `python3 -I scripts/ci/test_gate.py` | 74 tests, OK |
| `python3 -I scripts/ci/test_release_content.py` | 30 tests, OK |
| `python3 -I scripts/ci/test_package_contents.py` | 28 tests, OK |
| `packaging/tests/{aliases,apprun,checksums,flatpak,mastra_agent,mastra_runtime,web_channels}_test.py` | OK individually |
| `packaging/tests/nightly_test.py` with `jq` on `PATH` | 12 tests, OK |
| Assistant browser suite `.agents/skills/verify-intcad/browser/test_assistant_ui.py` | 12 passed, one pytest-plugin deprecation warning |
| `windows_icon_test.py` | NOT TESTED: no `powershell` on this host |

### 15.5 Rehearsal package hygiene and notices (not a candidate)

Independent listing of the earlier rehearsal Linux tarball
`~/.cache/int1503/rc-complete/intcad-0.1.3-linux-x86_64.tar.gz`
(SHA-256 prefix `35f114b267946d38`) shows what a shaped 0.1.3 package carries. It is not
candidate evidence; it is a check that the packaging shape is right.

| Item | Result |
|---|---|
| Entries | 13,656 |
| Licences and notices under `share/doc/intcad/` | `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, `THIRD-PARTY-LICENSES.txt`, `ASSISTANT-THIRD-PARTY-NOTICES.txt`, `WEB-VIEW-PLATFORM-NOTICES.txt` — all present |
| Built-in agent | 13,605 `mastra-agent/` entries, including `node`, `agent/main.mjs` and five `SKILL.md` (four modelling skills plus `intcad-context`) |
| Sourcemaps, `.pdb`, `.dSYM`, `.git` | none matched |
| Internal docs (`docs/release*`, `docs/superpowers`, `.dev/`) | none matched |
| Rust/TypeScript source outside `node_modules` | none matched |
| Credential-shaped names | 243 matches, all inside dependency packages (e.g. AWS credential providers) except `mastra-agent/agent/secrets.mjs`, which is the agent's key-withholding module: it strips provider keys from the command sandbox and throws if the SDK shape is unexpected. No secret value is present. |

The Windows rehearsal portable ZIP (`rc-complete/intcad-0.1.3-windows-x64-portable.zip`)
was listed directly (11,646 entries, Python `zipfile`): `LICENSE-APACHE`, `LICENSE-MIT`,
`NOTICE`, `THIRD-PARTY-LICENSES.txt`, `ASSISTANT-THIRD-PARTY-NOTICES.txt` and
`WEB-VIEW-PLATFORM-NOTICES.txt` are present, `mastra-agent/node.exe` and
`mastra-agent/agent/main.mjs` are present, and no sourcemap, `.pdb`, `.git` or internal
release docs matched. The build-time `contents.json` for `release-windows-x64` records
`agent.present: true`, `node: true`, the five skills and `missing: []`. The MSI itself could
not be listed here (`7z` shows the compound document, not installed files); its installed
inventory remains the Windows CI/installer check.

Open questions this does not settle: whether shipped `node_modules` documentation and the
modelling-skill `scripts/tests/` should be trimmed for size/policy, and whether every
dependency text required by licence is present (that needs the legal audit, not a listing).

**Deployed binary, Linux VM (`i3int`, 2026-11-24T05:21Z run):** the integration build
finished (`release` profile, 36 m 10 s) and the E2E harness deployed that exact artifact
(`solidcraft` SHA-256 `9ab6189e9514a1f859bfa5a1e83d34651b8f77bc1f6d947c61105af909771e28`,
source `4ce096fc37`) into the Ubuntu guest and started it.

| Step | Result |
|---|---|
| 1 build | PASS, artifact digest bound to the source commit |
| 2 deploy/start from expected path | PASS, guest binary hash equals the artifact |
| 3 open sample and see model | PASS, sample design opened, hover resolved a Plate edge |
| 4 real drag rotates the model | PASS, pitch changed with the camera otherwise fixed |
| 5 click a face selects it | PASS, `Plate` face selection recorded |
| 6 open Assistant and a control responds | BLOCKED: harness looked for `titlebar:assistant`; #221 moved the entry to `rail:assistant` |
| 7–8 resize / panel control | BLOCKED by step 6 |

Step 6 is a stale harness control name, not a product failure: `accept.py` was patched locally
to accept `rail:assistant` (falling back to `titlebar:assistant`) and the run is repeating.
This is a **deployed binary in a VM**, not an installer and not candidate acceptance.

Not yet run: any Rust build, package build, install, launch, or native UI check. This tree
excludes #224 (login) and the Migration Agent's in-progress fixes, so it is a combination
check, not the release scope. It is not pushed and does not authorize a merge.

**Approved deferrals:** Windows Authenticode, Apple Developer ID and notarization. These do not
block on a certificate request and do not waive Minisign, real installation, or OS-policy
evidence. A real launch prevented by policy is blocked, not PASS.

#### Migration handoff and review

- **Delivery:** `feat/closed-source-distribution` at `5f7a3462075fc4a83b38d0ce97f4666d58c1fde1`,
  based on `6032271178`. Source, handoff and audit are local to the Migration Agent's worktree.
  A read-only merge simulation found no textual conflicts with the recorded `develop` snapshot,
  #224 `0519d3aba6`, #221 plus local sizing `b002647acb`, or #214 plus local verdict fix
  `ee17261b82` individually. This does not establish semantic compatibility on the integrated
  checkout and does not change the separate Login Agent overlap in §15.2.
- **Local checks only:** 10 publisher verifier fixtures passed from the pinned commit;
  `actionlint` on `publish-release.yml` and `git diff --check` exited zero. No hosted publisher,
  credential exchange, key generation or cross-repo upload was run.
- **Provenance mismatch — BLOCKED:** the delivered publisher accepts a standalone
  `release.yml` run, but §8.1 requires the packages from the same stable `release-gate` run
  that was accepted. The publisher's handoff must bind the actual tested checkout, approved
  run and attempt, and accepted artifact digests. For this release the source workflow
  belongs to `intgral-ai/intcad`; future private-repo support is a separate task.
- **Independent standards review — CHANGES REQUIRED:** `publish-release.yml` mixes evidence
  into the flat checksum directory, reads a key file different from the client/test file, and
  permits a public release before full read-back. The publisher must separate staging and
  publication, require every read-back digest, and treat dispatch inputs as data. New updater
  errors also require the repository's locale handling.
- **Independent spec review — CHANGES REQUIRED:** the verifier does not require the whole
  14-pair set, the original artifact hashes or contract identity. A local verifier fixture
  with `build.json` removed still exited 0 and emitted `"build_revisions": {}`. The new guide
  still leaves part of the old download path in place; user-facing v0.1.2 instructions must
  not rely on its old in-app Download button. Re-review the Migration Agent's next pinned
  commit. Local verifier fixtures are not evidence that the publisher can safely publish.
- **Trust and distribution — BLOCKED:** the migration commit changes the feed and adds a
  signing contract; the embedded key remains `None`, the public-key file is a placeholder,
  and no approved signing identity or publisher environment was supplied. The proposed
  destination is not publicly reachable. No received path grants an exception to Minisign
  or moves migration out of the 0.1.3 scope.
- **Legacy migration — NOT TESTED:** pinned v0.1.2 source fetches the old repository's latest
  endpoint and only accepts its release asset URLs. It expects `SHA256SUMS.txt`, while
  v0.1.2's public release serves `SHA256SUMS`; its production signing key is `None`,
  so it does not itself require Minisign. The Migration Agent proposes an official download
  and in-place installation fallback when 0.1.3 is served from the new distribution repo.
  It is not approved or verified on
  the exact old and candidate installers. The new candidate must enforce Minisign for
  its own in-app updates.
- **Rights and publication rehearsal — BLOCKED:** the handoff raises shipped font and asset notices,
  agent dependency attribution and legacy workflow runtime licensing. It is a baseline
  audit, not legal clearance or a final installer inventory. A local worktree does not
  prove cross-repo publishing can run from `intgral-ai/intcad`. Private CI is not a v0.1.3 gate.

### 8.4 Measured merge of #214 and #224 heads (2026-11-24T17:45Z)

Re-measured with the merged develop base, because the earlier map used superseded heads.
A throwaway worktree merged `origin/develop` (`f18a9aec27`) + #214 (`d4819c6a9d`) + #224
(`da73900d6b`):

| File | Result |
|---|---|
| `.github/workflows/release-gate.yml`, `release.yml` | clean; both the agent env and `production_account_config`/`inject-auto` survive |
| `crates/engine/src/validation/tests.rs` | no longer conflicts (#226 is already merged) |
| `packaging/linux/installed-check-inside.sh` | **content conflict** |
| `tools/windows-assistant-smoke.ps1` | **content conflict** |
| everything else | clean |

Both conflicts are semantic, not textual:

1. #224 moves the Linux checker to `packaging/linux/assistant-check.py` and keeps
   `packaging/assistant-check.py` for macOS; it rewrites
   `installed-check-inside.sh` to call the Linux path. #214 adds the built-in-agent
   block that runs `check+=(--built-in-agent)`. The merged script must keep #224's path
   **and** #214's block.
2. #224's `packaging/linux/assistant-check.py` has **no `--built-in-agent` option**
   (measured: zero mentions), while #214 adds that option and its assertions to
   `packaging/assistant-check.py`. Merging both as-is makes the Linux installed check
   fail on unknown arguments for every package that carries the agent, which is exactly
   the candidate package. The option and its three assertions (agent offered first,
   `currentAgent == mastra`, plans reported; stopped agent reports no connections;
   it starts again) must be ported into the Linux copy.
3. `tools/windows-assistant-smoke.ps1`: #224 rewrites the whole launch/click/focus flow
   (owned job, `SendInput`, isolated profile); #214 appends `CheckBuiltInAgent` and calls
   it when `mastra-agent\agent.json` sits beside the installed exe. The ported function
   must be re-expressed in #224's flow (its `OpenAssistant` helper no longer exists).

Neither PR's own CI can catch this, because each is green on a tree that lacks the other.
The second merge must therefore include the port and re-run the installed checks; a
GitHub merge button cannot resolve it.

**The merged #214 + #224 tree compiles.** On the same throwaway merge tree (develop
`f18a9aec27` + `#214 d4819c6a9d` + `#224 da73900d6b`), `cargo check -p solidcraft-ui-egui
--profile ci --locked` finished with **exit 0** (`Finished ci profile … in 1m 58s`), with
`CARGO_TARGET_DIR` in a fresh directory. So the agent and account changes to
`crates/ui-egui/src/agent_chat/*` do not break each other; the packaging-check conflicts
above are the only measured blockers in this pair.

**#214's own packaging suites pass locally.** On `d4819c6a9d`, with `jq` on `PATH`:
`unittest discover -s packaging/tests -p '*_test.py'` ran **118 tests, 0 failures,
0 errors** (excluding `windows_icon_test.py`, which needs PowerShell and is covered on the
Windows runners). `nightly_test.py` alone: 12 tests OK; it fails without `jq` only because
the harness insists on that tool.
