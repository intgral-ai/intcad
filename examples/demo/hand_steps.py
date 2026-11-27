"""Hand steps for the next-step demos: a person's modelling through the app's own UI (toolbar
buttons, plane and face picks, typed values), driven over the control channel one pointer event at
a time so hover, dialogs and previews behave as for a person. The app draws the pointer itself in
recordings (SOLIDCRAFT_DEMO_KEYS=1), so nothing is composited afterwards.

`Hand(call, note)`: `call(method, params)` sends one control request and returns the reply;
`note(kind, **fields)` logs a beat."""

import json
import time


class Hand:
    def __init__(self, call, note, speed=1.0):
        self.call, self.note, self.speed = call, note, speed
        self.pos = (1150.0, 620.0)

    def ui(self, method, params=None):
        r = self.call(method, params or {})
        if "error" in r or r.get("ok") is False:
            raise RuntimeError(f"{method} {json.dumps(params)[:200]}: {r.get('error')}")
        return r.get("result")

    def at(self, **where):
        for _ in range(50 if "handle" in where else 3):
            r = self.call("ui.at", where)
            if r.get("ok", True) and "error" not in r:
                p = r["result"]
                return (p[0], p[1])
            time.sleep(0.1)
        raise RuntimeError(f"ui.at {where}: {r.get('error')}")

    def wait(self, s):
        time.sleep(s * self.speed)

    def glide(self, to, dur=0.45):
        frm = self.pos
        n = max(2, int(dur * 40))
        for i in range(1, n + 1):
            t = i / n
            e = t * t * (3 - 2 * t)
            self.ui("ui.move", {"x": frm[0] + (to[0] - frm[0]) * e, "y": frm[1] + (to[1] - frm[1]) * e})
            time.sleep(dur * self.speed / n)
        self.pos = to

    def click(self, to, dur=0.45, settle=0.35):
        self.glide(to, dur)
        self.wait(0.12)
        self.ui("ui.click", {"x": to[0], "y": to[1]})
        self.wait(settle)
        self.settle()

    def tool(self, command, panel=None):
        """Click a toolbar button, opening its panel's drop-down first when it sits in one."""
        self.settle()
        handles = self.ui("ui.handles")
        if panel and not any(h["name"] == f"toolbar:{command}" for h in handles):
            self.click(self.at(handle=panel), settle=0.4)
        self.click(self.at(handle=f"toolbar:{command}"), settle=0.5)

    def type(self, text, enter=True):
        """Type a value the way a person does: a character at a time."""
        for ch in text:
            self.ui("ui.text", {"text": ch})
            time.sleep(0.09 * self.speed)
        self.wait(0.4)
        if enter:
            self.ui("ui.key", {"key": "Enter"})
            self.wait(0.6)
            self.settle()

    def key(self, k):
        self.ui("ui.key", {"key": k})
        self.wait(0.2)
        self.settle()

    def settle(self, limit=120.0):
        """Wait until the camera, the dialog, the handles and any preview stop changing."""
        last, still = None, 0
        end = time.time() + limit
        while time.time() < end:
            time.sleep(0.1)
            state = self.ui("ui.inspect")
            current = json.dumps([state.get("camera"), state.get("tool"), state.get("dialog")], sort_keys=True, default=str)
            busy = (state.get("preview") or {}).get("busy", False)
            still = still + 1 if current == last and not busy else 0
            last = current
            if still >= 3:
                return
        raise RuntimeError("the UI did not settle")

    # -- modelling steps a person does ---------------------------------------------------------

    def sketch_on(self, **where):
        self.tool("sketch.create")
        self.click(self.at(**where), settle=0.9)
        self.settle()

    def centre_rectangle(self, centre, corner, space="sketch"):
        """A centre rectangle: click its centre, then a corner (sketch or world coordinates)."""
        self.tool("sketch.rectangle.center", panel="family:Rectangle")
        self.click(self.at(**{space: centre}), settle=0.2)
        self.click(self.at(**{space: corner}), dur=0.6, settle=0.3)
        self.key("Escape")

    def finish_sketch(self):
        self.tool("sketch.finish")
        self.wait(0.6)

    def extrude(self, distance, profile=None):
        """Extrude; `profile` (a world point inside it) picks the profile first, as a person
        clicks it when a face sketch leaves more than one."""
        self.tool("solid.extrude")
        self.wait(0.4)
        if profile is not None:
            self.click(self.at(world=profile), settle=0.4)
        self.type(distance)

    def fillet_edges(self, points, radius):
        self.tool("solid.fillet")
        for p in points:
            self.click(self.at(world=p), settle=0.25)
        self.type(radius)

    def hole_at(self, point, diameter):
        self.tool("solid.hole")
        self.click(self.at(world=point), settle=0.5)
        self.type(diameter)

    def home(self):
        self.ui("ui.view", {"view": "home", "animate": True})
        self.wait(0.8)
        self.settle()
