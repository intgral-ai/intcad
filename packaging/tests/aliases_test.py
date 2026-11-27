"""A release must carry the unversioned asset names the in-app update check resolves by name."""
import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "packaging/aliases.sh"
V = "0.1.3"


class ReleaseAliases(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def write(self, name, data=b"x"):
        (self.tmp / name).write_bytes(data)

    def run_script(self, *args):
        return subprocess.run(
            [str(SCRIPT), *args], capture_output=True, text=True, env={"PATH": os.environ["PATH"]}
        )

    def test_every_name_the_updater_asks_for_exists(self):
        # Exactly the unversioned forms in `asset_name` (update.rs), plus the ones that must not
        # get one: the tarball and the standalone exe are looked up with the version.
        self.write(f"intcad-{V}-linux-x86_64.AppImage")
        self.write(f"intcad-{V}-linux-x86_64.deb")
        self.write(f"intcad-{V}-linux-x86_64.rpm")
        self.write(f"intcad-{V}-linux-x86_64.flatpak")
        self.write(f"intcad-{V}-linux-x86_64.snap")
        self.write(f"intcad-{V}-windows-x64.msi")
        self.write(f"intcad-{V}-windows-x64-portable.zip")
        self.write(f"intcad-{V}-macos-universal.dmg")
        self.write(f"intcad-{V}-linux-x86_64.tar.gz")
        self.write(f"Intcad-{V}-windows-x64.exe")
        self.write(f"intcad-{V}-linux-x86_64.AppImage.zsync")

        result = self.run_script(str(self.tmp), V)

        self.assertEqual(result.returncode, 0, result.stderr)
        for alias in (
            "intcad-linux-x86_64.AppImage",
            "intcad-linux-x86_64.deb",
            "intcad-linux-x86_64.rpm",
            "intcad-linux-x86_64.flatpak",
            "intcad-linux-x86_64.snap",
            "intcad-windows-x64.msi",
            "intcad-windows-x64-portable.zip",
            "intcad-macos-universal.dmg",
        ):
            self.assertTrue((self.tmp / alias).is_file(), alias)
            self.assertEqual(
                (self.tmp / alias).read_bytes(),
                (self.tmp / f"intcad-{V}-{alias[len('intcad-'):]}").read_bytes(),
                f"{alias} is a byte copy",
            )
        self.assertFalse((self.tmp / "intcad-linux-x86_64.tar.gz").exists(), "the tarball keeps its version")
        self.assertFalse((self.tmp / "intcad-windows-x64.exe").exists(), "the standalone exe keeps its version")
        self.assertFalse((self.tmp / "intcad-linux-x86_64.AppImage.zsync").exists(), "zsync files keep their name")

    def test_v_prefix_is_accepted(self):
        self.write(f"intcad-{V}-windows-x64.msi")
        self.assertEqual(self.run_script(str(self.tmp), f"v{V}").returncode, 0)
        self.assertTrue((self.tmp / "intcad-windows-x64.msi").is_file())

    def test_an_alias_with_other_bytes_is_refused(self):
        self.write(f"intcad-{V}-windows-x64.msi", b"new")
        self.write("intcad-windows-x64.msi", b"old")

        result = self.run_script(str(self.tmp), V)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("different bytes", result.stderr)
        self.assertEqual((self.tmp / "intcad-windows-x64.msi").read_bytes(), b"old")

    def test_nothing_to_alias_is_reported(self):
        self.write(f"intcad-{V}-linux-x86_64.tar.gz")
        result = self.run_script(str(self.tmp), V)
        self.assertIn("no versioned asset needing an alias", result.stderr)


    def test_a_second_run_that_changes_nothing_is_quiet(self):
        self.write(f"intcad-{V}-windows-x64.msi")
        self.assertEqual(self.run_script(str(self.tmp), V).returncode, 0)

        again = self.run_script(str(self.tmp), V)

        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertNotIn("no versioned asset needing an alias", again.stderr)

    def test_a_symlinked_asset_is_refused(self):
        self.write(f"intcad-{V}-windows-x64.msi")
        elsewhere = self.tmp.parent / f"{self.tmp.name}-real.msi"
        elsewhere.write_bytes(b"msi")
        self.addCleanup(elsewhere.unlink)
        (self.tmp / f"intcad-{V}-linux-x86_64.deb").symlink_to(elsewhere)

        result = self.run_script(str(self.tmp), V)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("symlink", result.stderr)
        self.assertFalse((self.tmp / "intcad-linux-x86_64.deb").exists())

    def test_an_alias_that_is_a_symlink_is_refused(self):
        self.write(f"intcad-{V}-windows-x64.msi")
        elsewhere = self.tmp.parent / f"{self.tmp.name}-alias.msi"
        elsewhere.write_bytes(b"msi")
        self.addCleanup(elsewhere.unlink)
        (self.tmp / "intcad-windows-x64.msi").symlink_to(elsewhere)

        result = self.run_script(str(self.tmp), V)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not a regular file", result.stderr)
        self.assertEqual(elsewhere.read_bytes(), b"msi")


    def test_the_alias_is_a_copy_so_rebuilding_the_asset_cannot_change_it(self):
        self.write(f"intcad-{V}-windows-x64.msi", b"original")
        self.assertEqual(self.run_script(str(self.tmp), V).returncode, 0)
        alias = self.tmp / "intcad-windows-x64.msi"

        # Rebuilding the versioned file in place must not reach the published alias.
        (self.tmp / f"intcad-{V}-windows-x64.msi").write_bytes(b"rebuilt")

        self.assertEqual(alias.read_bytes(), b"original")


if __name__ == "__main__":
    unittest.main()
