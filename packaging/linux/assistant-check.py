#!/usr/bin/env python3
"""Exercise the installed Linux Assistant through native X11 input.

Run from packaging/linux/installed-check.sh's offline container with the installed program:

  python3 packaging/linux/assistant-check.py --exe /usr/bin/intcad --evidence DIR [--screen]

The app gets folders of its own, so nothing of the user's is read or written, and its update
check is turned off (SOLIDCRAFT_UPDATE_CHECK=0). The caller takes the network away
from the whole container (docs/releasing.md). The executable must stay in the
launcher's process/session; no `--run-with` prefix or backgrounding wrapper is supported.

Only isolated X11 (including an eligible headless XWayland session) can pass.
`--expect diagnostic` explicitly returns BLOCKED: this script has no native
Wayland input or visibility proof. The separate legacy macOS installed check is
unchanged and supplies no native page visibility, Close, or retention evidence.
`--screen` also captures the isolated Linux display. The app's
own screenshot does not include the native page. Windows has its own check,
tools/windows-assistant-smoke.ps1.

Child wait deadlines apply after a successful spawn. The packaging workflow's
external timeout is separate; this script does not bound total runtime.

Exits 1 for FAIL, 2 for BLOCKED (missing exact native capability).
"""
import argparse
import ctypes as C
import json
import math
import os
import re
import secrets
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

FOLDERS = ("SETTINGS", "VALIDATION", "CRASH", "CONTROL", "RECOVERY")
NOTE = "solidcraft: assistant web view:"
MAX_REPLY = 1024 * 1024


class Blocked(Exception):
    """No exact native capability; never a successful smoke."""


def remaining(deadline):
    left = deadline - time.monotonic()
    if left <= 0:
        raise TimeoutError("control deadline expired")
    return left


class Control:
    """The app's control channel: JSON lines on 127.0.0.1, the token in a connection's first request."""

    def __init__(self, port, token):
        self.port, self.token, self.sock, self.count = port, token, None, 0

    def close(self):
        if self.sock is not None:
            self.sock.close()
            self.sock = None

    def call(self, method, deadline, params=None):
        self.count += 1
        request = {"id": self.count, "method": method, "params": params or {}}
        try:
            if self.sock is None:
                self.sock = socket.create_connection(("127.0.0.1", self.port), timeout=remaining(deadline))
                request["token"] = self.token
            self.sock.settimeout(remaining(deadline))
            self.sock.sendall(json.dumps(request).encode() + b"\n")
            reply = bytearray()
            while len(reply) < MAX_REPLY:
                self.sock.settimeout(remaining(deadline))
                chunk = self.sock.recv(min(65536, MAX_REPLY - len(reply)))
                if not chunk:
                    raise OSError("the app closed the control connection")
                line, end, extra = chunk.partition(b"\n")
                reply.extend(line)
                if end:
                    data = json.loads(reply)
                    if extra or not isinstance(data, dict) or data.get("id") != self.count:
                        raise ValueError("unexpected control reply")
                    return data
            raise ValueError("oversized control reply")
        except (OSError, ValueError):
            self.close()
            raise


def tail(path, lines=40):
    try:
        with Path(path).open("rb") as log:
            log.seek(0, 2)
            log.seek(max(0, log.tell() - 512 * 1024))
            text = log.read(512 * 1024).decode(errors="replace").splitlines()
    except OSError as error:
        return [f"(no log: {error})"]
    return text[-lines:]


def missing_libraries(exe):
    """The libraries the app's file names and this system lacks (Linux; empty where ldd is absent)."""
    if not sys.platform.startswith("linux") or not shutil.which("ldd"):
        return []
    try:
        _, output = bounded_command(["ldd", exe], 10, "library diagnostics")
    except (OSError, ValueError) as error:
        print(f"FAIL: {error}", flush=True)
        return []
    return [line.split()[0] for line in output.splitlines() if "not found" in line]


def kill_and_wait(proc, what):
    # Popen owns this direct child. Its kill() checks whether it has already
    # exited before signaling. No context-manager wait or daemon auto-join.
    proc.kill()
    try:
        proc.wait(timeout=2)
    except subprocess.TimeoutExpired as error:
        raise ValueError(f"{what} child {proc.pid} unreaped after SIGKILL deadline") from error


def bounded_command(command, timeout, what):
    # No pipes: a stuck child or an inherited stdout cannot hold a pipe read
    # open. The wait deadline starts after Popen returns. Read only a diagnostic excerpt.
    with tempfile.TemporaryFile() as output:
        proc = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT)
        try:
            code = proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired as error:
            kill_and_wait(proc, what)
            raise ValueError(f"{what} timed out") from error
        except BaseException:
            kill_and_wait(proc, what)
            raise
        output.seek(0)
        return code, output.read(32 * 1024).decode(errors="replace")


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


def screen_picture(path, native):
    """Capture the isolated X11 or headless XWayland display."""
    if native.headless_wayland:
        tool = ["grim", str(path)]
    else:
        tool = ["import", "-window", "root", str(path)]
    if not shutil.which(tool[0]):
        raise Blocked(f"native capture requires {tool[0]}")
    code, output = bounded_command(tool, 15, "native screen capture")
    if code or not path.is_file() or not path.stat().st_size:
        raise ValueError(f"{tool[0]} made no picture: {output.strip()[:300]}")
    return tool[0]


def result(control, method, deadline, params=None):
    reply = control.call(method, deadline, params)
    if reply.get("ok") is not True or not isinstance(reply.get("result"), dict):
        raise ValueError(f"{method} did not return an observation")
    return reply["result"]


def assistant(inspect):
    state = inspect.get("assistant")
    if (not isinstance(state, dict) or type(state.get("open")) is not bool
            or type(state.get("surface_ready")) is not bool
            or state.get("host") not in ("available", "unavailable", "native_wayland", "failed")):
        raise ValueError("invalid Assistant observation (open, surface_ready, host required)")
    return state["open"], state["surface_ready"], state["host"]


def ui_rect(control, names, deadline, backing_scale=1):
    # Ask once when names have different prefixes, so both targets and scale
    # describe the same frame.
    params = {"prefix": names[0]} if len(names) == 1 else None
    layout = result(control, "ui.rects", deadline, params)
    scale, rects = layout.get("pixelsPerPoint"), layout.get("rects")
    number = lambda n: type(n) in (float, int) and -100000 <= n <= 100000
    if not number(scale) or not 0 < scale <= 16 or not isinstance(rects, list):
        raise ValueError("invalid ui.rects observation")
    for name in names:
        for rect in reversed(rects):
            if not isinstance(rect, dict) or rect.get("name") != name:
                continue
            x, y, w, h = (rect.get(k) for k in ("x", "y", "width", "height"))
            if not all(map(number, (x, y, w, h))) or w <= 0 or h <= 0:
                raise ValueError(f"invalid {name} rectangle")
            bounds = tuple(round(p * scale / backing_scale) for p in (x, y, x + w, y + h))
            if bounds[0] >= bounds[2] or bounds[1] >= bounds[3]:
                raise ValueError(f"invalid {name} rectangle in native pixels")
            return bounds, scale / backing_scale
    return None, scale


def await_value(app, seconds, message, observe):
    deadline = time.monotonic() + seconds
    while peek_exit(app) is None and time.monotonic() < deadline:
        try:
            value = observe(deadline)
        except TimeoutError:
            break
        if value:
            return value
        time.sleep(max(0, min(0.15, deadline - time.monotonic())))
    raise ValueError(message)


def connect_control(app, control, seconds):
    def connect(deadline):
        try:
            result(control, "ui.inspect", min(deadline, time.monotonic() + 2))
        except (ConnectionRefusedError, TimeoutError):
            return None
        return True

    await_value(app, seconds, "the installed app did not answer the read-only control channel", connect)


def exercise(app, control, native, seconds, capture, built_in_agent=False):
    def inspect(deadline):
        return result(control, "ui.inspect", deadline)

    def click_rect(rect):
        x, y, right, bottom = rect
        native.click((x + right) // 2, (y + bottom) // 2)

    def open_from_entry(retained=False):
        dismissed = False

        def button(deadline):
            nonlocal dismissed
            state = inspect(deadline)
            opened, ready, host = assistant(state)
            if opened:
                raise ValueError("Assistant was already open before the entry click")
            if retained and (not ready or host != "available"):
                raise ValueError("Assistant lost retained readiness before reopen")
            if state.get("home") is True:
                if not dismissed:
                    back = ui_rect(control, ("home:back",), deadline, native.backing_scale)[0]
                    if back:
                        click_rect(back)
                        dismissed = True
                return None
            return ui_rect(control, ("rail:assistant", "titlebar:assistant"), deadline, native.backing_scale)[0]

        click_rect(await_value(app, seconds, "Home did not dismiss or no native Assistant entry was drawn", button))

    def page(deadline):
        if assistant(inspect(deadline)) == (True, True, "available"):
            rect, scale = ui_rect(control, ("agent_chat:web",), deadline, native.backing_scale)
            if rect:
                actual = native.page(rect, timeout=min(5, remaining(deadline)))
                if actual:
                    return actual, scale
        return None

    open_from_entry()
    actual, scale = await_value(app, seconds, "no current-view readiness with matching visible native page", page)
    original = native.view_identity()
    print(f"ok: Assistant page ready and visible at {actual}", flush=True)
    capture()

    if built_in_agent:
        # The agent is found inside the package and started when its panel shows; what it tells
        # the Connections page (its plans) arrives once it has answered. The other agents stay
        # offered beside it: the package carries one, it does not take the others away.
        def agent(deadline):
            return result(control, "ui.agent_chat", deadline)

        deadline = time.monotonic() + 60
        state = {}
        while time.monotonic() < deadline and peek_exit(app) is None:
            state = agent(min(deadline, time.monotonic() + 15))
            if (state.get("connect") or {}).get("plans"):
                break
            time.sleep(0.5)
        agents = state.get("agents") or []
        names = [entry.get("name") for entry in agents]
        if not agents or agents[0].get("id") != "mastra":
            raise ValueError(f"the built-in agent is not offered first ({names})")
        if state.get("currentAgent") != "mastra":
            raise ValueError(f"the built-in agent is not the conversation's agent ({state.get('currentAgent')})")
        if not (state.get("connect") or {}).get("plans"):
            raise ValueError("the built-in agent did not start out of the package or report its connections")
        # Starting it again from a stopped state: the same app, the same package, a new session.
        # The control channel's own word for Stop is `stop` (web.rs `disconnect` is the page's).
        result(control, "ui.agent_chat", time.monotonic() + 15, {"stop": True})
        state = agent(time.monotonic() + 15)
        stopped = state.get("connect") or {}
        # A stopped agent keeps the page's empty shape (its keys, no plans) but says nothing
        # about a running conversation: no plan is signed in and nothing is connected.
        if not (state.get("state") == "off" and not stopped.get("plans") and not stopped.get("connected")):
            raise ValueError(f"a stopped agent reports no connections (state {state.get('state')}, "
                             f"plans {len(stopped.get('plans') or [])}, connected {stopped.get('connected')})")
        result(control, "ui.agent_chat", time.monotonic() + 15, {"connect": True})
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline and peek_exit(app) is None:
            state = agent(min(deadline, time.monotonic() + 15))
            if (state.get("connect") or {}).get("plans"):
                break
            time.sleep(0.5)
        if not (state.get("connect") or {}).get("plans"):
            raise ValueError("the built-in agent did not start again after it stopped")
        print("ok: the built-in agent started, stopped and started again", flush=True)

    native.close_page(actual, scale)

    def hidden(deadline):
        opened, ready, host = assistant(inspect(deadline))
        if opened:
            return False
        if not ready or host != "available":
            raise ValueError("page Close lost retained Assistant readiness")
        return native.retained_hidden(original, timeout=min(5, remaining(deadline)))

    await_value(app, seconds, "page Close did not hide the retained native page while the app stayed live", hidden)
    print("ok: native page Close hid the retained Assistant", flush=True)
    open_from_entry(retained=True)
    actual, _ = await_value(app, seconds, "reopened Assistant lacked readiness or visible native page", page)
    if native.view_identity() != original:
        raise ValueError("Assistant reopened with a different native view")
    print(f"ok: retained Assistant reopened at {actual}", flush=True)

    if peek_exit(app) is not None:
        raise ValueError("the app exited before the native window close")
    native.close_window()
    wait_native_exit(app, 45)
    print("ok: native window close gave a clean exit", flush=True)


def bind(lib, name, restype, *argtypes):
    fn = getattr(lib, name)
    fn.restype, fn.argtypes = restype, argtypes
    return fn


XID, PTR, INT = C.c_ulong, C.c_void_p, C.c_int


class WindowAttributes(C.Structure):
    _fields_ = [
        *((name, INT) for name in ("x", "y", "width", "height", "border_width", "depth")),
        ("visual", PTR), ("root", XID),
        *((name, INT) for name in ("window_class", "bit_gravity", "win_gravity", "backing_store")),
        ("backing_planes", XID), ("backing_pixel", XID), ("save_under", INT),
        ("colormap", XID), ("map_installed", INT), ("map_state", INT),
        ("all_event_masks", C.c_long), ("your_event_mask", C.c_long),
        ("do_not_propagate_mask", C.c_long), ("override_redirect", INT), ("screen", PTR),
    ]


class ClientMessage(C.Structure):
    _fields_ = [
        ("type", INT), ("serial", XID), ("send_event", INT), ("display", PTR),
        ("window", XID), ("message_type", XID), ("format", INT), ("data", C.c_long * 5),
    ]


class XEvent(C.Union):
    _fields_ = [("message", ClientMessage), ("pad", C.c_long * 24)]


def socket_peer(address):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
        probe.settimeout(2)
        probe.connect(address)
        size = struct.calcsize("3i")
        pid, _, _ = struct.unpack("3i", probe.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, size))
        return pid


def isolated_display():
    """Reject any physical/remote X server; check the local socket's actual listener."""
    display = os.environ.get("DISPLAY", "")
    match = re.fullmatch(r":([0-9]{1,5})(?:\.0)?", display)
    if match is None:
        raise Blocked("a local isolated Xvfb/headless XWayland DISPLAY is required")
    path = f"/tmp/.X11-unix/X{match[1]}"
    # Xlib may choose the abstract socket before the pathname.
    peers = set()
    for address in ("\0" + path, path):
        try:
            peers.add(socket_peer(address))
        except OSError:
            pass
    if len(peers) != 1:
        raise Blocked("cannot verify the isolated X server's socket peer")
    pid, = peers
    try:
        proc = Path(f"/proc/{pid}")
        executable = Path(os.readlink(proc / "exe")).name
        if executable == "Xvfb":
            return display, False
        # Sway creates XWayland's listening socket. Accept it only when it is
        # also the actual compositor for our private, inputless Wayland session.
        wayland = Path(os.environ["XDG_RUNTIME_DIR"]) / os.environ["WAYLAND_DISPLAY"]
        compositor = socket_peer(str(wayland))
        sway = Path(f"/proc/{compositor}")
        if pid == compositor and executable == "sway":
            with (sway / "environ").open("rb") as file:
                environment = file.read(128 * 1024).split(b"\0")
            if {b"WLR_BACKENDS=headless", b"WLR_LIBINPUT_NO_DEVICES=1"}.issubset(environment):
                return display, True
    except (OSError, KeyError):
        pass
    raise Blocked("DISPLAY is not a verified Xvfb or headless Sway XWayland server")


class X11:
    backing_scale = 1

    def __init__(self):
        display, self.headless_wayland = isolated_display()
        wptr, iptr = C.POINTER(XID), C.POINTER(INT)
        try:
            x = C.CDLL("libX11.so.6")
            t = C.CDLL("libXtst.so.6")
            self.open_display = bind(x, "XOpenDisplay", PTR, C.c_char_p)
            self.close_display = bind(x, "XCloseDisplay", INT, PTR)
            self.default_root = bind(x, "XDefaultRootWindow", XID, PTR)
            self.intern = bind(x, "XInternAtom", XID, PTR, C.c_char_p, INT)
            self.free = bind(x, "XFree", INT, PTR)
            self.sync = bind(x, "XSync", INT, PTR, INT)
            self.grab = bind(x, "XGrabServer", INT, PTR)
            self.ungrab = bind(x, "XUngrabServer", INT, PTR)
            self.select_input = bind(x, "XSelectInput", INT, PTR, XID, C.c_long)
            self.check_event = bind(x, "XCheckTypedWindowEvent", INT, PTR, XID, INT, C.POINTER(XEvent))
            self.get_property = bind(x, "XGetWindowProperty", INT, PTR, XID, XID, C.c_long, C.c_long,
                                     INT, XID, wptr, iptr, wptr, wptr, C.POINTER(PTR))
            self.tree = bind(x, "XQueryTree", INT, PTR, XID, wptr, wptr, C.POINTER(wptr), C.POINTER(C.c_uint))
            self.attrs = bind(x, "XGetWindowAttributes", INT, PTR, XID, C.POINTER(WindowAttributes))
            self.translate = bind(x, "XTranslateCoordinates", INT, PTR, XID, XID, INT, INT, iptr, iptr, wptr)
            self.send = bind(x, "XSendEvent", INT, PTR, XID, INT, C.c_long, C.POINTER(XEvent))
            self.escape = bind(x, "XKeysymToKeycode", C.c_ubyte, PTR, XID)
            self.xtest = bind(t, "XTestQueryExtension", INT, PTR, iptr, iptr, iptr, iptr)
            self.motion = bind(t, "XTestFakeMotionEvent", INT, PTR, INT, INT, INT, XID)
            self.button = bind(t, "XTestFakeButtonEvent", INT, PTR, C.c_uint, INT, XID)
            # A window can disappear between observations. Do not let Xlib's default
            # handler terminate the harness instead of observing the lost window.
            callback = C.CFUNCTYPE(INT, PTR, PTR)
            self.error_handler = callback(self.x_error)
            self.set_error_handler = bind(x, "XSetErrorHandler", PTR, PTR)
        except (OSError, AttributeError) as error:
            raise Blocked(f"X11/XTEST system capability unavailable: {error}") from error
        self.dpy = self.open_display(display.encode())
        if not self.dpy:
            raise Blocked("cannot open the verified X display")
        self.previous_handler = self.set_error_handler(C.cast(self.error_handler, PTR))
        self.protocol_error = False
        self.root = self.default_root(self.dpy)
        self.owner = self.page_window = None
        self.watched_page = None
        try:
            if not self.xtest(self.dpy, *(C.byref(INT()) for _ in range(4))):
                raise Blocked("the isolated X server has no XTEST extension")
            if not self.escape(self.dpy, 0xff1b):
                raise Blocked("the isolated X keymap has no Escape keycode")
            self.wm_close = self.atom("_NET_CLOSE_WINDOW")
            if not self.wm_close or self.wm_close not in self.property(self.root, "_NET_SUPPORTED", 4):
                raise Blocked("the window manager does not advertise _NET_CLOSE_WINDOW")
        except BaseException:
            self.close()
            raise

    def close(self):
        self.close_display(self.dpy)
        self.set_error_handler(self.previous_handler)

    def x_error(self, _display, _event):
        self.protocol_error = True
        return 0

    def atom(self, name):
        return self.intern(self.dpy, name.encode(), True)

    def property(self, window, name, kind):
        actual, fmt, count, extra, data = XID(), INT(), XID(), XID(), PTR()
        atom = self.atom(name)
        if not atom:
            return ()
        try:
            status = self.get_property(self.dpy, window, atom, 0, 4096, False, kind, C.byref(actual),
                                       C.byref(fmt), C.byref(count), C.byref(extra), C.byref(data))
            if status or actual.value != kind or fmt.value != 32 or extra.value or not data:
                return ()
            return tuple(C.cast(data, C.POINTER(XID))[i] for i in range(count.value))
        finally:
            if data:
                self.free(data)

    def children(self, window):
        root, parent, children, count = XID(), XID(), C.POINTER(XID)(), C.c_uint()
        try:
            if not self.tree(self.dpy, window, C.byref(root), C.byref(parent), C.byref(children), C.byref(count)):
                return ()
            return tuple(children[i] for i in range(min(count.value, 4096)))
        finally:
            if children:
                self.free(children)

    def geometry(self, window):
        attrs = self.attributes(window)
        if attrs is None or attrs.map_state != 2:  # IsViewable
            return None
        x, y, _ = self.at(window, self.root, 0, 0)
        return (x, y, x + attrs.width, y + attrs.height)

    def attributes(self, window):
        attrs = WindowAttributes()
        return attrs if self.attrs(self.dpy, window, C.byref(attrs)) else None

    def at(self, source, dest, x, y):
        out_x, out_y, child = INT(), INT(), XID()
        if not self.translate(self.dpy, source, dest, x, y, C.byref(out_x), C.byref(out_y), C.byref(child)):
            raise ValueError("cannot translate native X11 coordinates")
        return out_x.value, out_y.value, child.value

    def find_window(self, pid):
        windows = [w for w in self.property(self.root, "_NET_CLIENT_LIST", 33)  # XA_WINDOW
                   if self.property(w, "_NET_WM_PID", 6) == (pid,) and self.geometry(w)]
        if len(windows) == 1:
            self.owner, = windows
            self.pid = pid
            return True
        return False

    def owner_bounds(self):
        if self.property(self.owner, "_NET_WM_PID", 6) != (self.pid,):
            raise ValueError("the native frame no longer belongs to the launched app")
        bounds = self.geometry(self.owner)
        if not bounds:
            raise ValueError("the owned native frame is not visible")
        return bounds

    def page(self, expected):
        self.grab(self.dpy)
        try:
            owner = self.owner_bounds()
            screen = self.geometry(self.root)
            utility = self.atom("_NET_WM_WINDOW_TYPE_UTILITY")
            pages = [(w, self.geometry(w)) for w in self.children(self.owner)
                     if utility and utility in self.property(w, "_NET_WM_WINDOW_TYPE", 4)]
            visible = [(w, bounds) for w, bounds in pages if bounds]
            if len(visible) != 1:
                return None
            window, actual = visible[0]
            wanted = tuple(edge + owner[i % 2] for i, edge in enumerate(expected))
            if not all(abs(a - b) <= 2 for a, b in zip(actual, wanted)):
                return None
            x, y, right, bottom = actual
            if not all(contains(bounds, x, y) and contains(bounds, right - 1, bottom - 1)
                       for bounds in (owner, screen)):
                return None
            try:
                self.verify_hit((x + right) // 2, (y + bottom) // 2, window)
            except ValueError:
                return None
            self.page_window = window
            return actual
        finally:
            self.ungrab(self.dpy)
            self.sync(self.dpy, False)

    def view_identity(self):
        if self.watched_page is not None:
            return None if self.page_destroyed() else self.page_window
        self.watched_page = self.page_window
        self.protocol_error = False
        self.select_input(self.dpy, self.watched_page, 1 << 17)  # StructureNotifyMask
        self.sync(self.dpy, False)
        if self.protocol_error:
            raise ValueError("cannot watch the original native view for destruction")
        return self.watched_page

    def page_destroyed(self):
        event = XEvent()
        self.sync(self.dpy, False)
        # DestroyNotify: even reusing the same XID cannot certify retention.
        return bool(self.check_event(self.dpy, self.watched_page, 17, C.byref(event)))

    def retained_hidden(self, original):
        self.owner_bounds()
        if self.page_destroyed() or original not in self.children(self.owner):
            raise ValueError("page Close destroyed or detached the original native view")
        attrs = self.attributes(original)
        if attrs is None:
            raise ValueError("the original native view disappeared")
        return attrs.map_state == 0  # IsUnmapped, not merely obscured or unviewable

    def click(self, x, y):
        self.guarded_click((x, y), self.owner)

    def guarded_click(self, point, target, *, page_bounds=None):
        # The live geometry, hit and event must describe the same server state.
        self.grab(self.dpy)
        try:
            owner = self.owner_bounds()
            screen = self.geometry(self.root)
            if page_bounds is None:
                x, y = owner[0] + point[0], owner[1] + point[1]
            else:
                if (target != self.watched_page or self.page_destroyed()
                        or target not in self.children(self.owner)):
                    raise ValueError("page Close no longer targets the original owned view")
                live = self.geometry(target)
                if live is None or live != tuple(page_bounds):
                    raise ValueError("page Close geometry moved or is stale")
                if not (contains(owner, live[0], live[1]) and contains(owner, live[2] - 1, live[3] - 1)):
                    raise ValueError("native page bounds lie outside the owned frame")
                x, y = live[0] + point[0], live[1] + point[1]
                if not contains(live, x, y):
                    raise ValueError("page Close hit is outside the owned child")
            if not contains(owner, x, y) or not contains(screen, x, y):
                raise ValueError("native input hit is outside the owned frame or live screen")
            self.verify_hit(x, y, target)
            self.protocol_error = False
            if not all((self.motion(self.dpy, 0, x, y, 0),
                        self.button(self.dpy, 1, True, 0), self.button(self.dpy, 1, False, 0))):
                raise ValueError("XTEST did not send the native click")
            self.sync(self.dpy, False)
            if self.protocol_error:
                raise ValueError("XTEST click generated an X11 protocol error")
        finally:
            self.ungrab(self.dpy)
            self.sync(self.dpy, False)

    def verify_hit(self, x, y, target):
        # Translate down to the actual leaf, not just an intermediate ancestor.
        window = self.root
        hit = []
        for _ in range(32):
            hit.append(window)
            _, _, child = self.at(self.root, window, x, y)
            if not child:
                break
            window = child
        else:
            raise ValueError("cannot resolve the native input hit window")
        if target == self.owner:
            allowed = window == self.owner
        else:
            allowed = target in hit and self.owner in hit[:hit.index(target)]
        if not allowed:
            raise ValueError("native input hit is covered or outside the owned window")

    def close_page(self, bounds, scale):
        left, _, right, _ = bounds
        # App.tsx: Close is the last control in ChatShell.Bar. The checked-in
        # page has a centered <=44rem/48rem column, 12/16px side padding,
        # and a 28px button centered in a 32px row with 8px vertical padding.
        # Limit the hit to widths where the column is not capped. Its Close
        # center is 26px from the right at this width.
        width = (right - left) / scale
        if not 200 <= width <= 700:
            raise Blocked("the native Assistant page has no verified Close hit for this width")
        self.guarded_click((right - left - round(26 * scale), round(24 * scale)),
                           self.page_window, page_bounds=bounds)

    def close_window(self):
        self.owner_bounds()
        self.protocol_error = False
        event = XEvent()
        event.message = ClientMessage(33, 0, True, self.dpy, self.owner, self.wm_close, 32,
                                      (C.c_long * 5)(0, 2, 0, 0, 0))
        if not self.send(self.dpy, self.root, False, (1 << 19) | (1 << 20), C.byref(event)):
            raise ValueError("cannot send native _NET_CLOSE_WINDOW")
        self.sync(self.dpy, False)
        if self.protocol_error:
            raise ValueError("native _NET_CLOSE_WINDOW generated an X11 protocol error")


def contains(bounds, x, y):
    return bounds is not None and bounds[0] <= x < bounds[2] and bounds[1] <= y < bounds[3]


NATIVE_CALLS = ("find_window", "page", "view_identity", "retained_hidden", "click", "close_page", "close_window")


PACKET_SIZE = 4096


def send_packet(conn, value):
    data = json.dumps(value).encode()
    if len(data) >= PACKET_SIZE or conn.send(data) != len(data):
        raise ValueError("invalid native X11 packet size")


def receive_packet(conn):
    data = conn.recv(PACKET_SIZE)
    if not data:
        raise EOFError
    if len(data) >= PACKET_SIZE:
        raise ValueError("oversized native X11 packet")
    return json.loads(data)


def x11_worker(conn):
    # Xlib performs synchronous, uncancellable I/O. Keep it out of the harness
    # so a stalled/disconnected X server cannot prevent app cleanup.
    desktop = None
    try:
        desktop = X11()
        send_packet(conn, ("ok", desktop.headless_wayland))
        while True:
            method, args = receive_packet(conn)
            if method == "close":
                break
            if method not in NATIVE_CALLS:
                raise ValueError("invalid native operation")
            send_packet(conn, ("ok", getattr(desktop, method)(*args)))
    except Blocked as error:
        send_packet(conn, ("blocked", str(error)[:400]))
    except (OSError, ValueError) as error:
        send_packet(conn, ("failed", str(error)[:400]))
    except EOFError:
        pass
    finally:
        try:
            if desktop is not None:
                desktop.close()
        finally:
            conn.close()


class BoundedX11:
    backing_scale = 1

    def __init__(self):
        # One dedicated direct child, with no implicit interpreter shutdown join.
        # After spawn, the private socket bounds waits even for incomplete replies.
        self.conn, worker = socket.socketpair(type=socket.SOCK_SEQPACKET)
        self.stopped = False
        try:
            fd = worker.fileno()
            self.proc = subprocess.Popen(
                [sys.executable, str(Path(__file__).resolve()), "--x11-worker", str(fd)],
                pass_fds=(fd,), stdin=subprocess.DEVNULL,
            )
        except OSError:
            self.conn.close()
            raise
        finally:
            worker.close()
        try:
            self.headless_wayland = self.request(None, timeout=8)
        except BaseException:
            self.close()
            raise

    def request(self, method, *args, timeout=5):
        deadline = time.monotonic() + timeout
        try:
            if method:
                self.conn.settimeout(remaining(deadline))
                send_packet(self.conn, (method, args))
            self.conn.settimeout(remaining(deadline))
            outcome, payload = receive_packet(self.conn)
        except TimeoutError as error:
            self.abort()
            raise ValueError(f"native X11 {method or 'preflight'} timed out") from error
        except (EOFError, OSError) as error:
            raise ValueError("native X11 driver disconnected") from error
        if outcome == "blocked":
            raise Blocked(payload)
        if outcome != "ok":
            raise ValueError(payload)
        return payload

    def __getattr__(self, method):
        if method not in NATIVE_CALLS:
            raise AttributeError(method)
        return lambda *args, **kwargs: self.request(method, *args, **kwargs)

    def abort(self):
        self.stopped = True
        self.conn.close()
        kill_and_wait(self.proc, "native X11 worker")

    def close(self):
        if self.stopped:
            return
        try:
            try:
                self.conn.settimeout(0.2)
                send_packet(self.conn, ("close", ()))
            except OSError:
                pass
            self.proc.wait(timeout=1)
        except subprocess.TimeoutExpired:
            self.abort()
        finally:
            self.stopped = True
            self.conn.close()


def native_driver(expect):
    if expect == "diagnostic":
        raise Blocked("no verified native Wayland input/window observation; XWayland is not its diagnostic")
    if sys.platform == "darwin":
        raise Blocked("macOS cannot prove physical WebView visibility and a scoped Close hit")
    if sys.platform.startswith("linux"):
        if not os.environ.get("DISPLAY") and os.environ.get("WAYLAND_DISPLAY"):
            raise Blocked("no verified native Wayland input/window observation")
        return BoundedX11()
    raise Blocked("no Unix native driver for this platform")


def peek_exit(app):
    # Keep the leader's PID reserved, even after it becomes a zombie. No
    # Popen.poll()/wait() until after the last group operation.
    try:
        info = os.waitid(os.P_PID, app.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
    except OSError as error:
        raise ValueError("cannot verify unreaped app leader identity") from error
    if info is not None and info.si_pid != app.pid:
        raise ValueError("unreaped app leader identity mismatch")
    return info


def wait_native_exit(app, seconds):
    deadline = time.monotonic() + seconds
    while (info := peek_exit(app)) is None:
        left = deadline - time.monotonic()
        if left <= 0:
            raise ValueError("clean native exit timed out")
        time.sleep(min(0.1, left))
    if info.si_code != os.CLD_EXITED or info.si_status != 0:
        raise ValueError(f"native window close was not a zero exit ({info.si_code=}, {info.si_status=})")


def cleanup_app(app):
    peek_exit(app)
    if os.getsid(app.pid) != app.pid or os.getpgid(app.pid) != app.pid:
        raise ValueError("cannot verify dedicated app session and group identity")
    try:
        # Only this launch's dedicated group, while its leader PID is reserved.
        # Clean native exit is observed before this; signaling is teardown only.
        os.killpg(app.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    app.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--exe", required=True, help="the installed app's program file")
    parser.add_argument("--evidence", default="assistant-check", help="folder for the log and the pictures")
    parser.add_argument("--expect", choices=("page", "diagnostic"), default="page",
                        help="diagnostic is BLOCKED until native Wayland input/visibility is supported")
    parser.add_argument("--port", type=int, default=17979)
    parser.add_argument("--wait", type=float, default=90, help="seconds to wait for the page")
    parser.add_argument("--screen", action="store_true", help="also capture the isolated display")
    parser.add_argument("--built-in-agent", action="store_true",
                        help="the package carries the built-in agent; prove it starts and stops")
    parser.add_argument("--run-with", help="unsupported on Linux; start only the installed executable")
    args = parser.parse_args()
    if not math.isfinite(args.wait) or not 0 < args.wait <= 600:
        parser.error("--wait must be between 0 and 600 seconds")

    try:
        if args.run_with is not None:
            raise Blocked("--run-with is not supported on Linux")
        if sys.platform.startswith("linux") and not all(
            hasattr(os, name) for name in ("waitid", "P_PID", "WEXITED", "WNOHANG", "WNOWAIT", "CLD_EXITED")
        ):
            raise Blocked("Linux waitid(WNOWAIT) is required for safe app cleanup")
        native = native_driver(args.expect)
    except Blocked as error:
        print(f"BLOCKED: {error}", flush=True)
        return 2
    except (OSError, ValueError) as error:
        print(f"FAIL: {error}", flush=True)
        return 1
    try:
        evidence = Path(args.evidence).resolve()
        evidence.mkdir(parents=True, exist_ok=True)
        work = Path(tempfile.mkdtemp(prefix="assistant-check-"))
    except OSError as setup_error:
        print(f"FAIL: could not set up evidence: {setup_error}", flush=True)
        try:
            native.close()
        except (OSError, ValueError) as cleanup_error:
            print(f"FAIL: could not close native driver: {cleanup_error}", flush=True)
        return 1
    env = dict(os.environ)
    for name in FOLDERS:
        env[f"SOLIDCRAFT_{name}_DIR"] = str(work / name.lower())
    token = secrets.token_hex(24)
    env["SOLIDCRAFT_CONTROL_TOKEN"] = token
    # A release build asks GitHub for a newer release at every start. A package check never
    # starts one with the network open (docs/releasing.md): the caller takes the network away
    # (--run-with a sandbox, a container without one), and this turns the question off as well.
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    env["SOLIDCRAFT_NO_DESKTOP_INTEGRATION"] = "1"
    env.setdefault("NO_AT_BRIDGE", "1")  # no accessibility bus in a bare container; only a warning otherwise

    log_path = evidence / "app.log"
    command = [args.exe, "--control", str(args.port)]
    control = Control(args.port, token)
    app = None
    status = 1

    def notes():
        return [line for line in tail(log_path, 2000) if NOTE in line]

    def capture():
        # The egui frame does not include the embedded native page.
        frame = work / "app-frame.png"
        shot = control.call("ui.screenshot", time.monotonic() + 15, {"path": str(frame)})
        if shot.get("ok") is not True or not frame.is_file() or frame.stat().st_size == 0:
            raise ValueError("could not capture app-frame.png")
        shutil.move(str(frame), evidence / frame.name)
        print("ok: captured app-frame.png", flush=True)
        if args.screen:
            time.sleep(1.5)  # the page paints after it reports in
            screen = work / "screen.png"
            tool = screen_picture(screen, native)
            shutil.move(str(screen), evidence / screen.name)
            print(f"ok: captured screen.png with {tool}", flush=True)

    try:
        print("starting: " + shlex.join(command), flush=True)
        with log_path.open("wb") as log:
            app = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        await_value(app, 90, "no unique visible top-level window owned by the launched app",
                    lambda deadline: native.find_window(app.pid, timeout=min(5, remaining(deadline))))
        connect_control(app, control, 15)
        exercise(app, control, native, args.wait, capture, args.built_in_agent)
        if args.built_in_agent:
            # The app takes the agent with it: no copy of the packaged Node may outlive it.
            leftovers = [str(folder) for folder in agent_folders(args.exe)
                         if folder.is_dir() and node_running(folder / "node")]
            if leftovers:
                raise ValueError("the agent did not stop with the app (still running: "
                                 + ", ".join(leftovers) + ")")
        if next((work / "crash").rglob("report.json"), None) is not None:
            raise ValueError("a crash report was written")
        if notes():
            raise ValueError("the app reported a web view problem")
        status = 0
    except Blocked as error:
        print(f"BLOCKED: {error}", flush=True)
        status = 2
    except (OSError, ValueError) as error:
        print(f"FAIL: {error}", flush=True)
    finally:
        control.close()
        # Cleanup is never evidence. Signal only this run's verified group.
        if app is not None:
            try:
                cleanup_app(app)
            except (OSError, ValueError, subprocess.TimeoutExpired) as error:
                print(f"FAIL: could not reap the launched process: {error}", flush=True)
                status = 1
        try:
            native.close()
        except (OSError, ValueError) as error:
            print(f"FAIL: {error}", flush=True)
            status = 1
        for line in notes()[:10]:
            print("  " + line[:300], flush=True)
        if status == 1:
            lacking = missing_libraries(args.exe)
            if lacking:
                print("libraries the app's file names and this system lacks: " + " ".join(lacking), flush=True)
            print(f"--- last lines of {log_path}", flush=True)
            for line in tail(log_path):
                print("  " + line[:300], flush=True)
        shutil.rmtree(work, ignore_errors=True)
    return status


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--x11-worker":
        with socket.socket(fileno=int(sys.argv[2])) as conn:
            if conn.family != socket.AF_UNIX or conn.type != socket.SOCK_SEQPACKET:
                sys.exit("invalid private X11 worker socket")
            x11_worker(conn)
        sys.exit(0)
    sys.exit(main())
