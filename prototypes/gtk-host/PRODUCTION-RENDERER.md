# GTK 3 + production wgpu: first two milestones

Disposable experiment. It does not change production code or remove the eframe host.

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

- The Assistant page beside the CAD UI in the same GTK window on native Wayland (the page itself
  was proven in the earlier `gtk_host.rs`; the production renderer surface is the current blocker).
- IME/focus/DnD/scaling/menu stacking in this production-renderer GTK window.
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

On X11 the blit example with the WebKit view aborted under Xvfb (`MESA-EGL: DRI3 error`, core
dump); the direct-surface `gtk_wgpu_host.rs` renders the CAD UI on X11, and the earlier
`gtk_host.rs` renders the real page on X11, but the combined blit+page window was not validated
there. The production eframe/wry X11 host already has the CAD UI + page; no claim is made for the
blit example on X11.

## Production-host conditions (not yet met)

1. real `SolveApp`, production wgpu viewport and real page in one window: **yes on native
   Wayland** in this experiment; X11 combined blit not validated.
2. X11 and native Wayland both running: X11 via the existing production host; the GTK host's X11
   combined path is unproven.
3. Changes isolated in a Linux host/interface: the experiment is an example only; no production
   host code yet.
4. No new engine/large fork/licence change: true.
5. No obvious performance regression / busy loop / disabled security: **not measured**; the
   prototype repaints every 16 ms and uses a per-frame CPU copy, so this is not yet met.

The experiment therefore remains an experiment; it does not enable a new production host.
