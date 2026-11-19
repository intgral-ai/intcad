#!/usr/bin/env python3
"""Native-density receipts: offline validation and identified runtime capture.

    scripts/native-density.py --self-test
    scripts/native-density.py --check-log PATH --manifest PATH --out PATH
    scripts/native-density.py --app PATH --cef-root PATH --out DIR --task NAME

The report schema is the same in every mode:

    {"status": "PASS|FAIL|BLOCKED|NOT_RUN",
     "checks": [{"name": str, "status": str, "detail": str}],
     "artifacts": [{"path": str, "sha256": str, "size": int}],
     "limitations": [str]}

Receipts are bounded JSON lines in the app log: `CEF_RUNTIME {...}`,
`CEF_BROWSER {...}` and `CEF_FRAME {...}`. `requested_scale` is the app's
placement scale; it is not an independently observed native getter.

Only the Python standard library is used. Generated logs, screenshots,
resources and tokens stay local and are never committed.
"""
import argparse
import hashlib
import json
import math
import os
import re
import stat
import sys
import tempfile
import time
from pathlib import Path

REPORT_STATUSES = ("PASS", "FAIL", "BLOCKED", "NOT_RUN")
DEFAULT_SCALES = (1.0, 1.25, 1.5, 2.0)
TAG_FRAME = "CEF_FRAME"
TAG_RUNTIME = "CEF_RUNTIME"
TAG_BROWSER = "CEF_BROWSER"
TAG_LIMIT = "CEF_LIMIT"
KNOWN_TAGS = (TAG_RUNTIME, TAG_BROWSER, TAG_FRAME, TAG_LIMIT)
KNOWN_TAG_SET = frozenset(KNOWN_TAGS)
# Any `CEF_...` token followed by whitespace is a receipt tag. A known tag is
# parsed; an unknown one is a failure, so a stray `CEF_STATUS PARTIAL` cannot
# ride along beside valid receipts.
CEF_TAG_RE = re.compile(r"\b(CEF_[A-Z][A-Z0-9_]*)(?=\s)")
MAX_RECEIPT_BYTES = 4096
MAX_LINE_BYTES = 16384
MAX_DETAIL = 240
MAX_LIMITATION = 240
THEME_SOURCE = Path(__file__).resolve().parents[3] / "assets/styles/ui/themes"


def _status_rank(status):
    return {"PASS": 0, "NOT_RUN": 1, "BLOCKED": 2, "FAIL": 3}.get(status, 4)


def _aggregate(checks):
    """FAIL wins, then BLOCKED/NOT_RUN, so an unproven step never becomes PASS."""
    if not checks:
        return "NOT_RUN"
    worst = max(_status_rank(c.get("status")) for c in checks)
    return {0: "PASS", 1: "NOT_RUN", 2: "BLOCKED", 3: "FAIL"}.get(worst, "FAIL")


def _check(name, status, detail):
    if status not in REPORT_STATUSES:
        status = "FAIL"
    return {"name": name, "status": status, "detail": str(detail)[:MAX_DETAIL]}


def _report(checks, artifacts=None, limitations=None):
    return {
        "status": _aggregate(checks),
        "checks": checks,
        "artifacts": artifacts or [],
        "limitations": list(dict.fromkeys((limitations or []))),
    }


def _sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _normalised_text_sha256(path):
    text = Path(path).read_text(encoding="utf-8", errors="replace")
    return hashlib.sha256(text.strip().encode("utf-8")).hexdigest()


def read_receipts(path):
    """Parse bounded receipt lines; reject any `CEF_` tag outside the contract.

    Every line is scanned, but only lines that carry a `CEF_` tag are parsed or
    size-checked. A limit on the total number of log lines would let a verbose
    log drop real receipts, so lines without a tag are cheap to skip.
    """
    records, problems = [], []
    with open(path, "r", encoding="utf-8", errors="replace") as handle:
        for number, line in enumerate(handle, 1):
            if "CEF_" not in line:
                continue
            if len(line.encode("utf-8", "replace")) > MAX_LINE_BYTES:
                problems.append(f"line {number} exceeds {MAX_LINE_BYTES} bytes")
                continue
            for tag in dict.fromkeys(CEF_TAG_RE.findall(line)):
                if tag not in KNOWN_TAG_SET:
                    problems.append(f"line {number} has unknown receipt tag {tag}")
                    records.append({"tag": tag, "data": None, "problem": "unknown-tag"})
            for tag in KNOWN_TAGS:
                marker = tag + " "
                at = line.find(marker)
                if at < 0:
                    continue
                payload = line[at + len(marker):].strip()
                if len(payload.encode("utf-8", "replace")) > MAX_RECEIPT_BYTES:
                    problems.append(f"{tag} line {number} exceeds {MAX_RECEIPT_BYTES} bytes")
                    records.append({"tag": tag, "data": None, "problem": "oversized"})
                    break
                try:
                    data = json.loads(payload)
                except (ValueError, TypeError) as error:
                    problems.append(f"{tag} line {number}: invalid JSON: {error}")
                    records.append({"tag": tag, "data": None, "problem": "invalid-json"})
                    break
                records.append({"tag": tag, "data": data})
                break
    return records, problems


def _finite_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(float(value))


def _int_in(value, low, high):
    return isinstance(value, int) and not isinstance(value, bool) and low <= value <= high


def _pair_size(value, limit=32768):
    return (
        isinstance(value, list)
        and len(value) == 2
        and all(_int_in(edge, 0, limit) for edge in value)
        and all(edge > 0 for edge in value)
    )


def _rect_value(value, limit=32768):
    return (
        isinstance(value, list)
        and len(value) == 4
        and all(isinstance(part, int) and not isinstance(part, bool) for part in value)
        and 0 <= value[0] <= limit
        and 0 <= value[1] <= limit
        and 0 < value[2] <= limit
        and 0 < value[3] <= limit
    )


def _pair_positive(value, limit=65536):
    return (
        isinstance(value, list)
        and len(value) == 2
        and all(_finite_number(edge) and 0 < float(edge) <= limit for edge in value)
    )


def _frame_problem(frame):
    required = (
        "lifetime", "placement_revision", "element", "generation", "capture_counter",
        "requested_scale", "dip", "requested_physical", "physical", "coded", "visible",
        "content", "source", "imported_size", "imported_generation", "decision", "reason",
    )
    for name in required:
        if name not in frame:
            return f"missing {name}"
    if not _int_in(frame.get("lifetime"), 0, 2**63 - 1):
        return "invalid lifetime"
    if not _int_in(frame.get("placement_revision"), 0, 2**63 - 1):
        return "invalid placement_revision"
    if frame.get("element") not in ("view", "popup"):
        return "unknown element"
    if not _int_in(frame.get("generation"), 0, 2**63 - 1):
        return "invalid generation"
    counter = frame.get("capture_counter")
    if counter is not None and not _int_in(counter, 0, 2**63 - 1):
        return "invalid capture_counter"
    if not _finite_number(frame.get("requested_scale")) or not (0.25 <= float(frame["requested_scale"]) <= 8.0):
        return "invalid requested_scale"
    if not _pair_positive(frame.get("dip")):
        return "invalid dip"
    if not _pair_size(frame.get("requested_physical")):
        return "invalid requested_physical"
    effective = frame.get("effective_native_scale")
    if effective is not None and (not _finite_number(effective) or not (0.25 <= float(effective) <= 8.0)):
        return "invalid effective_native_scale"
    if not _pair_size(frame.get("physical")):
        return "invalid physical"
    if not _pair_size(frame.get("coded")):
        return "invalid coded"
    if not _rect_value(frame.get("visible")):
        return "invalid visible"
    if not _rect_value(frame.get("content")):
        return "invalid content"
    if frame.get("source") is None:
        return "missing source"
    if not _pair_size(frame.get("source")):
        return "invalid source"
    if not _pair_size(frame.get("imported_size")):
        return "invalid imported_size"
    if not _int_in(frame.get("imported_generation"), 0, 2**63 - 1):
        return "invalid imported_generation"
    if frame.get("decision") not in ("admitted", "refused"):
        return "unknown decision"
    if not isinstance(frame.get("reason"), (str, type(None))) or (isinstance(frame.get("reason"), str) and len(frame["reason"]) > 160):
        return "unbounded reason"
    return None


def _frame_matches_source(frame):
    coded = frame.get("coded")
    source = frame.get("source")
    imported = frame.get("imported_size")
    return coded == source == imported and frame.get("imported_generation") == frame.get("generation")


def _frame_geometry_problem(frame, max_edge=32768):
    visible, content, coded = frame["visible"], frame["content"], frame["coded"]
    full = [0, 0, coded[0], coded[1]]
    if visible != full or content != full:
        return "visible/content is not the full coded surface"
    if max(max(rect[0] + rect[2], rect[1] + rect[3]) for rect in (visible, content)) > max_edge:
        return "geometry exceeds bounded edge"
    return None


def _bounded_string(value, limit=160):
    return isinstance(value, str) and 0 < len(value) <= limit and all(ord(char) >= 32 for char in value)


def _scale_list(values):
    if not isinstance(values, (list, tuple)) or not values:
        return ()
    scales = []
    for value in values:
        if not _finite_number(value):
            return ()
        scale = float(value)
        if not (0.25 <= scale <= 8.0):
            return ()
        scales.append(scale)
    return tuple(sorted(set(scales)))


def _runtime_problem(runtime):
    if not isinstance(runtime, dict):
        return "runtime receipt is not an object"
    for name in ("backend", "osr_only", "disabled_features", "cef", "chromium", "api_version", "cef_commit"):
        if name not in runtime:
            return f"missing {name}"
    if runtime.get("backend") != "LinuxWayland":
        return "backend is not actual Linux Wayland"
    if runtime.get("osr_only") is not True:
        return "osr_only is not true"
    disabled = runtime.get("disabled_features")
    if not isinstance(disabled, list) or not all(_bounded_string(item, 128) for item in disabled):
        return "disabled_features is not a bounded list"
    if "Translate" not in disabled or "WaylandFractionalScaleV1" not in disabled:
        return "merged disabled features lost Translate or WaylandFractionalScaleV1"
    if not isinstance(runtime.get("cef"), list) or len(runtime["cef"]) != 3 or not all(_int_in(v, 0, 9999) for v in runtime["cef"]):
        return "invalid cef identity"
    if not isinstance(runtime.get("chromium"), list) or len(runtime["chromium"]) != 4 or not all(_int_in(v, 0, 99999) for v in runtime["chromium"]):
        return "invalid chromium identity"
    if not _int_in(runtime.get("api_version"), 1, 10**9):
        return "invalid API version"
    if not _bounded_string(runtime.get("cef_commit"), 64):
        return "invalid cef commit"
    return None


def _limit_problem(limit):
    if not isinstance(limit, dict):
        return "limit receipt is not an object"
    if not isinstance(limit.get("accelerated"), bool):
        return "limit accelerated is not a boolean"
    reason = limit.get("reason")
    if not isinstance(reason, str) or not reason or len(reason) > 160:
        return "limit reason is not a bounded non-empty string"
    return None


def _browser_problem(browser):
    if not isinstance(browser, dict):
        return "browser receipt is not an object"
    if not _int_in(browser.get("browser_id"), 1, 2**31 - 1):
        return "invalid browser_id"
    if browser.get("windowless") is not True:
        return "browser is not windowless"
    if browser.get("action") not in ("created", "popup_denied"):
        return "unknown browser action"
    return None


def validate_receipts(records, expected_scales=DEFAULT_SCALES):
    """Validate bounded app receipts and return the common report shape."""
    checks, limitations = [], [
        "requested_scale is the app's placement scale, not an independently observed native getter.",
        "dip is derived from physical/requested_scale, so receipt validation cannot independently prove native density; it checks scale coverage and actual callback/import identity.",
        "capture generations identify mailbox paints, not renderer commits.",
    ]
    if not isinstance(records, list) or not records:
        checks.append(_check("receipts_present", "FAIL", "no receipts were supplied"))
        return _report(checks, limitations=limitations)
    expected = _scale_list(expected_scales)
    if not expected:
        checks.append(_check("expected_scales", "FAIL", "expected scales are missing or out of range"))
        return _report(checks, limitations=limitations)

    runtimes, browsers, frames, problems = [], [], [], []
    for index, record in enumerate(records):
        if not isinstance(record, dict):
            problems.append(f"record {index} is not an object")
            continue
        tag = record.get("tag")
        if tag not in KNOWN_TAGS:
            problems.append(f"unknown receipt tag {tag!r}")
            continue
        data = record.get("data")
        if not isinstance(data, dict):
            problems.append(f"{tag} record {index} has no JSON object")
            continue
        if tag == TAG_RUNTIME:
            runtimes.append(data)
        elif tag == TAG_BROWSER:
            browsers.append(data)
        elif tag == TAG_FRAME:
            frames.append(data)
        elif data.get("status") is not None and data.get("status") not in REPORT_STATUSES:
            problems.append(f"unknown receipt status {data.get('status')!r}")
    checks.append(_check("receipt_tags", "FAIL" if problems else "PASS", "; ".join(problems[:6]) or "bounded known receipts"))

    if not runtimes:
        checks.append(_check("runtime_identity", "FAIL", "no CEF_RUNTIME receipt"))
    else:
        runtime_problems = [problem for problem in (_runtime_problem(item) for item in runtimes) if problem]
        if len(runtimes) > 1 and any(runtimes[0] != item for item in runtimes[1:]):
            runtime_problems.append("runtime receipts disagree")
        checks.append(_check("runtime_identity", "FAIL" if runtime_problems else "PASS", "; ".join(runtime_problems[:4]) or "identified Linux Wayland OSR runtime"))

    created = [browser for browser in browsers if isinstance(browser, dict) and browser.get("action") == "created"]
    browser_problems = [problem for problem in (_browser_problem(item) for item in browsers) if problem]
    if not created:
        browser_problems.append("no windowless browser created receipt")
    checks.append(_check("browser_identity", "FAIL" if browser_problems else "PASS", "; ".join(browser_problems[:4]) or "windowless browser created"))

    frame_problems = []
    refused_frames = 0
    for frame in frames:
        if frame.get("decision") == "refused" and frame.get("element") in ("view", "popup") and isinstance(frame.get("reason"), str):
            refused_frames += 1
            continue
        problem = _frame_problem(frame)
        if problem:
            frame_problems.append(problem)
            continue
        problem = _frame_geometry_problem(frame) or (None if _frame_matches_source(frame) else "source/coded/import mismatch")
        if problem:
            frame_problems.append(problem)
    checks.append(_check(
        "frame_schema",
        "FAIL" if frame_problems else "PASS",
        "; ".join(sorted(set(frame_problems))[:5]) or (f"{refused_frames} bounded refusal(s)" if refused_frames else "bounded complete frame receipts"),
    ))

    view_frames = [
        frame for frame in frames
        if _frame_problem(frame) is None and frame.get("element") == "view" and frame.get("decision") == "admitted"
    ]
    scale_checks = []
    for scale in expected:
        bad, blocked = [], []
        matches = [frame for frame in view_frames if abs(float(frame["requested_scale"]) - scale) <= 1e-4]
        if not matches:
            scale_checks.append(_check(f"scale_{scale:g}", "FAIL", "no admitted view frame at this requested scale"))
            continue
        bad = []
        for frame in matches:
            if _frame_geometry_problem(frame) or not _frame_matches_source(frame):
                bad.append("geometry/source mismatch")
                continue
            dip, physical = frame["dip"], frame["physical"]
            actual_scale = float(frame["requested_scale"])
            wanted = [math.ceil(float(dip[0]) * actual_scale), math.ceil(float(dip[1]) * actual_scale)]
            requested_physical = frame["requested_physical"]
            # Round the reported DIP, then ceil the physical extent. There is no
            # ±1 density allowance: one missing pixel is the defect under test.
            if requested_physical != wanted:
                bad.append(f"requested {dip} DIP at {actual_scale:g} is {requested_physical}, expected {wanted}")
            if physical != wanted:
                bad.append(f"capture physical {physical} is not the requested {wanted}")
            if physical != frame["coded"]:
                bad.append("native physical differs from coded size")
            effective = frame.get("effective_native_scale")
            if effective is None:
                blocked.append(f"no independent native scale for {dip} DIP at {actual_scale:g}")
            elif abs(float(effective) - actual_scale) > 1e-6:
                bad.append(f"independently observed scale {effective} differs from requested {actual_scale}")
        detail = "; ".join(sorted(set(bad))[:3])
        if not detail:
            detail = "; ".join(sorted(set(blocked))[:3]) or f"{len(matches)} admitted view frame(s), exact ceiling density"
        status = "FAIL" if bad else ("BLOCKED" if blocked else "PASS")
        scale_checks.append(_check(f"scale_{scale:g}", status, detail))
    checks.extend(scale_checks)

    popup_problems = []
    admitted_views = {}
    for frame in frames:
        if _frame_problem(frame) is not None:
            continue
        if frame.get("element") == "view" and frame.get("decision") == "admitted":
            admitted_views.setdefault(frame["lifetime"], []).append(frame)
    for frame in frames:
        if _frame_problem(frame) is not None or frame.get("element") != "popup":
            continue
        if frame.get("decision") == "admitted":
            if frame["lifetime"] not in admitted_views:
                popup_problems.append("popup admitted without a view in the same page lifetime")
            geometry = _frame_geometry_problem(frame)
            if geometry:
                popup_problems.append(f"popup {geometry}")
            elif not _frame_matches_source(frame):
                popup_problems.append("popup source/coded/import mismatch")
            if frame["physical"] != frame["coded"]:
                popup_problems.append("popup native size differs from its coded surface")
        elif not isinstance(frame.get("reason"), str):
            popup_problems.append("refused popup has no reason")
    checks.append(_check("popup_surface", "FAIL" if popup_problems else "PASS", "; ".join(popup_problems[:4]) or "popup receipts are separate surfaces"))

    limits = [record.get("data") for record in records if isinstance(record, dict) and record.get("tag") == TAG_LIMIT and isinstance(record.get("data"), dict)]
    limit_problems = [problem for problem in (_limit_problem(item) for item in limits) if problem]
    accelerated = [item for item in limits if item.get("accelerated") is False]
    positive = [item for item in limits if item.get("accelerated") is True]
    if limit_problems:
        checks.append(_check("accelerated_import", "FAIL", "; ".join(limit_problems[:4])))
    elif accelerated:
        reasons = ", ".join(sorted({str(item.get("reason", "unknown"))[:80] for item in accelerated}))
        checks.append(_check("accelerated_import", "BLOCKED", f"accelerated import unavailable: {reasons}"))
        limitations.append("A graceful refusal on llvmpipe/no-interop is not accelerated evidence.")
    elif positive:
        checks.append(_check("accelerated_import", "PASS", "explicit accelerated import receipt"))
    else:
        # Absence used to count as PASS. It is not evidence: a run without an
        # accelerated receipt leaves the accelerated step unobserved.
        checks.append(_check("accelerated_import", "NOT_RUN", "no CEF_LIMIT accelerated receipt"))
        limitations.append("No explicit accelerated-import receipt: accelerated OSR remains unproven for this run.")

    sequence = [(frame["lifetime"], frame["placement_revision"]) for frame in view_frames]
    aba = False
    for index, key in enumerate(sequence):
        if key in sequence[:index]:
            between = sequence[sequence.index(key) + 1:index]
            if any(item[0] == key[0] and item[1] != key[1] for item in between):
                aba = True
                break
    geometry_sequence = [(frame["lifetime"], tuple(frame["physical"])) for frame in view_frames]
    if len(geometry_sequence) >= 3:
        first, middle, third = geometry_sequence[0], geometry_sequence[1], geometry_sequence[2]
        if first[0] == third[0] and first[1] == third[1] and middle[0] != first[0]:
            aba = True
            limitations.append("late same-geometry A→B→A capture identities remain indistinguishable from causal freshness; no native epoch is inferred.")
    if aba:
        checks.append(_check("causal_freshness", "NOT_RUN", "same-geometry or same-lifetime A→B→A revisions; capture identities do not prove renderer freshness"))
        limitations.append("late same-lifetime A→B→A revisions remain indistinguishable from causal freshness; no native epoch is inferred.")

    revisions = {}
    identities = {}
    for frame in view_frames:
        revisions.setdefault(frame["lifetime"], set()).add(frame["placement_revision"])
        key = (frame["lifetime"], frame["placement_revision"], tuple(frame["dip"]), tuple(frame["requested_physical"]), float(frame["requested_scale"]))
        identities.setdefault(key, []).append(frame)
    if any(len(found) > 1 for found in revisions.values()) and not aba:
        checks.append(_check("placement_identity", "NOT_RUN", "placement revision changed; admission dedupe cannot be read as freshness"))
        limitations.append("a pending placement transition is visible only as separate revisions; its admitted frame is not causal freshness proof.")
    if any(len(frames) > 1 for frames in identities.values()):
        checks.append(_check("causal_freshness", "NOT_RUN", "new capture counters/generations under one placement identity are not causal freshness"))
        limitations.append("new capture_counter/document nonce under the same placement identity does not prove a renderer epoch.")

    return _report(checks, limitations=limitations)


def _manifest_artifact_problem(entry, base):
    if not isinstance(entry, dict):
        return "artifact entry is not an object"
    path = entry.get("path")
    if not isinstance(path, str) or not path:
        return "artifact has no path"
    resolved = Path(path)
    if not resolved.is_absolute():
        resolved = Path(base) / resolved
    if not resolved.is_file():
        return f"missing artifact {path}"
    try:
        size = resolved.stat().st_size
        whole = _sha256_file(resolved)
    except OSError as error:
        return f"cannot hash {path}: {error}"
    if entry.get("size") is not None and entry["size"] != size:
        return f"size changed for {path}"
    if entry.get("sha256") != whole:
        # A text-normalised hash can stay equal after a same-length edit; the
        # whole-file hash is the evidence and is checked first.
        return f"whole-file hash changed for {path}"
    if entry.get("text_sha256") is not None and _normalised_text_sha256(resolved) != entry["text_sha256"]:
        return f"normalised text hash changed for {path}"
    return None


def validate_manifest(manifest, base_dir):
    """Check whole-file hashes, resources and the complete built-in theme set."""
    checks = []
    base = Path(base_dir)
    if not isinstance(manifest, dict):
        return _report([_check("manifest_schema", "FAIL", "manifest is not an object")])
    unknown = manifest.get("status")
    if unknown is not None and unknown not in REPORT_STATUSES:
        return _report([_check("manifest_status", "FAIL", f"unknown manifest status {unknown!r}")])
    expected_themes = theme_ids_from_source()
    themes = manifest.get("themes")
    if not isinstance(themes, list) or sorted(themes) != sorted(expected_themes) or len(set(themes)) != len(expected_themes):
        checks.append(_check("theme_set", "FAIL", f"manifest must list all {len(expected_themes)} built-in themes exactly once"))
    else:
        checks.append(_check("theme_set", "PASS", f"{len(expected_themes)} built-in themes enumerated from source"))

    artifacts = manifest.get("artifacts", [])
    resources = manifest.get("resources", [])
    for label, entries in (("artifact_hashes", artifacts), ("resource_hashes", resources)):
        if not isinstance(entries, list):
            checks.append(_check(label, "FAIL", "entry list is not a list"))
            continue
        problems = [problem for problem in (_manifest_artifact_problem(entry, base) for entry in entries) if problem]
        checks.append(_check(label, "FAIL" if problems else "PASS", "; ".join(problems[:5]) or f"{len(entries)} whole file(s) unchanged"))

    # Inventory roles: an empty or anonymous list is never a passing artifact
    # bundle; the app and the supplied CEF root are mandatory, exactly once.
    if not isinstance(artifacts, list) or not artifacts:
        checks.append(_check("artifact_roles", "FAIL", "empty artifact inventory"))
    else:
        roles = [entry.get("role") for entry in artifacts if isinstance(entry, dict)]
        problems = []
        for role in ("app", "supplied-libcef"):
            count = roles.count(role)
            if count == 0:
                problems.append(f"missing mandatory role {role}")
            elif count > 1:
                problems.append(f"duplicate mandatory role {role}")
        checks.append(_check("artifact_roles", "FAIL" if problems else "PASS", "; ".join(problems[:4]) or "app and supplied-libcef present exactly once"))

    # Resource origin: every resource must stay inside the declared CEF root (or
    # the manifest's own directory) and the inventory cannot be empty.
    if not isinstance(resources, list) or not resources:
        checks.append(_check("resource_inventory", "FAIL", "missing mandatory CEF resources"))
    else:
        root = Path(manifest.get("cef_root", base)).resolve()
        if not root.is_absolute():
            root = (base / root).resolve()
        escapes = []
        for entry in resources:
            if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
                escapes.append("resource entry has no path")
                continue
            path = Path(entry["path"])
            resolved = path.resolve() if path.is_absolute() else (base / path).resolve()
            try:
                resolved.relative_to(root)
            except ValueError:
                escapes.append(f"resource escapes its origin: {entry['path']}")
            if entry.get("origin") not in (None, "cef-root"):
                escapes.append(f"unknown resource origin for {entry['path']}")
        checks.append(_check("resource_origin", "FAIL" if escapes else "PASS", "; ".join(escapes[:4]) or "resources stay under their declared CEF root"))

    # A self-consistent manifest is not authenticated origin. Without an explicit
    # authenticated receipt the origin remains BLOCKED, never PASS.
    origin = manifest.get("origin")
    if not isinstance(origin, dict) or origin.get("authenticated") is not True:
        checks.append(_check("authenticated_origin", "BLOCKED", "no authenticated artifact origin receipt"))
    elif not _bounded_string(origin.get("source"), 160) or not isinstance(origin.get("sha256"), str) or len(origin["sha256"]) != 64:
        checks.append(_check("authenticated_origin", "FAIL", "authenticated origin receipt is malformed"))
    else:
        checks.append(_check("authenticated_origin", "PASS", f"origin {origin['source']} recorded"))
    return _report(checks)


def parse_stat_line(line):
    right = line.rfind(")")
    left = line.find("(")
    if left < 0 or right <= left:
        return None
    head = line[:left].strip()
    after = line[right + 1:].split()
    if not head.isdigit() or len(after) < 20:
        return None
    try:
        return {"pid": int(head), "ppid": int(after[1]), "starttime": int(after[19])}
    except ValueError:
        return None


def _proc_stat(pid):
    try:
        line = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    return parse_stat_line(line)


def identity_problem(before, after, expected_starttime=None):
    """Synthetic proc-record check: missing, reused or wrong-generation identity."""
    if not isinstance(before, dict):
        return "process record is missing"
    if expected_starttime is not None and before.get("starttime") != expected_starttime:
        return "starttime does not match the requested launch"
    if after is not None and (after.get("pid") != before.get("pid") or after.get("starttime") != before.get("starttime")):
        return "PID/starttime was reused during capture"
    return None


def helper_identity_problem(helper, parent_pid, expected_starttime=None):
    """A CEF helper must belong to the intended parent and generation."""
    if not isinstance(helper, dict):
        return "helper process record is missing"
    if helper.get("ppid") != parent_pid:
        return "helper does not belong to the intended app PID"
    if not isinstance(helper.get("starttime"), int) or helper["starttime"] <= 0:
        return "helper has no valid starttime"
    if expected_starttime is not None and helper["starttime"] != expected_starttime:
        return "helper starttime does not match the owned launch"
    return None


def _resource_artifacts(cef_root):
    artifacts, problems = [], []
    count = 0
    for directory, names, files in os.walk(cef_root):
        names.sort()
        for name in sorted(files):
            count += 1
            if count > 4096:
                problems.append("resource inventory exceeds 4096 files")
                return artifacts, problems
            path = Path(directory) / name
            try:
                mode = path.lstat().st_mode
                resolved = path.resolve(strict=True)
                if not stat.S_ISREG(resolved.stat().st_mode):
                    continue
                artifacts.append({
                    "path": str(resolved),
                    "sha256": _sha256_file(resolved),
                    "size": resolved.stat().st_size,
                    "role": "cef-resource",
                    "symlink": stat.S_ISLNK(mode),
                })
            except OSError as error:
                problems.append(f"cannot hash {path}: {error}")
    return artifacts, problems


def capture_bundle(app, cef_root, pid):
    """Hash the app/CEF resources and verify the real mapped libcef path."""
    checks, artifacts, limitations = [], [], []
    app, cef_root, pid = Path(app), Path(cef_root), int(pid)
    if app.is_file():
        try:
            artifacts.append({"path": str(app.resolve()), "sha256": _sha256_file(app), "size": app.stat().st_size, "role": "app"})
            checks.append(_check("app_identity", "PASS", "app whole-file hash recorded"))
            try:
                running = Path(f"/proc/{pid}/exe").resolve(strict=True)
            except OSError as error:
                checks.append(_check("app_pid_identity", "FAIL", f"cannot read the running executable: {error}"))
            else:
                if running != app.resolve():
                    checks.append(_check("app_pid_identity", "FAIL", f"supplied app {app} is not the running executable {running}"))
                else:
                    checks.append(_check("app_pid_identity", "PASS", f"supplied app is /proc/{pid}/exe"))
        except OSError as error:
            checks.append(_check("app_identity", "FAIL", f"cannot hash app: {error}"))
    else:
        checks.append(_check("app_identity", "FAIL", "app path is not a regular file"))

    supplied_libcef = cef_root / "libcef.so"
    if cef_root.is_dir() and supplied_libcef.is_file():
        try:
            artifacts.append({"path": str(supplied_libcef.resolve()), "sha256": _sha256_file(supplied_libcef), "size": supplied_libcef.stat().st_size, "role": "supplied-libcef"})
            checks.append(_check("cef_root", "PASS", "supplied CEF root contains libcef.so (not treated as proof of the mapped library)"))
        except OSError as error:
            checks.append(_check("cef_root", "FAIL", f"cannot hash supplied libcef: {error}"))
    else:
        checks.append(_check("cef_root", "FAIL", "supplied CEF root has no libcef.so"))

    before = _proc_stat(pid)
    if before is None:
        checks.append(_check("process_starttime", "FAIL", f"/proc/{pid}/stat is unavailable"))
    else:
        checks.append(_check("process_starttime", "PASS", f"pid {before['pid']} starttime {before['starttime']}"))
        maps_path = Path(f"/proc/{pid}/maps")
        try:
            mapped = []
            for line in maps_path.read_text(encoding="utf-8", errors="replace").splitlines():
                fields = line.split(maxsplit=5)
                if len(fields) == 6 and fields[5].startswith("/"):
                    mapped.append(fields[5])
        except OSError as error:
            mapped = []
            checks.append(_check("mapped_libcef", "BLOCKED", f"cannot read process maps: {error}"))
        else:
            unique = sorted(set(mapped))
            candidates = [path for path in unique if Path(path).name == "libcef.so" or path.endswith("/libcef.so")]
            if not candidates:
                checks.append(_check("mapped_libcef", "BLOCKED", "libcef.so is not mapped in this process yet"))
                limitations.append("A supplied CEF_PATH or libcef text alone is not proof that CEF was loaded.")
            else:
                problems = []
                mapped_hashes = []
                for path in candidates:
                    try:
                        resolved = Path(path).resolve(strict=True)
                        mapped_hashes.append({"path": str(resolved), "sha256": _sha256_file(resolved), "size": resolved.stat().st_size, "role": "mapped-libcef"})
                        if resolved != supplied_libcef.resolve(strict=True):
                            problems.append(f"mapped {resolved} is outside the supplied CEF root")
                    except OSError as error:
                        problems.append(f"cannot hash mapped {path}: {error}")
                artifacts.extend(mapped_hashes)
                checks.append(_check("mapped_libcef", "FAIL" if problems else "PASS", "; ".join(problems[:3]) or "actual mapped libcef matches the supplied root"))
        resources, resource_problems = _resource_artifacts(cef_root) if cef_root.is_dir() else ([], ["CEF root is not a directory"])
        artifacts.extend(resources)
        checks.append(_check("resource_inventory", "FAIL" if resource_problems else "PASS", "; ".join(resource_problems[:4]) or f"{len(resources)} CEF resource file(s) hashed"))
        after = _proc_stat(pid)
        problem = identity_problem(before, after)
        checks.append(_check("pid_reuse", "FAIL" if problem else "PASS", problem or "PID/starttime stayed stable during capture"))
    return _report(checks, artifacts=artifacts, limitations=limitations)


def theme_ids_from_source():
    return sorted(path.stem for path in THEME_SOURCE.glob("*.json"))


# ── self-tests ──────────────────────────────────────────────────────────────


def _fixture_records(scales=DEFAULT_SCALES, lifetimes=None, postlude=True, limits=True):
    if lifetimes is None:
        lifetimes = [6 + index for index in range(len(scales))]
    records = [
        {"tag": TAG_RUNTIME, "data": {
            "backend": "LinuxWayland", "osr_only": True,
            "disabled_features": ["Translate", "WaylandFractionalScaleV1"],
            "cef": [154, 0, 34], "chromium": [154, 0, 8037, 98],
            "api_version": 15400,
            "cef_commit": "14c5a089e8452874cb1a556dc3fde26ed2d87723",
        }},
        {"tag": TAG_BROWSER, "data": {"browser_id": 7, "windowless": True, "action": "created"}},
    ]
    generation = 10
    for scale, lifetime in zip(scales, lifetimes):
        dip = [370, 898]
        physical = [math.ceil(dip[0] * scale), math.ceil(dip[1] * scale)]
        generation += 1
        records.append({"tag": TAG_FRAME, "data": {
            "lifetime": lifetime, "placement_revision": 1, "element": "view",
            "generation": generation, "capture_counter": generation + 400,
            "requested_scale": scale, "dip": dip, "requested_physical": physical,
            "effective_native_scale": scale, "physical": physical,
            "coded": physical, "visible": [0, 0, physical[0], physical[1]],
            "content": [0, 0, physical[0], physical[1]], "source": physical,
            "imported_size": physical, "imported_generation": generation,
            "decision": "admitted", "reason": None,
        }})
    if limits:
        records.append({"tag": TAG_LIMIT, "data": {"accelerated": True, "reason": "accelerated texture import admitted"}})
    if postlude:
        records.append({"tag": TAG_BROWSER, "data": {"browser_id": 7, "windowless": True, "action": "popup_denied"}})
    return records


def _assert_status(report, wanted):
    if report.get("status") != wanted:
        raise AssertionError(f"expected {wanted}, got {report.get('status')}: {report.get('checks')}")


def _test_complete_receipts_pass():
    report = validate_receipts(_fixture_records(), DEFAULT_SCALES)
    _assert_status(report, "PASS")
    if len(report["checks"]) < 4:
        raise AssertionError("expected scale and identity checks")


def _test_missing_runtime_identity_fails():
    records = _fixture_records()
    records[0]["data"].pop("cef_commit")
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")
    records = _fixture_records()
    records[0]["data"]["backend"] = "LinuxX11"
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")


def _test_missing_source_fails():
    records = _fixture_records()
    records[2]["data"]["source"] = None
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")


def _test_import_source_mismatch_fails():
    records = _fixture_records()
    records[2]["data"]["imported_size"] = [records[2]["data"]["coded"][0] + 2, records[2]["data"]["coded"][1]]
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")
    records = _fixture_records()
    records[2]["data"]["imported_generation"] += 1
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")


def _test_missing_expected_scale_fails():
    records = _fixture_records((1.0, 1.25, 1.5))
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")


def _test_popup_without_admitted_view_fails():
    records = _fixture_records()
    popup = dict(records[2]["data"])
    popup.update({"element": "popup", "lifetime": 99, "generation": 999, "imported_generation": 999})
    records.insert(2, {"tag": TAG_FRAME, "data": popup})
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")


def _test_unknown_or_blocking_status_fails():
    records = _fixture_records()
    records.append({"tag": TAG_FRAME, "data": {"element": "view", "decision": "maybe"}})
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")
    records = _fixture_records()
    records.append({"tag": "CEF_STATUS", "data": {"status": "PARTIAL"}})
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")


def _test_incomplete_theme_set_fails():
    ids = theme_ids_from_source()
    if len(ids) != 25 or len(set(ids)) != 25:
        raise AssertionError(f"source theme set is not 25 unique ids: {len(ids)}")
    with tempfile.TemporaryDirectory(prefix="native-density-themes-") as tmp:
        base = Path(tmp)
        manifest = {"themes": ids[:-1], "artifacts": []}
        _assert_status(validate_manifest(manifest, base), "FAIL")
        manifest["themes"] = ids + [ids[0]]
        _assert_status(validate_manifest(manifest, base), "FAIL")


def _complete_manifest(base, artifacts):
    app = base / "app.bin"
    app.write_bytes(b"app binary")
    resources_dir = base / "cef"
    resources_dir.mkdir(exist_ok=True)
    resource = resources_dir / "libcef.so"
    resource.write_bytes(b"cef resource")
    artifacts.append({"path": "app.bin", "sha256": _sha256_file(app), "size": app.stat().st_size, "role": "app"})
    artifacts.append({"path": "cef/libcef.so", "sha256": _sha256_file(resource), "size": resource.stat().st_size, "role": "supplied-libcef"})
    return {
        "themes": theme_ids_from_source(),
        "artifacts": artifacts,
        "resources": [{
            "path": "cef/libcef.so",
            "sha256": _sha256_file(resource),
            "size": resource.stat().st_size,
            "role": "cef-resource",
            "origin": "cef-root",
        }],
        "origin": {"authenticated": True, "source": "stock-cef-fixture", "sha256": "0" * 64},
    }


def _test_altered_whole_file_hash_with_same_text_hash_fails():
    with tempfile.TemporaryDirectory(prefix="native-density-hash-") as tmp:
        base = Path(tmp)
        artifact = base / "bundle.bin"
        artifact.write_text("alpha\n", encoding="utf-8")
        manifest = _complete_manifest(base, [{
            "path": "bundle.bin",
            "sha256": _sha256_file(artifact),
            "size": artifact.stat().st_size,
            "text_sha256": _normalised_text_sha256(artifact),
            "role": "bundle",
        }])
        if validate_manifest(manifest, base)["status"] != "PASS":
            raise AssertionError("unchanged manifest should pass")
        artifact.write_text("alpha ", encoding="utf-8")
        if _normalised_text_sha256(artifact) != manifest["artifacts"][0]["text_sha256"]:
            raise AssertionError("fixture did not keep the normalised text hash equal")
        _assert_status(validate_manifest(manifest, base), "FAIL")


def _test_mismatched_resources_fail():
    with tempfile.TemporaryDirectory(prefix="native-density-resources-") as tmp:
        base = Path(tmp)
        resource = base / "libcef.so"
        resource.write_bytes(b"first")
        manifest = {
            "themes": theme_ids_from_source(),
            "resources": [{"path": "libcef.so", "sha256": _sha256_file(resource), "size": 5}],
        }
        resource.write_bytes(b"other")
        _assert_status(validate_manifest(manifest, base), "FAIL")



def _test_empty_inventory_and_missing_roles_never_pass():
    with tempfile.TemporaryDirectory(prefix="native-density-inventory-") as tmp:
        base = Path(tmp)
        manifest = _complete_manifest(base, [])
        manifest["artifacts"] = []
        _assert_status(validate_manifest(manifest, base), "FAIL")
        manifest = _complete_manifest(base, [])
        manifest["artifacts"] = [entry for entry in manifest["artifacts"] if entry.get("role") != "app"]
        _assert_status(validate_manifest(manifest, base), "FAIL")
        manifest = _complete_manifest(base, [])
        manifest["artifacts"].append(dict(manifest["artifacts"][0]))
        _assert_status(validate_manifest(manifest, base), "FAIL")


def _test_resource_escape_or_origin_mismatch_fails():
    with tempfile.TemporaryDirectory(prefix="native-density-escape-") as tmp:
        base = Path(tmp)
        outside = base.parent / f"{base.name}-outside-libcef.so"
        outside.write_bytes(b"outside")
        manifest = _complete_manifest(base, [])
        manifest["resources"][0]["path"] = str(outside)
        _assert_status(validate_manifest(manifest, base), "FAIL")
        manifest = _complete_manifest(base, [])
        manifest["resources"][0]["origin"] = "host"
        _assert_status(validate_manifest(manifest, base), "FAIL")
        outside.unlink(missing_ok=True)


def _test_missing_authenticated_origin_is_blocked():
    with tempfile.TemporaryDirectory(prefix="native-density-origin-") as tmp:
        base = Path(tmp)
        manifest = _complete_manifest(base, [])
        manifest.pop("origin")
        report = validate_manifest(manifest, base)
        _assert_status(report, "BLOCKED")
        if not any(check["name"] == "authenticated_origin" and check["status"] == "BLOCKED" for check in report["checks"]):
            raise AssertionError("missing origin must BLOCK")


def _test_supplied_app_must_match_pid_executable():
    with tempfile.TemporaryDirectory(prefix="native-density-pid-") as tmp:
        base = Path(tmp)
        cef_root = base / "cef"
        cef_root.mkdir()
        (cef_root / "libcef.so").write_bytes(b"not really cef")
        other = base / "other-app"
        other.write_bytes(b"not the running executable")
        report = capture_bundle(other, cef_root, os.getpid())
        if not any(check["name"] == "app_pid_identity" and check["status"] == "FAIL" for check in report["checks"]):
            raise AssertionError("a supplied app that is not /proc/pid/exe must FAIL")
        real = Path("/proc/self/exe").resolve()
        report = capture_bundle(real, cef_root, os.getpid())
        if not any(check["name"] == "app_pid_identity" and check["status"] == "PASS" for check in report["checks"]):
            raise AssertionError("the real running executable must match")


def _test_task_identity_is_explicit():
    if task_identity_problem("", "/tmp", "linux") is None:
        raise AssertionError("an empty task name must be refused")
    if task_identity_problem("t", "/definitely/missing-source", "linux") is None:
        raise AssertionError("a missing source path must be refused")
    if task_identity_problem("t", tempfile.gettempdir(), "solaris") is None:
        raise AssertionError("an unknown platform must be refused")
    if task_identity_problem("t", tempfile.gettempdir(), "linux") is not None:
        raise AssertionError("an owned task/source/platform must be accepted")


def _test_pid_reuse_wrong_generation_and_helper_mismatch_fail():
    before = {"pid": 4242, "ppid": 7, "starttime": 100}
    if identity_problem(before, before) is not None:
        raise AssertionError("an unchanged record must be accepted")
    if identity_problem(before, {"pid": 4242, "ppid": 7, "starttime": 101}) != "PID/starttime was reused during capture":
        raise AssertionError("a reused PID must fail")
    if identity_problem(before, before, expected_starttime=99) != "starttime does not match the requested launch":
        raise AssertionError("a wrong owned generation must fail")
    if identity_problem(None, before) != "process record is missing":
        raise AssertionError("a missing process record must fail")
    if helper_identity_problem({"pid": 5, "ppid": 4242, "starttime": 100}, 4242, 100) is not None:
        raise AssertionError("an owned helper must be accepted")
    if helper_identity_problem({"pid": 5, "ppid": 9999, "starttime": 100}, 4242, 100) != "helper does not belong to the intended app PID":
        raise AssertionError("a foreign helper must fail")
    if helper_identity_problem({"pid": 5, "ppid": 4242, "starttime": 101}, 4242, 100) != "helper starttime does not match the owned launch":
        raise AssertionError("a reused helper generation must fail")
    if helper_identity_problem(None, 4242, 100) != "helper process record is missing":
        raise AssertionError("a missing helper record must fail")


def _test_missing_pid_fails_instead_of_passing():
    if _proc_stat(2**22) is not None:
        return
    report = capture_bundle(Path("/bin/true"), Path("/nonexistent-cef"), 2**22)
    if not any(check["name"] == "process_starttime" and check["status"] == "FAIL" for check in report["checks"]):
        raise AssertionError("a missing process must FAIL identity")

def _test_aba_same_geometry_reports_limitation_not_pass():
    records = _fixture_records((1.25, 1.25, 1.25), lifetimes=[10, 11, 10])
    # A→B→A: alternate lifetimes so the third equals the first.
    records[2]["data"]["generation"] = 20
    records[2]["data"]["imported_generation"] = 20
    records[3]["data"]["generation"] = 21
    records[3]["data"]["imported_generation"] = 21
    records[3]["data"]["lifetime"] = 12
    records[4]["data"]["generation"] = 22
    records[4]["data"]["imported_generation"] = 22
    report = validate_receipts(records, (1.25,))
    if report["status"] == "PASS":
        raise AssertionError("A→B→A must not be a causal-freshness PASS")
    joined = " ".join(report["limitations"])
    if "same-geometry" not in joined:
        raise AssertionError("missing same-geometry limitation")
    if not any(c["name"] == "causal_freshness" and c["status"] != "PASS" for c in report["checks"]):
        raise AssertionError("causal freshness must stay unproven")


def _test_llvmpipe_or_no_interop_is_blocked_not_pass():
    records = _fixture_records()
    records.append({"tag": TAG_LIMIT, "data": {"accelerated": False, "reason": "llvmpipe"}})
    report = validate_receipts(records, DEFAULT_SCALES)
    if report["status"] == "PASS":
        raise AssertionError("llvmpipe must not become PASS")
    if not any(c["name"] == "accelerated_import" and c["status"] == "BLOCKED" for c in report["checks"]):
        raise AssertionError("accelerated step must be BLOCKED")
    records = _fixture_records()
    records.append({"tag": TAG_LIMIT, "data": {"accelerated": False, "reason": "VULKAN_EXTERNAL_MEMORY_DMA_BUF unavailable"}})
    if validate_receipts(records, DEFAULT_SCALES)["status"] == "PASS":
        raise AssertionError("no-interop must not become PASS")


def _test_missing_accelerated_receipt_is_not_pass():
    records = _fixture_records(limits=False)
    report = validate_receipts(records, DEFAULT_SCALES)
    if report["status"] == "PASS":
        raise AssertionError("an absent accelerated receipt must not PASS")
    if not any(check["name"] == "accelerated_import" and check["status"] != "PASS" for check in report["checks"]):
        raise AssertionError("accelerated_import must not PASS without a positive receipt")


def _test_density_physical_tolerance():
    # There is no ±1 tolerance: a capture one pixel short of the exact request fails.
    records = _fixture_records((1.25,))
    frame = records[2]["data"]
    frame.update({
        "dip": [370, 898],
        "requested_physical": [463, 1123],
        "physical": [463, 1122],
        "coded": [463, 1122],
        "visible": [0, 0, 463, 1122],
        "content": [0, 0, 463, 1122],
        "source": [463, 1122],
        "imported_size": [463, 1122],
        "effective_native_scale": 1.25,
    })
    report = validate_receipts(records, (1.25,))
    _assert_status(report, "FAIL")
    if not any(check["name"] == "scale_1.25" and check["status"] == "FAIL" for check in report["checks"]):
        raise AssertionError("a one-pixel capture mismatch must fail exact density")
    # The exact 463x1123 fixture passes.
    _assert_status(validate_receipts(_fixture_records((1.25,)), (1.25,)), "PASS")


def _revision_records(revisions, lifetime=10, scale=1.25, counters=None):
    source = _fixture_records((scale,), limits=False, postlude=False)
    records = source[:2]
    base = source[2]["data"]
    for index, revision in enumerate(revisions):
        frame = dict(base)
        frame.update({
            "lifetime": lifetime,
            "placement_revision": revision,
            "generation": 30 + index,
            "imported_generation": 30 + index,
            "capture_counter": (counters or [])[index] if counters and index < len(counters) else 700 + index,
        })
        records.append({"tag": TAG_FRAME, "data": frame})
    records.append({"tag": TAG_LIMIT, "data": {"accelerated": True, "reason": "accelerated texture import admitted"}})
    return records


def _checks(report, name):
    return [check for check in report["checks"] if check["name"] == name]


def _test_requested_dip_cannot_be_reconstructed_from_capture():
    records = _fixture_records((1.25,))
    frame = records[2]["data"]
    frame.update({
        "dip": [370, 898],
        "requested_physical": [463, 1123],
        "physical": [464, 1122],
        "coded": [464, 1122],
        "visible": [0, 0, 464, 1122],
        "content": [0, 0, 464, 1122],
        "source": [464, 1122],
        "imported_size": [464, 1122],
    })
    report = validate_receipts(records, (1.25,))
    _assert_status(report, "FAIL")
    if not any("capture physical" in check["detail"] for check in _checks(report, "scale_1.25")):
        raise AssertionError("the capture size must not be used to reconstruct the requested DIP")


def _test_missing_effective_native_scale_blocks_exact_density():
    records = _fixture_records((1.25,))
    records[2]["data"].pop("effective_native_scale")
    report = validate_receipts(records, (1.25,))
    _assert_status(report, "BLOCKED")
    if not _checks(report, "scale_1.25") or _checks(report, "scale_1.25")[0]["status"] != "BLOCKED":
        raise AssertionError("unknown native scale must BLOCK exact density")


def _test_same_lifetime_placement_aba_is_detected():
    report = validate_receipts(_revision_records([1, 2, 1]), (1.25,))
    if report["status"] == "PASS":
        raise AssertionError("A→B→A revisions must not PASS")
    if not any(check["name"] == "causal_freshness" and check["status"] == "NOT_RUN" for check in report["checks"]):
        raise AssertionError("same-lifetime revision ABA must stay NOT_RUN")


def _test_later_aba_after_unrelated_prefix_is_detected():
    report = validate_receipts(_revision_records([1, 2, 3, 2]), (1.25,))
    if not any(check["name"] == "causal_freshness" and check["status"] != "PASS" for check in report["checks"]):
        raise AssertionError("a later A→B→A must still be detected")


def _test_pending_b_transition_is_not_hidden_by_admission_dedup():
    report = validate_receipts(_revision_records([1, 2]), (1.25,))
    if not any(check["name"] == "placement_identity" and check["status"] == "NOT_RUN" for check in report["checks"]):
        raise AssertionError("a pending B transition must not be hidden as one placement")


def _test_new_capture_counter_or_document_nonce_is_not_causal_proof():
    report = validate_receipts(_revision_records([1, 1], counters=[700, 900]), (1.25,))
    if report["status"] == "PASS":
        raise AssertionError("new counters under one placement identity must not PASS")
    if not any(check["name"] == "causal_freshness" and check["status"] == "NOT_RUN" for check in report["checks"]):
        raise AssertionError("a new capture counter is not causal proof")


def _test_partial_visible_fails():
    records = _fixture_records()
    frame = records[2]["data"]
    frame["visible"] = [0, 0, frame["coded"][0], frame["coded"][1] // 2]
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")


def _test_popup_import_mismatch_fails():
    records = _fixture_records()
    popup = dict(records[2]["data"])
    popup.update({"element": "popup", "generation": 900, "imported_generation": 900, "capture_counter": 1300})
    popup["imported_size"] = [popup["coded"][0] + 2, popup["coded"][1]]
    records.insert(3, {"tag": TAG_FRAME, "data": popup})
    _assert_status(validate_receipts(records, DEFAULT_SCALES), "FAIL")


def _test_empty_log_with_valid_manifest_fails():
    with tempfile.TemporaryDirectory(prefix="native-density-empty-") as tmp:
        base = Path(tmp)
        log = base / "app.log"
        log.write_bytes(b"")
        manifest = {
            "themes": theme_ids_from_source(),
            "artifacts": [{"path": "app.log", "sha256": _sha256_file(log), "size": 0}],
        }
        manifest_path = base / "manifest.json"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        report_path = base / "report.json"
        code = main(["--check-log", str(log), "--manifest", str(manifest_path), "--out", str(report_path)])
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if code != 1 or report.get("status") != "FAIL":
            raise AssertionError(f"empty log with a valid manifest must FAIL: exit={code} status={report.get('status')}")
        if not any(check["name"] == "receipts_present" and check["status"] == "FAIL" for check in report["checks"]):
            raise AssertionError("an empty log must report receipts_present FAIL")


def _test_unknown_tag_through_cli_fails():
    with tempfile.TemporaryDirectory(prefix="native-density-tag-") as tmp:
        base = Path(tmp)
        log = base / "app.log"
        lines = [
            f"solidcraft: assistant (CEF): {record['tag']} {json.dumps(record['data'], sort_keys=True)}"
            for record in _fixture_records()
        ]
        lines.append('solidcraft: assistant (CEF): CEF_STATUS {"status": "PARTIAL"}')
        log.write_text("\n".join(lines) + "\n", encoding="utf-8")
        manifest = {
            "themes": theme_ids_from_source(),
            "artifacts": [{"path": "app.log", "sha256": _sha256_file(log), "size": log.stat().st_size}],
        }
        manifest_path = base / "manifest.json"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        report_path = base / "report.json"
        code = main(["--check-log", str(log), "--manifest", str(manifest_path), "--out", str(report_path)])
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if code != 1 or report.get("status") != "FAIL":
            raise AssertionError(f"an unknown CEF_ tag must FAIL: exit={code} status={report.get('status')}")
        if not any(check["name"] == "receipt_lines" and "CEF_STATUS" in check["detail"] for check in report["checks"]):
            raise AssertionError("the CLI parser must report the unknown CEF_ tag")


def _test_receipts_after_many_plain_lines():
    with tempfile.TemporaryDirectory(prefix="native-density-lines-") as tmp:
        path = Path(tmp) / "app.log"
        records = _fixture_records()
        body = "".join(f"plain log line {index}\n" for index in range(5000))
        body += "".join(
            f"solidcraft: assistant (CEF): {record['tag']} {json.dumps(record['data'], sort_keys=True)}\n"
            for record in records
        )
        path.write_text(body, encoding="utf-8")
        parsed, problems = read_receipts(path)
        if problems:
            raise AssertionError(f"plain lines produced receipt problems: {problems}")
        if len(parsed) != len(records):
            raise AssertionError(f"receipts after 5000 plain lines were dropped: {len(parsed)} != {len(records)}")


def _test_capture_bundle_does_not_trust_supplied_paths():
    with tempfile.TemporaryDirectory(prefix="native-density-bundle-") as tmp:
        root = Path(tmp)
        app = root / "intcad"
        app.write_bytes(b"#!/bin/sh\nexit 0\n")
        cef_root = root / "cef"
        cef_root.mkdir()
        (cef_root / "libcef.so").write_bytes(b"not the mapped library")
        (cef_root / "resources.pak").write_bytes(b"pak")
        report = capture_bundle(app, cef_root, os.getpid())
        if report["status"] == "PASS":
            raise AssertionError("fabricated CEF root must not PASS without a mapped libcef")
        names = {c["name"] for c in report["checks"]}
        for wanted in ("app_identity", "cef_root", "process_starttime", "mapped_libcef"):
            if wanted not in names:
                raise AssertionError(f"missing {wanted} check")
        artifacts = {Path(a["path"]).name: a for a in report["artifacts"]}
        if artifacts.get("intcad", {}).get("sha256") != _sha256_file(app):
            raise AssertionError("app whole-file hash missing")


def _test_pid_starttime_with_parens_in_comm():
    after = ["S", "7"] + ["0"] * 17 + ["98765", "1", "0"]
    line = "4242 (a)b c) " + " ".join(after)
    parsed = parse_stat_line(line)
    if parsed is None or parsed["pid"] != 4242 or parsed["ppid"] != 7 or parsed["starttime"] != 98765:
        raise AssertionError(f"stat parsing did not keep the comm/parent/starttime: {parsed}")


def _test_offline_cli_round_trip():
    with tempfile.TemporaryDirectory(prefix="native-density-cli-") as tmp:
        base = Path(tmp)
        log = base / "app.log"
        records = _fixture_records()
        log.write_text("".join(
            f"solidcraft: assistant (CEF): {record['tag']} {json.dumps(record['data'], sort_keys=True)}\n"
            for record in records
        ), encoding="utf-8")
        manifest = _complete_manifest(base, [{"path": "app.log", "sha256": _sha256_file(log), "size": log.stat().st_size, "role": "app-log"}])
        manifest["expected_scales"] = list(DEFAULT_SCALES)
        manifest_path = base / "manifest.json"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        report_path = base / "report.json"
        code = main(["--check-log", str(log), "--manifest", str(manifest_path), "--out", str(report_path)])
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if code != 0 or report.get("status") != "PASS":
            raise AssertionError(f"offline CLI did not PASS: exit={code} status={report.get('status')} {report.get('checks')}")
        records = _fixture_records((1.0, 1.25, 1.5))
        log.write_text("".join(
            f"CEF_TAG {record['tag']} {json.dumps(record['data'], sort_keys=True)}\n"
            for record in records
        ), encoding="utf-8")
        manifest["artifacts"][0]["sha256"] = _sha256_file(log)
        manifest["artifacts"][0]["size"] = log.stat().st_size
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        code = main(["--check-log", str(log), "--manifest", str(manifest_path), "--out", str(report_path)])
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if code != 1 or report.get("status") != "FAIL":
            raise AssertionError(f"missing scale did not FAIL offline: exit={code} status={report.get('status')}")


def _test_positive_accelerated_receipt_is_gated_on_native_frame_kind():
    source = (Path(__file__).resolve().parents[3] / "apps/solidcraft/src/assistant_cef.rs").read_text(encoding="utf-8")
    at = source.find("emit_accelerated_limit(true")
    if at < 0:
        raise AssertionError("the app no longer emits a positive CEF_LIMIT receipt")
    gate = source[max(0, at - 400):at]
    if "NativeFrameKind::DmaBufImage" not in gate or "native_kind" not in gate:
        raise AssertionError("the positive accelerated receipt is not gated on the frame's real native kind")
    if "emit_accelerated_limit(false" in gate:
        raise AssertionError("the positive receipt sits beside a refusal branch")


def _run_self_tests():
    tests = [
        _test_complete_receipts_pass,
        _test_missing_runtime_identity_fails,
        _test_missing_source_fails,
        _test_import_source_mismatch_fails,
        _test_missing_expected_scale_fails,
        _test_popup_without_admitted_view_fails,
        _test_unknown_or_blocking_status_fails,
        _test_incomplete_theme_set_fails,
        _test_altered_whole_file_hash_with_same_text_hash_fails,
        _test_mismatched_resources_fail,
        _test_aba_same_geometry_reports_limitation_not_pass,
        _test_llvmpipe_or_no_interop_is_blocked_not_pass,
        _test_missing_accelerated_receipt_is_not_pass,
        _test_positive_accelerated_receipt_is_gated_on_native_frame_kind,
        _test_density_physical_tolerance,
        _test_empty_inventory_and_missing_roles_never_pass,
        _test_resource_escape_or_origin_mismatch_fails,
        _test_missing_authenticated_origin_is_blocked,
        _test_supplied_app_must_match_pid_executable,
        _test_missing_pid_fails_instead_of_passing,
        _test_pid_reuse_wrong_generation_and_helper_mismatch_fail,
        _test_task_identity_is_explicit,
        _test_requested_dip_cannot_be_reconstructed_from_capture,
        _test_missing_effective_native_scale_blocks_exact_density,
        _test_same_lifetime_placement_aba_is_detected,
        _test_later_aba_after_unrelated_prefix_is_detected,
        _test_pending_b_transition_is_not_hidden_by_admission_dedup,
        _test_new_capture_counter_or_document_nonce_is_not_causal_proof,
        _test_partial_visible_fails,
        _test_popup_import_mismatch_fails,
        _test_empty_log_with_valid_manifest_fails,
        _test_unknown_tag_through_cli_fails,
        _test_receipts_after_many_plain_lines,
        _test_capture_bundle_does_not_trust_supplied_paths,
        _test_pid_starttime_with_parens_in_comm,
        _test_offline_cli_round_trip,
    ]
    failures = 0
    for test in tests:
        try:
            test()
        except Exception as error:  # self-test harness: report every failure, never abort early
            failures += 1
            print(f"FAIL {test.__name__}: {type(error).__name__}: {error}")
        else:
            print(f"PASS {test.__name__}")
    print(f"self-test: {len(tests) - failures}/{len(tests)} passed, {failures} failed")
    return 0 if failures == 0 else 1


# ── CLI: offline report, live capture ───────────────────────────────────────


class Control:
    """The app's authenticated line control channel, bounded and loopback only."""

    def __init__(self, port, token, timeout=5):
        import socket

        sock = socket.create_connection(("127.0.0.1", int(port)), timeout=timeout)
        sock.settimeout(60)
        self.file = sock.makefile("rw", encoding="utf-8", newline="\n")
        self.token, self.number = token, 0

    def call(self, method, params=None):
        self.number += 1
        request = {"id": self.number, "method": method, "params": params or {}}
        if self.number == 1:
            request["token"] = self.token
        self.file.write(json.dumps(request) + "\n")
        self.file.flush()
        reply = self.file.readline()
        if not reply:
            raise OSError("control channel closed")
        return json.loads(reply)


def _exit_code(report):
    return 0 if report.get("status") == "PASS" else (1 if report.get("status") == "FAIL" else 2)


def _write_report(report, out):
    path = Path(out)
    if path.exists() and path.is_dir():
        path = path / "native-density.json"
    elif path.suffix.lower() != ".json":
        path = path / "native-density.json"
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    except OSError as error:
        print(f"native-density: cannot write report: {error}", file=sys.stderr)
        return None
    return path


def _offline(check_log, manifest_path, out):
    checks, artifacts, limitations = [], [], [
        "offline receipts prove parsing and provenance only; they cannot prove live native import or GUI behavior.",
    ]
    log_path = Path(check_log)
    manifest_file = Path(manifest_path)
    records, log_present = [], False
    if log_path.is_file():
        log_present = True
        records, problems = read_receipts(log_path)
        if problems:
            checks.append(_check("receipt_lines", "FAIL", "; ".join(problems[:5])))
        else:
            checks.append(_check("receipt_lines", "PASS", f"{len(records)} bounded receipt(s)"))
        try:
            artifacts.append({"path": str(log_path.resolve()), "sha256": _sha256_file(log_path), "size": log_path.stat().st_size, "role": "app-log"})
        except OSError as error:
            checks.append(_check("log_identity", "FAIL", f"cannot hash log: {error}"))
    else:
        checks.append(_check("receipt_lines", "BLOCKED", "log is missing"))
    manifest = None
    if manifest_file.is_file():
        try:
            manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
            artifacts.append({"path": str(manifest_file.resolve()), "sha256": _sha256_file(manifest_file), "size": manifest_file.stat().st_size, "role": "manifest"})
        except (OSError, ValueError) as error:
            checks.append(_check("manifest", "FAIL", f"cannot read manifest: {error}"))
    else:
        checks.append(_check("manifest", "BLOCKED", "manifest is missing"))

    scales = DEFAULT_SCALES
    if isinstance(manifest, dict):
        declared = manifest.get("expected_scales")
        if declared is not None:
            parsed = _scale_list(declared)
            if not parsed:
                checks.append(_check("expected_scales", "FAIL", "manifest expected_scales is invalid"))
            else:
                scales = parsed

    # An existing log is always validated, even when it yields no receipts:
    # an empty log is missing evidence, never a vacuous pass. A present manifest
    # is always hash/theme/resource checked, even when the log is absent.
    if log_present:
        receipt_report = validate_receipts(records, scales)
        checks.extend(receipt_report["checks"])
        limitations.extend(receipt_report["limitations"])
    if manifest is not None:
        manifest_report = validate_manifest(manifest, log_path.parent)
        checks.extend(manifest_report["checks"])
        limitations.extend(manifest_report["limitations"])
        for key in ("artifacts", "resources"):
            for entry in manifest.get(key, []) if isinstance(manifest.get(key), list) else []:
                if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
                    continue
                path = Path(entry["path"])
                if not path.is_absolute():
                    path = log_path.parent / path
                if path.is_file():
                    artifacts.append({"path": str(path.resolve()), "sha256": _sha256_file(path), "size": path.stat().st_size, "role": key[:-1]})
    report = _report(checks, artifacts=artifacts, limitations=limitations)
    written = _write_report(report, out)
    print(f"native-density offline: status={report['status']} checks={len(report['checks'])} report={written}")
    return _exit_code(report)


PLATFORMS = ("linux", "windows", "macos")


def task_identity_problem(task, source, platform):
    if not re.fullmatch(r"[A-Za-z0-9._-]{1,40}", str(task)):
        return "task name must be 1-40 safe characters"
    if not isinstance(source, str) or not source or not Path(source).is_dir():
        return "source path is not an existing directory"
    if platform not in PLATFORMS:
        return f"platform must be one of {PLATFORMS}"
    return None


def _live(app, cef_root, out, task, source, platform):
    import subprocess

    checks, artifacts, limitations = [], [], [
        "requested_scale is the app's placement scale, not an independently observed native getter.",
        "UI-scale preference coverage is not compositor or monitor movement.",
        "live mode never uses host OS input or screen capture.",
    ]
    if not re.fullmatch(r"[A-Za-z0-9._-]{1,40}", str(task)):
        checks.append(_check("task_name", "FAIL", "task name must be 1-40 safe characters"))
        report = _report(checks, limitations=limitations)
        _write_report(report, out)
        return _exit_code(report)
    app, cef_root, out = Path(app), Path(cef_root), Path(out)
    themes = theme_ids_from_source()
    checks.append(_check("theme_source", "PASS" if len(themes) == 25 and len(set(themes)) == 25 else "FAIL", f"{len(themes)} built-in themes from source"))
    if not app.is_file():
        checks.append(_check("app_identity", "BLOCKED", "app is missing"))
    if not (cef_root / "libcef.so").is_file():
        checks.append(_check("cef_root", "BLOCKED", "CEF root has no libcef.so"))
    if _aggregate(checks) in ("FAIL", "BLOCKED"):
        report = _report(checks, limitations=limitations)
        _write_report(report, out)
        return _exit_code(report)

    out.mkdir(parents=True, exist_ok=True)
    private = Path(tempfile.mkdtemp(prefix=f"native-density-{task}-", dir=out))
    dirs = {name: private / name.lower() for name in ("SETTINGS", "VALIDATION", "CRASH", "CONTROL", "RECOVERY")}
    for directory in dirs.values():
        directory.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    for name, directory in dirs.items():
        env[f"SOLIDCRAFT_{name}_DIR"] = str(directory)
    env["SOLIDCRAFT_UPDATE_CHECK"] = "0"
    env["NO_AT_BRIDGE"] = "1"
    env["CEF_PATH"] = str(cef_root)
    log_path = private / "app.log"
    log = open(log_path, "wb")
    process = None
    control = None
    scale_records = []
    try:
        process = subprocess.Popen([str(app), "--control", "0"], env=env, stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 120
        port, token = 0, ""
        while time.monotonic() < deadline:
            ports = sorted(dirs["CONTROL"].glob("*.port"))
            if ports:
                try:
                    port = int(ports[0].read_text().strip())
                    token = (dirs["CONTROL"] / f"{port}.token").read_text().strip()
                except (OSError, ValueError):
                    pass
                if port and token:
                    break
            if process.poll() is not None:
                break
            time.sleep(0.3)
        if not port or not token:
            checks.append(_check("control_channel", "BLOCKED", "bounded control-channel startup failed"))
        else:
            control = Control(port, token)
            checks.append(_check("control_channel", "PASS", "private authenticated loopback control channel"))
            opened = control.call("ui.agent_chat", {"open": True})
            checks.append(_check("assistant_open", "PASS" if opened.get("ok") else "FAIL", "assistant panel open request"))
            while time.monotonic() < deadline:
                state = control.call("ui.agent_chat").get("result") or {}
                if (state.get("webView") or {}).get("page"):
                    break
                time.sleep(0.4)
            state = control.call("ui.agent_chat").get("result") or {}
            page = bool((state.get("webView") or {}).get("page"))
            checks.append(_check("page_ready", "PASS" if page else "BLOCKED", "page did not report ready" if not page else "page reported ready"))
            baseline = control.call("ui.prefs", {})
            prefs = baseline.get("result") if isinstance(baseline.get("result"), dict) else {}
            ui_scale = prefs.get("ui_scale")
            if not _finite_number(ui_scale) or float(ui_scale) <= 0:
                ui_scale = 1.0
            rects = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
            ppp = rects.get("pixelsPerPoint")
            base = float(ppp) / float(ui_scale) if _finite_number(ppp) and float(ppp) > 0 else 0.0
            checks.append(_check("native_base_scale", "PASS" if base > 0 else "BLOCKED", f"measured base {base:.4f} from ppp/ui_scale"))
            for target in DEFAULT_SCALES:
                request = target / base if base > 0 else 0.0
                if not (0.5 <= request <= 2.5):
                    scale_records.append({"target": target, "requested_ui_scale": request, "actual_ppp": None, "match": False})
                    checks.append(_check(f"ui_scale_{target:g}", "FAIL", f"derived ui_scale {request:.4f} is outside the supported range"))
                    continue
                control.call("ui.prefs", {"ui_scale": request})
                seen, scale_deadline = None, time.monotonic() + 10
                while time.monotonic() < scale_deadline:
                    measured = control.call("ui.rects", {"prefix": "agent_chat:"}).get("result") or {}
                    seen = measured.get("pixelsPerPoint")
                    if _finite_number(seen) and abs(float(seen) - target) <= 0.02:
                        break
                    time.sleep(0.25)
                match = _finite_number(seen) and abs(float(seen) - target) <= 0.02
                scale_records.append({"target": target, "requested_ui_scale": request, "actual_ppp": seen, "match": match})
                checks.append(_check(f"ui_scale_{target:g}", "PASS" if match else "FAIL", f"ui_scale {request:.4f}, ppp {seen}"))
            time.sleep(2.5)
        bundle = capture_bundle(app, cef_root, process.pid)
        checks.extend(bundle["checks"])
        artifacts.extend(bundle["artifacts"])
        limitations.extend(bundle["limitations"])
        if control is not None:
            try:
                quit_reply = control.call("app.quit", {"force": True})
                checks.append(_check("authenticated_quit", "PASS" if quit_reply.get("ok") else "FAIL", "authenticated quit through the control channel"))
            except OSError as error:
                checks.append(_check("authenticated_quit", "FAIL", f"authenticated quit failed: {error}"))
        if process is not None:
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                checks.append(_check("bounded_shutdown", "FAIL", "app did not exit within 20 s"))
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
    except (OSError, ValueError, KeyError) as error:
        checks.append(_check("live_launch", "BLOCKED", f"{type(error).__name__}: {error}"))
    finally:
        log.close()
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()

    try:
        artifacts.append({"path": str(log_path.resolve()), "sha256": _sha256_file(log_path), "size": log_path.stat().st_size, "role": "app-log"})
        records, problems = read_receipts(log_path)
        if problems:
            checks.append(_check("receipt_lines", "FAIL", "; ".join(problems[:5])))
        else:
            checks.append(_check("receipt_lines", "PASS", f"{len(records)} bounded receipt(s)"))
        receipt_report = validate_receipts(records, DEFAULT_SCALES)
        checks.extend(receipt_report["checks"])
        limitations.extend(receipt_report["limitations"])
    except OSError as error:
        checks.append(_check("receipt_lines", "BLOCKED", f"app log unavailable: {error}"))
    scale_problems = [record for record in scale_records if not record.get("match")]
    if not scale_records:
        checks.append(_check("ui_scale_matrix", "BLOCKED", "no UI-scale requests completed"))
    else:
        checks.append(_check("ui_scale_matrix", "FAIL" if scale_problems else "PASS", f"{len(scale_records) - len(scale_problems)}/{len(scale_records)} requested UI scales measured"))
    report = _report(checks, artifacts=artifacts, limitations=limitations)
    report["task"] = {"task": str(task), "source": source, "platform": platform}
    report["ui_scale_requests"] = scale_records
    written = _write_report(report, out)
    print(f"native-density live: status={report['status']} checks={len(report['checks'])} report={written}")
    return _exit_code(report)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--self-test", action="store_true")
    group.add_argument("--check-log")
    group.add_argument("--app")
    parser.add_argument("--manifest")
    parser.add_argument("--cef-root")
    parser.add_argument("--out")
    parser.add_argument("--task")
    parser.add_argument("--source")
    parser.add_argument("--platform")
    args = parser.parse_args(argv)
    if args.self_test:
        return _run_self_tests()
    if args.check_log:
        if not args.manifest or not args.out:
            parser.error("--check-log requires --manifest and --out")
        return _offline(args.check_log, args.manifest, args.out)
    if args.app:
        if not args.cef_root or not args.out or not args.task or not args.source or not args.platform:
            parser.error("--app requires --cef-root, --out, --task, --source and --platform")
        identity = task_identity_problem(args.task, args.source, args.platform)
        if identity is not None:
            parser.error(identity)
        return _live(args.app, args.cef_root, args.out, args.task, args.source, args.platform)
    parser.error("no mode selected")
    return 2


if __name__ == "__main__":
    sys.exit(main())
