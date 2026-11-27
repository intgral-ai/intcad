# Downloads

Intcad installers are distributed through [GitHub Releases][releases]. Once published, they
require no GitHub account or access to the source repository. The release and download links
can return **404** until the approved release is public. If that happens, wait for the official
Intcad announcement rather than downloading from a mirror.

## Intcad v0.1.3

- **Windows:** [x64 installer][windows-x64-msi] or [ARM64 installer][windows-arm64-msi]. Portable
  ZIPs are listed on the [v0.1.3 release page][v013].
- **macOS:** [universal disk image][macos-dmg] for Apple silicon and Intel. Open it and drag
  `Intcad.app` into Applications.
- **Linux:** [x86_64 AppImage][linux-x64-appimage] or [ARM64 AppImage][linux-arm64-appimage].
  Look under **Assets** on the [v0.1.3 release page][v013] for `.tar.gz` archives and any `.deb`,
  `.rpm`, `.flatpak`, or `.snap` packages for your architecture.

## Verify a download

Download your package, `SHA256SUMS.txt`, and `SHA256SUMS.txt.minisig` from the same release. Get
the published minisign public key and confirm it separately through a trusted official Intcad
channel. Do not trust a key just because it is listed next to the download.

Set `INTCAD_MINISIGN_PUBLIC_KEY` to the confirmed base64 public key. In the download folder,
verify the signature before checking the file:

```sh
minisign -Vm SHA256SUMS.txt -x SHA256SUMS.txt.minisig -P "$INTCAD_MINISIGN_PUBLIC_KEY" &&
  sha256sum --check --ignore-missing SHA256SUMS.txt
```

The verified signature's trusted comment should read `Intcad v0.1.3` for this release, and the
exact filename you downloaded must report `OK`. If `sha256sum` is unavailable, compute the file's
SHA-256 with `shasum -a 256` on macOS or `Get-FileHash -Algorithm SHA256` in PowerShell and
compare it with the entry for that filename in the verified list. Do not install if any check
fails or you cannot confirm the key.

## Upgrading

**The first upgrade from v0.1.0–v0.1.2 to v0.1.3 is manual.** Save and back up your designs, quit
Intcad, then download and verify the v0.1.3 package that matches your current installation.
Run the installer or replace the app, AppImage, or extracted program files as appropriate.
Do not start from the old app's update window. The migration is intended to leave user data
in place, but an end-to-end upgrade of the published v0.1.2 installers, including retention
of settings and sign-in, has not been verified.

Starting with v0.1.3, **Help ▸ Check for updates** uses the new public feed on supported
installations. Package-manager and launcher-managed installations use their own update
mechanisms.

[releases]: https://github.com/intgral-ai/intcad-releases/releases
[v013]: https://github.com/intgral-ai/intcad-releases/releases/tag/v0.1.3
[windows-x64-msi]: https://github.com/intgral-ai/intcad-releases/releases/download/v0.1.3/intcad-windows-x64.msi
[windows-arm64-msi]: https://github.com/intgral-ai/intcad-releases/releases/download/v0.1.3/intcad-windows-arm64.msi
[macos-dmg]: https://github.com/intgral-ai/intcad-releases/releases/download/v0.1.3/intcad-macos-universal.dmg
[linux-x64-appimage]: https://github.com/intgral-ai/intcad-releases/releases/download/v0.1.3/intcad-linux-x86_64.AppImage
[linux-arm64-appimage]: https://github.com/intgral-ai/intcad-releases/releases/download/v0.1.3/intcad-linux-aarch64.AppImage
