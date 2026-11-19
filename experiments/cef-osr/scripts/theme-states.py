#!/usr/bin/env python3
"""One state screenshot per built-in theme: composer focused, attach button hovered, send disabled.

    scripts/theme-states.py <exe> <out-dir>

The app's own screenshot includes the page (it is a texture), so a screenshot per theme captures
the focus ring, the hovered control and the disabled send button in that theme.
"""
import json
import os
import secrets
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
THEMES = sorted(p.stem for p in (ROOT / "assets/styles/ui/themes").glob("*.json"))
FOLDERS = ("SETTINGS", "VALIDATION", "CRASH", "CONTROL", "RECOVERY")


class Control:
    def __init__(self, port, token):
        sock = socket.create_connection(("127.0.0.1", port), timeout=5)
        sock.settimeout(60)
        self.file = sock.makefile("rw", encoding="utf-8", newline="\n")
        self.token, self.n = token, 0

    def call(self, method, params=None):
        self.n += 1
        request = {"id": self.n, "method": method, "params": params or {}}
        if self.n == 1:
            request["token"] = self.token
        self.file.write(json.dumps(request) + "\n")
        self.file.flush()
        reply = self.file.readline()
        if not reply:
            raise OSError("control channel closed")
        return json.loads(reply)


def main():
    exe, out = sys.argv[1], Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="intcad-states-"))
    env = dict(os.environ)
    for name in FOLDERS:
        env[f"SOLIDCRAFT_{name}_DIR"] = str(work / name.lower())
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    env["NO_AT_BRIDGE"] = "1"
    port = 18195
    log = open(out / "app.log", "wb")
    app = subprocess.Popen([exe, "--control", str(port)], env=env, stdout=log, stderr=subprocess.STDOUT)
    token = (Path(env["SOLIDCRAFT_CONTROL_DIR"]) / f"{port}.token")
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline and not token.exists():
        time.sleep(0.3)
    control = Control(port, token.read_text().strip())
    while time.monotonic() < deadline:
        try:
            if control.call("ui.agent_chat", {"open": True}).get("ok"):
                break
        except OSError:
            time.sleep(0.5)
    while time.monotonic() < deadline:
        state = control.call("ui.agent_chat").get("result") or {}
        if (state.get("webView") or {}).get("page"):
            break
        time.sleep(0.4)
    time.sleep(1.5)

    rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
    ppp = rects.get("pixelsPerPoint") or 1.0
    web = next(r for r in rects.get("rects", []) if r["name"] == "agent_chat:web")
    win = subprocess.run(["xdotool", "search", "--name", "Intcad"], capture_output=True, text=True).stdout.split()
    origin = {"X": 0, "Y": 0}
    if win:
        for line in subprocess.run(["xdotool", "getwindowgeometry", "--shell", win[0]], capture_output=True, text=True).stdout.splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                origin[k] = v
        subprocess.run(["xdotool", "windowactivate", "--sync", win[0]], stderr=subprocess.DEVNULL)

    def at(fx, fy):
        return (round(int(origin["X"]) + (web["x"] + web["width"] * fx) * ppp),
                round(int(origin["Y"]) + (web["y"] + web["height"] * fy) * ppp))

    composer_x, composer_y = at(0.45, 0.905)
    attach_x, attach_y = at(0.06, 0.925)
    # "Edits" in the composer's action row, measured from the app's own screenshot.
    hover_x, hover_y = at(0.26, 0.925)

    for theme in THEMES:
        control.call("ui.prefs", {"theme": theme})
        time.sleep(0.9)
        # Hover the "Edits" text button: the page's hover state, with the send control disabled
        # because the composer is empty. (Keyboard focus needs the window focused at the X level,
        # which a compositor may keep elsewhere; hover is pointer-only and always delivered.)
        subprocess.run(["xdotool", "mousemove", str(hover_x), str(hover_y)], stderr=subprocess.DEVNULL)
        subprocess.run(["xdotool", "mousemove", str(attach_x), str(attach_y)], stderr=subprocess.DEVNULL)
        time.sleep(0.5)
        shot = out / f"{theme}.png"
        control.call("ui.screenshot", {"path": str(shot)})
        print(theme, shot.stat().st_size)

    control.call("app.quit", {"force": True})
    try:
        app.wait(timeout=20)
    except subprocess.TimeoutExpired:
        app.kill()


if __name__ == "__main__":
    main()
