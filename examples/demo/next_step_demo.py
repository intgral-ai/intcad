#!/usr/bin/env python3
"""next_step_demo.py PORT TOKEN OUTDIR AGENT [PLAN]: one take of the next-step autocomplete demo
(INT-1458) in a running Intcad, driven over the control channel, recorded from X display $DISPLAY.

AGENT: `claude` (Claude Code through claude-agent-acp, found on PATH) or the path of a scripted
ACP agent (target/ci/examples/acp_next_step_agent) for rehearsals.
PLAN: comma-separated decisions for each suggestion, in order: `accept` (Tab, then Tab on the
preview), `reject` (Tab, then Esc on the preview), `dismiss` (Esc on the suggestion) or `lid`
(the user adds a lid by hand: a second body above the case). Default:
accept,accept,accept,dismiss,lid,accept,accept,accept.

The story: an enclosure for a 50 x 30 mm board. The user says so in the chat, makes the case's
block, and from then on mostly presses Tab. Setup (agent connection, the chat note) happens before
the recording starts. Everything shown is the app's own UI; nothing is composited afterwards.

Writes OUTDIR/raw.mkv (MJPEG, 1920x1080, 30 fps; GStreamer ximagesrc), OUTDIR/events.jsonl (one
line per beat, with wall-clock `t` and `video_s` from the recording's start, for the edit) and
OUTDIR/take.json (the app's own next-step log at the end: outcomes, times, tokens, cost).
"""
import json
import math
import os
import signal
import socket
import subprocess
import sys
import threading
import time

port, token, out, agent = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
plan = (sys.argv[5] if len(sys.argv) > 5 else "accept,accept,accept,dismiss,lid,accept,accept,accept").split(",")
PAUSE_S = float(os.environ.get("NEXT_STEP_PAUSE", "3"))
READ_S = float(os.environ.get("READ_S", "2.6"))  # time to read a card before the key
os.makedirs(out, exist_ok=True)
sock = socket.create_connection(("127.0.0.1", port))
f = sock.makefile("rw")
n = 0
rec = None
rec_t0 = None
events = open(os.path.join(out, "events.jsonl"), "w")


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


def note(kind, **kw):
    now = time.time()
    e = {"t": round(now, 3), "video_s": round(now - rec_t0, 2) if rec_t0 else None, "kind": kind, **kw}
    events.write(json.dumps(e, ensure_ascii=False) + "\n")
    events.flush()
    print(json.dumps(e, ensure_ascii=False), flush=True)


def ex(command, params):
    r = call("engine.execute", {"command": command, "params": params})
    ok = "error" not in r and r.get("ok", True) is not False
    note("user", command=command, params=params, ok=ok, error=r.get("error"))
    return r


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


def spin(on):
    call("engine.execute", {"command": "view.spin", "params": {"on": on}})


CAMERA = None


def frame_for_final_size():
    """Before the take: fit a stand-in as big as the finished enclosure (case and lid), keep that
    camera with the model nudged up above the card, and undo the stand-in. The take never refits,
    so the view holds still while the part grows."""
    global CAMERA
    call("ui.prefs", {"home_fit_size": float(os.environ.get("FIT_SIZE", "0.62"))})
    call("engine.execute", {"command": "view.home", "params": {}})
    ex("solid.box", {"length": 60, "width": 40, "height": 36, "corner": [0, 0, 0], "body_name": "StandIn"})
    call("engine.execute", {"command": "view.fit", "params": {}})
    time.sleep(1.5)
    cam = call("ui.camera", {}).get("result") or {}
    target = cam.get("target") or [30, 20, 18]
    pitch = float(os.environ.get("PITCH", "0.8"))
    CAMERA = {"target": [target[0], target[1], target[2] - float(os.environ.get("NUDGE", "7"))], "yaw": cam.get("yaw"),
              "pitch": pitch, "distance": cam.get("distance")}
    call("engine.execute", {"command": "edit.undo", "params": {}})
    call("ui.camera", CAMERA)
    note("setup", what="camera", camera=CAMERA, revision=revision())


def fit():
    """Back to the take's camera (after a spin)."""
    if CAMERA:
        call("ui.camera", CAMERA)


class Sway(threading.Thread):
    """A slow sway of the camera about the take's view (a few degrees, never drifting), on its own
    control connection, so the model stays alive while the agent works. Moving the camera is
    looking, not acting: it neither dismisses a suggestion nor asks a new one."""

    def __init__(self):
        super().__init__(daemon=True)
        self.on = threading.Event()
        self.stop = False

    def run(self):
        s = socket.create_connection(("127.0.0.1", port))
        fh = s.makefile("rw")
        i = 0
        t0 = time.time()
        while not self.stop:
            if self.on.is_set() and CAMERA and CAMERA.get("yaw") is not None:
                t = time.time() - t0
                i += 1
                req = {"id": i, "method": "ui.camera", "params": {
                    "yaw": CAMERA["yaw"] + 0.11 * math.sin(2 * math.pi * t / 16.0),
                    "pitch": CAMERA["pitch"] + 0.035 * math.sin(2 * math.pi * t / 23.0)}}
                if i == 1:
                    req["token"] = token
                fh.write(json.dumps(req) + "\n")
                fh.flush()
                fh.readline()
            time.sleep(1 / 30)


sway = Sway()


def key(k):
    call("ui.key", {"key": k})
    note("key", key=k)


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
    """Spin while the agent thinks; stop when its suggestion shows."""
    count = len(log(chat()))
    spun = False
    end = time.time() + 240
    while time.time() < end:
        s = chat()
        st = s["nextStep"]["state"]
        if st == "asking" and not spun:
            spun = True
            note("thinking", label=label, since_edit_s=round(time.time() - since, 2))
        if st == "shown" or (len(log(s)) > count and log(s)[-1]["outcome"] not in ("asking",)):
            break
        time.sleep(0.1)
    else:
        note("timeout", what=f"{label} guess")
        return None
    s = chat()
    last = log(s)[-1]
    note("guess", label=label, outcome=last["outcome"], step=last["step"], reason=last["reason"],
         sources=(s["nextStep"].get("shown") or {}).get("sources"), ms=last["ms"], pause_to_card_s=round(time.time() - since, 2),
         input_tokens=last["inputTokens"], output_tokens=last["outputTokens"], context_tokens=last["contextTokens"], cost=last["cost"])
    return last


def tab_preview(label, decide):
    """Tab: the step goes to the agent and runs on a copy; its preview card waits; Tab or Esc."""
    before = revision()
    t0 = time.time()
    key("tab")
    # The turn has begun once the chat is busy or a preview exists (the guess leaves the chip first).
    wait(lambda s: s["state"] == "busy" or s.get("preview"), "the turn to start", 30)
    note("building", label=label)
    s = wait(lambda s: card_waiting(s) or (settled(s) and not s.get("preview")), "the preview card", 420)
    if not s:
        return None
    turn_s = round(time.time() - t0, 1)
    if not card_waiting(s):
        note("no-change", label=label, turn_s=turn_s)
        return s
    card = [i for i in s["items"] if "review" in i][-1:]
    note("ready", label=label, turn_s=turn_s, revision_before=before, revision_while_previewed=revision(), card=card)
    time.sleep(READ_S + 0.6)
    key("tab" if decide == "accept" else "escape")
    s = wait(lambda s: settled(s) and not s.get("preview"), "the decision", 120)
    note("decided", label=label, decision=decide, revision_after=revision())
    time.sleep(0.4)
    return s


# -- setup (not recorded) ------------------------------------------------------------------------
call("ui.resize", {"width": 1920, "height": 1080})
call("ui.home", {"open": False})
r = call("ui.prefs", {"theme": os.environ.get("THEME", "black")})
note("setup", what="theme", error=r.get("error"))
connect()
chat({"send": "I'm making a small enclosure for a 50 x 30 mm circuit board. I'll model it myself; suggest as I go."})
wait(settled, "the chat reply", 240)
chat({"open": False})
frame_for_final_size()
# With PROJECT_TEMPLATE: the design is saved inside a copy of that project folder (its .intcad/
# with the project's settings, intent and know-how), and the user trusts the project, so the
# guesses carry its brief. Without it, the design stays loose and unsaved (the baseline).
if os.environ.get("PROJECT_TEMPLATE"):
    import shutil
    project_dir = os.path.join("/tmp/sc-demo-project", os.environ.get("PROJECT_NAME", "Sensor enclosures"))
    shutil.rmtree(project_dir, ignore_errors=True)
    shutil.copytree(os.environ["PROJECT_TEMPLATE"], project_dir)
    r = call("engine.execute", {"command": "file.save_as", "params": {"path": os.path.join(project_dir, "enclosure.sc")}})
    note("setup", what="saved into project", path=project_dir, error=r.get("error"))
    r = call("ui.agent_chat", {"trust_project": True})
    note("setup", what="trusted project", error=r.get("error"))
time.sleep(1.5)

# -- the take ------------------------------------------------------------------------------------
start_recording()
time.sleep(2.5)
# Suggestions on just before the first edit, so nothing is guessed about the empty design.
chat({"next_step": True, "next_step_pause_s": PAUSE_S})
ex("solid.box", {"length": 60, "width": 40, "height": 25, "corner": [0, 0, 0], "body_name": "Case"})
sway.start()
sway.on.set()
since = time.time()
queue = list(plan)
i = 0


def jump_to_lid(why):
    """Nothing more will be asked about this design: carry on at the user's next hand step."""
    global queue
    if "lid" in queue:
        note("skip", why=why, skipped=queue[:queue.index("lid")])
        queue = queue[queue.index("lid"):]
        return True
    note("stop", why=why)
    return False


while queue:
    decide = queue.pop(0)
    i += 1
    label = f"step-{i}"
    if decide == "lid":
        time.sleep(1.2)
        ex("solid.box", {"length": 60, "width": 40, "height": 4, "corner": [0, 0, 32], "body_name": "Lid", "operation": "new"})
        since = time.time()
        continue
    g = await_guess(label, since)
    if not g or g["outcome"] != "shown":
        if jump_to_lid(f"no suggestion ({(g or {}).get('outcome')})"):
            continue
        break
    time.sleep(READ_S)
    if decide == "dismiss":
        key("escape")
        time.sleep(1.5)
        # Turned down: nothing new is asked until the design changes.
        if queue and queue[0] != "lid" and not jump_to_lid("dismissed"):
            break
        since = time.time()
        continue
    s = tab_preview(label, decide)
    if s is None:
        if jump_to_lid("no preview"):
            continue
        break
    if decide == "reject" or not card_waiting(s) and revision() is not None and (log(chat())[-1]["outcome"] in ("no change", "rejected")):
        if jump_to_lid(log(chat())[-1]["outcome"]):
            continue
        break
    since = time.time()
time.sleep(2.5)
chat({"next_step": False})
note("closing", what="suggestions off, turntable")
sway.on.clear()
time.sleep(0.6)
t0 = time.time()
TURN_S = 9.0
while time.time() - t0 < TURN_S:
    u = (time.time() - t0) / TURN_S
    ease = u * u * (3 - 2 * u)
    # Turn the whole rig (camera and its target) about the part's vertical centre line, so the
    # part turns in place instead of swinging round the off-centre fit target.
    a = math.pi * ease
    sign = float(os.environ.get("TURN_SIGN", "-1"))
    cx, cy = 30.0, 20.0
    tx, ty, tz = CAMERA["target"]
    rx = cx + (tx - cx) * math.cos(sign * a) - (ty - cy) * math.sin(sign * a)
    ry = cy + (tx - cx) * math.sin(sign * a) + (ty - cy) * math.cos(sign * a)
    call("ui.camera", {"yaw": CAMERA["yaw"] + a, "pitch": CAMERA["pitch"], "target": [rx, ry, tz]})
    time.sleep(1 / 30)
time.sleep(1.5)
stop_recording()
s = chat()
with open(os.path.join(out, "take.json"), "w") as fh:
    json.dump({"plan": plan, "agent": agent, "pause_s": PAUSE_S, "nextStep": s.get("nextStep")}, fh, indent=1, ensure_ascii=False)
call("engine.execute", {"command": "file.save_as", "params": {"path": os.path.join(out, "enclosure.sc")}})
note("done")
