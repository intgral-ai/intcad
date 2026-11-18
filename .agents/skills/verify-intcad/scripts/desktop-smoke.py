#!/usr/bin/env python3
"""Verify a native Intcad model and .sc round trip on an isolated desktop instance."""

import argparse
import json
import math
import os
from pathlib import Path
import platform
import secrets
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[4]
LENGTH, WIDTH, HEIGHT = 40, 30, 20
EXPECTED_VOLUME = LENGTH * WIDTH * HEIGHT


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def wait_for(check, description, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.15)
    raise RuntimeError(f"Timed out waiting for {description}")


def stop(app, call):
    if app.poll() is not None:
        return
    try:
        call("app.quit", force=True)
        app.wait(timeout=8)
    except (OSError, RuntimeError, subprocess.TimeoutExpired):
        if app.poll() is None:
            app.terminate()
        try:
            app.wait(timeout=5)
        except subprocess.TimeoutExpired:
            app.kill()
            app.wait(timeout=5)


def drive(call, output):
    def command(name, **params):
        return call("engine.execute", command=name, params=params)

    def measure():
        bodies = command("inspect.measure")["bodies"]
        require(len(bodies) == 1, f"Expected one body, measured {len(bodies)}")
        body = bodies[0]
        require(body["closed"], "Expected a closed solid")
        volume = body["volume_mm3"]
        require(math.isclose(volume, EXPECTED_VOLUME, rel_tol=0, abs_tol=0.01),
                f"Expected {EXPECTED_VOLUME} mm³, measured {volume} mm³")
        box = body["bbox"]
        dimensions = [end - start for start, end in zip(box["min"], box["max"])]
        require(len(dimensions) == 3 and all(
            math.isclose(actual, expected, rel_tol=0, abs_tol=0.01)
            for actual, expected in zip(dimensions, (LENGTH, WIDTH, HEIGHT))
        ), f"Expected {LENGTH} × {WIDTH} × {HEIGHT} mm, measured {dimensions}")
        return volume

    require(not command("inspect.measure")["bodies"], "Expected a blank design")
    command("sketch.create", plane="XY")
    command("sketch.rectangle.two_point", p0=[0, 0], p1=[LENGTH, WIDTH])
    command("solid.extrude", distance=f"{HEIGHT} mm")
    before = measure()

    design = output / "block.sc"
    command("file.save_as", path=str(design), cache=False)
    require(design.is_file() and design.stat().st_size > 0, "Save did not produce a design")
    original_id = call("document.revision")["document_id"]
    call("ui.documents", action="close")
    require(not command("inspect.measure")["bodies"], "The original design is still open and active")
    call("ui.file", path=str(design))
    wait_for(lambda: not call("ui.inspect")["file_busy"], "the desktop Open operation")
    reopened_id = call("document.revision")["document_id"]
    require(reopened_id and reopened_id != original_id, "Open did not load a different document")
    after = measure()

    render = output / "reopened.png"
    call("ui.render", path=str(render), timeout=30)
    with render.open("rb") as image:
        require(image.read(8) == b"\x89PNG\r\n\x1a\n", "Model capture is not a PNG")
    return before, after


def run(binary, output, revision):
    with tempfile.TemporaryDirectory(prefix="intcad-verify-") as scratch, \
            (output / "app.log").open("w", encoding="utf-8") as log, \
            (output / "control.jsonl").open("w", encoding="utf-8", buffering=1) as evidence:
        token = secrets.token_hex(32)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        env = {
            **os.environ,
            "SOLIDCRAFT_CONTROL_TOKEN": token,
            "SOLIDCRAFT_UPDATE_CHECK": "0",
        }
        for store in ("SETTINGS", "RECOVERY", "CRASH", "CONTROL", "CANDIDATE", "VALIDATION"):
            env[f"SOLIDCRAFT_{store}_DIR"] = str(Path(scratch) / store.lower())

        def record(data):
            evidence.write(json.dumps(data) + "\n")

        def call(method, *, timeout=10, **params):
            request = {"id": 1, "method": method, "params": params}
            with socket.create_connection(("127.0.0.1", port), timeout=timeout) as conn:
                conn.sendall((json.dumps({**request, "token": token}) + "\n").encode())
                with conn.makefile("rb") as replies:
                    line = replies.readline()
            require(line, f"{method} returned no reply")
            reply = json.loads(line)
            record({"request": request, "reply": reply})
            require(reply.get("ok") is True, f"{method} failed: {reply.get('error')}")
            return reply["result"]

        record({"source_commit": revision, "platform": platform.platform(), "binary": str(binary)})
        app = subprocess.Popen([str(binary), "--control", str(port)], cwd=scratch, env=env,
                               stdout=log, stderr=subprocess.STDOUT)
        try:
            def ready():
                require(app.poll() is None, f"Intcad exited with status {app.returncode}")
                try:
                    viewport = call("ui.inspect", timeout=1).get("viewport")
                except (ConnectionRefusedError, TimeoutError):
                    return False
                return viewport and all(dimension > 0 for dimension in viewport[2:4])

            wait_for(ready, "an authenticated desktop viewport", seconds=45)
            build = call("ui.help")["commit"]
            require(len(build) >= 7 and revision.startswith(build),
                    f"Running build {build!r} does not match clean checkout {revision}")
            before, after = drive(call, output)
        finally:
            stop(app, call)
            record({"app_exit_status": app.returncode})
        require(app.returncode == 0, f"Intcad quit with status {app.returncode}")
    print(f"PASS {revision}: {before:.6f} mm³ before save, {after:.6f} mm³ after reopen")
    print(f"Evidence: {output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--evidence", required=True, type=Path, help="a new directory outside the checkout")
    args = parser.parse_args()
    output = args.evidence.resolve()
    try:
        require(not git("status", "--porcelain", "--untracked-files=no"), "Commit tracked edits before running")
        require(ROOT not in output.parents and output != ROOT, "Keep evidence outside the checkout")
        binary = args.binary.resolve(strict=True)
        output.mkdir(mode=0o700, parents=True, exist_ok=False)
        run(binary, output, git("rev-parse", "HEAD"))
    except (KeyError, OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"Desktop smoke failed: {error}\nEvidence directory: {output}\n")


if __name__ == "__main__":
    main()
