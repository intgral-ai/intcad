# Handoff: Assistant, packaging and GTK host prototype

Last updated from run `43178617405` (in progress) and prototype `c396b9196`.

## Branches / PRs

- **PR 110**, branch `integration/pil-290-assistant`: one Web Assistant page, Linux focus/idle
  fixes, packaging. Head `01e7fdb58` (includes the Windows UpgradeCode fix and the gating Snap
  offline Assistant step). Base `pil-290-packaging`.
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
- `43178617405` at `01e7fdb58` (in progress): exact-SHA run for the Windows UpgradeCode fix,
  Flatpak aarch64 retry and gating Snap offline step.

## GTK host prototype

- `gtk_wgpu_host.rs`: direct wgpu surface from GTK; X11 works; native Wayland white (GTK 3 owns
  `wl_surface`).
- `gtk_wgpu_blit.rs`: offscreen wgpu → `GtkGLArea` blit; X11 + native Wayland; real `SolveApp` UI,
  real `solid.box` viewport, real Assistant page in one window. Force `Backends::VULKAN` to avoid
  `BadAccess` while GTK's GL context is current.
- Idle (page open, same box, Xvfb, RADV, 5 s): production eframe/wry main 57% CPU/668 MB + WebKit
  368 MB; GTK blit main 0%/363 MB + WebKit 363 MB. Page closed production: 0%/609 MB.
- Gaps: per-frame GPU→CPU→GPU copy (unmeasured interaction); `wtype` text does not reach the page
  composer in the blit window (focus); DnD/scaling/menu/exit; live agent turn.
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
