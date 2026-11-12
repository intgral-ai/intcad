# GTK 3 + production wgpu: disposable prototype (FROZEN)

**Status: frozen on 2026-11-12. Not merged, not a production host.** The production Linux host
stays `eframe`/`winit` + `wry` (PR 110). Native Wayland embedding is a separate follow-up task; the
shipping host keeps the diagnostic fallback there. This document keeps the prototype reproducible:
runnable code, the measurements that exist, the data path, the known defects, and the entry point
to resume the investigation.

## Milestone 1: production renderer in a GTK window

`apps/solidcraft/examples/gtk_wgpu_host.rs` creates a real `wgpu::Surface` from the GTK window's
native handle (Xlib or Wayland) and uses eframe's **`egui_wgpu::RenderState` / `Renderer`** — the
same renderer and viewport resources as the production app. No glow shader.

| backend | surface created | content visible | evidence |
|---|---|---|---|
| X11/XWayland | yes, Vulkan RADV GFX1200 | **yes** | `evidence/gtk-wgpu-x11.jpg` |
| native Wayland | yes, Vulkan RADV GFX1200 | **no**, GTK window stays white | `evidence/gtk-wgpu-wayland.jpg` |

## Milestone 2: real CAD UI, viewport and a model edit

`PROTO_UI=1` drives the real `solidcraft_ui_egui::SolveApp` (`raw_input_hook`, `ui`, `logic`) and
installs the production scene/window split resources with `set_wgpu_split(&rs, 24, 0, 4)`.

- **X11, real GPU (RADV GFX1200, niri XWayland):** the open-skyfall toolbar, browser, history,
  view cube and a real `solid.box` render in the GTK window. Evidence:
  `evidence/gtk-wgpu-ui-real.jpg`, `gtk-wgpu-ui-real.log`.
- **X11, software GPU (Xvfb + llvmpipe):** the same UI with a real model edit:
  `PROTO_MODEL=1` creates a 30×20×10 box; at frame 300 `PROTO_MODEL2=1` adds a
  12×12×36 box at `corner=[30,0,0]`. The browser goes from **1 body** to **2 bodies** and the
  viewport updates. Evidence: `evidence/gtk-wgpu-model-before.jpg`,
  `evidence/gtk-wgpu-model-after.jpg`, `gtk-wgpu-model.log`.
- Commands:

```sh
# real GPU, X11
PROTO_UI=1 PROTO_MODEL=1 GDK_BACKEND=x11 target/nix/debug/examples/gtk_wgpu_host
# software GPU, two-step edit (Xvfb)
PROTO_UI=1 PROTO_MODEL=1 PROTO_MODEL2=1 GDK_BACKEND=x11 target/nix/debug/examples/gtk_wgpu_host
```

This proves the production wgpu renderer **and the real egui CAD UI/viewport** can live in a GTK 3
window on X11, including a model change that updates the display.

## Native Wayland: the direct swapchain is not composited, the offscreen blit is

The direct `wgpu::Surface` on GTK's window `wl_surface` is created successfully, but GTK 3 owns
that surface's buffer/commit cycle; presenting a separate swapchain to it is not composited (white
window). This is a GTK 3 `wl_surface` ownership limit, not a page/shader/WebKit failure.

`apps/solidcraft/examples/gtk_wgpu_blit.rs` now proves the next path:

- wgpu renders the production `SolveApp` frame (and the real `solid.box` viewport) to an offscreen
  texture;
- the texture is copied to a buffer and mapped (row-aligned);
- GTK's `GtkGLArea` GL context uploads it with `tex_image_2d` and draws a full-screen quad.

On native Wayland with RADV GFX1200 this shows the real CAD UI and the box; the same code also
works on X11. Evidence: `evidence/gtk-wgpu-blit-wayland-cad.jpg` (crop of the GTK window; the
game strip above is another window) and `gtk-wgpu-blit-wayland-cad.log`.

### Data path and its limit

This is a **per-frame GPU → CPU → GPU** copy:
`wgpu texture → copy_texture_to_buffer (256-byte rows) → map → tight Vec<u8> → GL tex_image_2d →
GtkGLArea draw`. It is a localization tool, not an automatically acceptable production path. It
must be measured (CPU time, frame time, memory bandwidth) before any claim. The simple label run
reported ~53 fps average at 1200×800 on RADV with this copy; the CAD-UI run is the next measurement.

Next options, in cost order:

1. Measure this blit path properly; if the copy is acceptable, keep it as the experimental native
   Wayland host and improve the upload (persistent texture, dirty rows).
2. GPU interop: export the wgpu texture as a DMABUF/EGL image and import it into GTK's GL context
   (no CPU copy); more platform code, but the maintainable target if the copy is too slow.
3. Wayland subsurface: wgpu presents to its own `wl_subsurface` under GTK's window; avoids the
   copy but moves input/stacking ownership to the child surface.
4. GTK 4/GSK or WPE: larger host/engine changes, not justified by this result.

## Not yet proved

- IME/focus/DnD/scaling/menu stacking in this production-renderer GTK window. `wtype "nihao"`
  with fcitx in the focused blit window produced no candidate or committed text in the page
  composer; the WebView text area did not receive the keys even though `niri` reported the window
  focused. The earlier `gtk_host.rs` proved the page's IME on the same backend, so this is a focus
  hand-off gap in the blit prototype, not a page/IME failure. Do not claim IME here.
- A real interaction frame-time comparison with the eframe host. The prototype repaints every
  16 ms; the 50.7 fps average on the label run is not an interaction benchmark. The model run used
  llvmpipe, so it is not a real-GPU performance claim.

## Milestone 3: the real Assistant page in the same native-Wayland window

The blit window now also packs a WebKitGTK view beside the `GtkGLArea`, using the same
`openskyfall-assistant://` scheme, IPC shim and production `agent_chat::web::state` serializer as
the earlier `gtk_host.rs`. On native Wayland the same window shows:

- left: the real CAD UI and the `solid.box` viewport rendered by the production wgpu path and
  blitted through GTK's GL context;
- right: the real Assistant page ("Set up an assistant", "Open Preferences", the composer), with
  `PROTO load Finished`, title `Assistant`, and IPC messages received.

Evidence: `evidence/gtk-wgpu-blit-assistant-wayland.jpg`, `gtk-wgpu-blit-assistant-wayland.log`.

The page's initial state is the production serializer on an empty session; this proves rendering
and the page/host handshake, not a live agent turn.

On X11 the same window works after forcing `Backends::VULKAN` for the offscreen frame (GTK's
GLArea context is current; wgpu's GL/EGL backend otherwise fails with `BadAccess`). Evidence:
`evidence/gtk-wgpu-blit-assistant-x11.jpg`. This is Xvfb/llvmpipe for the combined window; the
hard-RADV X11 evidence is the direct-surface `gtk_wgpu_host.rs`.

## Frozen state and measurements

The last runnable code is committed on branch `prototype/gtk-host`. It is an example only; no
production crate or host changed.

What was fixed and measured before the freeze (X11 under Xvfb, 1400×900; native Wayland on niri,
RADV GFX1200; `PROTO_UI=1 PROTO_MODEL=1`, a 30×20×10 `solid.box`, real Assistant page open):

| case | result | evidence |
|---|---|---|
| idle, X11 | 0 frames/s logged after startup, 0 % main CPU, 362 MB | `gtk-blit-repaint.log/.jpg` |
| idle, native Wayland | 0 frames/s, 0 % main CPU, 343 MB | not archived as an image; log kept in the run notes |
| middle-drag orbit, X11 | 50.9–53.1 fps, 18.8–19.7 ms/frame | `gtk-blit-mid-before/after.jpg` |
| wheel zoom, X11 | viewport changes | `gtk-blit-wheel-before/after.jpg` |
| 2× scale, X11 | UI fills the window, text at native density | `gtk-blit-scale2.jpg` |

Finding: calling `egui::Context::set_pixels_per_point` every frame makes egui request an immediate
repaint (`set_zoom_factor`), so the window repainted at 60 fps forever. The code now applies the
scale only when it changes, and idle drops to 0 frames and 0 % CPU on both backends. This is a
prototype bug, not a production performance claim.

The prototype does not meet the production bar:

- the blit is a per-frame **GPU → CPU → GPU** copy: `copy_texture_to_buffer` (256-byte rows) →
  `map` → tight `Vec<u8>` → `tex_image_2d` → quad; there is no DMABUF/EGL interop and no dirty-row
  upload. Frame cost at 1200×800 is ~19 ms on this machine, dominated by the copy/sync.
- `device.poll(PollType::Wait { timeout: None })` blocks the GTK loop on the GPU every frame.
- no keyboard, IME, focus hand-off, drag-and-drop, menus/popovers, or window-edge resize tests in
  this window; pointer press/move/scroll exist, keyboard does not.
- the Assistant page receives the production serializer's state snapshot for an empty session; no
  live agent turn or tool approval is exercised here.
- X11 drag numbers are from Xvfb (GL through llvmpipe) with the wgpu frame still on RADV; the
  native-Wayland pointer was not injected (no pointer-injection tool in that session).
- forced `Backends::VULKAN` has no in-app fallback; a machine without Vulkan gets a blank window.

## Production-host conditions

1. real `SolveApp`, production wgpu viewport and real page in one window: **yes** on X11 and
   native Wayland in this experiment (`gtk-wgpu-blit-assistant-x11.jpg` and
   `gtk-wgpu-blit-assistant-wayland.jpg`). The page state is the production serializer on an
   empty session, so a live agent turn is not part of this proof.
2. X11 and native Wayland both running: **yes** for the experiment; the production eframe/wry X11
   host remains the shipping path.
3. Changes isolated in a Linux host/interface: the experiment is an example only; no production
   host code yet.
4. No new engine/large fork/licence change: true.
5. No obvious performance regression / busy loop / disabled security: **not met**. The earlier
   idle comparison (GTK 0 % vs production 57 %) is withdrawn: the blit prototype's repaint
   scheduling was broken (it could draw one frame, and the window was not verifiably drawing).
   The production page-open 57 % observation is a single-host data point, not a comparison. The
   reverted working code repaints every 16 ms (not egui's `repaint_delay`), pays a per-frame
   GPU→CPU→GPU copy, and has no input path; interaction, first input, IME, DnD and scaling are
   unmeasured. This condition is not met for a production decision.

The experiment therefore remains an experiment; it does not enable a new production host.

## Resume entry

- Code: `apps/solidcraft/examples/gtk_wgpu_blit.rs` (offscreen wgpu → CPU readback → GtkGLArea
  blit; `PROTO_UI`, `PROTO_MODEL`, `PROTO_STATS` env switches).
- Run: `nix develop .#rust --command cargo run -p solidcraft --example gtk_wgpu_blit`, with
  `PROTO_UI=1 PROTO_MODEL=1` and `GDK_BACKEND=x11|wayland`.
- Next steps if resumed: GPU interop (DMABUF/EGL or Wayland subsurface) to remove the CPU copy;
  GTK keyboard/IM context → egui events; then interaction frame time and IME/DnD/scaling tests.
- Do not merge this branch into production; PR 110 is the shipping path.
