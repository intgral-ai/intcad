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

The controlled matrix first established the page's host protocol, then varied only page content
and the GLArea. All runs used the same `openskyfall-assistant://` scheme, the same IPC shim, the
same production `web::state` serializer and the same niri/XWayland environment.

| page | GLArea | native Wayland | X11/XWayland | evidence |
|---|---|---|---|---|
| simple (visible heading/text/input) | no | page visible | page visible | `/tmp/opencode/proto4-wayland-*/simple-without.png`, `.../proto4-x11-*/...` |
| simple | yes | page visible beside real GL scene | same | `simple-with.png` |
| real Assistant | no | page visible (setup + composer) | same | `real-without.png` |
| real Assistant | yes | page visible beside real GL scene | same | `real-with.png` |

The initial blank was therefore **not** a GTK3/GL compositor conflict: the page had received no
`window.__assistantHost.state(...)`, so `App.tsx` returned `null`. With the IPC shim and a valid
initial state from the production serializer, the page renders in every group, including with the
GLArea. `window.ipc` messages (the page's `ready`) arrive at the host.

- Real scene render loop (native, `PROTO_SPIN=1`): **60–61 fps**, 16.67 ms/frame.
- Idle native host (no spin): **1.8 % CPU**, ~254 MB RSS.
- Typing `hello assistant` with `wtype` appears in the real composer.
- Chinese IME: with the flake's matching `fcitx5-gtk` module (`GTK_IM_MODULE_FILE` pointing at a
  cache built for the app's GTK/GLib), `nihao` produced the **candidate window** (你好, nihao, …)
  and Space committed `你哈哦` (candidate 1) into the composer. Evidence:
  `/tmp/opencode/ime-native4-candidates.png`, `ime-native4-text.png`. On X11/XWayland the same
  fcitx module gives `/tmp/opencode/x11-ime-candidates.png`, `x11-ime-text.png` (`你哈哦` committed).

## What the prototype still is not

- The CAD renderer is a small **prototype glow renderer** over the real engine scene/camera, not
  the production wgpu viewport. GPU/rotation proof is real; production rendering integration is
  not done.
- The initial Assistant state is a fixture produced by the **production serializer** on an empty
  session; it proves rendering/initialisation, not a live agent turn, tool approval or streaming.
- Drag-and-drop, menu stacking, window resize/exit and package behaviour still need the
  production host or a further prototype; none is claimed here.

## Other facts checked

- WPE WebKit is the official toolkit-free WebKit port (Igalia); `scrying 0.7.2` (MPL-2.0) has a
  `wpe` producer that imports WPE DMABUF frames into wgpu 30 (the project's version), but the
  host still owns the window, event loop, pointer/IME forwarding, DnD and lifecycle. `nixpkgs`
  here has `libwpe`/`cog` but no `wpewebkit`; Ubuntu publishes WPE 2.36 in jammy universe only;
  the Flatpak/Snap runtimes used here ship WebKitGTK, not WPE. WPE would add a second engine and
  a new LGPL/BSD packaging surface.
- GTK 4 + WebKitGTK 6.0 was not built; it is not part of this result.

## Method

| page | GLArea | what is checked |
|---|---|---|
| simple | without | visible background, heading and input box |
| simple | with | the same, beside the real GL viewport; DOM title: `PROTO DOM PROTO SIMPLE PAGE visible background, text, and input` |
| real Assistant | without | DOM has `data-proto-state`, visible composer/setup |
| real Assistant | with | the same, beside the real GL viewport; DOM title: `PROTO DOM No agent found Not connected Set up an assistant …` |

All four use the same `openskyfall-assistant://` scheme, the IPC shim, the production state
serializer and the same resource/environment; the only intended variable is the GLArea and the
page content. Logs record `state` injection, page `ipc` messages, `load-failed`, WebKit
`web-process-terminated`, and JavaScript `error`/`unhandledrejection` (the injected listeners
write `PROTO JSERROR`/`PROTO JSREJECT` through the page console), not only load-finished. The
real page with the GLArea logged neither JavaScript marker and its DOM title shows the setup
screen.

## Recommendation

The GTK 3 host **can** host the real page and the real GL scene in one window, on both X11 and
native Wayland; the earlier negative conclusion is withdrawn. The remaining cost is the host
replacement itself (eframe/winit event loop, menus, dialogs, production wgpu viewport, drag and
drop) and packaging, not page compositing.

No migration is proposed without a separate decision. The next evidence should be a production
renderer integration prototype (wgpu viewport in the GTK window) and the package matrix; WPE is
no longer justified by a compositing failure, since there is none.
