#!/usr/bin/env python3
"""Start an installed copy of the app, open the Assistant and see that its page came up.

For the package checks in .github/workflows/release.yml: the app here is the one a package put
on the machine (a .deb or .rpm in a bare container, the app copied out of the macOS disk image),
not a build tree. A build tree says nothing about what a package brings with it.

  python3 packaging/assistant-check.py --exe /usr/bin/solidcraft --evidence DIR [--screen]

The app gets folders of its own, so nothing of the user's is read or written, and its update
check is turned off (SOLIDCRAFT_UPDATE_CHECK=0). A package check also takes the network away
from the app it starts (docs/releasing.md): `--run-with` is where a sandbox goes, as in
`--run-with "sandbox-exec -f packaging/macos/no-network.sb"`; on Linux the whole container has
none. `--expect built-in` is for a system where the app is meant to draw the built-in
conversation (no web view there).
`--screen` also saves a picture of the whole screen, which is the only picture the page is in:
the app's own screenshot holds what the app draws, and the page is another window's. Leave it
out on a person's machine. Windows has its own check, tools/windows-assistant-smoke.ps1.

Prints one `ok:` or `FAIL:` line per check and exits 1 when any failed.
"""
import argparse
import json
import os
import secrets
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
    parser.add_argument("--expect", choices=("page", "built-in"), default="page")
    parser.add_argument("--port", type=int, default=17979)
    parser.add_argument("--wait", type=float, default=90, help="seconds to wait for the page")
    parser.add_argument("--screen", action="store_true", help="also save a picture of the whole screen")
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
        # The app answers on its control channel.
        deadline = time.monotonic() + 90
        reply = None
        while time.monotonic() < deadline and app.poll() is None:
            try:
                reply = control.call("ui.agent_chat", {"open": True})
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

        # The Assistant opens, and its page (or, where asked, the built-in conversation) is there.
        opened = time.monotonic()
        deadline = opened + args.wait
        while time.monotonic() < deadline and app.poll() is None:
            # The start page covers the workspace, and the panel with it: the page is made only
            # once the panel is drawn. A launch with --control keeps the start page hidden, but
            # it has been seen showing all the same, so it is put away here.
            if (control.call("ui.home", {}).get("result") or {}).get("open") is True:
                print("the start page was showing: hiding it", flush=True)
                control.call("ui.home", {"open": False})
            state = control.call("ui.agent_chat", {}).get("result") or {}
            web = state.get("webView") or {}
            if web.get("page") or web.get("host") is False:
                break
            time.sleep(0.5)
        web = state.get("webView") or {}
        print(f"webView: {json.dumps(web)} after {time.monotonic() - opened:.1f} s", flush=True)
        checks.check(state.get("open") is True, "the Assistant panel is open")
        if args.expect == "page":
            checks.check(web.get("host") is True, "the app made its web view")
            checks.check(web.get("page") is True, "the Assistant's page loaded and reported in")
        else:
            checks.check(web.get("host") is False, "no web view here, so the panel draws the built-in conversation")

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
            try:
                code = app.wait(timeout=45)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait()
        else:
            code = app.returncode
        if reached:
            checks.check(code == 0, f"the app quit cleanly (exit code {code})")
            # A crash leaves a folder with a report.json (crates/engine/src/crash.rs).
            crashes = sorted(str(p.parent.name) for p in (work / "crash").rglob("report.json"))
            checks.check(not crashes, "no crash report was written" + (f" ({', '.join(crashes[:5])})" if crashes else ""))
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
