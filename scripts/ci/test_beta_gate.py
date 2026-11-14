#!/usr/bin/env python3
"""Self-test for beta_gate.py. Run: python3 -I scripts/ci/test_beta_gate.py."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import beta_gate  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
SHELL = "SMLib shell: operation: SMLib failed (status 1001)"
FILLET = "SMLib fillet: operation: SMLib's result is not a closed solid"
ENTRY = dict(design="demo/a", entity="Body1/F1", action="face.shell", kind="error",
             category="shell_backend_1001", issue="#88", message=SHELL)
ACTIONS = [
    dict(design="demo/a", entity="Body1/F1", action="face.shell", kind="error", message=SHELL, ok=False),
    dict(design="demo/a", entity="Body1/F2", action="face.shell", kind="error", message=SHELL, ok=True),
    dict(design="demo/b", entity="Body1/F1", action="edge.fillet", kind="error", message=FILLET, ok=True),
]


def manifest(entries=None, total=3):
    return dict(format=1, about="test", source=dict(run=1, sha="a" * 40, totalActions=total, baseline="baseline.json"),
                entries=entries if entries is not None else [ENTRY])


def baseline(extra=()):
    designs = {
        "demo/a": {"Body1/F1": "face.shell face.extrude", "Body1/F2": "face.shell"},
        "demo/b": {"Body1/F1": "edge.fillet"},
    }
    for design, entity, action in extra:
        designs.setdefault(design, {})[entity] = action
    return dict(about="test", designs=designs)


def write_matrix(root, actions=None):
    actions = actions if actions is not None else ACTIONS
    for shard in range(1, 17):
        (root / f"results-{shard}.json").write_text(json.dumps(actions if shard == 1 else []))
    return root


class Matrix(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.matrix = self.root / "matrix"
        self.matrix.mkdir()
        self.exceptions = self.root / "tests/action_matrix/exceptions.json"
        self.exceptions.parent.mkdir(parents=True)
        self.exceptions.write_text(json.dumps(manifest()))
        (self.exceptions.parent / "baseline.json").write_text(json.dumps(baseline()))
        (self.root / "baseline.json").write_text(json.dumps(baseline()))
        self.entry = ENTRY
        self.entry["category"] = beta_gate.category(ENTRY["action"], ENTRY["message"])

    def test_valid_matrix(self):
        result = beta_gate.evaluate_matrix(write_matrix(self.matrix), self.exceptions)
        self.assertEqual((result["actions"], result["failures"], result["excused"]), (3, 1, 1))
        self.assertEqual(result["problems"], [])

    def test_missing_shard_is_not_a_pass(self):
        write_matrix(self.matrix)
        (self.matrix / "results-7.json").unlink()
        with self.assertRaisesRegex(beta_gate.BetaGateError, "missing shards 7"):
            beta_gate.evaluate_matrix(self.matrix, self.exceptions)

    def test_new_failure_is_unexcused(self):
        self.exceptions.write_text(json.dumps(manifest(total=4)))
        (self.root / "baseline.json").write_text(json.dumps(baseline(extra=(("demo/c", "Body1/F1", "face.shell"),))))
        actions = ACTIONS + [dict(design="demo/c", entity="Body1/F1", action="face.shell", kind="error", message=SHELL, ok=False)]
        result = beta_gate.evaluate_matrix(write_matrix(self.matrix, actions), self.exceptions)
        self.assertTrue(result["problems"])
        self.assertIn("unexcused action failures", result["problems"][0])

    def test_exception_that_passes_must_be_removed(self):
        actions = [dict(ACTIONS[0], ok=True), ACTIONS[1], ACTIONS[2]]
        result = beta_gate.evaluate_matrix(write_matrix(self.matrix, actions), self.exceptions)
        self.assertTrue(result["problems"])
        self.assertIn("now pass", result["problems"][0])

    def test_a_rules_change_rejudges_the_same_matrix_without_a_rerun(self):
        matrix = write_matrix(self.matrix)
        other = dict(ENTRY, design="demo/other")
        self.exceptions.write_text(json.dumps(manifest(entries=[other])))
        old = beta_gate.evaluate_matrix(matrix, self.exceptions)
        self.assertTrue(old["problems"])
        self.assertIn("unexcused action failures", old["problems"][0])
        self.exceptions.write_text(json.dumps(manifest()))
        new = beta_gate.evaluate_matrix(matrix, self.exceptions)
        self.assertEqual(new["problems"], [])
        self.assertEqual((new["excused"], new["unexcused"]), (1, 0))

    def test_changed_class_or_kind_is_not_excused(self):
        actions = [dict(ACTIONS[0], message="not supported yet: SMLib shell for this geometry"),
                   ACTIONS[1], ACTIONS[2]]
        result = beta_gate.evaluate_matrix(write_matrix(self.matrix, actions), self.exceptions)
        self.assertTrue(result["problems"])
        self.assertIn("changed failure class", result["problems"][0])
        actions = [dict(ACTIONS[0], kind="bad_params"), ACTIONS[1], ACTIONS[2]]
        result = beta_gate.evaluate_matrix(write_matrix(self.matrix, actions), self.exceptions)
        self.assertIn("changed failure class", result["problems"][0])

    def test_total_action_count_must_match_the_reviewed_run(self):
        self.exceptions.write_text(json.dumps(manifest(total=4)))
        with self.assertRaisesRegex(beta_gate.BetaGateError, "at least 4"):
            beta_gate.evaluate_matrix(write_matrix(self.matrix), self.exceptions)

    def test_unbaselined_failures_are_itemised_and_a_new_one_is_rejected(self):
        key = ("demo/b", "Body1/F9", "face.shell")
        entry = dict(design=key[0], entity=key[1], action=key[2], kind="error",
                     category=beta_gate.category(key[2], SHELL), issue="#88")
        value = manifest(total=4)
        value["unbaselined"] = [entry]
        self.exceptions.write_text(json.dumps(value))
        actions = ACTIONS + [dict(design=key[0], entity=key[1], action=key[2], kind="error", message=SHELL, ok=False)]
        result = beta_gate.evaluate_matrix(write_matrix(self.matrix, actions), self.exceptions)
        self.assertEqual((result["unbaselined"], result["unexcused"], result["problems"]), (1, 0, []))
        self.assertEqual(result["unbaselinedDigest"], beta_gate.digest([key]))
        # A different unbaselined failure is not accepted by identity.
        new_key = ("demo/c", "Body1/F9", "face.shell")
        actions2 = actions + [dict(design=new_key[0], entity=new_key[1], action=new_key[2], kind="error", message=SHELL, ok=False)]
        value["source"]["totalActions"] = 5
        self.exceptions.write_text(json.dumps(value))
        result = beta_gate.evaluate_matrix(write_matrix(self.matrix, actions2), self.exceptions)
        self.assertEqual(result["unexcused"], 1)
        self.assertTrue(result["problems"])
        # A listed unbaselined action that passes is an improvement, never a pass claim.
        value["source"]["totalActions"] = 4
        self.exceptions.write_text(json.dumps(value))
        passed = ACTIONS + [dict(design=key[0], entity=key[1], action=key[2], kind="error", message=SHELL, ok=True)]
        result = beta_gate.evaluate_matrix(write_matrix(self.matrix, passed), self.exceptions)
        self.assertEqual((result["unbaselined"], result["unbaselinedFixed"], result["problems"]), (0, 1, []))

    def test_repository_manifest_is_itemised_and_classified(self):
        data, entries = beta_gate.load_exceptions(ROOT / "tests/action_matrix/beta-exceptions.json")
        self.assertEqual(data["source"]["totalActions"], 54033)
        self.assertEqual(len(entries), 213)
        self.assertEqual(len(entries), len({(e["design"], e["entity"], e["action"]) for e in entries.values()}))
        self.assertTrue(all(entry["issue"] in ("#55", "#83", "#88") for entry in entries.values()))
        # The develop manifest carries the 972 reviewed identities; main's older
        # 956 entry is superseded by the later review recorded here.
        self.assertEqual(data["source"]["unbaselinedFailures"], 972)
        self.assertEqual(len(data["unbaselined"]), 972)


class Properties(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "target/nextest/ci").mkdir(parents=True)

    def write(self, raw, report, junit=None):
        (self.root / "target/nextest/ci/raw-exit.txt").write_text(raw)
        if report is not None:
            (self.root / "target/nextest/ci/known-failure-exception.txt").write_text(report)
        if junit is not None:
            (self.root / "target/nextest/ci/junit.xml").write_text(junit)

    def test_raw_failure_is_excused_only_by_the_grader(self):
        self.write("100", "KNOWN FAILURE: #56 solidcraft-kernel::boolean_props::translation_invariant\n"
                           "known-failure-exception: 1 ran, 0 pass, 1 known failure(s) excused, 0 unexcused\n")
        self.assertEqual(beta_gate.evaluate_properties(self.root)["unexcused"], 0)

    def test_infrastructure_exit_or_failed_grader_is_rejected(self):
        for raw, report in (("2", "0 unexcused"), ("100", "known-failure-exception: FAILED\nREGRESSION"),
                            ("0", "known-failure-exception: no unexcused proof")):
            with self.subTest(raw=raw, report=report):
                self.write(raw, report)
                with self.assertRaises(beta_gate.BetaGateError):
                    beta_gate.evaluate_properties(self.root)

    def test_missing_raw_exit_is_inferred_from_the_real_junit(self):
        junit = ("<testsuites><testsuite name='solidcraft-kernel::boolean_props'>"
                 "<testcase name='translation_invariant'><failure>"
                 "SmBSplineSurface::Test #5 Bad NonMonotonic CPts [owner SmSphere] "
                 "a = Sph { r: 2.5 } off = 100000.0</failure></testcase>"
                 "</testsuite></testsuites>")
        (self.root / "target/nextest/ci/junit.xml").write_text(junit)
        self.assertEqual(beta_gate.evaluate_properties(self.root)["rawExit"], 100)
        changed = junit.replace("NonMonotonic", "Monotonic")
        (self.root / "target/nextest/ci/junit.xml").write_text(changed)
        with self.assertRaisesRegex(beta_gate.BetaGateError, "failure text changed"):
            beta_gate.evaluate_properties(self.root)

    def test_raw_junit_is_accepted_only_for_the_exact_issue_56_failure(self):
        junit = ("<testsuites><testsuite name='solidcraft-kernel::boolean_props'>"
                 "<testcase name='translation_invariant'><failure>"
                 "SmBSplineSurface::Test #5 Bad NonMonotonic CPts [owner SmSphere] "
                 "a = Sph { r: 2.5 } off = 100000.0</failure></testcase>"
                 "<testcase name='box_box_exact'/></testsuite></testsuites>")
        self.write("100", None, junit)
        self.assertEqual(beta_gate.evaluate_properties(self.root)["source"], "exact-issue-56")
        changed = junit.replace("NonMonotonic", "Monotonic")
        self.write("100", None, changed)
        with self.assertRaisesRegex(beta_gate.BetaGateError, "failure text changed"):
            beta_gate.evaluate_properties(self.root)
        extra = junit.replace("</testsuite>", "<testcase name='other'><failure>x</failure></testcase></testsuite>")
        self.write("100", None, extra)
        with self.assertRaisesRegex(beta_gate.BetaGateError, "unexpected failures"):
            beta_gate.evaluate_properties(self.root)
        self.write("2", None, junit)
        with self.assertRaisesRegex(beta_gate.BetaGateError, "not a test result"):
            beta_gate.evaluate_properties(self.root)


class Jobs(unittest.TestCase):
    def job(self, name, conclusion="success"):
        return dict(name=name, conclusion=conclusion, status="completed")

    def jobs(self):
        jobs = [self.job(name) for name in beta_gate.REQUIRED_JOBS]
        jobs += [self.job(f"ci / Action matrix ({shard}/16)", "failure") for shard in range(1, 17)]
        jobs.append(self.job("ci / Full baseline randomized properties (raw, many cases)", "failure"))
        jobs.append(self.job("ci / ci", "failure"))
        return jobs

    def test_beta_does_not_require_the_full_package_format_aggregate(self):
        self.assertNotIn("packages / All nonpublishing package builds", beta_gate.REQUIRED_JOBS)

    def test_all_required_jobs_and_shards(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "jobs.json"
            path.write_text(json.dumps(dict(jobs=self.jobs())))
            result = beta_gate.evaluate_jobs(path, dict(problems=[]), dict(rawExit=100, unexcused=0))
        self.assertEqual(result["problems"], [])

    def test_build_failure_missing_shard_and_missing_property_job_fail(self):
        for mutate in ("build", "shard", "property"):
            with self.subTest(mutate=mutate):
                jobs = self.jobs()
                if mutate == "build":
                    jobs[0]["conclusion"] = "failure"
                elif mutate == "shard":
                    jobs = [job for job in jobs if job["name"] != "ci / Action matrix (7/16)"]
                else:
                    jobs = [job for job in jobs if job["name"] != "ci / Full baseline randomized properties (raw, many cases)"]
                with tempfile.TemporaryDirectory() as directory:
                    path = Path(directory) / "jobs.json"
                    path.write_text(json.dumps(dict(jobs=jobs)))
                    result = beta_gate.evaluate_jobs(path, dict(problems=[]), dict(rawExit=100, unexcused=0))
                self.assertTrue(result["problems"])

    def test_cancelled_or_timed_out_jobs_are_not_evidence(self):
        for name, conclusion in (
            ("ci / Select checks", "cancelled"),
            ("ci / Action matrix (3/16)", "cancelled"),
            ("ci / Full baseline randomized properties (raw, many cases)", "timed_out"),
        ):
            with self.subTest(name=name, conclusion=conclusion):
                jobs = self.jobs()
                next(job for job in jobs if job["name"] == name)["conclusion"] = conclusion
                with tempfile.TemporaryDirectory() as directory:
                    path = Path(directory) / "jobs.json"
                    path.write_text(json.dumps(dict(jobs=jobs)))
                    result = beta_gate.evaluate_jobs(path, dict(problems=[]), dict(rawExit=100, unexcused=0))
                self.assertTrue(result["problems"])


if __name__ == "__main__":
    unittest.main()
