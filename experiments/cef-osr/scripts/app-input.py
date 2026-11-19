#!/usr/bin/env python3
"""Input checks against the running IntCAD with the CEF Assistant.

Uses the app's own rectangles (`ui.rects`) to click the composer, then checks
the app's state for what was typed, the X clipboard for what was copied, and the
app's log for a CEF popup surface.

    scripts/app-input.py <control-port> [--log PATH] [--scope] [--move-origin] [--pid PID]

`--scope` asserts the fixed in-app probe and therefore needs the app started with
`SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE=1` and `--log` pointing at its output; without
`--scope` the probe results are NOT_RUN, not an inferred pass.
"""
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path


class Control:
    def __init__(self, port, token):
        sock = socket.create_connection(("127.0.0.1", port), timeout=5)
        sock.settimeout(60)
        self.file = sock.makefile("rw", encoding="utf-8", newline="\n")
        self.token, self.n = token, 0

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


def xdo(*args):
    subprocess.run(["xdotool", *args], check=False, stderr=subprocess.DEVNULL)


SCOPE_MARKER = "SCOPE {"


def log_text(path):
    try:
        return path.read_text(errors="replace") if path and path.exists() else ""
    except OSError:
        return ""


def scope_messages(text):
    messages = []
    for line in text.splitlines():
        at = line.find(SCOPE_MARKER)
        if at < 0:
            continue
        try:
            value = json.loads(line[at + len("SCOPE "):].strip())
        except ValueError:
            continue
        if isinstance(value, dict):
            messages.append(value)
    return messages


def latest_scope(text, action=None):
    for value in reversed(scope_messages(text)):
        if action is None or value.get("action") == action:
            return value
    return None


def oversize_actions(text):
    """Actions whose probe message was replaced because it would not fit one `/ipc` line."""
    return {value.get("of") for value in scope_messages(text) if value.get("action") == "scope_oversize"}


def browser_receipts(text):
    receipts = []
    for line in text.splitlines():
        at = line.find("CEF_BROWSER ")
        if at < 0:
            continue
        try:
            value = json.loads(line[at + len("CEF_BROWSER "):].strip())
        except ValueError:
            continue
        if isinstance(value, dict):
            receipts.append(value)
    return receipts


def process_tree(root_pid):
    """Every descendant of `root_pid`, including zygote-parented renderers."""
    parents = {}
    proc = Path("/proc")
    for entry in (proc.iterdir() if proc.is_dir() else []):
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text(errors="replace")
            right = stat.rfind(")")
            parents[int(entry.name)] = int(stat[right + 1:].split()[1])
        except (OSError, ValueError, IndexError):
            continue
    children = {}
    for pid, ppid in parents.items():
        children.setdefault(ppid, []).append(pid)
    seen, stack = set(), [root_pid]
    while stack:
        for child in children.get(stack.pop(), []):
            if child not in seen:
                seen.add(child)
                stack.append(child)
    return seen


def main():
    port = int(sys.argv[1])
    log_path = Path(sys.argv[sys.argv.index("--log") + 1]) if "--log" in sys.argv else None
    scope_requested = "--scope" in sys.argv
    token = Path("/tmp/intcad-shot-token").read_text().strip()
    control = Control(port, token)
    control.call("ui.agent_chat", {"open": True})
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        state = control.call("ui.agent_chat").get("result") or {}
        if (state.get("webView") or {}).get("page"):
            break
        time.sleep(0.4)
    time.sleep(1.5)

    rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
    ppp = rects.get("pixelsPerPoint") or 1.0
    web = next((r for r in rects.get("rects", []) if r["name"] == "agent_chat:web"), None)
    if not web:
        print(json.dumps({"error": "no agent_chat:web rect", "rects": rects}))
        return 1
    geom = subprocess.run(["xdotool", "getwindowgeometry", "--shell", "@0"], capture_output=True, text=True)
    wid = subprocess.run(["xdotool", "search", "--name", "Intcad"], capture_output=True, text=True).stdout.split()
    window = wid[0] if wid else ""
    if window:
        geom = subprocess.run(["xdotool", "getwindowgeometry", "--shell", window], capture_output=True, text=True)
    origin = {}
    for line in geom.stdout.splitlines():
        if "=" in line:
            key, value = line.strip().split("=", 1)
            origin[key] = value
    ox, oy = int(origin.get("X", 0)), int(origin.get("Y", 0))

    def point(fx, fy):
        return (
            str(round(ox + (web["x"] + web["width"] * fx) * ppp)),
            str(round(oy + (web["y"] + web["height"] * fy) * ppp)),
        )

    if window:
        xdo("windowactivate", "--sync", window)
        time.sleep(0.3)

    results = {}
    # --- typing reaches the page's composer ---------------------------------
    # The composer sits just above the bottom bar of the panel (measured from the
    # app's own screenshot: ~0.885 of the web rect's height).
    cx, cy = point(0.45, 0.885)
    xdo("mousemove", cx, cy, "click", "1")
    time.sleep(0.6)
    xdo("type", "--delay", "40", "clip-abc")
    time.sleep(1.0)
    state = json.dumps(control.call("ui.agent_chat").get("result") or {}, ensure_ascii=False)
    # The control state does not carry the composer draft; the clipboard round-trip below is
    # what proves the page received the typing.
    results["state_exposes_composer"] = "clip-abc" in state
    # What the page received is what Ctrl+A/Ctrl+C gives back through the bridge.
    xdo("key", "--clearmodifiers", "ctrl+a")
    time.sleep(0.2)
    xdo("key", "--clearmodifiers", "ctrl+c")
    time.sleep(1.0)
    first = subprocess.run(["xclip", "-o", "-selection", "clipboard"], capture_output=True, text=True, timeout=10)
    results["typed_copied_out"] = first.stdout.strip()

    # --- system clipboard -> page (Chinese) --------------------------------
    subprocess.run(["bash", "-c", "printf '你好，世界' | xclip -selection clipboard >/dev/null 2>&1 &"], check=False)
    time.sleep(0.6)
    xdo("key", "--clearmodifiers", "ctrl+a")
    time.sleep(0.2)
    xdo("key", "--clearmodifiers", "ctrl+v")
    time.sleep(1.2)
    state = json.dumps(control.call("ui.agent_chat").get("result") or {}, ensure_ascii=False)
    results["state_exposes_composer_after_paste"] = "你好，世界" in state

    # --- page selection -> system clipboard --------------------------------
    xdo("key", "--clearmodifiers", "ctrl+a")
    time.sleep(0.2)
    xdo("key", "--clearmodifiers", "ctrl+c")
    time.sleep(1.0)
    copied = subprocess.run(["xclip", "-o", "-selection", "clipboard"], capture_output=True, text=True, timeout=10)
    results["copied_out"] = copied.stdout.strip()

    # --- a native <select> in the page gets a CEF popup surface ------------
    sx, sy = point(0.03, 0.02)
    scope = latest_scope(log_text(log_path), "rects")
    if scope and isinstance(scope.get("select"), dict):
        box = scope["select"]
        sx, sy = (str(round(ox + (web["x"] + box["x"]) * ppp)), str(round(oy + (web["y"] + box["y"]) * ppp)))
    xdo("mousemove", sx, sy, "click", "1")
    time.sleep(1.2)
    if log_path and log_path.exists():
        results["popup_logged"] = "popup surface" in log_text(log_path)
    results["web_rect"] = web
    results["click"] = [int(cx), int(cy)]

    # --- fixed scope probe: explicit opt-in, real app receipts only --------
    # The app emits `SCOPE {json}` for DOM rects and denied/cancelled results,
    # and `CEF_BROWSER {json}` with action=popup_denied for target=_blank. This
    # script cannot turn those on itself; the app must start with
    # SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE=1 and `--scope` must ask for the checks.
    scope_results = {}
    if not scope_requested:
        scope_results["status"] = "NOT_RUN"
        scope_results["detail"] = "pass --scope (app started with SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE=1) to assert the fixed probe"
    elif log_path is None:
        scope_results["status"] = "BLOCKED"
        scope_results["detail"] = "--scope needs --log to read the app's SCOPE/CEF_BROWSER receipts"
    else:
        rects_scope = latest_scope(log_text(log_path), "rects")
        if not isinstance(rects_scope, dict):
            oversize = "rects" in oversize_actions(log_text(log_path))
            scope_results["status"] = "FAIL" if oversize else "BLOCKED"
            scope_results["detail"] = (
                "the rects probe message exceeded the app's /ipc log cap and became a scope_oversize receipt"
                if oversize
                else "fixed scope probe did not report bounded DOM rects"
            )
        else:
            def click_scope(name):
                box = rects_scope.get(name)
                if not isinstance(box, dict):
                    return False
                gx = str(round(ox + (web["x"] + box["x"] + box["w"] / 2) * ppp))
                gy = str(round(oy + (web["y"] + box["y"] + box["h"] / 2) * ppp))
                xdo("mousemove", gx, gy, "click", "1")
                time.sleep(0.8)
                return True

            def count_scope(action):
                return sum(1 for message in scope_messages(log_text(log_path)) if message.get("action") == action)

            def wait_scope(action, before, timeout=4.0):
                end = time.monotonic() + timeout
                while time.monotonic() < end:
                    messages = [m for m in scope_messages(log_text(log_path)) if m.get("action") == action]
                    if len(messages) > before:
                        return messages[-1]
                    time.sleep(0.2)
                return None

            sequence = {}
            before = count_scope("window_open")
            if click_scope("open"):
                message = wait_scope("window_open", before)
                sequence["window_open_denied"] = bool(message and message.get("denied") is True)
            else:
                sequence["window_open_denied"] = False
            before = len(browser_receipts(log_text(log_path)))
            if click_scope("target"):
                denied = False
                end = time.monotonic() + 4.0
                while time.monotonic() < end:
                    fresh = browser_receipts(log_text(log_path))[before:]
                    if any(receipt.get("action") == "popup_denied" for receipt in fresh):
                        denied = True
                        break
                    time.sleep(0.2)
                sequence["target_blank_popup_denied"] = denied
            else:
                sequence["target_blank_popup_denied"] = False
            before = count_scope("dialog_current")
            if click_scope("dialog"):
                message = wait_scope("dialog_current", before)
                sequence["dialog_cancelled"] = bool(message and message.get("cancelled") is True)
            else:
                sequence["dialog_cancelled"] = False
            before = count_scope("devtools_entry")
            if click_scope("devtools"):
                message = wait_scope("devtools_entry", before)
                sequence["devtools_entry_denied"] = bool(message and message.get("denied") is True) and "DevTools window" not in log_text(log_path)
            else:
                sequence["devtools_entry_denied"] = False
            before = count_scope("fullscreen")
            if click_scope("fullscreen"):
                entered = wait_scope("fullscreen", before)
                if entered and entered.get("active") is True:
                    before = count_scope("fullscreen")
                    if click_scope("fullscreen"):
                        exited = wait_scope("fullscreen", before)
                        sequence["fullscreen_enter_exit"] = bool(exited and exited.get("active") is False)
                    else:
                        sequence["fullscreen_enter_exit"] = False
                else:
                    sequence["fullscreen_enter_exit"] = False
            else:
                sequence["fullscreen_enter_exit"] = False
            scope_results["results"] = sequence
            scope_results["status"] = "PASS" if sequence and all(sequence.values()) else "FAIL"
            scope_results["detail"] = json.dumps(sequence)
    results["scope_probe"] = scope_results

    if "--move-origin" in sys.argv:
        if not window:
            results["origin_change"] = {"status": "FAIL", "detail": "no app window to move"}
        else:
            xdo("windowmove", window, str(ox + 37), str(oy + 23))
            time.sleep(0.6)
            moved = {}
            for line in subprocess.run(["xdotool", "getwindowgeometry", "--shell", window], capture_output=True, text=True).stdout.splitlines():
                if "=" in line:
                    key, value = line.strip().split("=", 1)
                    moved[key] = value
            new_ox, new_oy = int(moved.get("X", ox)), int(moved.get("Y", oy))
            web_after = subprocess.run(["xdotool", "search", "--name", "Intcad"], capture_output=True, text=True).stdout.split()
            moved_ok = [new_ox, new_oy] != [ox, oy] and bool(web_after)
            results["origin_change"] = {
                "before": [ox, oy],
                "after": [new_ox, new_oy],
                "window_present": bool(web_after),
                "status": "PASS" if moved_ok else "FAIL",
                "detail": "the app-control path must recompute the pixel-aligned origin, not cache it",
            }
            ox, oy = new_ox, new_oy
    else:
        results["origin_change"] = {"status": "NOT_RUN", "detail": "pass --move-origin to exercise a window move"}

    if "--pid" in sys.argv:
        app_pid = int(sys.argv[sys.argv.index("--pid") + 1])
        renderers = []
        for pid in sorted(process_tree(app_pid)):
            try:
                cmdline = (Path("/proc") / str(pid) / "cmdline").read_bytes().replace(b"\0", b" ").decode("utf-8", "replace")
            except OSError:
                continue
            if "--type=renderer" in cmdline:
                renderers.append(pid)
        one_renderer = len(renderers) == 1
        results["process_scope"] = {
            "renderers": renderers,
            "one_renderer": one_renderer,
            "status": "PASS" if one_renderer else "FAIL",
            "detail": f"{len(renderers)} renderer(s) in the app's process tree (zygote descendants included)",
        }
    else:
        results["process_scope"] = {"status": "NOT_RUN", "detail": "no --pid supplied; CEF process scoping not asserted"}
    print(json.dumps(results, ensure_ascii=False))
    failed = [name for name, value in (results.get("scope_probe") or {}).get("results", {}).items() if not value]
    if (results.get("scope_probe") or {}).get("status") in ("FAIL", "BLOCKED"):
        failed.append("scope_probe")
    if (results.get("origin_change") or {}).get("status") == "FAIL":
        failed.append("origin_change")
    if (results.get("process_scope") or {}).get("status") == "FAIL":
        failed.append("process_scope")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
