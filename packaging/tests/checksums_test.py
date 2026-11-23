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


class ReleaseChecksums(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def write(self, name: str, data: bytes) -> None:
        (self.tmp / name).write_bytes(data)

    def run_script(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [str(SCRIPT), *args], capture_output=True, text=True, env={"PATH": os.environ["PATH"]}
        )

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

    def test_a_missing_folder_fails(self):
        result = self.run_script(str(self.tmp / "missing"))

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr)


if __name__ == "__main__":
    unittest.main()
