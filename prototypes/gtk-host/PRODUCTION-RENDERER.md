# GTK 3 + production wgpu: first milestone

Disposable experiment. It does not change production code or remove the eframe host.

## What ran

`apps/solidcraft/examples/gtk_wgpu_host.rs`:

- a GTK 3 `GtkDrawingArea` window;
- a real `wgpu::Surface` created from the GTK window's native handle (Xlib or Wayland);
- the production `eframe::egui_wgpu::Renderer` (the same renderer the app uses), drawing an egui
  frame — not the glow shader of the earlier `gtk_host.rs` prototype.

Commands (same machine, RX 9060 XT/RADV, niri):

```sh
GDK_BACKEND=x11     target/nix/debug/examples/gtk_wgpu_host
GDK_BACKEND=wayland target/nix/debug/examples/gtk_wgpu_host
```

## Result

| backend | surface created | content visible | evidence |
|---|---|---|---|
| X11/XWayland | yes, Vulkan (RADV GFX1200) | **yes**; egui frame, frame counter | `evidence/gtk-wgpu-x11.jpg`, `gtk-wgpu-x11.log` |
| native Wayland | yes, Vulkan (RADV GFX1200) | **no**: the GTK window stays white | `evidence/gtk-wgpu-wayland.jpg`, `gtk-wgpu-wayland.log` |

X11 frame counter: frame 349, 50.7 fps average (the loop requests a frame every 16 ms; this is a
prototype repaint policy, not production scheduling).

## The Wayland limitation (specific, not a broad GTK claim)

On Wayland the wgpu surface is created successfully — the adapter and backend print — but the
GTK window shows no wgpu content. GTK 3 owns the `wl_surface`'s buffer/commit cycle for the window
(and its drawing area); presenting a separate wgpu swapchain to the same `wl_surface` is not
composited over GTK's own buffer. This is the API/ownership boundary to solve, not a page or
shader failure.

The next viable paths, in order of cost:

1. **Offscreen wgpu → GTK GL blit**: render the production wgpu frame into a texture, then blit
   that texture in a `GtkGLArea` (GTK's own GL surface). This keeps the production renderer and
   should work on X11 and Wayland; it adds one texture copy and a GL/EGL hand-off.
2. **Wayland subsurface**: give wgpu its own `wl_subsurface` under GTK's window surface; GTK keeps
   the parent, wgpu presents to the child. Needs careful input/stacking ownership.
3. **GTK 4 + WebKitGTK 6.0 / GSK**: one renderer, but a much larger host migration; not started.

WPE is not justified by this result: the failure is GTK 3 `wl_surface` ownership, not WebKit.

## What this milestone does and does not prove

- Proves: the production `egui_wgpu::Renderer` and a wgpu swapchain can draw in a GTK 3 window on
  X11, with the real Vulkan adapter.
- Does not yet prove: the real `SolveApp` CAD UI/scene in that surface, model editing, the
  Assistant page beside it, IME/focus/DnD, or frame-time parity with the eframe host.
- The next milestone is the offscreen wgpu texture + GTK GL blit, then `SolveApp` and the page.
