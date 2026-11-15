# Native Wayland and the Assistant's web view

**Status: decision needed before a host replacement. The Assistant currently runs on X11 and
XWayland only; native Wayland shows a diagnostic instead of a second chat UI.** This page records
what was checked, why the obvious patch does not work, and the two viable paths with their cost.

## What the app does now

The desktop app is an `eframe`/`winit` window and renders the CAD viewport through `wgpu`. The
Assistant is a page from `packages/assistant-web` in a `wry` web view (`apps/solidcraft/src/
assistant_web.rs`). On Linux `wry 0.57.2` builds that view as an **X11 child window**:

```text
wry-0.57.2/src/webkitgtk/mod.rs
  new_as_child() -> new_x11(parent, ..., is_child = true)
  let parent = match window.window_handle()?.as_raw() {
      RawWindowHandle::Xlib(w) => w.window,
      _ => return Err(Error::UnsupportedWindowHandle),
  };
  gdk_x11_window_foreign_new_for_display(...)   // GDK's X11-only foreign window
```

`wry`'s Linux backend is GTK 3 + WebKitGTK 4.1 and has no Wayland child path: a native Wayland
window's `RawWindowHandle` is `WaylandSurface`, which the code above rejects. This is not a setting
or an environment variable; it is the shape of the backend. `wry`'s own GTK construction uses
`gtk::Window` with a foreign `GdkWindow`, and GDK's foreign-window API is X11-specific.

Wayland has no protocol for one client to make a child surface of another client's `wl_surface`
(the `wl_subsurface` parent must be in the same client), and WebKitGTK/GTK do not expose a way to
hand their surface to `winit`'s connection. A GTK toplevel cannot be embedded in a non-GTK Wayland
surface. Making the Assistant a second toplevel and moving it near the app window is **not** a
child surface: it is the separate-window design the product rejected, and a tiling compositor such
as niri would manage it independently.

## What was observed

The app was built with this branch's GTK/WebKitGTK dependencies and run on a real niri session
(one 2560×1440 output at 100 Hz, logical scale 1.25) with both window systems:

| Run | Window system | `ui.agent_chat` `webView` | Panel |
|---|---|---|---|
| `SOLIDCRAFT_WAYLAND=0` (XWayland) | X11/XWayland | `{host: true, page: true}` | page in the X11 child window |
| `SOLIDCRAFT_WAYLAND=1` (native) | xdg-shell on niri | `{host: false, page: false}` | diagnostic, **not** the old conversation |

The timings, screenshots and `frame_gaps_ms` for these runs are in
[linux-webview-performance.md](linux-webview-performance.md); the commands are in
`docs/agent-chat.md` and `docs/control-protocol.md`.

This is a deliberate stop, not a silent downgrade: the app says on its error output and in the
panel that the page needs an X11 window, offers Graphics settings, and never pretends
that native Wayland has the Assistant.

## Options

| Option | What it means | Cost / risk | Verdict |
|---|---|---|---|
| **A. Keep X11/XWayland** | Current. Full Assistant on X11 and XWayland; native Wayland gets the diagnostic. | No host rewrite. File drops and the web view already work. Native Wayland users must switch the whole window to X11 for the Assistant. | **Shipping default until a host decision** |
| **B. GTK host for the whole app** | Replace `eframe`/`winit` with a GTK 3/4 application window; render the CAD viewport into a GTK `GLArea`/wgpu surface; put WebKitGTK in a GTK box beside it. | Replaces the event loop, menus, file dialogs, window/titlebar code and the `winit`/`eframe` integration the app's tests and platform code rely on. Large, months-scale, and risks the CAD viewport. | Needs explicit architecture decision and a real prototype with the CAD viewport, not a toy GL area |
| **C. WPE WebKit into an egui texture** | Use WPE WebKit (the embedded WebKit port, no GTK) with an EGL/DMA-BUF backend, import its buffer into `wgpu`, draw it in the panel, and forward pointer/keyboard/IME events. | True embedding on X11 **and** native Wayland; no host rewrite. New engine/dependency (LGPL, like WebKitGTK), a texture-import/input/IME bridge, and packaging for every Linux format. | The most plausible true-native path; needs a prototype and licensing review |
| **D. Separate Assistant window** | A GTK/Wayland toplevel beside the app. | Rejected by the requirement that the Assistant be embedded; niri would tile it. | Rejected |

## Decision requested

Authorize **B** (GTK host) or **C** (WPE texture) for a time-boxed prototype that uses the real
CAD viewport and the real Assistant page, with success criteria: 60 Hz model orbit at 1.25 scale,
Chinese IME composition/candidates, drag-and-drop of a file onto the page, focus return to the
model, and idle CPU comparable to XWayland. Until then, **A** is the production path and native
Wayland is a documented limitation.

This page does not claim that native Wayland is supported, and no code path silently switches the
user to XWayland.
