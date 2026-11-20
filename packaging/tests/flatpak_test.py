"""Fast failure-path checks for the desktop Flatpak and snap process probes."""
import hashlib
import importlib.util
import itertools
import json
import os
import re
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
        guide = (ROOT / "docs/book/src/guide/getting-started.md").read_text()
        self.assertNotIn("your settings and autosaves move with it", guide)
        self.assertNotIn("it copies your settings", guide)
        self.assertIn("does not adopt", guide)
        self.assertIn("remove the old app manually", guide)
        self.assertNotIn("../../../compatibility.md", guide, "mdBook only builds docs/book/src")
        targets = re.findall(r"\[compatibility notes\]\(([^)]+)\)", guide)
        self.assertTrue(
            any(target.startswith("https://github.com/intgral-ai/intcad/blob/")
                and target.endswith("/docs/compatibility.md#linux-app-id") for target in targets),
            targets,
        )
        self.assertIn("does not currently publish", guide)

    @staticmethod
    def _run_publication(seed_refs, fail=None):
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
            seed = root / "seed.txt"
            seed.write_text(seed_refs)
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
  pull) cat "${MOCK_SEED:?}" >> "$refs" ;;
  refs)
    if [ "${MOCK_FAIL:-}" = refs ]; then exit 1; fi
    if [ "${args[1]:-}" = "--delete" ]; then
      prefix=${args[2]:?}
      if [ "${MOCK_FAIL:-}" = delete ]; then exit 1; fi
      awk -v p="$prefix" 'index($0,p)!=1' "$refs" > "$refs.tmp"
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
    for last in "$@"; do repo=$last; done
    refs="${MOCK_STATE:?}/refs-$(printf '%s' "$repo" | tr '/' '_')"
    # Model Flatpak: generated AppStream branches are rebuilt from the app refs
    # currently in the repository, never inherited from the mirrored repo.
    grep -v -E '^appstream2?/' "$refs" > "$refs.tmp" || true
    mv "$refs.tmp" "$refs"
    app_refs="$(grep '^app/' "$refs" || true)"
    : > "${MOCK_STATE:?}/appstream-sources.log"
    while IFS= read -r app_ref; do
      [ -n "$app_ref" ] || continue
      printf '%s\n' "$app_ref" >> "$MOCK_STATE/appstream-sources.log"
    done <<< "$app_refs"
    archs="$(printf '%s\n' "$app_refs" | awk -F/ 'NF >= 3 { print $3 }' | sort -u)"
    while IFS= read -r arch; do
      [ -n "$arch" ] || continue
      echo "appstream/$arch" >> "$refs"
      echo "appstream2/$arch" >> "$refs"
    done <<< "$archs"
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
                "MOCK_SEED": str(seed),
                "MOCK_FAIL": fail or "",
            })
            result = subprocess.run(
                ["bash", str(ROOT / "packaging/flatpak-site.sh"), str(site)],
                cwd=ROOT, env=env, text=True, capture_output=True,
            )
            refs = []
            for path in state.glob("refs-*"):
                refs.extend(path.read_text().splitlines())
            debug = (state / "debug.log").read_text() if (state / "debug.log").exists() else ""
            sources_path = state / "appstream-sources.log"
            sources = sources_path.read_text() if sources_path.exists() else ""
            return result, refs, debug, sources

    def test_previous_repo_foreign_refs_are_pruned(self):
        seed = "\n".join([
            "app/ai.intgral.intcad/x86_64/stable",
            "app/ai.intgral.intcad/x86_64/master",
            "app/ai.intgral.intcad/x86_64/beta",
            "app/ai.intgral.intcad/aarch64/stable",
            "app/ai.intgral.intcad/aarch64/master",
            "app/ai.intgral.intcad/aarch64/beta",
            "app/ai.storyteller.solidcraft/x86_64/stable",
            "app/ai.storyteller.solidcraft/x86_64/stable-extra",
            "app/ai.storyteller.solidcraft/aarch64/stable-extra",
            "app/io.tangencylabs.solidcraft/x86_64/stable",
            "app/ai.intgral.intcad2/x86_64/stable",
        ])
        result, refs, debug, _ = self._run_publication(seed)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr + debug)
        self.assertTrue(refs, "mock ostree saw no refs")
        for ref in refs:
            self.assertTrue(
                ref.startswith("app/ai.intgral.intcad/")
                or ref.startswith("appstream/")
                or ref.startswith("appstream2/"),
                ref,
            )
        for arch in ("x86_64", "aarch64"):
            self.assertIn(f"appstream/{arch}", refs)
            self.assertIn(f"appstream2/{arch}", refs)
        for ref in (
            "app/ai.intgral.intcad/x86_64/stable",
            "app/ai.intgral.intcad/x86_64/master",
            "app/ai.intgral.intcad/x86_64/beta",
            "app/ai.intgral.intcad/aarch64/stable",
            "app/ai.intgral.intcad/aarch64/master",
            "app/ai.intgral.intcad/aarch64/beta",
        ):
            self.assertIn(ref, refs)
        self.assertFalse(any("storyteller" in ref or "tangencylabs" in ref or "intcad2" in ref for ref in refs))

    def test_exact_app_id_prefix_fails_closed(self):
        seed = "\n".join([
            "app/ai.intgral.intcad",
            "app/ai.intgral.intcad/x86_64/beta",
            "app/ai.storyteller.solidcraft/x86_64/stable",
        ])
        result, _, debug, _ = self._run_publication(seed)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("prefix of Intcad ref", result.stderr)
        self.assertNotIn("flatpak build-update-repo", debug)

    def test_hostile_leading_dash_ref_fails_closed(self):
        seed = "\n".join(["-evil", "app/ai.intgral.intcad/x86_64/beta"])
        result, _, debug, _ = self._run_publication(seed)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue("unexpected ref" in result.stderr or "unsafe ref" in result.stderr, result.stderr)
        self.assertNotIn("flatpak build-update-repo", debug)

    def test_delete_failure_exits_nonzero(self):
        seed = "\n".join([
            "app/ai.intgral.intcad/x86_64/beta",
            "app/ai.storyteller.solidcraft/x86_64/stable",
        ])
        result, _, debug, _ = self._run_publication(seed, fail="delete")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("flatpak build-update-repo", debug)

    def test_refs_enumeration_failure_exits_nonzero(self):
        seed = "\n".join([
            "app/ai.intgral.intcad/x86_64/beta",
            "app/ai.storyteller.solidcraft/x86_64/stable",
        ])
        result, _, debug, _ = self._run_publication(seed, fail="refs")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("flatpak build-update-repo", debug)

    def test_second_publication_regenerates_appstream_from_retained_refs(self):
        initial = "\n".join([
            "app/ai.intgral.intcad/x86_64/stable",
            "app/ai.intgral.intcad/x86_64/master",
            "app/ai.intgral.intcad/x86_64/beta",
            "app/ai.intgral.intcad/aarch64/stable",
            "app/ai.intgral.intcad/aarch64/master",
            "app/ai.intgral.intcad/aarch64/beta",
            "app/ai.storyteller.solidcraft/x86_64/stable",
            "app/ai.intgral.intcad2/aarch64/stable",
        ])
        first, first_refs, first_debug, first_sources = self._run_publication(initial)
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr + first_debug)
        for arch in ("x86_64", "aarch64"):
            self.assertIn(f"appstream/{arch}", first_refs)
            self.assertIn(f"appstream2/{arch}", first_refs)
        self.assertTrue(first_sources.strip(), "first publication did not generate AppStream")
        self.assertTrue(all(ref.startswith("app/ai.intgral.intcad/")
                            for ref in first_sources.splitlines()))

        second_seed = "\n".join(first_refs + ["appstream/riscv64", "appstream2/riscv64"])
        second, refs, debug, sources = self._run_publication(second_seed)
        self.assertEqual(second.returncode, 0, second.stdout + second.stderr + debug)
        self.assertIn("flatpak build-import-bundle", debug)
        for stale in ("appstream/riscv64", "appstream2/riscv64"):
            self.assertIn(f"refs --delete {stale}", debug)
            self.assertLess(debug.index(f"refs --delete {stale}"),
                            debug.index("flatpak build-update-repo"))
        self.assertLess(debug.index("flatpak build-import-bundle"),
                        debug.index("flatpak build-update-repo"))
        for arch in ("x86_64", "aarch64"):
            for branch in ("stable", "master", "beta"):
                self.assertIn(f"app/ai.intgral.intcad/{arch}/{branch}", refs)
            self.assertIn(f"appstream/{arch}", refs)
            self.assertIn(f"appstream2/{arch}", refs)
        self.assertNotIn("appstream/riscv64", refs)
        self.assertNotIn("appstream2/riscv64", refs)
        self.assertFalse(any("storyteller" in ref or "tangencylabs" in ref or "intcad2" in ref
                             for ref in refs))
        self.assertEqual(set(sources.splitlines()), {
            f"app/ai.intgral.intcad/{arch}/{branch}"
            for arch in ("x86_64", "aarch64")
            for branch in ("stable", "master", "beta")
        })

    def test_unexpected_metadata_ref_shape_fails_closed(self):
        seed = "\n".join([
            "app/ai.intgral.intcad/x86_64/stable",
            "appstream/x86_64/extra",
        ])
        result, _, debug, _ = self._run_publication(seed)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected ref", result.stderr)
        self.assertNotIn("flatpak build-update-repo", debug)

if __name__ == "__main__":
    unittest.main()
