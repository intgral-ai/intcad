#!/usr/bin/env python3
"""Open the Assistant in a running IntCAD and save the app's own screenshot.

The CEF page is drawn by the app, so `ui.screenshot` includes it: the picture
proves the page is a texture in the window, not a separate one.

    scripts/app-shot.py <control-port> <out.png> [--wait 20]
"""
import json
import sys
import time
from pathlib import Path

import socket


class Control:
    def __init__(self, port, token):
        sock = socket.create_connection(("127.0.0.1", port), timeout=5)
        sock.settimeout(60)
        self.file = sock.makefile("rw", encoding="utf-8", newline="\n")
        self.token = token
        self.n = 0

    def call(self, method, params=None):
        self.n += 1
        request = {"id": self.n, "method": method, "params": params or {}}
        if self.n == 1:
            request["token"] = self.token
        self.file.write(json.dumps(request) + "\n")
        self.file.flush()
        reply = self.file.readline()
        if not reply:
            raise SystemExit("the control channel closed")
        return json.loads(reply)


def main():
    port = int(sys.argv[1])
    target = Path(sys.argv[2])
    wait = float(sys.argv[sys.argv.index("--wait") + 1]) if "--wait" in sys.argv else 20.0
    token = Path(f"/tmp/intcad-shot-token").read_text().strip() if Path("/tmp/intcad-shot-token").exists() else ""
    control = Control(port, token)
    control.call("ui.agent_chat", {"open": True})
    deadline = time.monotonic() + wait
    state = {}
    while time.monotonic() < deadline:
        state = control.call("ui.agent_chat").get("result") or {}
        if (state.get("webView") or {}).get("page"):
            break
        time.sleep(0.4)
    time.sleep(2.0)
    shot = control.call("ui.screenshot", {"path": str(target)})
    print(json.dumps({"webView": state.get("webView"), "screenshot": shot.get("ok")}))
    control.call("app.quit", {"force": True})


if __name__ == "__main__":
    main()
