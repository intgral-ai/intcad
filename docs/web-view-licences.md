# Desktop web-view licences and notices

**Status:** engineering notes, attribution and shipping notices for the Assistant's web view.
This is **not** rights clearance and not a legal opinion. The exception in
[`../skills/solidcraft-ground-rules/SKILL.md`](../skills/solidcraft-ground-rules/SKILL.md)
permits these components only as unmodified, dynamically linked, system-provided desktop
libraries. The open human legal-review questions are at the end; a lawyer has not answered
them.

The Assistant draws its interface in a web view (`apps/solidcraft/src/assistant_web.rs`).
The page itself (`packages/assistant-web/dist`) has separate notices in
`packages/assistant-web/THIRD-PARTY-NOTICES.txt`; this file is about the web-view engine
and its platform runtimes.

## Components

Versions in the first column are the exact ones in `Cargo.lock`; the system libraries are
whatever the distribution supplies, so no single version applies to every package.

| Component | Version | Licence | Upstream |
|---|---|---|---|
| GTK 3 (`libgtk-3.so.0`, `libgdk-3.so.0`) | distribution 3.24.x; Nix shell 3.24.52 | LGPL-2.1-or-later (GTK's `meson.build`; `COPYING` and headers carry the GNU Library GPL v2-or-later text) | https://gitlab.gnome.org/GNOME/gtk |
| WebKitGTK 4.1 (`libwebkit2gtk-4.1.so.0`, `libjavascriptcoregtk-4.1.so.0`) | distribution 2.4x/2.5x; Nix shell 2.54.2+abi=4.1 | LGPL (WebKit ships the GNU Library GPL v2-or-later text) and BSD-2-Clause, per file | https://webkitgtk.org/ |
| `wry` | 0.57.2 | Apache-2.0 OR MIT | https://github.com/tauri-apps/wry |
| `gtk`, `gtk-sys` | 0.18.2 | MIT | https://github.com/gtk-rs/gtk3-rs |
| `webkit2gtk`, `webkit2gtk-sys` | 2.0.2 | MIT | https://github.com/tauri-apps/webkit2gtk-rs |
| `javascriptcore-rs`, `javascriptcore-rs-sys` | 1.1.2, 1.1.1 | MIT | https://github.com/tauri-apps/javascriptcore-rs |
| `webview2-com`, `webview2-com-sys` | 0.39.1 | MIT for the bindings; Microsoft's terms for the bundled WebView2Loader binaries | https://github.com/wravery/webview2-rs |
| Microsoft Edge WebView2 Runtime (Windows) | machine's Evergreen runtime, not in Cargo.lock | Microsoft Software License Terms for the WebView2 Runtime | https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution |
| WKWebView (macOS) | part of macOS | Apple SDK terms; no library redistributed | https://developer.apple.com/documentation/webkit/wkwebview |

The Rust crates' licence texts are in `THIRD-PARTY-LICENSES.txt`, which
`cargo xtask licences` regenerates. That file is generated on merge and is never edited in a
pull request.

### Why these are not ATTRIBUTION.md rows

`ATTRIBUTION.md` inventories **files in this repository** and `cargo xtask assets` enforces
that. GTK 3 and WebKitGTK are neither in this repository nor bundled by most packages, so
an attribution row would name a file that does not exist and would be folded into the wrong
table. Their notice belongs with the other library notices in `NOTICE` and in
`packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt`, which is what distributors ship.

## Linkage facts

- The app links the system libraries **dynamically and unmodified**. They are ELF `NEEDED`
  entries, not `dlopen`ed, so the app does not start without them.
- `packaging/linux/nfpm.yaml` declares `libgtk-3-0t64 | libgtk-3-0` and
  `libwebkit2gtk-4.1-0` for the deb, and the two sonames for the rpm. They are dependencies,
  not payload.
- Check a build with `readelf -d target/release/solidcraft | grep NEEDED`; it names
  `libgtk-3.so.0`, `libgdk-3.so.0`, `libwebkit2gtk-4.1.so.0` and
  `libjavascriptcoregtk-4.1.so.0` beside GLib, Pango, cairo and ATK.
- WebKitGTK's own `NEEDED` entries include the LGPL-2.1-or-later GNOME libraries (GLib,
  GIO, Pango, cairo, gdk-pixbuf, ATK, libsoup). Those are transitive, not direct app
  dependencies, but they are part of what a binary distributor must notice.
- On Windows, `webview2-com-sys` compiles `WebView2LoaderStatic.lib` into the executable, so
  no `WebView2Loader.dll` ships. The Evergreen WebView2 **Runtime** is not in the MSI or the
  portable zip; Windows 11 has it and most Windows 10 machines do. The app falls back to the
  built-in panel when it is absent.
- On macOS, `wry` uses the system WKWebView framework; nothing is bundled.

## Distribution shapes and what each must ship

| Shape | GTK 3 / WebKitGTK source | Notice requirement |
|---|---|---|
| deb, rpm, tarball | system packages, declared dependencies, not bundled | Ship the LGPL/BSD notice and licence text (for example `packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt` in `/usr/share/doc/solidcraft/`). The distro supplies the library source. **Missing today.** |
| AppImage | host system libraries; the AppImage bundles neither | The AppRun message explains which package to install; still ship the notice in the AppImage's docs. **Missing today.** |
| Flatpak | the GNOME 51 runtime supplies GTK 3 and WebKitGTK 4.1; neither is in the app bundle | The runtime is a separate Flatpak, so this bundle redistributes neither library. The Assistant page's notices still ship in the app. |
| Snap | the core24 `gnome` extension (`gnome-46-2404`) supplies GTK 3 and WebKitGTK 4.1; this snap stages neither | The content snap is separate, so this snap redistributes neither library. The Assistant page's notices still ship in the app. |
| Nix package | `flake.nix` adds `gtk3` and `webkitgtk_4_1` to the runtime library closure, wraps `LD_LIBRARY_PATH` with `wrapGAppsHook3`, and installs `share/doc/solidcraft/WEB-VIEW-PLATFORM-NOTICES.txt` | The closure **does redistribute** both libraries. The package metadata lists `lgpl21Plus` and `bsd2` alongside MIT/Apache, and the notice carries the LGPL/BSD texts. Source-availability evidence per release is still the release owner's item. |
| Windows MSI / portable zip | Evergreen WebView2 Runtime is not redistributed; the SDK loader is statically linked | State the Microsoft terms and the runtime dependency in the notices; confirm whether the WebView2Loader notice is required. **Missing today.** |
| macOS disk image | system WKWebView | No third-party library notice for the engine itself; the Assistant page notices still ship. |

`NOTICE` is compiled into every program (`Help > About > Licences`, `solidcraft-cli
licences`), so the notice reaches single-executable and AppImage users. The About dialog
still cannot show the system libraries' full licence text because `THIRD-PARTY-LICENSES.txt`
covers only Cargo crates; shipping `packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt` with
each package is the current answer. A UI change to show it in About is a `crates/ui-egui`
change and is not part of this work.

## Required notice, in plain terms

LGPL-2.1 section 6 asks a distributor of a work that links the library to:

1. give prominent notice that the library is used and is covered by the LGPL;
2. supply a copy of the licence;
3. include the library's copyright notice where the program displays notices;
4. allow the user to replace the library (a suitable shared-library mechanism, i.e. the
   dynamic linking used here, or a written source/relink offer).

BSD-2-Clause asks a binary distributor to reproduce the copyright notice, conditions and
disclaimer in the documentation or other materials. `NOTICE`,
`packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt` and the About dialog cover (1)-(3) for
the unmodified dynamic case; (4) is the dynamic link itself. None of this is a legal
conclusion.

## Open human legal-review questions

These are unresolved. Do not describe the web view as cleared, approved or compliant.

1. **GTK 3 licence expression.** `meson.build` declares `LGPL-2.1-or-later`, while `COPYING`
   and the headers carry the GNU Library GPL v2-or-later text. Is stating
   "LGPL-2.1-or-later" accurate for GTK 3, or must the notice say "LGPL-2.0-or-later"?
2. **WebKit per-file mix.** WebKit's licensing page says "portions licensed under the LGPL
   and BSD licenses". Which copyright notices must a binary distributor reproduce for
   WebKitGTK 4.1 as a whole, and is Apple's BSD notice sufficient for the BSD parts?
3. **Transitive LGPL closure.** WebKitGTK pulls in LGPL-2.1-or-later GLib, Pango, cairo,
   gdk-pixbuf, ATK and libsoup. Does the notice need to enumerate them, or is naming
   WebKitGTK and supplying the LGPL text enough when the distribution supplies the
   libraries?
4. **Bundled closures.** The Nix package ships the actual libraries (Flatpak and Snap
   use their runtimes' copies). What source-availability form satisfies LGPL section
   6(a)/(b)/(c): the distribution's source packages, the upstream release tarballs, or
   the Nix store source derivations? Who owns that evidence per release?
5. **WebView2 SDK loader.** `webview2-com-sys` statically links Microsoft's
   `WebView2LoaderStatic.lib` but its crate declares only MIT and carries no Microsoft
   licence file. Does shipping the executable require reproducing Microsoft's WebView2 SDK
   terms, and where should that notice go?
6. **Evergreen WebView2 Runtime.** The app does not redistribute it and does not ship the
   bootstrapper. Confirm that stating the dependency in the notices is sufficient, and that
   no Microsoft redistribution agreement is needed for the MSI/portable zip.
7. **Trademarks.** Are the words "GTK", "WebKitGTK" and "Microsoft Edge WebView2" used in
   the notices in a way their owners permit?
8. **The rule itself.** The exception is deliberately narrow. If a future dependency needs
   LGPL at a different layer (a CAD kernel, solver, geometry or UI library), it needs a new
   explicit rule change and review, not this exception.

Owners: licence/notice wording and the answers above need a human with authority to accept
the risk; packaging owners own the Flatpak, Snap and Nix changes; the release owner owns
the shipped notice files. No name is recorded here because none has signed off.

## Evidence

```sh
# Exact crate versions (the Windows-only crate needs the Windows target)
cargo tree -i wry -i gtk -i webkit2gtk
cargo tree --target x86_64-pc-windows-msvc -i webview2-com

# Direct linkage of a built app
readelf -d target/release/solidcraft | grep NEEDED

# Licences of every shipped crate
cargo xtask licences --check

# Asset/attribution gate (this file is Markdown and needs no row)
cargo xtask assets
```

Upstream licence pages: <https://www.webkit.org/licensing/>,
<https://gitlab.gnome.org/GNOME/gtk/-/blob/gtk-3-24/meson.build>,
<https://gitlab.gnome.org/GNOME/gtk/-/blob/gtk-3-24/COPYING>,
<https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution>.
