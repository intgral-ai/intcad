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
        self._init(sock, token)

    @classmethod
    def from_socket(cls, sock, token):
        control = cls.__new__(cls)
        control._init(sock, token)
        return control

    def _init(self, sock, token):
        sock.settimeout(60)
        self.sock = sock
        self.file = sock.makefile("rw", encoding="utf-8", newline="\n")
        self.token, self.n = token, 0

    def close(self):
        """Release the stream and socket this control owns; safe to repeat."""
        file, self.file = getattr(self, "file", None), None
        sock, self.sock = getattr(self, "sock", None), None
        if file is not None:
            try:
                file.close()
            except OSError:
                pass
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass

    def __enter__(self):
        return self

    def __exit__(self, *_exception):
        self.close()
        return False

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


# Rectangles the app's fixed hook must expose for this script's consumer.
CONSUMER_RECTS = ("open", "target", "dialog", "devtools", "fullscreen", "select")


def scope_schema_problem(value):
    """Why the merged rects/rects_extra receipts cannot feed this consumer."""
    if not isinstance(value, dict) or value.get("action") != "rects":
        return "not a rects receipt"
    missing = [name for name in CONSUMER_RECTS if not isinstance(value.get(name), dict)]
    return "missing consumer rectangle(s): " + ", ".join(missing) if missing else None


def consumer_rects(text, action=None):
    """The main rects receipt merged with the separately bounded extra targets."""
    base = latest_scope(text, "rects")
    if not isinstance(base, dict):
        return base
    extra = latest_scope(text, "rects_extra")
    if isinstance(extra, dict):
        base = dict(base)
        for name in ("dialog", "devtools", "fullscreen"):
            if isinstance(extra.get(name), dict):
                base[name] = extra[name]
    return base


def _aggregate_status(statuses):
    rank = {"PASS": 0, "NOT_RUN": 1, "BLOCKED": 2, "FAIL": 3}
    if not statuses:
        return "NOT_RUN"
    worst = max(rank.get(status, 4) for status in statuses)
    return {0: "PASS", 1: "NOT_RUN", 2: "BLOCKED", 3: "FAIL"}.get(worst, "FAIL")


def result_report(results):
    """Turn live checks into bounded statuses. A present wrong value outranks a
    missing observation; missing checks never become PASS."""
    tests = {}

    def entry(name, status, detail):
        tests[name] = {"status": status, "detail": str(detail)[:160]}

    if "typed_copied_out" not in results:
        entry("typed_clipboard_round_trip", "NOT_RUN", "typed clipboard round-trip was not observed")
    else:
        typed = results["typed_copied_out"]
        entry("typed_clipboard_round_trip", "PASS" if typed == "clip-abc" else "FAIL", repr(typed)[:80])
    if "copied_out" not in results:
        entry("copied_out_unicode", "NOT_RUN", "Unicode clipboard round-trip was not observed")
    else:
        copied = results["copied_out"]
        entry("copied_out_unicode", "PASS" if copied == "你好，世界" else "FAIL", repr(copied)[:80])
    if "popup_logged" not in results:
        entry("popup_surface_logged", "NOT_RUN", "popup surface was not observed")
    else:
        popup = results["popup_logged"]
        entry("popup_surface_logged", "PASS" if popup is True else "FAIL", repr(popup)[:80])
    scope = results.get("scope_probe") or {}
    entry("scope_probe", scope.get("status", "NOT_RUN"), scope.get("detail", "scope probe not run"))
    origin = results.get("origin_change") or {}
    entry("origin_change", origin.get("status", "NOT_RUN"), origin.get("detail", "window move not exercised"))
    process = results.get("process_scope") or {}
    entry("process_scope", process.get("status", "NOT_RUN"), process.get("detail", "no pid supplied"))
    for name, value in tests.items():
        if value["status"] not in ("PASS", "FAIL", "BLOCKED", "NOT_RUN"):
            value["status"] = "FAIL"
    status = _aggregate_status([value["status"] for value in tests.values()])
    return {"status": status, "checks": tests}


def result_exit(report):
    return 0 if report.get("status") == "PASS" else (1 if report.get("status") == "FAIL" else 2)


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
    if "--self-test" in sys.argv:
        return _run_self_tests()
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
        rects_scope = consumer_rects(log_text(log_path))
        if not isinstance(rects_scope, dict):
            oversize = "rects" in oversize_actions(log_text(log_path))
            scope_results["status"] = "FAIL" if oversize else "BLOCKED"
            scope_results["detail"] = (
                "the rects probe message exceeded the app's /ipc log cap and became a scope_oversize receipt"
                if oversize
                else "fixed scope probe did not report bounded DOM rects"
            )
        elif (consumer_problem := scope_schema_problem(rects_scope)) is not None:
            scope_results["status"] = "FAIL"
            scope_results["detail"] = consumer_problem
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
    report = result_report(results)
    results["report"] = report
    print(json.dumps(results, ensure_ascii=False))
    return result_exit(report)


def _run_self_tests():
    tests = []

    def check(function):
        tests.append(function)

    def passing():
        results = {
            "typed_copied_out": "clip-abc",
            "copied_out": "你好，世界",
            "popup_logged": True,
            "scope_probe": {"status": "PASS", "detail": "ok"},
            "origin_change": {"status": "PASS", "detail": "moved"},
            "process_scope": {"status": "PASS", "detail": "one renderer"},
        }
        report = result_report(results)
        if report["status"] != "PASS" or result_exit(report) != 0:
            raise AssertionError(f"complete results must PASS: {report}")

    def missing_is_not_pass():
        report = result_report({})
        if report["status"] != "NOT_RUN" or result_exit(report) != 2:
            raise AssertionError(f"missing checks must be NOT_RUN/2: {report}")

    def wrong_outranks_missing():
        report = result_report({
            "typed_copied_out": "",
            "scope_probe": {"status": "NOT_RUN", "detail": "not run"},
            "origin_change": {"status": "NOT_RUN", "detail": "not run"},
            "process_scope": {"status": "NOT_RUN", "detail": "not run"},
        })
        if report["status"] != "FAIL" or result_exit(report) != 1:
            raise AssertionError(f"a present wrong value must FAIL over NOT_RUN: {report}")

    def socket_closes_on_exception():
        left, right = socket.socketpair()
        control = Control.from_socket(left, "token")
        try:
            with control:
                raise RuntimeError("boom")
        except RuntimeError:
            pass
        if control.file is not None or control.sock is not None or left.fileno() != -1:
            raise AssertionError("the control socket/file were not released on exception")
        control.close()
        right.close()

    def hook_matches_consumer():
        source = (Path(__file__).resolve().parents[3] / "apps/solidcraft/src/assistant_cef.rs").read_text(encoding="utf-8")
        start = source.find("SCOPE_PROBE_JS")
        probe = source[start:start + 20000]
        for needle in ("action: 'rects'", "action: 'rects_extra'", "__cef_scope_dialog", "__cef_scope_devtools", "__cef_scope_fullscreen"):
            if needle not in probe:
                raise AssertionError(f"the fixed hook no longer emits {needle}")
        if "rects_extra" not in probe:
            raise AssertionError("the extra consumer targets are not separately bounded")
        merged = consumer_rects("SCOPE {\"action\":\"rects\",\"open\":{},\"target\":{},\"select\":{}} \n"
                               "SCOPE {\"action\":\"rects_extra\",\"dialog\":{},\"devtools\":{},\"fullscreen\":{}}")
        problem = scope_schema_problem(merged)
        if problem is not None:
            raise AssertionError(f"the merged consumer rectangles are incomplete: {problem}")

    def no_global_process_actions():
        source = Path(__file__).read_text(encoding="utf-8").split("def _run_self_tests():")[0]
        for needle in ("os.kill", "killpg", "pkill", "killall", ".terminate()"):
            if needle in source:
                raise AssertionError(f"the script must not use {needle}")

    for test in (passing, missing_is_not_pass, wrong_outranks_missing, socket_closes_on_exception, hook_matches_consumer, no_global_process_actions):
        check(test)
    failures = 0
    for test in tests:
        try:
            test()
        except Exception as error:
            failures += 1
            print(f"FAIL {test.__name__}: {type(error).__name__}: {error}")
        else:
            print(f"PASS {test.__name__}")
    print(f"self-test: {len(tests) - failures}/{len(tests)} passed, {failures} failed")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
