# Downloads

Every platform and package, from the public distribution repository
[`intgral-ai/intcad-releases`](https://github.com/intgral-ai/intcad-releases/releases). You do not
need a GitHub account or access to the source repository to download; the files are public.

- **Windows:** download and run the [installer][windows-msi]. Or take the
  [portable zip][windows-portable] that runs without installing.
- **macOS:** open the [disk image][macos-dmg] and drag Intcad into Applications (Apple silicon and
  Intel).
- **Linux:** [AppImage][linux-appimage] or the [`.tar.gz`][linux-tar]; the `.deb`, `.rpm`,
  `.flatpak` and `.snap` files are attached when a release builds them.

The archive and the standalone `.exe` carry their version in the file name
(`intcad-<version>-linux-x86_64.tar.gz`, `Intcad-<version>-windows-x64.exe`); the installer, disk
image, AppImage and package files are also published without one, which is the name the in-app
update check looks for.

Each release carries `SHA256SUMS.txt`, the same list under the earlier name `SHA256SUMS`, and
`SHA256SUMS.txt.minisig`, a minisign signature over the list. Check a download with:

```sh
sha256sum --check --ignore-missing SHA256SUMS.txt
minisign -Vm SHA256SUMS.txt -x SHA256SUMS.txt.minisig
```

`minisign -V` needs the public key, which is built into the client and printed in the release
notes.

## Upgrading

Intcad 0.1.3 and later update themselves (**Help ▸ Check for updates**). 0.1.0 to 0.1.2 read
updates from a repository that is no longer public: download once from the links above and
install over the old copy. Designs, preferences and the signed-in session are kept.

Older tags, checksums and release notes are on the
[releases page](https://github.com/intgral-ai/intcad-releases/releases).

[windows-msi]: https://github.com/intgral-ai/intcad-releases/releases/latest/download/intcad-windows-x64.msi
[windows-portable]: https://github.com/intgral-ai/intcad-releases/releases/latest/download/intcad-windows-x64-portable.zip
[macos-dmg]: https://github.com/intgral-ai/intcad-releases/releases/latest/download/intcad-macos-universal.dmg
[linux-appimage]: https://github.com/intgral-ai/intcad-releases/releases/latest/download/intcad-linux-x86_64.AppImage
[linux-tar]: https://github.com/intgral-ai/intcad-releases/releases
