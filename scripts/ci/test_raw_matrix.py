import hashlib
import json
import tempfile
import sys
import os
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate  # noqa: E402
import raw_matrix  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
SHA = "a" * 40
SUITE = "solidcraft-cli::action_matrix"


def plan_bytes():
    jobs = {job: dict(decision="not_applicable", reason="no_relevant_change", missingInputs=[])
            for job in sorted(gate.JOBS)}
    jobs["repo"] = dict(decision="run", reason="always", missingInputs=[])
    jobs["rust"] = dict(decision="run", reason="full_validation", missingInputs=[])
    plan = dict(version=1, sourceRevision=SHA, releaseRequired=False, eventName="schedule", scope="full",
                jobs=jobs, openObligations=[], rustTestPackages=[], rustDeferredPackages=[], rustDeferredTo="")
    return (json.dumps(plan, indent=2, sort_keys=True) + "\n").encode()


def listing(name):
    return {
        "rust-build-meta": {"target-directory": "/tmp/target"},
        "rust-suites": {SUITE: {"binary-id": SUITE, "status": "listed",
                                 "testcases": {name: {"ignored": True, "filter-match": {"status": "matches"}}}}},
        "test-count": 1,
    }


def junit(name, outcome):
    body = f'<testcase name="{name}">'
    if outcome == "failed":
        body += "<failure>failed</failure>"
    elif outcome == "skipped":
        body += "<skipped/>"
    body += "</testcase>"
    return f'<testsuites><testsuite name="{SUITE}">{body}</testsuite></testsuites>'.encode()


def command_argv(verb, expression):
    common = ["--archive-file", "tests.tar.zst", "--workspace-remap", ".", "--extract-to", ".",
              "--extract-overwrite", "--profile", "ci", "--ignore-default-filter", "--run-ignored", "only",
              "-E", expression]
    if verb == "list":
        return ["cargo", "nextest", "list", *common, "--message-format", "json"]
    return ["cargo", "nextest", "run", *common, "--success-output", "final", "--test-threads", "2"]


def bundle(root, shard, plan, *, outcome="passed", exit_code=0, name=None, revision=SHA, digest=None,
           expected_extra=None, directory=None, skipped=False):
    name = name or ("action_matrix_determinism" if shard == "determinism" else "action_matrix_shard")
    folder = Path(root) / (directory or f"action-matrix-{shard}")
    folder.mkdir(parents=True, exist_ok=True)
    expression = raw_matrix.RUN_EXPRESSION[shard]
    commands = [dict(argv=command_argv("list", expression), exitCode=0),
                dict(argv=command_argv("run", expression), exitCode=exit_code)]
    (folder / "commands.json").write_text(json.dumps(commands))
    selected = listing(name)
    if expected_extra:
        selected["rust-suites"][SUITE]["testcases"][expected_extra] = {
            "ignored": True, "filter-match": {"status": "matches"}}
        selected["test-count"] = 2
    (folder / "selection.json").write_text(json.dumps(selected))
    (folder / "junit.xml").write_bytes(junit(name, "skipped" if skipped else outcome))
    evidence = dict(version=1, shard=shard,
                    matrixShard="" if shard == "determinism" else shard + "/16", sourceRevision=revision,
                    planDigest=digest or gate.digest_for(gate.BoundPlan(plan)),
                    rawExitCode=exit_code, expectedIds=[f"{SUITE}::{name}"] +
                    ([f"{SUITE}::{expected_extra}"] if expected_extra else []),
                    passedIds=[] if outcome != "passed" else [f"{SUITE}::{name}"] +
                    ([f"{SUITE}::{expected_extra}"] if expected_extra else []),
                    failedIds=[f"{SUITE}::{name}"] if outcome == "failed" else [],
                    skippedIds=[f"{SUITE}::{name}"] if skipped else [],
                    junitSha256=hashlib.sha256((folder / "junit.xml").read_bytes()).hexdigest())
    (folder / "matrix-evidence.json").write_text(json.dumps(evidence))
    return folder


class RawMatrixTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.plan = plan_bytes()
        self.plan_path = self.root / "plan.json"
        self.plan_path.write_bytes(self.plan)
        self.evidence = self.root / "evidence"
        self.out = self.root / "out"

    def tearDown(self):
        self.temp.cleanup()

    def all_shards(self, **kwargs):
        for shard in raw_matrix.SHARDS:
            bundle(self.evidence, shard, self.plan, **kwargs)

    def verify(self):
        return raw_matrix.verify(self.plan_path, self.evidence, self.out)

    def test_all_seventeen_pass(self):
        self.all_shards()
        self.assertEqual(self.verify(), 0)
        result = json.load(open(self.out / "raw-result.json"))
        self.assertTrue(result["rawPassed"])
        self.assertEqual(len(result["shards"]), 17)

    def test_raw_failure_stays_red_even_when_regression_accepts_known_failure(self):
        known = next(line.split()[0] for line in (ROOT / "tests/KNOWN_FAILING_TESTS").read_text().splitlines()
                     if line and not line.startswith("#"))
        self.all_shards()
        failed = bundle(self.evidence, "7", self.plan, outcome="failed", exit_code=100,
                        name=known.split("::", 1)[1])
        self.assertEqual(self.verify(), 1)
        result = json.load(open(self.out / "raw-result.json"))
        self.assertFalse(result["rawPassed"])
        self.assertEqual(result["shards"]["7"]["exitCode"], 100)

    def test_missing_shard_cannot_pass(self):
        for shard in raw_matrix.SHARDS:
            if shard != "16":
                bundle(self.evidence, shard, self.plan)
        self.assertEqual(self.verify(), 1)
        self.assertIn("16", " ".join(json.load(open(self.out / "raw-result.json"))["problems"]))

    def test_missing_determinism_cannot_pass(self):
        for shard in raw_matrix.SHARDS:
            if shard != "determinism":
                bundle(self.evidence, shard, self.plan)
        self.assertEqual(self.verify(), 1)
        self.assertIn("determinism", json.load(open(self.out / "raw-result.json"))["problems"][0])

    def test_skipped_case_cannot_pass(self):
        self.all_shards()
        for path in self.evidence.rglob("matrix-evidence.json"):
            if path.parent.name.endswith("-3"):
                (path.parent / "junit.xml").write_bytes(junit("action_matrix_shard", "skipped"))
                data = json.load(open(path))
                data["skippedIds"] = data["passedIds"]
                data["passedIds"] = []
                data["failedIds"] = []
                data["junitSha256"] = hashlib.sha256((path.parent / "junit.xml").read_bytes()).hexdigest()
                path.write_text(json.dumps(data))
        self.assertEqual(self.verify(), 1)

    def test_exit_zero_with_failed_junit_cannot_pass(self):
        self.all_shards()
        bundle(self.evidence, "4", self.plan, outcome="failed", exit_code=0)
        self.assertEqual(self.verify(), 1)

    def test_exit_100_with_passed_junit_cannot_pass(self):
        self.all_shards()
        bundle(self.evidence, "4", self.plan, outcome="passed", exit_code=100)
        self.assertEqual(self.verify(), 1)

    def test_wrong_executed_shard_cannot_pass(self):
        self.all_shards()
        path = self.evidence / "action-matrix-8" / "matrix-evidence.json"
        data = json.load(open(path))
        data["matrixShard"] = "9/16"
        path.write_text(json.dumps(data))
        self.assertEqual(self.verify(), 1)

    def test_wrong_revision_cannot_pass(self):
        self.all_shards()
        bundle(self.evidence, "5", self.plan, revision="b" * 40)
        self.assertEqual(self.verify(), 1)

    def test_duplicate_shard_cannot_pass(self):
        self.all_shards()
        bundle(self.evidence / "extra", "1", self.plan)
        self.assertEqual(self.verify(), 1)

    def test_extra_cases_without_evidence_cannot_pass(self):
        self.all_shards()
        bundle(self.evidence, "6", self.plan, expected_extra="extra_case")
        (self.evidence / "action-matrix-6" / "junit.xml").write_bytes(junit("action_matrix_shard", "passed"))
        self.assertEqual(self.verify(), 1)

    def test_infrastructure_exit_cannot_pass(self):
        self.all_shards()
        bundle(self.evidence, "2", self.plan, exit_code=1)
        self.assertEqual(self.verify(), 1)

    def _record(self, shard, exit_code, outcome):
        out = self.root / "record" / shard
        expression = raw_matrix.RUN_EXPRESSION[shard]

        def capture(argvs, destination):
            destination = Path(destination)
            destination.mkdir(parents=True, exist_ok=True)
            name = "action_matrix_determinism" if shard == "determinism" else "action_matrix_shard"
            (destination / "selection.json").write_text(json.dumps(listing(name)))
            junit_path = Path("target/nextest/ci/junit.xml")
            junit_path.parent.mkdir(parents=True, exist_ok=True)
            junit_path.write_bytes(junit(name, outcome))
            return [dict(argv=argvs[0], exitCode=0), dict(argv=argvs[1], exitCode=exit_code)]

        old = os.getcwd()
        os.chdir(self.root)
        try:
            with mock.patch.object(raw_matrix.main_merge, "check_revision"), \
                 mock.patch.object(raw_matrix.main_merge, "capture_commands", side_effect=capture), \
                 mock.patch.dict(os.environ, {"ACTION_MATRIX_SHARD": "" if shard == "determinism" else shard + "/16"}):
                status = raw_matrix.record(self.plan_path, shard, out)
        finally:
            os.chdir(old)
        return status, out

    def test_record_pass_and_raw_failure(self):
        status, out = self._record("5", 0, "passed")
        self.assertEqual(status, 0)
        self.assertEqual(json.load(open(out / "matrix-evidence.json"))["rawExitCode"], 0)
        status, out = self._record("6", 100, "failed")
        self.assertEqual(status, 100)
        self.assertEqual(json.load(open(out / "matrix-evidence.json"))["rawExitCode"], 100)

    def test_record_rejects_exit_100_with_passed_junit(self):
        status, _ = self._record("7", 100, "passed")
        self.assertEqual(status, 1)

    def test_missing_commands_cannot_pass(self):
        self.all_shards()
        (self.evidence / "action-matrix-2" / "commands.json").unlink()
        self.assertEqual(self.verify(), 1)


if __name__ == "__main__":
    unittest.main()
