#!/usr/bin/env python3
"""Check the installed snap's desktop, signed-out refusals and Assistant runtime.

Run in a private network namespace with only loopback, as in release.yml.
The confined shell runs after Snap's command chain. It must still be in that
namespace before it execs the desktop. It doesn't open the Assistant page.
Requires xvfb-run or Weston and copies frames to --out as snap-<backend>.png.
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


def drive(app, port, frame):
    """Check the control refusals and runtime marker, and capture a frame."""
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
        # Opening the panel is a controlled entry: signed out it must be refused, and the
        # Assistant's retained page cannot be driven without account authority. Prove the
        # refusal, then read the bounded marker: the web runtime must be available.
        request_id = 4
        try:
            io.write(json.dumps({"id": request_id, "method": "ui.agent_chat", "params": {"open": True}}) + "\n")
            io.flush()
            raw = io.readline()
            opening = json.loads(raw)
        except (OSError, ValueError) as error:
            raise ValueError(f"the Assistant open request got no answer: {error}") from None
        details = (opening.get("errorDetails") or {}) if isinstance(opening, dict) else {}
        if opening.get("ok") is not False or details.get("code") not in flatpak_smoke.FAIL_CLOSED:
            raise ValueError(f"the Assistant panel opened without account authority: {opening}")
        replies.append(raw)
        request_id += 1
        deadline = time.monotonic() + 120
        marker = None
        while True:
            reply = call({"id": request_id, "method": "ui.inspect", "params": {}})
            marker = ((reply.get("result") or {}).get("assistant")) or {}
            if marker.get("host") == "available":
                break
            if time.monotonic() >= deadline:
                raise RuntimeError(f"the Assistant web runtime is not reported: {marker}")
            time.sleep(.25)
            request_id += 1
        print("page NOT VERIFIED offline: the Assistant page needs account authority to open", flush=True)
        capture = call({"id": request_id + 1, "method": "ui.screenshot", "params": {"path": str(frame)}})
    renderer = flatpak_smoke.validate_offline("".join(replies[:3]))
    if capture.get("ok") is not True or renderer != "gpu":
        raise ValueError(f"GPU frame capture failed: renderer={renderer}, {capture}")
    return renderer, flatpak_smoke.frame_statistics(frame.read_bytes())


def run_backend(backend, out):
    if [name for _, name in socket.if_nameindex()] != ["lo"]:
        raise RuntimeError("Snap smoke requires a loopback-only network namespace")
    netns = os.readlink("/proc/self/ns/net")
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
        # Keep the normal Snap profile and command chain, but check the
        # namespace after confinement, in the process that execs the desktop.
        launch = 'actual=$(readlink /proc/self/ns/net); if [ "$actual" != "$1" ]; then ' \
                 'echo "Snap network namespace changed; refusing desktop launch" >&2; exit 1; fi; ' \
                 'shift; exec "$SNAP/usr/bin/intcad" "$@"'
        # Its own process group, so stopping it also stops the app under xvfb-run.
        app = subprocess.Popen(prefix + ["snap", "run", "--shell", "intcad", "-ec", launch,
                                         "intcad-offline", netns, "--control", str(PORTS[backend])],
                                env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            renderer, stats = drive(app, PORTS[backend], work / "frame.png")
            # The controlled quit is refused while signed out; the finally block stops the app.
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
              f"Assistant web runtime reported, page NOT VERIFIED offline, frame={stats}")
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
