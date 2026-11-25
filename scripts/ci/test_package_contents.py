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
import package_contents  # noqa: E402  (after the path above)
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


def make_zst_tar(case, path, names):
    """A `.pkg.tar.zst` holding `names`, and how it was made.

    Python's `tarfile` writes zstd only from 3.14; the CI runner's 3.12 does not, and a test that
    just calls `w:zst` fails there on the interpreter instead of on the code under test. `zstd` on
    the PATH is the fallback; with neither, the case is skipped and says why.
    """
    plain = Path(str(path)[: -len(".zst")])
    with tarfile.open(plain, "w") as archive:
        for name in names:
            file = Path(path).parent / "payload"
            file.write_text("x")
            archive.add(file, arcname=name)
    try:
        with tarfile.open(path, "w:zst") as archive:
            for name in names:
                archive.add(Path(path).parent / "payload", arcname=name)
    except (tarfile.CompressionError, NotImplementedError, ValueError):
        done = subprocess.run(["zstd", "-q", "-f", str(plain), "-o", str(path)], capture_output=True)
        if done.returncode != 0:
            plain.unlink(missing_ok=True)
            case.skipTest("this Python cannot write a zstd tar and no zstd command is available")
    plain.unlink(missing_ok=True)
    return path


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
            derivative = Path(directory) / "intcad-linux-x86_64.flatpak"
            derivative.write_bytes(b"not a bundle this runner can read")
            report = Path(directory) / "contents.json"
            result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                     "--package-id", "release-flatpak-x86_64",
                                     "--source-revision", "a" * 40, "--distribution", "native",
                                     "--derived-from", "release-linux-x86_64",
                                     "--source-artifact", str(source), "--out", str(report),
                                     str(derivative)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(report.read_text())
            self.assertEqual(data["derivedFrom"], "release-linux-x86_64")
            self.assertEqual(data["sourceArtifact"], source.name)
            self.assertEqual(len(data["sourceSha256"]), 64)
            # The derivative itself is recorded and its payload is covered by the
            # inspected source package; a digest with no coverage would not be evidence.
            checked = {item["name"]: item for item in data["checked"]}
            self.assertEqual(sorted(checked), [derivative.name])
            self.assertEqual(checked[derivative.name]["format"], "covered")
            self.assertEqual(checked[derivative.name]["coveredBy"], "release-linux-x86_64")
            self.assertEqual(len(checked[derivative.name]["sha256"]), 64)
            self.assertEqual(data["unchecked"], [])


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

    def report(self, directory, names, distribution="mastra-agent", package_id="release-linux-x86_64"):
        package = Path(directory) / "intcad-linux-x86_64.tar.gz"
        make_tar(package, names)
        out = Path(directory) / "contents.json"
        result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                 "--package-id", package_id,
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
            self.assertIn("mastra-agent/node", data["agent"]["missing"])
            data = self.report(directory, ["usr/bin/intcad", *self.AGENT[:2]])
            self.assertTrue(data["agent"]["missing"])

    def test_a_universal_app_reports_a_node_for_each_slice(self):
        with tempfile.TemporaryDirectory() as directory:
            # A mounted DMG lists its directories too; the agent folder and each slice folder are
            # entries, not payload files.
            names = ["Applications", "Intcad.app/", "Intcad.app/Contents/",
                     "Intcad.app/Contents/Resources/", "Intcad.app/Contents/Resources/mastra-agent/"]
            for slice_ in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
                base = "Intcad.app/Contents/Resources/mastra-agent/" + slice_
                names.append(base + "/")
                names.extend(base + "/" + name.split("mastra-agent/", 1)[1] for name in self.AGENT)
            data = self.report(directory, names, package_id="release-macos")
            self.assertEqual(data["agent"]["nodeTargets"], ["aarch64-apple-darwin", "x86_64-apple-darwin"])
            self.assertEqual(data["agent"]["missing"], [])

    def test_an_msi_records_the_install_root_it_names(self):
        with tempfile.TemporaryDirectory() as directory:
            names = ["PFiles64/Intcad/mastra-agent/" + name.split("mastra-agent/", 1)[1]
                     for name in self.AGENT]
            data = self.report(directory, names, package_id="release-windows-x64")
            self.assertTrue(data["agent"]["present"])
            self.assertTrue(data["agent"]["node"])
            self.assertEqual(data["agent"]["missing"], [])

    def test_two_copies_cannot_split_the_agent_between_them(self):
        # One complete copy at `usr/lib` and a Node-less copy at `lib`: each place is its own
        # agent, so the second is reported incomplete instead of borrowing the first's Node.
        with tempfile.TemporaryDirectory() as directory:
            names = list(self.AGENT)
            names += [name.replace("usr/lib/", "lib/", 1) for name in self.AGENT
                      if not name.endswith("/node")]
            data = self.report(directory, ["usr/bin/intcad", *names])
            self.assertTrue(data["agent"]["present"])
            self.assertIn("mastra-agent/node", data["agent"]["missing"])

    def test_a_macos_agent_outside_the_app_bundle_is_not_the_apps(self):
        with tempfile.TemporaryDirectory() as directory:
            names = ["Contents/Resources/mastra-agent/node"]
            data = self.report(directory, names, package_id="release-macos")
            self.assertIsNone(data["agent"])

    def test_a_slice_without_the_modelling_skills_is_incomplete_beside_one_with_them(self):
        with tempfile.TemporaryDirectory() as directory:
            names = []
            for slice_ in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
                base = "Intcad.app/Contents/Resources/mastra-agent/" + slice_ + "/"
                names.extend(base + name.split("mastra-agent/", 1)[1]
                             for name in self.AGENT if "/skills/" not in name)
                skills = ("context-skills/intcad-context" if slice_ == "aarch64-apple-darwin"
                          else "modelling-skills/solidcraft-modeling")
                names.append(base + "skills/" + skills + "/SKILL.md")
            data = self.report(directory, names, package_id="release-macos")
            self.assertEqual(data["agent"]["missing"],
                             ["aarch64-apple-darwin/mastra-agent/skills/modelling-skills/<skill>/SKILL.md"])

    def test_a_deb_style_dot_prefix_is_the_packages_own(self):
        # `dpkg-deb --fsys-tarfile` names members `./usr/...`; the `.` is not part of the path.
        with tempfile.TemporaryDirectory() as directory:
            names = ["./" + name for name in self.AGENT]
            data = self.report(directory, ["usr/bin/intcad", *names])
            self.assertTrue(data["agent"]["present"])
            self.assertEqual(data["agent"]["missing"], [])
            self.assertEqual(data["markers"], [])

    def test_a_malformed_wrapper_name_is_not_the_packages_own(self):
        with tempfile.TemporaryDirectory() as directory:
            for wrapper in ("intcad-0.1.2-extras", "intcad-0.1.2-beta-linux-next.01"):
                names = [wrapper + "/lib/intcad/mastra-agent/node"]
                data = self.report(directory, names)
                self.assertIsNone(data["agent"], wrapper)
            data = self.report(directory, ["intcad-0.1.0-beta.1-linux-x86_64/lib/intcad/mastra-agent/node"])
            self.assertTrue(data["agent"]["present"], "a real prerelease wrapper still counts")

    def test_a_linux_tarball_wrapper_around_usr_is_the_packages_own(self):
        # `intcad-<version>/usr/lib/...` is what the tarball carries, and the app finds it there.
        with tempfile.TemporaryDirectory() as directory:
            names = ["intcad-0.1.2-linux-x86_64/usr/lib/intcad/mastra-agent/" + name.split("mastra-agent/", 1)[1]
                     for name in self.AGENT]
            data = self.report(directory, names)
            self.assertTrue(data["agent"]["present"])
            self.assertTrue(data["agent"]["node"])
            self.assertEqual(data["agent"]["missing"], [])

    def test_a_wrapper_with_a_leading_zero_prerelease_identifier_does_not_count(self):
        # The release workflow rejects numeric prerelease identifiers like `01`.
        with tempfile.TemporaryDirectory() as directory:
            names = ["intcad-0.1.3-beta.01-linux-x86_64/" + name for name in self.AGENT]
            data = self.report(directory, names)
            self.assertIsNone(data["agent"])

    def test_a_directory_is_not_the_file_it_is_named(self):
        # A tar lists a directory without a type of its own; it must not satisfy a required file
        # (`node`) or a skill (`SKILL.md`).
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory) / "intcad-linux-x86_64.tar.gz"
            tree = Path(directory) / "tree"
            (tree / "usr/lib/intcad/mastra-agent/node").mkdir(parents=True)
            (tree / "usr/lib/intcad/mastra-agent/agent.json").write_text("{}")
            (tree / "usr/lib/intcad/mastra-agent/skills/modelling-skills/x/SKILL.md").mkdir(parents=True)
            with tarfile.open(package, "w:gz") as archive:
                archive.add(tree / "usr", arcname="usr")
            out = Path(directory) / "contents.json"
            result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                     "--package-id", "release-linux-x86_64",
                                     "--source-revision", "a" * 40, "--distribution", "mastra-agent",
                                     "--out", str(out), str(package)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(out.read_text())
            self.assertFalse(data["agent"]["node"], "a directory named node is not a program")
            self.assertEqual(data["agent"]["skills"], [], "a directory named SKILL.md is not a skill")

    def test_an_agent_folder_outside_the_apps_resources_does_not_count(self):
        # The app looks beside its own resources: a copy an installer drops in `extras/` is not
        # the agent it starts, so a package is not credited with one there.
        with tempfile.TemporaryDirectory() as directory:
            data = self.report(directory, ["usr/bin/intcad", *self.AGENT,
                                           *[name.replace("mastra-agent/", "extras/mastra-agent/")
                                             for name in self.AGENT]],
                               package_id="release-macos")
            self.assertIsNone(data["agent"], "an extras/ folder is not the agent the app starts")
            # A wrapper named after something else is not the package's own either.
            for name in ("extras/Intcad/mastra-agent/node", "intcad-extras/lib/intcad/mastra-agent/node"):
                self.assertFalse(
                    package_contents.agent_report([name], roots=package_contents.roots_for(
                        "release-windows-x64" if name.startswith("extras/") else "release-linux-x86_64"))["present"],
                    name)
            self.assertTrue(data["markers"], "a Node outside the agent's install place is a marker")

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

    def test_a_derived_package_records_its_agent_carrying_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = self.make_package(directory)
            out = Path(directory) / "contents.json"
            result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                     "--package-id", "release-flatpak-x86_64",
                                     "--source-revision", "a" * 40,
                                     "--distribution", "mastra-agent",
                                     "--derived-from", "release-linux-x86_64",
                                     "--source-artifact", str(source), "--out", str(out)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(out.read_text())
            self.assertTrue(data["agent"]["present"] and data["agent"]["node"])
            self.assertEqual(data["markers"], [], "the agent's own Node is not the source's marker")
            self.assertEqual(data["unchecked"], [])

    def test_a_repack_is_inspected_itself_not_only_its_source(self):
        # The .flatpak/.pkg.tar.zst a person installs is recorded on its own bytes; a repack that
        # dropped the agent is refused even when the source tarball carried it.
        with tempfile.TemporaryDirectory() as directory:
            source = self.make_package(directory)
            repack = Path(directory) / "intcad-0.1.2-linux-x86_64.pkg.tar.zst"
            make_zst_tar(self, repack, ["usr/bin/intcad", "usr/share/doc/intcad/NOTICE"])
            out = Path(directory) / "contents.json"
            result = subprocess.run([sys.executable, str(HERE / "package_contents.py"),
                                     "--package-id", "release-arch-x86_64",
                                     "--source-revision", "a" * 40,
                                     "--distribution", "mastra-agent",
                                     "--derived-from", "release-linux-x86_64",
                                     "--source-artifact", str(source),
                                     "--out", str(out), str(repack)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(out.read_text())
            checked = [item["name"] for item in data["checked"]]
            self.assertIn(repack.name, checked, "the repack's own bytes are recorded")
            self.assertIn(source.name, data["agents"], "and the source it was built from")
            self.assertFalse(data["agents"][repack.name]["present"],
                             "the repack without an agent is recorded as carrying none")
            self.assertTrue(data["agents"][source.name]["present"])

    def test_a_package_with_both_the_agent_and_the_legacy_runtime_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            data = self.record(directory, self.make_package(directory, legacy_runtime=True), "mastra-agent")
            self.assertTrue(data["markers"], "the legacy mastra/ runtime is still a marker")
            self.assertIn("lib/intcad/mastra/runtime.json", [item["path"] for item in data["markers"]])


if __name__ == "__main__":
    unittest.main()
