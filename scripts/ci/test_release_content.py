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
           source_name=None, source_sha=None, agent=None, agent_markers=(), agents=None):
    value = {
        "packageId": identity,
        "sourceRevision": SHA,
        "distribution": distribution,
        "checked": [{"name": name, "sha256": digest, "format": "tar", "entries": 10} for name, digest in checked],
        "ignored": list(ignored),
        "unchecked": [{"name": name, "reason": "no inspector"} for name in unchecked],
        "markers": [{"name": "pkg", "path": path} for path in markers],
        "agentMarkers": [{"name": "pkg", "path": path} for path in agent_markers],
        "agent": agent,
        # Every artifact's own answer, as `package_contents.py` records it.
        "agents": agents if agents is not None else (
            {name: agent for name, _ in checked} if agent is not None else {}),
        "derivedFrom": derived,
        "sourceArtifact": name_of(source_name),
        "sourceSha256": source_sha,
    }
    return value


def name_of(value):
    return value


WHOLE_AGENT = {"present": True, "node": True, "nodeTargets": [], "missing": [],
               "skills": ["modelling-skills/solidcraft-modeling"]}
MAC_AGENT = {"present": True, "node": True, "missing": [],
             "nodeTargets": ["aarch64-apple-darwin", "x86_64-apple-darwin"],
             "skills": ["modelling-skills/solidcraft-modeling"]}


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
                                                   markers=("usr/lib/intcad/mastra/runtime/node",))}
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
        # A derived package follows the source's own distribution: a native source here.
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": build("release-flatpak-x86_64", [("solidcraft.flatpak", DG2)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": report("release-flatpak-x86_64", [("solidcraft.flatpak", DG2)],
                                             derived="release-linux-x86_64",
                                             source_name="solidcraft.tar.gz", source_sha=DG),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents, "native"), [])
        contents["release-flatpak-x86_64"]["derivedFrom"] = "release-macos"
        self.assertTrue(content.problems(SHA, descriptors, contents, "native"))

    def test_a_derived_package_needs_a_cleaned_source_scan(self):
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
        for field, value in (("markers", ["usr/lib/intcad/mastra"]), ("unchecked", ["solidcraft.tar.gz"])):
            contents["release-flatpak-x86_64"] = report(
                "release-flatpak-x86_64", [], derived="release-linux-x86_64",
                source_name="solidcraft.tar.gz", source_sha=DG, **{field: value})
            problems = content.problems(SHA, descriptors, contents)
            self.assertTrue(problems, field)
            self.assertIn("derived package", problems[0])

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

    def test_a_derived_package_may_alias_the_source_by_digest(self):
        # The release this stands for carries no agent: both packages are native.
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("intcad-0.1.0-beta.1-linux-x86_64.tar.gz", DG)]),
            "release-arch-x86_64": build("release-arch-x86_64", [("intcad-0.1.0-beta.1-linux-x86_64.pkg.tar.zst", DG2)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("intcad-0.1.0-beta.1-linux-x86_64.tar.gz", DG)]),
            "release-arch-x86_64": report("release-arch-x86_64",
                                          [("intcad-0.1.0-beta.1-linux-x86_64.pkg.tar.zst", DG2)],
                                          derived="release-linux-x86_64",
                                          source_name="intcad-0.1.0beta.1-linux-x86_64.tar.gz", source_sha=DG),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents, "native"), [])
        contents["release-arch-x86_64"]["sourceSha256"] = DG2
        self.assertTrue(content.problems(SHA, descriptors, contents, "native"))

    def test_explicit_mastra_distribution_stays_blocked(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)],
                                                   distribution="mastra")}
        problems = content.problems(SHA, descriptors, contents, "mastra")
        self.assertTrue(problems)
        self.assertIn("#15/#14", problems[0])


class AgentContentDecision(unittest.TestCase):
    """A release that declares the built-in agent must prove it from every supported package."""

    IDENTITIES = ("release-linux-x86_64", "release-macos", "release-windows-x64")

    def evidence(self, agent=WHOLE_AGENT, **extra):
        descriptors = {identity: build(identity, [("pkg", DG)]) for identity in self.IDENTITIES}
        contents = {}
        for identity in self.IDENTITIES:
            carried = MAC_AGENT if identity == "release-macos" else WHOLE_AGENT
            if agent is not None and agent is not WHOLE_AGENT:
                carried = dict(carried, **{k: v for k, v in agent.items() if v is not None})
            contents[identity] = report(identity, [("pkg", DG)], distribution="mastra-agent",
                                        agent=dict(carried) if carried else None, **extra)
        return descriptors, contents

    def test_a_release_with_the_agent_in_every_supported_package_is_supported(self):
        descriptors, contents = self.evidence()
        self.assertEqual(content.problems(SHA, descriptors, contents, "mastra-agent"), [])

    def test_a_supported_package_without_the_agent_fails_the_declaration(self):
        descriptors, contents = self.evidence()
        contents["release-macos"]["agent"] = None
        contents["release-macos"]["agents"] = {"pkg": None}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("release-macos" in problem and "no mastra-agent/" in problem for problem in problems), problems)

    def test_an_agent_without_its_node_or_with_missing_files_fails(self):
        descriptors, contents = self.evidence(agent={**WHOLE_AGENT, "node": False})
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("no Node program" in problem for problem in problems), problems)
        descriptors, contents = self.evidence(agent={**WHOLE_AGENT, "missing": ["mastra-agent/agent.json"]})
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("incomplete" in problem for problem in problems), problems)

    def test_an_agent_without_skills_fails_the_declaration(self):
        descriptors, contents = self.evidence(agent={**WHOLE_AGENT, "skills": []})
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("carries no modelling skill" in problem for problem in problems), problems)

    def test_one_broken_artifact_fails_the_package_beside_a_good_one(self):
        # A correct zip must not hide a broken MSI built from the same stage.
        descriptors, contents = self.evidence()
        identity = "release-windows-x64"
        contents[identity]["agents"] = {
            "pkg": WHOLE_AGENT,
            "broken.msi": {"present": True, "node": False, "missing": [], "skills": ["modelling-skills/solidcraft-modeling"]},
        }
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("broken.msi" in problem and "no Node program" in problem for problem in problems), problems)

    def test_a_universal_app_needs_a_node_for_each_slice(self):
        descriptors, contents = self.evidence()
        contents["release-macos"]["agent"] = dict(MAC_AGENT, nodeTargets=["aarch64-apple-darwin"])
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("no Node for x86_64-apple-darwin" in problem for problem in problems), problems)

    def test_a_skill_that_is_not_the_modelling_set_does_not_count(self):
        descriptors, contents = self.evidence()
        for identity in self.IDENTITIES:
            carried = dict(MAC_AGENT if identity == "release-macos" else WHOLE_AGENT)
            carried["skills"] = ["context-skills/intcad-context"]
            contents[identity]["agent"] = carried
            contents[identity]["agents"] = {"pkg": carried}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("carries no modelling skill" in problem for problem in problems), problems)

    def test_a_legacy_runtime_or_stray_node_still_fails_with_the_agent(self):
        descriptors, contents = self.evidence(markers=("usr/lib/intcad/mastra/runtime.json",))
        self.assertTrue(content.problems(SHA, descriptors, contents, "mastra-agent"))
        descriptors, contents = self.evidence(agent_markers=("usr/lib/intcad/node",))
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("outside mastra-agent/" in problem for problem in problems), problems)

    def test_an_unsupported_package_that_carries_the_agent_fails(self):
        descriptors, contents = self.evidence()
        identity = "release-linux-aarch64"
        descriptors[identity] = build(identity, [("pkg", DG)])
        contents[identity] = report(identity, [("pkg", DG)], distribution="native", agent=WHOLE_AGENT)
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "only" in problem for problem in problems), problems)

    def test_a_derived_package_must_match_its_source_distribution(self):
        descriptors, contents = self.evidence()
        identity = "release-flatpak-x86_64"
        descriptors[identity] = build(identity, [("pkg", DG2)])
        contents[identity] = report(identity, [("pkg", DG2)], distribution="native",
                                    derived="release-linux-x86_64", source_name="pkg", source_sha=DG)
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "does not match its source" in problem for problem in problems), problems)
        contents[identity]["distribution"] = "mastra-agent"
        # The repack's own bytes carry the agent too, as the recorder records them.
        contents[identity]["checked"] = [{"name": "pkg", "sha256": DG2, "format": "tar", "entries": 10}]
        contents[identity]["agents"] = {"pkg": MAC_AGENT, "pkg-src": MAC_AGENT}
        contents[identity]["sourceArtifact"] = "pkg-src"
        self.assertEqual(content.problems(SHA, descriptors, contents, "mastra-agent"), [])

    def test_a_package_of_only_a_cli_archive_carries_no_agent(self):
        # The headless CLI has no Assistant: a package set that is only that archive proves
        # nothing about the app's agent.
        descriptors = {"release-macos": build("release-macos", [("intcad-cli-0.1.2-macos-arm64.zip", DG)])}
        contents = {"release-macos": report(
            "release-macos", [("intcad-cli-0.1.2-macos-arm64.zip", DG)], distribution="mastra-agent",
            agent={"present": False, "node": False, "nodeTargets": [], "missing": [], "skills": []})}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("no artifact of the package carries the agent" in problem for problem in problems), problems)

    def test_a_repack_artifact_without_a_verdict_fails_and_a_runtime_marker_does_too(self):
        descriptors, contents = self.evidence()
        identity = "release-snap-x86_64"
        descriptors[identity] = build(identity, [("one", DG), ("two", DG2)])
        contents[identity] = report(identity, [("one", DG), ("two", DG2)], distribution="mastra-agent",
                                    derived="release-linux-x86_64", source_name="src", source_sha=DG)
        contents[identity]["agents"] = {"one": WHOLE_AGENT, "src": WHOLE_AGENT}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "two: inspected without an agent verdict" in problem
                            for problem in problems), problems)
        contents[identity]["agents"]["two"] = WHOLE_AGENT
        contents[identity]["markers"] = [{"name": "two", "path": "usr/lib/intcad/mastra/runtime.json"}]
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "runtime marker" in problem for problem in problems), problems)

    def test_a_native_repack_is_not_asked_for_the_agent(self):
        # The agent is built for x86_64 Linux only; the aarch64 Flatpak is native on both sides.
        descriptors = {
            "release-linux-aarch64": build("release-linux-aarch64", [("tarball", DG)]),
            "release-flatpak-aarch64": build("release-flatpak-aarch64", [("pkg", DG2)]),
        }
        contents = {
            "release-linux-aarch64": report("release-linux-aarch64", [("tarball", DG)]),
            "release-flatpak-aarch64": report("release-flatpak-aarch64", [("pkg", DG2)],
                                              derived="release-linux-aarch64",
                                              source_name="tarball", source_sha=DG),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents, "mastra-agent"), [])

    def test_a_repack_without_the_agent_in_its_own_bytes_fails(self):
        # The source tarball carries the agent; the repack's own payload does not.
        descriptors, contents = self.evidence()
        identity = "release-flatpak-x86_64"
        descriptors[identity] = build(identity, [("pkg", DG2)])
        contents[identity] = report(identity, [("pkg", DG2)], distribution="mastra-agent",
                                    derived="release-linux-x86_64", source_name="src", source_sha=DG)
        contents[identity]["agents"] = {
            "pkg": {"present": False, "node": False, "nodeTargets": [], "missing": [], "skills": []},
            "src": WHOLE_AGENT,
        }
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "pkg: carries no mastra-agent/" in problem
                            for problem in problems), problems)

    def test_a_declaration_the_packages_do_not_match_fails(self):
        descriptors, contents = self.evidence()
        contents["release-linux-x86_64"]["distribution"] = "native"
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("recorded as native" in problem for problem in problems), problems)
        self.assertTrue(content.problems(SHA, descriptors, contents, "unknown-kind"))


class ReleaseWorkflowWiring(unittest.TestCase):
    """The aggregate must decide from package bytes, not append a permanent failure."""

    def setUp(self):
        self.gate = (ROOT / ".github/workflows/release-gate.yml").read_text()
        self.release = (ROOT / ".github/workflows/release.yml").read_text()

    def test_aggregate_no_longer_appends_unconditional_mastra_failures(self):
        self.assertNotIn("problems.append('#15: required runtime-artifact", self.gate)
        self.assertNotIn("problems.append('#14: required installed-runtime", self.gate)
        self.assertIn("content.problems(sha, descriptors, contents, os.environ.get('SOLIDCRAFT_DISTRIBUTION', 'mastra-agent'))", self.gate)
        self.assertIn('SOLIDCRAFT_DISTRIBUTION: "mastra-agent"', self.gate, "a release gate must declare what it ships")

    def test_release_jobs_record_actual_package_contents(self):
        self.assertIn('SOLIDCRAFT_DISTRIBUTION: "mastra-agent"', self.release)
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
