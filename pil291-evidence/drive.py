#!/usr/bin/env python3
"""Drive the desktop app through its control channel for PIL-291 acceptance.

  python3 drive.py screenshot OUT.png
  python3 drive.py exec COMMAND '{"json": "params"}'
  python3 drive.py call METHOD '{"json": "params"}'

The port and token come from PIL291_PORT / PIL291_TOKEN, else the discovery files
under ~/.local/share/solidcraft/control/.
"""

import json
import os
import socket
import sys
import time

PORT = int(os.environ.get("PIL291_PORT", "0"))
TOKEN = os.environ.get("PIL291_TOKEN", "")


def control_dir():
    base = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return os.path.join(base, "solidcraft", "control")


def resolve():
    global PORT, TOKEN
    if not PORT:
        with open(os.path.join(control_dir(), "current.port")) as f:
            PORT = int(f.read().strip())
    if not TOKEN:
        with open(os.path.join(control_dir(), f"{PORT}.token")) as f:
            TOKEN = f.read().strip()
    return PORT, TOKEN


def call(method, params=None, timeout=900):
    port, token = resolve()
    sock_path = os.environ.get("PIL291_SOCK")
    if sock_path:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(timeout)
        s.connect(sock_path)
    else:
        s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    f = s.makefile("rw")
    f.write(json.dumps({"id": 1, "method": method, "params": params or {}, "token": token}) + "\n")
    f.flush()
    line = f.readline()
    s.close()
    reply = json.loads(line)
    if not reply.get("ok"):
        raise RuntimeError(f"{method}: {reply.get('error')}")
    return reply.get("result")


def execute(command, params=None):
    return call("engine.execute", {"command": command, "params": params or {}})


def wait_ready(seconds=120):
    end = time.time() + seconds
    while time.time() < end:
        try:
            call("ui.inspect", {}, timeout=5)
            return
        except Exception:
            time.sleep(0.5)
    raise RuntimeError("the app did not answer on the control channel")


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    verb = sys.argv[1]
    if verb == "info":
        print(json.dumps({"port": resolve()[0]}))
    elif verb == "screenshot":
        print(json.dumps(call("ui.screenshot", {"path": sys.argv[2]})))
    elif verb == "exec":
        print(json.dumps(execute(sys.argv[2], json.loads(sys.argv[3]) if len(sys.argv) > 3 else {})))
    elif verb == "call":
        print(json.dumps(call(sys.argv[2], json.loads(sys.argv[3]) if len(sys.argv) > 3 else {})))
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
