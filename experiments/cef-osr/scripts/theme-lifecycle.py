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


def launch(exe, settings, port, reload_after=None):
    env = dict(os.environ)
    for name in FOLDERS:
        env[f"SOLIDCRAFT_{name}_DIR"] = str(settings / name.lower())
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    if reload_after is not None:
        env["SOLIDCRAFT_ASSISTANT_CEF_TEST_RELOAD_AFTER"] = str(reload_after)
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


def report_status(checks):
    rank = {"PASS": 0, "NOT_RUN": 1, "BLOCKED": 2, "FAIL": 3}
    worst = max((rank.get(c["status"], 4) for c in checks), default=3)
    return {0: "PASS", 1: "NOT_RUN", 2: "BLOCKED", 3: "FAIL"}.get(worst, "FAIL")


def main():
    exe = sys.argv[1]
    out = Path(sys.argv[2])
    reload_after = sys.argv[sys.argv.index("--reload-after") + 1] if "--reload-after" in sys.argv else None
    out.mkdir(parents=True, exist_ok=True)
    checks, limitations = [], [
        "UI-scale preference coverage is not actual compositor or monitor movement.",
        "fd growth is reported as measured; successful theme acknowledgements cannot turn a growth failure into PASS.",
    ]
    settings = Path(tempfile.mkdtemp(prefix="intcad-lifecycle-"))
    port = 18190
    app, control = launch(exe, settings, port, reload_after)
    time.sleep(2)

    results = {}
    # Startup theme.
    state = control.call("ui.agent_chat").get("result") or {}
    results["startup_theme"] = (state.get("theme") or {}).get("id")
    checks.append({"name": "startup_theme", "status": "PASS" if results["startup_theme"] else "FAIL", "detail": str(results["startup_theme"])})

    # Scale-only transitions through the authenticated preference, not X11 scale env.
    baseline = control.call("ui.prefs", {}).get("result")
    baseline = baseline if isinstance(baseline, dict) else {}
    base_ui_scale = baseline.get("ui_scale")
    if not isinstance(base_ui_scale, (int, float)) or isinstance(base_ui_scale, bool) or base_ui_scale <= 0:
        base_ui_scale = 1.0
    baseline_rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
    baseline_ppp = baseline_rects.get("pixelsPerPoint")
    native_base = float(baseline_ppp) / float(base_ui_scale) if isinstance(baseline_ppp, (int, float)) and baseline_ppp > 0 else 0.0
    checks.append({"name": "native_base", "status": "PASS" if native_base > 0 else "BLOCKED", "detail": f"ppp {baseline_ppp} / ui_scale {base_ui_scale} = {native_base:.4f}"})
    scales = [1.0, 1.25, 1.5, 2.0]
    results["scale_only"] = []
    for target in scales:
        request = target / native_base if native_base > 0 else 0.0
        if not (0.5 <= request <= 2.5):
            checks.append({"name": f"scale_only_{target:g}", "status": "FAIL", "detail": f"derived ui_scale {request:.4f} is outside 0.5..2.5"})
            continue
        control.call("ui.prefs", {"ui_scale": request})
        seen, deadline = None, time.monotonic() + 10
        while time.monotonic() < deadline:
            rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
            seen = rects.get("pixelsPerPoint")
            if isinstance(seen, (int, float)) and abs(float(seen) - target) <= 0.02:
                break
            time.sleep(0.25)
        match = isinstance(seen, (int, float)) and abs(float(seen) - target) <= 0.02
        results["scale_only"].append({"target": target, "requested_ui_scale": request, "pixelsPerPoint": seen, "match": match})
        checks.append({"name": f"scale_only_{target:g}", "status": "PASS" if match else "FAIL", "detail": f"requested ui_scale {request:.4f}, ppp {seen}"})
    control.call("ui.prefs", {"ui_scale": base_ui_scale})
    time.sleep(0.5)

    # Resize-only and origin-only sequences: the app must keep its own pixel origin.
    resize_results = []
    for width, height in ((1500, 900), (1200, 800)):
        control.call("ui.resize", {"width": width, "height": height})
        time.sleep(0.8)
        rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
        web_rect = next((r for r in rects.get("rects", []) if r["name"] == "agent_chat:web"), None)
        state = control.call("ui.agent_chat").get("result") or {}
        page = (state.get("webView") or {}).get("page")
        resize_results.append({"width": width, "height": height, "web": web_rect, "page": page})
    results["resize_only"] = resize_results
    checks.append({
        "name": "resize_only",
        "status": "PASS" if resize_results and all(item["web"] and item["page"] for item in resize_results) else "FAIL",
        "detail": f"{len(resize_results)} resize(s), page and web rect present",
    })

    # Optional explicit page reload; observed through the bounded app log and page state.
    reload_observed = False
    if reload_after is not None:
        log_file = settings / "app.log"
        reload_deadline = time.monotonic() + 20
        while time.monotonic() < reload_deadline:
            try:
                if log_file.exists() and "CEF: test reload" in log_file.read_text(errors="replace"):
                    reload_observed = True
                    break
            except OSError:
                pass
            time.sleep(0.5)
        page_back = None
        page_deadline = time.monotonic() + 20
        while time.monotonic() < page_deadline:
            state = control.call("ui.agent_chat").get("result") or {}
            page_back = (state.get("webView") or {}).get("page")
            if reload_observed and page_back:
                break
            time.sleep(0.5)
        results["reload_observed"] = reload_observed
        results["reload_page"] = page_back
        checks.append({
            "name": "reload_requested",
            "status": "PASS" if reload_observed and page_back else "BLOCKED",
            "detail": f"observed={reload_observed} page={page_back}",
        })
    else:
        checks.append({"name": "reload_requested", "status": "NOT_RUN", "detail": "reload sequence was not requested"})

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
        want = (expected or "").lstrip("#")
        delta = max((abs(int(want[i:i + 2], 16) - px[o + i // 2]) for i in range(0, 6, 2)), default=255) if len(want) == 6 else 255
        checks.append({"name": "switch_no_flash", "status": "PASS" if delta <= 2 else "FAIL", "detail": f"surface delta {delta} immediately after switch"})
    else:
        # A missing web rect used to remove this required check silently.
        checks.append({"name": "switch_no_flash", "status": "FAIL", "detail": "no agent_chat:web rect; the immediate post-switch repaint cannot be verified"})

    # Reopen: hide the panel, show it again; the theme and page must survive.
    control.call("ui.agent_chat", {"open": False})
    time.sleep(0.8)
    control.call("ui.agent_chat", {"open": True})
    time.sleep(1.5)
    state = control.call("ui.agent_chat").get("result") or {}
    results["after_reopen_theme"] = (state.get("theme") or {}).get("id")
    results["after_reopen_page"] = (state.get("webView") or {}).get("page")
    checks.append({"name": "reopen", "status": "PASS" if results["after_reopen_theme"] and results["after_reopen_page"] else "FAIL", "detail": f"theme={results['after_reopen_theme']} page={results['after_reopen_page']}"})

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
    fd_growth = results["leak_round2"][1] - results["leak_round1"][1]
    results["fd_growth"] = fd_growth
    checks.append({"name": "fd_growth", "status": "PASS" if fd_growth <= 8 else "FAIL", "detail": f"round2-round1 fds {fd_growth}; prior unresolved growth must not pass on theme acknowledgements"})

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
    checks.append({"name": "restart_theme", "status": "PASS" if results["restart_theme"] == "gruvbox-light" else "FAIL", "detail": str(results["restart_theme"])})
    control.call("app.quit", {"force": True})
    try:
        app.wait(timeout=20)
    except subprocess.TimeoutExpired:
        app.kill()

    report = {
        "status": report_status(checks),
        "checks": checks,
        "artifacts": [{"path": str(out / "lifecycle.json")}],
        "limitations": limitations,
        "results": results,
    }
    report_path = out / "lifecycle.json"
    report_path.write_text(json.dumps(report, indent=1))
    print(json.dumps(report, indent=1))
    return 0 if report["status"] == "PASS" else (1 if report["status"] == "FAIL" else 2)


if __name__ == "__main__":
    sys.exit(main())
