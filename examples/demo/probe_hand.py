#!/usr/bin/env python3
"""probe_hand.py PORT TOKEN OUT: try each bracket hand step in a running app and screenshot it."""
import json
import os
import socket
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hand_steps import Hand  # noqa: E402

port, token, out = int(sys.argv[1]), sys.argv[2], sys.argv[3]
os.makedirs(out, exist_ok=True)
sock = socket.create_connection(("127.0.0.1", port))
f = sock.makefile("rw")
n = 0


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


def note(kind, **kw):
    print(json.dumps({"t": round(time.time(), 2), "kind": kind, **kw}), flush=True)


def shot(name):
    os.system(f"xwd -root -silent | convert xwd:- {os.path.join(out, name)}")
    note("shot", name=name)


def state():
    i = call("ui.inspect").get("result") or {}
    return {"bodies": [b.get("name") for b in i.get("bodies", [])], "dialog": str(i.get("dialog"))[:300], "tool": i.get("tool")}


call("ui.resize", {"width": 1920, "height": 1080})
call("ui.home", {"open": False})
call("ui.prefs", {"theme": "black"})
time.sleep(1)
h = Hand(call, note)
steps = [
    ("base", lambda: (h.sketch_on(plane="XY"), h.centre_rectangle([0, 0], [35, 25]), h.finish_sketch(), h.home(), h.extrude("8"))),
    ("upright", lambda: (h.sketch_on(world=[0, 0, 8]), h.centre_rectangle([0, 22, 8], [35, 25, 8], space="world"), h.finish_sketch(), h.home(), h.extrude("60", profile=[0, 22, 8]))),
    ("hole", lambda: (h.hole_at([-27, -12, 8], "5.5"),)),
    ("fillet", lambda: (h.fillet_edges([[0, 19, 8]], "3"),)),
]
for name, run in steps:
    try:
        run()
        note("step", name=name, ok=True, state=state())
    except Exception as e:  # noqa: BLE001
        note("step", name=name, ok=False, error=str(e)[:400], state=state())
    shot(f"{name}.png")
    if "error" in json.dumps(state()).lower():
        h.key("Escape")
feats = call("engine.execute", {"command": "doc.inspect", "params": {}})
note("doc", reply=json.dumps(feats)[:1500])
