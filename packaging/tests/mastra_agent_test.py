"""The built-in agent's folder is staged into a package only after its check passes.

Uses a made-up folder: nothing of Mastra's, no npm, no network. It proves that
packaging/mastra/stage-agent.sh copies a folder that passes, and copies nothing when the folder
holds a stored sign-in, is for another target, or is not there. It does not build the real
folder and says nothing about it. Needs `node` (any current version) for the check itself.

The made-up folder is checked against a made-up lock file through the script's test-only `--pin`;
without the test guard the script refuses that option.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
STAGE = ROOT / "packaging/mastra/stage-agent.sh"
REMOVE = ROOT / "packages/mastra-runtime/scripts/remove-enterprise.mjs"
TARGET = "x86_64-unknown-linux-gnu"
PACKAGES = ("@mastra/code-sdk", "@agentclientprotocol/sdk", "@mastra/core")
BUILDER = "export const createAcpSession = () => {};\n"


def write(root, relative, text):
    path = Path(root) / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def folder(root):
    """A folder shaped like a built one, with made-up contents."""
    write(root, "node", "not a real program\n")
    write(root, "agent/main.mjs", "// the entry\n")
    write(root, "agent.json", json.dumps({"target": TARGET}))
    write(root, "licenses/THIRD-PARTY-LICENSES.txt", "texts\n")
    write(root, "licenses/NODE-LICENSE.txt", "text\n")
    for name in PACKAGES:
        write(root, f"node_modules/{name}/package.json", json.dumps({"name": name, "version": "1.0.0", "license": "Apache-2.0"}))
    write(root, "node_modules/@mastra/code-sdk/dist/acp/runtime.js", BUILDER)
    write(root, "node_modules/@mastra/core/dist/ee-AbC123-xY_z9.js", "export { requireFGA as y };\n")
    subprocess.run(["node", str(REMOVE), str(Path(root) / "node_modules")], check=True, capture_output=True)
    return Path(root)


def pin(root):
    """What the made-up folder is checked against: its lock file and its session builder's checksum."""
    write(root, "package.json", json.dumps({"dependencies": {"@mastra/code-sdk": "1.0.0", "@agentclientprotocol/sdk": "1.0.0"}}))
    entries = {f"node_modules/{name}": {"version": "1.0.0", "resolved": f"https://registry.npmjs.org/{name}/-/x-1.0.0.tgz", "integrity": "sha512-AAAA"} for name in PACKAGES}
    write(root, "package-lock.json", json.dumps({"packages": {"": {}, **entries}}))
    write(root, "adapted-from.json", json.dumps({"package": "@mastra/code-sdk", "version": "1.0.0", "file": "dist/acp/runtime.js", "sha256": hashlib.sha256(BUILDER.encode()).hexdigest(), "adaptedIn": "agent/session.mjs"}))
    return Path(root)


class StageAgent(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp(prefix="stage-agent-test-"))
        self.built = folder(self.work / "built")
        self.pin = pin(self.work / "pin")
        self.dest = self.work / "package" / "mastra-agent"

    def tearDown(self):
        shutil.rmtree(self.work, ignore_errors=True)

    def stage(self, *extra, target=TARGET, guard="1", built=None):
        env = {key: value for key, value in os.environ.items() if key not in ("SOLIDCRAFT_MASTRA_AGENT_DIR", "SOLIDCRAFT_MASTRA_STAGE_TEST")}
        if guard:
            env["SOLIDCRAFT_MASTRA_STAGE_TEST"] = guard
        if built:
            env["SOLIDCRAFT_MASTRA_AGENT_DIR"] = built
        return subprocess.run(["bash", str(STAGE), "--target", target, "--dest", str(self.dest), "--pin", str(self.pin), *extra], capture_output=True, text=True, env=env, check=False)

    def test_a_folder_that_passes_is_copied_whole(self):
        done = self.stage("--from", str(self.built))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue((self.dest / "agent/main.mjs").is_file())
        self.assertTrue((self.dest / "node_modules/@mastra/core/package.json").is_file())

    def test_a_stored_sign_in_stops_it_and_nothing_is_copied(self):
        write(self.built, "node_modules/@mastra/core/auth.json", "{}\n")
        done = self.stage("--from", str(self.built))
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("nothing was staged", done.stderr)
        self.assertFalse(self.dest.exists())

    def test_enterprise_content_stops_it(self):
        write(self.built, "node_modules/@mastra/core/dist/auth/ee/index.js", "export {};\n")
        done = self.stage("--from", str(self.built))
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse(self.dest.exists())

    def test_a_folder_for_another_target_stops_it(self):
        done = self.stage("--from", str(self.built), target="aarch64-apple-darwin")
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse(self.dest.exists())

    def test_a_missing_folder_says_how_to_build_it(self):
        done = self.stage("--from", str(self.work / "none"))
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("agent-package.mjs build", done.stderr)
        self.assertFalse(self.dest.exists())

    def test_the_folder_is_taken_from_the_environment_when_not_named(self):
        done = self.stage(built=str(self.built))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue((self.dest / "agent.json").is_file())

    def test_another_lock_file_is_refused_outside_a_test(self):
        done = self.stage("--from", str(self.built), guard=None)
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("--pin is test-only", done.stderr)
        self.assertFalse(self.dest.exists())

    def test_a_package_the_lock_file_does_not_name_stops_it(self):
        write(self.built, "node_modules/slipped-in/package.json", json.dumps({"name": "slipped-in", "version": "1.0.0", "license": "MIT"}))
        done = self.stage("--from", str(self.built))
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse(self.dest.exists())


if __name__ == "__main__":
    unittest.main()
