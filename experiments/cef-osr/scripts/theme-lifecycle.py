#!/usr/bin/env python3
"""Theme lifecycle on the real app: startup, reopen, hot switch without a flash, restart, leaks.

    scripts/theme-lifecycle.py <exe> <out-dir>

Runs the app twice with the same settings folder: the first run switches themes, checks the panel
closes and reopens with the theme intact, samples the page immediately after a switch (no white or
black flash) and records RSS/fd across 25 switches; the second run proves the theme was persisted.
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

sys.path.insert(0, str(Path(__file__).resolve().parent))
from pngtool import read_png
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


def tree_usage(root):
    stats, members = {}, {root}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat", encoding="utf-8", errors="replace") as handle:
                after = handle.read().rsplit(")", 1)[1].split()
            stats[int(entry)] = (int(after[1]), int(after[21]) * 4096)
        except (OSError, IndexError, ValueError):
            continue
    changed = True
    while changed:
        changed = False
        for pid, (ppid, _) in stats.items():
            if ppid in members and pid not in members:
                members.add(pid)
                changed = True
    rss = sum(stats[p][1] for p in members if p in stats)
    fds = sum(len(os.listdir(f"/proc/{p}/fd")) for p in members if os.path.isdir(f"/proc/{p}/fd"))
    return round(rss / 1048576, 1), fds, len(members)


def launch(exe, settings, port):
    env = dict(os.environ)
    for name in FOLDERS:
        env[f"SOLIDCRAFT_{name}_DIR"] = str(settings / name.lower())
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    (settings).mkdir(parents=True, exist_ok=True)
    log = open(settings / "app.log", "wb")
    app = subprocess.Popen([exe, "--control", str(port)], env=env, stdout=log, stderr=subprocess.STDOUT)
    token_file = settings / "control" / f"{port}.token"
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline and not token_file.exists():
        time.sleep(0.3)
    control = Control(port, token_file.read_text().strip())
    while time.monotonic() < deadline:
        try:
            if control.call("ui.agent_chat", {"open": True}).get("ok"):
                break
        except OSError:
            time.sleep(0.5)
    return app, control


def main():
    exe = sys.argv[1]
    out = Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    settings = Path(tempfile.mkdtemp(prefix="intcad-lifecycle-"))
    port = 18190
    app, control = launch(exe, settings, port)
    time.sleep(2)

    results = {}
    # Startup theme.
    state = control.call("ui.agent_chat").get("result") or {}
    results["startup_theme"] = (state.get("theme") or {}).get("id")

    # Hot switch, sampled at once: the panel must not flash white or black.
    control.call("ui.prefs", {"theme": "nord"})
    quick = out / "switch-nord-now.png"
    control.call("ui.screenshot", {"path": str(quick)})
    rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
    ppp = rects.get("pixelsPerPoint") or 1.0
    web = next((r for r in rects.get("rects", []) if r["name"] == "agent_chat:web"), None)
    state = control.call("ui.agent_chat").get("result") or {}
    expected = ((state.get("theme") or {}).get("colors") or {}).get("surface")
    if web:
        w, h, ch, px = read_png(str(quick))
        x = round((web["x"] + web["width"] * 0.5) * ppp)
        y = round((web["y"] + web["height"] * 0.62) * ppp)
        o = (min(y, h - 1) * w + min(x, w - 1)) * ch
        painted = "#%02x%02x%02x" % tuple(px[o:o + 3])
        results["switched_surface"] = {"expected": expected, "painted_at_once": painted}

    # Reopen: hide the panel, show it again; the theme and page must survive.
    control.call("ui.agent_chat", {"open": False})
    time.sleep(0.8)
    control.call("ui.agent_chat", {"open": True})
    time.sleep(1.5)
    state = control.call("ui.agent_chat").get("result") or {}
    results["after_reopen_theme"] = (state.get("theme") or {}).get("id")
    results["after_reopen_page"] = (state.get("webView") or {}).get("page")

    # Leak: 25 switches, no browser recreation.
    themes = sorted(p.stem for p in (Path(exe).resolve().parents[2] / "assets/styles/ui/themes").glob("*.json"))

    def cycle(times, offset):
        for index in range(times):
            control.call("ui.prefs", {"theme": themes[(offset + index) % len(themes)]})
            time.sleep(0.15)
        time.sleep(2)
        return tree_usage(app.pid)

    # Warm up, then compare two equal rounds: a leak keeps growing, a cache settles.
    results["leak_warm"] = cycle(5, 0)
    results["leak_round1"] = cycle(25, 5)
    results["leak_round2"] = cycle(25, 30)
    results["switches"] = 55

    # Persist a non-default theme and quit.
    control.call("ui.prefs", {"theme": "gruvbox-light"})
    time.sleep(1.0)
    control.call("app.quit", {"force": True})
    try:
        app.wait(timeout=20)
    except subprocess.TimeoutExpired:
        app.kill()

    # Restart with the same settings: the theme must come back.
    app, control = launch(exe, settings, port + 1)
    time.sleep(2)
    state = control.call("ui.agent_chat").get("result") or {}
    results["restart_theme"] = (state.get("theme") or {}).get("id")
    control.call("app.quit", {"force": True})
    try:
        app.wait(timeout=20)
    except subprocess.TimeoutExpired:
        app.kill()

    (out / "lifecycle.json").write_text(json.dumps(results, indent=1))
    print(json.dumps(results, indent=1))


if __name__ == "__main__":
    main()
