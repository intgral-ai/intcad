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
3. Download only the assets the Beta needs:

   ```sh
   gh run download <run-id> -n release-linux-x86_64 -D dist/linux
   gh run download <run-id> -n release-macos -D dist/macos
   gh run download <run-id> -n release-windows-x64 -D dist/windows-x64
   ```
4. Check the actual package content and install shape:

   ```sh
   tar -tzf dist/linux/intcad-<version>-linux-x86_64.tar.gz | head
   hdiutil attach -readonly -nobrowse dist/macos/intcad-<version>-macos-universal.dmg
   # Windows x64: msiexec /i <msi> /qn, then check C:\Program Files\Intcad
   ```
   The `package-evidence-*` artifacts contain `build.json` and `contents.json`; the latter is the
   runtime-marker scan used by the release gate.
5. If an asset is attached to a prerelease, first write the unversioned aliases the in-app update
   check resolves by name (`packaging/aliases.sh dist <version>`: AppImage, deb, rpm, Flatpak, Snap,
   MSI, portable zip, DMG). Without them those installations find no package and never update; the
   tarball and the standalone exe keep their version because the updater asks for them by it. Then
   make the checksum list with
   `packaging/checksums.sh dist --require <name>` once per asset the channel promises — the
   versioned files **and** the unversioned aliases clients resolve by name, across every platform.
   The list must cover the whole release, not one platform: an asset attached without its line
   leaves those users unable to update. The script refuses a list that omits a `--require`d name
   and verifies every hash before writing. Attach both `SHA256SUMS.txt` (the name the in-app update
   check reads) and `SHA256SUMS`, then update the release notes with the exact platform, signature
   state, verification scope and remaining gaps. Re-download and check the hashes. The script needs
   Bash and GNU `sha256sum --strict`: run it on Linux (the release runner or a container), not on a
   stock macOS host.

## The complete asset set for one version

`release.yml` produces these per version (`<version>` without the `v`); `packaging/aliases.sh`
adds the second column because the in-app update check resolves those names without a version.

| Release workflow artifact name | Name the release must also carry |
|---|---|
| `intcad-<version>-linux-x86_64.tar.gz` | — (the updater asks for this name with the version) |
| `intcad-<version>-linux-x86_64.AppImage` (+ `.zsync`) | `intcad-linux-x86_64.AppImage` |
| `intcad-<version>-linux-x86_64.deb` | `intcad-linux-x86_64.deb` |
| `intcad-<version>-linux-x86_64.rpm` | `intcad-linux-x86_64.rpm` |
| `intcad-<version>-linux-x86_64.flatpak` | `intcad-linux-x86_64.flatpak` |
| `intcad-<version>-linux-x86_64.snap` | `intcad-linux-x86_64.snap` |
| `intcad-<version>-windows-x64.msi` | `intcad-windows-x64.msi` |
| `intcad-<version>-windows-x64-portable.zip` | `intcad-windows-x64-portable.zip` |
| `Intcad-<version>-windows-x64.exe` | — (the updater asks for this name with the version) |
| `intcad-<version>-macos-universal.dmg` | `intcad-macos-universal.dmg` |
| `intcad-cli-<version>-macos-universal.zip` | — |
| `SHA256SUMS.txt` and `SHA256SUMS` | the same bytes under both names |

For 0.1.3 that is the set the candidate run produced plus the eight aliases; `checksums.sh
--require` for the three installers is the check that the list, not just the directory, is whole.

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
