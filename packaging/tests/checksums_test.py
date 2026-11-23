"""The release checksum list names the files a release publishes, under the names clients fetch.

The in-app update check (`crates/ui-egui/src/update/install.rs`, `SUMS`) and the web and Flatpak
publication scripts fetch `SHA256SUMS.txt`. Releases published by hand before this script used
`SHA256SUMS`, so both names must carry the same bytes.
"""
import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "packaging/checksums.sh"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class ChecksumTestCase(unittest.TestCase):
    """The temporary folder and the two calls every case needs. No tests of its own."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def write(self, name: str, data: bytes) -> None:
        (self.tmp / name).write_bytes(data)

    def run_script(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [str(SCRIPT), *args], capture_output=True, text=True, env={"PATH": os.environ["PATH"]}
        )


class ReleaseChecksums(ChecksumTestCase):
    def test_writes_both_names_with_sorted_hashes(self):
        self.write("intcad-0.1.3-windows-x64.msi", b"msi")
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")

        result = self.run_script(str(self.tmp))

        self.assertEqual(result.returncode, 0, result.stderr)
        expected = (
            f"{sha256(b'tar')}  intcad-0.1.3-linux-x86_64.tar.gz\n"
            f"{sha256(b'msi')}  intcad-0.1.3-windows-x64.msi\n"
        )
        self.assertEqual((self.tmp / "SHA256SUMS.txt").read_text(), expected)
        self.assertEqual((self.tmp / "SHA256SUMS").read_bytes(), (self.tmp / "SHA256SUMS.txt").read_bytes())

    def test_lists_neither_itself_nor_an_earlier_list(self):
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")
        self.write("SHA256SUMS.txt", b"stale\n")
        self.write("SHA256SUMS", b"stale\n")

        result = self.run_script(str(self.tmp))

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.tmp / "SHA256SUMS.txt").read_text(), f"{sha256(b'tar')}  intcad-0.1.3-linux-x86_64.tar.gz\n")

    def test_checks_every_hash_before_finishing(self):
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")

        result = self.run_script(str(self.tmp))

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("intcad-0.1.3-linux-x86_64.tar.gz: OK", result.stdout)

    def test_a_second_run_replaces_a_stale_list(self):
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")
        self.assertEqual(self.run_script(str(self.tmp)).returncode, 0)
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"rebuilt tar")

        result = self.run_script(str(self.tmp))

        self.assertEqual(result.returncode, 0, result.stderr)
        line = f"{sha256(b'rebuilt tar')}  intcad-0.1.3-linux-x86_64.tar.gz\n"
        self.assertEqual((self.tmp / "SHA256SUMS.txt").read_text(), line)
        self.assertEqual((self.tmp / "SHA256SUMS").read_text(), line)

    def test_an_empty_folder_fails(self):
        result = self.run_script(str(self.tmp))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no release files", result.stderr)

    def test_a_required_asset_must_be_listed(self):
        # The failure this guards: a per-platform run publishes a list that omits the other
        # platforms' files, and those users cannot update.
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")
        self.write("intcad-windows-x64.msi", b"msi")

        result = self.run_script(
            str(self.tmp),
            "--require",
            "intcad-0.1.3-linux-x86_64.tar.gz",
            "--require",
            "intcad-windows-x64.msi",
            "--require",
            "intcad-macos-universal.dmg",
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("intcad-macos-universal.dmg", result.stderr)
        self.assertFalse((self.tmp / "SHA256SUMS.txt").exists(), "a refused list is not written")

    def test_a_required_name_is_matched_whole(self):
        self.write("intcad-windows-x64.msi.part", b"part")

        result = self.run_script(str(self.tmp), "--require", "intcad-windows-x64.msi")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("intcad-windows-x64.msi", result.stderr)

    def test_a_symlinked_asset_is_refused(self):
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")
        (self.tmp / "intcad-windows-x64.msi").symlink_to(self.tmp / "intcad-0.1.3-linux-x86_64.tar.gz")

        result = self.run_script(str(self.tmp))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("symlink", result.stderr)

    def test_every_line_and_both_files_verify(self):
        self.write("a.tgz", b"a")
        self.write("b.msi", b"b")

        result = self.run_script(str(self.tmp))

        self.assertEqual(result.returncode, 0, result.stderr)
        listed = sorted((self.tmp / "SHA256SUMS.txt").read_text().splitlines())
        self.assertEqual(listed, sorted([f"{sha256(b'a')}  a.tgz", f"{sha256(b'b')}  b.msi"]))
        self.assertEqual((self.tmp / "SHA256SUMS").read_bytes(), (self.tmp / "SHA256SUMS.txt").read_bytes())

    def test_a_missing_folder_fails(self):
        result = self.run_script(str(self.tmp / "missing"))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr)

class EvidenceFilesAreNotAssets(ChecksumTestCase):
    def test_build_and_contents_json_are_left_out(self):
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")
        self.write("build.json", b"{}")
        self.write("contents.json", b"{}")

        result = self.run_script(str(self.tmp))

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            (self.tmp / "SHA256SUMS.txt").read_text(),
            f"{sha256(b'tar')}  intcad-0.1.3-linux-x86_64.tar.gz\n",
        )

    def test_a_folder_is_refused_rather_than_silently_skipped(self):
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")
        (self.tmp / "release-macos").mkdir()
        (self.tmp / "release-macos" / "intcad-macos-universal.dmg").write_bytes(b"dmg")

        result = self.run_script(str(self.tmp))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("flatten", result.stderr)

    def test_a_checksum_list_that_is_a_symlink_is_refused(self):
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")
        elsewhere = self.tmp.parent / f"{self.tmp.name}-elsewhere"
        elsewhere.write_text("not a list")
        self.addCleanup(elsewhere.unlink)
        (self.tmp / "SHA256SUMS").symlink_to(elsewhere)

        result = self.run_script(str(self.tmp))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("symlink", result.stderr)
        self.assertEqual(elsewhere.read_text(), "not a list", "the old file is untouched")


    def test_a_name_with_a_newline_is_refused(self):
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")
        (self.tmp / "intcad-0.1.3\nlosing.tar.gz").write_bytes(b"bad")

        result = self.run_script(str(self.tmp))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("newline", result.stderr)
        self.assertFalse((self.tmp / "SHA256SUMS.txt").exists(), "nothing is published")

    def test_a_temporary_list_that_is_a_symlink_writes_nothing_through_it(self):
        self.write("intcad-0.1.3-linux-x86_64.tar.gz", b"tar")
        elsewhere = self.tmp.parent / f"{self.tmp.name}-new-list"
        elsewhere.write_text("keep me")
        self.addCleanup(elsewhere.unlink)
        (self.tmp / "SHA256SUMS.txt.new").symlink_to(elsewhere)

        result = self.run_script(str(self.tmp))

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(elsewhere.read_text(), "keep me", "the symlink's target is untouched")
        self.assertTrue((self.tmp / "SHA256SUMS.txt").is_file())
        self.assertFalse((self.tmp / "SHA256SUMS.txt").is_symlink())


if __name__ == "__main__":
    unittest.main()
