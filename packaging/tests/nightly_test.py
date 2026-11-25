"""Exercise the real nightly decision script without GitHub or repository mutations."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import unittest
from unittest.mock import Mock, patch


SCRIPT = Path(__file__).resolve().parents[1] / "nightly.sh"
ROOT = SCRIPT.parent.parent


class AssistantNativeSmoke(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("assistant_check", ROOT / "packaging/linux/assistant-check.py")
        cls.smoke = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.smoke)

    def setUp(self):
        self.control, self.native, self.app = Mock(), Mock(), Mock()
        self.app.pid = 456
        self.exit_info = None
        self.native.close_window.side_effect = lambda: setattr(
            self, "exit_info", Mock(si_pid=456, si_code=os.CLD_EXITED, si_status=0)
        )
        waitid = patch.object(self.smoke.os, "waitid", side_effect=lambda *_: self.exit_info)
        waitid.start()
        self.addCleanup(waitid.stop)
        self.native.page.return_value = (100, 200, 420, 700)
        self.native.view_identity.return_value = 314
        self.native.retained_hidden.return_value = True
        self.native.backing_scale = 1
        self.states = []
        self.rects = [
            {"name": "home:back", "x": 10, "y": 2, "width": 20, "height": 18},
            {"name": "titlebar:assistant", "x": 10, "y": 2, "width": 20, "height": 18},
            {"name": "titlebar:assistant", "x": 80, "y": 2, "width": 20, "height": 18},
            {"name": "agent_chat:web", "x": 80, "y": 160, "width": 256, "height": 400},
        ]

        def call(method, deadline, params=None):
            if method == "ui.inspect":
                return {"ok": True, "result": self.states.pop(0) if len(self.states) > 1 else self.states[0]}
            if method == "ui.rects":
                prefix = (params or {}).get("prefix", "")
                return {"ok": True, "result": {"pixelsPerPoint": 1.25, "rects": [
                    rect for rect in self.rects if rect["name"].startswith(prefix)
                ]}}
            self.fail(f"unexpected control call: {method}")

        self.control.call.side_effect = call

    def state(self, open_, ready, host="available"):
        self.states.append({"assistant": {"open": open_, "surface_ready": ready, "host": host}})

    def exercise(self):
        now = 0

        def sleep(seconds):
            nonlocal now
            now += seconds

        with patch.object(self.smoke.time, "monotonic", side_effect=lambda: now), \
             patch.object(self.smoke.time, "sleep", side_effect=sleep):
            self.smoke.exercise(self.app, self.control, self.native, 1, Mock())

    def test_missing_or_invalid_readiness_fails(self):
        for state in ({}, {"open": True, "host": "available"},
                      {"open": True, "host": "available", "surface_ready": "true"},
                      {"open": True, "host": "available", "surface_ready": False}):
            with self.subTest(state=state):
                self.states.clear()
                self.state(False, False)
                self.states.append({"assistant": state})
                with self.assertRaisesRegex(ValueError, "readiness|surface_ready"):
                    self.exercise()
        self.native.close_page.assert_not_called()

    def test_page_close_must_hide_page_without_ending_process(self):
        self.state(False, False)
        self.state(True, True)
        with self.assertRaisesRegex(ValueError, "page Close"):
            self.exercise()
        self.native.close_window.assert_not_called()

    def test_closed_panel_must_retain_its_readiness(self):
        self.state(False, False)
        self.state(True, True)
        self.state(False, False)
        with self.assertRaisesRegex(ValueError, "retained.*readiness"):
            self.exercise()
        self.native.close_window.assert_not_called()

    def test_reopened_page_must_be_the_original_native_view(self):
        self.state(False, False)
        self.state(True, True)
        self.state(False, True)
        self.state(False, True)
        self.state(True, True)

        self.native.view_identity.side_effect = (314, 315)
        with self.assertRaisesRegex(ValueError, "different native view"):
            self.exercise()
        self.native.close_window.assert_not_called()

    def test_readiness_must_survive_until_the_reopen_click(self):
        self.state(False, False)
        self.state(True, True)
        self.state(False, True)
        self.state(False, False)
        with self.assertRaisesRegex(ValueError, "retained.*readiness"):
            self.exercise()
        self.native.click.assert_called_once()

    def test_exit_timeout_cannot_pass(self):
        self.state(False, False)
        self.state(True, True)
        self.state(False, True)
        self.state(False, True)
        self.state(True, True)
        self.native.close_window.side_effect = None
        with self.assertRaisesRegex(ValueError, "clean native exit"):
            self.exercise()
        self.app.wait.assert_not_called()

    def test_observed_open_close_reopen_then_native_exit(self):
        self.state(False, False)
        self.state(True, True)
        self.state(False, True)
        self.state(False, True)
        self.state(True, True)
        self.exercise()
        self.assertEqual(self.native.click.call_args_list[0].args, (112, 13))
        self.assertEqual(self.native.click.call_count, 2)
        self.native.retained_hidden.assert_called_once()
        self.assertEqual(self.native.retained_hidden.call_args.args, (314,))
        self.native.close_window.assert_called_once_with()

    def test_rail_opens_and_reopens_with_or_without_titlebar_in_same_reply(self):
        for titlebar_drawn in (True, False):
            with self.subTest(titlebar_drawn=titlebar_drawn):
                if not titlebar_drawn:
                    self.rects[:] = [r for r in self.rects if r["name"] != "titlebar:assistant"]
                self.rects[:0] = [
                    {"name": "rail:assistant", "x": 320, "y": 32, "width": 20, "height": 18},
                    {"name": "rail:assistant", "x": 320, "y": 40, "width": 20, "height": 18},
                ]
                self.states.clear()
                for opened, ready in ((False, False), (True, True), (False, True), (False, True), (True, True)):
                    self.state(opened, ready)
                self.native.click.reset_mock()
                self.control.call.reset_mock()
                self.exit_info = None
                self.exercise()
                self.assertEqual([call.args for call in self.native.click.call_args_list], [(412, 61)] * 2)
                snapshots = [call.args[2] for call in self.control.call.call_args_list if call.args[0] == "ui.rects"]
                self.assertEqual(snapshots, [None, {"prefix": "agent_chat:web"}] * 2)
                self.rects[:2] = []

    def test_no_assistant_target_does_not_click_prefix_lookalike(self):
        self.rects[:] = [r for r in self.rects if r["name"] != "titlebar:assistant"]
        self.rects.append({"name": "rail:assistant-extra", "x": 320, "y": 40, "width": 20, "height": 18})
        self.state(False, False)
        with self.assertRaisesRegex(ValueError, "no native Assistant entry"):
            self.exercise()
        self.native.click.assert_not_called()

    def test_malformed_drawn_rail_geometry_does_not_fall_back_or_click(self):
        self.rects.append({"name": "rail:assistant", "x": 320, "y": 40, "width": "20", "height": 18})
        self.state(False, False)
        self.state(True, True)
        with self.assertRaisesRegex(ValueError, "invalid rail:assistant rectangle"):
            self.exercise()
        self.native.click.assert_not_called()

    def test_initial_home_is_dismissed_by_native_click(self):
        self.state(False, False)
        self.states[0]["home"] = True
        self.state(False, False)
        self.states[1]["home"] = False
        self.state(True, True)
        self.state(False, True)
        self.state(False, True)
        self.state(True, True)
        self.exercise()
        self.assertEqual([call.args for call in self.native.click.call_args_list], [(25, 13), (112, 13), (112, 13)])

    def test_socket_uses_remaining_deadline_after_each_read(self):
        sock = Mock()
        sock.recv.side_effect = [b" ", TimeoutError("expired")]
        sock.makefile.return_value.readline.side_effect = TimeoutError("expired")
        budget = iter((1.0, 1.1, 1.2, 1.8))
        with patch.object(self.smoke.time, "monotonic", side_effect=lambda: next(budget)), \
             patch.object(self.smoke.socket, "create_connection", return_value=sock):
            with self.assertRaisesRegex(OSError, "expired"):
                self.smoke.Control(1234, "fixture").call("ui.inspect", 2.0)
        self.assertAlmostEqual(sock.settimeout.call_args.args[0], 0.2)
        sock.close.assert_called_once()

    def test_missing_isolated_display_is_blocked_before_launch(self):
        popen = subprocess.Popen

        def launch_only_worker(command, **kwargs):
            self.assertEqual(command[2], "--x11-worker")
            return popen(command, **kwargs)

        with tempfile.TemporaryDirectory() as evidence, \
             patch.object(self.smoke.sys, "argv", ["assistant-check", "--exe", "/not-launched", "--evidence", evidence]), \
             patch.dict(os.environ, {"DISPLAY": "remote:0"}, clear=True), \
             patch.object(self.smoke.subprocess, "Popen", side_effect=launch_only_worker) as launch:
            self.assertEqual(self.smoke.main(), 2)
            launch.assert_called_once()

    def test_x11_close_targets_only_the_verified_page_child(self):
        self.assertTrue(hasattr(self.smoke, "X11"), "no native X11 driver")
        desktop = object.__new__(self.smoke.X11)
        desktop.page_window = 314
        desktop.guarded_click = Mock()
        desktop.close_page((700, 220, 1020, 820), 1.0)
        desktop.guarded_click.assert_called_once_with((294, 24), 314, page_bounds=(700, 220, 1020, 820))

    def test_x11_visible_child_must_match_client_relative_geometry(self):
        desktop = object.__new__(self.smoke.X11)
        desktop.owner = 1
        desktop.owner_bounds = Mock(return_value=(20, 30, 1600, 1000))
        desktop.dpy, desktop.root = 123, 0
        desktop.grab, desktop.ungrab, desktop.sync, desktop.verify_hit = Mock(), Mock(), Mock(), Mock()
        desktop.atom = Mock(return_value=12)
        desktop.children = Mock(return_value=(5, 6))
        desktop.property = Mock(side_effect=lambda w, *_: (12,) if w == 6 else ())
        desktop.geometry = Mock(side_effect=lambda w: (0, 0, 1600, 1000) if w == 0 else (120, 230, 440, 730))
        self.assertEqual(desktop.page((100, 200, 420, 700)), (120, 230, 440, 730))
        desktop.verify_hit.assert_called_once_with(280, 480, 6)
        self.assertIsNone(desktop.page((100, 200, 420, 695)))

    def test_mapped_retained_page_cannot_be_clipped_or_covered(self):
        desktop = object.__new__(self.smoke.X11)
        desktop.dpy, desktop.root, desktop.owner, desktop.page_window = 123, 0, 1, 6
        desktop.grab, desktop.ungrab, desktop.sync = Mock(), Mock(), Mock()
        desktop.owner_bounds = Mock(return_value=(20, 30, 1600, 1000))
        desktop.atom = Mock(return_value=12)
        desktop.children = Mock(return_value=(6,))
        desktop.property = Mock(return_value=(12,))
        page = (120, 230, 440, 730)
        desktop.verify_hit = Mock()
        for screen in ((0, 0, 1600, 729), (0, 0, 1600, 1000)):
            with self.subTest(screen=screen):
                desktop.geometry = Mock(side_effect=lambda w: screen if w == 0 else page)
                desktop.owner_bounds.return_value = (20, 30, 1600, 1000)
                if screen[3] == 1000:
                    desktop.verify_hit.side_effect = ValueError("covered")
                self.assertIsNone(desktop.page((100, 200, 420, 700)))
        desktop.verify_hit.assert_called_once_with(280, 480, 6)

    def test_startup_retries_a_transient_unopened_control_listener(self):
        self.assertTrue(hasattr(self.smoke, "connect_control"), "no bounded startup listener retry")
        self.control.call.side_effect = [ConnectionRefusedError(), {"ok": True, "result": {}}]
        with patch.object(self.smoke.time, "monotonic", side_effect=iter(i / 1000 for i in range(30))), \
             patch.object(self.smoke.time, "sleep"):
            self.smoke.connect_control(self.app, self.control, 0.03)
        self.assertEqual([call.args[0] for call in self.control.call.call_args_list], ["ui.inspect", "ui.inspect"])

    def test_unsupported_platforms_block_before_any_desktop_probe(self):
        for platform, mode in (("darwin", "page"), ("linux", "diagnostic")):
            with self.subTest(platform=platform), \
                 patch.object(self.smoke.sys, "platform", platform), \
                 patch.object(self.smoke.C, "CDLL") as load:
                with self.assertRaisesRegex(self.smoke.Blocked, "physical|native Wayland"):
                    self.smoke.native_driver(mode)
                load.assert_not_called()

    def test_frame_hit_rejects_any_covering_descendant(self):
        desktop = object.__new__(self.smoke.X11)
        desktop.root, desktop.owner = 1, 3
        desktop.owner_bounds = Mock(return_value=(0, 0, 500, 500))
        desktop.geometry = Mock(return_value=(0, 0, 500, 500))
        desktop.at = Mock(side_effect=lambda _root, w, *_: (0, 0, {1: 2, 2: 3, 3: 4, 4: 0}[w]))
        desktop.motion = Mock()
        desktop.button = Mock()
        desktop.sync = Mock()
        desktop.grab, desktop.ungrab = Mock(), Mock()
        desktop.dpy = 123
        with self.assertRaisesRegex(ValueError, "hit"):
            desktop.guarded_click((40, 50), 3)
        desktop.motion.assert_not_called()
        desktop.ungrab.assert_called_once_with(123)

    def test_page_hit_rejects_covering_sibling_but_allows_live_page_descendant(self):
        desktop = object.__new__(self.smoke.X11)
        desktop.root, desktop.owner, desktop.page_window = 1, 2, 3
        children = {1: 2, 2: 4, 3: 5, 4: 0, 5: 0}
        desktop.at = Mock(side_effect=lambda _root, w, *_: (0, 0, children[w]))
        with self.assertRaisesRegex(ValueError, "hit"):
            desktop.verify_hit(40, 50, 3)
        children[2] = 3
        desktop.verify_hit(40, 50, 3)

    def test_destroy_notify_invalidates_reused_xid(self):
        desktop = object.__new__(self.smoke.X11)
        desktop.watched_page = desktop.page_window = 314
        desktop.dpy = 123
        desktop.sync = Mock()
        desktop.check_event = Mock(return_value=True)
        self.assertIsNone(desktop.view_identity())

    def test_cleanup_does_not_signal_group_after_original_leader_is_reaped(self):
        self.app.pid = 456
        with patch.object(self.smoke.os, "killpg") as kill_group, \
             patch.object(self.smoke.os, "waitid", side_effect=ChildProcessError):
            with self.assertRaisesRegex((OSError, ValueError), "unreaped|identity"):
                self.smoke.cleanup_app(self.app)
        kill_group.assert_not_called()
        self.app.wait.assert_not_called()

    def test_cleanup_signals_verified_group_before_reaping_exited_leader(self):
        self.app.pid = 456
        self.app.poll.return_value = 0  # This Popen method must never be called.
        order = Mock()
        order.attach_mock(self.app.wait, "reap")
        with patch.object(self.smoke.os, "waitid", return_value=Mock(si_pid=456)) as peek, \
             patch.object(self.smoke.os, "getsid", return_value=456), \
             patch.object(self.smoke.os, "getpgid", return_value=456), \
             patch.object(self.smoke.os, "killpg", order.signal):
            self.smoke.cleanup_app(self.app)
        peek.assert_called_once_with(os.P_PID, 456, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        self.app.poll.assert_not_called()
        self.assertEqual([call[0] for call in order.mock_calls], ["signal", "reap"])

    def test_cleanup_refuses_group_without_session_ownership(self):
        with patch.object(self.smoke.os, "getsid", return_value=451), \
             patch.object(self.smoke.os, "killpg") as kill_group:
            with self.assertRaisesRegex(ValueError, "session and group identity"):
                self.smoke.cleanup_app(self.app)
        kill_group.assert_not_called()
        self.app.wait.assert_not_called()

    def test_hit_recomputes_frame_origin_after_grab_and_refreshes_a_moved_page(self):
        desktop = object.__new__(self.smoke.X11)
        desktop.dpy, desktop.root, desktop.owner, desktop.page_window = 123, 1, 2, 3
        state = {"grabbed": False}
        desktop.grab = Mock(side_effect=lambda _: state.update(grabbed=True))
        desktop.ungrab = Mock(side_effect=lambda _: state.update(grabbed=False))
        desktop.owner_bounds = Mock(side_effect=lambda: (100, 100, 900, 900) if state["grabbed"] else (50, 50, 850, 850))
        desktop.geometry = Mock(side_effect=lambda w: {1: (0, 0, 1200, 900), 3: (450, 230, 770, 800)}[w])
        desktop.children = Mock(return_value=(3,))
        desktop.page_destroyed = Mock(return_value=False)
        desktop.watched_page = 3
        desktop.verify_hit, desktop.motion, desktop.button, desktop.sync = Mock(), Mock(), Mock(), Mock()
        desktop.click(40, 50)
        desktop.motion.assert_called_once_with(123, 0, 140, 150, 0)
        desktop.motion.reset_mock()
        # The page moved since the observation: the Close hit is re-derived from the live
        # child and only then sent; the stale coordinates are never clicked.
        desktop.close_page((440, 220, 760, 790), 1)
        desktop.motion.assert_called_once_with(123, 0, 744, 254, 0)

    def test_frame_coordinates_outside_live_screen_never_send_input(self):
        desktop = object.__new__(self.smoke.X11)
        desktop.dpy, desktop.root, desktop.owner = 123, 1, 2
        desktop.grab, desktop.ungrab, desktop.sync = Mock(), Mock(), Mock()
        desktop.owner_bounds = Mock(return_value=(100, 100, 900, 900))
        desktop.geometry = Mock(return_value=(0, 0, 1200, 800))
        desktop.verify_hit, desktop.motion, desktop.button = Mock(), Mock(), Mock()
        with self.assertRaisesRegex(ValueError, "screen"):
            desktop.click(100, 720)
        desktop.motion.assert_not_called()

    def test_linux_and_legacy_macos_installed_smokes_use_separate_helpers(self):
        self.assertIn("python3 /packaging/linux/assistant-check.py", (ROOT / "packaging/linux/installed-check-inside.sh").read_text())
        self.assertIn('"$ROOT/packaging/assistant-check.py"', (ROOT / "packaging/macos/installed-check.sh").read_text())

    def test_bad_or_nonexec_prefix_is_rejected_before_resources(self):
        for prefix in ("'unterminated", "sh -c 'intcad & wait'", "sandbox-exec -f profile", ""):
            with self.subTest(prefix=prefix), \
                 patch.object(self.smoke.sys, "argv", ["assistant-check", "--exe", "/fixture", "--run-with", prefix]), \
                 patch.object(self.smoke, "native_driver") as native, \
                 patch.object(self.smoke.Path, "mkdir") as mkdir, \
                 patch.object(self.smoke.subprocess, "Popen") as launch, contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(self.smoke.main(), 2)
                native.assert_not_called()
                mkdir.assert_not_called()
                launch.assert_not_called()

    def test_native_roundtrip_timeout_kills_only_driver_worker(self):
        worker = object.__new__(self.smoke.BoundedX11)
        worker.conn, worker.proc = Mock(), Mock()
        worker.conn.send.side_effect = len
        worker.proc.wait.side_effect = subprocess.TimeoutExpired("worker", 2)
        worker.stopped = False
        with patch.object(self.smoke, "receive_packet", side_effect=TimeoutError):
            with self.assertRaisesRegex(ValueError, "X11.*unreaped"):
                worker.request("page", (1, 2, 3, 4), timeout=0.5)
        worker.close()  # No second reap attempt or automatic join.
        worker.proc.kill.assert_called_once_with()
        worker.proc.wait.assert_called_once_with(timeout=2)

    def test_capture_reports_unreaped_and_does_not_wait_without_deadline(self):
        capture = Mock()
        capture.wait.side_effect = [subprocess.TimeoutExpired("capture", 15), subprocess.TimeoutExpired("capture", 2)]
        with tempfile.TemporaryDirectory() as work, \
             patch.object(self.smoke.shutil, "which", return_value="/usr/bin/import"), \
             patch.object(self.smoke.subprocess, "Popen", return_value=capture):
            self.native.headless_wayland = False
            with self.assertRaisesRegex(ValueError, "capture.*unreaped"):
                self.smoke.screen_picture(Path(work) / "screen.png", self.native)
        capture.kill.assert_called_once_with()
        self.assertEqual([c.kwargs for c in capture.wait.call_args_list], [{"timeout": 15}, {"timeout": 2}])

    def test_evidence_setup_and_worker_cleanup_failures_are_both_reported(self):
        native = Mock()
        native.close.side_effect = ValueError("native X11 worker child 123 unreaped")
        with tempfile.TemporaryDirectory() as evidence, \
             patch.object(self.smoke.sys, "argv", ["assistant-check", "--exe", "/fixture", "--evidence", evidence]), \
             patch.object(self.smoke, "native_driver", return_value=native), \
             patch.object(self.smoke.Path, "mkdir", side_effect=OSError("evidence storage unavailable")), \
             patch.object(self.smoke.subprocess, "Popen") as launch, \
             contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(self.smoke.main(), 1)
        self.assertEqual(output.getvalue().splitlines(), [
            "FAIL: could not set up evidence: evidence storage unavailable",
            "FAIL: could not close native driver: native X11 worker child 123 unreaped",
        ])
        native.close.assert_called_once_with()
        launch.assert_not_called()

    def test_unreaped_worker_failure_still_cleans_up_verified_app_group(self):
        native = Mock()
        order = Mock()
        native.close.side_effect = ValueError("native X11 worker child 123 unreaped after SIGKILL deadline")
        order.attach_mock(native.close, "close_worker")
        order.attach_mock(self.app.wait, "reap_app")
        with tempfile.TemporaryDirectory() as evidence, \
             patch.object(self.smoke.sys, "argv", ["assistant-check", "--exe", "/fixture", "--evidence", evidence]), \
             patch.object(self.smoke, "native_driver", return_value=native), \
             patch.object(self.smoke, "connect_control"), \
             patch.object(self.smoke, "exercise", side_effect=ValueError("native screen capture timed out")), \
             patch.object(self.smoke, "missing_libraries", return_value=[]), \
             patch.object(self.smoke.subprocess, "Popen", return_value=self.app), \
             patch.object(self.smoke.os, "getsid", return_value=456), \
             patch.object(self.smoke.os, "getpgid", return_value=456), \
             patch.object(self.smoke.os, "killpg", order.kill_group), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(self.smoke.main(), 1)
        self.assertIn("unreaped after SIGKILL deadline", output.getvalue())
        self.assertEqual([c[0] for c in order.mock_calls], ["kill_group", "reap_app", "close_worker"])
        order.kill_group.assert_called_once_with(456, self.smoke.signal.SIGKILL)

    def test_missing_native_capture_tool_is_blocked_not_skipped(self):
        with patch.object(self.smoke.sys, "platform", "linux"), \
             patch.object(self.smoke.shutil, "which", return_value=None):
            self.native.headless_wayland = False
            with self.assertRaisesRegex(self.smoke.Blocked, "capture"):
                self.smoke.screen_picture(Path("unused"), self.native)

    def test_linux_launch_owns_session_and_cleans_up_after_capability_block(self):
        native = Mock()
        native.find_window.return_value = True
        self.app.pid = 456
        with tempfile.TemporaryDirectory() as evidence, \
             patch.object(self.smoke.sys, "argv", ["assistant-check", "--exe", "/fixture", "--evidence", evidence]), \
             patch.object(self.smoke, "native_driver", return_value=native), \
             patch.object(self.smoke, "connect_control"), \
             patch.object(self.smoke, "exercise", side_effect=self.smoke.Blocked("fixture capability")), \
             patch.object(self.smoke.subprocess, "Popen", return_value=self.app) as launch, \
             patch.object(self.smoke.os, "getsid", return_value=456), \
             patch.object(self.smoke.os, "getpgid", return_value=456), \
             patch.object(self.smoke.os, "killpg") as kill_group, \
             contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(self.smoke.main(), 2)
        self.assertIn("BLOCKED: fixture capability", output.getvalue())
        self.assertEqual(launch.call_args.args[0], ["/fixture", "--control", "17979"])
        self.assertEqual(launch.call_args.kwargs["env"]["SOLIDCRAFT_UPDATE_CHECK"], "0")
        self.assertTrue(launch.call_args.kwargs.get("start_new_session"))
        kill_group.assert_called_once_with(456, self.smoke.signal.SIGKILL)
        self.app.wait.assert_called_once_with(timeout=5)


def release(tag, published, *, draft=False, prerelease=False):
    return {"tag_name": tag, "published_at": published, "draft": draft,
            "prerelease": prerelease, "target_commitish": "main"}


class NightlyDecision(unittest.TestCase):
    def run_decision(self, pages, refs=None, *, event="schedule", sha="current", api_fail=False):
        self.assertIsNotNone(shutil.which("jq"), "install jq to run packaging checks")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "output"
            summary = root / "summary"
            calls = root / "calls"
            output.touch()
            summary.touch()
            for command, body in {
                "gh": '''import json, os, sys
args = sys.argv[1:]
with open(os.environ["CALLS"], "a") as log: log.write(json.dumps(args) + "\\n")
if args != ["api", "--paginate", "--slurp", "repos/test/project/releases?per_page=100"]:
    sys.exit("unexpected gh invocation (slurp and jq must not be combined)")
if os.environ["API_FAIL"] == "1": sys.exit("mock API failure")
print(os.environ["PAGES"])
''',
                "git": '''import json, os, sys
args = sys.argv[1:]
with open(os.environ["CALLS"], "a") as log: log.write(json.dumps(args) + "\\n")
refs = json.loads(os.environ["REFS"])
if len(args) != 3 or args[:2] != ["rev-parse", "--verify"] or args[2] not in refs:
    sys.exit("missing tag")
print(refs[args[2]])
''',
            }.items():
                path = root / command
                path.write_text("#!/usr/bin/env python3\n" + body, encoding="utf-8")
                path.chmod(0o755)
            env = {"PATH": str(root) + os.pathsep + os.environ["PATH"],
                   "GITHUB_EVENT_NAME": event, "GITHUB_REPOSITORY": "test/project",
                   "GITHUB_SHA": sha, "GITHUB_OUTPUT": str(output),
                   "GITHUB_STEP_SUMMARY": str(summary), "CALLS": str(calls),
                   "PAGES": json.dumps(pages), "REFS": json.dumps(refs or {}),
                   "API_FAIL": "1" if api_fail else "0"}
            result = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True, check=False)
            return result, output.read_text(), summary.read_text(), calls.read_text() if calls.exists() else ""

    def test_newest_published_prerelease_across_pages_skips_exact_tag_commit(self):
        pages = [[release("v1", "2026-01-01T00:00:00Z")],
                 [release("v2-nightly", "2026-01-03T00:00:00Z", prerelease=True),
                  release("v3-draft", "2026-01-04T00:00:00Z", draft=True)]]
        result, output, summary, calls = self.run_decision(pages, {"refs/tags/v2-nightly^{commit}": "current"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "build=false\n")
        self.assertIn("v2-nightly", summary)
        self.assertIn("refs/tags/v2-nightly^{commit}", calls)
        self.assertNotIn("v3-draft^{commit}", calls)

    def test_changed_commit_builds(self):
        result, output, _, _ = self.run_decision([[release("v1", "2026-01-01T00:00:00Z")]],
                                                {"refs/tags/v1^{commit}": "previous"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "build=true\n")

    def test_no_published_release_builds(self):
        for pages in [[], [[]], [[release("draft", None, draft=True)]]]:
            with self.subTest(pages=pages):
                result, output, _, calls = self.run_decision(pages)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(output, "build=true\n")
                self.assertNotIn("rev-parse", calls)

    def test_non_schedule_does_not_query_github(self):
        for event in ["push", "workflow_dispatch"]:
            with self.subTest(event=event):
                result, output, _, calls = self.run_decision([], event=event, api_fail=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(output, "build=true\n")
                self.assertEqual(calls, "")

    def test_api_failure_does_not_report_build_or_skip(self):
        result, output, _, _ = self.run_decision([], api_fail=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(output, "")

    def test_missing_tag_fails_instead_of_using_moving_target_commitish(self):
        result, output, _, _ = self.run_decision([[release("missing", "2026-01-01T00:00:00Z")]])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(output, "")


class ForkWorkflowSafety(unittest.TestCase):
    """Catch executable publication paths, not the presence of old helper tests."""

    WORKFLOWS = ("release", "release-gate", "docker", "pages", "flatpak-pages", "snap-store", "winget")

    def workflow(self, name):
        return (ROOT / ".github/workflows" / (name + ".yml")).read_text()

    def test_no_event_or_manual_input_can_reactivate_a_publisher(self):
        for name in self.WORKFLOWS:
            with self.subTest(workflow=name):
                text = self.workflow(name)
                events = text.split("\non:\n", 1)[1].split("\npermissions:", 1)[0]
                self.assertNotRegex(events, r"(?m)^  (release|schedule):")
                if name == "release-gate":
                    # A tag-only push is the formal post-promotion gate. Any branch
                    # push, or a push with branches, would turn merges into a full
                    # matrix and must stay out.
                    push = re.search(r"(?ms)^  push:\n(.*?)(?=^  \w)", events)
                    self.assertIsNotNone(push, "the post-promotion tag trigger is missing")
                    self.assertRegex(push[1], r'(?m)^    tags: \["v\*"\]$')
                    self.assertNotRegex(push[1], r"(?m)^    branches:")
                else:
                    self.assertNotRegex(events, r"(?m)^  push:")
                self.assertNotRegex(events, r"(?m)^      (draft|signpath_signing_policy|packages_only):")
                self.assertNotRegex(text, r"gh release (create|edit|upload|delete)|snapcraft (upload|promote)|--submit\b|gh workflow run")

    def test_builds_never_receive_signing_or_publication_credentials(self):
        for name in self.WORKFLOWS:
            with self.subTest(workflow=name):
                text = self.workflow(name)
                self.assertNotIn("secrets.", text)
                self.assertNotRegex(text, r"(?m)^\s+[\w-]+: write\s*$")
                self.assertNotRegex(text, r"signpath/github-action|docker/login-action|actions/deploy-pages|signing.sh|--sign\b")
                self.assertNotRegex(text, r"(?m)^\s+push: true\s*$")

    def test_disabled_publishers_are_separate_from_build_jobs(self):
        for name, job in (("docker", "publish"), ("pages", "deploy"),
                          ("flatpak-pages", "publish"), ("snap-store", "promote"), ("winget", "submit")):
            with self.subTest(workflow=name):
                block = re.search(r"(?ms)^  " + job + r":\n(.*?)(?=^  [\w-]+:|\Z)", self.workflow(name))
                self.assertIsNotNone(block, "retain an explicit quarantined publisher job")
                self.assertRegex(block[1], r"(?m)^    if: \$\{\{ false && github.event_name == 'workflow_dispatch' \}\}$")
                gate = self.workflow("release-gate")
                self.assertNotRegex(gate, r"needs:.*\b" + re.escape(name) + r"\b")

    def test_package_failures_cannot_be_allowed_or_nightly_filtered(self):
        text = self.workflow("release")
        # No step may be allowed to fail, with one owner-approved exception: the installed-app UI
        # check informs the release report and does not block it (2026-11-25). It must stay the
        # only advisory step, and it must be advisory in exactly one place, so a package build,
        # content check, ldd run or account check cannot quietly become advisory too.
        self.assertEqual(text.count("continue-on-error"), 1)
        advisory = re.search(
            r"(?ms)^      - name: Install the packages on bare systems and open the Assistant\n"
            r"(.*?)(?=^      - )", text)
        self.assertIsNotNone(advisory, "the installed-app UI check step is missing")
        self.assertRegex(advisory[1], r"(?m)^        continue-on-error: true$")
        self.assertNotIn("packaging/nightly.sh", text)
        self.assertNotIn("needs.version.outputs.build", text)
        self.assertNotRegex(text, r"(?m)^  release:")
        # Keep actual platform packages and safe, nonpublishing checks.
        for job in ("linux", "flatpak", "arch", "snap", "freebsd", "macos-build", "macos", "windows", "web"):
            self.assertRegex(text, r"(?m)^  " + job + r":")
        self.assertIn("packaging/docker/smoke-test.sh", self.workflow("docker"))
        self.assertIn("cargo xtask web-offline-test --site dist/web", self.workflow("pages"))

    def test_package_smokes_do_not_start_network_enabled_desktop(self):
        text = self.workflow("release")
        # The Snap desktop smoke is allowed only inside a private network namespace with loopback:
        # the package job must never start a network-enabled desktop.
        self.assertIn("sudo unshare -n", text)
        self.assertRegex(text, r"unshare -n[^\n]*snap-smoke\.py")
        self.assertNotRegex(text, r"(?m)^\s*python3 packaging/linux/snap-smoke\.py")
        self.assertIn("ip link set lo up", text)
        self.assertIn("intcad.cli --version", text)
        # Flatpak's existing sandbox denies network; keep its real desktop smoke.
        manifest = (ROOT / "packaging/linux/flatpak/ai.intgral.intcad.bundle.yml").read_text()
        self.assertNotRegex(manifest, r"(?m)^\s+- --share=network\s*$")
        self.assertIn("packaging/linux/flatpak-bundle.sh", text)
        # The installed checks start the desktop app: each takes the network away from it.
        check = (ROOT / "packaging/assistant-check.py").read_text()
        self.assertIn('env["SOLIDCRAFT_UPDATE_CHECK"] = "0"', check)
        linux = (ROOT / "packaging/linux/installed-check.sh").read_text()
        # Two containers: one installs (with the network), one starts the app (without it).
        self.assertEqual(re.findall(r"(?ms)^docker run .*?installed-check-inside\.sh (\w+)", linux), ["install", "run"])
        self.assertEqual(len(re.findall(r"(?m)^docker run ", linux)), 2)
        self.assertIn("installed-check-inside.sh run", linux[linux.index("docker run --rm --network none"):])
        self.assertIn("this system reaches the network", (ROOT / "packaging/linux/installed-check-inside.sh").read_text())
        # The macOS step is the installed-check script, which copies the app out of the image,
        # proves the no-network profile blocks curl, and runs the Assistant check under it.
        self.assertIn("packaging/macos/installed-check.sh", text)
        mac = (ROOT / "packaging/macos/installed-check.sh").read_text()
        self.assertIn('SANDBOX="$ROOT/packaging/macos/no-network.sb"', mac)
        self.assertIn('sandbox-exec -f "$SANDBOX" /usr/bin/curl', mac)
        self.assertIn('--run-with "$prefix"', mac)
        self.assertIn("(deny network*)", (ROOT / "packaging/macos/no-network.sb").read_text())
        windows = (ROOT / "packaging/windows/installed-check.ps1").read_text()
        self.assertLess(windows.index("New-NetFirewallRule"), windows.index("windows-assistant-smoke.ps1') -Exe"))
        self.assertIn("$env:SOLIDCRAFT_UPDATE_CHECK = '0'", windows)
        self.assertNotRegex(text, r"windows-assistant-smoke\.ps1")

    def test_build_only_tokens_and_fork_integration_lints_are_read_only(self):
        for name in ("packaging-lint", "lists"):
            text = self.workflow(name)
            self.assertRegex(text, r"branches: \[main, develop\]")
            self.assertNotRegex(text, r"(?m)^\s+[\w-]+: write\s*$")
        self.assertIn("scripts/ci/test_gate.py", self.workflow("packaging-lint"))


if __name__ == "__main__":
    unittest.main()
