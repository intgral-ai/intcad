#!/usr/bin/env python3
"""Self-test for fea_suite.py and its workflow. Run: python3 -I scripts/ci/test_fea_suite.py.

The fixtures are the formats the real programs print (libtest's `--list` and result lines, a
reply's `programs` field compact and pretty, `gmsh --version`, `ccx -v`), not a real run: the
workflow runs the real suite. Each test names the failure it keeps from passing as green.
"""
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
import fea_suite  # noqa: E402

SHA = "c00b75f29" + "0" * 31
TESTS = ["cmd::stress::tests::a5_a_clamps_moment_is_the_force_times_its_lever_arm",
         "cmd::stress::tests::r2_a_plate_with_a_hole_in_tension_settles_at_heywoods_peak"]


def good_run(tests=TESTS, revision=SHA, mesher="gmsh 4.12.1", solver="CalculiX 2.21",
             gmsh_says="4.12.1\n", ccx_says="\nThis is Version 2.21\n\n"):
    """The files one good Linux run leaves, keyed by file name."""
    listing = "".join(f"{t}: test\n" for t in tests) + f"\n{len(tests)} tests, 0 benchmarks\n"
    stdout = (f"\nrunning {len(tests)} tests\n" + "".join(f"test {t} ... ok\n" for t in tests)
              + f"\ntest result: ok. {len(tests)} passed; 0 failed; 0 ignored; 0 measured; 3713 filtered out; "
              "finished in 702.31s\n\n")
    compact = json.dumps({"mesher": mesher, "solver": solver}, separators=(",", ":"))
    pretty = json.dumps({"programs": {"mesher": mesher, "solver": solver}}, indent=2)
    stderr = ("    Finished `release` profile [optimized + debuginfo] target(s) in 0.4s\n"
              f"A5 solid: ... ; moments 0.0% apart; programs {compact}\n{pretty}\n")
    programs = {"gmsh": {"argv": ["/usr/bin/gmsh", "--version"], "output": gmsh_says},
                "ccx": {"argv": ["/usr/bin/ccx", "-v"], "output": ccx_says}}
    return {"revision.txt": revision + "\n", "programs.json": json.dumps(programs), "listing.txt": listing,
            "suite.stdout": stdout, "suite.stderr": stderr}


def write(directory, files):
    directory.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        (directory / name).write_text(text, encoding="utf-8")
    return directory


class Grade(unittest.TestCase):
    def grade(self, files, platform="linux"):
        with tempfile.TemporaryDirectory() as directory:
            return fea_suite.grade(write(Path(directory) / "run", files), platform)

    def test_a_full_good_run_passes_and_reads_the_versions_both_ways(self):
        verdict = self.grade(good_run())
        self.assertEqual(verdict["problems"], [])
        self.assertEqual(verdict["listed"], TESTS)
        self.assertEqual(verdict["onRunner"], {"mesher": "gmsh 4.12.1", "solver": "CalculiX 2.21"})
        self.assertEqual(verdict["inReplies"], {"mesher": ["gmsh 4.12.1"], "solver": ["CalculiX 2.21"]})
        self.assertEqual(verdict["summary"]["passed"], 2)
        self.assertEqual(verdict["sourceRevision"], SHA)

    def test_an_empty_run_is_not_a_pass(self):
        # A filter that matches nothing (a renamed module) would otherwise pass with 0 tests.
        files = good_run(tests=[])
        self.assertIn("linux: the listing names no test: an empty run is not a pass", self.grade(files)["problems"])

    def test_a_failed_case_fails(self):
        files = good_run()
        files["suite.stdout"] = files["suite.stdout"].replace(f"test {TESTS[1]} ... ok", f"test {TESTS[1]} ... FAILED")
        files["suite.stdout"] = files["suite.stdout"].replace("test result: ok. 2 passed; 0 failed",
                                                              "test result: FAILED. 1 passed; 1 failed")
        problems = self.grade(files)["problems"]
        self.assertIn(f"linux: {TESTS[1]}: FAILED", problems)
        self.assertTrue(any("summary FAILED" in p for p in problems), problems)

    def test_a_listed_case_that_never_reported_fails(self):
        # A run killed part way (timeout, out of memory) leaves cases with no result line.
        files = good_run()
        files["suite.stdout"] = files["suite.stdout"].split(f"test {TESTS[1]}")[0]
        problems = self.grade(files)["problems"]
        self.assertIn(f"linux: {TESTS[1]}: no result", problems)
        self.assertIn("linux: no test summary (the run did not finish)", problems)

    def test_a_summary_that_disagrees_with_the_listing_fails(self):
        files = good_run()
        files["suite.stdout"] = files["suite.stdout"].replace("ok. 2 passed", "ok. 3 passed")
        self.assertTrue(any("against 2 listed" in p for p in self.grade(files)["problems"]))

    def test_another_version_in_a_reply_fails(self):
        files = good_run()
        files["suite.stderr"] += '{"mesher":"gmsh 4.13.1","solver":"CalculiX 2.21"}\n'
        self.assertIn("linux: a reply names the mesher 'gmsh 4.13.1', not the pinned 'gmsh 4.12.1'",
                      self.grade(files)["problems"])

    def test_replies_that_name_no_programs_fail(self):
        # The versions a run used are part of what it proves (spec A7).
        files = good_run()
        files["suite.stderr"] = "VA1 run limit here: 7.0 GB\n"
        problems = self.grade(files)["problems"]
        self.assertTrue(any("no reply named its mesher" in p for p in problems), problems)
        self.assertTrue(any("no reply named its solver" in p for p in problems), problems)

    def test_other_programs_on_the_runner_fail(self):
        problems = self.grade(good_run(ccx_says="\nThis is Version 2.22\n"))["problems"]
        self.assertIn("linux: ccx on the runner is CalculiX 2.22, not the pinned CalculiX 2.21", problems)
        problems = self.grade(good_run(gmsh_says="could not run /usr/bin/gmsh: not found"))["problems"]
        self.assertIn("linux: gmsh on the runner is unknown, not the pinned gmsh 4.12.1", problems)

    def test_missing_files_and_unknown_platforms_fail(self):
        files = good_run()
        del files["suite.stderr"]
        self.assertIn("linux: missing suite.stderr", self.grade(files)["problems"])
        self.assertEqual(self.grade(good_run(), platform="beos")["problems"], ["no pinned versions for platform 'beos'"])

    def test_banners_parse_as_the_programs_print_them(self):
        self.assertEqual(fea_suite.program_versions("4.12.1\n", "\nThis is Version 2.21\n\n"),
                         ("gmsh 4.12.1", "CalculiX 2.21"))
        self.assertEqual(fea_suite.program_versions("", None), (None, None))


class Handoff(unittest.TestCase):
    def test_the_gate_requires_this_commits_run_and_regrades_it(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(fea_suite.handoff_problems(root, SHA),
                             [f"fea-suite linux: no run of this commit (fea-suite-linux-{SHA})"])
            write(root / f"fea-suite-linux-{SHA}", good_run())
            self.assertEqual(fea_suite.handoff_problems(root, SHA), [])
            # A pass flag the producer wrote is not read: the raw lines decide.
            (root / f"fea-suite-linux-{SHA}" / "result.json").write_text('{"problems": []}')
            stdout = root / f"fea-suite-linux-{SHA}" / "suite.stdout"
            stdout.write_text(stdout.read_text().replace(f"{TESTS[0]} ... ok", f"{TESTS[0]} ... FAILED"))
            self.assertIn(f"fea-suite linux: {TESTS[0]}: FAILED", fea_suite.handoff_problems(root, SHA))

    def test_a_run_of_another_commit_is_not_this_ones(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root / f"fea-suite-linux-{SHA}", good_run(revision="1" * 40))
            self.assertIn(f"fea-suite linux: tested {'1' * 40}, not this commit {SHA}",
                          fea_suite.handoff_problems(root, SHA))


class Command(unittest.TestCase):
    def test_apt_packages_pin_every_package_exactly(self):
        packages = fea_suite.apt_packages("linux")
        self.assertEqual(packages, ["calculix-ccx=2.21-1", "gmsh=4.12.1+ds1-1.1build2",
                                    "libgmsh4.12t64=4.12.1+ds1-1.1build2"])

    def test_grade_writes_its_verdict_and_fails_the_step_on_a_problem(self):
        with tempfile.TemporaryDirectory() as directory:
            run = write(Path(directory) / "run", good_run())
            self.assertEqual(fea_suite.main(["grade", "--platform", "linux", "--dir", str(run)]), 0)
            self.assertEqual(json.loads((run / "result.json").read_text())["problems"], [])
            (run / "listing.txt").write_text("\n0 tests, 0 benchmarks\n")
            self.assertEqual(fea_suite.main(["grade", "--platform", "linux", "--dir", str(run)]), 1)


class Workflows(unittest.TestCase):
    def setUp(self):
        self.suite = (ROOT / ".github/workflows/fea-suite.yml").read_text(encoding="utf-8")
        self.gate = (ROOT / ".github/workflows/release-gate.yml").read_text(encoding="utf-8")

    def test_nightly_manual_and_release_gate_entry_points_only(self):
        for trigger in ("schedule:", "workflow_dispatch:", "workflow_call:"):
            self.assertIn(trigger, self.suite)
        self.assertNotRegex(self.suite, r"(?m)^  (push|pull_request|pull_request_target):")
        self.assertNotRegex(self.suite, r"(?m)^\s+[\w-]+: write\s*$")
        for forbidden in ("continue-on-error", "secrets.", "|| true", "SOLIDCRAFT_FEA_MAX_NODES"):
            self.assertNotIn(forbidden, self.suite)

    def test_the_job_runs_the_suite_against_the_pinned_programs(self):
        self.assertIn("fea_suite.py apt-packages --platform linux", self.suite)
        self.assertIn("fea_suite.py programs --platform linux", self.suite)
        self.assertIn("fea_suite.py grade --platform linux", self.suite)
        self.assertIn("SOLIDCRAFT_GMSH: /usr/bin/gmsh", self.suite)
        self.assertIn("SOLIDCRAFT_CCX: /usr/bin/ccx", self.suite)
        self.assertIn("-p solidcraft-engine --lib -- --ignored --test-threads=1 --nocapture cmd::stress fea::", self.suite)
        self.assertIn("-p solidcraft-engine --lib -- --ignored --list cmd::stress fea::", self.suite)
        self.assertIn("name: fea-suite-linux-${{ steps.head.outputs.sha || github.sha }}", self.suite)
        # Each platform the gate requires has a job that uploads its run.
        for platform in fea_suite.PLATFORMS:
            self.assertIn(f"name: fea-suite-{platform}-", self.suite)

    def test_the_release_gate_requires_the_suite_and_regrades_it(self):
        self.assertIn("uses: ./.github/workflows/fea-suite.yml", self.gate)
        self.assertIn("needs: [candidate, ci, kernel-rig, fea, packages, beta-gate]", self.gate)
        self.assertIn("pattern: fea-suite-*-${{ github.sha }}", self.gate)
        self.assertIn("fea_suite.handoff_problems(Path('fea-handoff'), sha)", self.gate)
        required = re.search(r"required = (\(.*?\)) if beta else (\(.*?\))", self.gate)
        self.assertIsNotNone(required)
        self.assertIn("'fea'", required[1], "a Beta needs the suite too")
        self.assertIn("'fea'", required[2])


if __name__ == "__main__":
    unittest.main(verbosity=2)
