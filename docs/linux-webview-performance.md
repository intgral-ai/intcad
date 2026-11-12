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

- **Chinese input method**: this Nix host has fcitx5-gtk built against a different GLib micro
  revision than the GTK/WebKitGTK the app links, so GTK refuses the `im-fcitx5` module
  (`GLib version too old (micro mismatch)`). A plain `xim` attempt did not engage either. The
  code now performs the GTK focus hand-over the input method needs; a candidate-list and
  committed-text run needs a distribution (or Nix profile) where the GTK immodule and GTK share
  one GLib.
- **niri pointer injection**: niri exposes no pointer-injection tool here (`ydotool` needs
  `/dev/uinput`, which this user cannot open; `xdotool` coordinates are XWayland coordinates and
  do not address the compositor's real pointer). The X11/Xvfb run above typed `hello` into the
  page; the native run shows the diagnostic. A real niri click/focus run is for a person.
- A real agent stream (no agent is installed on this host), a software GPU (`llvmpipe`), and
  screens at other scales/edge configurations are not covered.
