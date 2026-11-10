#!/usr/bin/env python3
"""Tests for the native-package content decision (#14/#15 applicability)."""

import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
import release_content as content  # noqa: E402

SHA = "a" * 40
DG = "b" * 64
DG2 = "c" * 64


def build(identity, artifacts):
    return {
        "packageId": identity,
        "sourceRevision": SHA,
        "artifacts": [{"name": name, "sha256": digest} for name, digest in artifacts],
    }


def report(identity, checked, *, distribution="native", markers=(), unchecked=(), ignored=(), derived=None,
           source_name=None, source_sha=None):
    value = {
        "packageId": identity,
        "sourceRevision": SHA,
        "distribution": distribution,
        "checked": [{"name": name, "sha256": digest, "format": "tar", "entries": 10} for name, digest in checked],
        "ignored": list(ignored),
        "unchecked": [{"name": name, "reason": "no inspector"} for name in unchecked],
        "markers": [{"name": "pkg", "path": path} for path in markers],
        "derivedFrom": derived,
        "sourceArtifact": source_name,
        "sourceSha256": source_sha,
    }
    return value


class NativeContentDecision(unittest.TestCase):
    def test_native_installers_with_inspected_bytes_do_not_require_mastra(self):
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG), ("solidcraft.deb", DG2)]),
            "release-windows-x64": build("release-windows-x64", [("solidcraft.msi", DG)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG), ("solidcraft.deb", DG2)]),
            "release-windows-x64": report("release-windows-x64", [("solidcraft.msi", DG)]),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents), [])

    def test_runtime_markers_fail_the_native_declaration(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)],
                                                   markers=("usr/lib/solidcraft/mastra/runtime/node",))}
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("mastra", problems[0])
        self.assertIn("#15", problems[0])

    def test_uninspected_artifact_fails(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [], unchecked=("solidcraft.tar.gz",))}
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("content not inspected", problems[0])

    def test_missing_content_evidence_fails(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        problems = content.problems(SHA, descriptors, {})
        self.assertTrue(problems)
        self.assertIn("missing package content evidence", problems[0])

    def test_digest_mismatch_fails(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG2)])}
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("digest mismatch", problems[0])

    def test_derived_package_must_name_an_inspected_native_source(self):
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": build("release-flatpak-x86_64", [("solidcraft.flatpak", DG2)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": report("release-flatpak-x86_64", [], derived="release-linux-x86_64",
                                             source_name="solidcraft.tar.gz", source_sha=DG),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents), [])
        contents["release-flatpak-x86_64"]["derivedFrom"] = "release-macos"
        self.assertTrue(content.problems(SHA, descriptors, contents))

    def test_a_covered_standalone_exe_needs_its_covering_package(self):
        descriptors = {"release-windows-x64": build("release-windows-x64", [("solidcraft.zip", DG), ("SolidCraft.exe", DG2)])}
        contents = {
            "release-windows-x64": report("release-windows-x64", [("solidcraft.zip", DG), ("SolidCraft.exe", DG2)]),
        }
        contents["release-windows-x64"]["checked"][1]["coveredBy"] = "solidcraft.zip"
        self.assertEqual(content.problems(SHA, descriptors, contents), [])
        contents["release-windows-x64"]["checked"][1]["coveredBy"] = "missing.zip"
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("claims coverage", problems[0])

    def test_explicit_mastra_distribution_stays_blocked(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)],
                                                   distribution="mastra")}
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("#15/#14", problems[0])


class ReleaseWorkflowWiring(unittest.TestCase):
    """The aggregate must decide from package bytes, not append a permanent failure."""

    def setUp(self):
        self.gate = (ROOT / ".github/workflows/release-gate.yml").read_text()
        self.release = (ROOT / ".github/workflows/release.yml").read_text()

    def test_aggregate_no_longer_appends_unconditional_mastra_failures(self):
        self.assertNotIn("problems.append('#15: required runtime-artifact", self.gate)
        self.assertNotIn("problems.append('#14: required installed-runtime", self.gate)
        self.assertIn("content.problems(sha, descriptors, contents)", self.gate)

    def test_release_jobs_record_actual_package_contents(self):
        self.assertIn('SOLIDCRAFT_DISTRIBUTION: "native"', self.release)
        self.assertIn("scripts/ci/package_contents.py", self.release)
        self.assertIn("dist/release/contents.json", self.release)
        self.assertIn("package-evidence-${{ env.PACKAGE_ID }}", self.release)
        self.assertIn("dist/release/build.json", self.release)

    def test_derived_packages_must_name_the_inspected_linux_source(self):
        self.assertIn("release-flatpak-', 'release-arch-', 'release-snap-'", self.release)
        self.assertIn("--derived-from", self.release)
        self.assertIn("release-linux-", self.release)

if __name__ == "__main__":
    unittest.main()
