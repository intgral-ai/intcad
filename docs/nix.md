# SolidCraft with Nix

The flake supports `x86_64-linux`, `aarch64-linux` and `aarch64-darwin`. Enable Nix's `nix-command`
and `flakes` experimental features. The committed `flake.lock` pins nixpkgs and
its Rust toolchain; package evaluation checks that compiler against the Rust
floor in `Cargo.toml`.

## Run, build or install

Run the current app without installing it:

```sh
nix run github:intgral-ai/open-skyfall-desktop#solidcraft -- --sample
```

Inside a checkout, `nix run .` and `nix run .#solidcraft` launch the desktop app.
`nix run .#cli -- --version` launches the CLI. The legacy `.#solidcraft` app name
also works. These commands use the flake's exact wrapped binary, rather than an
older executable already on your shell's `PATH`.

Build without installing:

```sh
nix build github:intgral-ai/open-skyfall-desktop#solidcraft
./result/bin/solidcraft --version
./result/bin/solidcraft --sample
```

The result contains `solidcraft`, `solidcraft-cli` and the deprecated old-name CLI alias
(see [compatibility.md](compatibility.md)). **Building does not install a desktop-menu entry.** For a user
profile installation, run:

```sh
nix profile install github:intgral-ai/open-skyfall-desktop#solidcraft
solidcraft --sample
```

On Linux, this installs the SolidCraft desktop entry, MIME definition and generated icon
sizes alongside the commands. On macOS the package contains the binaries only; for a bundled
`SolidCraft.app`, use the release `.dmg`. NixOS/Home Manager can instead put the flake's
`packages.${system}.solidcraft` in `environment.systemPackages`/`home.packages`.
If a non-NixOS desktop does not discover profile-installed launchers, ensure that
its session includes `~/.nix-profile/share` in `XDG_DATA_DIRS`, then log out and
back in. The desktop/application ID is unchanged (see
[compatibility.md](compatibility.md#package-ids)).

Check which executable a plain terminal command would run:

```sh
type -a solidcraft
readlink -f "$(command -v solidcraft)"
solidcraft --version
nix run github:intgral-ai/open-skyfall-desktop#solidcraft -- --version
```

An unrelated old executable on `PATH` is not updated by `nix build`.
Use the flake app or `./result/bin/solidcraft` to verify the package you just built.

## Source builds and the Mastra runtime binding

A Nix build compiles from a source store path with no `.git` directory and no `git` in the
sandbox. Ordinary builds therefore use the unbound revision sentinel
`0000000000000000000000000000000000000000`. They compile normally, but the packaged Mastra
runtime gate refuses that host revision, so a source build never claims a verified installed
runtime. This repository's `flake.nix` passes no revision, so its builds stay unbound. A Nix or
source release build that needs the binding supplies the full 40-hex commit in
`SOLIDCRAFT_GIT_REVISION_OVERRIDE`; a dirty or exported source tree without it keeps the sentinel.

A release or Mastra-bundling build needs the real reviewed revision. Set
`SOLIDCRAFT_GIT_REVISION_OVERRIDE` to the full 40-hex commit (or build from a checkout with
`git`). A strict build (`SOLIDCRAFT_WITH_MASTRA=1` or `SOLIDCRAFT_RELEASE=1`) without one fails
with an explicit error instead of producing an unbindable package. For a source tarball:

```sh
SOLIDCRAFT_RELEASE=1 SOLIDCRAFT_GIT_REVISION_OVERRIDE="$COMMIT" cargo build --release
```

A downstream flake that needs a bound package must pass the same variable into the derivation;
this repository does not change `flake.nix` here.

## Graphics and file dialogs

The app wrapper includes Wayland, X11, xkbcommon, EGL, Vulkan-loader and D-Bus
libraries, and GTK 3 with WebKitGTK 4.1 for the Assistant's page. `wrapGAppsHook3`
also sets the GDK pixbuf loaders, GIO modules and GSettings schemas those two need,
so the packaged app works outside `nix develop`. It builds an input-method cache from this closure's GTK and
`fcitx5-gtk` (`$out/lib/gtk-3.0/3.0.0/immodules.cache`) and sets `GTK_IM_MODULE_FILE` to it, so a
user's fcitx5 daemon can compose Chinese without GTK loading the session's module, which is built
against a different GTK/GLib and is refused for a micro-version mismatch. `GTK_PATH` keeps the
same `fcitx5-gtk`; choosing fcitx as the session input method is still the user's setting. It exposes `/run/opengl-driver/lib` and
`/run/opengl-driver/share` so NixOS's graphics drivers and Vulkan manifests are
discoverable. Enable `hardware.graphics.enable = true` on NixOS and keep the
system's driver stack consistent with the compositor. File pickers also need a
working session D-Bus and the desktop's `xdg-desktop-portal` service.

On macOS (`aarch64-darwin`), none of this applies: winit talks to AppKit, wgpu renders through
Metal, and the GPU drivers ship with the operating system, so the darwin package carries no
library wrapper and no extra search paths.

On another distribution, graphics drivers outside the Nix store may need a
matching [nixGL wrapper](https://github.com/nix-community/nixGL). Follow its
GPU-specific instructions; a wrapper for a different driver is not a substitute.
For Intel/Mesa, after building:

```sh
nix run github:nix-community/nixGL#nixGLIntel -- ./result/bin/solidcraft --sample
```

If a window is black, first verify that you launched the newly wrapped package
and that your system exposes a functioning GPU driver. `WGPU_BACKEND=vulkan`
selects Vulkan for diagnosis; a loader without a usable driver still cannot render.
OpenGL rendering uses 1x anti-aliasing to avoid the multisampled black-frame failure.
Vulkan and other backends keep the requested sample count; saved preferences are preserved.
For older builds, including 0.3.0, set Anti-aliasing to 1x and restart when using OpenGL.
If that window cannot show the settings dialog, edit only the `msaa` field to `1` in
`~/.local/share/solidcraft/graphics.json`; preserve any other settings.

## Wayland, XWayland and niri

On Linux the app runs through XWayland whenever an X display is available, because
winit only supports dragging files from a file manager into the window on X11. On
niri (detected from `XDG_CURRENT_DESKTOP` with a Wayland socket present) it stays on
native Wayland instead, since window management such as maximize/restore misbehaves
under niri's XWayland. The trade-offs: **on native Wayland, dropping files onto the
window does not work** (use File > Open or pass the file on the command line), and the
**Assistant's web view is unavailable**: WebKitGTK draws the page in an X11 child
window, and a native Wayland surface has no such child. The Assistant panel says so and
offers **Graphics settings**; it does not fall back to a second egui
conversation. To use the Assistant on niri, set **Window system** to X11 (or Automatic)
and restart — the model view is then XWayland as well. This applies to every build, not
only the Nix package. The way back to native Wayland and the embedding question are
tracked in [native-wayland.md](native-wayland.md).

Choose it in Preferences › Graphics › **Window system** (Automatic, Wayland or X11; it applies
at the next start). The pane says which one is running and, on native Wayland, links to
winit's issue about dropping files there. Native Wayland is the better choice with
display scaling whenever the compositor doesn't let X11 apps scale themselves: XWayland
windows are then drawn at 100 % and stretched, so the interface looks soft and small. That's
the case with **fractional scaling** (a monitor at 125 % or 140 %), and on GNOME 47 (RHEL and
Rocky Linux 10) at any scale, 200 % included; see
[the troubleshooting page](book/src/guide/troubleshooting.md#the-interface-looks-blurry-on-linux-with-display-scaling).

The environment overrides the preference: `SOLIDCRAFT_WAYLAND=1` (always native Wayland) or
`SOLIDCRAFT_WAYLAND=0` (XWayland whenever an X display is available).

## Development shells

`nix develop` supplies runtime libraries and uses the Rust/C toolchain already on
your `PATH`. `nix develop .#rust` supplies the pinned toolchain, C linker and
`target/nix` build directory. Both retain the GUI and CLI's existing Cargo package
names (`solidcraft`, `solidcraft-cli`).
