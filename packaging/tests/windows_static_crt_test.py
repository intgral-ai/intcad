"""The static-CRT guard reads both PE32 (the x86 package) and PE32+ import tables."""
import pathlib
import shutil
import struct
import subprocess
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "windows/check-static-crt.ps1"
SHELL = shutil.which("pwsh") or shutil.which("powershell")


def image(pe64=True, imported="kernel32.dll", magic=None):
    """A minimal PE with one section and one import name, enough for the guard's header walk."""
    data = bytearray(0x400)
    def u16(at, value): struct.pack_into("<H", data, at, value)
    def u32(at, value): struct.pack_into("<I", data, at, value)
    u16(0, 0x5A4D)
    u32(0x3C, 0x80)
    u32(0x80, 0x4550)
    u16(0x84, 0x014C if not pe64 else 0x8664)  # i386 / AMD64
    u16(0x86, 1)                                # one section
    optional = 0x98
    size = 240 if pe64 else 224
    u16(0x94, size)
    u16(optional, magic if magic is not None else (0x020B if pe64 else 0x010B))
    directories = optional + (112 if pe64 else 96)
    # Section: rva 0x1000 maps to raw 0x200, size 0x200.
    u32(directories + 8, 0x1000)   # import directory RVA
    u32(directories + 12, 40)      # import directory size
    section = optional + size
    data[section:section + 8] = b".rdata\x00\x00"
    u32(section + 8, 0x200)        # virtual size
    u32(section + 12, 0x1000)      # virtual address
    u32(section + 16, 0x200)       # raw size
    u32(section + 20, 0x200)       # raw pointer
    # Import descriptor (20 bytes) at rva 0x1000 / raw 0x200, name at rva 0x1020 / raw 0x220.
    u32(0x200 + 12, 0x1020)
    name = imported.encode("ascii") + b"\x00"
    data[0x220:0x220 + len(name)] = name
    return bytes(data)


class StaticCrtGuard(unittest.TestCase):
    def run_guard(self, data):
        if not SHELL:
            self.skipTest("needs pwsh or Windows PowerShell")
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "app.exe"
            path.write_bytes(data)
            return subprocess.run([SHELL, "-NoProfile", "-File", str(SCRIPT), "-Path", str(path)],
                                  capture_output=True, text=True)

    def test_both_optional_header_magics_are_read(self):
        # PE32 is the x86 package; the first version of this guard rejected it outright.
        for pe64 in (True, False):
            with self.subTest(pe64=pe64):
                done = self.run_guard(image(pe64=pe64))
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertIn("static C runtime", done.stdout)

    def test_a_redistributable_import_is_refused_in_both_images(self):
        for pe64 in (True, False):
            with self.subTest(pe64=pe64):
                done = self.run_guard(image(pe64=pe64, imported="vcruntime140.dll"))
                self.assertNotEqual(done.returncode, 0)
                self.assertIn("vcruntime140.dll", done.stderr)

    def test_another_optional_header_magic_is_refused(self):
        done = self.run_guard(image(magic=0x0107))
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("Not a PE32 or PE32+ image", done.stderr)


if __name__ == "__main__":
    unittest.main()
