# GTK host prototype (disposable)

This is the time-boxed prototype authorized before any host migration. It is **not** the
application and is not wired into `main.rs`.

## What is real

- The CAD side is the application's own `solidcraft_engine::Session` (a real `solid.box` command),
  `solidcraft_engine::view::home_camera/scene`, and `solidcraft_render::Camera`/`Scene`, drawn by
  a `GtkGLArea` on the GPU. Every orbit/zoom is the real scene and camera transform.
- The Assistant side is the real checked-in page
  (`packages/assistant-web/dist/index.html`) in WebKitGTK 4.1.
- The window, input routing, focus, drag-and-drop destination and frame timing are GTK's, so the
  prototype exercises the candidate host rather than a mock.

It does **not** include the app's full egui UI, command windows, menus, tool-approval bridge or
the production wgpu pipeline. Panels/menus are therefore not evaluated by this prototype; the
production host would have to replace the eframe/winit event loop and port those.

## Build and run

```sh
nix develop .#rust --command cargo build -p solidcraft --example gtk_host
# X11 / XWayland:
GDK_BACKEND=x11 DISPLAY=:0 PROTO_LOG=1 target/debug/examples/gtk_host
# native Wayland:
GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-1 PROTO_LOG=1 target/debug/examples/gtk_host
```

`PROTO_SPIN=1` requests a frame every 8 ms (render-rate measurement, not idle policy).
`PROTO_PAGE=...` selects another page. `PROTO_LOG=1` prints one `PROTO frame ...` line per second.

## API and maintenance facts checked

- GTK 3 `GtkGLArea` is the candidate host widget; it is stable and already linked for WebKitGTK.
  The prototype uses GL core 3.3 through the process's GL loader, because `gdk::GLContext` in
  gtk-rs 0.18 does not expose `get_proc_address`.
- WebKitGTK 4.1 is the existing, packaged engine; WPE WebKit is the official toolkit-free WebKit
  port, maintained by Igalia (https://wpewebkit.org/), with the same WebKit LGPL/BSD licensing.
- WPE embedding in Rust would use `scrying 0.7.2` (MPL-2.0) with its `wpe` feature: it imports
  WPE DMABUF frames into wgpu 30 (the version this project uses) and exposes navigation, mouse/
  pointer, IME and capability reporting, but **the host still owns the window, event loop, input
  forwarding and drag-and-drop**; unified cross-platform DnD is explicitly still open in that
  crate. `nixpkgs` here has `libwpe` and `cog` but no `wpewebkit` package; Ubuntu publishes WPE
  2.36 only in jammy universe, and the Flatpak/Snap runtimes used here ship WebKitGTK, not WPE.
- WPE would therefore add a second web engine and a new LGPL/BSD packaging surface to deb/rpm/
  AppImage/Flatpak/Snap/Arch/Nix; the current ground-rule exception is scoped to WebKitGTK/GTK.

## Measurements and conclusion

See the task's final report; this README is committed with the prototype and its run commands.
