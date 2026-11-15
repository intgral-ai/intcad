# INT-1472 handoff: Linux WebView placement drift

Handed off from the PIL-291 acceptance run. The placement fix is not kept on this branch.

## Build where it was observed

- Candidate 2, commit `c0a7566d4` (`coord/pil-291-candidate2`).
- Official Nix wrapper: `/nix/store/lylbk0nfi28yb2xg85kpbihcd8s2dnzg-solidcraft-0.1.0-beta.3`
- `.solidcraft-wrapped` SHA-256:
  `f5add322935e30bc78973803864190eb54e5361b98b8123630efd8cc77ed9bf1`
- Session: niri/Wayland, XWayland (app forced to X11 with `SOLIDCRAFT_WAYLAND=0`),
  `pixels_per_point = 1.25`.

## Symptom

The page (the GtkWindow holding the WebKit view) is a child of the app's X11 window. It is
placed at `(0,0)` instead of the Assistant panel rect: the page covers the top-left of the
app window and the right-hand Assistant panel area is blank. The panel is `side=right`,
width 601.6 points.

Observed state in the wrapper instance:

- App X11 window: `2398x1397` (window on screen, panel right).
- Expected panel rect (points): left `1317.81`, top `165.81`, size `591.59 x 896.59`.
- Expected target pixels at `ppp=1.25`: `x=1647 y=207 w=740 h=1121`.
- Actual child geometry: `x=0 y=0 w=740 h=1121`.

A previous instance with a `1272x1397` window showed the same: expected target `x=521
y=207`, child at `0,0`.

## How the placement works now

`AssistantWeb::keep_place` (apps/solidcraft/src/assistant_web.rs):

1. On creation, `build_embedded_gtk` reparents the GtkWindow into the app's X11 window,
   calls `place_gtk` (X11 `configure_window` with the target), shows it, calls
   `place_gtk` again, and schedules one 120 ms `place_gtk`.
2. After that, `place_gtk` is only called when the panel rect (`Placed`) differs from the
   last one (`self.placed != Some(at)`).

So anything that moves the child window after the last placement — a toolkit map, a
GDK configure, or the window manager — is never corrected.

## Reproduction (deterministic)

With the wrapper (or the debug build) running, the Assistant open, and the page in the
right-hand panel:

```sh
# child window id of the 740x1121 page (xwininfo -root -tree | grep 740x1121)
DISPLAY=:0 xdotool windowmove <child-id> 0 0
```

Before any fix the child stays at `0,0` indefinitely; the page covers the top-left and the
panel area stays blank. No drag or panel toggle was done, so this is exactly the user's
screenshot state.

## Measured timeline (debug build with tracing, cold start)

```
t=...171206  at rect=(647.41,165.81)..(1239.00,1062.41) size=(591.59x896.59) ppp=1.250
t=...171269  reparented xid=20971532 into parent=14680068
t=...171269  geometry x=0 y=0          (immediately after reparent)
t=...171269  place_gtk target x=809 y=207
t=...171269  geometry x=809 y=207      (after configure)
t=...171270  map-event; geometry x=809 y=207
t=...171876  timeout(120ms) re-place; geometry x=809 y=207
...
t=...177453  window resized; at rect=(...) target x=1799 ... geometry follows
t=...180397  ... target 1794, 1790, ... following the column width animation
```

The child follows resizes and reopen (`hide`/`show`) correctly in this run, so the
remaining trigger is a move after the last `place_gtk`, not a stale panel rect.

## Fix direction that worked (not committed here)

Compare the child geometry with the target about every 100 ms while the page is shown
(`get_geometry` on the view's X11 window) and call `place_gtk` when it differs. Verified
on the debug build:

```
moved child to 0,0 at t=...399.686
self-heal at t=...399.765   (79 ms later)
geometry x=521 y=207        (back on the panel rect)
```

Cold start, window resize and Assistant reopen all stayed on the target. The change was
reverted from `coord/pil-291-candidate2` so this branch keeps only DFM work; INT-1472 owns
the fix.
