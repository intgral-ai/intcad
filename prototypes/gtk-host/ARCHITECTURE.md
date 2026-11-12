# GTK host: architecture decision material (frozen experiment, not enabled)

**Decision (2026-11-12): the GTK host is paused and frozen as a reproducible prototype.** It is not
merged and does not enable a new production host. PR 110 (eframe/winit + wry, X11/XWayland) is the
shipping path; native Wayland embedding is a separate follow-up task and is not counted as
supported by this stage. Resume entry: `PRODUCTION-RENDERER.md`.

## Reused unchanged

- `solidcraft_engine`, `solidcraft_gpu`, `solidcraft_render`, `solidcraft_doc`, `solidcraft_io`,
  kernel/sketch/step: no CAD or business rewrite.
- `solidcraft_ui_egui::SolveApp`: `raw_input_hook`, `logic`, `ui`; the real CAD UI and viewport.
- `egui_wgpu::RenderState` / `Renderer` and the production `set_wgpu_split` scene/window resources.
- `agent_chat` business state, `packages/assistant-web` page, the `openskyfall-assistant://` scheme,
  IPC/state serializer and the WebKit page.
- The existing eframe/wry host for Windows/macOS and for X11 until a Linux host is accepted.

## Replaced only on the Linux GTK host

| eframe/winit today | GTK host |
|---|---|
| `eframe::run_native`, `App`/`Frame` | GTK main loop + `SolveApp` calls |
| winit window/event loop and X11/Wayland choice | `GtkWindow` + GTK backend |
| eframe `wgpu` surface for the window | offscreen wgpu texture + `GtkGLArea` blit (current experiment) |
| `eframe::Storage` / `frame.storage_mut()` | small file/prefs adapter with the same debounce |
| wry X11 child view and X11 keyboard handover | GTK WebKit child view (native Wayland-capable) |
| `rfd` parent from `eframe::Frame` | `rfd`/GTK dialog parent |
| macOS native menu | unchanged (macOS keeps its host) |

No upstream fork is required: `gtk`, `gdkx11`, WebKitGTK and `wgpu` are already in the tree or
ordinary crates. The Linux host can live behind `#[cfg(target_os = "linux")]` in a new module or a
feature, while Windows/macOS keep `eframe`; upstream sync touches the host module and a small
`assistant_web` host trait, not the CAD kernel.

## What the experiment has proved

- One GTK window on **X11 and native Wayland** can show the real CAD UI, the production wgpu
  viewport and the real Assistant page (X11: `evidence/gtk-wgpu-blit-assistant-x11.jpg`; Wayland:
  `gtk-wgpu-blit-assistant-wayland.jpg`).
- A direct wgpu swapchain on GTK's window `wl_surface` is not composited; the offscreen wgpu →
  `GtkGLArea` blit works on both backends.
- Forcing `Backends::VULKAN` avoids wgpu GL/EGL `BadAccess` while GTK's GLArea context is current.
- The production eframe/wry host, page open, same box, 5 s (Xvfb, RADV) observed **57%** main
  CPU and 668 MB + WebKit 368 MB; page closed 0%, 609 MB. An earlier GTK "0%" comparison is
  withdrawn: the blit prototype's repaint scheduling was broken (it could draw one frame), so no
  performance comparison is claimed. The production 57% observation stands alone.

## What is not proved / known gaps

- The blit uses a **per-frame GPU → CPU → GPU** copy; its interaction frame time and CPU cost are
  unmeasured. It is a localization path, not yet a production rendering path.
- Focus hand-off in the blit window: `wtype nihao` with fcitx produced no candidate/committed text
  in the page composer even though niri reported the window focused. The earlier `gtk_host.rs`
  proved the page IME on the same backend, so this is a prototype focus bug.
- No interaction frame-time comparison (rotate/pan/zoom, typing/streaming), no first-input,
  DnD, 125/150% scaling, cross-screen, edge resize, maximize/restore, menu/popover stacking,
  panel reopen/exit tests in the combined window.
- The page state is the production serializer on an empty session; no live agent turn/tool action.
- A new Linux host changes packaging (new GTK/WebKit deps are already linked by wry; the host
  itself is Rust). Every Linux package must be re-validated after the switch; old package evidence
  does not cover a new host.
- A long-lived GTK fork is not needed, but a Linux-only host module is a maintenance surface for
  GTK 3 / WebKitGTK changes until GTK 4/WebKit 6 is justified.

## Recommendation

1. PR 110 (eframe/winit + wry, X11/XWayland Assistant) is the shipping path for this stage.
   Windows/macOS keep their existing hosts.
2. The GTK prototype is frozen, not merged. Its measurements (idle 0 frames/s, ~52 fps drag under
   Xvfb, 2× scale, RGBA, DPI, pointer/wheel input) are experiment evidence only: the per-frame
   GPU→CPU→GPU copy, blocking `poll`, missing keyboard/IME/DnD and the forced-Vulkan fallback
   remain blockers.
3. If the GTK direction is resumed, the first step is GPU interop (DMABUF/EGL or a Wayland
   subsurface), then keyboard/IM, then interaction and IME acceptance. Only after those pass
   should a Linux-only host behind a preference be considered.
4. Native Wayland on the shipping eframe/wry host remains diagnostic; it must be recorded as not
   supported, not as a pass.
