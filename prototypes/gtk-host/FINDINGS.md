# GTK host prototype findings (time-boxed, disposable)

Run on the host described in `docs/linux-webview-performance.md` (AMD RX 9060 XT/RADV, niri
100 Hz, scale 1.25). This branch is a prototype only; it is not proposed for merge.

## What ran

- `examples/gtk_host.rs`: a GTK 3 window with a `GtkGLArea` drawing the real
  `solidcraft-engine` `solid.box` scene through `view::home_camera/scene` and
  `solidcraft_render::Camera`, and the real `packages/assistant-web/dist` page in WebKitGTK 4.1
  served from an `openskyfall-assistant:` URI scheme (the checked-in page, not a mock).
- `examples/web_test.rs`: the same WebKitGTK page loading without the GL area.

## Results

| Check | Result |
|---|---|
| GTK window on native niri Wayland | yes (`gtk_host` xdg surface) |
| Real scene GPU draw (GtkGLArea) | yes; the box renders and orbit/zoom changes it |
| Render-rate loop (`PROTO_SPIN=1`, native) | **60–61 fps**, 16.67 ms/frame |
| Idle CPU after load (native, no spin) | **1.8 %** of one process, ~254 MB RSS |
| WebKitGTK page alone (native) | **yes** — `hello webkit` is painted |
| Assistant page alone (native, WebKitGTK) | load events `Started/Committed/Finished`; console silent |
| Real page **inside the GL host window** | **no painted page**: the WebView is allocated 430×1118 but stays blank while the GLArea is in the same window |
| Page alone on Xvfb | not a valid test: WebKitGTK also blank there, even without GLArea; Xvfb lacks the accelerated path |
| Page keyboard/focus/IME in host | not reached because the page never painted; `wtype`/xdotool text could not be seen |

The negative result is reproducible with the WebView before or after the GLArea, with WebKit
compositing disabled and with its DMABUF renderer disabled. It is a **GTK 3 limitation of two
independent GL users in one window** in this environment, not a page/scheme failure: the same
page engine paints when it is the only GL consumer.

## Other facts checked

- WPE WebKit is the official toolkit-free WebKit port (Igalia); `scrying 0.7.2` (MPL-2.0) has a
  `wpe` producer that imports WPE DMABUF frames into wgpu 30 (the project's version), but the
  host still owns the window, event loop, pointer/IME forwarding, DnD and lifecycle. `nixpkgs`
  here has `libwpe`/`cog` but no `wpewebkit`; Ubuntu publishes WPE 2.36 in jammy universe only;
  the Flatpak/Snap runtimes used here ship WebKitGTK, not WPE. WPE would add a second engine and
  a new LGPL/BSD packaging surface.
- GTK 4 + WebKitGTK 6.0 uses GSK's single renderer and is the next experiment worth time-boxing;
  it was not built here. It would still replace the eframe/winit host and its menus/dialogs.

## Recommendation

Do **not** adopt the GTK 3 host as the production path on this evidence: it cannot show the
Assistant page beside the CAD GL viewport in the same window. The two viable routes are:

1. **WPE texture embedding** — keeps the existing winit host, imports WebKit frames as a wgpu
   texture, and has the best embedding story; blocked on packaging a WPE runtime for Nix,
   Flatpak and Snap and on a new licence/notice review.
2. **GTK 4 + WebKit 6 single-renderer host** — still a large host replacement, but unlike GTK 3
   it has one GSK scene graph; it needs a separate prototype before any migration.

Until one is chosen and prototyped, X11/XWayland remains the supported Linux Assistant host.
