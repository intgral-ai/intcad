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
            make_tar(package, ["bin/solidcraft", "bin/solidcraft-cli", "share/doc/solidcraft/NOTICE"])
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
            make_tar(package, ["bin/solidcraft", "mastra/runtime/node_modules/x", "lib/node"])
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
        self.assertFalse(contents.marker("share/doc/solidcraft/node.txt"))

    def test_a_zip_is_inspected(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory) / "solidcraft.zip"
            make_zip(package, ["bin/solidcraft", "node_modules/x/index.js"])
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
            make_zip(package, ["solidcraft.exe"])
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
            app.write_text("#!/bin/sh\nmkdir -p squashfs-root/usr/bin\ntouch squashfs-root/usr/bin/solidcraft\n")
            app.chmod(0o755)
            previous = os.getcwd()
            os.chdir(root)
            try:
                names = contents.names_appimage(Path("fake.AppImage"))
            finally:
                os.chdir(previous)
            self.assertIn("usr/bin/solidcraft", [name.replace("\\", "/") for name in names])

    def test_derived_package_records_source_name_and_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "solidcraft-linux-x86_64.tar.gz"
            make_tar(source, ["bin/solidcraft"])
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


if __name__ == "__main__":
    unittest.main()
