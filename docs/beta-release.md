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
   tar -tzf dist/linux/solidcraft-<version>-linux-x86_64.tar.gz | head
   hdiutil attach -readonly -nobrowse dist/macos/solidcraft-<version>-macos-universal.dmg
   # Windows x64: msiexec /i <msi> /qn, then check C:\Program Files\SolidCraft
   ```
   The `package-evidence-*` artifacts contain `build.json` and `contents.json`; the latter is the
   runtime-marker scan used by the release gate.
5. If an asset is attached to a prerelease, update `SHA256SUMS` and the release notes with the exact
   platform, signature state, verification scope and remaining gaps. Re-download and check the
   hashes.

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
