#!/usr/bin/env python3
"""next_step_bracket.py PORT TOKEN OUTDIR AGENT [PLAN]: one take of the next-step demo on a NEMA 17
motor bracket, mixing the person's own modelling (through the app's UI) with suggestions taken
by Tab. Recorded from X display $DISPLAY.

AGENT: `claude` (Claude Code through claude-agent-acp) or a scripted ACP agent's path.
PLAN: comma-separated beats, in order:
  hand:base     sketch an 80 × 60 rectangle on XY and extrude it 8 mm (the base plate)
  hand:upright  sketch on the base's top face along its back edge and extrude it 60 mm (the upright)
  hand:fillet   fillet the inside corner between base and upright, 3 mm
  hand:hole     drill one Ø5.5 mm hole near the base's front left corner
  hand:cable    drill a Ø8 mm cable hole through the upright, off to the right of the motor
  ignore        let a suggestion show, read it, and carry on with the next hand step
  accept        Tab, then Tab on its preview
  dismiss       Esc on the suggestion
Default: hand:base,hand:upright,ignore,hand:fillet,accept,accept,accept,hand:cable,accept,accept.
PROJECT_TEMPLATE: a project folder to save the design into (its .intcad/ brief reaches the agent).

Writes OUTDIR/raw.mkv (MJPEG 1920×1080, 30 fps), OUTDIR/events.jsonl (each beat with `video_s`)
and OUTDIR/take.json (the app's own next-step log)."""
import json
import math
import os
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hand_steps import Hand  # noqa: E402

port, token, out, agent = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
plan = (sys.argv[5] if len(sys.argv) > 5 else "hand:base,hand:upright,ignore,hand:fillet,accept,accept,accept,hand:cable,accept,accept").split(",")
PAUSE_S = float(os.environ.get("NEXT_STEP_PAUSE", "3"))
READ_S = float(os.environ.get("READ_S", "2.6"))
os.makedirs(out, exist_ok=True)
sock = socket.create_connection(("127.0.0.1", port))
f = sock.makefile("rw")
n = 0
rec = None
rec_t0 = None
events = open(os.path.join(out, "events.jsonl"), "w")
lock = threading.Lock()


def call(method, params=None):
    global n
    with lock:
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


def note(kind, **kw):
    now = time.time()
    e = {"t": round(now, 3), "video_s": round(now - rec_t0, 2) if rec_t0 else None, "kind": kind, **kw}
    events.write(json.dumps(e, ensure_ascii=False) + "\n")
    events.flush()
    print(json.dumps(e, ensure_ascii=False), flush=True)


def wait(pred, what, secs):
    end = time.time() + secs
    while time.time() < end:
        s = chat()
        if s.get("waiting"):
            chat({"approve": True})
            note("approved", what=what, waiting=s.get("waiting"))
            continue
        if pred(s):
            return s
        time.sleep(0.15)
    note("timeout", what=what, nextStep=(chat().get("nextStep") or {}).get("state"))
    return None


def settled(s):
    return s.get("state") == "ready" and not s.get("pending")


def card_waiting(s):
    return s.get("state") == "ready" and (s.get("preview") or {}).get("phase") == "Review"


def log(s):
    return s["nextStep"]["log"]


def revision():
    return (call("document.revision").get("result") or {}).get("revision")


def key(k):
    call("ui.key", {"key": k})
    note("key", key=k)


# -- camera ---------------------------------------------------------------------------------------
CAMERA = None
CENTRE = (0.0, 0.0)


def camera():
    return call("ui.camera", {}).get("result") or {}


def frame_for_final_size():
    """Fit a stand-in as big as the finished bracket, keep that camera nudged above the card,
    and undo the stand-in. Hand steps that turn the view (a new sketch) ease back to it."""
    global CAMERA
    call("ui.prefs", {"home_fit_size": float(os.environ.get("FIT_SIZE", "0.6"))})
    call("engine.execute", {"command": "view.home", "params": {}})
    call("engine.execute", {"command": "solid.box", "params": {"length": 80, "width": 60, "height": 68, "corner": [-40, -30, 0], "body_name": "StandIn"}})
    call("engine.execute", {"command": "view.fit", "params": {}})
    time.sleep(1.5)
    cam = camera()
    target = cam.get("target") or [0, 0, 34]
    CAMERA = {"target": [target[0], target[1], target[2] - float(os.environ.get("NUDGE", "9"))], "yaw": cam.get("yaw"),
              "pitch": float(os.environ.get("PITCH", "0.62")), "distance": cam.get("distance")}
    call("engine.execute", {"command": "edit.undo", "params": {}})
    call("ui.camera", CAMERA)
    note("setup", what="camera", camera=CAMERA)


def ease_to(goal, dur=0.9):
    """Move the camera to `goal` smoothly (yaw by the short way round)."""
    start = camera()
    if not start.get("target") or not goal:
        return
    dyaw = (goal["yaw"] - start["yaw"] + math.pi) % (2 * math.pi) - math.pi
    t0 = time.time()
    while True:
        u = min(1.0, (time.time() - t0) / dur)
        e = u * u * (3 - 2 * u)
        call("ui.camera", {
            "target": [a + (b - a) * e for a, b in zip(start["target"], goal["target"])],
            "yaw": start["yaw"] + dyaw * e,
            "pitch": start["pitch"] + (goal["pitch"] - start["pitch"]) * e,
            "distance": start["distance"] + (goal["distance"] - start["distance"]) * e,
        })
        if u >= 1.0:
            return
        time.sleep(1 / 30)


class Sway(threading.Thread):
    """A slow sway of the camera about the take's view while the agent works (never drifting)."""

    def __init__(self):
        super().__init__(daemon=True)
        self.on = threading.Event()

    def run(self):
        t0 = time.time()
        while True:
            if self.on.is_set() and CAMERA:
                t = time.time() - t0
                call("ui.camera", {"yaw": CAMERA["yaw"] + 0.10 * math.sin(2 * math.pi * t / 16.0),
                                   "pitch": CAMERA["pitch"] + 0.03 * math.sin(2 * math.pi * t / 23.0)})
            time.sleep(1 / 30)


sway = Sway()


def sway_on(on):
    if on:
        ease_to(CAMERA, 0.6)
        sway.on.set()
    else:
        sway.on.clear()
        time.sleep(0.1)


# -- recording ------------------------------------------------------------------------------------
def start_recording():
    global rec, rec_t0
    path = os.path.join(out, "raw.mkv")
    disp = os.environ.get("DISPLAY", ":21")
    pipeline = (f"ximagesrc display-name={disp} use-damage=false show-pointer=false ! video/x-raw,framerate=30/1 ! "
                f"queue max-size-buffers=0 max-size-time=0 max-size-bytes=0 ! videoconvert ! "
                f"jpegenc quality=92 ! matroskamux ! filesink location={path}")
    rec = subprocess.Popen(["gst-launch-1.0", "-e", *pipeline.split()], stdout=open(os.path.join(out, "gst.log"), "w"), stderr=subprocess.STDOUT)
    time.sleep(1.0)
    rec_t0 = time.time() - 1.0
    note("recording", path=path, display=disp)


def stop_recording():
    if rec:
        rec.send_signal(signal.SIGINT)
        try:
            rec.wait(timeout=60)
        except subprocess.TimeoutExpired:
            rec.kill()
        note("recording-stopped", code=rec.returncode)


# -- the agent ------------------------------------------------------------------------------------
def connect():
    if agent == "claude":
        chat({"refresh": True})
        end = time.time() + 90
        while not any(a.get("found") and a.get("id") == "claude" for a in (chat().get("agents") or [])) and time.time() < end:
            time.sleep(0.5)
        for params in ({"open": True, "width": 420}, {"permissions": "edits"}, {"preview_changes": True}, {"agent": "claude", "default_agent": "claude"}):
            chat(params)
        time.sleep(1)
        chat({"connect": True})
    else:
        chat({"add_agent": {"id": "nextstep", "name": "Scripted next-step agent", "kind": "acp", "command": agent},
              "agent": "nextstep", "open": True, "connect": True, "width": 420, "preview_changes": True})
    s = wait(settled, "the agent to be ready", 180)
    note("setup", what="connected", state=(s or {}).get("state"), agent=(s or {}).get("currentAgent"))
    if not s:
        raise SystemExit("no agent")


def await_guess(label, since):
    count = len(log(chat()))
    end = time.time() + 240
    while time.time() < end:
        s = chat()
        if s["nextStep"]["state"] == "shown" or (len(log(s)) > count and log(s)[-1]["outcome"] not in ("asking",)):
            break
        time.sleep(0.1)
    else:
        note("timeout", what=f"{label} guess")
        return None
    s = chat()
    last = log(s)[-1]
    shown = s["nextStep"].get("shown") or {}
    note("guess", label=label, outcome=last["outcome"], step=last["step"], reason=last["reason"], memory=shown.get("memory"),
         sources=shown.get("sources"), ms=last["ms"], pause_to_card_s=round(time.time() - since, 2), cost=last["cost"])
    return last


def tab_preview(label, decide):
    before = revision()
    t0 = time.time()
    key("tab")
    wait(lambda s: s["state"] == "busy" or s.get("preview"), "the turn to start", 30)
    note("building", label=label)
    s = wait(lambda s: card_waiting(s) or (settled(s) and not s.get("preview")), "the preview card", 420)
    if not s:
        return None
    turn_s = round(time.time() - t0, 1)
    if not card_waiting(s):
        note("no-change", label=label, turn_s=turn_s)
        return s
    note("ready", label=label, turn_s=turn_s, revision_before=before, card=[i for i in s["items"] if "review" in i][-1:])
    time.sleep(READ_S + 0.6)
    key("tab" if decide == "accept" else "escape")
    s = wait(lambda s: settled(s) and not s.get("preview"), "the decision", 120)
    note("decided", label=label, decision=decide, revision_after=revision())
    time.sleep(0.4)
    return s


# -- the person's steps ---------------------------------------------------------------------------
hand = Hand(call, note)


def after_sketch():
    hand.finish_sketch()
    ease_to(CAMERA, 0.9)


HAND = {
    "base": lambda: (hand.sketch_on(plane="XY"), hand.centre_rectangle([0, 0], [40, 30]), after_sketch(), hand.extrude("8")),
    "upright": lambda: (hand.sketch_on(world=[0, 0, 8]), hand.centre_rectangle([0, 27, 8], [40, 30, 8], space="world"), after_sketch(),
                        hand.extrude("60", profile=[0, 27, 8])),
    "fillet": lambda: hand.fillet_edges([[0, 24, 8]], "3"),
    "hole": lambda: hand.hole_at([-32, -22, 8], "5.5"),
    "cable": lambda: hand.hole_at([28, 24, 20], "8"),
}


def do_hand(name):
    sway_on(False)
    note("hand", step=name)
    try:
        HAND[name]()
        note("hand-done", step=name, revision=revision())
    except Exception as e:  # noqa: BLE001
        note("hand-failed", step=name, error=str(e)[:300])
        call("ui.key", {"key": "Escape"})
    ease_to(CAMERA, 0.6)


# -- setup (not recorded) -------------------------------------------------------------------------
call("ui.resize", {"width": 1920, "height": 1080})
call("ui.home", {"open": False})
call("ui.prefs", {"theme": os.environ.get("THEME", "black")})
connect()
chat({"send": "I'm making a bracket that holds a NEMA 17 motor on the arm's 2020 frame. I'll model it myself; suggest as I go."})
wait(settled, "the chat reply", 240)
chat({"open": False})
frame_for_final_size()
if os.environ.get("PROJECT_TEMPLATE"):
    project_dir = os.path.join("/tmp/sc-demo-project", os.environ.get("PROJECT_NAME", "Arm motor brackets"))
    shutil.rmtree(project_dir, ignore_errors=True)
    shutil.copytree(os.environ["PROJECT_TEMPLATE"], project_dir)
    r = call("engine.execute", {"command": "file.save_as", "params": {"path": os.path.join(project_dir, "bracket.sc")}})
    note("setup", what="saved into project", path=project_dir, error=r.get("error"))
    r = call("ui.agent_chat", {"trust_project": True})
    note("setup", what="trusted project", error=r.get("error"))
time.sleep(1.0)

# -- the take -------------------------------------------------------------------------------------
start_recording()
sway.start()
time.sleep(2.0)
chat({"next_step": True, "next_step_pause_s": PAUSE_S})
since = time.time()
queue = list(plan)
i = 0
while queue:
    beat = queue.pop(0)
    i += 1
    label = f"beat-{i}"
    if beat.startswith("hand:"):
        do_hand(beat.split(":", 1)[1])
        since = time.time()
        continue
    sway_on(True)
    g = await_guess(label, since)
    if not g or g["outcome"] != "shown":
        nxt = next((b for b in queue if b.startswith("hand:")), None)
        note("skip", why=f"no suggestion ({(g or {}).get('outcome')})", to=nxt)
        if nxt:
            queue = queue[queue.index(nxt):]
            continue
        break
    time.sleep(READ_S)
    if beat == "ignore":
        note("ignored", label=label)
        continue
    if beat == "dismiss":
        key("escape")
        time.sleep(1.2)
        continue
    s = tab_preview(label, beat)
    last = log(chat())[-1]["outcome"] if s else None
    if s is None or last in ("no change", "rejected"):
        nxt = next((b for b in queue if b.startswith("hand:")), None)
        note("skip", why=f"after Tab: {last}", to=nxt)
        if nxt:
            queue = queue[queue.index(nxt):]
            continue
        break
    since = time.time()

# -- closing ----------------------------------------------------------------------------------------
time.sleep(2.0)
chat({"next_step": False})
sway_on(False)
ease_to(CAMERA, 0.6)
note("closing", what="suggestions off, turntable")
t0 = time.time()
TURN_S = 9.0
while time.time() - t0 < TURN_S:
    u = (time.time() - t0) / TURN_S
    a = math.pi * (u * u * (3 - 2 * u))
    cx, cy = CENTRE
    tx, ty, tz = CAMERA["target"]
    rx = cx + (tx - cx) * math.cos(-a) - (ty - cy) * math.sin(-a)
    ry = cy + (tx - cx) * math.sin(-a) + (ty - cy) * math.cos(-a)
    call("ui.camera", {"yaw": CAMERA["yaw"] + a, "pitch": CAMERA["pitch"], "target": [rx, ry, tz]})
    time.sleep(1 / 30)
time.sleep(1.5)
stop_recording()
s = chat()
with open(os.path.join(out, "take.json"), "w") as fh:
    json.dump({"plan": plan, "agent": agent, "pause_s": PAUSE_S, "nextStep": s.get("nextStep")}, fh, indent=1, ensure_ascii=False)
call("engine.execute", {"command": "file.save_as", "params": {"path": os.path.join(out, "bracket.sc")}})
note("done")
