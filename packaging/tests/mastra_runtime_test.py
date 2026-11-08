"""Mastra packaging gate: admission, approval-awareness, receipt binding and Codex non-inclusion.

These tests use original fixtures and a *test-only* admission command. They prove the packaging
path fails closed; they do not approve the real artifact (which remains unapproved, #15) and do not
claim an installed-package or clean-machine run. A synthetic approval exists only in a temporary
fixture policy.
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
STAGE = ROOT / "packaging/mastra/stage.sh"
VALIDATE = ROOT / "packaging/mastra/validate_runtime.py"
NODE = {"x86_64-pc-windows-msvc": "node.exe"}


def digest(path):
    return "sha256:" + hashlib.sha256(Path(path).read_bytes()).hexdigest()


def repo_value(relative):
    return (ROOT / relative).read_bytes()


def revision():
    return subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()


def runtime_version():
    return json.loads(repo_value("packages/mastra-runtime/package.json"))["version"]


def node_version():
    return (ROOT / ".nvmrc").read_text().strip()


def approval_for(target):
    return {
        "x86_64-unknown-linux-gnu": "mastra-runtime-linux-x64-v1",
        "x86_64-pc-windows-msvc": "mastra-runtime-windows-x64-v1",
        "aarch64-apple-darwin": "mastra-runtime-macos-arm64-v1",
        "x86_64-apple-darwin": "mastra-runtime-macos-x64-v1",
    }[target]


def make_runtime(directory, target="x86_64-unknown-linux-gnu", *, codex=False, ee=False, approve=True):
    runtime = Path(directory)
    runtime.mkdir(parents=True, exist_ok=True)
    files = {
        "dist/src/main.js": b"console.log('runtime');\n",
        "package.json": b'{"name":"runtime"}\n',
        "schema/mastra-runtime/v1/manifest.json": b"{}\n",
        "licenses/LICENSES.txt": b"licences\n",
        "notices/NOTICE.txt": b"notice\n",
        "node_modules/example/index.js": b"module\n",
    }
    node = NODE.get(target, "node")
    files[node] = b"#!/bin/sh\nexit 0\n"
    if codex:
        files["node_modules/codex-acp/cli.js"] = b"codex\n"
    if ee:
        files["node_modules/@mastra/core/ee/index.js"] = b"enterprise\n"
    for relative, body in files.items():
        path = runtime / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)
    inventory = runtime / "inventory.json"
    inventory.write_bytes(b"{}\n")
    inventory_digest = digest(inventory)
    if approve:
        policy_path = runtime.parent / f"policy-{target}.json"
        policy = json.loads(repo_value("packaging/mastra/policy.json"))
        policy["approvals"] = [{"id": approval_for(target), "target": target, "inventoryDigest": inventory_digest}]
        policy_path.write_text(json.dumps(policy, indent=2) + "\n")
    else:
        policy_path = ROOT / "packaging/mastra/policy.json"
    artifact = [{"path": relative, "digest": digest(runtime / relative)} for relative in sorted(files)]
    receipt = {
        "schemaVersion": "1.0",
        "runtimeVersion": runtime_version(),
        "protocolVersions": ["1.0"],
        "targetTriple": target,
        "nodeVersion": node_version(),
        "repositoryRevision": revision(),
        "contractDigest": "sha256:" + hashlib.sha256(repo_value("schema/mastra-runtime/v1/manifest.json")).hexdigest(),
        "policyDigest": digest(policy_path),
        "sourceRecordsDigest": "sha256:" + hashlib.sha256(repo_value("packaging/mastra/source-records.json")).hexdigest(),
        "provenanceInventory": {"path": "inventory.json", "digest": inventory_digest},
        "artifactDigests": artifact,
    }
    (runtime / "runtime.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return runtime, policy_path


def validate(runtime, policy_path, target="x86_64-unknown-linux-gnu"):
    command = [
        "python3",
        "-I",
        str(VALIDATE),
        "--runtime",
        str(runtime),
        "--target",
        target,
        "--policy",
        str(policy_path),
        "--approval",
        approval_for(target),
        "--revision",
        revision(),
        "--contract-digest",
        "sha256:" + hashlib.sha256(repo_value("schema/mastra-runtime/v1/manifest.json")).hexdigest(),
        "--policy-digest",
        digest(policy_path),
        "--sources-digest",
        "sha256:" + hashlib.sha256(repo_value("packaging/mastra/source-records.json")).hexdigest(),
        "--runtime-version",
        runtime_version(),
        "--node-version",
        node_version(),
    ]
    return subprocess.run(command, capture_output=True, text=True)


class RuntimeValidation(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_a_complete_matching_tree_passes_only_with_a_matching_approval(self):
        runtime, policy = make_runtime(self.tmp / "runtime")
        result = validate(runtime, policy)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_an_unapproved_inventory_is_rejected_even_if_the_tree_is_complete(self):
        runtime, policy = make_runtime(self.tmp / "runtime", approve=False)
        result = validate(runtime, policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("has no matching approval", result.stderr)
        self.assertIn("#15", result.stderr)

    def test_every_supported_target_validates_with_its_own_approval(self):
        for target in ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "aarch64-apple-darwin", "x86_64-apple-darwin"):
            runtime, policy = make_runtime(self.tmp / target, target)
            result = validate(runtime, policy, target)
            self.assertEqual(result.returncode, 0, f"{target}: {result.stderr}")

    def test_missing_entry_point_is_incomplete(self):
        runtime, policy = make_runtime(self.tmp / "runtime")
        (runtime / "dist/src/main.js").unlink()
        result = validate(runtime, policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("dist/src/main.js is missing", result.stderr)

    def test_a_different_revision_fails_closed(self):
        runtime, policy = make_runtime(self.tmp / "runtime")
        receipt = json.loads((runtime / "runtime.json").read_text())
        receipt["repositoryRevision"] = "0" * 40
        (runtime / "runtime.json").write_text(json.dumps(receipt))
        result = validate(runtime, policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("repositoryRevision", result.stderr)

    def test_codex_is_never_bundled(self):
        runtime, policy = make_runtime(self.tmp / "runtime", codex=True)
        result = validate(runtime, policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Codex", result.stderr)
        self.assertIn("discovered, not bundled", result.stderr)

    def test_enterprise_paths_are_rejected_even_after_copy(self):
        runtime, policy = make_runtime(self.tmp / "runtime", ee=True)
        result = validate(runtime, policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ee/", result.stderr)


class StageGate(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.checker = self.tmp / "admission"
        self.dest = self.tmp / "package/usr/lib/solidcraft/mastra"
        self.inputs = self.tmp / "inputs"
        self.inputs.mkdir()

    def run_stage(self, artifact, target="x86_64-unknown-linux-gnu", *, exit_code=0, guard=True, policy=None):
        self.checker.write_text(f"#!/bin/sh\nexit {exit_code}\n")
        self.checker.chmod(0o755)
        env = {
            **os.environ,
            "SOLIDCRAFT_RUNTIME_ARTIFACT_CHECK": str(self.checker),
            "SOLIDCRAFT_MASTRA_INPUTS": str(self.inputs),
        }
        if guard:
            env["SOLIDCRAFT_MASTRA_STAGE_TEST"] = "1"
        command = [str(STAGE), "--target", target, "--artifact", str(artifact), "--dest", str(self.dest)]
        if policy is not None:
            command += ["--policy", str(policy)]
        return subprocess.run(command, env=env, capture_output=True, text=True)

    def test_no_artifact_fails_before_copying(self):
        result = subprocess.run(
            [str(STAGE), "--target", "x86_64-unknown-linux-gnu", "--artifact", str(self.tmp / "absent"), "--dest", str(self.dest)],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("packaging fails closed", result.stderr)
        self.assertIn("#15", result.stderr)
        self.assertFalse(self.dest.exists())

    def test_the_fake_gate_is_refused_without_the_test_guard(self):
        runtime, policy = make_runtime(self.tmp / "runtime")
        result = self.run_stage(runtime, guard=False, policy=policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("test-only seam", result.stderr)
        self.assertFalse(self.dest.exists())

    def test_an_admission_failure_is_not_bypassed(self):
        runtime, policy = make_runtime(self.tmp / "runtime")
        result = self.run_stage(runtime, exit_code=1, policy=policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("was not admitted", result.stderr)
        self.assertFalse(self.dest.exists(), "an unapproved artifact must not be copied")

    def test_an_unapproved_artifact_cannot_be_staged_even_with_a_fake_gate(self):
        runtime, policy = make_runtime(self.tmp / "runtime", approve=False)
        result = self.run_stage(runtime, policy=policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("has no matching approval", result.stderr)
        self.assertFalse(self.dest.exists(), "the validator must refuse an unapproved inventory")

    def test_an_admitted_tree_is_staged_application_relative(self):
        runtime, policy = make_runtime(self.tmp / "runtime")
        result = self.run_stage(runtime, policy=policy)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.dest / "runtime.json").is_file())
        self.assertTrue((self.dest / "node").is_file())
        self.assertTrue((self.dest / "node_modules/example/index.js").is_file())

    def test_a_codex_file_is_rejected_at_the_staging_boundary(self):
        runtime, policy = make_runtime(self.tmp / "runtime", codex=True)
        result = self.run_stage(runtime, policy=policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Codex", result.stderr)
        self.assertFalse(self.dest.exists(), "no package tree may contain a bundled Codex")

    def test_unsupported_targets_are_rejected(self):
        runtime, policy = make_runtime(self.tmp / "runtime")
        result = self.run_stage(runtime, target="aarch64-unknown-linux-gnu", policy=policy)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not ship for", result.stderr)


class PackagingOptIn(unittest.TestCase):
    def test_ordinary_packaging_is_off_by_default(self):
        # The release workflow pins the default off; each package script keeps an opt-in switch.
        release = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn('SOLIDCRAFT_WITH_MASTRA: "0"', release)
        for relative in ("packaging/linux/package.sh", "packaging/macos/package.sh"):
            script = (ROOT / relative).read_text()
            self.assertIn("SOLIDCRAFT_WITH_MASTRA:-0", script, relative)
            self.assertIn("--with-mastra", script, relative)
            self.assertRegex(script, r'if \[ "\$WITH_MASTRA" = 1 \]; then', relative)
        windows = (ROOT / "packaging/windows/package.ps1").read_text()
        self.assertIn("[switch] $WithMastra", windows)
        self.assertIn("if ($WithMastra)", windows)


if __name__ == "__main__":
    unittest.main()
