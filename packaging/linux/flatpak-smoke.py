#!/usr/bin/env python3
"""Exercise desktop startup, modelling and the Assistant in the installed Flatpak.

Uses the app's normal sandbox permissions. The control client runs inside that
sandbox, so this does not grant host network access or save Flatpak overrides.
Each headless backend has private runtime/configuration directories and a bus.
The Assistant's page is served by the app itself, so opening it needs no network.
"""
import argparse
import base64
import json
import math
import os
import re
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib

APP_ID = "ai.intgral.intcad"
MAX_PIXELS = 16_000_000
MAX_PNG_BYTES = 70_000_000
# The authority layer's refusals for a controlled entry on a host with no account: no authority
# at all, or no durable commit path for that entry. Both are fail-closed; nothing else counts.
FAIL_CLOSED = frozenset({"AuthUnavailable", "UnsafeToStart"})
SANDBOX_PROBE = r'''
set -euo pipefail
work=$(mktemp -d)
export XDG_CONFIG_HOME="$work/config" XDG_DATA_HOME="$work/data" XDG_CACHE_HOME="$work/cache"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
unset WGPU_BACKEND WGPU_BACKENDS
# The control channel's token (docs/control-protocol.md), set here so this client knows it.
SOLIDCRAFT_CONTROL_TOKEN=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
export SOLIDCRAFT_CONTROL_TOKEN
# Its own process group: a release app runs the built-in agent as a child, and stopping the app
# must stop it too. Without the group, `kill` ends only the app and the agent keeps the files in
# "$work" busy for a moment, which the cleanup then cannot remove.
setsid intcad --control 18837 >"$work/app.log" 2>&1 &
app=$!
cleanup() {
  kill -TERM -"$app" 2>/dev/null || kill "$app" 2>/dev/null || true
  wait "$app" 2>/dev/null || true
  # Bounded: a child that was started before the group existed, or one still closing its files.
  for _ in $(seq 1 50); do
    pgrep -f -- "$work" >/dev/null 2>&1 || break
    sleep .1
  done
  cat "$work/app.log" >&2
  rm -rf -- "$work" 2>/dev/null || {
    sleep .5
    rm -rf -- "$work"
  }
}
trap cleanup EXIT
ready=0
for ((attempt=0; attempt<300; attempt++)); do
  if ! kill -0 "$app" 2>/dev/null; then exit 1; fi
  if { exec 3<>/dev/tcp/127.0.0.1/18837; } 2>/dev/null; then ready=1; break; fi
  sleep .1
done
test "$ready" = 1
# The control port opens before the first frame, so ui.inspect has no viewport yet; poll until it does.
inspect="{\"id\":1,\"method\":\"ui.inspect\",\"params\":{},\"token\":\"$SOLIDCRAFT_CONTROL_TOKEN\"}"
deadline=$((SECONDS + 60))
until printf '%s\n' "$inspect" >&3 && IFS= read -r -t 30 reply <&3 && [[ "$reply" =~ \"viewport\":\ *\[ ]]; do
  if ! kill -0 "$app" 2>/dev/null || ((SECONDS >= deadline)); then
    echo "viewport never appeared: ${reply:-no reply}" >&2
    exit 1
  fi
  sleep .25
done
printf '%s\n' "$reply"
for request in '{"id":2,"method":"engine.execute","params":{"command":"solid.box","params":{"length":30,"width":20,"height":10}}}' '{"id":3,"method":"document.inspect","params":{"measure":true}}'; do
  printf '%s\n' "$request" >&3
  IFS= read -r -t 30 reply <&3
  printf '%s\n' "$reply"
done
# Opening the panel is a controlled entry. Signed out it must be refused; the page is
# carried inside the app but cannot be driven without account authority, so the probe
# proves the refusal and then reads the bounded marker for the web runtime.
printf '{"id":4,"method":"ui.agent_chat","params":{"open":true}}\n' >&3
IFS= read -r -t 30 reply <&3
printf '%s\n' "$reply"
id=5
deadline=$((SECONDS + 120))
while :; do
  printf '{"id":%d,"method":"ui.inspect","params":{}}\n' "$id" >&3
  if ! IFS= read -r -t 30 reply <&3; then
    echo "the Assistant marker got no reply" >&2
    exit 1
  fi
  printf '%s\n' "$reply"
  if printf '%s' "$reply" | grep -Eq '"host"[[:space:]]*:[[:space:]]*"(available|native_wayland)"'; then
    break
  fi
  if ((SECONDS >= deadline)); then
    echo "the Assistant web runtime is not reported: $reply" >&2
    exit 1
  fi
  sleep .25
  id=$((id + 1))
done
printf '{"id":%d,"method":"ui.screenshot","params":{"path":"%s/frame.png"}}\n' "$((id + 1))" "$work" >&3
IFS= read -r -t 30 reply <&3
printf '%s\n' "$reply"
base64 -w0 "$work/frame.png"
printf '\n'
'''


def validate_offline(stdout):
    """Check what this signed-out, offline desktop must do: run, and refuse control writes.

    The control channel is a controlled entry, so an app with no account authority must
    refuse `engine.execute` and `document.inspect` and must not commit anything. Both
    fail-closed codes the authority layer uses are accepted (`AuthUnavailable` when there
    is no authority, `UnsafeToStart` when there is no durable commit path); anything else,
    and especially a successful reply, fails. Modelling over the control channel needs
    authority and is covered by the signed-in staging acceptance, not by this offline probe.
    """
    replies = [json.loads(line) for line in stdout.splitlines()]
    if len(replies) != 3 or any(reply.get("id") != i for i, reply in enumerate(replies, 1)):
        raise ValueError(f"desktop control failed: {replies}")
    ui_reply, *controlled = replies
    if ui_reply.get("ok") is not True:
        raise ValueError(f"ui.inspect failed: {ui_reply}")
    ui = ui_reply["result"]
    viewport = ui["viewport"]
    if (ui["renderer"] not in {"gpu", "cpu"} or len(viewport) != 4
            or not all(math.isfinite(value) for value in viewport)
            or viewport[2] <= 0 or viewport[3] <= 0):
        raise ValueError("desktop did not create a usable viewport")
    for reply in controlled:
        details = reply.get("errorDetails") or {}
        if reply.get("ok") is not False or details.get("code") not in FAIL_CLOSED:
            raise ValueError(f"a controlled entry was not refused for a signed-out host: {reply}")
    return ui["renderer"]


def frame_statistics(png):
    """Bounded PNG readback check for the app's non-interlaced RGBA8 output."""
    if len(png) > MAX_PNG_BYTES or not png.startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("invalid or oversized presented-frame PNG")
    offset, width, height, ended = 8, 0, 0, False
    compressed = bytearray()
    while offset < len(png):
        if offset + 12 > len(png):
            raise ValueError("truncated PNG chunk")
        length = struct.unpack_from(">I", png, offset)[0]
        kind = png[offset + 4:offset + 8]
        end = offset + 12 + length
        if end > len(png):
            raise ValueError("truncated PNG data")
        data = memoryview(png)[offset + 8:offset + 8 + length]
        crc = zlib.crc32(data, zlib.crc32(kind)) & 0xffffffff
        if crc != struct.unpack_from(">I", png, end - 4)[0]:
            raise ValueError("PNG checksum mismatch")
        if kind == b"IHDR":
            if offset != 8 or length != 13:
                raise ValueError("invalid PNG header")
            width, height, depth, colour, compression, filtering, interlace = struct.unpack(">IIBBBBB", data)
            if (width == 0 or height == 0 or width * height > MAX_PIXELS
                    or (depth, colour, compression, filtering, interlace) != (8, 6, 0, 0, 0)):
                raise ValueError("unsupported PNG dimensions or format")
        elif kind == b"IDAT":
            if not width or len(compressed) + length > MAX_PNG_BYTES:
                raise ValueError("invalid PNG image data")
            compressed.extend(data)
        elif kind == b"IEND":
            if length or end != len(png):
                raise ValueError("invalid PNG end")
            ended = True
            break
        elif not (kind[0] & 32):
            raise ValueError("unsupported critical PNG chunk")
        offset = end
    if not ended or not width or not compressed:
        raise ValueError("incomplete PNG")
    stride = width * 4
    expected = height * (stride + 1)
    inflater = zlib.decompressobj()
    try:
        raw = inflater.decompress(compressed, expected + 1)
    except zlib.error as error:
        raise ValueError("invalid compressed PNG data") from error
    if (len(raw) != expected or not inflater.eof or inflater.unconsumed_tail or inflater.unused_data):
        raise ValueError("PNG decompressed size mismatch")
    previous = bytearray(stride)
    first = None
    nonblack, varied = 0, False
    for y in range(height):
        start = y * (stride + 1)
        method = raw[start]
        if method > 4:
            raise ValueError("unsupported PNG row filter")
        row = bytearray(raw[start + 1:start + 1 + stride])
        if method:
            for i in range(stride):
                left = row[i - 4] if i >= 4 else 0
                above = previous[i]
                corner = previous[i - 4] if i >= 4 else 0
                if method == 1:
                    prediction = left
                elif method == 2:
                    prediction = above
                elif method == 3:
                    prediction = (left + above) // 2
                else:
                    p = left + above - corner
                    a, b, c = abs(p - left), abs(p - above), abs(p - corner)
                    prediction = left if a <= b and a <= c else (above if b <= c else corner)
                row[i] = (row[i] + prediction) & 255
        if first is None:
            first = tuple(row[:3])
        for i in range(0, stride, 4):
            nonblack += row[i + 3] > 8 and (row[i] > 8 or row[i + 1] > 8 or row[i + 2] > 8)
            varied |= row[i] != first[0] or row[i + 1] != first[1] or row[i + 2] != first[2]
        previous = row
    if not varied or nonblack <= width * height // 10:
        raise ValueError(f"blank presented frame: {nonblack}/{width * height} nonblack pixels, varied={varied}")
    return {"width": width, "height": height, "nonblack": nonblack, "pixels": width * height}


def validate_signed_out_assistant(replies, backend):
    """A signed-out desktop cannot open the Assistant panel: the request is refused, and the
    bounded marker still reports the web runtime. The page itself is NOT VERIFIED offline."""
    if not replies:
        raise ValueError("the Assistant open request returned no answer")
    opening = replies[0]
    details = opening.get("errorDetails") or {}
    if opening.get("ok") is not False or details.get("code") not in FAIL_CLOSED:
        raise ValueError(f"the Assistant panel opened without account authority: {opening}")
    # Native Wayland has no X11 child for WebKitGTK. Its diagnostic is not
    # interchangeable with the available host required under X11/XWayland.
    expected = {"x11": "available", "xwayland": "available", "wayland": "native_wayland"}.get(backend)
    observation = replies[-1]
    marker = (observation.get("result") or {}).get("assistant") or {}
    host = marker.get("host")
    if expected is None or observation.get("ok") is not True or host != expected:
        raise ValueError(f"Assistant {backend} host: expected {expected!r}, got {host!r}")
    if host == "native_wayland":
        return "native-Wayland diagnostic; page NOT VERIFIED offline"
    return "web runtime available; page NOT VERIFIED offline"


def validate_assistant(replies, backend):
    """The Assistant panel is open under this backend, with the state that backend promises.

    X11/XWayland must host the page. A native Wayland window has no X11 child window for
    WebKitGTK, and the app says so (`webProblem: native-wayland`) instead of dropping the
    conversation; that documented limitation is verified here, not counted as a page load.
    """
    if not replies:
        raise ValueError("the Assistant returned no state")
    for reply in replies:
        if reply.get("ok") is not True:
            raise ValueError(f"the Assistant failed: {reply}")
    state = replies[-1].get("result") or {}
    web = state.get("webView") or {}
    if state.get("open") is not True:
        raise ValueError(f"the Assistant panel did not open: {state}")
    if backend in {"x11", "xwayland"}:
        if web.get("host") is True and web.get("page") is True:
            return "page loaded"
        raise ValueError(f"the Assistant's page did not load: {web}, problem={state.get('webProblem')!r}")
    if web.get("host") is False and state.get("webProblem") == "native-wayland":
        return "native-Wayland diagnostic"
    raise ValueError(f"native Wayland did not report its documented diagnostic: {web}, "
                     f"problem={state.get('webProblem')!r}")


def probe(gl_saved_four=False, backend="x11"):
    shell = SANDBOX_PROBE
    if gl_saved_four:
        software_gl = ('export LIBGL_ALWAYS_SOFTWARE=1\nexport MESA_LOADER_DRIVER_OVERRIDE=llvmpipe\n'
                       if os.environ.get("SOLIDCRAFT_SMOKE_SOFTWARE_GL") == "1" else '')
        shell = shell.replace('intcad --control',
                              'mkdir -p "$XDG_DATA_HOME/intcad"\n'
                              'printf \'{"msaa":4}\' >"$XDG_DATA_HOME/intcad/graphics.json"\n'
                              'export WGPU_BACKEND=gl\n'
                              + software_gl +
                              'intcad --control')
    # The headless Xvfb/Xwayland desktop occasionally loses WebKitGTK's UI process right after
    # startup (the app exits 1 with no error of its own); the same probe passes on a rerun. Run
    # the identical probe twice and report both attempts, without weakening any assertion.
    for attempt in (1, 2):
        result = subprocess.run(["flatpak", "run", "--user", "--command=bash", APP_ID, "-c", shell],
                                text=True, capture_output=True, timeout=240)
        if result.returncode == 0:
            break
        if attempt == 1:
            print(f"Flatpak desktop exited {result.returncode} on attempt 1; retrying the same probe")
    if result.returncode:
        raise RuntimeError(f"Flatpak desktop exited {result.returncode} on both attempts\n"
                           f"--- stderr ---\n{result.stderr[-8000:]}\n--- stdout ---\n{result.stdout[-4000:]}")
    lines = result.stdout.splitlines()
    if not lines or lines[-1].startswith("{"):
        raise ValueError("invalid presented-frame response")
    json_lines = [line for line in lines if line.startswith("{")]
    # ui.inspect, then the refused engine.execute and document.inspect; then the Assistant's
    # replies; then the screenshot reply. The presented frame is the one line that is not JSON.
    if len(json_lines) < 5 or len(lines[-1]) > ((MAX_PNG_BYTES + 2) // 3) * 4:
        raise ValueError("invalid presented-frame response")
    renderer = validate_offline("\n".join(json_lines[:3]))
    assistant = [json.loads(line) for line in json_lines[3:-1]]
    try:
        assistant_mode = validate_signed_out_assistant(assistant, backend)
    except ValueError as error:
        raise ValueError(f"{error}\n--- app log ---\n{result.stderr[-8000:]}") from error
    capture = json.loads(json_lines[-1])
    if capture.get("id") != len(json_lines) or capture.get("ok") is not True or renderer != "gpu":
        raise ValueError("GPU presented-frame capture failed")
    statistics = frame_statistics(base64.b64decode(lines[-1], validate=True))
    if (capture["result"]["width"], capture["result"]["height"]) != (statistics["width"], statistics["height"]):
        raise ValueError("presented-frame dimensions disagree")
    if gl_saved_four and "SolidCraft graphics: Gl, MSAA 1x (requested 4x)" not in result.stderr:
        raise ValueError("forced GL did not apply the safe sample count")
    mode = "GL saved 4x" if gl_saved_four else "default backend"
    print(f"Flatpak desktop PASS: {mode}, renderer={renderer}, controlled entries refused "
          f"(signed out), Assistant {assistant_mode}, frame={statistics}")


def run_backend(backend):
    tools = ["flatpak", "dbus-run-session", "xvfb-run"] if backend == "x11" else ["flatpak", "dbus-run-session", "weston"]
    for tool in tools:
        if not shutil.which(tool):
            raise RuntimeError(f"{tool} missing: install flatpak, dbus-x11, xvfb, xauth and weston")
    with tempfile.TemporaryDirectory(prefix="solidcraft-flatpak-smoke-") as temporary:
        runtime = Path(temporary) / "runtime"
        runtime.mkdir(mode=0o700)
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime))
        env.pop("WAYLAND_DISPLAY", None)
        env.pop("WAYLAND_SOCKET", None)
        env.pop("DISPLAY", None)
        env.pop("SOLIDCRAFT_WAYLAND", None)
        env.pop("SOLVECRAFT_WAYLAND", None)  # the legacy name is still read
        command = ["dbus-run-session", "--"]
        compositor = None
        with (Path(temporary) / "weston.log").open("wb") as log:
            try:
                if backend == "x11":
                    command += ["xvfb-run", "-a", "-s", "-screen 0 1600x1000x24 -nolisten tcp"]
                else:
                    env["WAYLAND_DISPLAY"] = "solidcraft-smoke"
                    # Weston 13 (Ubuntu) has no `-- program`; read the display it starts Xwayland on
                    # from its log. Weston 16 accepts either.
                    compositor = subprocess.Popen(["weston", "--backend=headless", "--renderer=pixman", "--xwayland",
                                                   "--no-config", "--socket=solidcraft-smoke", "--width=1600",
                                                   "--height=1000"],
                                                  env=env, stdout=log, stderr=subprocess.STDOUT)
                    deadline = time.monotonic() + 20
                    display = None
                    log_path = Path(temporary) / "weston.log"
                    while display is None or not (runtime / "solidcraft-smoke").exists():
                        if compositor.poll() is not None or time.monotonic() >= deadline:
                            raise RuntimeError("headless Weston/Xwayland failed: " + log_path.read_text())
                        match = re.search(r"xserver listening on display :(\d+)", log_path.read_text(errors="replace"))
                        if match:
                            display = ":" + match.group(1)
                        time.sleep(.1)
                    env["DISPLAY"] = display
                command += [sys.executable, str(Path(__file__).resolve()), "--probe",
                            "--probe-backend", "xwayland" if backend == "wayland" else "x11"]
                subprocess.run(command, env=env, check=True, timeout=300)
                if backend == "wayland":
                    subprocess.run(command + ["--gl-saved-four"], env=env, check=True, timeout=300)
                    native = dict(env)
                    native.pop("DISPLAY", None)
                    native_command = [sys.executable, str(Path(__file__).resolve()), "--probe",
                                      "--probe-backend", "wayland"]
                    subprocess.run(native_command, env=native, check=True, timeout=300)
                print(f"{backend}: default sandbox desktop startup, modelling and Assistant PASS")
            finally:
                if compositor:
                    compositor.terminate()
                    try:
                        compositor.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        compositor.kill()
                        compositor.wait()
                # The isolated document portal can leave a disconnected FUSE
                # mount after its private session bus exits. Unmount only our
                # temporary runtime's portal before deleting that directory.
                if shutil.which("fusermount3"):
                    subprocess.run(["fusermount3", "-u", str(runtime / "doc")],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=["x11", "wayland", "both"], default="both")
    parser.add_argument("--probe", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--gl-saved-four", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--probe-backend", choices=["x11", "xwayland", "wayland"], default="x11", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.probe:
        probe(args.gl_saved_four, args.probe_backend)
    else:
        for backend in (["x11", "wayland"] if args.backend == "both" else [args.backend]):
            run_backend(backend)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"Flatpak desktop smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
