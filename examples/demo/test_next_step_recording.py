"""Test the recorder using local project fixtures and a stub of the control channel."""

import importlib
import json
from pathlib import Path
import socket
import sys
import tempfile
from threading import Thread
import unittest
from unittest.mock import patch


class NextStepRecordingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Importing the recorder must not launch a connection or use user arguments.
        with patch.object(sys, "argv", [sys.argv[0]]):
            cls.demo = importlib.import_module("next_step_demo")

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.run_root = self.root / "one take"
        self.run_root.mkdir()
        self.template = self.root / "template"
        self.template.mkdir()
        (self.template / ".intcad").mkdir()
        (self.template / ".intcad" / "project.toml").write_text('name = "Case"\n')

    def test_project_copy_preserves_existing_destination_and_stays_in_run(self):
        outside = self.root / "existing project"
        outside.mkdir()
        sentinel = outside / "keep"
        sentinel.write_text("original")
        with patch.dict("os.environ", {"PROJECT_NAME": str(outside)}):
            copied = self.demo.stage_project(self.run_root, self.template)
        self.assertEqual(copied, self.run_root / "project")
        self.assertTrue((copied / ".intcad" / "project.toml").is_file())
        marker = copied / "keep"
        marker.write_text("previous take")
        with self.assertRaises(FileExistsError):
            self.demo.stage_project(self.run_root, self.template)
        self.assertEqual(marker.read_text(), "previous take")
        self.assertEqual(sentinel.read_text(), "original")

    def test_project_copy_does_not_follow_a_symlink_destination_or_template_link(self):
        outside = self.root / "outside"
        outside.mkdir()
        target = self.run_root / "project"
        target.symlink_to(outside, target_is_directory=True)
        with self.assertRaises(FileExistsError):
            self.demo.stage_project(self.run_root, self.template)
        target.unlink()
        (self.template / "linked").symlink_to(outside, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.demo.stage_project(self.run_root, self.template)
        self.assertEqual(list(outside.iterdir()), [])

    def test_ready_ghost_is_landed_directly_and_verified_as_one_undo(self):
        stub = ControlStub()
        record = self.demo.record_suggestion(stub.call, stub.wait)
        self.assertEqual(stub.keys, ["Tab"])
        self.assertTrue(stub.prepared)
        self.assertEqual((record["id"], record["undo_added"], record["before_revision"], record["after_revision"]), (7, 1, 24, 25))

    def test_missing_ghost_sends_no_tab_and_wrong_undo_or_chat_is_rejected(self):
        stub = ControlStub(never_ready=True)
        with self.assertRaises(TimeoutError):
            self.demo.record_suggestion(stub.call, stub.wait)
        self.assertEqual(stub.keys, [])
        for wrong in ("undo", "chat", "geometry"):
            with self.subTest(wrong=wrong):
                stub = ControlStub(wrong=wrong)
                with self.assertRaises(RuntimeError):
                    self.demo.record_suggestion(stub.call, stub.wait)

    def test_take_writes_the_rendered_and_landed_evidence_under_its_output(self):
        client, server = socket.socketpair()
        self.addCleanup(client.close)
        self.addCleanup(server.close)
        stub = ControlStub()
        saved = []
        problems = []

        def serve():
            with server, server.makefile("rw") as wire:
                for line in wire:
                    req = json.loads(line)
                    method, params = req["method"], req["params"]
                    try:
                        if req["id"] == 1:
                            self.assertEqual(req["token"], "synthetic-run-token")
                        if method == "ui.home":
                            result = None
                        elif method == "ui.agent_chat" and params:
                            self.assertNotIn("send", params)
                            self.assertNotIn("add_agent", params)
                            self.assertEqual(params.get("agent", "claude"), "claude")
                            result = None
                        elif method == "engine.execute" and params["command"] in ("solid.box", "file.save_as"):
                            if params["command"] == "file.save_as":
                                saved.append(params["params"]["path"])
                            result = None
                        elif method == "ui.agent_chat" and not req.get("token") and not saved:
                            result = {"agents": [{"id": "claude", "found": True}], "state": "ready"}
                        else:
                            result = stub.call(method, params)
                        reply = {"id": req["id"], "ok": True, "result": result}
                    except Exception as e:
                        problems.append(str(e))
                        reply = {"id": req["id"], "ok": False, "error": "unexpected recorder call"}
                    wire.write(json.dumps(reply) + "\n")
                    wire.flush()

        def recorder(argv, **kwargs):
            self.assertIn(f"location={self.run_root / 'raw.mkv'}", argv)
            (self.run_root / "raw.mkv").write_bytes(b"synthetic recording")
            return FinishedRecorder()

        worker = Thread(target=serve, daemon=True)
        worker.start()
        with patch.dict("os.environ", {"PROJECT_TEMPLATE": str(self.template), "DISPLAY": ":71"}):
            with patch.object(self.demo.socket, "create_connection", return_value=client):
                with patch.object(self.demo.subprocess, "Popen", side_effect=recorder):
                    with patch.object(self.demo.time, "sleep", return_value=None):
                        self.demo.record(9491, "synthetic-run-token", self.run_root, "bracket")
        worker.join(timeout=2)
        self.assertFalse(worker.is_alive())
        self.assertEqual(problems, [])
        self.assertEqual(saved, [str(self.run_root / "project" / "recorded-design.sc")])
        events = [json.loads(line) for line in (self.run_root / "events.jsonl").read_text().splitlines()]
        self.assertEqual([row["kind"] for row in events], ["ghost", "accepted"])
        self.assertEqual(events[1]["undo_added"], 1)


class FinishedRecorder:
    returncode = 0

    def poll(self):
        return self.returncode

    def wait(self, timeout):
        return self.returncode


class ControlStub:
    def __init__(self, never_ready=False, wrong=None):
        self.never_ready = never_ready
        self.wrong = wrong
        self.keys = []
        self.prepared = False
        self.polls = 0

    def wait(self, check, what, secs):
        for _ in range(5):
            if value := check():
                return value
        raise TimeoutError(what)

    def call(self, method, params=None):
        accepted = bool(self.keys)
        if method == "ui.agent_chat" and not params:
            self.polls += 1
            on_screen = self.polls >= 3 and not self.never_ready
            self.prepared |= on_screen
            return {
                "state": "busy" if accepted and self.wrong == "chat" else "ready",
                "items": [{"user": "unexpected"}] if accepted and self.wrong == "chat" else [],
                "preview": None,
                "nextStep": {
                    "ghost": None if accepted else {"onScreen": on_screen, "bodies": [{"name": "Case", "volume": 75.0, "faces": 8}]},
                    "log": [{"id": 7, "outcome": "accepted" if accepted else "ghost", "step": "Shell the case"}],
                },
            }
        if method == "ui.key":
            if not self.prepared:
                raise AssertionError("Tab was sent before the ghost was on screen")
            self.keys.append(params["key"])
            return {}
        if method == "document.revision":
            return {"document_id": "disposable-design", "revision": 25 if accepted else 24}
        if method == "document.inspect":
            return {"undo": 4 if not accepted else 6 if self.wrong == "undo" else 5}
        if method == "engine.execute" and params["command"] == "inspect.measure":
            return {"bodies": [{"name": "Case", "volume_mm3": 75.0 if accepted and self.wrong != "geometry" else 100.0,
                                "kernel": {"faces": 8 if accepted and self.wrong != "geometry" else 6}}]}
        raise AssertionError(f"unexpected control call: {method} {params}")
