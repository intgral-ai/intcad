# GTK production-renderer prototype: responsibility map

Disposable experiment, not a production change. The existing `gtk_host.rs` proves GTK 3 can host
the real Assistant page and a GL scene; it uses a prototype glow renderer. This map is the first
step of the authorized follow-up: put the **production wgpu path and the real egui CAD UI** in a
GTK window, on X11 and native Wayland.

## What eframe/winit owns today

| Area | Current code | What a GTK host must provide |
|---|---|---|
| Window creation and lifecycle | `eframe::run_native`, `NativeOptions` (`apps/solidcraft/src/main.rs:1101`) | `GtkApplication` + `GtkWindow`; same initial size, titlebar/icon, close/quit flow |
| Backend choice | `prefer_x11` and `EventLoopBuilderExtX11::with_x11` (`main.rs:1127`) | Let GTK choose its backend, or set `GDK_BACKEND`; record the real one |
| Event loop / repaint | eframe/winit drives `App::logic`, `raw_input_hook`, `ui`, `save`, `on_exit` | GTK main loop; run the same `SolveApp` methods once per repaint; schedule with `request_repaint_after` |
| wgpu device/surface | `eframe::egui_wgpu` (`WgpuSetup`, device-lost watch, MSAA/depth) | `wgpu::Instance` + surface from the GTK window's X11/Wayland handle; device-loss handling |
| egui renderer | `eframe::egui_wgpu::Renderer` and the app's viewport callbacks | Same `egui_wgpu::Renderer` fed with `egui::FullOutput`; same `RenderState` resources for the viewport |
| Persistence | `eframe::Storage` / `app.ron`, `PrefsWriter` (`main.rs:120`) | Custom `eframe::Storage` implementation or a small file adapter with the same debounce |
| Input | winit -> egui `RawInput` (keys, IME, pointer, clipboard, DnD, scale) | GTK event controllers -> the same `RawInput`; GTK native IME/clipboard/DnD |
| File dialogs | `rfd` portal dialogs, parent from `eframe::Frame` | `rfd` with a GTK/portal parent, or GTK dialogs; keep the same folder memory |
| Native menus | macOS `native_menu`; Linux in-UI menus | GTK menus for the prototype; keep in-UI menus elsewhere |
| Assistant web view | `AssistantWeb` with wry child X11 window, X11 keyboard handover, GTK turns (`apps/solidcraft/src/assistant_web.rs`) | GTK WebKit view in the same window; reuse the page/IPC/state bridge, but a host adapter replaces the X11-child/global-focus code |
| Multi-window | egui viewports where used; native menus/dialogs | Decide per window: GTK windows or egui viewports; do not silently drop any |

## What can be reused unchanged

- `solidcraft_engine`, `solidcraft_gpu`, `solidcraft_render`, `solidcraft_doc`, kernel/IO.
- `SolveApp`: `logic`, `raw_input_hook`, `ui` are public; `content_ui` is a thin safe-area wrapper.
- `agent_chat` business state, the page in `packages/assistant-web`, the IPC schema/checks.
- The real scene/camera and the production viewport renderer, if the GTK host provides the same
  `egui_wgpu::RenderState`/callbacks.
- The Assistant page and bridge; only the host side (window, focus, keyboard) changes.

## What must be replaced for a GTK host

- `eframe::run_native` and the `eframe::App`/`Frame` plumbing.
- The wry X11 child-window host and its `Keyboard` handover (`assistant_web.rs`), on Linux.
- The winit event loop and its backend choice; GTK owns the window and input.
- The eframe storage adapter and `frame.storage_mut()` use in `App::ui`.
- `eframe::Frame` parameters passed into dialogs/Assistant placement.

## Smallest runnable milestone

1. A GTK 3 window whose content is a wgpu surface created from the GTK window's real X11 and
   Wayland handles; `egui_wgpu::Renderer` draws a small egui UI. Run on X11 and native Wayland;
   record backend, frame time and idle cost.
2. Put `SolveApp` in that loop: `raw_input_hook`, `logic`, `ui`, with the production scene
   viewport. Edit a real box and show the update.
3. Add the checked-in Assistant page in the same window (GTK WebKit), reuse the state/IPC bridge,
   verify focus, IME, file drop, scale, menus over the page, resize and quit.
4. Compare against the current eframe host on the same machine/model/settings: interaction frame
   time, idle CPU/GPU, memory. Separate refresh-rate and GPU-load effects.

## Known risks / decisions needed before any migration

- GTK 3 has no portable `RawWindowHandle`; the prototype must construct Xlib/Wayland handles from
  `GdkWindow`/`gdk_wayland_window_get_wl_surface`. This is the main API risk.
- `egui_wgpu` must accept the same `RenderState` as the production viewport; otherwise the scene
  has to be re-integrated, which is a larger change.
- A GTK event loop is not winit: repaint throttling, focus, IME and menu/popover stacking need
  their own equivalents; none is claimed until measured.
- A long-lived GTK fork is not proposed by this prototype alone; a migration would need a
  separate architecture decision and a maintenance plan.
