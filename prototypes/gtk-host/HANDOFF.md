# Handoff: Assistant, packaging and GTK host prototype

Last updated from run `43243621752` (in progress) and prototype `0cb774406`.

## Branches / PRs

- **PR 110**, branch `integration/pil-290-assistant`: one Web Assistant page, Linux focus/idle
  fixes, packaging. Head `202072454` (Windows UpgradeCode/ARP/repo-root fixes, gating Snap
  offline Assistant with the network-namespace isolation proof, nightly-test update, software GL
  for the forced-GL headless Flatpak probe). Base `pil-290-packaging`.
- **PR 111**, branch `prototype/gtk-host`: disposable GTK 3 host evidence. Head `c396b9196`.

## Package run evidence

- `43170938257` at `66d7f10ca` (superseded for Windows/Snap gating):
  - Pass: Linux x86_64/aarch64 (packages + bare-system installed Assistant checks), Web, FreeBSD,
    macOS build x86_64, Arch x86_64/aarch64, Snap x86_64/aarch64.
  - Snap offline Assistant passed on both arches under `sudo unshare -n` (loopback only):
    `snap x11: PASS renderer=gpu, box=6000 mm³, Assistant page loaded`.
  - Flatpak x86_64 passed: X11 page, Wayland/XWayland page (Weston 13 log parsing), native-Wayland
    diagnostic.
  - Flatpak aarch64 failed at the X11 probe (`Flatpak desktop exited 1`; app log stops after GPU
    tessellation with MESA-EGL DRI3). x86_64 passes the same code; open aarch64 gap.
  - Windows x64 failed because the MSI reports `{GUID}` and the script compared unbraced text;
    fixed in `01e7fdb58`. Windows x86/arm64 were not conclusive before cancellation.
- `43178617405` at `01e7fdb58`: Linux both, Web, FreeBSD, macOS builds/universal, Arch both,
  Snap both (offline Assistant), Flatpak both passed. Windows arm64 passed; Windows x64/x86
  failed after installing/removing the MSI because the ARP DisplayVersion is the MSI's numeric
  `0.1.0` while the script expected the app's `0.1.0-beta.4`. Fixed in `bf3acc2b6`: ARP is
  compared with `$meta.ProductVersion`; `solidcraft-cli --version` must report the full release.
- `43187753411` at `bf3acc2b6`: Linux/Web/FreeBSD/macOS/Arch/Snap/Flatpak x86_64/Windows x86 and
  arm64 passed. Windows x64 failed because `webview2-runtime.ps1`'s local `$root` overwrote the
  caller's `$Root`, so the Assistant smoke path became the EdgeWebView directory; fixed in
  `5c91281` (`$RepoRoot`, `$runtimeRoot`). Flatpak aarch64 default XWayland page passed, but the
  forced-GL `--gl-saved-four` probe exited 1 under Xvfb (`Gl, MSAA 1x`); `202072454` selects the
  software GL driver for that headless probe.
- `43243621752` at `202072454` (in progress): final exact-SHA run for the Windows repo-root fix,
  Flatpak aarch64 forced-GL, Snap isolation and all regressions.


## GTK host prototype

- `gtk_wgpu_host.rs`: direct wgpu surface from GTK; X11 works; native Wayland white (GTK 3 owns
  `wl_surface`).
- `gtk_wgpu_blit.rs`: offscreen wgpu → `GtkGLArea` blit; X11 + native Wayland; real `SolveApp` UI,
  real `solid.box` viewport, real Assistant page in one window. Force `Backends::VULKAN` to avoid
  `BadAccess` while GTK's GL context is current.
- Idle comparison withdrawn: the GTK blit's repaint scheduling was broken (one-frame loop), so
  the 0% figure is not comparable. Production eframe/wry page-open 57% CPU/668 MB + WebKit 368 MB
  is a single-host observation; page closed 0%/609 MB. The working code repaints on a 16 ms timer.
- Independent review found the later repaint/performance experiment broken (one-frame loop, DPI,
  colour, no input, lifecycle); it was reverted to the working `2c93231d8` and the idle/53 fps
  comparison is withdrawn. Gaps: CPU-copy interaction cost; DPI; input path; focus/IME; DnD/
  scaling/menu/exit; live agent turn.
- Docs: `RESPONSIBILITIES.md`, `PRODUCTION-RENDERER.md`, `ARCHITECTURE.md`.

## Next actions

1. Read run `43178617405`: Windows x64 (UpgradeCode fix), Windows x86/arm64, Flatpak aarch64,
   macOS universal, Linux/Arch/Snap/Flatpak x86_64 regression.
2. If Flatpak aarch64 still fails: capture the app's full stderr/crash, try a rerun; do not mark
   aarch64 Flatpak passed without evidence. The x86_64 path is proven.
3. Fix the blit window's focus/IME, then measure interaction frame time (drag/zoom) and idle on
   X11 and native Wayland; try DMABUF/EGL or a Wayland subsurface to remove the CPU copy.
4. Only after that, consider a Linux-only GTK host behind a preference; Windows/macOS stay eframe.

## Repro commands

```sh
# Package run (after pushing a SHA)
gh workflow run release.yml --repo intgral-ai/open-skyfall-desktop --ref integration/pil-290-assistant
gh run view <run-id> --json jobs --jq '.jobs[]|{name,status,conclusion}'

# Nix package Assistant on X11
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET SOLIDCRAFT_WAYLAND=0 \
  xvfb-run -a python3 packaging/assistant-check.py \
  --exe /nix/store/<pkg>/bin/solidcraft --evidence /tmp/opencode/nix-assistant-check-x11

# GTK blit prototype (X11 / native Wayland)
PROTO_UI=1 PROTO_MODEL=1 GDK_BACKEND=x11     target/nix/debug/examples/gtk_wgpu_blit
PROTO_UI=1 PROTO_MODEL=1 GDK_BACKEND=wayland target/nix/debug/examples/gtk_wgpu_blit
```
