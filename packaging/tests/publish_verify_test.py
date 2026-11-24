"""A release directory is refused unless it is exactly what the updater can install.

`packaging/publish/release-verify.py` runs before anything is uploaded: it checks the two
checksum names, the unversioned aliases `crates/ui-egui/src/update.rs` resolves, the platform
set, the hashes and the source revision recorded by every `build.json`.
"""
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "packaging/publish/release-verify.py"
VERSION = "0.1.3"
REVISION = "0123456789abcdef0123456789abcdef01234567"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class ReleaseVerify(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def write(self, name, data=b"x"):
        (self.tmp / name).write_bytes(data)
        return data

    def sums(self, names, legacy=True, differ=False):
        """Write the two lists for `names` (all files must exist)."""
        lines = [f"{sha256((self.tmp / n).read_bytes())}  {n}\n" for n in sorted(names)]
        (self.tmp / "SHA256SUMS.txt").write_text("".join(lines), encoding="utf-8")
        if legacy:
            body = "".join(lines)
            (self.tmp / "SHA256SUMS").write_text(body + ("\n" if differ else ""), encoding="utf-8")

    def complete_release(self, version=VERSION):
        names = [
            f"intcad-{version}-linux-x86_64.tar.gz",
            f"intcad-{version}-linux-x86_64.AppImage",
            f"intcad-{version}-macos-universal.dmg",
            f"intcad-cli-{version}-macos-universal.zip",
            f"intcad-{version}-windows-x64.msi",
            "intcad-linux-x86_64.AppImage",
            "intcad-macos-universal.dmg",
            "intcad-windows-x64.msi",
        ]
        # Aliases are byte copies of the versioned file they stand for (packaging/aliases.sh).
        for name in names:
            source = name
            for alias, versioned in (
                ("intcad-linux-x86_64.AppImage", f"intcad-{version}-linux-x86_64.AppImage"),
                ("intcad-macos-universal.dmg", f"intcad-{version}-macos-universal.dmg"),
                ("intcad-windows-x64.msi", f"intcad-{version}-windows-x64.msi"),
            ):
                if name == alias:
                    source = versioned
            self.write(name, source.encode())
        self.write("build.json", json.dumps({"sourceRevision": REVISION}).encode())
        self.sums(names)
        return names

    def run_verify(self, *args):
        return subprocess.run(
            ["python3", "-I", str(SCRIPT), "--dir", str(self.tmp), "--version", VERSION, "--revision", REVISION, *args],
            capture_output=True,
            text=True,
            env={"PATH": os.environ["PATH"]},
        )

    def test_a_complete_release_passes_and_writes_a_manifest(self):
        self.complete_release()
        result = self.run_verify("--manifest", str(self.tmp / "manifest.json"))
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads((self.tmp / "manifest.json").read_text())
        self.assertEqual(manifest["version"], VERSION)
        self.assertIn("intcad-windows-x64.msi", manifest["files"])
        self.assertEqual(manifest["build_revisions"], {"build.json": REVISION})

    def test_the_unversioned_alias_is_required(self):
        names = self.complete_release()
        names.remove("intcad-windows-x64.msi")
        (self.tmp / "intcad-windows-x64.msi").unlink()
        self.sums(names)
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn("intcad-windows-x64.msi is missing", result.stderr)

    def test_a_changed_file_is_refused(self):
        self.complete_release()
        self.write("intcad-macos-universal.dmg", b"tampered")
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn("does not match SHA256SUMS.txt", result.stderr)

    def test_the_two_lists_must_be_identical(self):
        names = self.complete_release()
        self.sums(names, differ=True)
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn("differ", result.stderr)

    def test_a_file_that_is_not_listed_is_refused(self):
        self.complete_release()
        self.write("intcad-linux-x86_64.deb", b"stray")
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn("directory holds", result.stderr)

    def test_another_revisions_package_is_refused(self):
        self.complete_release()
        self.write("build.json", json.dumps({"sourceRevision": "f" * 40}).encode())
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn("sourceRevision", result.stderr)

    def test_a_missing_platform_is_refused(self):
        names = self.complete_release()
        names = [n for n in names if "macos" not in n]
        (self.tmp / f"intcad-{VERSION}-macos-universal.dmg").unlink()
        (self.tmp / "intcad-macos-universal.dmg").unlink()
        self.sums(names)
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn("macos: no", result.stderr)

    def test_another_version_in_a_name_is_refused(self):
        self.complete_release()
        self.write(f"intcad-0.1.2-linux-x86_64.rpm", b"old")
        names = [p.name for p in self.tmp.iterdir() if p.suffix != ".json" and not p.name.startswith("SHA256SUMS")]
        self.sums(names)
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn("built for 0.1.2", result.stderr)

    def test_a_signature_can_be_required(self):
        self.complete_release()
        result = self.run_verify("--require-signature")
        self.assertEqual(result.returncode, 1)
        self.assertIn("is not signed", result.stderr)
        self.write("SHA256SUMS.txt.minisig", b"untrusted comment: test\n")
        result = self.run_verify("--require-signature")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_without_the_legacy_name_clients_before_0_1_3_cannot_install(self):
        names = self.complete_release()
        (self.tmp / "SHA256SUMS").unlink()
        result = self.run_verify()
        self.assertEqual(result.returncode, 1)
        self.assertIn("SHA256SUMS is missing", result.stderr)
        self.assertTrue(names)


if __name__ == "__main__":
    unittest.main()
