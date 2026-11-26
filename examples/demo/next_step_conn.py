#!/usr/bin/env python3
"""One action of the built-in agent's Connections page over the control channel, for a sign-in
on a machine without a screen (next_step_ab.sh serve). Prints what the page is then told.

Usage: next_step_conn.py OUTDIR status
       next_step_conn.py OUTDIR login <plan id>        (anthropic, openai-codex, …)
       next_step_conn.py OUTDIR code <plan id> <code>  (the code the sign-in page showed)
       next_step_conn.py OUTDIR cancel
The port is NSM_PORT (default 9493); the token is read from OUTDIR/token.
"""
import json, os, socket, sys, time

out, what = sys.argv[1], sys.argv[2]
port = int(os.environ.get("NSM_PORT", "9493"))
token = open(os.path.join(out, "token")).read().strip()
sock = socket.create_connection(("127.0.0.1", port))
f = sock.makefile("rw")


calls = 0


def call(params):
    global calls
    calls += 1
    req = {"id": calls, "method": "ui.agent_chat", "params": params}
    if calls == 1:
        req["token"] = token
    f.write(json.dumps(req) + "\n")
    f.flush()
    while True:
        msg = json.loads(f.readline())
        if msg.get("id") == calls:
            return msg


actions = {
    "status": None,
    "login": lambda: {"type": "conn_login", "id": sys.argv[3]},
    "code": lambda: {"type": "conn_code", "id": sys.argv[3], "code": sys.argv[4]},
    "cancel": lambda: {"type": "conn_cancel"},
}
action = actions[what]
r = call({"open": True} if action is None else {"connection": action()})
if "error" in r:
    print("refused:", r["error"])
    raise SystemExit(1)
time.sleep(1.5)
connect = (call({}).get("result") or {}).get("connect")
print(json.dumps(connect, indent=1, ensure_ascii=False))
