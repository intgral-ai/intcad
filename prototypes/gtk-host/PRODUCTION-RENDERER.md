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

## Native Wayland limitation (specific)

The wgpu surface is created successfully on native Wayland (the adapter/backend print), but GTK 3
owns the window `wl_surface`'s buffer/commit cycle; presenting a separate wgpu swapchain to the same
`wl_surface` is not composited over GTK's buffer, so the window stays white. This is a GTK 3
`wl_surface` ownership limit, not a page/shader/WebKit failure.

Next paths, in cost order:

1. **Offscreen wgpu → `GtkGLArea` blit**: render the production frame to a wgpu texture, then blit
   it in GTK's GL surface. Keeps the production renderer; adds a texture copy.
2. **Wayland subsurface**: wgpu presents to its own `wl_subsurface` under GTK's window.
3. GTK 4/GSK or WPE: larger host/engine changes, not justified by this result.

## Not yet proved

- The Assistant page beside the CAD UI in the same GTK window on native Wayland (the page itself
  was proven in the earlier `gtk_host.rs`; the production renderer surface is the current blocker).
- IME/focus/DnD/scaling/menu stacking in this production-renderer GTK window.
- A real interaction frame-time comparison with the eframe host. The prototype repaints every
  16 ms; the 50.7 fps average on the label run is not an interaction benchmark. The model run used
  llvmpipe, so it is not a real-GPU performance claim.
