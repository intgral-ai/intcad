# Linux web view: frame timing and input evidence

This records what was measured on a real desktop for the Linux Assistant panel, and what the fix
changed. It is evidence for this branch, not a general performance claim.

## Host

- AMD Radeon RX 9060 XT (RADV GFX1200), Vulkan, MSAA 4x; `ui.inspect` reports
  `renderer: gpu`, `adapter: Vulkan · AMD Radeon RX 9060 XT (RADV GFX1200)`.
- niri, one 2560×1440 output at 100 Hz, logical scale 1.25.
- Application window through XWayland (`SOLIDCRAFT_WAYLAND=0`), and a native Wayland run
  (`SOLIDCRAFT_WAYLAND=1`).
- No software-rendered or headless result is used for the interaction claims.

The app exposes `frame_gaps_ms` in `ui.inspect`: the gaps between the last frames, oldest first.
`frame_ms` is the UI's own work in the last frame, so a 100 ms gap with 0.5 ms of work is a frame
the event loop did not ask for, not a slow frame.

## Measurement

`/tmp/opencode/measure_fps.py` opened the Assistant, moved the real pointer into the 3D view with
`xdotool`, held button 1, moved in small steps for three seconds, released, and then read
`frame_gaps_ms`. The app was started with isolated settings/control folders and
`SOLIDCRAFT_UPDATE_CHECK=0`.

## Results (this branch)

| Run | Frames | p50 gap | p95 gap | Mean rate | Gaps > 50 ms |
|---|---:|---:|---:|---:|---:|
| Assistant closed, model drag | 214 | 10.00 ms | 10.15 ms | **99.9 fps** | 0 |
| Assistant open, model drag | 240 | 10.00 ms | 10.14 ms | **99.9 fps** | 0 |
| Assistant open, no pointer movement | 100 | 85.03 ms | — | **~11.8 fps** (idle) | — |
| Native Wayland, Assistant open (diagnostic) | 30 | 10.0 ms while the control channel drove frames, then a 2.9 s idle gap | — | 100 Hz while active | 0 |

The 100 Hz display is the ceiling; the app reached it in both drags. **No ~10 FPS interaction
lock was reproduced on this host.** The idle case is deliberately below the display rate: an
untouched Assistant asks for a frame about every 85 ms, not every 16 ms, and does not busy-loop.

### What changed

Before the fix, `turn_gtk` treated *any* GTK work in a turn as page work:

```text
while gtk::events_pending() { gtk::main_iteration_do(false); turns += 1; }
self.at_work |= turns > 0;
```

GTK always has idle timers, so `at_work` was effectively always true and an open Assistant kept the
app at the display's rate (10 ms gaps) with no user input. The branch now counts only frames the
page actually drew (the web view's `draw` signal), page IPC, drags and presses; `fit()` no longer
re-allocates the page on every frame, only when the view moved or was shown again. The same idle
run then measured 85 ms p50 instead of 10 ms, and stayed at 100 fps under a real drag.

The first key after a click is handed over in GTK's press callback, not on the next frame, so it
is not lost to the hand-over; the page is also given GTK focus so the input method can offer a
composition.

## What is not proven here

- **Chinese input method**: the Nix package used to let GTK read the system's immodules cache,
  which names the session's fcitx5-gtk built against another GLib, so GTK refused the module
  (`GLib version too old (micro mismatch)`). The wrapper now builds a cache from this closure's
  GTK and fcitx5-gtk (`lib/gtk-3.0/3.0.0/immodules.cache`) and sets `GTK_IM_MODULE_FILE` to it;
  the built package starts the Assistant with no module warning. ASCII typing into the page
  composer after one click is verified on XWayland; the fcitx candidate window and committed
  Chinese text are still a person test at the desktop (the test session locked before that run),
  not something a module-loading check proves.
- **niri pointer injection**: niri exposes no pointer-injection tool here (`ydotool` needs
  `/dev/uinput`, which this user cannot open; `xdotool` coordinates are XWayland coordinates and
  do not address the compositor's real pointer). The X11/Xvfb run above typed `hello` into the
  page; the native run shows the diagnostic. A real niri click/focus run is for a person.
- A real agent stream (no agent is installed on this host), a software GPU (`llvmpipe`), and
  screens at other scales/edge configurations are not covered.

## XWayland production path on niri

The shipping host is `eframe`/`winit` + `wry`; on niri it prefers native Wayland (whose X11 child
cannot exist), so this run selected the XWayland path explicitly:

```sh
SOLIDCRAFT_WAYLAND=0 solidcraft --control 18905
```

The window (1280×720 logical at scale 1.25) opened with the real Assistant page
(`ui.agent_chat` reported `webView: {host: true, page: true}`), the CAD UI and the page in one
window. One click in the composer followed by `xdotool type "abc"` put the text in the page on the
first click — the press hand-over works, no second click or sleep
(`docs/screenshots/linux-xwayland-assistant-first-input.png`).

The input method was the failure this run found: GTK loaded the session's `im-fcitx5.so`
(fcitx5-with-addons 5.1.23) from `/run/current-system/sw/etc/gtk-3.0/immodules.cache`, while the
app links GTK/GLib from its own closure, so it printed `GLib version too old (micro mismatch)` and
`Loading IM context type 'fcitx' failed`, and ASCII typing went through only while the IM was not
engaged. `064f45be6` makes the Nix wrapper build an immodules cache from this closure's GTK and
fcitx5-gtk and set `GTK_IM_MODULE_FILE`; the built package then started with neither warning.

The input method is now fixed in the Linux host. wry's `build_as_child` gives the page a
**foreign** GdkWindow; GDK selects only `STRUCTURE_NOTIFY | PROPERTY_CHANGE` on it, so GTK never
receives focus-in, never marks the window active, and the page's input method never starts
(`page press: has_focus=false parent_active=false`). The host now creates a GtkWindow that owns its
GdkWindow, reparents it into the app's X11 window before mapping it, and gives the keyboard to
that window; GTK's input method then has a real client window.

Verified in an isolated Xvfb + fcitx5 session (state 2, pinyin), four consecutive runs, with the
production app (not the GTK prototype): clicking the composer shows the preedit and the candidate
window, and space commits the Chinese text:

- `docs/screenshots/linux-xwayland-ime-preedit.png` (`ni hao`, candidates `1. 你好 …`)
- `docs/screenshots/linux-xwayland-ime-commit.png` (`你好` in the composer)

Drag-and-drop, edge resize, menu/popover stacking over the page, panel reopen/exit and 150%
scaling are still not covered here; window maximise/restore on niri needs a compositor action
(`maximize-window-to-edges` resized the window and the panel with it). One early window (not
fullscreen, no explicit scale variables) drew the page at about twice the expected size; later
default runs rendered normally, so this is recorded as an unreproduced fractional-scale
observation, not a pass.

## Host acceptance, X11 (headless Xvfb, production app)

Run at `b88bed5b9` in an isolated `Xvfb` + private D-Bus + fcitx5 session, `SOLIDCRAFT_WAYLAND=0`:

| Check | Result |
|---|---|
| First click into the composer, then typing (four cold starts) | pass; text appears without a second click |
| Chinese composition and commit (fcitx5, pinyin) | pass; preedit + candidate window, space commits `你好` |
| Clipboard | pass; Ctrl+C put `clipboard-check` on the X clipboard (`xclip -o`), Ctrl+V put it back |
| 150% (`WINIT_X11_SCALE_FACTOR=1.5`) | pass; UI, panel and page scale together, nothing clipped |
| Panel close/reopen | pass; the page keeps the draft text |
| Preferences over the page | pass; the page gives way/hides, the dialog is not covered |
| Exit | pass; `ui.window close` ends the process |
| Window 1400→1000→1400 | layout ends correct; a capture 3 s after the shrink still showed the page at its old place/black at the new one, and a 10 s capture was correct. The page repaints slowly after a resize; needs a person on a real desktop |
| File drag-and-drop | not verified: the headless harness has no file manager or drag source |
| Maximise/restore | niri has no `maximize-window`; `maximize-window-to-edges` resized the window and the panel with it (tested on the previous host, not repeated here) |
| Live agent turn / tool approval | not verified: no agent with credentials is installed on this machine |

## Frame timing with the embedded GDK window (headless Xvfb)

Same production app at `b88bed5b9`, Assistant open, a 30×20×10 box, 1400×900 Xvfb (llvmpipe
presentation; the wgpu frame is RADV). `ui.inspect`'s `frame_gaps_ms`:

| Case | Frames | p50 | p95 |
|---|---:|---:|---:|
| Assistant open, no input | 169 | 84.6 ms (~11.8 fps) | 85.4 ms |
| Middle-drag in the 3D view | 240 | 22.7 ms (~44 fps) | 85.3 ms |
| Typing in the page composer | 240 | 8.6 ms (~116 fps) | 9.8 ms |

The idle case is a low refresh, not a busy loop. The drag rate is below the display rate on this
headless software-presentation host; the real-desktop measurement on the same branch was 99.9 fps
(`Assistant open, model drag` above), so this table is a headless lower bound, not a regression
claim. Typing keeps the page at the keyboard-turn rate; the cost of that rate on CPU is not
measured separately.

### Packaged Nix app

The package rebuilt at `b88bed5b9` (its Nix store path carries the version it was built at; wrapper cache
and the embedded GTK window) passed `packaging/assistant-check.py` under Xvfb (page loaded, frame,
clean exit, no crash report) and composes Chinese with the same harness: preedit/candidates, then
space commits `你好` (`docs/screenshots/linux-nix-ime-preedit.png`, `-commit.png`).
