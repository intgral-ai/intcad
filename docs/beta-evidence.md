# Beta packaging and validation evidence

This page records the packaging, install and content evidence for a Beta
candidate. It is not a publication authorization. It does not move a tag, change
a release asset, sign anything or claim that the release gate is green.

Last checked: 2026-11-10 (UTC) against `intgral-ai/open-skyfall-desktop`.

## 1. Candidate and evidence boundary

| Item | Value |
|---|---|
| Promotion source | `develop` |
| Candidate examined here | `359cf584933a88948b3f56f3acc878bfd4ab1197` |
| Candidate contract digest | `sha256:f5e9ed0238da5b9f1ce5cf70bff4a23ed7134476cab5c43b717134c4a282662a` |
| Published Beta tag | `v0.1.0-beta.1` |
| Tag object | `c9d104cf8876d99824709ba1d0c902ce3ac09fe0` |
| Tag commit (peeled) | `f3285bdb729e2f027abb694f4c97758a4b75353d` |
| Candidate evidence run | [42912913457](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42912913457) is queued with no jobs yet |

The candidate has **not** been built as a Beta promotion. `42912913457` was
still pending when this page was written, so it proves nothing yet.

If promotion, conflict resolution or a rebase changes the merge SHA, every
package, descriptor, install check and BYOK/GUI result below must be rebuilt or
rerun at the new SHA. A tree that looks equal is not a substitute.

The published `v0.1.0-beta.1` tag and its assets are frozen. Do not move the tag,
replace an asset under the same version or present those bytes as the new
candidate.

## 2. Build evidence

| Run | SHA | Workflow | Jobs and artifacts | Result | What it proves |
|---|---|---|---|---|---|
| [42771648772](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42771648772) | `f3285bdb7` | Package builds (nonpublishing) | All jobs: Linux x86_64/aarch64, Flatpak x86_64/aarch64, Arch x86_64/aarch64, Snap x86_64/aarch64, FreeBSD, macOS universal, Windows x64/x86/arm64, web; `All nonpublishing package builds` | success | A nonpublishing build existed at the Beta.1 tag commit and every package job passed. |
| [42812058134](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42812058134) | `1a45733ef` | Package builds (nonpublishing) | `release-linux-x86_64`, `release-linux-aarch64`, `release-macos`, `release-windows-x64`, Flatpak, Snap, FreeBSD, web; Arch x86_64/aarch64 failed | failure | The later content-scanning workflow built Linux, macOS and Windows packages at `1a45733ef`. Arch failed. This SHA is an ancestor of the candidate, not the candidate. |
| [42790743092](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092) | `7bf03f6d9` | Release gate (manual) | `packages` jobs for Linux, Flatpak, Snap, FreeBSD, macOS, Windows and web succeeded; Arch failed; aggregate failed | failure | The full gate was red. The exact failures are listed in section 5. |

`42771648772` is the Beta.1 package run. It built all package jobs, but its
`package-evidence-*` artifacts contain only `build.json`. The `contents.json`
runtime-marker scan did not exist at that commit.

`42812058134` is the first run in this evidence set whose `package-evidence-*`
artifacts contain both `build.json` and `contents.json`. It proves the scan can
pass for Linux, macOS and Windows at `1a45733ef`; it does not prove the
candidate.

## 3. Published Beta.1 asset and install evidence

### 3.1 Assets and hashes

`v0.1.0-beta.1` is a published prerelease. The GitHub asset digests and
`SHA256SUMS` agree:

| Asset | Size (bytes) | SHA-256 |
|---|---:|---|
| `solidcraft-0.1.0-beta.1-linux-x86_64.tar.gz` | 132,777,900 | `92862630de39a8a2ae38e6cbfcea8d74fc9050ad0a9972139dbed6f26dc10307` |
| `solidcraft-0.1.0-beta.1-macos-universal.dmg` | 301,037,796 | `081b8ede4d3831aece2b27556f743ff96d12d7fcd48ee19234c7f97358142740` |
| `solidcraft-0.1.0-beta.1-windows-x64.msi` | 107,274,240 | `dd5fa56aeba1bbcb21a4462a4c5a4ac15461cf51b770ec96ea35023c186e5a27` |

The Linux tar.gz was downloaded again during this evidence pass. Its SHA-256
matched the table, it contained `bin/solidcraft`, `bin/solidcraft-cli`,
`bin/solvecraft-cli` and the licence files, and a name scan found no `mastra`,
`node_modules`, `ee`, `node` or `node.exe` entry. In an `ubuntu:22.04` container
the extracted `solidcraft-cli --version` and `solidcraft --version` both printed
`0.1.0-beta.1`. This is Linux x86_64 install/start evidence for the **published
Beta.1 asset**, not for the candidate.

Only the Linux asset was downloaded in full in this pass. The macOS and Windows
conclusions below come from the existing verify runs.

### 3.2 Verify runs

| Run | SHA checked out by the workflow | Jobs | Result | What it proves |
|---|---|---|---|---|
| [42786271869](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42786271869) | `8e33b6f0d` (throwaway verification branch) | [macOS DMG mount and arch check](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42786271869/job/129728145613); [Windows MSI install and executable check](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42786271869/job/129728145079) | success | The published Beta.1 DMG and MSI could be mounted/installed and the expected executables ran at CLI level. |
| [42785643419](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42785643419) | `740b0ed0f` | Same two jobs | failure | Both jobs failed at `gh release download` with `release not found` because the release was not published yet. This run is not install evidence. |

The successful verify run records:

- macOS: `lipo -info` reported `x86_64 arm64`; `codesign --verify --deep`
  reported `valid on disk` and `satisfies its Designated Requirement`; the app
  printed `solidcraft 0.1.0-beta.1`; the image ejected cleanly.
- Windows: the x64 MSI installed silently to `C:\Program Files\SolidCraft`;
  `solidcraft.exe`, `solidcraft-cli.exe` and `solvecraft-cli.exe` were present;
  `solidcraft-cli --version` printed `solidcraft-cli 0.1.0-beta.1`.

The verify workflow checks out a throwaway branch. Its checkout SHA is not the
product source. The evidence binds to the published tag and the asset SHA-256,
not to `8e33b6f0d`.

The verify jobs do **not** test notarization, Gatekeeper acceptance, SmartScreen,
Authenticode, GUI startup or a model turn.

## 4. Content, dependency and provenance evidence

| Evidence | SHA | Artifact/report | Result | What it proves |
|---|---|---|---|---|
| [42812058134](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42812058134) `package-evidence-*` | `1a45733ef` | `release-linux-x86_64`, `release-macos`, `release-windows-x64` | `markers: []`, `unchecked: []`; Linux checked AppImage, deb, rpm and tar.gz; macOS checked DMG and CLI zip; Windows checked MSI, portable zip and standalone exe | The current content-scanning workflow can prove a native package set at this older SHA. It is not candidate evidence. |
| [42790743092](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092) `package-evidence-*` | `7bf03f6d9` | Linux x86_64/aarch64, macOS, Windows x64, Flatpak, Snap, FreeBSD, web | `markers: []` for the reports that exist, but Linux AppImages were `unchecked`; Windows `contractDigest` mismatched; Arch descriptors were missing | A clean marker scan is not enough when an artifact was not inspected or a descriptor does not bind to the checkout. The gate rejected this set. |
| [42771648772](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42771648772) `package-evidence-*` | `f3285bdb7` | `build.json` only | Linux/macOS `contractDigest` = `sha256:f5e9ed...`; Windows `contractDigest` = `sha256:e2e98e...`; no `contents.json` | Beta.1 package descriptors exist, but the Windows digest does not match the canonical manifest digest and exact runtime-marker evidence is absent. |
| Published Beta.1 Linux tar.gz | `f3285bdb7` tag, asset hash | `92862630...` | No runtime-marker names found | The published Linux archive itself has no Mastra/Node sidecar by name. This is a name scan, not the full `contents.json` gate. |
| [42790743092](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092) jobs | `7bf03f6d9` | `Format and repository checks`, `Mastra artifact policy`, `Dependency advisories` | success | Repository format/policy/licence checks and the dependency-advisory check passed at this SHA. |

`contents.json` is produced from the package bytes. A declaration such as
`SOLIDCRAFT_WITH_MASTRA=0` or `SOLIDCRAFT_DISTRIBUTION=native` is not proof. The
gate requires every canonical package to be inspected, `markers` empty,
`unchecked` empty and the checked names and SHA-256 values to equal `build.json`.
Derived packages must also name an inspected Linux source and matching digest.

### Beta.1 provenance gap

The published Linux tar.gz does not match the Linux tar.gz recorded in the
`f3285bdb7` package run:

| Record | SHA-256 of Linux x86_64 tar.gz |
|---|---|
| Published `v0.1.0-beta.1` asset and `SHA256SUMS` | `92862630de39a8a2ae38e6cbfcea8d74fc9050ad0a9972139dbed6f26dc10307` |
| `build.json` in `package-evidence-release-linux-x86_64` from `42771648772` | `6b538e0b9c9a8091f7a00b3a83563645752474178a8085a3005bb54abb9ebf78` |

The published Linux asset was uploaded at `06:14:22Z`; the `42771648772` run was
created at `06:54:36Z`. The macOS and Windows published hashes do match that
run's `build.json`. The exact build that produced the published Linux bytes is
therefore **not established**. The archive content looks correct and its own
hash matches `SHA256SUMS`, but it cannot be tied to `42771648772` by hash.

Do not repeat this in the next Beta. Use one build run and download the published
bytes from that run, or record the exact repack step and hashes.

## 5. Release-gate failures at `7bf03f6d9`

The manual release-gate run `42790743092` failed. The failures below are the
actual results, not a policy summary:

| Area | Failed job | Evidence |
|---|---|---|
| CI | [Native Linux selected regression gate](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129743367584) | Three non-known failures: `solidcraft-kernel::boolean_props::edge_only_box_join_is_refused_in_both_orders`, `solidcraft-kernel::boolean_props::translation_invariant`, `solidcraft-ui-egui::agent_chat::api_client::tests::host_ai_unsupported_media_and_schemas_fail_before_io`. |
| CI | [Windows app startup](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129743367777) | `resize border: no window (exited: False)`. A fix is proposed in open PR [#90](https://github.com/intgral-ai/open-skyfall-desktop/pull/90); it is not merged at `359cf5849`. |
| CI | [Full baseline randomized properties](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129756687616) | Two SMLib boolean property failures, plus one skipped case. |
| CI | Action matrix shards 1-16 ([example](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129756687999)) | Every shard failed on the known kernel/record regressions. |
| CI | [CI aggregate](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129805625128) | `rust`, `windows-startup`, `property-tests` and `action-matrix` were not successful. |
| kernel-rig | [Build the CLI](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129743404223) | `NO CASE: document.snapshot.capture` in `tests/kernel-cases/EXEMPT`; the command/grader failed. |
| kernel-rig | [kernel-rig](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129752893119) | `kernel-rig evidence: build: real job result is not success`; shards were skipped. An EXEMPT fix is proposed in open PR [#86](https://github.com/intgral-ai/open-skyfall-desktop/pull/86); it is not merged at `359cf5849`. |
| packages | [Arch Linux x86_64](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129760240941) and [Arch Linux aarch64](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129760241038) | Arch package failure. In the later run `42812058134` the actual error was `derived package needs exactly one source Linux tarball`, after the `pkgver` alias copy. A fix is proposed in open PR [#89](https://github.com/intgral-ai/open-skyfall-desktop/pull/89); it is not merged at `359cf5849`. |
| packages | [All nonpublishing package builds](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129799514714) | At least one package job failed, so the aggregate failed. |
| release-gate | [release-gate](https://github.com/intgral-ai/open-skyfall-desktop/actions/runs/42790743092/job/129805718073) | `ci`, `kernel-rig` and `packages` were not successful; the kernel-rig handoff was missing; Windows `contractDigest` mismatched; Arch descriptors were missing; Linux AppImage content was unchecked; derived packages had no verified Linux source. |

PR state at the time of writing:

- [#86](https://github.com/intgral-ai/open-skyfall-desktop/pull/86) is open and
  draft (`fix/kernel-rig-snapshot-exempt`).
- [#89](https://github.com/intgral-ai/open-skyfall-desktop/pull/89) is open and
  draft (`fix/arch-source-alias`).
- [#90](https://github.com/intgral-ai/open-skyfall-desktop/pull/90) is open and
  blocked (`fix/windows-resize-title`).

A promotion at `359cf5849` does not include those fixes. If they land, the
promotion SHA changes and every package and check must be rebuilt there.

The host-AI test failure in the `Native Linux selected regression gate` job was
fixed by merged PR #85 (`9a2e4c9d0`), which is in the candidate. Do not carry that
specific failure forward. The two boolean-property failures remain the known
kernel issue.

## 6. What the three-platform Beta promotion build must prove

The minimum Beta package set is Linux x86_64 (and Linux aarch64 if it is
offered), macOS universal and Windows x64. Each platform has two separate
claims: **package/build content** and **environment-executable install/start**.
Do not merge them into one green check.

| Platform | Exact artifacts | Build/content checks | Install/start checks | Hosted runner sufficient? |
|---|---|---|---|---|
| Linux x86_64 | `release-linux-x86_64`; `package-evidence-release-linux-x86_64` | SHA-256 of tar.gz, deb, rpm and AppImage; `contents.json` has `markers: []`, `unchecked: []` and checked hashes equal `build.json`; AppImage was inspected, not skipped; no Mastra/Node/`ee` names; `contractDigest` equals the candidate manifest digest | Extract the tar.gz; run `solidcraft-cli --version` and `solidcraft --version` in a clean Ubuntu 22.04 container with a minimal `PATH`; launch the GUI on a real desktop | CLI and content yes; GUI and GPU/Vulkan no |
| Linux aarch64 (if built) | `release-linux-aarch64`; `package-evidence-release-linux-aarch64` | Same content and digest checks; the AppImage must be present and checked | Run the ARM64 binaries on an ARM64 Linux environment; do not use x86 emulation as a GUI proof | Build/content yes on an ARM runner; GUI no |
| macOS universal | `release-macos`; `package-evidence-release-macos` | DMG and CLI zip inspected; `markers: []`, `unchecked: []`; both slices present; `codesign --verify --deep` passes; record that the signature is ad-hoc and not notarized; `contractDigest` matches | Mount the DMG; run `SolidCraft --version`; open the app on a clean physical Mac; record Gatekeeper behaviour | Mount, `lipo`, code-sign and CLI yes; GUI, Gatekeeper and real Apple-silicon/Intel launch no |
| Windows x64 | `release-windows-x64`; `package-evidence-release-windows-x64` | MSI, portable zip and standalone exe inspected; `markers: []`, `unchecked: []`; MSI header/version; record Authenticode status; `contractDigest` matches | Silent MSI install; check `C:\Program Files\SolidCraft`; run `solidcraft-cli --version`; launch `solidcraft.exe` on a clean physical Windows machine; record SmartScreen | Install and CLI yes, but the Release package job currently skips its own CLI check (see section 7); GUI, SmartScreen and a clean machine no |

Every platform also needs these repository-level checks at the exact candidate:

- `cargo xtask ci` passes, or the promotion records exactly which required jobs
  are still red and why.
- `cargo xtask licences --check` passes; `cargo deny`/the Dependency advisories
  job passes.
- `packaging-lint.yml` passes for the package scripts and workflows.
- The `package-builds` job (`All nonpublishing package builds`) reports success for
  every package job that was run.
- Each `build.json` has `sourceRevision` equal to the candidate, the expected
  `target`, a non-null `contractDigest`, and the SHA-256 of every built file.
- `contents.json` proves native distribution. A workflow declaration is not
  enough.

`release.yml` still builds all package jobs, including Arch, Flatpak and Snap.
A failure in any of them fails `All nonpublishing package builds`. If the Beta
only ships three platforms, do not describe the other jobs as Beta evidence;
their failure still blocks a full release-gate run.

`release-gate` expects a descriptor for every one of its 14 canonical package
ids, not only the three Beta platforms. A three-artifact download can support a
Beta note. It cannot replace the main-target gate.

### Checks that need a physical or clean machine

These cannot be proven on a hosted runner and must not be claimed from the runs
above:

- First GUI launch, window creation, GPU selection and the real Vulkan, Metal or
  D3D12 driver path.
- File dialogs, opening and saving a design, and the normal desktop integration.
- Gatekeeper on macOS after download, and SmartScreen on Windows.
- A clean machine with no development toolchain, no repository checkout and no
  preinstalled runtime.
- Real BYOK provider calls, host validation and the Anatomy click-to-locate flow.
- macOS on both Apple silicon and Intel hardware.
- Windows on a normal desktop with the expected system runtime.

Hosted runners are useful for package bytes, content scans, mount/install and
CLI `--version`. They are not a clean-machine GUI acceptance.

## 7. Known gaps

| Gap | Status | Detail |
|---|---|---|
| Candidate build | **not done** | No package run exists at `359cf5849`. `42912913457` is queued with no jobs. |
| Candidate install/start | **not done** | All install/start evidence below is for `v0.1.0-beta.1` or an older SHA. |
| Linux GUI/BYOK | **partial, old SHA** | The real DeepSeek Linux acceptance belongs to the published Beta.1 asset at `f3285bdb7`. It cannot be reused for `359cf5849`. |
| macOS BYOK | **unknown** | No real-provider model turn on macOS. Only mount, `lipo`, code-sign and CLI checks exist. |
| Windows BYOK | **unknown** | No real-provider model turn on Windows. Only MSI install and CLI checks exist. |
| Clean physical-machine GUI | **not done** | No clean physical-machine run for any platform. |
| Windows signature | **unsigned** | The workflow has no signing step and the release notes call the MSI unsigned. SmartScreen was not tested. Do not call it signed. |
| macOS signature | **ad-hoc, not notarized** | `packaging/macos/package.sh` uses an ad-hoc signature without `--sign`. `codesign --verify` passed; notarization, stapling and Gatekeeper acceptance did not run. |
| Beta.1 Linux provenance | **unbound** | Published tar hash `92862630...` differs from the `f3285bdb7` build artifact hash `6b538e0b...`. The build that produced the published bytes is not recorded. |
| Beta.1 exact content scan | **partial** | No `contents.json` exists for the Beta.1 build run. The published Linux archive was name-scanned here; macOS and Windows exact assets were not rescanned. |
| Beta.1 Windows contract digest | **mismatch** | `build.json` has `sha256:e2e98e...`; the canonical manifest digest is `sha256:f5e9ed...`. The release gate rejected it. |
| Linux AppImage inspection at `7bf03f6d9` | **failed** | The AppImage was missing from the artifact, so `contents.json` marked it unchecked and the gate failed. The later `1a45733ef` run inspected it; the candidate has not been checked. |
| Arch package | **failed; fix open** | PR #89 is open and unmerged. |
| Windows startup | **failed; fix open** | PR #90 is open and unmerged. |
| kernel-rig | **failed; fix open** | PR #86 is open and unmerged. |
| Full kernel/property/action matrix | **red** | Known kernel/record regressions. Packaging does not fix them. |
| Release gate | **red at `7bf03f6d9`** | Candidate gate result is unknown. A Beta package build alone does not satisfy `release-gate`. |
| Windows x86 and arm64 | **not Beta evidence** | The workflow builds them. The Beta does not require them. x86 binaries were not run on the x64 host; arm64 binaries were not run at all. |
| Windows package-job CLI check | **skipped** | `packaging/windows/package.ps1` tests `$CanRun`, but that variable and `$HostArch` are not defined. The `42771648772` x64 log says `skipping solidcraft-cli --version: an x64 build doesn't run on this  machine` (double space from the empty `$HostArch`). The Beta.1 verify run proves the MSI for Beta.1; a new candidate needs a separate install/CLI check. |
| Full package downloads | **not fetched** | This pass fetched `build.json`/`contents.json` evidence and the published Beta.1 Linux tar.gz. It did not download the full Linux/macOS/Windows package blobs from the old runs. |
| Signing credentials | **none** | There are no Apple or Windows signing credentials and no notarization. |

## 8. Candidate binding

- Every real BYOK/GUI result belongs to the exact SHA and exact package bytes
  that produced it. The Beta.1 Linux DeepSeek result belongs to
  `f3285bdb7` and the published Linux tar.gz with hash `92862630...`.
- It cannot be carried to `359cf5849` or to a later promotion merge SHA.
- A verify run that downloads a published asset is bound to that asset hash, not
  to the throwaway branch SHA in its checkout.
- A `package-evidence-*` descriptor from another SHA is not candidate evidence.
  `sourceRevision`, `contractDigest` and artifact hashes must all match the
  candidate.
- Old Beta.1 evidence must not be presented as a new build. If the next Beta uses
  a new `beta.N`, build new artifacts and attach only those after review.

## 9. Reviewer checklist for the promotion PR

Replace `<RUN>` and `<SHA>` with the actual run and candidate SHA. Do not mark a
box from an older run.

- [ ] The promotion PR targets `main`; `<SHA>` is the PR head or merge SHA and is
      recorded in the PR body.
- [ ] `release.yml` run `<RUN>` has `headSha == <SHA>` and
      `All nonpublishing package builds` succeeded.
- [ ] Downloaded artifacts are `release-linux-x86_64`,
      `release-linux-aarch64` (if offered), `release-macos` and
      `release-windows-x64`.
- [ ] `package-evidence-release-linux-x86_64`, `...-linux-aarch64` (if offered),
      `...-macos` and `...-windows-x64` were downloaded from the same run.
- [ ] Every `build.json` has `sourceRevision == <SHA>`, the expected `target`,
      the candidate `contractDigest` and the hash of every downloaded artifact.
- [ ] Every `contents.json` has `distribution: "native"`, `markers: []`,
      `unchecked: []`, and checked names/hashes equal `build.json`. Any derived
      package names an inspected Linux source with a matching digest.
- [ ] The Linux tar.gz hash matches `build.json`; the AppImage is present and
      inspected if it is shipped.
- [ ] `solidcraft-cli --version` and `solidcraft --version` run from the Linux
      tar.gz in a clean Ubuntu 22.04 container.
- [ ] The macOS DMG mounts; `lipo` shows `x86_64 arm64`;
      `codesign --verify --deep` passes; `SolidCraft --version` runs. The record
      says ad-hoc, not notarized.
- [ ] The Windows MSI installs silently; both executables are present;
      `solidcraft-cli --version` runs. The record says unsigned.
- [ ] A real GUI launch on a clean physical machine is recorded for Linux,
      macOS and Windows, with screenshots or a recording.
- [ ] macOS and Windows BYOK runs are recorded for `<SHA>` if the Beta claims
      them. A Linux-only BYOK result is labelled Linux-only.
- [ ] `release-gate` run for `<SHA>` is checked. If it is red, the PR says which
      jobs are red and does not call the Beta release-gate green.
- [ ] The known gaps in section 7 are copied to the PR with any changes.
- [ ] No tag was moved, no Beta.1 asset was replaced, and nothing was published.

## 10. Commands for a nonpublishing candidate build

The dispatch `ref` must be a branch or tag, not a raw commit SHA. Point a
temporary branch at the exact candidate, confirm the run's `headSha`, then
delete the branch after the build if it is no longer needed. Do not create a
tag for this.

```sh
CANDIDATE=359cf584933a88948b3f56f3acc878bfd4ab1197
BRANCH=release/beta-candidate

git switch -c "$BRANCH" "$CANDIDATE"
git push -u origin "$BRANCH"        # only when the promotion owner authorizes it

gh workflow run release.yml \
  -R intgral-ai/open-skyfall-desktop \
  --ref "$BRANCH"

gh run list -R intgral-ai/open-skyfall-desktop \
  --workflow release.yml --limit 5 \
  --json databaseId,headSha,status,conclusion

RUN=<run-id>
test "$(gh run view "$RUN" -R intgral-ai/open-skyfall-desktop --json headSha -q .headSha)" = "$CANDIDATE"
gh run view "$RUN" -R intgral-ai/open-skyfall-desktop --json conclusion,jobs
```

Download the three-platform set and its evidence:

```sh
mkdir -p dist
for artifact in \
  release-linux-x86_64 release-linux-aarch64 release-macos release-windows-x64 \
  package-evidence-release-linux-x86_64 package-evidence-release-linux-aarch64 \
  package-evidence-release-macos package-evidence-release-windows-x64
do
  gh run download "$RUN" -R intgral-ai/open-skyfall-desktop \
    -n "$artifact" -D "dist/$artifact"
done
```

Check every descriptor and content report:

```sh
CONTRACT="sha256:$(git show "$CANDIDATE":schema/mastra-runtime/v1/manifest.json | sha256sum | cut -d' ' -f1)"

for f in dist/package-evidence-*/build.json; do
  jq -e --arg sha "$CANDIDATE" --arg contract "$CONTRACT" \
    '.sourceRevision == $sha and .contractDigest == $contract' "$f"
done

for f in dist/package-evidence-*/contents.json; do
  jq -e --arg sha "$CANDIDATE" \
    '.sourceRevision == $sha and .distribution == "native"
     and (.markers | length == 0) and (.unchecked | length == 0)' "$f"
done
```

Linux x86_64:

```sh
cd dist/release-linux-x86_64
jq -r '.artifacts[] | "\(.sha256)  \(.name)"' build.json | sha256sum -c -
mkdir -p /tmp/osc-linux && tar -xzf ./*.tar.gz -C /tmp/osc-linux
docker run --rm -v /tmp/osc-linux:/pkg:ro ubuntu:22.04 bash -lc '
  B="$(find /pkg -maxdepth 1 -type d -name "solidcraft-*-linux-x86_64" | head -1)"
  PATH=/usr/bin:/bin "$B/bin/solidcraft-cli" --version
  PATH=/usr/bin:/bin "$B/bin/solidcraft" --version
'
```

macOS (on a Mac):

```sh
hdiutil attach -readonly -nobrowse -mountpoint /tmp/osc-mnt \
  dist/release-macos/*.dmg
APP="$(find /tmp/osc-mnt -maxdepth 1 -name '*.app' -type d | head -1)"
lipo "$APP/Contents/MacOS/SolidCraft" -verify_arch arm64
lipo "$APP/Contents/MacOS/SolidCraft" -verify_arch x86_64
codesign --verify --deep --verbose=2 "$APP"
"$APP/Contents/MacOS/SolidCraft" --version
spctl --assess --type execute --verbose=4 "$APP" || true  # ad-hoc is expected to fail
xcrun stapler validate "$APP" || true                    # not notarized is expected to fail
hdiutil detach /tmp/osc-mnt
```

Windows (elevated PowerShell):

```powershell
$msi = Get-ChildItem dist\release-windows-x64\*.msi | Select-Object -First 1
Start-Process msiexec.exe -ArgumentList @('/i', "`"$($msi.FullName)`"", '/qn', '/norestart') -Wait
Get-ChildItem 'C:\Program Files\SolidCraft'
& 'C:\Program Files\SolidCraft\solidcraft-cli.exe' --version
Get-AuthenticodeSignature 'C:\Program Files\SolidCraft\solidcraft.exe' |
  Format-List Status, StatusMessage
```

Then run the GUI and the BYOK flow on a clean physical machine. Record the exact
candidate SHA, asset hash, provider, model, prompt, report/snapshot ids and
screenshots. A run on another SHA is not evidence for this candidate.

## 6. Beta.3 candidate evidence (run 43241626115)

Candidate head `e04a00b2662d062e528e59ee26d0127d48f2e1a8`; PR #106 tested
merge `196b44b15d0cdd38947af3e60c3b40fe4f013b33` (tree
`49fd420d3c94245acb35b6cfa6ba068b0e228357`). All descriptors below bind that
`sourceRevision`.

| Platform | Evidence | Result |
|---|---|---|
| Linux x86_64 | `solidcraft-0.1.0-beta.3-linux-x86_64.tar.gz` sha `a9f0426a7561cd35bb0591ef1a4136e953df198ff6d758b1e0f99b05aac3a5bd` | Extracted in a clean `ubuntu:22.04` container: `solidcraft-cli 0.1.0-beta.3`, `solidcraft 0.1.0-beta.3`. |
| Linux aarch64 | tar sha `8da20bb8cd08805f7d77d8d499f979b34fa2332a46f2e1f73bea614a6b9e1f1f` | Content scan `markers=[]`, `unchecked=[]`; not launched (no aarch64 host). |
| Windows x64 | MSI sha `95bb005ab6762564bc20dff86b7f346e1f6628f3521c0947f70864d725e8348a` | MSI cabinet CLI/app `FileVersion/ProductVersion 0.1.0-beta.3`; install/start check pending. |
| Windows x86 / arm64 | content scans clean | not launched (no x86/arm64 host). |
| macOS universal | pending | First x86_64 build job was cancelled by hosted-runner capacity; retried in the same run. The aarch64 half already reports `solidcraft 0.1.0-beta.3`. |

The package descriptors have `markers=[]` and `unchecked=[]`; the checked files
match `build.json`. The stable-only #14/#15 integration stops are not part of
the Beta descriptor path.

Accepted gaps remain: unsigned/not notarized, no clean physical-machine GUI
acceptance, and macOS/Windows real BYOK unverified. No tag or public release was
created; `v0.1.0-beta.1` and its assets remain untouched.

