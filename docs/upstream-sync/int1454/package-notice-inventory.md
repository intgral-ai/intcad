# Historical package notice inventory

Read-only inspection of existing IntCAD artifacts. Presence is not a
rights conclusion. No installation, build, CI dispatch, release or
publication was performed. No artifact below is final landed-SHA evidence.

| Artifact | Product source and boundary | SHA-256 | Actual package contents |
| --- | --- | --- | --- |
| `Intcad-f82b56372d41.msi` | `f82b56372d41260f1505b98aa50b009d0c35029b`, clean local Windows GUI-only smoke. The MSI digest matches its sidecar and the historical receipt; the embedded desktop executable digest matches the build manifest. | `b1d55038d01c9452f4b534bfe93e8d365c0f08c2875518395aaccaf84890be4a` | `cab1.cab` carries `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, `THIRD-PARTY-LICENSES.txt`, `ASSISTANT-THIRD-PARTY-NOTICES.txt` and `WEB-VIEW-PLATFORM-NOTICES.txt`, mapped from cabinet IDs through pinned `assets/platforms/windows/solidcraft.wxs`. All six compare byte-for-byte with their pinned source files. `cab2.cab` contains only the desktop executable. No separate vendored `solidcore/{LICENSE,NOTICE,ATTRIBUTION.md}` or Mastra-runner notices among the MSI payload files. |
| Linux x86_64 `intcad-0.1.2-linux-x86_64.tar.gz` | PR #178 nonpublishing test merge `760f000f49af13fa89599792b36e6410a251eae7`. The retained tarball matches the entry in artifact `14620548705`. | `b1ab0547d3e3395b3a0b5a69331e92937e0cc1f2f0ed2787bcab1b52f3674537` | The eight product notice files listed below exist under the package's `share/doc/intcad/`; the runtime also carries its two notice files. |
| Linux x86_64 `intcad-0.1.2-linux-x86_64.deb` | Same PR #178 test merge and artifact. | `af28a3eb084a04ed6f255c844a987fa0986aa32d5f2f4c461e62caac9ce3688d` | The same eight product files exist under `usr/share/doc/intcad/` in the data archive; the runtime also carries its two notice files. |
| Linux x86_64 `intcad-0.1.2-linux-x86_64.rpm` | Same PR #178 test merge and artifact. | `440e9adbabaae84b5cedd975bb822c983b763504ef27a83646f5ff06c73f705a` | The RPM file table lists the same eight files under `/usr/share/doc/intcad/` and the two runtime files. This is a manifest inspection, not an installation or signature check. |
| Linux x86_64 `intcad-0.1.2-linux-x86_64.AppImage` | Same PR #178 test merge and artifact. | `f05bf83d6700aecb1ddcb2af13c337431ea9b3a47a9e20990798ce49acb7b9ea` | The embedded SquashFS lists the two runtime files under `usr/lib/intcad/mastra-agent/licenses/`. None of the eight product notice filenames was found outside `node_modules/`, except the runtime file named `THIRD-PARTY-LICENSES.txt`. See the embedded-text follow-up below. |
| `intcad-0.1.2-macos-aarch64.dmg` | Same PR #178 test merge, [macOS job](https://github.com/intgral-ai/intcad/actions/runs/44715398096/job/136217397497), artifact `14622163271`. | `adab62b9d83214f51c50d95c31792a9789fae760d23a56b79f3e069d64410ab0` | The HFS listing shows all eight files in `Intcad.app/Contents/Resources/Licenses/` plus its README, and the two runtime notices in `Contents/Resources/mastra-agent/licenses/`. |
| `intcad-0.1.2-windows-x64-portable.zip` | Same PR #178 test merge, [Windows job](https://github.com/intgral-ai/intcad/actions/runs/44715398096/job/136217397922), artifact `14622644962`. | `20800f4f27b07ff8fd8521e412324420b90dd6f7d473472de096e808721e47ca` | The ZIP lists the first six product notice files below at its package root, and the two runtime notices under `mastra-agent/licenses/`. |
| `intcad-0.1.2-windows-x64.msi` | Same PR #178 test merge and Windows artifact. | `136700cc64d8dd5e1263d0ef31f564442882428615086c972089ba455d4bad04` | `cab1.cab` contains the first six product notices; extracted texts match pinned sources after CRLF normalization. 7-Zip reported a compound-container end-of-archive error while retrieving the cabinets, so this verifies those six streams only, not the complete MSI payload or runtime notice layout. |

**Product notice list (tar/deb/RPM/DMG):** `LICENSE-MIT`, `LICENSE-APACHE`,
`NOTICE`, `THIRD-PARTY-LICENSES.txt`, `ASSISTANT-THIRD-PARTY-NOTICES.txt`,
`WEB-VIEW-PLATFORM-NOTICES.txt`, `MASTRA-AGENT-NODE-LICENSE.txt`, and
`MASTRA-AGENT-THIRD-PARTY-LICENSES.txt`. The separate Mastra runtime
directory lists `NODE-LICENSE.txt` and `THIRD-PARTY-LICENSES.txt`; this is
not a per-module licence audit. The local Linux ZIP digest
`74effe27702165b697a622dd32f51c3153a0d8c73c07f4f9b21052d6b2cae721`
matches [artifact 14620548705](https://api.github.com/repos/intgral-ai/intcad/actions/artifacts/14620548705).
The [job log](https://github.com/intgral-ai/intcad/actions/runs/44715398096/job/136217397869)
records the checked-out test-merge SHA and artifact ID. The run's API
`head_sha` is the PR head `cf28eaecb873f241cc6dacefd488de685cb4592b`, not
the package build tree. That historical run does not cover INT-1454's
final package gate.

The macOS and Windows artifact ZIP SHA-256 values also matched GitHub's
published artifact digests (`96cd9497e0c3fa636c6785c61a5890848362ca1a43068cec5c006f214341ba00`
and `84f813ff378121a27cab0db3df63404dba6651173225fca0f2d29a35d7ab8a1d`).
Both job checkout logs identify the same test-merge SHA as the Linux job.

## Not verified as packages

- [Packaging lint 44646584340](https://github.com/intgral-ai/intcad/actions/runs/44646584340),
  source `6f624aa55d0945581a2002b8c5f2b560e21b53ec`, retained no
  artifacts. Assistant notice rebuilding is not package-payload evidence.
- [Windows A6 44893965299](https://github.com/intgral-ai/intcad/actions/runs/44893965299),
  product `581e606f0e30f3404b1bc9cd786a44bc015537ea`,
  workflow `9641436fb7bf284fae32582f07af26d3102ccf58`, uploaded
  [evidence ZIP 14704593233](https://api.github.com/repos/intgral-ai/intcad/actions/artifacts/14704593233)
  (`4a219a27ef7b129556aa317db7d377d95e8db199ddcc153128a999d00daccc69`),
  containing logs and a reported executable hash, not an installer or
  package notices. The isolated Linux A6 binary is also not a package.
- No SHA-bound Flatpak package or final-`develop` installer was located
  in the inspected INT-1454 receipts. Formats outside the listed
  artifacts were not verified.

Local manifests, metadata, checkout logs, and PR #178 macOS/Windows
artifact ZIPs: `/tmp/opencode/int1454-package-inventory.r23tiI/`.
The Linux `artifact.zip` and `intcad.tar.gz` are retained in
`~/.local/state/intcad-e2e/pr178-mastra-ln-artifact/`. The GUI-only MSI,
its `.sha256` sidecar, and `artifact.json` are retained in
`~/.local/state/intcad-e2e/windows/builds/int1454-ui-c28767d6c085/f82b56372d41-e3b0c44298fc/`.
Package SHA-256 values were computed from bytes, not inferred from filenames.
MSI cabinet IDs came from `7z`/`cabextract`; the Linux inspections used
`ar`/`tar`, `rpm -qpl`, and `7z -tsquashfs` without executing the AppImage.
The older GUI-only MSI and PR #178 test packages do not verify final
package notice completeness.

## AppImage follow-up: embedded texts

For the same SHA-bound PR #178 AppImage, rehashed the original artifact
ZIP and AppImage against the digests above, then extracted only
`usr/bin/intcad` and `usr/bin/intcad-cli` with `7z -tsquashfs` without
running either program. Searched each binary for the **complete
byte-for-byte Git blobs** at `760f000f49af13fa89599792b36e6410a251eae7`:

| Embedded text | Desktop `intcad` | `intcad-cli` |
| --- | --- | --- |
| `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, `THIRD-PARTY-LICENSES.txt` | 4/4 exact matches | 4/4 exact matches |
| `packages/assistant-web/THIRD-PARTY-NOTICES.txt`, `packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt`, `ATTRIBUTION.md` | 3/3 exact matches | No exact full-blob match |

This is consistent with the compile-time inclusion in
`crates/engine/src/licences.rs` and `crates/ui-egui/src/help.rs`.
At this source pin, `packaging/linux/package.sh:164–175` moves the
AppDir's entire `usr/share/doc` subtree out before sealing; it does not
move the Mastra runtime directory. The source routes the desktop texts
to About > Licences and About > Attributions, and routes the four core
texts to `intcad-cli licences`.
Binary hashes: desktop
`de5cac5682fe8a4a7a5fdd23984ed4cf68e869f1f58b0c28f26aaf6ffbd50601`,
CLI `109ee0da72883111c9cadb693595906d92f168aaf064bd585c4cc43211425694`.
Local offsets and sizes:
`/tmp/opencode/int1454-appimage-notices.3MJIHh/embedded.txt`
(SHA-256 `4f3337a8b78ed968c499d002aa030c2f306501b3c8327a1c0335cb259a207a51`).

This confirms embedded bytes despite the missing standalone filenames.
It does not check access through the About dialog or CLI, distribution
rights, notice completeness, or final-candidate packaging. The test merge
does not have the vendored `solidcore/` tree, so these results cannot
verify the final vendored-kernel notice.
