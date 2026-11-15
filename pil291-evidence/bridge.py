#!/usr/bin/env python3
"""Forward JSON-line control requests from a unix socket to the app's loopback socket.

Run this inside the app's network namespace when the app is started under
`unshare -rn` (the app's 127.0.0.1 is not reachable from the host namespace).

  python3 bridge.py /tmp/opencode/control-46101.sock 46101
"""

import json
import os
import socket
import sys

path = sys.argv[1]
port = int(sys.argv[2])
try:
    os.unlink(path)
except FileNotFoundError:
    pass
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(path)
server.listen(8)
print(f"bridge: {path} -> 127.0.0.1:{port}", flush=True)

while True:
    conn, _ = server.accept()
    with conn:
        f = conn.makefile("rw")
        line = f.readline()
        if not line:
            continue
        try:
            up = socket.create_connection(("127.0.0.1", port), timeout=900)
            uf = up.makefile("rw")
            uf.write(line)
            uf.flush()
            reply = uf.readline()
            up.close()
            f.write(reply)
        except Exception as error:  # noqa: BLE001 - forwarded to the caller
            f.write(json.dumps({"id": None, "ok": False, "error": str(error)}) + "\n")
        f.flush()
