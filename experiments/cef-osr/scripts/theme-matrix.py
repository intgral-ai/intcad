#!/usr/bin/env python3
"""Every built-in theme through the real app, with a screenshot and a colour check.

Launches IntCAD (CEF Assistant), opens the panel, then for each theme file in
assets/styles/ui/themes: sets it through the app's own control channel, waits for
the page to repaint, saves the app's screenshot, and compares the page's painted
background with the surface colour the app reports for that theme.

    scripts/theme-matrix.py <exe> [--out DIR] [--ui-scales 1,1.25,1.5,2] [--themes a,b,...]

The colour check catches the two faults this work fixes: a page that keeps its own
palette instead of the window's, and an sRGB texture that is decoded twice (the
painted background comes out darker than the token). Scales are driven through the
authenticated `ui.prefs {"ui_scale": ...}` path, not an X11 environment override.
"""
import hashlib
import json
import os
import re
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


def read_png(path):
    import struct
    import zlib

    data = Path(path).read_bytes()
    i, idat = 8, b""
    while i < len(data):
        length = struct.unpack(">I", data[i:i + 4])[0]
        kind, chunk = data[i + 4:i + 8], data[i + 8:i + 8 + length]
        i += 12 + length
        if kind == b"IHDR":
            w, h, _depth, color = struct.unpack(">IIBB", chunk[:10])
        elif kind == b"IDAT":
            idat += chunk
        elif kind == b"IEND":
            break
    raw = zlib.decompress(idat)
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color]
    stride = w * channels
    out, prev, p = bytearray(), bytearray(stride), 0
    for _ in range(h):
        kind = raw[p]
        p += 1
        line = bytearray(raw[p:p + stride])
        p += stride
        for x in range(stride):
            a = line[x - channels] if x >= channels else 0
            b = prev[x]
            c = prev[x - channels] if x >= channels else 0
            if kind == 1:
                line[x] = (line[x] + a) & 255
            elif kind == 2:
                line[x] = (line[x] + b) & 255
            elif kind == 3:
                line[x] = (line[x] + ((a + b) >> 1)) & 255
            elif kind == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        out += line
        prev = line
    return w, h, channels, bytes(out)


def pixel(png, x, y):
    w, h, ch, px = png
    x, y = min(max(x, 0), w - 1), min(max(y, 0), h - 1)
    o = (y * w + x) * ch
    return tuple(px[o:o + 3])


def hexed(rgb):
    return "#%02x%02x%02x" % tuple(rgb)


def parse_scope_log(path):
    """Latest fixed-hook `SCOPE {json}` line, not a transcript or a URL dump."""
    latest = None
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as handle:
            for line in handle:
                marker = "SCOPE {"
                at = line.find(marker)
                if at < 0:
                    continue
                try:
                    value = json.loads(line[at + len("SCOPE "):].strip())
                except ValueError:
                    continue
                if isinstance(value, dict) and value.get("action") == "rects":
                    latest = value
    except OSError:
        return None
    return latest


def theme_report_status(checks):
    rank = {"PASS": 0, "NOT_RUN": 1, "BLOCKED": 2, "FAIL": 3}
    worst = max((rank.get(c["status"], 4) for c in checks), default=3)
    return {0: "PASS", 1: "NOT_RUN", 2: "BLOCKED", 3: "FAIL"}.get(worst, "FAIL")


def main():
    exe = sys.argv[1]
    out = Path(sys.argv[sys.argv.index("--out") + 1]) if "--out" in sys.argv else Path(tempfile.mkdtemp(prefix="theme-matrix-"))
    scales = sys.argv[sys.argv.index("--ui-scales") + 1].split(",") if "--ui-scales" in sys.argv else ["1", "1.25", "1.5", "2"]
    wanted = sys.argv[sys.argv.index("--themes") + 1].split(",") if "--themes" in sys.argv else THEMES
    out.mkdir(parents=True, exist_ok=True)
    checks, artifacts, limitations = [], [], [
        "UI-scale preference coverage is not actual compositor or monitor movement.",
        "requested_scale is the app's placement scale; capture/source/import density comes from frame receipts.",
    ]
    if len(THEMES) != 25 or len(set(THEMES)) != 25:
        checks.append({"name": "theme_source", "status": "FAIL", "detail": f"expected 25 unique source themes, found {len(THEMES)}"})
    else:
        checks.append({"name": "theme_source", "status": "PASS", "detail": "25 built-in themes enumerated from source"})

    work = Path(tempfile.mkdtemp(prefix="intcad-theme-"))
    env = dict(os.environ)
    for name in FOLDERS:
        env[f"SOLIDCRAFT_{name}_DIR"] = str(work / name.lower())
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    env["NO_AT_BRIDGE"] = "1"
    # Opt-in, fixed in-source scope probe: bounded DOM rects/computed values only.
    env["SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE"] = "1"
    log_path = out / "app.log"
    log = open(log_path, "wb")
    app = subprocess.Popen([exe, "--control", "0"], env=env, stdout=log, stderr=subprocess.STDOUT)
    # The app picks a free port and writes it, with the token, into its control folder.
    control_dir = Path(env["SOLIDCRAFT_CONTROL_DIR"])
    deadline = time.monotonic() + 120
    port, token = 0, ""
    while time.monotonic() < deadline:
        ports = [p for p in control_dir.glob("*.port")]
        if ports:
            port = int(ports[0].read_text().strip())
            token = (control_dir / f"{port}.token").read_text().strip()
            break
        time.sleep(0.3)
    control = None
    while time.monotonic() < deadline and control is None:
        try:
            control = Control(port, token)
        except OSError:
            time.sleep(0.5)
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
        time.sleep(0.5)
    time.sleep(2)

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
    baseline = control.call("ui.prefs", {}).get("result")
    baseline = baseline if isinstance(baseline, dict) else {}
    base_scale = baseline.get("ui_scale")
    if not isinstance(base_scale, (int, float)) or isinstance(base_scale, bool) or base_scale <= 0:
        base_scale = 1.0
    native_base = float(ppp) / float(base_scale)
    checks.append({"name": "native_base", "status": "PASS" if native_base > 0 else "BLOCKED", "detail": f"ppp {ppp} / ui_scale {base_scale} = {native_base:.4f}"})

    results = []
    all_checks = checks
    for target_text in scales:
        try:
            target = float(target_text)
        except ValueError:
            all_checks.append({"name": f"ui_scale_{target_text}", "status": "FAIL", "detail": "invalid scale"})
            continue
        request = target / native_base if native_base > 0 else 0.0
        if not (0.5 <= request <= 2.5):
            all_checks.append({"name": f"ui_scale_{target:g}", "status": "FAIL", "detail": f"derived ui_scale {request:.4f} is outside 0.5..2.5"})
            continue
        control.call("ui.prefs", {"ui_scale": request})
        seen, scale_deadline = None, time.monotonic() + 10
        while time.monotonic() < scale_deadline:
            measured = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
            seen = measured.get("pixelsPerPoint")
            if isinstance(seen, (int, float)) and abs(float(seen) - target) <= 0.02:
                break
            time.sleep(0.25)
        match = isinstance(seen, (int, float)) and not isinstance(seen, bool) and abs(float(seen) - target) <= 0.02
        if match:
            ppp = float(seen)
        all_checks.append({"name": f"ui_scale_{target:g}", "status": "PASS" if match else "FAIL", "detail": f"requested ui_scale {request:.4f}, measured pixelsPerPoint {seen}"})
        scope = parse_scope_log(log_path)
        if scope is None:
            all_checks.append({"name": f"role_fonts_{target:g}", "status": "BLOCKED", "detail": "fixed scope probe did not report computed values"})
        else:
            fonts = scope.get("fonts") or {}
            root_font, prose, textarea, label = fonts.get("root"), fonts.get("prose"), fonts.get("textarea"), fonts.get("label")
            missing = [name for name, value in (("prose", prose), ("textarea", textarea), ("label", label)) if not isinstance(value, (int, float))]
            if missing:
                all_checks.append({"name": f"role_fonts_{target:g}", "status": "BLOCKED", "detail": f"computed values unavailable for: {', '.join(missing)}"})
            else:
                problems = []
                if not isinstance(root_font, (int, float)) or abs(float(root_font) - 12.0) > 0.1:
                    problems.append(f"root font {root_font} != 12")
                if any(abs(float(value) - 12.5) > 0.1 for value in (prose, textarea, label)):
                    problems.append(f"prose/textarea/label fonts {[prose, textarea, label]} != 12.5")
                all_checks.append({"name": f"role_fonts_{target:g}", "status": "FAIL" if problems else "PASS", "detail": "; ".join(problems) or "root 12, prose/textarea/label 12.5"})

        for theme in wanted:
            answer = control.call("ui.prefs", {"theme": theme})
            ok = bool(answer.get("ok"))
            time.sleep(1.4)
            state = control.call("ui.agent_chat").get("result") or {}
            palette = state.get("theme") or {}
            shot = out / f"{theme}-{target_text.replace('.', '_')}.png"
            control.call("ui.screenshot", {"path": str(shot)})
            time.sleep(0.15)
            png = read_png(shot)
            bg = pixel(png, round((web["x"] + web["width"] * 0.5) * ppp + int(origin["X"])),
                       round((web["y"] + web["height"] * 0.62) * ppp + int(origin["Y"])))
            expected = (palette.get("colors") or {}).get("surface")
            want = expected.lstrip("#") if expected else ""
            delta = max(
                (abs(int(want[i:i + 2], 16) - bg[i // 2]) for i in range(0, 6, 2)),
                default=255,
            ) if len(want) == 6 else 255
            tokens = palette.get("colors") or {}
            def color_ok(value):
                return isinstance(value, str) and re.fullmatch(r"#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?", value) is not None
            token_ok = all(color_ok(tokens.get(name)) for name in ("window", "surface", "text", "panel", "border", "accent", "borderStrong", "hover", "selected"))
            all_checks.append({
                "name": f"palette_{target:g}_{theme}",
                "status": "PASS" if ok and palette.get("id") == theme and token_ok and delta <= 2 else "FAIL",
                "detail": f"applied={ok} reported={palette.get('id')} tokens={token_ok} surface delta={delta}",
            })
            results.append({
                "theme": theme,
                "ui_scale": target,
                "requested_ui_scale": request,
                "applied": ok,
                "reported_id": palette.get("id"),
                "dark": palette.get("dark"),
                "expected_surface": expected,
                "painted_surface": hexed(bg),
                "delta": delta,
                "match": delta <= 2,
                "tokens": {name: tokens.get(name) for name in ("window", "surface", "text", "panel", "border", "accent")},
                "screenshot": str(shot),
            })
            print(json.dumps(results[-1]))

    control.call("app.quit", {"force": True})
    try:
        app.wait(timeout=20)
    except subprocess.TimeoutExpired:
        app.kill()
    log.close()
    artifacts.append({"path": str(log_path), "sha256": hashlib.sha256(log_path.read_bytes()).hexdigest() if log_path.exists() else "", "size": log_path.stat().st_size if log_path.exists() else 0})
    report = {
        "status": theme_report_status(all_checks),
        "checks": all_checks,
        "artifacts": artifacts,
        "limitations": limitations,
        "themes": results,
    }
    report_path = out / "matrix.json"
    report_path.write_text(json.dumps(report, indent=1))
    bad = [r["theme"] for r in results if not r["match"]]
    print(f"themes={len(results)} mismatched={len(bad)} {bad}")
    print(f"status={report['status']} report={report_path}")
    return 0 if report["status"] == "PASS" else (1 if report["status"] == "FAIL" else 2)


if __name__ == "__main__":
    sys.exit(main())
