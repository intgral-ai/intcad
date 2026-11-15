#!/usr/bin/env python3
"""Self-test for release_candidate.py. Run: python3 -I scripts/ci/test_release_candidate.py."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_candidate  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


class CandidateState(unittest.TestCase):
    def test_draft_never_starts_the_gate(self):
        verify, reason = release_candidate.state(
            "pull_request", "opened", "true", "release/v0.1.0-beta.3", "main", "", "")
        self.assertFalse(verify)
        self.assertIn("draft", reason)

    def test_ready_for_review_starts_the_gate(self):
        verify, reason = release_candidate.state(
            "pull_request", "ready_for_review", "false", "release/v0.1.0-beta.3", "main", "", "")
        self.assertTrue(verify)
        self.assertEqual(reason, "ready_for_review")

    def test_ready_opened_starts_the_gate(self):
        verify, _ = release_candidate.state(
            "pull_request", "opened", "false", "release/v0.1.0-beta.3", "main", "", "")
        self.assertTrue(verify)

    def test_reopened_ready_starts_and_reopened_draft_does_not(self):
        self.assertTrue(release_candidate.state(
            "pull_request", "reopened", "false", "release/v0.1.0-beta.3", "main", "", "")[0])
        self.assertFalse(release_candidate.state(
            "pull_request", "reopened", "true", "release/v0.1.0-beta.3", "main", "", "")[0])

    def test_later_commit_does_not_start_the_gate(self):
        # release-gate.yml no longer subscribes to synchronize.  Keep this
        # decision explicit in case a future trigger re-adds it.
        verify, reason = release_candidate.state(
            "pull_request", "synchronize", "false", "release/v0.1.0-beta.3", "main", "", "")
        self.assertFalse(verify)
        self.assertIn("candidate changed", reason)

    def test_pr_to_develop_does_not_start_the_gate(self):
        verify, reason = release_candidate.state(
            "pull_request", "opened", "false", "feature/x", "develop", "", "")
        self.assertFalse(verify)
        self.assertIn("main", reason)

    def test_hotfix_to_main_starts_the_gate(self):
        verify, _ = release_candidate.state(
            "pull_request", "ready_for_review", "false", "hotfix/windows", "main", "", "")
        self.assertTrue(verify)

    def test_main_merge_group_and_explicit_runs_start_the_gate(self):
        self.assertTrue(release_candidate.state(
            "merge_group", "", "", "", "", "refs/heads/main", "refs/heads/release/v0.1.0")[0])
        self.assertTrue(release_candidate.state(
            "workflow_dispatch", "", "", "", "", "", "")[0])
        self.assertTrue(release_candidate.state(
            "workflow_call", "", "", "", "", "", "")[0])

    def test_manual_run_uses_a_diagnostic_context(self):
        self.assertEqual(release_candidate.context("workflow_dispatch"), "release-gate-diagnostic")
        self.assertEqual(release_candidate.context("pull_request"), "release-gate")
        self.assertEqual(release_candidate.context("merge_group"), "release-gate")

    def test_develop_merge_group_keeps_the_not_applicable_path(self):
        verify, reason = release_candidate.state(
            "merge_group", "", "", "", "", "refs/heads/develop", "refs/heads/feature/x")
        self.assertTrue(verify)
        self.assertIn("applicability", reason)


class Triggers(unittest.TestCase):
    def test_preparation_ci_runs_but_ready_transition_does_not_duplicate_it(self):
        text = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertIn("types: [opened, synchronize, reopened]", text)
        self.assertNotIn("types: [opened, synchronize, reopened, ready_for_review]", text)

    def test_release_gate_starts_only_on_fixed_candidate_transitions(self):
        text = (ROOT / ".github/workflows/release-gate.yml").read_text(encoding="utf-8")
        self.assertIn("types: [opened, reopened, ready_for_review]", text)
        self.assertNotIn("types: [opened, synchronize, reopened, ready_for_review]", text)
        self.assertIn("needs.candidate.outputs.verify", text)
        self.assertIn("Refuse an unfixed candidate", text)
        self.assertIn("Release gate not started", text)
        self.assertIn("needs.candidate.outputs.context", text)
        self.assertIn("release-gate-diagnostic",
                      (ROOT / "scripts/ci/release_candidate.py").read_text(encoding="utf-8"))
        self.assertIn("release-gate:", text)
        self.assertIn("if: always()", text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
