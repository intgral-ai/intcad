"""Fast failure-path checks for the desktop Flatpak and snap process probes."""
import hashlib
import importlib.util
import itertools
import json
import os
import struct
import subprocess
import tempfile
import textwrap
import zlib
from pathlib import Path
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("flatpak_smoke", ROOT / "packaging/linux/flatpak-smoke.py")
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)
SNAP_SPEC = importlib.util.spec_from_file_location("snap_smoke", ROOT / "packaging/linux/snap-smoke.py")
SNAP = importlib.util.module_from_spec(SNAP_SPEC)
SNAP_SPEC.loader.exec_module(SNAP)


def healthy_replies():
    return [
        {"id": 1, "ok": True, "result": {"renderer": "gpu", "viewport": [250, 126, 1000, 700]}},
        {"id": 2, "ok": True, "result": {"feature": 1, "bodies": ["Body1"]}},
        {"id": 3, "ok": True, "result": {
            "timeline": [{"error": None}],
            "bodies": [{"volume_mm3": 20 * 30 * 10, "bbox": {"min": [0, 0, 0], "max": [30, 20, 10]}}],
        }},
    ]


def output(replies):
    return "\n".join(json.dumps(reply) for reply in replies)


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(data, zlib.crc32(kind)))


def rgba_png(width, height, rows):
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


class Control:
    """A control connection whose first frame can arrive after the port opens."""
    def __init__(self, viewports, page=True):
        self.viewports = iter(viewports)
        self.page = page
        self.requests = []
        self.settimeout = Mock()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        pass

    def makefile(self, mode):
        return self

    def write(self, data):
        self.requests.append(json.loads(data))

    def flush(self):
        pass

    def readline(self):
        request = self.requests[-1]
        index = request["id"] - 1
        reply = healthy_replies()[index] if index < 3 else {"id": request["id"], "ok": True, "result": {}}
        if index == 0:
            reply["result"]["viewport"] = next(self.viewports)
        if request["method"] == "ui.home":
            reply["result"] = {"open": False}
        elif request["method"] == "ui.agent_chat":
            reply["result"] = {"open": True, "webView": {"host": True, "page": self.page}}
        return json.dumps(reply) + "\n"


class SnapStartup(unittest.TestCase):
    def setUp(self):
        self.app = Mock()
        self.app.poll.return_value = None
        self.frame = Mock()
        self.frame.read_bytes.return_value = rgba_png(2, 2, b"\x00" + bytes([255, 0, 0, 255, 0, 255, 0, 255])
                                                      + b"\x00" + bytes([0, 0, 255, 255, 255, 255, 255, 255]))

    def drive(self, control, times=None):
        with patch.object(SNAP.socket, "create_connection", return_value=control), \
                patch.object(SNAP.time, "sleep"), \
                patch.object(SNAP.time, "monotonic", side_effect=times, return_value=0):
            return SNAP.drive(self.app, 18839, self.frame)

    def test_control_opens_before_first_frame(self):
        control = Control([None, [250, 126, 1000, 700]])
        renderer, statistics = self.drive(control)
        self.assertEqual(renderer, "gpu")
        self.assertEqual(statistics["nonblack"], 4)
        self.assertEqual([request["method"] for request in control.requests], [
            "ui.inspect", "ui.inspect", "engine.execute", "document.inspect", "ui.home",
            "ui.agent_chat", "ui.agent_chat", "ui.screenshot", "app.quit"
        ])

    def test_ready_startup_does_not_wait(self):
        control = Control([[250, 126, 1000, 700]])
        self.drive(control)
        self.assertEqual(sum(r["method"] == "ui.inspect" for r in control.requests), 1)

    def test_assistant_must_load_its_page(self):
        control = Control([[250, 126, 1000, 700]], page=False)
        with self.assertRaisesRegex(RuntimeError, "page"):
            self.drive(control, [0, 0, 0, 0, 200])
        self.assertIn("ui.agent_chat", [r["method"] for r in control.requests])

    def test_missing_first_frame_times_out_before_modelling(self):
        control = Control(itertools.repeat(None))
        with self.assertRaisesRegex(RuntimeError, "viewport"):
            self.drive(control, [0, 0, 0, 61])
        self.assertTrue(all(r["method"] == "ui.inspect" for r in control.requests))

    def test_app_exit_while_waiting_for_frame_fails_before_modelling(self):
        control = Control(itertools.repeat(None))
        self.app.poll.side_effect = [None, None, 7]
        self.app.returncode = 7
        with self.assertRaisesRegex(RuntimeError, "exited"):
            self.drive(control)
        self.assertTrue(all(r["method"] == "ui.inspect" for r in control.requests))

    def test_failed_control_response_does_not_retry_or_model(self):
        control = Control([None])
        control.readline = Mock(return_value=json.dumps({"id": 1, "ok": False, "error": "startup failure"}) + "\n")
        with self.assertRaisesRegex(ValueError, "ui.inspect"):
            self.drive(control)
        self.assertEqual([r["method"] for r in control.requests], ["ui.inspect"])

    def test_read_timeout_uses_remaining_readiness_deadline_then_restores_operation_timeout(self):
        control = Control([None, [250, 126, 1000, 700]])
        self.drive(control, [0, 0, 20, 50, 50, 50])
        self.assertEqual([call.args[0] for call in control.settimeout.call_args_list], [40, 10, 60])

    def test_closed_or_malformed_control_fails_before_modelling(self):
        for response, error in [("", RuntimeError), ("not json\n", ValueError),
                                (json.dumps({"id": 9, "ok": True}) + "\n", ValueError)]:
            with self.subTest(response=response):
                control = Control([None])
                control.readline = Mock(return_value=response)
                with self.assertRaises(error):
                    self.drive(control)
                self.assertEqual([r["method"] for r in control.requests], ["ui.inspect"])


class DesktopProbe(unittest.TestCase):
    def test_real_presented_frame_is_required_not_just_responsive_control(self):
        black = rgba_png(2, 2, b"\x00" + bytes([0, 0, 0, 255]) * 2 + b"\x00" + bytes([0, 0, 0, 255]) * 2)
        with self.assertRaisesRegex(ValueError, "blank presented frame"):
            SMOKE.frame_statistics(black)
        # Independently encoded two rows: red/green, blue/white.
        colour = rgba_png(2, 2, b"\x00" + bytes([255, 0, 0, 255, 0, 255, 0, 255])
                          + b"\x00" + bytes([0, 0, 255, 255, 255, 255, 255, 255]))
        self.assertEqual(SMOKE.frame_statistics(colour), {"width": 2, "height": 2, "nonblack": 4, "pixels": 4})
        # Same pixels encoded with PNG Sub filtering (not a second decoder).
        sub = rgba_png(2, 2, b"\x01" + bytes([255, 0, 0, 255, 1, 255, 0, 0])
                       + b"\x01" + bytes([0, 0, 255, 255, 255, 255, 0, 0]))
        self.assertEqual(SMOKE.frame_statistics(sub), SMOKE.frame_statistics(colour))

    def test_frame_byte_dimensions_and_decompression_are_bounded(self):
        with self.assertRaisesRegex(ValueError, "dimensions"):
            SMOKE.frame_statistics(rgba_png(SMOKE.MAX_PIXELS + 1, 1, b""))
        with self.assertRaisesRegex(ValueError, "decompressed size"):
            SMOKE.frame_statistics(rgba_png(1, 1, b"\x00" * 10000))
        truncated = rgba_png(1, 1, b"\x00\xff\x00\x00\xff")[:-1]
        with self.assertRaises(ValueError):
            SMOKE.frame_statistics(truncated)

    def test_ui_thread_and_independent_box_measurement_are_required(self):
        replies = healthy_replies()
        self.assertEqual(SMOKE.validate_replies(output(replies)), "gpu")
        replies[2]["result"]["bodies"][0]["volume_mm3"] = 5900
        with self.assertRaises(ValueError):
            SMOKE.validate_replies(output(replies))

    def test_cli_version_or_partial_control_output_is_not_desktop_success(self):
        with self.assertRaises(ValueError):
            SMOKE.validate_replies("intcad-cli 0.3.0\n")
        with self.assertRaises(ValueError):
            SMOKE.validate_replies(output(healthy_replies()[:1]))

    def test_startup_without_viewport_or_failed_feature_is_not_success(self):
        for failure in ["viewport", "feature", "control", "bounds"]:
            with self.subTest(failure=failure):
                replies = healthy_replies()
                if failure == "viewport":
                    replies[0]["result"]["viewport"][2] = 0
                elif failure == "feature":
                    replies[2]["result"]["timeline"][0]["error"] = "operation failed"
                elif failure == "control":
                    replies[1] = {"id": 2, "ok": False, "error": "startup failure"}
                else:
                    replies[2]["result"]["bodies"][0]["bbox"]["max"][0] = 29
                with self.assertRaises(ValueError):
                    SMOKE.validate_replies(output(replies))

    def test_flatpak_assistant_needs_an_open_panel_and_a_loaded_page(self):
        good = {"ok": True, "result": {"open": True, "webView": {"host": True, "page": True}}}
        self.assertEqual(SMOKE.validate_assistant([good], "x11"), "page loaded")
        for state in [
            {"ok": False, "error": "no Assistant"},
            {"ok": True, "result": {"open": False, "webView": {"host": True, "page": False}}},
            {"ok": True, "result": {"open": True, "webView": {"host": False, "page": False}}},
            {"ok": True, "result": {"open": True, "webView": {"host": True, "page": False}}},
            {"ok": True, "result": {"open": True}},
        ]:
            with self.subTest(state=state), self.assertRaises(ValueError):
                SMOKE.validate_assistant([state], "x11")

    def test_native_wayland_diagnostic_is_not_a_page_load(self):
        diagnostic = {"ok": True, "result": {"open": True,
                                             "webView": {"host": False, "page": False},
                                             "webProblem": "native-wayland"}}
        self.assertEqual(SMOKE.validate_assistant([diagnostic], "wayland"),
                         "native-Wayland diagnostic")
        for backend in ("x11", "xwayland"):
            with self.assertRaises(ValueError):
                SMOKE.validate_assistant([diagnostic], backend)
        failed = {"ok": True, "result": {"open": True,
                                         "webView": {"host": False, "page": False},
                                         "webProblem": "failed"}}
        with self.assertRaises(ValueError):
            SMOKE.validate_assistant([failed], "wayland")

    def test_xwayland_requires_the_page_and_native_wayland_rejects_one(self):
        page = {"ok": True, "result": {"open": True,
                                       "webView": {"host": True, "page": True},
                                       "webProblem": None}}
        self.assertEqual(SMOKE.validate_assistant([page], "xwayland"), "page loaded")
        with self.assertRaises(ValueError):
            SMOKE.validate_assistant([page], "wayland")


class FlatpakRepositorySource(unittest.TestCase):
    """The Intcad Flatpak repository must not rebase or import upstream app ids."""

    def test_no_foreign_end_of_life_rebase(self):
        script = (ROOT / "packaging/flatpak-site.sh").read_text()
        self.assertNotIn("--end-of-life-rebase", script)
        self.assertNotIn("legacy_ids", script)
        self.assertNotIn("ai.storyteller", script)
        # Intcad-native publication remains: its own id, bundle import, repo update and remote file.
        self.assertIn("app_id=ai.intgral.intcad", script)
        self.assertIn("flatpak build-import-bundle", script)
        self.assertIn("flatpak build-update-repo", script)
        self.assertIn("solidcraft.flatpakrepo", script)
        docs = (ROOT / "docs/compatibility.md").read_text()
        self.assertNotIn("end-of-life-rebase=<old id>=ai.intgral.intcad", docs)
        self.assertNotIn("Flatpak moves `~/.var/app/<old id>`", docs)

    def test_manifest_and_docs_have_no_foreign_rebase_claim(self):
        manifest = (ROOT / "packaging/linux/flatpak/ai.intgral.intcad.bundle.yml").read_text()
        self.assertNotIn("--filesystem=~/.var/app/ai.storyteller", manifest)
        self.assertNotIn("end-of-life", manifest)
        self.assertNotIn("moves old installs", manifest)
        self.assertIn("id: ai.intgral.intcad", manifest)
        self.assertIn("command: intcad", manifest)
        docs = (ROOT / "docs/compatibility.md").read_text()
        self.assertNotIn("Flatpak rebase list", docs)
        self.assertNotIn("Existing installs are moved over", docs)

    def test_previous_repo_foreign_refs_are_pruned(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            site = root / "site"
            (site / "app").mkdir(parents=True)
            (site / "index.html").write_text("index")
            (site / "app" / "index.html").write_text("app")
            fixtures = root / "fixtures"
            fixtures.mkdir()
            sums = []
            for arch in ("x86_64", "aarch64"):
                asset = f"intcad-1.0.0-linux-{arch}.flatpak"
                data = f"bundle-{arch}".encode()
                (fixtures / asset).write_bytes(data)
                sums.append(f"{hashlib.sha256(data).hexdigest()}  {asset}")
            (fixtures / "SHA256SUMS.txt").write_text("\n".join(sums) + "\n")
            mockbin = root / "bin"
            mockbin.mkdir()
            state = root / "state"
            state.mkdir()
            mocks = {
                "gh": """#!/usr/bin/env bash
set -euo pipefail
if [ "$1" = api ]; then echo "${RELEASE_TAG:?}"; exit 0; fi
if [ "$1 $2" = "release download" ]; then
  pattern=; dir=
  while [ $# -gt 0 ]; do
    case "$1" in
      --pattern) pattern=$2; shift 2 ;;
      --dir) dir=$2; shift 2 ;;
      *) shift ;;
    esac
  done
  cp "${MOCK_FIXTURES:?}/$pattern" "$dir/$pattern"
  exit 0
fi
echo "unexpected gh: $*" >&2; exit 1
""",
                "curl": """#!/usr/bin/env bash
exit 0
""",
                "ostree": """#!/usr/bin/env bash
set -euo pipefail
echo "ostree $*" >> "${MOCK_STATE:?}/debug.log"
repo=; args=()
for a in "$@"; do
  case "$a" in
    --repo=*) repo=${a#--repo=} ;;
    *) args+=("$a") ;;
  esac
done
refs="${MOCK_STATE:?}/refs-$(printf '%s' "$repo" | tr '/' '_')"
case "${args[0]:-}" in
  init) mkdir -p "$repo"; : > "$refs"; touch "$repo/summary" ;;
  remote) ;;
  pull) cat >> "$refs" <<'REFS'
app/ai.storyteller.solidcraft/x86_64/stable
app/ai.storyteller.solidcraft/aarch64/stable
app/io.tangencylabs.solidcraft/x86_64/stable
app/ai.intgral.intcad/x86_64/beta
REFS
    ;;
  refs)
    if [ "${args[1]:-}" = "--delete" ]; then
      ref=${args[2]:?}
      grep -Fvx "$ref" "$refs" > "$refs.tmp" || true
      mv "$refs.tmp" "$refs"
    else
      cat "$refs" 2>/dev/null || true
    fi
    ;;
  rev-parse|show) ;;
  *) echo "unexpected ostree: $*" >&2; exit 1 ;;
esac
""",
                "flatpak": """#!/usr/bin/env bash
set -euo pipefail
echo "flatpak $*" >> "${MOCK_STATE:?}/debug.log"
cmd=${1:?}; repo=${2:?}
refs="${MOCK_STATE:?}/refs-$(printf '%s' "$repo" | tr '/' '_')"
case "$cmd" in
  build-import-bundle)
    asset=${3:?}
    case "$asset" in
      *x86_64*) arch=x86_64 ;;
      *aarch64*) arch=aarch64 ;;
      *) echo "unknown asset: $asset" >&2; exit 1 ;;
    esac
    echo "app/ai.intgral.intcad/$arch/stable" >> "$refs"
    ;;
  build-update-repo)
    for last in "$@"; do :; done
    echo "update last=$last" >> "${MOCK_STATE:?}/debug.log"
    echo summary > "$last/summary"
    ;;
  build-commit-from) echo "unexpected build-commit-from" >&2; exit 1 ;;
  *) echo "unexpected flatpak: $*" >&2; exit 1 ;;
esac
""",
            }
            for name, text in mocks.items():
                path = mockbin / name
                path.write_text(textwrap.dedent(text))
                path.chmod(0o755)
            env = os.environ.copy()
            env.update({
                "PATH": f"{mockbin}:{env['PATH']}",
                "RELEASE_TAG": "v1.0.0",
                "GITHUB_REPOSITORY": "owner/repo",
                "PAGES_BASE_URL": "https://example.test/project",
                "MOCK_STATE": str(state),
                "MOCK_FIXTURES": str(fixtures),
            })
            result = subprocess.run(
                ["bash", str(ROOT / "packaging/flatpak-site.sh"), str(site)],
                cwd=ROOT, env=env, text=True, capture_output=True,
            )
            debug = (state / "debug.log").read_text() if (state / "debug.log").exists() else ""
            listing = subprocess.run(["find", str(site), "-maxdepth", "4", "-type", "f"], text=True, capture_output=True).stdout
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr + debug + listing)
            refs = []
            for path in state.glob("refs-*"):
                refs.extend(path.read_text().splitlines())
            self.assertTrue(refs, "mock ostree saw no refs")
            for ref in refs:
                self.assertTrue(ref.startswith("app/ai.intgral.intcad/"), ref)
            self.assertIn("app/ai.intgral.intcad/x86_64/beta", refs, "legitimate Intcad branch lost")
            self.assertIn("app/ai.intgral.intcad/x86_64/stable", refs)
            self.assertIn("app/ai.intgral.intcad/aarch64/stable", refs)


if __name__ == "__main__":
    unittest.main()
