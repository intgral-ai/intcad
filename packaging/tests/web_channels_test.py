"""web-channels.sh takes each release's own web zip, whatever that release called it.

Releases made since Intcad has its own file names attach `intcad-web-<version>.zip`; earlier tags
carry the same kind of zip under the name of their time. One deploy can take stable from an old
tag and the release candidate from a new one, so the name is resolved per tag and checked against
that tag's SHA256SUMS.txt under the exact name.
"""
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "packaging/web-channels.sh"
# The name earlier releases used, kept apart from the rest of a file name so that
# scripts/identity_to_intcad.py never rewrites the old names this test is about.
EARLIER = "solidcraft"

FAKE_GH = r'''#!/usr/bin/env python3
"""The three gh calls web-channels.sh makes, answered from a folder per release."""
import fnmatch, json, os, shutil, sys

releases = json.load(open(os.environ["FAKE_GH_RELEASES"]))
args = sys.argv[1:]
opt = lambda name: args[args.index(name) + 1]
if args[0] == "api":
    stable = "prerelease == false" in opt("--jq")
    tags = [t for t in releases["order"] if releases["tags"][t]["prerelease"] != stable]
    print("\n".join(tags[:1] if stable else tags))
elif args[:2] == ["release", "view"]:
    print("\n".join(sorted(os.listdir(releases["tags"][args[2]]["dir"]))))
elif args[:2] == ["release", "download"]:
    source = releases["tags"][args[2]]["dir"]
    names = [n for n in os.listdir(source) if fnmatch.fnmatchcase(n, opt("--pattern"))]
    if not names:
        sys.exit("no assets match the file pattern")
    for name in names:
        shutil.copy(os.path.join(source, name), opt("--dir"))
else:
    sys.exit("unexpected gh call: " + " ".join(args))
'''


@unittest.skipUnless(shutil.which("sha256sum") and shutil.which("unzip"), "needs sha256sum and unzip")
class WebChannels(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.site = self.tmp / "site"
        self.site.mkdir()
        self.edge = self.tmp / "edge"
        self.edge.mkdir()
        (self.edge / "index.html").write_text("edge")
        fake = self.tmp / "bin" / "gh"
        fake.parent.mkdir()
        fake.write_text(FAKE_GH)
        fake.chmod(0o755)
        self.releases = {"order": [], "tags": {}}

    def release(self, tag, zips, prerelease=False, listed=None):
        """A published release `tag` with the web zips `zips`. `listed` replaces what its
        SHA256SUMS.txt says: {name in the list: name of the zip whose hash it gives, or None for a wrong hash}."""
        folder = self.tmp / "releases" / tag
        folder.mkdir(parents=True)
        hashes = {}
        for name in zips:
            with zipfile.ZipFile(folder / name, "w") as z:
                top = name[: -len(".zip")]
                z.writestr(f"{top}/index.html", f"{tag} from {name}")
                z.writestr(f"{top}/sw.js", "// service worker")
                if prerelease:
                    z.writestr(f"{top}/channel-storage.txt", "per-channel storage")
            hashes[name] = hashlib.sha256((folder / name).read_bytes()).hexdigest()
        listed = {name: name for name in zips} if listed is None else listed
        (folder / "SHA256SUMS.txt").write_text("".join(f"{hashes[of] if of else 'f' * 64}  {name}\n" for name, of in listed.items()))
        self.releases["order"].append(tag)
        self.releases["tags"][tag] = {"prerelease": prerelease, "dir": str(folder)}

    def deploy(self):
        index = self.tmp / "releases.json"
        index.write_text(json.dumps(self.releases))
        env = {**os.environ, "PATH": f"{self.tmp / 'bin'}{os.pathsep}{os.environ['PATH']}", "GITHUB_REPOSITORY": "owner/repo", "FAKE_GH_RELEASES": str(index)}
        return subprocess.run(["bash", str(SCRIPT), str(self.site), str(self.edge)], env=env, capture_output=True, text=True)

    def test_an_old_stable_and_a_new_release_candidate_deploy_together(self):
        old, new = f"{EARLIER}-web-0.30.0.zip", "intcad-web-" + "0.31.0-rc.1.zip"
        self.release("v0.31.0-rc.1", [new], prerelease=True)
        self.release("v0.30.0", [old])
        done = self.deploy()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual((self.site / "app/index.html").read_text(), f"v0.30.0 from {old}")
        self.assertEqual((self.site / "app-rc/index.html").read_text(), f"v0.31.0-rc.1 from {new}")
        self.assertEqual((self.site / "app-edge/index.html").read_text(), "edge")
        self.assertTrue((self.site / "app-transfer.html").is_file() and (self.site / "app-transfer.js").is_file())

    def test_a_new_stable_deploys_under_its_new_name(self):
        new = "intcad-web-" + "0.31.0.zip"
        self.release("v0.31.0", [new])
        done = self.deploy()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual((self.site / "app/index.html").read_text(), f"v0.31.0 from {new}")

    def test_the_zip_must_have_the_hash_its_release_lists(self):
        old = f"{EARLIER}-web-0.30.0.zip"
        self.release("v0.30.0", [old], listed={old: None})
        done = self.deploy()
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse((self.site / "app/index.html").exists())

    def test_a_hash_listed_under_the_other_name_does_not_count(self):
        old, new = f"{EARLIER}-web-0.30.0.zip", "intcad-web-" + "0.30.0.zip"
        self.release("v0.30.0", [old], listed={new: old})
        done = self.deploy()
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse((self.site / "app/index.html").exists())

    def test_a_release_with_two_web_zips_is_refused(self):
        old, new = f"{EARLIER}-web-0.30.0.zip", "intcad-web-" + "0.30.0.zip"
        self.release("v0.30.0", [old, new])
        done = self.deploy()
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("exactly one web zip", done.stderr)
        self.assertFalse((self.site / "app/index.html").exists())


if __name__ == "__main__":
    unittest.main()
