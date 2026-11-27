#!/usr/bin/env python3
"""Next-step guesses, one design per run, for comparing the model the guesses run on
(preference `agent_next_step_model`: `same`, empty for a small model, or an id). The agent is the
built-in one, or Claude Code (through claude-agent-acp on PATH) with NSM_AGENT=claude.
The design is built by hand steps only (no Tab), so the only agent calls are the guesses: each
state below is one edit, then the pause, then one guess, logged with its model, time, tokens and
cost as the agent reported them.

Usage: next_step_ab.py PORT TOKEN OUTDIR DESIGN MODEL
  DESIGN: case, bracket, plate.   MODEL: same | auto | <model id>   (auto = empty preference)
Also: next_step_ab.py PORT TOKEN OUTDIR status   (what the Connections page says, nothing else)
"""
import json, os, socket, sys, time

port, token, out, design = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
model = sys.argv[5] if len(sys.argv) > 5 else "auto"
agent = os.environ.get("NSM_AGENT", "builtin")
PAUSE_S = 3
os.makedirs(out, exist_ok=True)
sock = socket.create_connection(("127.0.0.1", port))
f = sock.makefile("rw")
n = 0
record = {"agent": agent, "design": design, "model": model, "pause_s": PAUSE_S, "events": []}


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


def note(kind, what, **kw):
    e = {"t": round(time.time(), 2), "kind": kind, "what": what, **kw}
    record["events"].append(e)
    print(json.dumps(e, ensure_ascii=False), flush=True)
    with open(os.path.join(out, f"{agent}-{design}-{model.replace('/', '_')}.json"), "w") as fh:
        json.dump(record, fh, indent=1, ensure_ascii=False)


def wait(pred, what, secs):
    end = time.time() + secs
    while time.time() < end:
        s = chat()
        if pred(s):
            return s
        time.sleep(0.25)
    note("timeout", what, nextStep=(chat().get("nextStep") or {}).get("state"))
    return None


def ex(command, params):
    r = call("engine.execute", {"command": command, "params": params})
    note("user", command, ok="error" not in r, error=r.get("error"))


def connected(s):
    return bool((s.get("connect") or {}).get("connected"))


call("ui.resize", {"width": 1500, "height": 940})
call("ui.home", {"open": False})
if agent == "claude":
    chat({"refresh": True})
    wait(lambda s: any(a.get("found") and a.get("id") == "claude" for a in (s.get("agents") or [])),
         "Claude Code on PATH", 90)
    for params in ({"open": True}, {"permissions": "edits"}, {"agent": "claude", "default_agent": "claude"}):
        chat(params)
    time.sleep(1)
    chat({"connect": True})
else:
    chat({"open": True})
    s = wait(lambda s: s.get("connect") is not None, "the built-in agent's connections", 60)
    if design == "status":
        print(json.dumps((s or {}).get("connect"), indent=1, ensure_ascii=False))
        raise SystemExit(0)
    if not s or not connected(s):
        note("setup", "not connected: sign in on the Connections page first", connect=(s or {}).get("connect"))
        raise SystemExit(2)
s = wait(lambda s: s.get("state") == "ready" and not s.get("pending"), "the chat's agent to be ready", 180)
note("setup", "connected", state=(s or {}).get("state"), agent=(s or {}).get("currentAgent"),
     chat_model=(s or {}).get("model"), configOptions=(s or {}).get("configOptions"))
r = call("ui.prefs", {"agent_next_step_model": "" if model == "auto" else model})
note("prefs", "agent_next_step_model", error=r.get("error"))
chat({"next_step": True, "next_step_pause_s": PAUSE_S})

DESIGNS = {
    "case": [
        [("solid.box", {"length": 60, "width": 40, "height": 25, "corner": [0, 0, 0], "body_name": "Case"})],
        [("solid.fillet", {"edges": [[0, 0, 12.5], [60, 0, 12.5], [60, 40, 12.5], [0, 40, 12.5]], "radius": 3, "body": "Case"})],
    ],
    "bracket": [
        [("solid.box", {"length": 80, "width": 50, "height": 10, "corner": [0, 0, 0], "body_name": "Base"}),
         ("solid.fillet", {"edges": [[0, 0, 5], [80, 0, 5], [80, 50, 5], [0, 50, 5]], "radius": 6, "body": "Base"})],
        [("solid.box", {"length": 20, "width": 20, "height": 20, "corner": [30, 15, 10], "body_name": "Boss", "operation": "new"})],
    ],
    "plate": [
        [("solid.box", {"length": 100, "width": 60, "height": 6, "corner": [0, 0, 0], "body_name": "Plate"}),
         ("solid.fillet", {"edges": [[0, 0, 3], [100, 0, 3], [100, 60, 3], [0, 60, 3]], "radius": 5, "body": "Plate"})],
        [("solid.cylinder", {"radius": 2.5, "height": 8, "base": [10, 10, -1], "operation": "cut", "targets": ["Plate"], "name": "Hole"})],
    ],
}


def log_of(s):
    return (s.get("nextStep") or {}).get("log") or []


for i, steps in enumerate(DESIGNS[design], 1):
    before = len(log_of(chat()))
    for command, params in steps:
        ex(command, params)
    since = time.time()
    s = wait(lambda s: len(log_of(s)) > before and log_of(s)[-1]["outcome"] != "asking", f"guess {i}", 240)
    if not s:
        break
    g = log_of(s)[-1]
    note("guess", f"{design}-{i}", seconds_from_edit=round(time.time() - since, 2), **g)

s = chat()
record["nextStep"] = s.get("nextStep")
note("done", design)
