#!/usr/bin/env python3
"""Start an installed copy of the app, open the Assistant and see that its page came up.

For the package checks in .github/workflows/release.yml: the app here is the one a package put
on the machine (a .deb or .rpm in a bare container, the app copied out of the macOS disk image),
not a build tree. A build tree says nothing about what a package brings with it.

  python3 packaging/assistant-check.py --exe /usr/bin/intcad --evidence DIR [--screen]

The app gets folders of its own, so nothing of the user's is read or written, and its update
check is turned off (SOLIDCRAFT_UPDATE_CHECK=0). A package check also takes the network away
from the app it starts (docs/releasing.md): `--run-with` is where a sandbox goes, as in
`--run-with "sandbox-exec -f packaging/macos/no-network.sb"`; on Linux the whole container has
none. `--expect diagnostic` is for a run where the Assistant is meant to say why it has no
web view (a native Wayland window, or a missing WebView2 runtime); it must still open the
panel, not crash, and print the reason.
`--built-in-agent` is for a package that carries the built-in agent (a test package made with
--with-mastra-agent): the app must offer that agent alone and start it out of the package when
the Assistant opens. No message is sent.
`--screen` also saves a picture of the whole screen, which is the only picture the page is in:
the app's own screenshot holds what the app draws, and the page is another window's. Leave it
out on a person's machine. Windows has its own check, tools/windows-assistant-smoke.ps1.

Prints one `ok:` or `FAIL:` line per check and exits 1 when any failed.
"""
import argparse
import json
import os
import secrets
import signal
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

FOLDERS = ("SETTINGS", "VALIDATION", "CRASH", "CONTROL", "RECOVERY")
NOTE = "solidcraft: assistant web view:"


class Control:
    """The app's control channel: JSON lines on 127.0.0.1, the token in a connection's first request."""

    def __init__(self, port, token):
        self.port, self.token, self.file, self.count = port, token, None, 0

    def call(self, method, params=None):
        if self.file is None:
            sock = socket.create_connection(("127.0.0.1", self.port), timeout=5)
            sock.settimeout(60)
            self.file = sock.makefile("rw", encoding="utf-8", newline="\n")
            first = True
        else:
            first = False
        self.count += 1
        request = {"id": self.count, "method": method, "params": params or {}}
        if first:
            request["token"] = self.token
        try:
            self.file.write(json.dumps(request) + "\n")
            self.file.flush()
            line = self.file.readline()
        except OSError:
            self.file = None
            raise
        if not line:
            self.file = None
            raise OSError("the app closed the control connection")
        return json.loads(line)


class Checks:
    def __init__(self):
        self.failed = 0

    def check(self, passed, what):
        print(("ok: " if passed else "FAIL: ") + what, flush=True)
        if not passed:
            self.failed += 1
        return passed

    def note(self, what):
        """Record what this probe cannot prove on a signed-out host; never counts as a pass."""
        print("NOT VERIFIED: " + what, flush=True)


def tail(path, lines=40):
    try:
        text = Path(path).read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as error:
        return [f"(no log: {error})"]
    return text[-lines:]


def missing_libraries(exe):
    """The libraries the app's file names and this system lacks (Linux; empty where ldd is absent)."""
    if not sys.platform.startswith("linux") or not shutil.which("ldd"):
        return []
    result = subprocess.run(["ldd", exe], capture_output=True, text=True, check=False)
    return [line.split()[0] for line in result.stdout.splitlines() if "not found" in line]


def agent_folders(exe):
    """The packaged agent folders an installed program may have, as the app resolves them."""
    exe = Path(exe).resolve()
    here = exe.parent
    return [here.parent / "lib" / "intcad" / "mastra-agent",       # Linux packages and AppImage
            here.parent / "Resources" / "mastra-agent",            # macOS .app
            here / "mastra-agent"]                                 # beside a Windows program


def node_running(program):
    """Whether a process runs this exact Node program. A cheap probe, not a full listing."""
    if not Path(program).is_file():
        return False
    pgrep = shutil.which("pgrep")
    if not pgrep:
        return False
    done = subprocess.run([pgrep, "-f", "--", str(program)], capture_output=True, text=True, check=False)
    return done.returncode == 0


def screen_picture(path):
    """A picture of the whole screen, with whatever tool this system has; the tool's name or None."""
    if sys.platform == "darwin":
        tools = [["screencapture", "-x", str(path)]]
    elif os.environ.get("WAYLAND_DISPLAY"):
        tools = [["grim", str(path)]]
    else:
        tools = [["import", "-window", "root", str(path)]]
    for tool in tools:
        if shutil.which(tool[0]):
            done = subprocess.run(tool, capture_output=True, text=True, check=False)
            if done.returncode == 0 and path.is_file() and path.stat().st_size > 0:
                return tool[0]
            print(f"{tool[0]} made no picture: {done.stderr.strip()[:300]}", flush=True)
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--exe", required=True, help="the installed app's program file")
    parser.add_argument("--evidence", default="assistant-check", help="folder for the log and the pictures")
    parser.add_argument("--expect", choices=("page", "diagnostic"), default="page")
    parser.add_argument("--port", type=int, default=17979)
    parser.add_argument("--wait", type=float, default=90, help="seconds to wait for the page")
    parser.add_argument("--screen", action="store_true", help="also save a picture of the whole screen")
    parser.add_argument("--built-in-agent", action="store_true", help="the package carries the built-in agent: expect it alone, started")
    parser.add_argument("--run-with", default="", help='a command to start the app with: a sandbox, "arch -x86_64"')
    args = parser.parse_args()

    evidence = Path(args.evidence).resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="assistant-check-"))
    env = dict(os.environ)
    for name in FOLDERS:
        env[f"SOLIDCRAFT_{name}_DIR"] = str(work / name.lower())
    token = secrets.token_hex(24)
    env["SOLIDCRAFT_CONTROL_TOKEN"] = token
    # A release build asks GitHub for a newer release at every start. A package check never
    # starts one with the network open (docs/releasing.md): the caller takes the network away
    # (--run-with a sandbox, a container without one), and this turns the question off as well.
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    env.setdefault("NO_AT_BRIDGE", "1")  # no accessibility bus in a bare container; only a warning otherwise

    checks = Checks()
    log_path = evidence / "app.log"
    command = [*shlex.split(args.run_with), args.exe, "--control", str(args.port)]
    print("starting: " + " ".join(command), flush=True)
    with open(log_path, "wb") as log:
        try:
            app = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
        except OSError as error:
            checks.check(False, f"the installed app starts ({error})")
            return 1

    control = Control(args.port, token)
    state, reached = {}, False
    try:
        # The app answers on its control channel. `ui.inspect` is the bounded, allowed probe.
        deadline = time.monotonic() + 90
        reply = None
        while time.monotonic() < deadline and app.poll() is None:
            try:
                reply = control.call("ui.inspect", {})
                if reply.get("ok") is True and (reply.get("result") or {}).get("assistant"):
                    break
            except (OSError, ValueError):
                time.sleep(0.5)
        reached = reply is not None and reply.get("ok") is True
        if not checks.check(reached, "the installed app starts and answers on its control channel"):
            if app.poll() is not None:
                print(f"the app ended at once, exit code {app.returncode}", flush=True)
            else:
                print(f"last reply: {json.dumps(reply)[:500]}", flush=True)
            return 1

        # Opening the panel is a controlled entry: a signed-out host must be refused. The bounded
        # marker still reports the web runtime; the page itself cannot be opened without account
        # authority, so it is recorded as not verified rather than claimed.
        opened = time.monotonic()
        deadline = opened + args.wait
        marker = {}
        refusal = None
        while time.monotonic() < deadline and app.poll() is None:
            try:
                opening = control.call("ui.agent_chat", {"open": True})
            except (OSError, ValueError):
                opening = None
            if opening is not None and opening.get("ok") is not True:
                refusal = opening
            marker = (control.call("ui.inspect", {}).get("result") or {}).get("assistant") or {}
            if marker.get("host") == "available":
                break
            time.sleep(0.5)
        print(f"Assistant marker: {json.dumps(marker)} after {time.monotonic() - opened:.1f} s", flush=True)
        details = (refusal or {}).get("errorDetails") or {}
        checks.check((refusal or {}).get("ok") is False
                     and details.get("code") in {"AuthUnavailable", "UnsafeToStart"},
                     "the Assistant panel refuses to open without account authority")
        if args.expect == "page":
            checks.check(marker.get("host") == "available", "the app reports its web runtime")
            checks.note("the Assistant's page loaded offline (needs account authority to open)")
        else:
            checks.check(marker.get("host") == "unavailable", "the app reports no web runtime here")
            checks.note("the Assistant's page is absent offline (no host)")
        checks.note("the Assistant panel is open (controlled entry, refused while signed out)")

        if args.built_in_agent:
            checks.note("the built-in agent starts, reports connections and starts again "
                        "(the agent control entries need account authority)")
        # Pictures: what the app draws, and (asked for) the whole screen, which has the page in it.
        frame = evidence / "app-frame.png"
        try:
            shot = control.call("ui.screenshot", {"path": str(frame)})
        except (OSError, ValueError) as error:
            shot = {"ok": False, "error": str(error)}
        checks.check(shot.get("ok") is True and frame.is_file(), "the app drew a frame (app-frame.png)")
        if args.screen:
            time.sleep(1.5)  # the page paints after it reports in
            tool = screen_picture(evidence / "screen.png")
            checks.check(tool is not None, f"a picture of the screen was taken (screen.png, {tool or 'no tool'})")
    except (OSError, ValueError) as error:
        checks.check(False, f"the app kept answering ({error})")
    finally:
        # Quit as the app's own tests do, and see how it ended.
        code = None
        if app.poll() is None:
            try:
                control.call("app.quit", {"force": True})
            except (OSError, ValueError):
                pass
            if app.poll() is None:
                app.terminate()
            try:
                code = app.wait(timeout=45)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait()
                code = app.returncode
        else:
            code = app.returncode
        if reached:
            checks.note(f"the app quit cleanly (controlled quit is refused signed out; exit code {code})")
            if code not in (0, -signal.SIGTERM):
                checks.check(False, f"the app stopped on terminate without a crash (exit code {code})")
            # A crash leaves a folder with a report.json (crates/engine/src/crash.rs).
            crashes = sorted(str(p.parent.name) for p in (work / "crash").rglob("report.json"))
            checks.check(not crashes, "no crash report was written" + (f" ({', '.join(crashes[:5])})" if crashes else ""))
            if args.built_in_agent:
                # The app takes the agent with it: no copy of the packaged Node may outlive it.
                leftovers = [str(folder) for folder in agent_folders(Path(args.exe))
                             if folder.is_dir() and node_running(folder / "node")]
                checks.check(not leftovers, "the agent stopped with the app"
                             + (f" (still running: {', '.join(leftovers)})" if leftovers else ""))
        notes = [line for line in tail(log_path, 2000) if NOTE in line]
        if args.expect == "page" and reached:
            checks.check(not notes, "the app reported no web view problem")
        for line in notes[:10]:
            print("  " + line[:300], flush=True)
        if checks.failed:
            lacking = missing_libraries(args.exe)
            if lacking:
                print("libraries the app's file names and this system lacks: " + " ".join(lacking), flush=True)
            print(f"--- last lines of {log_path}", flush=True)
            for line in tail(log_path):
                print("  " + line[:300], flush=True)
        shutil.rmtree(work, ignore_errors=True)
    return 1 if checks.failed else 0


if __name__ == "__main__":
    sys.exit(main())
