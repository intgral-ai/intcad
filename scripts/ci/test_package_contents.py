#!/usr/bin/env python3
"""Tests for the actual-package content recorder."""

import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
import package_contents as contents  # noqa: E402


def make_tar(path, names):
    with tarfile.open(path, "w:gz") as archive:
        for name in names:
            file = Path(path).parent / "payload"
            file.write_text("x")
            archive.add(file, arcname=name)


def make_zip(path, names):
    with zipfile.ZipFile(path, "w") as archive:
        for name in names:
            archive.writestr(name, "x")


class PackageContents(unittest.TestCase):
    def test_a_native_linux_archive_has_no_runtime_markers(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory) / "solidcraft.tar.gz"
            make_tar(package, ["bin/intcad", "bin/intcad-cli", "share/doc/intcad/NOTICE"])
            report = Path(directory) / "contents.json"
            result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                     "--package-id", "release-linux-x86_64",
                                     "--source-revision", "a" * 40, "--distribution", "native",
                                     "--out", str(report), str(package)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(report.read_text())
            self.assertEqual(data["markers"], [])
            self.assertEqual(data["unchecked"], [])
            self.assertEqual([item["name"] for item in data["checked"]], ["solidcraft.tar.gz"])

    def test_a_bundled_mastra_or_node_marker_is_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory) / "solidcraft.tar.gz"
            make_tar(package, ["bin/intcad", "mastra/runtime/node_modules/x", "lib/node"])
            report = Path(directory) / "contents.json"
            result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                     "--package-id", "release-linux-x86_64",
                                     "--source-revision", "a" * 40, "--distribution", "native",
                                     "--out", str(report), str(package)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            paths = [item["path"] for item in json.loads(report.read_text())["markers"]]
            self.assertIn("mastra/runtime/node_modules/x", paths)
            self.assertIn("lib/node", paths)

    def test_an_empty_runtime_directory_is_not_a_bundled_runtime(self):
        self.assertEqual(contents.marker_entries(["portable/mastra/"]), [])
        self.assertTrue(contents.marker_entries(["portable/mastra/", "portable/mastra/runtime.json"]))

    def test_other_runtime_component_and_node_sidecar_are_markers(self):
        self.assertTrue(contents.marker("../ee/app/main.js"))
        self.assertTrue(contents.marker("resources/node.exe"))
        self.assertFalse(contents.marker("share/doc/intcad/node.txt"))

    def test_a_zip_is_inspected(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory) / "solidcraft.zip"
            make_zip(package, ["bin/intcad", "node_modules/x/index.js"])
            _, names = contents.inspect(package)
            self.assertIn("node_modules/x/index.js", names)

    def test_an_uninspectable_artifact_is_unchecked_not_native(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory) / "solidcraft.unknown"
            package.write_bytes(b"x")
            report = Path(directory) / "contents.json"
            result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                     "--package-id", "release-linux-x86_64",
                                     "--source-revision", "a" * 40, "--distribution", "native",
                                     "--out", str(report), str(package)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(report.read_text())
            self.assertEqual(data["checked"], [])
            self.assertEqual([item["name"] for item in data["unchecked"]], ["solidcraft.unknown"])

    def test_a_standalone_exe_is_covered_by_an_inspected_msi_or_zip(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory) / "solidcraft.zip"
            make_zip(package, ["intcad.exe"])
            app = Path(directory) / "SolidCraft.exe"
            app.write_bytes(b"MZ")
            report = Path(directory) / "contents.json"
            result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                     "--package-id", "release-windows-x64",
                                     "--source-revision", "a" * 40, "--distribution", "native",
                                     "--out", str(report), str(package), str(app)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(report.read_text())
            self.assertEqual(data["unchecked"], [])
            covered = [item for item in data["checked"] if item.get("coveredBy")]
            self.assertEqual([item["name"] for item in covered], ["SolidCraft.exe"])
            self.assertEqual(covered[0]["coveredBy"], "solidcraft.zip")

    def test_appimage_path_is_resolved_before_extracting_in_another_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            app = root / "fake.AppImage"
            app.write_text("#!/bin/sh\nmkdir -p squashfs-root/usr/bin\ntouch squashfs-root/usr/bin/intcad\n")
            app.chmod(0o755)
            previous = os.getcwd()
            os.chdir(root)
            try:
                names = contents.names_appimage(Path("fake.AppImage"))
            finally:
                os.chdir(previous)
            self.assertIn("usr/bin/intcad", [name.replace("\\", "/") for name in names])

    def test_derived_package_records_source_name_and_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "intcad-linux-x86_64.tar.gz"
            make_tar(source, ["bin/intcad"])
            report = Path(directory) / "contents.json"
            result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                     "--package-id", "release-flatpak-x86_64",
                                     "--source-revision", "a" * 40, "--distribution", "native",
                                     "--derived-from", "release-linux-x86_64",
                                     "--source-artifact", str(source), "--out", str(report)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(report.read_text())
            self.assertEqual(data["derivedFrom"], "release-linux-x86_64")
            self.assertEqual(data["sourceArtifact"], source.name)
            self.assertEqual(len(data["sourceSha256"]), 64)


class AgentContents(unittest.TestCase):
    """The built-in agent as the packages carry it: its own Node and files, whole or not there."""

    AGENT = [
        "usr/lib/intcad/mastra-agent/node",
        "usr/lib/intcad/mastra-agent/agent/main.mjs",
        "usr/lib/intcad/mastra-agent/agent.json",
        "usr/lib/intcad/mastra-agent/licenses/THIRD-PARTY-LICENSES.txt",
        "usr/lib/intcad/mastra-agent/licenses/NODE-LICENSE.txt",
        "usr/lib/intcad/mastra-agent/node_modules/@mastra/code-sdk/package.json",
        "usr/lib/intcad/mastra-agent/node_modules/@agentclientprotocol/sdk/package.json",
        "usr/lib/intcad/mastra-agent/skills/modelling-skills/solidcraft-modeling/SKILL.md",
        "usr/lib/intcad/mastra-agent/skills/modelling-skills/solidcraft-editing/SKILL.md",
    ]

    def report(self, directory, names, distribution="mastra-agent"):
        package = Path(directory) / "intcad-linux-x86_64.tar.gz"
        make_tar(package, names)
        out = Path(directory) / "contents.json"
        result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                 "--package-id", "release-linux-x86_64",
                                 "--source-revision", "a" * 40,
                                 "--distribution", distribution,
                                 "--out", str(out), str(package)],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(out.read_text())

    def test_a_whole_agent_folder_is_recorded_and_is_no_runtime_marker(self):
        with tempfile.TemporaryDirectory() as directory:
            data = self.report(directory, ["usr/bin/intcad", *self.AGENT])
            self.assertEqual(data["agent"], {"present": True, "node": True, "nodeTargets": [], "missing": [],
                                             "skills": ["modelling-skills/solidcraft-editing",
                                                        "modelling-skills/solidcraft-modeling"]})
            self.assertEqual(data["agentMarkers"], [])
            self.assertEqual(data["markers"], [])

    def test_an_agent_without_skills_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            data = self.report(directory, ["usr/bin/intcad", *[name for name in self.AGENT if "/SKILL.md" not in name]])
            self.assertEqual(data["agent"]["skills"], [])

    def test_an_agent_without_its_node_or_entry_is_incomplete(self):
        with tempfile.TemporaryDirectory() as directory:
            without_node = [name for name in self.AGENT if not name.endswith("/node")]
            data = self.report(directory, ["usr/bin/intcad", *without_node])
            self.assertFalse(data["agent"]["node"])
            self.assertEqual(data["agent"]["missing"], [])
            data = self.report(directory, ["usr/bin/intcad", *self.AGENT[:2]])
            self.assertTrue(data["agent"]["missing"])

    def test_a_universal_app_reports_a_node_for_each_slice(self):
        with tempfile.TemporaryDirectory() as directory:
            names = []
            for slice_ in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
                base = "Intcad.app/Contents/Resources/mastra-agent/" + slice_
                names.extend(base + "/" + name.split("mastra-agent/", 1)[1] for name in self.AGENT)
            data = self.report(directory, names)
            self.assertEqual(data["agent"]["nodeTargets"], ["aarch64-apple-darwin", "x86_64-apple-darwin"])
            self.assertEqual(data["agent"]["missing"], [])

    def test_a_legacy_runtime_or_a_stray_node_outside_the_agent_is_a_marker(self):
        with tempfile.TemporaryDirectory() as directory:
            data = self.report(directory, ["usr/bin/intcad", *self.AGENT,
                                           "usr/lib/intcad/mastra/runtime.json",
                                           "usr/lib/intcad/node_modules/x/index.js"])
            paths = [item["path"] for item in data["agentMarkers"]]
            self.assertIn("usr/lib/intcad/mastra/runtime.json", paths)
            self.assertIn("usr/lib/intcad/node_modules/x/index.js", paths)


class ReleasePackageEndToEnd(unittest.TestCase):
    """The recorder over a real package archive: a tarball made the way packaging/linux does."""

    def make_package(self, directory, *, with_agent=True, legacy_runtime=False):
        root = Path(directory) / "stage"
        files = {
            "bin/intcad": "x",
            "share/doc/intcad/THIRD-PARTY-LICENSES.txt": "x",
        }
        if with_agent:
            for name in AgentContents.AGENT:
                files[name.replace("usr/lib/intcad/", "lib/intcad/")] = "x"
        if legacy_runtime:
            files["lib/intcad/mastra/runtime.json"] = "{}"
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        package = Path(directory) / "intcad-0.1.2-linux-x86_64.tar.gz"
        with tarfile.open(package, "w:gz") as archive:
            for path in sorted(root.rglob("*")):
                archive.add(path, arcname=str(path.relative_to(root)))
        return package

    def record(self, directory, package, distribution):
        out = Path(directory) / "contents.json"
        result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                 "--package-id", "release-linux-x86_64",
                                 "--source-revision", "a" * 40,
                                 "--distribution", distribution,
                                 "--out", str(out), str(package)],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(out.read_text())

    def test_a_real_archive_with_the_agent_is_recorded_whole(self):
        with tempfile.TemporaryDirectory() as directory:
            data = self.record(directory, self.make_package(directory), "mastra-agent")
            self.assertTrue(data["agent"]["present"] and data["agent"]["node"])
            self.assertEqual(data["agent"]["missing"], [])
            self.assertEqual(data["agent"]["skills"], ["modelling-skills/solidcraft-editing",
                                                       "modelling-skills/solidcraft-modeling"])
            self.assertEqual(data["markers"], [])
            self.assertEqual(data["agentMarkers"], [])

    def test_the_same_archive_is_a_runtime_marker_when_it_claims_native(self):
        with tempfile.TemporaryDirectory() as directory:
            data = self.record(directory, self.make_package(directory), "native")
            self.assertTrue(data["markers"], "a native package may not carry the agent's node or node_modules")

    def test_a_package_with_both_the_agent_and_the_legacy_runtime_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            data = self.record(directory, self.make_package(directory, legacy_runtime=True), "mastra-agent")
            self.assertTrue(data["markers"], "the legacy mastra/ runtime is still a marker")
            self.assertIn("lib/intcad/mastra/runtime.json", [item["path"] for item in data["markers"]])


if __name__ == "__main__":
    unittest.main()
