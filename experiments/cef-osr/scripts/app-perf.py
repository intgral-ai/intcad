#!/usr/bin/env python3
"""Measure the real IntCAD CAD viewport with the Assistant open.

Runs one build of the app (WRY host or CEF host), opens the Assistant through the
app's own control channel, drags in the 3D view with xdotool, and reads the
frame gaps the app itself publishes (`ui.inspect.frame_gaps_ms`) — the same
instrument `docs/linux-webview-performance.md` uses.

    scripts/app-perf.py <exe> wry|cef [--seconds 3] [--out DIR]

Environment for the CEF run: CEF_PATH, and optionally
SOLIDCRAFT_ASSISTANT_CEF_SWITCHES (e.g. `ozone-platform=x11,use-angle=gl-egl`).
DISPLAY must point at an X11 seat (XWayland or Xorg): the drag is injected with
xdotool, which addresses X clients only.
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

HERE = Path(__file__).resolve().parent
FOLDERS = ("SETTINGS", "VALIDATION", "CRASH", "CONTROL", "RECOVERY")


class Control:
    def __init__(self, port, token):
        self.port, self.token, self.file, self.n = port, token, None, 0

    def call(self, method, params=None):
        if self.file is None:
            sock = socket.create_connection(("127.0.0.1", self.port), timeout=5)
            sock.settimeout(60)
            self.file = sock.makefile("rw", encoding="utf-8", newline="\n")
        self.n += 1
        request = {"id": self.n, "method": method, "params": params or {}}
        if self.n == 1:
            request["token"] = self.token
        self.file.write(json.dumps(request) + "\n")
        self.file.flush()
        reply = self.file.readline()
        if not reply:
            raise OSError("the app closed the control channel")
        return json.loads(reply)


def cpu_snapshot(root):
    stats = {}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat", encoding="utf-8", errors="replace") as handle:
                line = handle.read()
            after = line.rsplit(")", 1)[1].split()
            stats[int(entry)] = (int(after[1]), int(after[11]) + int(after[12]), int(after[21]) * 4096)
        except (OSError, IndexError, ValueError):
            continue
    members = {root}
    changed = True
    while changed:
        changed = False
        for pid, (ppid, _, _) in stats.items():
            if ppid in members and pid not in members:
                members.add(pid)
                changed = True
    ticks = sum(stats[p][1] for p in members if p in stats)
    rss = sum(stats[p][2] for p in members if p in stats)
    return ticks, rss, len(members)


def percentile(values, fraction):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, round((len(ordered) - 1) * fraction))
    return ordered[index]


def main():
    exe = sys.argv[1]
    mode = sys.argv[2]
    seconds = 3.0
    if "--seconds" in sys.argv:
        seconds = float(sys.argv[sys.argv.index("--seconds") + 1])
    out = Path(sys.argv[sys.argv.index("--out") + 1]) if "--out" in sys.argv else Path(tempfile.mkdtemp(prefix="app-perf-"))
    out.mkdir(parents=True, exist_ok=True)

    work = Path(tempfile.mkdtemp(prefix=f"intcad-perf-{mode}-"))
    env = dict(os.environ)
    for name in FOLDERS:
        env[f"SOLIDCRAFT_{name}_DIR"] = str(work / name.lower())
    token = secrets.token_hex(24)
    env["SOLIDCRAFT_CONTROL_TOKEN"] = token
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    env["NO_AT_BRIDGE"] = "1"
    if mode == "cef":
        env["SOLIDCRAFT_ASSISTANT_CEF"] = "1"
        env.setdefault("SOLIDCRAFT_ASSISTANT_CEF_SWITCHES", "ozone-platform=x11,use-angle=gl-egl")
        if "CEF_PATH" not in env:
            sys.exit("CEF_PATH is required for the cef run")
    else:
        env.pop("SOLIDCRAFT_ASSISTANT_CEF", None)
        env["SOLIDCRAFT_WAYLAND"] = "0"

    port = 18080 + (0 if mode == "wry" else 1)
    log = open(out / f"app-{mode}.log", "wb")
    started = time.monotonic()
    app = subprocess.Popen([exe, "--control", str(port)], env=env, stdout=log, stderr=subprocess.STDOUT)

    control = Control(port, token)
    deadline = time.monotonic() + 120
    reply = None
    while time.monotonic() < deadline and app.poll() is None:
        try:
            reply = control.call("ui.agent_chat", {"open": True})
            break
        except (OSError, ValueError):
            time.sleep(0.5)
    if not reply or not reply.get("ok"):
        print(json.dumps({"mode": mode, "error": "control channel did not answer"}))
        app.kill()
        return 1

    opened = time.monotonic()
    state = {}
    while time.monotonic() < deadline:
        state = control.call("ui.agent_chat", {}).get("result") or {}
        if (state.get("webView") or {}).get("page"):
            break
        time.sleep(0.5)
    page_ready = time.monotonic() - opened
    web = state.get("webView") or {}

    frame = out / f"{mode}-window.png"
    try:
        control.call("ui.screenshot", {"path": str(frame)})
    except Exception:
        pass

    # Let the page settle: the first second after load is the page's own work.
    time.sleep(5.0)
    idle = control.call("ui.inspect").get("result") or {}
    idle_gaps = idle.get("frame_gaps_ms") or []
    time.sleep(3.0)
    idle2 = control.call("ui.inspect").get("result") or {}
    idle2_gaps = idle2.get("frame_gaps_ms") or []

    # Drag in the 3D view: to the left of the Assistant panel.
    wid = subprocess.run(["xdotool", "search", "--name", "Intcad"], capture_output=True, text=True).stdout.split()
    window = wid[0] if wid else ""
    if window:
        subprocess.run(["xdotool", "windowactivate", "--sync", window], check=False, stderr=subprocess.DEVNULL)
        time.sleep(0.3)
        subprocess.run(["xdotool", "mousemove", "520", "420", "click", "1"], check=False, stderr=subprocess.DEVNULL)
    ticks0, _, _ = cpu_snapshot(app.pid)
    start = time.monotonic()
    subprocess.run(["xdotool", "mousemove", "520", "420"], check=False)
    time.sleep(0.2)
    subprocess.run(["xdotool", "mousedown", "1"], check=False)
    step = 0
    while time.monotonic() - start < seconds:
        x = 520 + (step % 80) * 3
        y = 420 + ((step // 80) % 2) * 60
        subprocess.run(["xdotool", "mousemove", str(x), str(y)], check=False)
        step += 1
        time.sleep(0.03)
    subprocess.run(["xdotool", "mouseup", "1"], check=False)
    busy_gaps = (control.call("ui.inspect").get("result") or {}).get("frame_gaps_ms") or []
    ticks1, rss, processes = cpu_snapshot(app.pid)
    elapsed = max(0.001, time.monotonic() - start)
    cpu = (ticks1 - ticks0) / os.sysconf("SC_CLK_TCK") / elapsed * 100.0

    result = {
        "mode": mode,
        "page_ready_s": round(page_ready, 2),
        "webView": web,
        "idle": {
            "frames": len(idle_gaps),
            "p50_ms": percentile(idle_gaps, 0.5),
            "p95_ms": percentile(idle_gaps, 0.95),
            "max_ms": max(idle_gaps) if idle_gaps else None,
        },
        "idle_late": {
            "frames": len(idle2_gaps),
            "p50_ms": percentile(idle2_gaps, 0.5),
            "p95_ms": percentile(idle2_gaps, 0.95),
            "max_ms": max(idle2_gaps) if idle2_gaps else None,
        },
        "drag": {
            "frames": len(busy_gaps),
            "p50_ms": percentile(busy_gaps, 0.5),
            "p95_ms": percentile(busy_gaps, 0.95),
            "p99_ms": percentile(busy_gaps, 0.99),
            "max_ms": max(busy_gaps) if busy_gaps else None,
            "over_50ms": sum(1 for gap in busy_gaps if gap > 50),
            "cpu_percent": round(cpu, 1),
            "rss_mib": round(rss / 1048576, 1),
            "processes": processes,
        },
        "total_s": round(time.monotonic() - started, 1),
    }
    (out / f"perf-{mode}.json").write_text(json.dumps(result, indent=1))
    print(json.dumps(result))
    control.call("app.quit", {"force": True})
    try:
        app.wait(timeout=20)
    except subprocess.TimeoutExpired:
        app.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())
