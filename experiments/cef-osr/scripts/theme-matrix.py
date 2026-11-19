#!/usr/bin/env python3
"""Every built-in theme through the real app, with a screenshot and a colour check.

Launches IntCAD (CEF Assistant), opens the panel, then for each theme file in
assets/styles/ui/themes: sets it through the app's own control channel, waits for
the page to repaint, saves the app's screenshot, and compares the page's painted
background with the surface colour the app reports for that theme.

    scripts/theme-matrix.py <exe> [--out DIR] [--dpi 1.25] [--themes a,b,...]

The colour check catches the two faults this work fixes: a page that keeps its own
palette instead of the window's, and an sRGB texture that is decoded twice (the
painted background comes out darker than the token).
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


def main():
    exe = sys.argv[1]
    out = Path(sys.argv[sys.argv.index("--out") + 1]) if "--out" in sys.argv else Path(tempfile.mkdtemp(prefix="theme-matrix-"))
    dpi = sys.argv[sys.argv.index("--dpi") + 1] if "--dpi" in sys.argv else "1.25"
    wanted = sys.argv[sys.argv.index("--themes") + 1].split(",") if "--themes" in sys.argv else THEMES
    out.mkdir(parents=True, exist_ok=True)

    work = Path(tempfile.mkdtemp(prefix="intcad-theme-"))
    env = dict(os.environ)
    for name in FOLDERS:
        env[f"SOLIDCRAFT_{name}_DIR"] = str(work / name.lower())
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    env["NO_AT_BRIDGE"] = "1"
    env["WINIT_X11_SCALE_FACTOR"] = dpi
    log = open(out / "app.log", "wb")
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

    def screen_point(fx, fy):
        return (
            round(int(origin["X"]) + (web["x"] + web["width"] * fx) * ppp),
            round(int(origin["Y"]) + (web["y"] + web["height"] * fy) * ppp),
        )

    results = []
    for theme in wanted:
        answer = control.call("ui.prefs", {"theme": theme})
        ok = bool(answer.get("ok"))
        time.sleep(1.4)
        state = control.call("ui.agent_chat").get("result") or {}
        palette = state.get("theme") or {}
        shot = out / f"{theme}.png"
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
        results.append({
            "theme": theme,
            "applied": ok,
            "reported_id": palette.get("id"),
            "dark": palette.get("dark"),
            "expected_surface": expected,
            "painted_surface": hexed(bg),
            # egui dithers its gamma values by ±0.5/255, so one step is expected.
            "delta": delta,
            "match": delta <= 2,
            "screenshot": str(shot),
        })
        print(json.dumps(results[-1]))

    control.call("app.quit", {"force": True})
    try:
        app.wait(timeout=20)
    except subprocess.TimeoutExpired:
        app.kill()
    (out / f"matrix-{dpi.replace('.', '_')}.json").write_text(json.dumps(results, indent=1))
    bad = [r["theme"] for r in results if not r["match"]]
    print(f"themes={len(results)} mismatched={len(bad)} {bad}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
