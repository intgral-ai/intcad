#!/usr/bin/env python3
"""Every built-in theme through the real app, with a screenshot and a colour check.

Launches IntCAD (CEF Assistant), opens the panel, then for each theme file in
assets/styles/ui/themes: sets it through the app's own control channel, waits for
the page to repaint, saves the app's screenshot, and compares the page's painted
background with the surface colour the app reports for that theme.

    scripts/theme-matrix.py <exe> [--out DIR] [--ui-scales 1,1.25,1.5,2] [--themes a,b,...]
    scripts/theme-matrix.py --self-test

The colour check catches the two faults this work fixes: a page that keeps its own
palette instead of the window's, and an sRGB texture that is decoded twice (the
painted background comes out darker than the token). Scales are driven through the
authenticated `ui.prefs {"ui_scale": ...}` path, not an X11 environment override.
Page evidence arrives as small `SCOPE` messages (rects, fonts, tokens, roles,
swatches), each with its own size ceiling under the app's 600-character `/ipc` log
cap. The native rail is measured from the app screenshot, the app page draws fixed
analytic swatch patches whose screenshot pixels are compared with their declared
and computed CSS colours, and a page-level `#swatch` control exists only in the
standalone fixture. `--self-test` runs the colour/alpha/rail/swatch/size logic
without launching an app.
"""
import hashlib
import json
import math
import os
import re
import secrets
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
THEMES = sorted(p.stem for p in (ROOT / "assets/styles/ui/themes").glob("*.json"))
FOLDERS = ("SETTINGS", "VALIDATION", "CRASH", "CONTROL", "RECOVERY")


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
            raise OSError("control channel closed")
        return json.loads(reply)


def read_png(path):
    import struct
    import zlib

    data = Path(path).read_bytes()
    i, idat = 8, b""
    while i < len(data):
        length = struct.unpack(">I", data[i:i + 4])[0]
        kind, chunk = data[i + 4:i + 8], data[i + 8:i + 8 + length]
        i += 12 + length
        if kind == b"IHDR":
            w, h, _depth, color = struct.unpack(">IIBB", chunk[:10])
        elif kind == b"IDAT":
            idat += chunk
        elif kind == b"IEND":
            break
    raw = zlib.decompress(idat)
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color]
    stride = w * channels
    out, prev, p = bytearray(), bytearray(stride), 0
    for _ in range(h):
        kind = raw[p]
        p += 1
        line = bytearray(raw[p:p + stride])
        p += stride
        for x in range(stride):
            a = line[x - channels] if x >= channels else 0
            b = prev[x]
            c = prev[x - channels] if x >= channels else 0
            if kind == 1:
                line[x] = (line[x] + a) & 255
            elif kind == 2:
                line[x] = (line[x] + b) & 255
            elif kind == 3:
                line[x] = (line[x] + ((a + b) >> 1)) & 255
            elif kind == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        out += line
        prev = line
    return w, h, channels, bytes(out)


def pixel(png, x, y):
    w, h, ch, px = png
    x, y = min(max(x, 0), w - 1), min(max(y, 0), h - 1)
    o = (y * w + x) * ch
    return tuple(px[o:o + 3])


def hexed(rgb):
    return "#%02x%02x%02x" % tuple(rgb)


def css_rgb(value):
    """Resolve the CSS colours the page reports (hex or rgb()/rgba())."""
    if not isinstance(value, str):
        return None
    text = value.strip().lower()
    if re.fullmatch(r"#[0-9a-f]{6}(?:[0-9a-f]{2})?", text):
        return tuple(int(text[index:index + 2], 16) for index in (1, 3, 5))
    match = re.fullmatch(r"rgba?\(([^)]*)\)", text)
    if not match:
        return None
    parts = [part.strip() for part in match.group(1).split(",")]
    if len(parts) < 3:
        return None
    try:
        return tuple(int(round(float(part))) for part in parts[:3])
    except ValueError:
        return None


def css_alpha(value):
    """Alpha of a CSS colour (hex, #rrggbbaa, rgb()/rgba(), transparent)."""
    if not isinstance(value, str):
        return None
    text = value.strip().lower()
    if text == "transparent":
        return 0.0
    short = re.fullmatch(r"#([0-9a-f]{6})([0-9a-f]{2})?", text)
    if short:
        return int(short.group(2), 16) / 255.0 if short.group(2) else 1.0
    match = re.fullmatch(r"rgba?\(([^)]*)\)", text)
    if not match:
        return None
    parts = [part.strip() for part in match.group(1).split(",")]
    if len(parts) == 4:
        try:
            return float(parts[3])
        except ValueError:
            return None
    return 1.0 if len(parts) >= 3 else None


def norm_colour(value):
    return value.strip().lower() if isinstance(value, str) else None


def palette_colours(colors):
    """Every palette entry as (r, g, b, alpha): alpha is part of a themed colour."""
    resolved = set()
    for value in colors.values():
        if not isinstance(value, str):
            continue
        rgb, alpha = css_rgb(value), css_alpha(value)
        if rgb is not None and alpha is not None:
            resolved.add(rgb + (round(alpha, 3),))
    return resolved


def matches_palette(value, colours):
    """A painted colour matches only when RGB *and* alpha match a palette entry."""
    rgb = css_rgb(value)
    alpha = css_alpha(value)
    if rgb is None or alpha is None:
        return False
    return any(entry[:3] == rgb and abs(entry[3] - alpha) <= 0.02 for entry in colours)


def relative_luminance(rgb):
    """WCAG relative luminance, to prove a swatch set contains a real midtone."""
    def channel(value):
        v = float(value) / 255.0
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4

    r, g, b = (channel(value) for value in rgb[:3])
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


# Roles the shipped Assistant page really exposes. Its rail and swatch are native chrome,
# so the rail is measured from the app screenshot and a page swatch only exists in the
# standalone probe fixture.
REQUIRED_ROLES = ("prose", "label", "textarea", "live")


def page_token_check(messages, colors):
    message = messages.get("tokens")
    if not isinstance(message, dict):
        return ("BLOCKED", "the page did not report its resolved CSS variables")
    tokens = message.get("tokens") or {}
    mapping = {
        "background": "window",
        "background2": "surface",
        "border": "borderStrong",
        "accent": "accent",
        "selected": "selected",
        "foreground": "text",
    }
    problems = []
    for token, key in mapping.items():
        page = norm_colour(tokens.get(token))
        app = norm_colour(colors.get(key))
        if page is None:
            problems.append(f"page --{token} is missing")
        elif app is None:
            problems.append(f"app palette has no {key}")
        elif page != app:
            problems.append(f"--{token} page {page} != app {key} {app}")
    return ("FAIL" if problems else "PASS", "; ".join(problems) or "page CSS variables carry the app's theme tokens")


def page_transparency_check(messages):
    message = messages.get("tokens")
    if not isinstance(message, dict):
        return ("BLOCKED", "the page did not report its resolved CSS variables")
    problems = []
    for name in ("html_background", "body_background"):
        alpha = css_alpha(message.get(name))
        if alpha is None:
            problems.append(f"{name} {message.get(name)!r} is not a resolvable colour")
        elif alpha > 0.01:
            problems.append(f"{name} is opaque ({message.get(name)})")
    return ("FAIL" if problems else "PASS", "; ".join(problems) or "html/body stay transparent over the native backing")


def page_roles_check(messages, colors):
    message = messages.get("roles")
    if not isinstance(message, dict):
        return ("BLOCKED", "the page did not report its computed role colours")
    missing = [name for name in REQUIRED_ROLES if message.get(name) is None]
    colours = palette_colours(colors)
    problems = []
    for name in REQUIRED_ROLES:
        if name not in missing and not matches_palette(message[name], colours):
            problems.append(f"{name} {message[name]} is not a painted palette colour with palette alpha")
    # A page-side swatch is optional test instrumentation, never evidence for the app's
    # native swatch; it is checked when reported so a wrong one cannot hide.
    swatch = message.get("swatch")
    if swatch is not None and not matches_palette(swatch, colours):
        problems.append(f"swatch {swatch} is not a painted palette colour with palette alpha")
    # Check every present value before reporting unobserved coverage. The hook uses
    # null for an absent element; a present empty/malformed colour is wrong styling.
    if problems:
        return ("FAIL", "; ".join(problems))
    if missing:
        return ("BLOCKED", "coverage not observed: no rendered " + ", ".join(missing) + " element at probe time")
    detail = "real page roles paint palette colours with palette alpha"
    if swatch:
        detail += "; page swatch is fixture instrumentation, not the app's native swatch"
    return ("PASS", detail)


def page_fonts_check(messages):
    message = messages.get("fonts")
    if not isinstance(message, dict):
        return ("BLOCKED", "the page did not report its computed font sizes")
    expected = {"root": 12.0, "prose": 12.5, "textarea": 12.5, "label": 12.5}
    missing = [name for name in expected if message.get(name) is None]
    problems = []
    for name, want in expected.items():
        if name in missing:
            continue
        value = message[name]
        if (not isinstance(value, (int, float)) or isinstance(value, bool)
                or not math.isfinite(value) or abs(value - want) > 0.1):
            problems.append(f"{name} font {value!r} != {want:g}")
    if problems:
        return ("FAIL", "; ".join(problems))
    if missing:
        return ("BLOCKED", "no real " + ", ".join(missing) + " element measured")
    return ("PASS", "root 12, real prose/textarea/label 12.5")


SCOPE_ACTIONS = ("rects", "fonts", "tokens", "roles", "swatches")


def parse_scope_lines(path):
    """Latest `SCOPE {json}` message per action, not a transcript or a URL dump."""
    latest = {}
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as handle:
            for line in handle:
                marker = "SCOPE {"
                at = line.find(marker)
                if at < 0:
                    continue
                try:
                    value = json.loads(line[at + len("SCOPE "):].strip())
                except ValueError:
                    continue
                if isinstance(value, dict) and isinstance(value.get("action"), str):
                    latest[value["action"]] = value
    except OSError:
        return {}
    return latest


def scope_for_theme(path, theme, timeout=2.0, actions=SCOPE_ACTIONS):
    """Wait, bounded, for the page to report every split message for `theme`."""
    end = time.monotonic() + timeout
    while True:
        matched = {action: message for action, message in parse_scope_lines(path).items() if message.get("theme") == theme}
        if all(action in matched for action in actions):
            return matched
        if time.monotonic() >= end:
            return matched
        time.sleep(0.2)


def rounded_pixel(value):
    """Match the CEF host's f32::round edge alignment (ties away from zero)."""
    return math.floor(value + 0.5) if value >= 0 else math.ceil(value - 0.5)


def window_pixel_point(x, y, ppp):
    """ui.rects points -> ui.screenshot pixels, with NO OS/screen translation.

    control.rs returns window points and ctx.pixels_per_point(); lib.rs saves the
    viewport Screenshot image directly. Neither contract includes a screen origin.
    """
    return rounded_pixel(x * ppp), rounded_pixel(y * ppp)


def web_pixel_bounds(web, ppp):
    """CEF aligns both placement edges, not only its width (assistant_cef.rs)."""
    return (*window_pixel_point(web["x"], web["y"], ppp),
            *window_pixel_point(web["x"] + web["width"], web["y"] + web["height"], ppp))


def page_pixel_point(web, ppp, x, y):
    """Browser getBoundingClientRect CSS DIP -> aligned window image pixels."""
    left, top = window_pixel_point(web["x"], web["y"], ppp)
    return rounded_pixel(left + x * ppp), rounded_pixel(top + y * ppp)


def surface_pixel(png, web, ppp):
    left, top, right, bottom = web_pixel_bounds(web, ppp)
    x, y = rounded_pixel(left + (right - left) * 0.5), rounded_pixel(top + (bottom - top) * 0.62)
    if not (0 <= x < png[0] and 0 <= y < png[1]):
        raise ValueError(f"surface sample ({x}, {y}) is outside the window screenshot")
    return pixel(png, x, y)


def rail_sample_point(state, web, ppp, rail_dip):
    """Where the native strip beside the page is, from the control state's real `side`.

    `ui.agent_chat` reports `side` ("left"/"right"); the page-facing `web::state` object is a
    different payload with a `left` boolean, so it must not be used here.
    """
    left = state.get("side") == "left"
    half = max(1.0, float(rail_dip or 8.0) / 2.0)
    x0, y0, x1, y1 = web_pixel_bounds(web, ppp)
    return rounded_pixel(x1 + half * ppp if left else x0 - half * ppp), rounded_pixel((y0 + y1) / 2.0)


def rail_pixel_check(png, state, web, ppp, rail_dip, expected):
    """The native strip beside the page shares its themed backing surface.

    Floating, the page leaves a rail-wide strip at the panel's inner edge for the drag;
    docked, the frame margin shows the same fill. Either way the pixel is native.
    web::frame/backing use Tokens.panel, exposed as theme.colors.surface by the
    theme bridge. theme.colors.panel is Tokens.panel_header, NOT this backing.
    """
    x, y = rail_sample_point(state, web, ppp, rail_dip)
    if not (0 <= x < png[0] and 0 <= y < png[1]):
        return ("FAIL", f"native strip sample ({x}, {y}) is outside the window screenshot")
    actual = pixel(png, x, y)
    want = css_rgb(expected)
    delta = max((abs(a - b) for a, b in zip(want, actual)), default=255) if want else 255
    return ("PASS" if want and delta <= 2 else "FAIL",
            f"native strip {hexed(actual)} at ({x}, {y}) vs native surface {expected} delta={delta}")


# Exact fixed analytic fixtures, never inferred from a receipt or a test image.
# Parity with the real hook is checked on the production validation path as well
# as in self-tests: a changed/unavailable source contract cannot silently PASS.
EXPECTED_SWATCH_COLOURS = ("#808080", "#336699", "#e08040")


def page_swatches_check(messages, png, web, ppp):
    """App-page swatch patches: screenshot pixels against their declared CSS colours.

    The hook draws fixed analytic patches (not product colours) and reports each patch's
    declared colour, the browser's computed colour and its rect. The screenshot is the
    composed app output, so this is GPU/pipeline evidence, not a DOM-only claim. Missing
    coverage is BLOCKED; a clipped, oversized, unmeasured or mismatched patch FAILs.
    """
    message = messages.get("swatches")
    if not isinstance(message, dict):
        return ("BLOCKED", "the app page did not report its fixed swatch patches")
    try:
        _, js = probe_source()
        if hook_swatch_colours(js) != EXPECTED_SWATCH_COLOURS:
            return ("BLOCKED", "the real hook's fixed swatch declaration differs from the validator contract")
    except (OSError, AssertionError, ValueError) as error:
        return ("BLOCKED", f"cannot verify the real hook's fixed swatch declaration: {error}")
    patches = message.get("patches")
    inner = message.get("inner")
    if not isinstance(patches, list) or len(patches) != len(EXPECTED_SWATCH_COLOURS):
        return ("FAIL", f"exactly {len(EXPECTED_SWATCH_COLOURS)} fixed swatch patches required: {patches!r}")
    declared_colours = [norm_colour(patch.get("css")) if isinstance(patch, dict) else None for patch in patches]
    if len(set(declared_colours)) != len(EXPECTED_SWATCH_COLOURS) or set(declared_colours) != set(EXPECTED_SWATCH_COLOURS):
        return ("FAIL", f"missing, duplicate or substituted fixed swatch colours: {declared_colours!r}")
    if not (isinstance(inner, list) and len(inner) == 2 and all(isinstance(v, (int, float)) for v in inner)):
        return ("FAIL", f"swatch viewport is not measurable: {inner!r}")
    width, height = float(inner[0]), float(inner[1])
    image_w, image_h = png[0], png[1]
    problems = []
    luminances = []
    seen = set()
    for patch in patches:
        if not isinstance(patch, dict):
            problems.append("a swatch patch is not an object")
            continue
        declared = norm_colour(patch.get("css"))
        computed = norm_colour(patch.get("computed"))
        rgb = css_rgb(declared)
        rect = patch.get("rect")
        name = declared or f"patch {len(problems) + 1}"
        if rgb is None or declared is None or css_alpha(declared) != 1.0:
            problems.append(f"{name} declared colour {patch.get('css')!r} is not an opaque hex colour")
            continue
        if computed is None or css_rgb(computed) != rgb or css_alpha(computed) != 1.0:
            problems.append(f"{name} computed colour {patch.get('computed')!r} != declared {declared} (opaque)")
            continue
        if not (isinstance(rect, dict) and all(isinstance(rect.get(key), (int, float)) for key in ("x", "y", "w", "h"))):
            problems.append(f"{name} patch rect is not measurable: {rect!r}")
            continue
        box_w, box_h = float(rect["w"]), float(rect["h"])
        if not (8.0 <= box_w <= 64.0 and 8.0 <= box_h <= 64.0):
            problems.append(f"{name} patch is clipped or oversized: {box_w}x{box_h} DIP")
            continue
        if not (0.0 <= float(rect["x"]) and 0.0 <= float(rect["y"])
                and float(rect["x"]) + box_w <= width and float(rect["y"]) + box_h <= height):
            problems.append(f"{name} patch is clipped by the viewport: {rect!r} in {inner!r}")
            continue
        sx, sy = page_pixel_point(web, ppp, float(rect["x"]) + box_w / 2.0, float(rect["y"]) + box_h / 2.0)
        if not (0 <= sx < image_w and 0 <= sy < image_h):
            problems.append(f"{name} patch centre ({sx:.0f}, {sy:.0f}) is outside the screenshot")
            continue
        actual = pixel(png, sx, sy)
        delta = max(abs(a - b) for a, b in zip(rgb, actual))
        if delta > 2:
            problems.append(f"{name} painted {hexed(actual)} at ({sx:.0f}, {sy:.0f}) != declared {declared} delta={delta}")
        seen.add(declared)
        luminances.append(relative_luminance(rgb))
    if seen != set(EXPECTED_SWATCH_COLOURS):
        problems.append("not all three fixed swatch colours were measured")
    if luminances and not any(0.15 <= value <= 0.85 for value in luminances):
        problems.append("no nontrivial midtone: a double sRGB decode could not be distinguished")
    return ("FAIL" if problems else "PASS",
            "; ".join(problems) or f"{len(patches)} app swatch pixels match their declared opaque CSS colours")



def theme_report_status(checks):
    rank = {"PASS": 0, "NOT_RUN": 1, "BLOCKED": 2, "FAIL": 3}
    worst = max((rank.get(c["status"], 4) for c in checks), default=3)
    return {0: "PASS", 1: "NOT_RUN", 2: "BLOCKED", 3: "FAIL"}.get(worst, "FAIL")


def probe_source():
    """The fixed hook's JS and the IPC log cap, read from the real app source."""
    text = (ROOT / "apps/solidcraft/src/assistant_cef.rs").read_text(encoding="utf-8")
    match = re.search(r'const SCOPE_PROBE_JS: &str = r#"(.*?)"#;', text, re.S)
    if not match:
        raise AssertionError("SCOPE_PROBE_JS not found in assistant_cef.rs")
    return text, match.group(1)


def _js_value_end(text, start):
    """End index of one JS value starting at `start`, at the next top-level `,` or closer."""
    i = start
    while i < len(text) and text[i] in " \t\r\n":
        i += 1
    begin = i
    depth = 0
    while i < len(text):
        char = text[i]
        if char in "'\"":
            quote = char
            i += 1
            while i < len(text) and text[i] != quote:
                i += 2 if text[i] == "\\" else 1
            i += 1
            continue
        if char in "([{":
            depth += 1
        elif char in ")]}":
            if depth == 0:
                return text[begin:i].strip(), i
            depth -= 1
        elif char == "," and depth == 0:
            return text[begin:i].strip(), i
        i += 1
    return text[begin:i].strip(), i


def _js_object_fields(text, start):
    """Top-level (key, value-expression) pairs of a JS object literal at `text[start] == '{'`."""
    if start >= len(text) or text[start] != "{":
        raise AssertionError("not an object literal")
    fields, i = [], start + 1
    while True:
        while i < len(text) and text[i] in " \t\r\n,":
            i += 1
        if i >= len(text):
            raise AssertionError("unterminated object literal")
        if text[i] == "}":
            return fields
        if text[i] in "'\"":
            quote = text[i]
            end = text.index(quote, i + 1)
            key, i = text[i + 1:end], end + 1
        else:
            match = re.match(r"[A-Za-z_$][\w$]*", text[i:])
            if not match:
                raise AssertionError(f"unexpected JS object key near {text[i:i + 20]!r}")
            key = match.group(0)
            i += len(key)
        j = i
        while j < len(text) and text[j] in " \t\r\n":
            j += 1
        if j < len(text) and text[j] == ":":
            value, i = _js_value_end(text, j + 1)
            fields.append((key, value))
        else:
            fields.append((key, key))
            i = j


def _js_items(text):
    """Top-level values of a JS array literal (brackets included)."""
    if len(text) < 2 or text[0] != "[" or text[-1] != "]":
        raise AssertionError("not an array literal")
    items, i = [], 1
    while i < len(text) - 1:
        while i < len(text) and text[i] in " \t\r\n,":
            i += 1
        if i >= len(text) - 1:
            break
        value, i = _js_value_end(text, i)
        if value:
            items.append(value)
    return items


# Conservative widths for values the hook formats at runtime, chosen so the bound can
# only over-estimate: a rect's JSON object, a resolved CSS colour, a theme id.
RECT_BOUND, COLOUR_BOUND, ID_BOUND = 40, 32, 40


def _swatch_patch_count(js):
    match = re.search(r"const SWATCHES = \[(.*?)\];", js, re.S)
    if not match:
        raise AssertionError("the hook declares no SWATCHES fixtures")
    return len(_js_items("[" + match.group(1) + "]"))


def hook_swatch_colours(js):
    """Read every declared fixture, not just colour-looking fragments of the hook."""
    match = re.search(r"const SWATCHES = (\[.*?\]);", js, re.S)
    if not match:
        raise AssertionError("the hook declares no SWATCHES fixtures")
    colours, ids = [], []
    for item in _js_items(match.group(1)):
        fields = _js_object_fields(item, 0)
        if len(fields) != 2 or {key for key, _ in fields} != {"id", "css"}:
            raise AssertionError(f"unrecognised fixed swatch declaration: {item}")
        values = dict(fields)
        if not re.fullmatch(r"'[a-z]+'", values["id"]) or not re.fullmatch(r"'#[0-9a-f]{6}'", values["css"]):
            raise AssertionError(f"nonliteral fixed swatch declaration: {item}")
        ids.append(values["id"][1:-1])
        colours.append(values["css"][1:-1])
    if not colours or len(set(ids)) != len(ids) or len(set(colours)) != len(colours):
        raise AssertionError("empty or duplicate fixed swatch declaration")
    return tuple(colours)


def _patch_object_bound(js):
    marker = "patches: SWATCHES.map("
    at = js.find(marker)
    if at < 0:
        at = js.find("SWATCHES.map(")
    if at < 0:
        raise AssertionError("the hook does not build its swatch message from SWATCHES")
    brace = js.find("{", at)
    return _object_bound(js, brace)


def _value_bound(expr, js):
    expr = expr.strip()
    if not expr:
        return 0
    if expr[0] in "'\"" and expr[-1] == expr[0]:
        return len(expr)
    if expr[0] == "[":
        items = _js_items(expr)
        return 2 + sum(_value_bound(item, js) for item in items) + max(0, len(items) - 1)
    if expr[0] == "{":
        return _object_bound(js, js.find(expr)) if js.find(expr) >= 0 else _object_bound(expr, 0)
    if expr.startswith("SWATCHES.map("):
        count = _swatch_patch_count(js)
        return 2 + count * _patch_object_bound(js) + max(0, count - 1)
    if "parseFloat(" in expr:
        return 8
    if "getPropertyValue" in expr:
        return 12  # the bridge writes at most #rrggbbaa into those six variables
    if ".backgroundColor" in expr or ".color" in expr or "colour(" in expr:
        return COLOUR_BOUND
    if expr.startswith("rect("):
        return RECT_BOUND
    if expr == "theme":
        return ID_BOUND
    if expr.endswith(".css"):
        return 11
    if expr in ("devicePixelRatio", "innerWidth", "innerHeight"):
        return 8
    if re.fullmatch(r"-?\d+(?:\.\d+)?", expr):
        return len(expr)
    return 40


def _object_bound(js, start):
    fields = _js_object_fields(js, start)
    total = 2  # braces
    for index, (key, value) in enumerate(fields):
        total += (1 if index else 0) + len(key) + 3 + _value_bound(value, js)
    return total


def scope_message_bounds(js):
    """Worst-case `SCOPE {json}` length per action, computed from the real hook source."""
    bounds = {}
    for match in re.finditer(r"send\(\{", js):
        start = match.start() + len("send(")
        fields = dict(_js_object_fields(js, start))
        action = fields.get("action", "").strip()
        if len(action) < 2 or action[0] not in "'\"":
            continue
        bounds[action[1:-1]] = len("SCOPE ") + _object_bound(js, start)
    return bounds


def _self_test():
    failures = 0

    def check(name, test):
        nonlocal failures
        try:
            test()
        except Exception as error:  # self-test harness: report every failure, never abort early
            failures += 1
            print(f"FAIL {name}: {type(error).__name__}: {error}")
        else:
            print(f"PASS {name}")

    def alpha_reads_hex_and_rgba():
        if abs(css_alpha("#11223380") - 128 / 255) > 0.001:
            raise AssertionError(css_alpha("#11223380"))
        if css_alpha("#112233") != 1.0 or css_alpha("rgba(1, 2, 3, 0)") != 0.0 or css_alpha("transparent") != 0.0:
            raise AssertionError("hex/rgba/transparent alpha")

    def transparent_role_fails_even_with_matching_rgb():
        palette = {"text": "#ffffff"}
        roles = {"action": "roles", "theme": "t", "prose": "rgba(255, 255, 255, 0)",
                 "label": "rgb(255, 255, 255)", "textarea": "rgb(255, 255, 255)", "live": "rgb(255, 255, 255)"}
        status, detail = page_roles_check({"roles": roles}, palette)
        if status != "FAIL" or "prose" not in detail:
            raise AssertionError(f"transparent role must fail: {status} {detail}")

    def wrong_role_colour_fails():
        palette = {"text": "#ffffff"}
        roles = {"action": "roles", "theme": "t", "prose": "rgb(255, 255, 255)", "label": "rgb(255, 255, 255)",
                 "textarea": "rgb(255, 255, 255)", "live": "rgb(1, 2, 3)"}
        status, _ = page_roles_check({"roles": roles}, palette)
        if status != "FAIL":
            raise AssertionError(f"off-palette role passed: {status}")

    def missing_role_is_blocked_not_failed():
        palette = {"text": "#ffffff"}
        roles = {"action": "roles", "theme": "t", "prose": "rgb(255, 255, 255)", "label": "rgb(255, 255, 255)",
                 "textarea": "rgb(255, 255, 255)"}
        status, detail = page_roles_check({"roles": roles}, palette)
        if status != "BLOCKED" or "coverage" not in detail:
            raise AssertionError(f"a missing role is unobserved coverage, not a styling failure: {status} {detail}")
        if page_roles_check({}, palette)[0] != "BLOCKED":
            raise AssertionError("an unreported roles message is BLOCKED, not PASS")

    def missing_roles_do_not_hide_present_wrong_styles():
        palette = {"text": "#ffffff"}
        for role, value in (
            ("textarea", "#010203"),
            ("label", "rgba(255, 255, 255, 0)"),
            ("live", ""),
            ("swatch", "#010203"),
        ):
            status, detail = page_roles_check({"roles": {role: value}}, palette)
            if status != "FAIL" or role not in detail:
                raise AssertionError(f"missing prose must not hide wrong {role}: {status} {detail}")
        for roles in ({}, {"prose": None}, {"textarea": "#ffffff"}):
            if page_roles_check({"roles": roles}, palette)[0] != "BLOCKED":
                raise AssertionError(f"genuinely incomplete, correct role coverage must be BLOCKED: {roles}")

    def missing_fonts_do_not_hide_present_wrong_sizes():
        for fonts in (
            {"root": 12.0, "textarea": 14.0},
            {"root": 16.0, "label": 12.5},
            {"root": 12.0, "label": "12.5"},
            {"root": 12.0, "textarea": float("nan")},
            {"root": 12.0, "label": True},
        ):
            status, detail = page_fonts_check({"fonts": fonts})
            if status != "FAIL":
                raise AssertionError(f"missing prose must not hide wrong measured fonts {fonts}: {status} {detail}")
        for fonts in ({}, {"root": None, "prose": None}, {"root": 12.0, "textarea": 12.5}):
            if page_fonts_check({"fonts": fonts})[0] != "BLOCKED":
                raise AssertionError(f"genuinely incomplete, correct font coverage must be BLOCKED: {fonts}")
        fonts = {"root": 12.0, "prose": 12.5, "textarea": 12.5, "label": 12.5}
        if page_fonts_check({"fonts": fonts})[0] != "PASS":
            raise AssertionError("all real roles at their reviewed sizes must PASS")

    def optional_role_swatch_is_fixture_instrumentation():
        palette = {"text": "#ffffff"}
        roles = {"action": "roles", "theme": "t", "prose": "#ffffff", "label": "#ffffff",
                 "textarea": "#ffffff", "live": "#ffffff"}
        status, _ = page_roles_check({"roles": roles}, palette)
        if status != "PASS":
            raise AssertionError("no page swatch must not block the shipped page")
        roles["swatch"] = "rgba(255, 255, 255, 0)"
        if page_roles_check({"roles": roles}, palette)[0] != "FAIL":
            raise AssertionError("a transparent fixture swatch must fail")
        roles["swatch"] = "#ffffff"
        status, detail = page_roles_check({"roles": roles}, palette)
        if status != "PASS" or "fixture" not in detail:
            raise AssertionError(f"fixture swatch reason not recorded: {status} {detail}")

    def rail_uses_the_control_side_not_the_page_left_field():
        web = {"x": 100, "y": 0, "width": 50, "height": 100}
        x_left, _ = rail_sample_point({"side": "left"}, web, 1.0, 8.0)
        x_right, _ = rail_sample_point({"side": "right"}, web, 1.0, 8.0)
        if not x_left > web["x"] + web["width"]:
            raise AssertionError(f"left dock must sample right of the page: {x_left}")
        if not x_right < web["x"]:
            raise AssertionError(f"right dock must sample left of the page: {x_right}")
        if rail_sample_point({}, web, 1.0, 8.0)[0] != x_right:
            raise AssertionError("a state without `side` must not guess the left dock")
        # The page-facing payload's `left` boolean must not steer this: `side` is authoritative.
        poisoned = {"side": "right", "left": True}
        if rail_sample_point(poisoned, web, 1.0, 8.0)[0] != x_right:
            raise AssertionError("the page state's `left` field must not be used for the rail")

    def rail_pixel_matches_panel_and_rejects_the_page():
        png = (160, 160, 3, bytes([0x11, 0x22, 0x33]) * (160 * 160))
        web = {"x": 10, "y": 0, "width": 100, "height": 100}
        status, detail = rail_pixel_check(png, {"side": "right"}, web, 1.0, 8.0, "#112233")
        if status != "PASS":
            raise AssertionError(detail)
        if rail_pixel_check(png, {"side": "right"}, web, 1.0, 8.0, "#ffffff")[0] != "FAIL":
            raise AssertionError("a page-coloured rail pixel must fail")
        # Left-docked: the strip is on the page's right.
        if rail_pixel_check(png, {"side": "left"}, web, 1.0, 8.0, "#112233")[0] != "PASS":
            raise AssertionError("left-side rail sampling")

    def window_pixel_fixture(measured_ppp=1.25):
        # Window PNG for an OS window at (137,83), NOT a screen capture. Only
        # the hand-calculated window-relative sample pixels have the right colours.
        # ui.rects supplies points; CEF rounds its origin/edges to physical pixels.
        message, _, _ = swatch_test_image()
        px = bytearray(400 * 300 * 3)
        samples = (
            (52, 63, (128, 128, 128)),
            (82, 63, (51, 102, 153)),
            (112, 63, (224, 128, 64)),
            (22, 78, (17, 34, 51)),    # right dock: rail at the page's left
            (152, 78, (17, 34, 51)),   # left dock: rail at the page's right
            (87, 88, (17, 34, 51)),    # interior surface, within aligned bounds
        ) if measured_ppp == 1.25 else (
            # 1.26 is within the matrix's 0.02 measured-ppp tolerance. The
            # same actual controls now distinguish rounding the CEF origin
            # BEFORE a CSS offset from rounding their unaligned sum.
            (52, 63, (128, 128, 128)),
            (82, 63, (51, 102, 153)),
            (113, 63, (224, 128, 64)),
            (22, 78, (17, 34, 51)),
            (153, 78, (17, 34, 51)),
            (88, 88, (17, 34, 51)),
        )
        for x, y, rgb in samples:
            offset = (y * 400 + x) * 3
            px[offset:offset + 3] = bytes(rgb)
        return {
            "png": (400, 300, 3, bytes(px)),
            "web": {"name": "agent_chat:web", "x": 21.2, "y": 30.0, "width": 96, "height": 64},
            "ppp": measured_ppp, "origin": {"X": 137, "Y": 83},
            "state": {"side": "right", "left": True},
            "palette": {"metrics": {"space8": 8.0}},
            "expected": "#112233", "theme_scope": message,
        }

    def actual_main_pixel_expression(name, values):
        # Execute the real production caller expressions, not copies that could
        # quietly drop the offset while main still adds it. No app/IPC is run.
        import ast
        tree = ast.parse(Path(__file__).read_text(encoding="utf-8"))
        main_node = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == "main")
        if name == "surface":
            nodes = [node.value for node in ast.walk(main_node) if isinstance(node, ast.Assign)
                     and any(isinstance(target, ast.Name) and target.id == "bg" for target in node.targets)]
        else:
            nodes = [node for node in ast.walk(main_node) if isinstance(node, ast.Call)
                     and isinstance(node.func, ast.Name) and node.func.id == name]
        if len(nodes) != 1:
            raise AssertionError(f"expected one real main {name} caller, found {len(nodes)}")
        return eval(compile(ast.Expression(nodes[0]), __file__, "eval"), globals(), values)

    def rail_caller_samples_window_pixels_at_nonzero_os_origin():
        for ppp in (1.25, 1.26):
            values = window_pixel_fixture(ppp)
            for side in ("right", "left"):
                values["state"]["side"] = side
                status, detail = actual_main_pixel_expression("rail_pixel_check", values)
                if status != "PASS":
                    raise AssertionError(f"{side} dock, OS origin (137,83), ppp={ppp}: {status} {detail}")
        values["png"] = (400, 300, 3, bytes(400 * 300 * 3))
        if actual_main_pixel_expression("rail_pixel_check", values)[0] != "FAIL":
            raise AssertionError("the actual rail caller must reject wrong window pixels")

    def swatch_caller_samples_window_pixels_at_nonzero_os_origin():
        for ppp in (1.25, 1.26):
            values = window_pixel_fixture(ppp)
            status, detail = actual_main_pixel_expression("page_swatches_check", values)
            if status != "PASS":
                raise AssertionError(f"OS origin (137,83), ppp={ppp}: {status} {detail}")
        values["png"] = (400, 300, 3, bytes(400 * 300 * 3))
        if actual_main_pixel_expression("page_swatches_check", values)[0] != "FAIL":
            raise AssertionError("the actual swatch caller must reject wrong window pixels")

    def surface_caller_samples_window_pixels_at_nonzero_os_origin():
        for ppp in (1.25, 1.26):
            actual = actual_main_pixel_expression("surface", window_pixel_fixture(ppp))
            if actual != (17, 34, 51):
                raise AssertionError(f"OS origin (137,83), ppp={ppp}: surface sampled {actual}, expected (17,34,51)")

    def swatch_test_image():
        # Independently transcribed from the real hook's three 20px controls at
        # left:8px + padding:2px, gap:4px; never built from the validator's expected set.
        row = bytes((0x80, 0x80, 0x80)) * 32 + bytes((0x33, 0x66, 0x99)) * 24 + bytes((0xe0, 0x80, 0x40)) * 40
        png = (96, 64, 3, row * 64)
        web = {"x": 0, "y": 0, "width": 96, "height": 64}
        message = {
            "swatches": {
                "action": "swatches", "theme": "t", "inner": [96, 64],
                "patches": [
                    {"css": "#808080", "computed": "rgb(128, 128, 128)", "rect": {"x": 10, "y": 10, "w": 20, "h": 20}},
                    {"css": "#336699", "computed": "rgb(51, 102, 153)", "rect": {"x": 34, "y": 10, "w": 20, "h": 20}},
                    {"css": "#e08040", "computed": "rgb(224, 128, 64)", "rect": {"x": 58, "y": 10, "w": 20, "h": 20}},
                ],
            }
        }
        return message, png, web

    def swatch_pixels_validate_synthetic_composited_buffers():
        # Pure offline PNG-buffer logic, NOT evidence of actual app/GPU rendering.
        message, png, web = swatch_test_image()
        status, detail = page_swatches_check(message, png, web, 1.0)
        if status != "PASS":
            raise AssertionError(detail)
        if page_swatches_check({}, png, web, 1.0)[0] != "BLOCKED":
            raise AssertionError("missing app swatch coverage must be BLOCKED, never fixture-only PASS")
        black = (96, 64, 3, bytes(96 * 64 * 3))
        if page_swatches_check(message, black, web, 1.0)[0] != "FAIL":
            raise AssertionError("a wrong composited pixel must fail")
        mismatch = json.loads(json.dumps(message))
        mismatch["swatches"]["patches"][1]["computed"] = "rgb(128, 128, 128)"
        if page_swatches_check(mismatch, png, web, 1.0)[0] != "FAIL":
            raise AssertionError("computed CSS that disagrees with the declared colour must fail")
        oversized = json.loads(json.dumps(message))
        oversized["swatches"]["patches"][0]["rect"] = {"x": 10, "y": 10, "w": 4, "h": 20}
        if page_swatches_check(oversized, png, web, 1.0)[0] != "FAIL":
            raise AssertionError("a clipped/oversized patch must fail")
        outside = json.loads(json.dumps(message))
        outside["swatches"]["patches"][0]["rect"] = {"x": 90, "y": 10, "w": 20, "h": 20}
        if page_swatches_check(outside, png, web, 1.0)[0] != "FAIL":
            raise AssertionError("a patch clipped by the viewport must fail")
        partial = json.loads(json.dumps(message))
        partial["swatches"]["patches"][2]["computed"] = None
        if page_swatches_check(partial, png, web, 1.0)[0] != "FAIL":
            raise AssertionError("all three patches must be measured, not only two")

    def incomplete_or_substituted_swatches_never_pass():
        message, png, web = swatch_test_image()
        patches = message["swatches"]["patches"]
        variants = {
            "missing warm": patches[:2],
            "missing mid": patches[1:],
            "empty": [],
            "duplicate replacing warm": patches[:2] + [patches[0]],
            "extra duplicate": patches + [patches[0]],
            "substituted midtone": patches[:2] + [
                {"css": "#808081", "computed": "rgb(128, 128, 129)", "rect": patches[0]["rect"]}],
        }
        for name, bad_patches in variants.items():
            bad = {"swatches": dict(message["swatches"], patches=bad_patches)}
            status, detail = page_swatches_check(bad, png, web, 1.0)
            if status not in ("FAIL", "BLOCKED"):
                raise AssertionError(f"{name} must never PASS: {status} {detail}")

    def swatch_validator_requires_real_hook_parity():
        message, png, web = swatch_test_image()
        original = globals()["probe_source"]
        text, js = original()
        try:
            # Only the read-only source boundary is replaced, never the validator.
            # A full otherwise-PASS image must stop passing if its declaration
            # cannot be verified against the real hook.
            globals()["probe_source"] = lambda: (text, js.replace("'#e08040'", "'#e08041'"))
            if page_swatches_check(message, png, web, 1.0)[0] != "BLOCKED":
                raise AssertionError("source/validator fixture drift must block the production validator")

            def unavailable():
                raise OSError("source unavailable (self-test)")

            globals()["probe_source"] = unavailable
            if page_swatches_check(message, png, web, 1.0)[0] != "BLOCKED":
                raise AssertionError("unavailable hook declaration must block the production validator")
        finally:
            globals()["probe_source"] = original

    def offscreen_pixel_checks_do_not_clamp_to_an_edge():
        png = (64, 64, 3, bytes((17, 34, 51)) * (64 * 64))
        web = {"x": 1, "y": 0, "width": 50, "height": 60}
        if rail_pixel_check(png, {"side": "right"}, web, 1.0, 8.0, "#112233")[0] != "FAIL":
            raise AssertionError("offscreen native rail must not PASS on a clamped edge pixel")
        message, png, web = swatch_test_image()
        # The warm centre 68.6 rounds to 69, just OUTSIDE this 69px PNG. If the
        # bounds test uses the unrounded centre, pixel() clamps to a matching warm edge.
        cropped = b"".join(png[3][y * 96 * 3:(y * 96 + 69) * 3] for y in range(64))
        message["swatches"]["patches"][2]["rect"]["x"] = 58.6
        if page_swatches_check(message, (69, 64, 3, cropped), web, 1.0)[0] != "FAIL":
            raise AssertionError("a rounded-offscreen swatch centre must FAIL, never clamp to a matching pixel")

    def split_scope_messages_parse_per_action_and_theme():
        lines = [
            "noise\n",
            'SCOPE {"action":"rects","theme":"a","select":{"x":1}}\n',
            'SCOPE {"action":"fonts","theme":"a","prose":12.5}\n',
            'SCOPE {"action":"rects","theme":"b","select":{"x":2}}\n',
            'SCOPE {"action":"fonts","theme":"a","pro"  (truncated)\n',
            'SCOPE {"action":"scope_oversize","of":"tokens","len":900}\n',
        ]
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "app.log"
            path.write_text("".join(lines), encoding="utf-8")
            latest = parse_scope_lines(path)
            if latest["rects"]["theme"] != "b" or latest["fonts"]["theme"] != "a":
                raise AssertionError(latest)
            if "pro" in latest["fonts"]:
                raise AssertionError("a truncated line must not parse")
            matched = scope_for_theme(path, "a", timeout=0.0)
            if sorted(matched) != ["fonts"]:
                raise AssertionError(f"theme filter: {sorted(matched)}")

    def budgets_stay_under_the_ipc_cap():
        text, js = probe_source()
        cap = re.search(r"const IPC_LOG_TEXT_CAP: usize = (\d+);", text)
        if not cap or int(cap.group(1)) != 600:
            raise AssertionError("the /ipc log cap is not the documented 600 characters")
        ceiling = re.search(r"const SCOPE_MAX_TEXT = (\d+);", js)
        if not ceiling:
            raise AssertionError("the probe has no SCOPE_MAX_TEXT ceiling")
        ceiling = int(ceiling.group(1))
        budget = re.search(r"SCOPE_BUDGET = \{([^}]*)\}", js)
        if not budget:
            raise AssertionError("the probe has no per-action SCOPE_BUDGET")
        budgets = {name: int(value) for name, value in re.findall(r"(\w+): (\d+)", budget.group(1))}
        if sorted(budgets) != sorted(SCOPE_ACTIONS):
            raise AssertionError(f"budgets must cover every split action: {budgets}")
        if not 0 < ceiling < int(cap.group(1)):
            raise AssertionError(f"SCOPE_MAX_TEXT {ceiling} must stay under the cap {cap.group(1)}")
        # Worst-case lengths come from parsing the real hook source, not a hand-copied body.
        bounds = scope_message_bounds(js)
        for action in SCOPE_ACTIONS:
            if action not in bounds:
                raise AssertionError(f"the hook does not send a bounded {action} message: {sorted(bounds)}")
            if budgets[action] > ceiling:
                raise AssertionError(f"{action} budget {budgets[action]} exceeds the probe ceiling {ceiling}")
            if bounds[action] > budgets[action]:
                raise AssertionError(f"{action} worst case {bounds[action]} > budget {budgets[action]}")
        for action, bound in bounds.items():
            if action not in budgets and bound > ceiling:
                raise AssertionError(f"event message {action} worst case {bound} > ceiling {ceiling}")

    def probe_fixture_matches_the_app_budgets():
        _, js = probe_source()
        app = re.search(r"SCOPE_BUDGET = \{([^}]*)\}", js)
        fixture = (ROOT / "experiments/cef-osr/poc/fixtures/probe.html").read_text(encoding="utf-8")
        fixture_budget = re.search(r"SCOPE_BUDGET = \{([^}]*)\}", fixture)
        if not app or not fixture_budget:
            raise AssertionError("the app hook or the probe fixture has no SCOPE_BUDGET")
        rollup = lambda text: {name: int(value) for name, value in re.findall(r"(\w+): (\d+)", text)}
        if rollup(app.group(1)) != rollup(fixture_budget.group(1)):
            raise AssertionError("the standalone fixture's budgets drifted from the app hook's")
        if hook_swatch_colours(js) != EXPECTED_SWATCH_COLOURS or hook_swatch_colours(fixture) != EXPECTED_SWATCH_COLOURS:
            raise AssertionError("app/standalone swatch declarations must match the production validator's exact fixture contract")
        for action in SCOPE_ACTIONS:
            if f"action: '{action}'" not in fixture:
                raise AssertionError(f"the fixture does not send {action}")

    def probe_is_split_guarded_and_rail_free():
        _, js = probe_source()
        for action in SCOPE_ACTIONS:
            if f"send({{ action: '{action}'" not in js:
                raise AssertionError(f"the probe does not send {action} as its own message")
        if "text.length <= cap && text.length <= SCOPE_MAX_TEXT" not in js:
            raise AssertionError("the probe has no emit-time size guard")
        if "scope_oversize" not in js:
            raise AssertionError("an over-budget message must become a bounded failure")
        if 'class*="rail"' in js or '[class*="swatch"]' in js:
            raise AssertionError("the page must not claim a rail/swatch element it does not have")

    tests = [
        ("css_alpha_reads_hex_and_rgba", alpha_reads_hex_and_rgba),
        ("transparent_role_fails_even_with_matching_rgb", transparent_role_fails_even_with_matching_rgb),
        ("wrong_role_colour_fails", wrong_role_colour_fails),
        ("missing_role_is_blocked_not_failed", missing_role_is_blocked_not_failed),
        ("missing_roles_do_not_hide_present_wrong_styles", missing_roles_do_not_hide_present_wrong_styles),
        ("missing_fonts_do_not_hide_present_wrong_sizes", missing_fonts_do_not_hide_present_wrong_sizes),
        ("optional_role_swatch_is_fixture_instrumentation", optional_role_swatch_is_fixture_instrumentation),
        ("rail_uses_the_control_side_not_the_page_left_field", rail_uses_the_control_side_not_the_page_left_field),
        ("rail_pixel_matches_panel_and_rejects_the_page", rail_pixel_matches_panel_and_rejects_the_page),
        ("rail_caller_samples_window_pixels_at_nonzero_os_origin", rail_caller_samples_window_pixels_at_nonzero_os_origin),
        ("swatch_caller_samples_window_pixels_at_nonzero_os_origin", swatch_caller_samples_window_pixels_at_nonzero_os_origin),
        ("surface_caller_samples_window_pixels_at_nonzero_os_origin", surface_caller_samples_window_pixels_at_nonzero_os_origin),
        ("swatch_pixels_validate_synthetic_composited_buffers", swatch_pixels_validate_synthetic_composited_buffers),
        ("incomplete_or_substituted_swatches_never_pass", incomplete_or_substituted_swatches_never_pass),
        ("swatch_validator_requires_real_hook_parity", swatch_validator_requires_real_hook_parity),
        ("offscreen_pixel_checks_do_not_clamp_to_an_edge", offscreen_pixel_checks_do_not_clamp_to_an_edge),
        ("split_scope_messages_parse_per_action_and_theme", split_scope_messages_parse_per_action_and_theme),
        ("budgets_stay_under_the_ipc_cap", budgets_stay_under_the_ipc_cap),
        ("probe_fixture_matches_the_app_budgets", probe_fixture_matches_the_app_budgets),
        ("probe_is_split_guarded_and_rail_free", probe_is_split_guarded_and_rail_free),
    ]
    for name, test in tests:
        check(name, test)
    print(f"self-test: {len(tests) - failures}/{len(tests)} passed, {failures} failed")
    return 0 if failures == 0 else 1


def main():
    exe = sys.argv[1]
    out = Path(sys.argv[sys.argv.index("--out") + 1]) if "--out" in sys.argv else Path(tempfile.mkdtemp(prefix="theme-matrix-"))
    scales = sys.argv[sys.argv.index("--ui-scales") + 1].split(",") if "--ui-scales" in sys.argv else ["1", "1.25", "1.5", "2"]
    wanted = sys.argv[sys.argv.index("--themes") + 1].split(",") if "--themes" in sys.argv else THEMES
    out.mkdir(parents=True, exist_ok=True)
    checks, artifacts, limitations = [], [], [
        "UI-scale preference coverage is not actual compositor or monitor movement.",
        "requested_scale is the app's placement scale; capture/source/import density comes from frame receipts.",
        "The page reports its computed styles as small SCOPE messages (rects, fonts, tokens, roles, swatches); the app's `/ipc` log keeps 600 characters, so an over-budget message is a bounded scope_oversize failure instead of truncated JSON.",
        "The rail is native app chrome, not a page element: it is checked from the app screenshot pixel beside the web rect, using the control state's `side`.",
        "App swatch pixels are fixed analytic test patches drawn only under SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE=1 (opaque midtones, never product colours); the app screenshot pixel is compared with the patch's declared and computed CSS colour.",
        "Missing prose/label/textarea/aria-live at probe time is unobserved coverage (BLOCKED) only if every present measured style is correct; a present wrong style always FAILs.",
        "ui.screenshot is a window image. Samples use current ui.rects window points/pixelsPerPoint and pixel-aligned CEF placement edges, never an OS screen origin.",
    ]
    if len(THEMES) != 25 or len(set(THEMES)) != 25:
        checks.append({"name": "theme_source", "status": "FAIL", "detail": f"expected 25 unique source themes, found {len(THEMES)}"})
    else:
        checks.append({"name": "theme_source", "status": "PASS", "detail": "25 built-in themes enumerated from source"})

    work = Path(tempfile.mkdtemp(prefix="intcad-theme-"))
    env = dict(os.environ)
    for name in FOLDERS:
        env[f"SOLIDCRAFT_{name}_DIR"] = str(work / name.lower())
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    env["NO_AT_BRIDGE"] = "1"
    # Opt-in, fixed in-source scope probe: bounded DOM rects/computed values only.
    env["SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE"] = "1"
    log_path = out / "app.log"
    log = open(log_path, "wb")
    app = subprocess.Popen([exe, "--control", "0"], env=env, stdout=log, stderr=subprocess.STDOUT)
    # The app picks a free port and writes it, with the token, into its control folder.
    control_dir = Path(env["SOLIDCRAFT_CONTROL_DIR"])
    deadline = time.monotonic() + 120
    port, token = 0, ""
    while time.monotonic() < deadline:
        ports = [p for p in control_dir.glob("*.port")]
        if ports:
            port = int(ports[0].read_text().strip())
            token = (control_dir / f"{port}.token").read_text().strip()
            break
        time.sleep(0.3)
    control = None
    while time.monotonic() < deadline and control is None:
        try:
            control = Control(port, token)
        except OSError:
            time.sleep(0.5)
    while time.monotonic() < deadline:
        try:
            if control.call("ui.agent_chat", {"open": True}).get("ok"):
                break
        except OSError:
            time.sleep(0.5)
    while time.monotonic() < deadline:
        state = control.call("ui.agent_chat").get("result") or {}
        if (state.get("webView") or {}).get("page"):
            break
        time.sleep(0.5)
    time.sleep(2)

    rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
    ppp = rects.get("pixelsPerPoint") or 1.0
    baseline = control.call("ui.prefs", {}).get("result")
    baseline = baseline if isinstance(baseline, dict) else {}
    base_scale = baseline.get("ui_scale")
    if not isinstance(base_scale, (int, float)) or isinstance(base_scale, bool) or base_scale <= 0:
        base_scale = 1.0
    native_base = float(ppp) / float(base_scale)
    checks.append({"name": "native_base", "status": "PASS" if native_base > 0 else "BLOCKED", "detail": f"ppp {ppp} / ui_scale {base_scale} = {native_base:.4f}"})

    results = []
    all_checks = checks
    for target_text in scales:
        try:
            target = float(target_text)
        except ValueError:
            all_checks.append({"name": f"ui_scale_{target_text}", "status": "FAIL", "detail": "invalid scale"})
            continue
        request = target / native_base if native_base > 0 else 0.0
        if not (0.5 <= request <= 2.5):
            all_checks.append({"name": f"ui_scale_{target:g}", "status": "FAIL", "detail": f"derived ui_scale {request:.4f} is outside 0.5..2.5"})
            continue
        control.call("ui.prefs", {"ui_scale": request})
        seen, scale_deadline = None, time.monotonic() + 10
        while time.monotonic() < scale_deadline:
            measured = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
            seen = measured.get("pixelsPerPoint")
            if isinstance(seen, (int, float)) and abs(float(seen) - target) <= 0.02:
                break
            time.sleep(0.25)
        match = isinstance(seen, (int, float)) and not isinstance(seen, bool) and abs(float(seen) - target) <= 0.02
        if match:
            ppp = float(seen)
        all_checks.append({"name": f"ui_scale_{target:g}", "status": "PASS" if match else "FAIL", "detail": f"requested ui_scale {request:.4f}, measured pixelsPerPoint {seen}"})

        for theme in wanted:
            answer = control.call("ui.prefs", {"theme": theme})
            ok = bool(answer.get("ok"))
            time.sleep(1.4)
            state = control.call("ui.agent_chat").get("result") or {}
            palette = state.get("theme") or {}
            theme_scope = scope_for_theme(log_path, theme)
            # A scale/theme/layout change can move every point. Use current window
            # geometry, with the last published rect authoritative (ui.rects contract).
            rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
            ppp = rects["pixelsPerPoint"]
            web = next(r for r in reversed(rects.get("rects", [])) if r["name"] == "agent_chat:web")
            shot = out / f"{theme}-{target_text.replace('.', '_')}.png"
            control.call("ui.screenshot", {"path": str(shot)})
            time.sleep(0.15)
            png = read_png(shot)
            bg = surface_pixel(png, web, ppp)
            expected = (palette.get("colors") or {}).get("surface")
            want = expected.lstrip("#") if expected else ""
            delta = max(
                (abs(int(want[i:i + 2], 16) - bg[i // 2]) for i in range(0, 6, 2)),
                default=255,
            ) if len(want) == 6 else 255
            tokens = palette.get("colors") or {}
            def color_ok(value):
                return isinstance(value, str) and re.fullmatch(r"#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?", value) is not None
            token_ok = all(color_ok(tokens.get(name)) for name in ("window", "surface", "text", "panel", "border", "accent", "borderStrong", "hover", "selected"))
            all_checks.append({
                "name": f"palette_{target:g}_{theme}",
                "status": "PASS" if ok and palette.get("id") == theme and token_ok and delta <= 2 else "FAIL",
                "detail": f"applied={ok} reported={palette.get('id')} tokens={token_ok} surface delta={delta}",
            })
            # Page-side computed evidence, not just the app's own palette report:
            # the tokens the page actually resolved, its real transparency, and the
            # real prose/label/textarea/aria-live role colours. The rail is native, so
            # it is measured from the app screenshot instead of the page.
            if not theme_scope:
                all_checks.append({
                    "name": f"page_roles_{target:g}_{theme}",
                    "status": "BLOCKED",
                    "detail": "the page did not report computed styles for this theme",
                })
            else:
                for label, outcome in (
                    ("page_tokens", page_token_check(theme_scope, tokens)),
                    ("page_transparency", page_transparency_check(theme_scope)),
                    ("page_roles", page_roles_check(theme_scope, tokens)),
                    ("role_fonts", page_fonts_check(theme_scope)),
                    ("native_rail", rail_pixel_check(
                        png,
                        state,
                        web,
                        ppp,
                        (palette.get("metrics") or {}).get("space8"),
                        expected,
                    )),
                    ("page_swatches", page_swatches_check(
                        theme_scope,
                        png,
                        web,
                        ppp,
                    )),
                ):
                    all_checks.append({
                        "name": f"{label}_{target:g}_{theme}",
                        "status": outcome[0],
                        "detail": outcome[1],
                    })
            results.append({
                "theme": theme,
                "ui_scale": target,
                "requested_ui_scale": request,
                "applied": ok,
                "reported_id": palette.get("id"),
                "dark": palette.get("dark"),
                "expected_surface": expected,
                "painted_surface": hexed(bg),
                "delta": delta,
                "match": delta <= 2,
                "tokens": {name: tokens.get(name) for name in ("window", "surface", "text", "panel", "border", "accent")},
                "screenshot": str(shot),
            })
            print(json.dumps(results[-1]))

    # The probe refuses to post a message that would be cut by the app's 600-character
    # `/ipc` log cap; such a refusal is a failure, not a silent omission.
    oversize = parse_scope_lines(log_path).get("scope_oversize")
    if oversize:
        all_checks.append({
            "name": "page_scope_size",
            "status": "FAIL",
            "detail": "the page probe replaced an over-budget message: " + json.dumps(oversize),
        })

    control.call("app.quit", {"force": True})
    try:
        app.wait(timeout=20)
    except subprocess.TimeoutExpired:
        app.kill()
    log.close()
    artifacts.append({"path": str(log_path), "sha256": hashlib.sha256(log_path.read_bytes()).hexdigest() if log_path.exists() else "", "size": log_path.stat().st_size if log_path.exists() else 0})
    report = {
        "status": theme_report_status(all_checks),
        "checks": all_checks,
        "artifacts": artifacts,
        "limitations": limitations,
        "themes": results,
    }
    report_path = out / "matrix.json"
    report_path.write_text(json.dumps(report, indent=1))
    bad = [r["theme"] for r in results if not r["match"]]
    print(f"themes={len(results)} mismatched={len(bad)} {bad}")
    print(f"status={report['status']} report={report_path}")
    return 0 if report["status"] == "PASS" else (1 if report["status"] == "FAIL" else 2)


if __name__ == "__main__":
    sys.exit(_self_test() if "--self-test" in sys.argv else main())
