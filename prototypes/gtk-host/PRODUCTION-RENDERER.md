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
