#!/usr/bin/env python3
"""Start the installed SolidCraft snap on headless X11 and Wayland and open the Assistant.

Runs the confined app (`snap install --dangerous` first) with its normal plugs, drives it over
the control channel from the host (snaps share the host's network namespace), requires a usable
viewport and that every controlled entry is refused with `AuthUnavailable` while the host is
signed out (nothing may be committed without account authority), opens the Assistant and
requires its page to load (webView.host and webView.page true), checks the GPU drew a non-blank
frame, and copies each frame to --out (default: the current directory) as snap-<backend>.png.
Needs xvfb-run and weston. Reuses the frame checks of flatpak-smoke.py.

The page is served by the app itself, so it needs no network; the update check is turned off
with SOLIDCRAFT_UPDATE_CHECK=0. release.yml runs the X11 variant inside `sudo unshare -n` with
only loopback, after proving the namespace has a single link; that keeps the release policy that
no network-enabled desktop is started. A strict snap can still be moved back to the host network
namespace by snapd; the control connection then fails and the job reports the failure rather than
passing. The release job also checks that the gnome extension provides libgtk-3.so.0 and
libwebkit2gtk-4.1.so.0.
"""
import argparse
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import secrets
import tempfile
import time

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("flatpak_smoke", HERE / "flatpak-smoke.py")
flatpak_smoke = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(flatpak_smoke)

PORTS = {"x11": 18839, "wayland": 18840}
# The control channel's token (docs/control-protocol.md), given to the app so this client knows it.
TOKEN = secrets.token_hex(32)
REQUESTS = [
    {"id": 1, "method": "ui.inspect", "params": {}, "token": TOKEN},
    {"id": 2, "method": "engine.execute",
     "params": {"command": "solid.box", "params": {"length": 30, "width": 20, "height": 10}}},
    {"id": 3, "method": "document.inspect", "params": {"measure": True}},
]


def drive(app, port, frame, backend):
    """Connect to the app's control port, prove the refusals, read the Assistant marker and capture a frame."""
    deadline = time.monotonic() + 60
    while True:
        if app.poll() is not None:
            raise RuntimeError(f"the app exited with {app.returncode} before listening")
        try:
            conn = socket.create_connection(("127.0.0.1", port), timeout=60)
            break
        except OSError:
            if time.monotonic() >= deadline:
                raise RuntimeError("the control port never opened") from None
            time.sleep(.2)
    with conn, conn.makefile("rw") as io:
        # Listening precedes the first frame, whose ui.inspect still has viewport:null.
        deadline = time.monotonic() + 60
        while True:
            if app.poll() is not None:
                raise RuntimeError(f"the app exited with {app.returncode} before creating a viewport")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RuntimeError("the viewport never appeared")
            conn.settimeout(remaining)
            io.write(json.dumps(REQUESTS[0]) + "\n")
            io.flush()
            try:
                line = io.readline()
            except TimeoutError as error:
                raise RuntimeError("the viewport never appeared before control timed out") from error
            if not line:
                raise RuntimeError("the control connection closed before creating a viewport")
            reply = json.loads(line)
            if not isinstance(reply, dict) or reply.get("id") != 1 or reply.get("ok") is not True:
                raise ValueError(f"ui.inspect failed: {reply}")
            ui = reply["result"]
            if ui["renderer"] not in {"gpu", "cpu"}:
                raise ValueError("ui.inspect returned an invalid renderer")
            viewport = ui["viewport"]
            if viewport is not None:
                if (not isinstance(viewport, list) or len(viewport) != 4
                        or not all(isinstance(value, (int, float)) and math.isfinite(value) for value in viewport)
                        or viewport[2] <= 0 or viewport[3] <= 0):
                    raise ValueError("ui.inspect returned an unusable viewport")
                break
            time.sleep(.25)
        conn.settimeout(60)
        replies = [line]

        def call(request):
            io.write(json.dumps(request) + "\n")
            io.flush()
            raw = io.readline()
            if not raw:
                raise RuntimeError(f"the control connection closed at {request['method']}")
            reply = json.loads(raw)
            if not isinstance(reply, dict) or reply.get("id") != request["id"] or reply.get("ok") is not True:
                raise ValueError(f"{request['method']} failed: {reply}")
            replies.append(raw)
            return reply

        def refused(request):
            """A controlled entry on a signed-out host must be refused, not executed."""
            io.write(json.dumps(request) + "\n")
            io.flush()
            raw = io.readline()
            if not raw:
                raise RuntimeError(f"the control connection closed at {request['method']}")
            reply = json.loads(raw)
            details = (reply.get("errorDetails") or {}) if isinstance(reply, dict) else {}
            if (not isinstance(reply, dict) or reply.get("id") != request["id"]
                    or reply.get("ok") is not False
                    or details.get("code") not in flatpak_smoke.FAIL_CLOSED):
                raise ValueError(f"{request['method']} was not refused for a signed-out host: {reply}")
            replies.append(raw)
            return reply

        refused(REQUESTS[1])
        refused(REQUESTS[2])
        # The Assistant: `--open-assistant` showed the panel, and the restricted ui.inspect
        # marker is the only observation a signed-out host may make of it. A native Wayland
        # window reports its documented diagnostic instead of a hosted page.
        request_id = 4
        markers = []
        deadline = time.monotonic() + 120
        while True:
            marker_reply = call({"id": request_id, "method": "ui.inspect", "params": {}})
            state = marker_reply["result"] or {}
            marker = state.get("assistant") or {}
            markers.append(marker_reply)
            if marker.get("open") is not True:
                raise ValueError(f"the Assistant panel did not open: {state}")
            if marker.get("host") == "failed":
                raise ValueError(f"the Assistant reported a failed web host: {marker}")
            if backend in {"x11", "xwayland"} and marker.get("host") == "available" and marker.get("surface_ready") is True:
                break
            if backend == "wayland" and marker.get("host") == "native_wayland":
                break
            if time.monotonic() >= deadline:
                raise RuntimeError(f"the Assistant's page never loaded: {marker}")
            time.sleep(.25)
            request_id += 1
        capture = call({"id": request_id + 1, "method": "ui.screenshot", "params": {"path": str(frame)}})
        mode = flatpak_smoke.validate_assistant(markers, backend)
    renderer = flatpak_smoke.validate_offline("".join(replies[:3]))
    if capture.get("ok") is not True or renderer != "gpu":
        raise ValueError(f"GPU frame capture failed: renderer={renderer}, {capture}")
    return renderer, flatpak_smoke.frame_statistics(frame.read_bytes()), mode


def run_backend(backend, out):
    runtime = Path(f"/run/user/{os.getuid()}")
    if not runtime.is_dir():
        raise RuntimeError(f"{runtime} is missing: snaps expect the login session's runtime directory")
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), SOLIDCRAFT_CONTROL_TOKEN=TOKEN,
               SOLIDCRAFT_UPDATE_CHECK="0")
    for name in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY", "SOLIDCRAFT_WAYLAND", "SOLVECRAFT_WAYLAND"):
        env.pop(name, None)
    # The snap can write to the (non-hidden) home folder, not to the host's /tmp.
    work = Path(tempfile.mkdtemp(prefix="solidcraft-snap-smoke-", dir=Path.home()))
    compositor = None
    log = (work / "app.log").open("wb")
    try:
        # No private dbus-run-session bus: snapd moves the app into its own systemd scope through
        # the session's systemd, which a private bus doesn't have.
        prefix = []
        if backend == "x11":
            prefix += ["xvfb-run", "-a", "-s", "-screen 0 1600x1000x24 -nolisten tcp"]
        else:
            # The wayland plug only allows sockets named wayland-N in the runtime directory.
            env["WAYLAND_DISPLAY"] = "wayland-77"
            compositor = subprocess.Popen(["weston", "--backend=headless", "--renderer=pixman", "--no-config",
                                           "--socket=wayland-77", "--width=1600", "--height=1000"],
                                          env=env, stdout=log, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 20
            while not (runtime / "wayland-77").exists():
                if compositor.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("headless Weston failed")
                time.sleep(.1)
        with socket.socket() as probe:
            if probe.connect_ex(("127.0.0.1", PORTS[backend])) == 0:
                raise RuntimeError(f"port {PORTS[backend]} is already in use (another SolidCraft?)")
        # Its own process group, so stopping it also stops the app under xvfb-run.
        app = subprocess.Popen(prefix + ["/snap/bin/intcad", "--control", str(PORTS[backend]), "--open-assistant"],
                               env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            renderer, stats, mode = drive(app, PORTS[backend], work / "frame.png", backend)
        finally:
            if app.poll() is None:
                os.killpg(app.pid, signal.SIGTERM)
            try:
                app.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(app.pid, signal.SIGKILL)
                app.wait()
        shutil.copy(work / "frame.png", out / f"snap-{backend}.png")
        print(f"snap {backend}: PASS renderer={renderer}, controlled entries refused (signed out), "
              f"Assistant {mode} (marker), frame={stats}")
    except BaseException:
        log.flush()
        sys.stderr.write((work / "app.log").read_text(errors="replace")[-8000:])
        raise
    finally:
        log.close()
        if compositor:
            compositor.terminate()
            compositor.wait(timeout=10)
        shutil.rmtree(work, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=["x11", "wayland", "both"], default="both")
    parser.add_argument("--out", type=Path, default=Path.cwd())
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    for backend in (["x11", "wayland"] if args.backend == "both" else [args.backend]):
        run_backend(backend, args.out.resolve())


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"snap desktop smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
