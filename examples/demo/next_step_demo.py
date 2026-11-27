#!/usr/bin/env python3
"""Record one ready ghost and its direct Tab acceptance on a case or a bracket."""

import json
import math
import os
from pathlib import Path
import shutil
import signal
import socket
import stat
import subprocess
import sys
import time


def stage_project(run_root, template):
    """Copy a template only to a new, fixed child of this take's output directory."""
    root = Path(run_root).resolve(strict=True)
    source = Path(template)
    if source.is_symlink():
        raise ValueError("project template must not be a symlink")
    source = source.resolve(strict=True)
    if root.is_relative_to(source):
        raise ValueError("the take must be outside the template")

    def reject_links(path, names):
        for name in names:
            mode = (Path(path) / name).lstat().st_mode
            if not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):
                raise ValueError(f"not a regular project file or directory: {name}")
        return ()

    target = root / "project"
    shutil.copytree(source, target, ignore=reject_links, symlinks=True)
    return target


def wait(check, what, seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if value := check():
            return value
        time.sleep(0.1)
    raise TimeoutError(what)


def inspect_bodies(call):
    reply = call("engine.execute", {"command": "inspect.measure", "params": {}})
    return {b["name"]: (b.get("volume_mm3"), (b.get("kernel") or {}).get("faces")) for b in reply["bodies"]}


def record_suggestion(call, wait_for=wait, note=lambda **kwargs: None):
    before = call("document.revision")
    undo = call("document.inspect")["undo"]
    bodies_before = inspect_bodies(call)

    def ready():
        state = call("ui.agent_chat")
        step = state.get("nextStep") or {}
        ghost = step.get("ghost") or {}
        log = step.get("log") or []
        if state["state"] == "ready" and not state.get("turn") and not state.get("preview") and ghost.get("onScreen") and log:
            if log[-1]["outcome"] == "ghost":
                return state, ghost, log[-1]
        return None

    state, ghost, guess = wait_for(ready, "no rendered ghost to accept", 120)
    expected = {b["name"]: (b["volume"], b["faces"]) for b in ghost["bodies"]}
    if call("document.revision") != before or call("document.inspect")["undo"] != undo:
        raise RuntimeError("design changed before acceptance")
    note(kind="ghost", id=guess["id"], step=guess["step"], bodies=expected)
    call("ui.key", {"key": "Tab"})

    def landed():
        current = call("ui.agent_chat")
        if current["state"] != "ready" or current.get("turn") or current.get("preview") or current["items"] != state["items"]:
            raise RuntimeError("Tab must not start a chat turn or produce a review")
        log = (current.get("nextStep") or {}).get("log") or []
        return next((row for row in log if row["id"] == guess["id"] and row["outcome"] == "accepted"), None)

    wait_for(landed, "rendered ghost did not land", 20)
    after = call("document.revision")
    undo_added = call("document.inspect")["undo"] - undo
    actual = inspect_bodies(call)
    if after["document_id"] != before["document_id"] or after["revision"] == before["revision"] or undo_added != 1 or actual == bodies_before:
        raise RuntimeError("acceptance must change the same design in exactly one undo step")
    if actual.keys() != expected.keys() or any(
        va is None or ve is None or fa is None or fa != fe or not math.isclose(va, ve, rel_tol=1e-6, abs_tol=1e-6)
        for (va, fa), (ve, fe) in ((actual[name], expected[name]) for name in expected)
    ):
        raise RuntimeError("the landed bodies do not match the rendered ghost")
    return {"id": guess["id"], "step": guess["step"], "before_revision": before["revision"], "after_revision": after["revision"],
            "undo_added": undo_added, "bodies": actual}


def record(port, token, root, design):
    template = os.environ.get("PROJECT_TEMPLATE")
    project = stage_project(root, template) if template else None
    path = project / "recorded-design.sc" if project else None
    if path is not None and path.exists():
        raise FileExistsError(path)
    with socket.create_connection(("127.0.0.1", port), timeout=10) as sock, sock.makefile("rw") as stream:
        n = 0

        def call(method, params=None):
            nonlocal n
            n += 1
            request = {"id": n, "method": method, "params": params or {}}
            if n == 1:
                request["token"] = token
            stream.write(json.dumps(request) + "\n")
            stream.flush()
            while line := stream.readline():
                reply = json.loads(line)
                if reply.get("id") == n:
                    if reply.get("ok") is not True:
                        raise RuntimeError(f"{method} failed: {reply.get('error', 'unknown error')}")
                    return reply.get("result")
            raise RuntimeError("control connection closed")

        call("ui.home", {"open": False})
        call("ui.agent_chat", {"refresh": True, "next_step": False})
        wait(lambda: any(a["id"] == "claude" and a["found"] for a in call("ui.agent_chat")["agents"]), "Claude Code adapter not found", 30)
        call("ui.agent_chat", {"agent": "claude", "connect": True, "open": False})
        wait(lambda: call("ui.agent_chat")["state"] == "ready", "Claude Code is not ready", 60)
        if path:
            call("engine.execute", {"command": "file.save_as", "params": {"path": str(path)}})
            call("ui.agent_chat", {"trust_project": True})

        with (root / "events.jsonl").open("x") as events, (root / "gst.log").open("x") as recording_log:
            video = root / "raw.mkv"
            if video.exists() or video.is_symlink():
                raise FileExistsError(video)
            start = time.monotonic()

            def note(**entry):
                print(json.dumps({"video_s": round(time.monotonic() - start, 2), **entry}), file=events, flush=True)

            rec = subprocess.Popen(["gst-launch-1.0", "-e", "ximagesrc", f"display-name={os.environ['DISPLAY']}",
                                    "use-damage=false", "show-pointer=false", "!", "video/x-raw,framerate=30/1", "!",
                                    "videoconvert", "!", "jpegenc", "quality=92", "!", "matroskamux", "!", "filesink",
                                    f"location={video}"], stdout=recording_log, stderr=subprocess.STDOUT)
            try:
                call("ui.agent_chat", {"next_step": True, "next_step_pause_s": 3})
                size = (60, 40, 25) if design == "case" else (80, 50, 10)
                call("engine.execute", {"command": "solid.box", "params": dict(zip(("length", "width", "height"), size))})
                note(kind="accepted", **record_suggestion(call, note=note))
                call("ui.agent_chat", {"next_step": False})
                time.sleep(1)
            finally:
                if rec.poll() is None:
                    rec.send_signal(signal.SIGINT)
                try:
                    rec.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    rec.kill()
                    rec.wait()
                    raise
            if not video.is_file() or video.stat().st_size == 0 or rec.returncode not in (0, 130, -signal.SIGINT):
                raise RuntimeError("recording did not complete; see gst.log")


if __name__ == "__main__":
    port, token, root, design = sys.argv[1:]
    if design not in ("case", "bracket"):
        raise SystemExit("choose case or bracket")
    record(int(port), token, Path(root).resolve(strict=True), design)
