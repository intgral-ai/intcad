---
name: solidcraft-packaging
description: "Use when preparing or shipping a SolidCraft release, changing installers or packages (AppImage, MSI, dmg, Flatpak, Snap, WinGet, Nix, FreeBSD, web), release asset names, signing or the Pages deployment."
---

# Packaging and releases

Development pushes are not application releases; pushing to `main` or a tag starts nothing. Follow `docs/releasing.md` when asked to prepare or ship a release.

## desktop-only Mastra distribution exception
The desktop TS companion (`packages/mastra-runtime/`) follows `docs/mastra-distribution.md` and `skills/solidcraft-ground-rules/SKILL.md`; host/core stay Rust/wasm, web has no Node/runtime.

A release carries the built-in agent (`SOLIDCRAFT_WITH_MASTRA_AGENT=1`, `SOLIDCRAFT_DISTRIBUTION=mastra-agent`), built per runner from the pinned lock and Node, proved from installed bytes (`docs/mastra-packaging.md`). Legacy `mastra/` stays `=0`.

Cargo is not npm/Node or file-rights clearance; #7 source/build audits and #14 installed/signature checks stay required, with `artifact`/`verified` + JSON status, never policy or host-only.

## Preparing
1. One version for the whole batch, from the actual changes (patch, minor, major, `-rc.N`), not from commit prefixes. Moving to `1.0` is the owner's decision.
2. Update `[workspace.package] version` in `Cargo.toml`, then `cargo update --workspace --offline`; only our own crates' `version =` lines in `Cargo.lock` change.
3. Fold the pending design-format changes with `scripts/fold-format-pending.sh` (`skills/solidcraft-file-formats/SKILL.md`).
4. Run `cargo xtask ci` and the checks in `.github/workflows/packaging-lint.yml`. Land the version commit on `main` through the normal review flow; never `[skip ci]`.

## Building and publishing
- Advance `release` with fast-forwards only (`git merge --ff-only`); never force-push. `.github/workflows/release.yml` builds every package and creates a draft GitHub Release.
- Verify the draft before publishing: the run succeeded for the intended SHA, every platform and architecture is present, `sha256sum --check SHA256SUMS.txt` passes, and the CLI runs (`--version`, an example model).
- Only when shipping is authorised, publish the verified draft with `gh release edit`, then confirm its tag resolves to the recorded SHA and the deployment ran. A request to prepare a release is not a request to publish.
- Never overwrite a published version, tag or asset: use a new version. Never push the version tag. Packages are unsigned unless `docs/releasing.md` says otherwise; do not claim they are signed.
- Nightlies are prereleases from the same workflow; they never become latest or advance `release`.

## Names that must stay stable
- Assets are `intcad-<version>-...`, also attached unversioned (`intcad-windows-x64.msi`). The README links and the in-app update check (`crates/ui-egui/src/update.rs`) pick them by name; renaming one breaks both.
- The old product name survives only in the shims in `docs/compatibility.md`. Do not rewrite published assets, checksums, tags or historical notes.
- Documentation, the browser app and the Flatpak repository publish under project Pages `/solidcraft/`. No old-name Pages aliases.

## Where things live
- `packaging/` holds the per-platform scripts (`linux`, `windows`, `macos`, `freebsd`, `web`); `packaging/tests` checks them.
- Nix `flake.nix` + `docs/nix.md`; building `docs/building.md`; downloads `docs/downloads.md`.
- Build with `CARGO_TARGET_DIR=<repo>/target`. Windows from Linux: `cargo xwin build --release --target x86_64-pc-windows-msvc -p solidcraft`.
