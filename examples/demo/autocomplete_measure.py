#!/usr/bin/env python3
"""Autocomplete's measurements with a real agent (docs/design/autocomplete-tests.md, M1, M2, M3, M6, M7, M10, M11):
hand steps only, then the suggestion, its line and its ghost; with TAB=1 the ghost is taken with Tab
and compared with what landed. Run by next_step_ab.sh (NSM_SCRIPT=autocomplete_measure.py).

Usage: autocomplete_measure.py PORT TOKEN OUTDIR DESIGN   DESIGN: case | bracket | plate | flange | in-... (those in inches) | holed | holed2 | sheet1 | sheet2 | sheet3
"""
import json, os, socket, sys, time

port, token, out, design = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
agent = os.environ.get("NSM_AGENT", "claude")
TAB = os.environ.get("TAB") == "1"
os.makedirs(out, exist_ok=True)
sock = socket.create_connection(("127.0.0.1", port))
f = sock.makefile("rw")
n = 0
rows = []


def call(method, params=None):
    global n
    n += 1
    req = {"id": n, "method": method, "params": params or {}}
    if n == 1:
        req["token"] = token
    f.write(json.dumps(req) + "\n")
    f.flush()
    while True:
        msg = json.loads(f.readline())
        if msg.get("id") == n:
            return msg


def chat(params=None):
    r = call("ui.agent_chat", params or {})
    return r.get("result", r)


def next_step():
    return chat().get("nextStep") or {}


def note(**kw):
    rows.append(kw)
    print(json.dumps(kw, ensure_ascii=False), flush=True)
    with open(os.path.join(out, f"{agent}-{design}.json"), "w") as fh:
        json.dump(rows, fh, indent=1, ensure_ascii=False)


def wait(pred, secs, step=0.05):
    end = time.time() + secs
    while time.time() < end:
        s = pred()
        if s:
            return s
        time.sleep(step)
    return None


def ex(command, params=None):
    r = call("engine.execute", {"command": command, "params": params or {}})
    if "error" in r or r.get("ok") is False:
        note(kind="hand step failed", command=command, reply=str(r)[:300])
    return r.get("result", r)


def revision():
    r = ex("document.revision")
    return r.get("result", r)


def bodies():
    r = ex("inspect.measure", {})
    r = r.get("result", r)
    return {b["name"]: (b.get("volume_mm3"), (b.get("kernel") or {}).get("faces")) for b in r.get("bodies", [])}


def boxes():
    r = ex("inspect.measure", {})
    r = r.get("result", r)
    return {b["name"]: b.get("bbox") for b in r.get("bodies", [])}


def inches(steps):
    """The same steps of an inch design: 25 mm is an inch, sizes as text with their unit, points in mm."""
    point = lambda v: [point(x) for x in v] if isinstance(v, list) else v * 25.4 / 25 if isinstance(v, (int, float)) else v
    size = lambda v: f"{round(v / 25, 6):g} in" if isinstance(v, (int, float)) else point(v)
    return [(c, {k: size(v) for k, v in p.items()}) for c, p in steps]


call("ui.resize", {"width": 1500, "height": 940})
call("ui.home", {"open": False})
chat({"refresh": True})
# The built-in agent (`mastra`) is carried, not found on the computer.
wait(lambda: any((a.get("found") or agent == "mastra") and a.get("id") == agent for a in (chat().get("agents") or [])), 90, 0.5)
for params in ({"open": True}, {"permissions": "edits"}, {"agent": agent, "default_agent": agent}):
    chat(params)
time.sleep(1)
chat({"connect": True})
s = wait(lambda: (lambda c: c if c.get("state") == "ready" and not c.get("pending") else None)(chat()), 180, 0.5) or {}
note(kind="setup", state=s.get("state"), agent=s.get("currentAgent"), model=s.get("model"), auth=s.get("authStatus"))
call("ui.prefs", {"agent_next_step_per_hour": 500})
chat({"next_step": True, "next_step_pause_s": 1.5})
time.sleep(6)

DESIGNS = {
    "case": [
        [("solid.box", {"length": 60, "width": 40, "height": 25, "corner": [0, 0, 0], "body_name": "Case", "name": "Case"})],
        [("solid.fillet", {"edges": [[0, 0, 12.5], [60, 0, 12.5], [60, 40, 12.5], [0, 40, 12.5]], "radius": 3, "body": "Case"})],
        [("solid.shell", {"faces": [[30, 20, 25]], "thickness": 2, "body": "Case"})],
    ],
    "bracket": [
        [("solid.box", {"length": 80, "width": 50, "height": 10, "corner": [0, 0, 0], "body_name": "Base", "name": "Base"}),
         ("solid.fillet", {"edges": [[0, 0, 5], [80, 0, 5], [80, 50, 5], [0, 50, 5]], "radius": 6, "body": "Base"})],
        [("solid.box", {"length": 20, "width": 20, "height": 20, "corner": [30, 15, 10], "body_name": "Boss", "name": "Boss", "operation": "new"})],
    ],
    "plate": [
        [("solid.box", {"length": 100, "width": 60, "height": 6, "corner": [0, 0, 0], "body_name": "Plate", "name": "Plate"}),
         ("solid.fillet", {"edges": [[0, 0, 3], [100, 0, 3], [100, 60, 3], [0, 60, 3]], "radius": 5, "body": "Plate"})],
        [("solid.cylinder", {"radius": 2.5, "height": 8, "base": [10, 10, -1], "operation": "cut", "targets": ["Plate"], "name": "Hole"})],
    ],
    "flange": [
        [("solid.cylinder", {"diameter": 90, "height": 10, "base": [0, 0, 0], "operation": "new", "body_name": "Flange", "name": "Flange"}),
         ("solid.cylinder", {"diameter": 40, "height": 30, "base": [0, 0, 10], "operation": "join", "targets": ["Flange"], "name": "Hub"})],
        [("solid.hole", {"position": [0, 0, 40], "diameter": 25, "name": "Bore"}),
         ("solid.hole", {"position": [33, 0, 10], "diameter": 6.6, "name": "BoltHole"})],
    ],
    # M10: one suggestion each. The plate with its hole, and one whose fillets and hole have no number
    # in common (r 4; r 3, so 6 across).
    "holed": [
        [("solid.box", {"length": 100, "width": 60, "height": 6, "corner": [0, 0, 0], "body_name": "Plate", "name": "Plate"}),
         ("solid.fillet", {"edges": [[0, 0, 3], [100, 0, 3], [100, 60, 3], [0, 60, 3]], "radius": 5, "body": "Plate"}),
         ("solid.cylinder", {"radius": 2.5, "height": 8, "base": [10, 10, -1], "operation": "cut", "targets": ["Plate"], "name": "Hole"})],
    ],
    "holed2": [
        [("solid.box", {"length": 100, "width": 60, "height": 6, "corner": [0, 0, 0], "body_name": "Plate", "name": "Plate"}),
         ("solid.fillet", {"edges": [[0, 0, 3], [100, 0, 3], [100, 60, 3], [0, 60, 3]], "radius": 4, "body": "Plate"}),
         ("solid.cylinder", {"radius": 3, "height": 8, "base": [10, 10, -1], "operation": "cut", "targets": ["Plate"], "name": "Hole"})],
    ],
    # M8: sheet-metal parts (a build with ghost::SHEET_METAL on).
    "sheet1": [
        [("sketch.create", {"plane": "XY", "name": "Base"}), ("sketch.rectangle.two_point", {"p0": [0, 0], "p1": [100, 60]}), ("sketch.finish", {}),
         ("sheet.flange", {"sketch": "Base"})],
        [("sheet.flange", {"edges": [[50, 0, 2.5]], "height": 20})],
    ],
    "sheet2": [
        [("sketch.create", {"plane": "XY", "name": "Base"}), ("sketch.rectangle.two_point", {"p0": [0, 0], "p1": [80, 80]}), ("sketch.finish", {}),
         ("sheet.flange", {"sketch": "Base"}), ("sheet.flange", {"edges": [[40, 0, 2.5]], "height": 25})],
        [("sheet.flange", {"edges": [[40, 80, 2.5]], "height": 25})],
    ],
    "sheet3": [
        [("sketch.create", {"plane": "XY", "name": "Base"}), ("sketch.rectangle.two_point", {"p0": [0, 0], "p1": [120, 50]}), ("sketch.finish", {}),
         ("sheet.flange", {"sketch": "Base"}), ("sheet.flange", {"edges": [[0, 25, 2.5]], "height": 30})],
        [("sheet.flange", {"edges": [[120, 25, 2.5]], "height": 30})],
        [("solid.hole", {"position": [60, 25, 2.5], "diameter": 8, "name": "Hole"})],
    ],
}
# M11: in-case, in-bracket, in-plate, in-flange.
DESIGNS.update({f"in-{k}": [inches(s) for s in DESIGNS[k]] for k in ("case", "bracket", "plate", "flange")})
if design.startswith("in-"):
    ex("document.units", {"units": "in"})
LIVE = {"asking", "shown"}
for i, steps in enumerate(DESIGNS[design], 1):
    time.sleep(float(os.environ.get("NSM_WAIT", "0")))  # as a person works between suggestions
    before = len(next_step().get("log") or [])
    for command, params in steps:
        ex(command, params)
    rev = revision()

    def settled():
        s = next_step()
        log = s.get("log") or []
        return s if len(log) > before and log[-1]["outcome"] not in LIVE else None

    s = wait(settled, 120)
    if not s:
        note(kind="timeout", what=f"{design}-{i}", nextStep=next_step())
        continue
    g = s["log"][-1]
    row = {"kind": "guess", "what": f"{design}-{i}", **g, "ghost": s.get("ghost"), "guessed": (s.get("chip") or {}).get("text"),
           "keySignIn": s.get("keySignIn"), "stampHeld": revision() == rev, "part": boxes()}
    if TAB and s.get("ghost"):
        shown = wait(lambda: (next_step().get("ghost") or {}).get("onScreen"), 5)
        t0 = time.time()
        call("ui.key", {"key": "Tab"})
        done = wait(lambda: next((l for l in next_step().get("log", []) if l["id"] == g["id"] and l["outcome"] == "accepted"), None), 20, 0.01)
        row["tab_ms"] = round((time.time() - t0) * 1000) if done else None
        row["on_screen"] = bool(shown)
        ghost = {b["name"]: (b["volume"], b["faces"]) for b in s["ghost"]["bodies"]}
        now = bodies()
        row["landed"] = {"ghost": ghost, "now": now, "box": boxes()}
        row["equal"] = set(now) == set(ghost) and all(
            now[k][1] == ghost[k][1] and ghost[k][0] and now[k][0] and abs(now[k][0] - ghost[k][0]) <= 1e-6 * abs(ghost[k][0]) for k in ghost)
        ex("edit.undo")
    note(**row)
chat({"next_step": False})
note(kind="done", what=design, lines=next_step().get("lines"), withdrawn=next_step().get("withdrawn"))
