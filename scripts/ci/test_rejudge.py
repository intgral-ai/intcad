#!/usr/bin/env python3
"""Self-test for rejudge.py. Run: python3 -I scripts/ci/test_rejudge.py."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rejudge  # noqa: E402

SHA = "a" * 40


class Rejudge(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "selection").mkdir()
        (self.root / "selection/plan.json").write_text(
            json.dumps({"version": 1, "sourceRevision": SHA, "jobs": {}}), encoding="utf-8")
        (self.root / "evidence/job").mkdir(parents=True)
        (self.root / "evidence/job/receipt.json").write_text("{}", encoding="utf-8")
        (self.root / "matrix").mkdir()
        for shard in range(1, 17):
            (self.root / f"matrix/results-{shard}.json").write_text("[]", encoding="utf-8")
        (self.root / "properties/target/nextest/ci").mkdir(parents=True)
        (self.root / "properties/target/nextest/ci/junit.xml").write_text(
            "<testsuites/>", encoding="utf-8")
        (self.root / "jobs.json").write_text('{"jobs": []}', encoding="utf-8")
        (self.root / "exceptions.json").write_text("{}", encoding="utf-8")
        (self.root / "baseline.json").write_text("{}", encoding="utf-8")
        self.args = rejudge.parser().parse_args([
            "record", "--root", str(self.root),
            "--exceptions", str(self.root / "exceptions.json"),
            "--baseline", str(self.root / "baseline.json"),
            "--source-sha", SHA, "--run-id", "1", "--run-attempt", "1",
            "--repository", "intgral-ai/open-skyfall-desktop",
            "--workflow", "Release gate", "--event", "pull_request",
            "--out", str(self.root / "execution.json"),
        ])
        rejudge.record(self.args)

    def check_args(self, *extra):
        return rejudge.parser().parse_args([
            "check", "--root", str(self.root),
            "--exceptions", str(self.root / "exceptions.json"),
            "--baseline", str(self.root / "baseline.json"),
            "--execution", str(self.root / "execution.json"), *extra,
        ])

    def evaluate_args(self, *extra):
        return rejudge.parser().parse_args([
            "evaluate", "--root", str(self.root),
            "--exceptions", str(self.root / "exceptions.json"),
            "--baseline", str(self.root / "baseline.json"),
            "--execution", str(self.root / "execution.json"),
            "--out", str(self.root / "rejudge.json"), *extra,
        ])

    def test_recorded_execution_is_reusable(self):
        manifest, changed = rejudge.check(self.root, self.root / "execution.json", self.check_args())
        self.assertEqual(manifest["sourceRevision"], SHA)
        self.assertFalse(changed)

    def test_rules_change_is_reported_but_execution_inputs_still_match(self):
        (self.root / "exceptions.json").write_text('{"rules": 2}', encoding="utf-8")
        _, changed = rejudge.check(self.root, self.root / "execution.json", self.check_args())
        self.assertTrue(changed)

    def test_rules_change_report_keeps_old_execution_identity_and_formal_gate_false(self):
        (self.root / "exceptions.json").write_text('{"rules": 2}', encoding="utf-8")
        with mock.patch.object(rejudge.beta_gate, "evaluate",
                               return_value={"verdict": "accepted-beta", "problems": []}):
            code = rejudge.evaluate(self.evaluate_args("--candidate-sha", SHA))
        self.assertEqual(code, 0)
        report = json.loads((self.root / "rejudge.json").read_text(encoding="utf-8"))
        self.assertFalse(report["formalGate"])
        self.assertEqual(report["execution"]["sourceRevision"], SHA)
        self.assertEqual(report["verdict"], "accepted-beta")
        self.assertTrue(report["rules"]["changedSinceExecution"])
        self.assertNotEqual(report["rules"]["digest"], report["rules"]["currentDigest"])
        self.assertTrue(report["reuse"]["rulesOnly"])
        self.assertIn("action matrix geometry", report["notReexecuted"])
        self.assertTrue(report["stillRequiredForCandidate"])

    def test_a_rules_change_cannot_accept_a_missing_shard(self):
        (self.root / "exceptions.json").write_text('{"rules": 2}', encoding="utf-8")
        (self.root / "matrix/results-7.json").unlink()
        with mock.patch.object(rejudge.beta_gate, "evaluate") as gate:
            with self.assertRaisesRegex(rejudge.RejudgeError, "matrix"):
                rejudge.evaluate(self.evaluate_args())
            gate.assert_not_called()

    def test_changed_matrix_is_refused(self):
        (self.root / "matrix/results-3.json").write_text('[{"action": "x"}]', encoding="utf-8")
        with self.assertRaisesRegex(rejudge.RejudgeError, "matrixDigest changed"):
            rejudge.check(self.root, self.root / "execution.json", self.check_args())

    def test_missing_shard_is_refused_before_the_gate(self):
        (self.root / "matrix/results-7.json").unlink()
        with self.assertRaisesRegex(rejudge.RejudgeError, "matrix"):
            rejudge.check(self.root, self.root / "execution.json", self.check_args())

    def test_candidate_sha_must_match_execution(self):
        with self.assertRaisesRegex(rejudge.RejudgeError, "candidate SHA differs"):
            rejudge.check(self.root, self.root / "execution.json",
                          self.check_args("--candidate-sha", "b" * 40))

    def test_evaluate_refuses_another_candidate_before_the_gate(self):
        args = rejudge.parser().parse_args([
            "evaluate", "--root", str(self.root),
            "--exceptions", str(self.root / "exceptions.json"),
            "--baseline", str(self.root / "baseline.json"),
            "--execution", str(self.root / "execution.json"),
            "--candidate-sha", "b" * 40,
        ])
        with self.assertRaisesRegex(rejudge.RejudgeError, "candidate SHA differs"):
            rejudge.evaluate(args)

    def test_changed_decision_code_is_refused(self):
        data = json.loads((self.root / "execution.json").read_text(encoding="utf-8"))
        data["gateScriptSha256"] = "0" * 64
        (self.root / "execution.json").write_text(json.dumps(data), encoding="utf-8")
        with self.assertRaisesRegex(rejudge.RejudgeError, "beta_gate.py changed"):
            rejudge.check(self.root, self.root / "execution.json", self.check_args())

    def test_manifest_source_must_match_the_plan(self):
        data = json.loads((self.root / "execution.json").read_text(encoding="utf-8"))
        data["sourceRevision"] = "b" * 40
        (self.root / "execution.json").write_text(json.dumps(data), encoding="utf-8")
        with self.assertRaisesRegex(rejudge.RejudgeError, "plan sourceRevision"):
            rejudge.check(self.root, self.root / "execution.json", self.check_args())

    def test_missing_execution_manifest_is_not_reusable(self):
        with self.assertRaisesRegex(rejudge.RejudgeError, "execution:"):
            rejudge.check(self.root, self.root / "missing.json", self.check_args())

    def test_record_requires_a_real_run_identity(self):
        args = rejudge.parser().parse_args([
            "record", "--root", str(self.root),
            "--exceptions", str(self.root / "exceptions.json"),
            "--baseline", str(self.root / "baseline.json"),
            "--source-sha", SHA, "--run-id", "0", "--run-attempt", "1",
            "--repository", "repo", "--workflow", "w", "--event", "e",
            "--out", str(self.root / "other.json"),
        ])
        with self.assertRaisesRegex(rejudge.RejudgeError, "positive"):
            rejudge.record(args)


if __name__ == "__main__":
    unittest.main(verbosity=2)
