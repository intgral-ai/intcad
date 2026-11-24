# Releasing Intcad

Package builds and the release gate never publish. The separate draft workflow
does not authorize a release, a tag, signing or credential setup. Those actions
require explicit owner approval. Store submissions, Pages deployments and
container pushes remain outside this release path.

## Candidate and final source

Development integrates through `develop`. Use a temporary same-repository
`release/vX.Y.Z` PR to `main`, not a persistent `release` branch. Task PRs squash;
release promotion and main/develop reconciliation use **merge commits**, preserving
both histories. Never replace, reset or force-push `main`.

Choose a workspace version from the actual user-visible changes; moving to `1.0`
needs an explicit product decision. A later authorized version-preparation change
updates `[workspace.package] version` in `Cargo.toml`, refreshes only workspace
versions with `cargo update --workspace --offline`, and folds pending formats with
`scripts/fold-format-pending.sh`. It goes through a checked PR, never `[skip ci]`.
This page does not authorize making those changes now.

Record the **final tested checkout SHA**, not just a branch, tag or version. The
release gate runs on the main-target PR merge SHA or main merge-group SHA. A
manual dispatch checks out the selected ref and creates the diagnostic
`release-gate-diagnostic` context, not the required `release-gate`; it cannot
satisfy main's required check. No workflow here changes branch protection/settings.

If promotion, conflict resolution or a **base-branch update** changes that merge
SHA, **rebuild all packages and rerun all required validation at the new merge
SHA**. The check run is attached to the PR head SHA, while the plan, receipts and
package descriptors bind the merge SHA. When the base advances, strict up-to-date
reports the PR as behind and blocks merge even if the old check still shows green;
this was verified on a temporary docs-only probe (PR #116, `mergeStateStatus:
BEHIND`). Update the candidate branch, which creates a new head; the old check no
longer covers that head. Draft→ready alone does **not** merge the new base, so it
does not clear the behind state. If the new base is already reachable from the
head, there is no new base content to merge; the head-bound check still covers the
same head tree. Do not relabel old inventories, receipts or artifacts, move a tag,
or claim that equal trees make different source revisions interchangeable.
Reconcile main back to develop with a checked merge commit after promotion; clean
up the temporary release branch only after both histories are preserved.

## Candidate preparation and explicit verification

The release gate is started by an explicit candidate transition, not by every
push. Keep the promotion PR draft while preparing. Its direct affected `ci` lane
still runs on preparation commits; the full `release-gate` does not start.

Mark the PR ready when the candidate is fixed. Opening an already-ready PR,
reopening it while ready, or `ready_for_review` starts the full gate on that
candidate. If a commit is added afterwards, the new SHA has no release-gate
result; the old result is not reused. Convert the PR back to draft and ready again
to verify the new SHA. A manual dispatch is diagnostic only: its
`release-gate-diagnostic` context cannot satisfy the required check.

A draft or intermediate run still reports the required `release-gate` name, but
the job fails with "release gate not started". It never reports success, so it
cannot satisfy the required context by accident. The later ready run for the same
SHA is the latest check and is the one the merge evaluates. A superseded
candidate's old check stays on its old SHA and cannot authorize the new one. Do
not cancel another task's running CI.

The full flow is: prepare with affected checks -> fix the candidate SHA -> start
the gate -> read the recorded failure and repair the smallest affected input ->
start the next full gate at the new SHA -> publish only after the required
`release-gate` succeeds. See [validation policy](validation-policy.md).

## What runs without publishing

- `release.yml` has reusable and manual build/check entries only. There is no
  schedule, push/release trigger, `draft`/`packages_only` publication switch,
  release/store job or signing-secret interface. No nightly helper is invoked.
  Its historical selection tests remain in `packaging/tests/nightly_test.py`.
- Linux x86_64/aarch64 packages include AppImage/zsync, deb, rpm and tar.gz;
  Flatpak, Arch and Snap reuse those Linux binaries. FreeBSD, universal macOS,
  Windows x64/x86/arm64 and web packages retain their build/package checks.
  Snap and Arch failures are no longer allowed errors, including prereleases.
- Packages are unsigned, except the macOS package script's existing ad-hoc local
  signature. There are no Apple, SignPath or update-signing requests.
- Package artifacts remain Actions artifacts, never downloads published to a
  release/store/registry. Each final package artifact also contains `build.json`:
  checkout `sourceRevision`, package identity, `target`, SHA-256 of the exact #5
  contract manifest bytes (`contractDigest`, `sha256:` plus 64 lowercase hex as
  frozen by #5/#15), and names/SHA-256 of built files.
  A separate `package-evidence-*` artifact carries the same descriptor. An absent
  contract yields null, **not** a claimed digest or release-ready artifact.
  Intermediate icons/macOS binaries use SHA-qualified artifact names and only
  same-run downloads from that exact checkout; no old-run artifact lookup exists.
  Descriptor production and release aggregation refuse modified tracked source.
- `build.json` is build metadata, **not #15's runtime inventory or verification**.
  In particular `universal-apple-darwin` names the combined macOS build; it cannot
  replace #15 inventories/installed evidence for both real Darwin targets.
- `package-builds` checks every real build result. A success there means only
  nonpublishing builds/checks succeeded, not runtime/installed release acceptance.

Do not start a network-enabled desktop to smoke-test a release build. The existing
CLI/version, archive, installer and icon checks remain. Flatpak keeps its actual
desktop modelling smoke **inside its existing network-denying sandbox**, with no
network override, and that smoke now also opens the Assistant and fails unless its
page reported in. Snap's desktop smoke is still not invoked: a strict snap uses the
host network, so the step cannot take the network away. The snap step instead checks
that the connected gnome-46-2404 content snap contains GTK 3 and WebKitGTK 4.1; the
Assistant's page there remains a hardware/manual check (`packaging/linux/snap-smoke.py`).
The CLI/package checks remain; the missing safe installed desktop/runtime acceptance is
explicitly blocking, not a skipped successful requirement. Other package desktop
`--version` calls exit before UI/updater startup; do not replace them with a normal app
launch.

### Installed checks of the deb, rpm, AppImage, disk image and MSI

The Assistant's interface is a web view, so the app needs the system's GTK 3 and
WebKitGTK 4.1 on Linux and uses the WebView2 runtime on Windows. A build tree, or
the machine that built a package, cannot show that a package brings what the app
needs. These steps install each package as a user would and open the Assistant in
the installed app, **with the network taken away from the app** in every case,
and with its update check off as well (`SOLIDCRAFT_UPDATE_CHECK=0`):

- **Linux** (`packaging/linux/installed-check.sh`): the deb on bare Ubuntu 22.04,
  Ubuntu 24.04 and Debian 13 containers and the rpm on a bare Fedora container,
  installed with nothing but what each declares, where the app's file must find
  every library it names. The AppImage brings no system libraries: on a bare
  system its launcher must name the missing ones, then they are installed. The
  package and the test's tools are installed in one container; the app is started
  only in a second one made from it with `--network none`, which first shows that
  it reaches nothing. One run is in a Wayland session (sway, with XWayland).
- **macOS**: the app is copied out of the disk image and started under
  `packaging/macos/no-network.sb`, a sandbox profile that leaves only this
  machine's own address; the step first shows that the profile stops `curl`. The
  runner's download carries no quarantine mark, so **Gatekeeper is not exercised**:
  an ad-hoc signature does not pass it on a user's Mac.
- **Windows** (`packaging/windows/installed-check.ps1`): the MSI is installed for
  the machine, the installed `intcad.exe` must be byte for byte the one the job
  built, the firewall blocks it and the WebView2 runtime's processes from reaching
  out (a copy of `curl.exe` under the same kind of rule shows the block holds),
  `tools/windows-assistant-smoke.ps1` runs against the installed app, and the MSI
  is removed again. The arm64 MSI is built but cannot run on the x64 runner. The
  32-bit MSI is installed, compared and removed, and its app is **not started**: on
  a runner the installed 32-bit app showed no window within three minutes, before
  the Assistant was opened (run 43112420636). No 32-bit build had been started in
  CI before, and the cause is not known; the step writes this into the job summary.

The page those apps show is itself a build output that is checked in
(`packages/assistant-web/dist`, so that the app builds without Node). The job
`Assistant page` of `packaging-lint.yml` builds it again from a fresh clone of the
pinned Mastra commit (`packages/assistant-web/tools/build.mjs --check`) and fails
unless the page, the list of what went into it (`inventory.json`) and its third-party
notices (`THIRD-PARTY-NOTICES.txt`) are what is checked in, byte for byte. The build
refuses any file under one of Mastra's enterprise-licensed `ee/` directories and any
npm package with an unreviewed licence. The deb, rpm, tarball, disk image, MSI and
portable zip carry the notices as `ASSISTANT-THIRD-PARTY-NOTICES.txt`, and the app
carries them built in (Help > About > Licences), which is what an AppImage or a
single downloaded executable has. That list and those notices are what a build tool
found and what the packages state about themselves, **not** a rights clearance.

`packaging/assistant-check.py` is the check itself on Linux and macOS: the panel
opens, the page loads and reports in, the app quits with exit code 0 and leaves no
crash report. Logs and pictures are uploaded as `installed-check-*` artifacts and
are **not** release evidence: #14's installed-runtime acceptance, signatures and
notarization stay open.

The deb and rpm declare the Assistant's GTK 3 and WebKitGTK 4.1 and their installed
checks open the page without a network. The Flatpak step opens the same page check
inside its sandbox (the GNOME 51 runtime supplies both libraries). The Snap stages
neither, because its core24 gnome extension (gnome-46-2404) already supplies both;
the release job verifies they are present in the mounted content snap, but does not
start the desktop. The Arch package is built by an external action whose fixed
`depends` list omits the two libraries, so the job adds `gtk3` and `webkit2gtk-4.1`
to `.PKGINFO` and fails unless both are there; it does not start the app. Not covered
by a run here: the tarball, Nix, an Assistant launch from Snap or Arch, a real GPU,
and any machine with a person in front of it. The deb, rpm, AppImage, macOS and
Flatpak steps above are the page runs in this workflow; Snap and Arch get
package/library checks only.

Docker keeps a local image build and HTTP smoke test (`push: false`). Pages keeps
the book/web build and offline browser test as an ordinary Actions artifact, with
no release-channel/Flatpak downloads or deployment. Docker/Pages/Flatpak/Snap/
WinGet publisher jobs have explicit false conditions and no publisher steps,
credentials or write permissions. Their skipped jobs are **not required checks**
and never contribute release evidence. Changing the default branch or manually
running these workflows cannot enable publication.

## Required context: `release-gate`

The top-level, always-evaluated job is named exactly `release-gate`. PRs targeting
main, main merge groups, and manual/reusable full validation select full native,
wasm, platform, heavy/property, package, contract/runtime/integration and provenance
requirements. No draft/approval label reduces those requirements. Develop merge
groups report explicit non-applicability for stable promotion; develop task PRs
continue to use their applicable `ci` gate, not a fake release success.

The aggregate checks actual reusable-workflow results, the immutable full
`releaseRequired` plan and its exact receipt digests via `scripts/ci/gate.py`,
and all 14 final package descriptors against the checkout SHA/target/contract.
The nested `ci` aggregate already checks its real selected jobs and platform/heavy
results. Missing/failed/cancelled/skipped required jobs, absent receipts,
incomplete/empty/skipped test identities, stale SHA/digests, or missing descriptors
fail. Raw unknown native failures remain blocking; the existing reviewed
known-failing grader is not a full-suite pass or publication approval.

An unapproved legacy workflow runtime still blocks the gate. A native release
must prove its contents are native. A release with the built-in agent must pass
#214's real per-package content checks once integrated. Those checks do not
replace legal review or installed-agent acceptance.

The existing kernel rig is a real required consumer, not a future integration
placeholder. Release calls explicitly force **every area and the ladder**, even
on draft/affected PR events. A source-bound plan selects exact existing case paths;
the existing Rust command-coverage, area known-failing and ladder-golden graders
run unchanged. Same-run SHA-qualified binary/results artifacts bind the checkout,
actual build/coverage/shard command exits, binary hashes and complete raw case
outcomes. The nested aggregate checks physical binary hashes and real job results;
the release aggregate rechecks its full raw handoff and requires that reusable
workflow's actual success. A constant pass flag, partial/empty/skipped report,
missing shard or wrong source/digest cannot stand in for this evidence.

Direct and reusable CI/rig calls have separate concurrency namespaces, including
the caller workflow name, while updates within each caller/event/ref still dedup.
The rig uses read-only GitHub-hosted Linux runners, bounded existing area shards
and build concurrency, software Vulkan and repository-scoped dependency caches
saved only by trusted main/develop pushes. These are execution requirements, not
proof of actual hosted capacity or native test success; measured runs are still
required before activation.

### Owner interfaces, not substitute implementations

#17 consumes #5's same-checkout `schema/mastra-runtime/v1/manifest.json` and bundle;
it does not create a competing schema. Verify actual registered Rust contract and
lifecycle selections before enabling owner filters. #7's real
`packages/mastra-runtime/` workspace must provide locked install, typecheck, test,
build, `verify:schema` and `check:deps` npm scripts; `scripts/ci/main_merge.py`
invokes those scripts directly from the committed root lockfile. There is no
`ci.sh` and no executable stub here.

Only #15 implements `cargo xtask runtime-artifact`. Its approved modes are:

```text
--mode policy
--mode host-only --tree . --tree-kind repository --target TARGET --expected-revision SHA --advertised false
--mode artifact --tree STAGING --inputs-root AUDITED_INPUTS --approval APPROVAL_ID --target TARGET --expected-revision SHA --expected-contract-digest CONTRACT_DIGEST --expected-policy-digest POLICY_DIGEST --expected-sources-digest SOURCES_DIGEST --advertised true
```

These are a deferred integration contract, **not commands available or run in this
branch**. Use fixed trusted-root policy/source paths and expected digests derived
from the immutable tested checkout, not producer-selected alternatives. For each
actual staged runtime/target, require successful verifier exit **and** its matching
`artifact`/`verified` report and source/target/contract/policy/source digests. Only
that owner verifier validates the staging/resource inventory. Policy success,
`policy_valid`, `host_only`/`not_applicable`, an arbitrary JSON claiming `verified`,
or a host-only package must never stand in. There is no `if exists` success path.

#14 must supply real installed-runtime tests and source/resource manifests tied to
the same final artifacts/targets/digest. This foundation neither stages a runtime
nor installs Node nor invents an installed-consumer entry point. Until these real
owner adapters land, explicit aggregate failures remain; an uploaded file alone
does not unlock them. A changed final SHA requires new inventories and rebuilds,
not only rerunning a policy check.

## Validation and later activation

Run packaging Python suites, selector/gate tests, actionlint and shellcheck for
workflow changes. Before any implementation commit, run the repository's full
`cargo xtask ci` gate under the documented bounded-resource policy. Focused lint
or unit-test success is not native/wasm/installed validation. Preserve raw full
baseline failures separately; do not add exclusions or a broad failure allowlist.

Real GitHub-hosted cache/runner capacity, image versions, memory/disk/time and
actual per-target installed evidence need later measured runs. Local YAML/lint
success certifies none of those. Settings activation, real checks/read-back and
publication destination/signing approval are separate owner decisions; #17 stays
open for missing runtime and installed-release acceptance. No dispatch, settings,
credential or publishing action is requested by this document.

## macOS disk image implementation

`packaging/macos/package.sh` builds the `.dmg` with hash-pinned dmgbuild in a
throwaway `target/` venv, without Finder/AppleScript. The artwork/layout remains
in `assets/platforms/macos/dmg/`; this foundation changes none of it. The script
mounts the finished image and checks its layout, volume icon, bundle and local
signature, then checks the CLI zip. A future signed/notarized distribution needs
its own approval and real platform verification; an ad-hoc signature is not one.

## Distribution repository

For v0.1.3, build, gate and stage from the existing public
[`intgral-ai/intcad`](https://github.com/intgral-ai/intcad) repository (ID `1446384930`)
on standard GitHub-hosted runners. Do not migrate until the public release has been
verified and the CTO separately approves the migration. After migration, the same
workflow can run from the independent private `intgral-ai/intcad-client` repository
only after the owner configures its immutable repository ID and private mode. It
rejects a private repository that is a fork, a mismatched ID or slug, and a change
in visibility. The public release source is itself a fork of SolidCraft, and it
stays one for v0.1.3.

The public **`intgral-ai/intcad-releases`** repository holds installers, release notes,
checksums and signatures; it does not hold the source or build configuration. The
in-app updater and AppImage zsync feed use it. User-facing documentation and
screenshot links stay in the public source repository, which remains readable after
archiving. The draft workflow does not deploy a guide. A separate guide site is a
future task.

### Staging a signed draft

1. Start the formal **Release gate** for a fixed stable candidate. The gate only
   records `stable-release-handoff-a<attempt>` after its checks pass for a ready,
   same-repository `release/vX.Y.Z` PR to `main` or a stable main merge group.
   The handoff records the actual `GITHUB_SHA`, run and attempt, plus the names,
   IDs, archive digests and descriptor digests of all 14 package/evidence pairs.
   Standalone builds, diagnostic dispatches, Beta candidates and old attempts do
   not qualify. If a rerun only repeats some jobs, start a fresh formal gate;
   a previous attempt cannot supply the missing pairs.
2. For a release carrying #214's built-in agent, review the actual npm/Node and
   file rights and the installed-agent results for the exact candidate. The
   owner records each accepted review in `RELEASE_AGENT_LEGAL_APPROVED` and
   `RELEASE_AGENT_INSTALLED_APPROVED`, as `<source SHA>:<run ID>:<attempt>`.
   A page smoke, a content scan or a successful build is not installed-agent
   acceptance or rights clearance.
3. After the owner authorizes staging, dispatch **Stage signed release draft**
   from protected `main`. Supply `tag`, `source_sha`, `run_id`, `run_attempt` and
   `notes_file`. The notes must be `docs/release-notes-<version>.md` (or a file
   under `docs/release-notes/`) at that source SHA; any other tracked Markdown is
   refused, so internal documentation cannot become the update text.
   A reviewer must approve the `release-signing` job before it receives signing
   credentials. The source tag must resolve to the exact tested commit; the PR
   head and a different commit with the same tree are not substitutes.
4. The workflow reads back the live gate run and exact package jobs. It verifies
   all package/evidence pairs and package bytes against the gate handoff before
   flattening them. It writes the old and new checksum names with identical
   bytes, requires all updater aliases and signs `SHA256SUMS.txt`. It refuses an
   existing draft, release or tag.
5. The workflow creates **only a draft**. It rejects an absent API digest, then
   downloads every uploaded asset back and compares its bytes with the local file,
   and verifies both checksum lists and the signature. Leave any failed read-back draft unpublished.
   Making a verified draft public is a separate owner decision and action.

The owner configures `release-signing` with required reviewers, `can_admins_bypass`
off (an explicit `true` is refused; an absent field is accepted with a warning
because some plans omit it), and a deployment branch policy that permits only
`main` (no tags).

A project with a single person cannot both dispatch and be forbidden to approve
its own deployment. The default stays strict: with fewer than two reviewers, or
with self-review allowed, admission fails unless the repository variable
`RELEASE_SINGLE_OWNER=true` is set by the owner. An explicit `false` or an unset
variable keeps the strict rule. When the exception applies, the workflow warns,
and the verified draft's summary records `Approval: single-owner`, the enabling
variable, the reviewer account and the dispatch actor, so the exception and who
enabled it are auditable. With two or more reviewers, self-review must still be
prevented. The workflow checks that `main` is protected and reads back the
reviewer and branch policy before the sensitive job. For the public source, use
`RELEASE_SOURCE_MODE=public` (the default) and either leave
`RELEASE_SOURCE_REPO_ID` unset or set it to `1446384930`. Configure `private`
and the new repository's immutable ID only after migration approval and after
confirming that the private repository's plan supports required reviewers.
Do not relax approval requirements if the plan lacks them.

Store `RELEASE_APP_ID`, `RELEASE_APP_PRIVATE_KEY`, `MINISIGN_SECRET_KEY` and, if
needed, `MINISIGN_PASSWORD` only in `release-signing`. Install the GitHub App on
`intgral-ai/intcad-releases` only, with Contents read/write. Do not give normal
package builds or the release gate signing or publication credentials.

### Update signing

The client verifies a minisign signature over `SHA256SUMS.txt`. The trusted comment
must be exactly `Intcad v<tag version>` (for example, `Intcad v0.1.3`). The client
refuses a missing signature or a signature for another version or key. There is
no unsigned fallback.

The client and draft workflow use the same public key file,
`crates/ui-egui/src/update/trusted-release-key.pub`. Without a production key, the
client refuses to install **any** update and the draft workflow refuses to sign.
This pipeline cannot produce a v0.1.3 draft without the key. A no-key release can
only be published outside this workflow after equivalent review, with the in-app
update feature disabled; that is a product decision, not an unsigned fallback.

Secret-key custody is owner-only. Never give the secret key to an agent or put
it in a repository file, CI log or chat:

1. On an offline machine, generate the key: `minisign -G -p intcad.pub -s intcad.key` (add a
   passphrase unless you keep the key in a hardware-backed store).
2. Commit only the base64 public-key line from `intcad.pub` to
   `crates/ui-egui/src/update/trusted-release-key.pub` before running the gate.
   Verify that the installed candidate accepts the production signature and
   refuses a missing or invalid signature.
3. Store `intcad.key` in the `release-signing` environment secret `MINISIGN_SECRET_KEY`, and keep
   one offline backup outside GitHub. Record the public key and the date in the release notes.
4. Rotation: generate a new pair and distribute a client that trusts the new key
   before switching signing keys. This client does not support signing one release
   with two keys. For revocation, remove the secret and treat the old key as
   compromised. Never add an unsigned fallback.
5. Tests use their own key pair (`install_tests.rs`); the production key never enters a test.
