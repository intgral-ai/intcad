# Building a Beta without the production release gate

The `Release` workflow (`.github/workflows/release.yml`) is the nonpublishing package builder. It
is independent of `release-gate.yml` and the `main` promotion. Use it for a Beta when the full
kernel/release gate is not green and the Beta scope is explicitly narrower.

This page does **not** authorize publishing, signing, a `main` merge, or a stable tag. It records
the build entry only. The per-candidate run IDs, install checks, content evidence and remaining
gaps belong in [docs/beta-evidence.md](beta-evidence.md); do not carry an older Beta result to a
new SHA.

## Entry

1. Choose the exact reviewed commit or the existing Beta tag. Do not move an existing tag or replace
   an asset with different bytes under the same version.
2. Run the `Release` workflow on that ref (Actions → Release → Run workflow), or:

   ```sh
   gh workflow run release.yml -R intgral-ai/open-skyfall-desktop --ref <tag-or-branch>
   ```

   The workflow builds Linux x86_64/aarch64, Flatpak, Arch, Snap, FreeBSD, macOS universal, Windows
   x64/x86/arm64 and web. It publishes nothing.
3. Download the assets into **one flat folder** (the publication scripts read one folder; the
   evidence artifacts go elsewhere):

   ```sh
   gh run download <run-id> -n release-linux-x86_64 -D dist
   gh run download <run-id> -n release-linux-aarch64 -D dist
   gh run download <run-id> -n release-macos -D dist
   gh run download <run-id> -n release-windows-x64 -D dist
   gh run download <run-id> -n release-windows-x86 -D dist
   gh run download <run-id> -n release-windows-arm64 -D dist
   gh run download <run-id> -n release-flatpak-x86_64 -D dist
   gh run download <run-id> -n release-snap-x86_64 -D dist
   gh run download <run-id> -n package-evidence-release-linux-x86_64 -D evidence
   ```
   `-n` with `-D` puts the artifact's own files directly in that folder, so `dist` stays flat;
   `checksums.sh` refuses a folder of folders rather than silently listing none of the files inside.
   The evidence artifacts are named `package-evidence-release-<package id>`, not
   `package-evidence-<platform>`, and contain `build.json` and `contents.json`; the latter is the
   runtime-marker scan used by the release gate.
4. Check the actual package content and install shape:

   ```sh
   tar -tzf dist/intcad-<version>-linux-x86_64.tar.gz | head
   hdiutil attach -readonly -nobrowse dist/intcad-<version>-macos-universal.dmg
   # Windows x64: msiexec /i <msi> /qn, then check C:\Program Files\Intcad
   ```
5. Before publishing, write the unversioned aliases the in-app update check resolves by name
   (`packaging/aliases.sh dist <version>`: AppImage, deb, rpm, Flatpak, Snap, MSI, portable zip,
   DMG). Without them those installations find no package and never update; the tarball and the
   standalone `.exe` keep their version because the updater asks for them by it. Then make the
   checksum list with `packaging/checksums.sh dist --require <name>` once per asset the channel
   publishes — the versioned files **and** the unversioned aliases clients resolve by name, across
   every platform. The list must cover the whole release, not one platform: an asset attached
   without its line leaves those users unable to update. The script refuses a subfolder, a symlink
   or a `--require`d name it cannot find, and verifies every hash before it writes; both names are
   written only after that. Attach both `SHA256SUMS.txt` (the name the in-app update check reads)
   and `SHA256SUMS`, then update the release notes with the exact platform, signature state,
   verification scope and remaining gaps. Re-download and check the hashes. The script needs Bash
   and GNU `sha256sum --strict`: run it on Linux (the release runner or a container), not on a
   stock macOS host.

   Two limits that no assembly step removes:

   - **The updater reads `releases/latest` and refuses a draft or prerelease.** Aliases and the
     checksum list on a prerelease are preparation for the real release: clients only find them
     once that version is the newest published, non-prerelease release.
   - **Flatpak and a store-installed Snap do not update themselves.** They are updated by `flatpak
     update ai.intgral.intcad` / `sudo snap refresh intcad`; a missing alias or checksum line does
     not block them, and publishing the files is still what makes the newer version available.

## The complete asset set for one version

`<version>` is without the `v`. `release.yml` makes the first column per architecture; `packaging/aliases.sh` adds the second
column, because the in-app update check resolves those names without a version
(`crates/ui-egui/src/update.rs`, `asset_name`). Neither checksum list is made by `release.yml`:
both are written by the publish step with `packaging/checksums.sh`.

| `release.yml` artifact | Files in it | Name the release must also carry |
|---|---|---|
| `release-linux-x86_64` | `intcad-<version>-linux-x86_64.tar.gz` | — (the updater asks for this name with the version) |
| | `intcad-<version>-linux-x86_64.AppImage` + `.zsync`, and the same `.zsync` as `solidcraft-…` and `solvecraft-…` | `intcad-linux-x86_64.AppImage` |
| | `intcad-<version>-linux-x86_64.deb` | `intcad-linux-x86_64.deb` |
| | `intcad-<version>-linux-x86_64.rpm` | `intcad-linux-x86_64.rpm` |
| `release-linux-aarch64` | the same four for `aarch64` | the same names with `aarch64` |
| `release-flatpak-x86_64`, `release-flatpak-aarch64` | `intcad-<version>-linux-<arch>.flatpak` | `intcad-linux-<arch>.flatpak` (a Flatpak updates through `flatpak update`, not this file) |
| `release-snap-x86_64`, `release-snap-aarch64` | `intcad-<version>-linux-<arch>.snap` | `intcad-linux-<arch>.snap` (a store Snap updates through `snap refresh`) |
| `release-windows-x64` | `intcad-<version>-windows-x64.msi` | `intcad-windows-x64.msi` |
| | `intcad-<version>-windows-x64-portable.zip` | `intcad-windows-x64-portable.zip` |
| | `Intcad-<version>-windows-x64.exe` | — (the updater asks for this name with the version) |
| `release-windows-x86`, `release-windows-arm64` | MSI and portable zip for `x86` / `arm64` | `intcad-windows-x86.msi`, `intcad-windows-arm64.msi`, and the two portable zips |
| `release-macos` | `intcad-<version>-macos-universal.dmg` | `intcad-macos-universal.dmg` (`asset_name` returns this name for the macOS app, so the unversioned file is what clients fetch) |
| | `intcad-cli-<version>-macos-universal.zip` | — (the updater never asks for it) |
| `release-arch-x86_64`, `release-arch-aarch64`, `release-freebsd-x86_64`, `release-web` | `.pkg.tar.zst`, FreeBSD tarball, web bundle | — (no updater path asks for these by name) |
| publish step | `SHA256SUMS.txt` and `SHA256SUMS` | the same bytes under both names, written by `checksums.sh` |

For the 0.1.3 candidate, `release-linux-x86_64`, `release-windows-x64` and `release-macos` were
downloaded, which yields **six** aliases (AppImage, deb, rpm, MSI, portable zip, DMG) and an
18-line list: the 16 files above plus the two legacy `.zsync` copies. Flatpak, Snap, aarch64 and
the Windows x86/arm64 assets come from their own artifacts and were **not** in that downloaded
set — `checksums.sh --require` for the tarball, the MSI and the DMG proves those three are listed,
not that the whole published set is.

## What this entry does not prove

- It is not a `main` release candidate.
- It does not satisfy `release-gate` (full CI, kernel-rig, all package descriptors and the native
  content decision). A successful `release.yml` run alone is package-build evidence, not gate
  evidence.
- It does not sign or notarize anything.
- A real-provider or clean-machine result belongs to the exact commit that produced the asset; it
  cannot be carried to a different candidate without re-running. The published `v0.1.0-beta.1`
  assets are not evidence for a new candidate.

See `docs/releasing.md` for the production path and `docs/action-matrix.md` for the kernel ratchet.
