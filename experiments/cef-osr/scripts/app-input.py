#!/usr/bin/env python3
"""Input checks against the running IntCAD with the CEF Assistant.

Uses the app's own rectangles (`ui.rects`) to click the composer, then checks
the app's state for what was typed, the X clipboard for what was copied, and the
app's log for a CEF popup surface.

    scripts/app-input.py <control-port> <window-title-or-empty> [--log PATH]
"""
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path


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
            raise SystemExit("the control channel closed")
        return json.loads(reply)


def xdo(*args):
    subprocess.run(["xdotool", *args], check=False, stderr=subprocess.DEVNULL)


def main():
    port = int(sys.argv[1])
    log_path = Path(sys.argv[sys.argv.index("--log") + 1]) if "--log" in sys.argv else None
    token = Path("/tmp/intcad-shot-token").read_text().strip()
    control = Control(port, token)
    control.call("ui.agent_chat", {"open": True})
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        state = control.call("ui.agent_chat").get("result") or {}
        if (state.get("webView") or {}).get("page"):
            break
        time.sleep(0.4)
    time.sleep(1.5)

    rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
    ppp = rects.get("pixelsPerPoint") or 1.0
    web = next((r for r in rects.get("rects", []) if r["name"] == "agent_chat:web"), None)
    if not web:
        print(json.dumps({"error": "no agent_chat:web rect", "rects": rects}))
        return 1
    geom = subprocess.run(["xdotool", "getwindowgeometry", "--shell", "@0"], capture_output=True, text=True)
    wid = subprocess.run(["xdotool", "search", "--name", "Intcad"], capture_output=True, text=True).stdout.split()
    window = wid[0] if wid else ""
    if window:
        geom = subprocess.run(["xdotool", "getwindowgeometry", "--shell", window], capture_output=True, text=True)
    origin = {}
    for line in geom.stdout.splitlines():
        if "=" in line:
            key, value = line.strip().split("=", 1)
            origin[key] = value
    ox, oy = int(origin.get("X", 0)), int(origin.get("Y", 0))

    def point(fx, fy):
        return (
            str(round(ox + (web["x"] + web["width"] * fx) * ppp)),
            str(round(oy + (web["y"] + web["height"] * fy) * ppp)),
        )

    if window:
        xdo("windowactivate", "--sync", window)
        time.sleep(0.3)

    results = {}
    # --- typing reaches the page's composer ---------------------------------
    # The composer sits just above the bottom bar of the panel (measured from the
    # app's own screenshot: ~0.885 of the web rect's height).
    cx, cy = point(0.45, 0.885)
    xdo("mousemove", cx, cy, "click", "1")
    time.sleep(0.6)
    xdo("type", "--delay", "40", "clip-abc")
    time.sleep(1.0)
    state = json.dumps(control.call("ui.agent_chat").get("result") or {}, ensure_ascii=False)
    # The control state does not carry the composer draft; the clipboard round-trip below is
    # what proves the page received the typing.
    results["state_exposes_composer"] = "clip-abc" in state
    # What the page received is what Ctrl+A/Ctrl+C gives back through the bridge.
    xdo("key", "--clearmodifiers", "ctrl+a")
    time.sleep(0.2)
    xdo("key", "--clearmodifiers", "ctrl+c")
    time.sleep(1.0)
    first = subprocess.run(["xclip", "-o", "-selection", "clipboard"], capture_output=True, text=True, timeout=10)
    results["typed_copied_out"] = first.stdout.strip()

    # --- system clipboard -> page (Chinese) --------------------------------
    subprocess.run(["bash", "-c", "printf '你好，世界' | xclip -selection clipboard >/dev/null 2>&1 &"], check=False)
    time.sleep(0.6)
    xdo("key", "--clearmodifiers", "ctrl+a")
    time.sleep(0.2)
    xdo("key", "--clearmodifiers", "ctrl+v")
    time.sleep(1.2)
    state = json.dumps(control.call("ui.agent_chat").get("result") or {}, ensure_ascii=False)
    results["state_exposes_composer_after_paste"] = "你好，世界" in state

    # --- page selection -> system clipboard --------------------------------
    xdo("key", "--clearmodifiers", "ctrl+a")
    time.sleep(0.2)
    xdo("key", "--clearmodifiers", "ctrl+c")
    time.sleep(1.0)
    copied = subprocess.run(["xclip", "-o", "-selection", "clipboard"], capture_output=True, text=True, timeout=10)
    results["copied_out"] = copied.stdout.strip()

    # --- a native <select> in the page gets a CEF popup surface ------------
    sx, sy = point(0.03, 0.02)
    xdo("mousemove", sx, sy, "click", "1")
    time.sleep(1.2)
    if log_path and log_path.exists():
        results["popup_logged"] = "popup surface" in log_path.read_text(errors="replace")
    results["web_rect"] = web
    results["click"] = [int(cx), int(cy)]
    print(json.dumps(results, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
