#!/usr/bin/env python3
"""Tests for the native-package content decision (#14/#15 applicability)."""

import base64
import itertools
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

try:
    import yaml
except ModuleNotFoundError:
    yaml = None  # The stdlib-only repository lane also runs this file.

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
import release_content as content  # noqa: E402

SHA = "a" * 40
DG = "b" * 64
DG2 = "c" * 64

CONFIG = {
    "INTCAD_WORKOS_PUBLIC_CLIENT_ID": "client_Fixture42",
    "INTCAD_AUTHKIT_VERIFICATION_ORIGIN": "https://auth.example.invalid:8443",
    "INTCAD_PRODUCT_HTTPS_ORIGIN": "https://product.example.invalid",
}
CLIENT, AUTHKIT, PRODUCT = CONFIG
SNAPSHOT = "STAGING_ACCOUNT_CONFIG"
HELPER = HERE / "staging_account_config.py"
WORKFLOWS = ROOT / ".github/workflows"
CONFIG_PATH = ".github/config/intcad-staging-account.json"


def config_checkout(root, payload=None):
    subprocess.run(["git", "init", "-q", root], check=True)
    if payload is not None:
        path = root / CONFIG_PATH
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
        subprocess.run(["git", "add", CONFIG_PATH], cwd=root, check=True)
    subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                    "commit", "-q", "--allow-empty", "-m", "package fixture"], cwd=root, check=True)
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()


def workflow(name):
    # BaseLoader preserves the Actions `on` key instead of interpreting it as YAML 1.1 `true`.
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


@unittest.skipIf(yaml is None, "workflow wiring needs PyYAML (required by packaging lint)")
class WindowsAssistantSmokeWorkflow(unittest.TestCase):
    def test_packaging_lint_runs_the_windows_assistant_smoke_unittest(self):
        steps = workflow("packaging-lint.yml")["jobs"]["lint"]["steps"]
        command = "python3 -m unittest scripts/e2e/test_windows_assistant_uia.py"
        self.assertTrue(
            any(step.get("run", "").strip() == command and "if" not in step
                and step.get("continue-on-error", "false") == "false" for step in steps),
            f"Packaging lint must run {command} without skipping or ignoring failures",
        )

    def test_windows_assistant_smoke_changes_trigger_packaging_lint(self):
        events = workflow("packaging-lint.yml")["on"]
        for event in ("pull_request", "push"):
            for path in ("tools/windows-assistant-smoke.ps1",
                         "scripts/e2e/test_windows_assistant_uia.py",
                         "scripts/e2e/windows_assistant_uia.py"):
                with self.subTest(event=event, path=path):
                    self.assertIn(path, events[event]["paths"])


class StagingConfig(unittest.TestCase):
    def setUp(self):
        self.assertTrue(HELPER.is_file(), "missing staging config helper")
        from staging_account_config import validate_config
        self.validate = validate_config

    def test_only_the_complete_tuple_is_allowed(self):
        for flags in itertools.product((False, True), repeat=3):
            values = {key: CONFIG[key] for key, present in zip(CONFIG, flags) if present}
            with self.subTest(flags=flags):
                if all(flags):
                    self.assertEqual(self.validate(values), CONFIG)
                else:
                    with self.assertRaises(ValueError) as rejected:
                        self.validate(values)
                    for key in CONFIG:
                        self.assertEqual(key in str(rejected.exception), key not in values)
        with self.assertRaises(ValueError):
            self.validate({**CONFIG, "EXTRA": "unapproved"})

    def test_malformed_client_ids_are_rejected_without_disclosure(self):
        for value in ("", "client_", "other_123", "client_abc ", "client_\u00e9",
                      "client_abc\tdef", "client_abc\x00", "client_abc\nOTHER=bad",
                      "client_abc;echo", "client_$(echo)", "client_abc_def"):
            with self.subTest(value=value):
                with self.assertRaises(ValueError) as rejected:
                    self.validate({**CONFIG, CLIENT: value})
                self.assertIn(CLIENT, str(rejected.exception))
                if value:
                    self.assertNotIn(value, str(rejected.exception))

    def test_only_bare_https_origins_are_allowed(self):
        for key in (AUTHKIT, PRODUCT):
            for value in ("http://bad.invalid", "https://user:pw@bad.invalid",
                          "https://bad.invalid?", "https://bad.invalid#fragment",
                          "https://bad.invalid/", "https://bad.invalid/v1",
                          "https://bad.invalid:", "https://bad.invalid:65536",
                          "https://bad.invalid:0", "https://bad.invalid:abc",
                          "https://", "https://bad.invalid\\foo", "https://bad.\tinvalid",
                          "https://bad.invalid\nOTHER=bad"):
                with self.subTest(key=key, value=value):
                    with self.assertRaises(ValueError) as rejected:
                        self.validate({**CONFIG, key: value})
                    self.assertIn(key, str(rejected.exception))
                    self.assertNotIn(value, str(rejected.exception))

    def test_snapshot_then_inject_preserves_all_values(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            sha = config_checkout(root, json.dumps(CONFIG).encode())
            output, env_file = root / "output", root / "env"
            env = {**os.environ, **dict.fromkeys(CONFIG, "ignored-ambient-value"),
                   "GITHUB_SHA": sha, "GITHUB_OUTPUT": str(output)}
            result = subprocess.run([sys.executable, "-I", HELPER, "snapshot"], cwd=root,
                                    env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            snapshot = output.read_text()
            self.assertEqual(snapshot, "config=" + json.dumps(CONFIG, separators=(",", ":")) + "\n")
            subprocess.run([sys.executable, "-I", HELPER, "inject"],
                           env={**os.environ, SNAPSHOT: snapshot.removeprefix("config=").rstrip("\n"),
                                "GITHUB_ENV": str(env_file)}, check=True)
            injected = dict(line.split("=", 1) for line in env_file.read_text().splitlines())
            tuple_sha = __import__("staging_account_config").tuple_sha256(CONFIG)
            self.assertEqual(injected, {**CONFIG, "ACCOUNT_CONFIG_SOURCE": "checked-in:staging",
                                        "ACCOUNT_CONFIG_SHA256": tuple_sha})

    def test_snapshot_rejects_bad_committed_config_without_output(self):
        sentinel = "concealed-fixture-value"
        for payload in (None, b"\xff", ("{" + sentinel).encode(),
                        json.dumps({**CONFIG, "EXTRA": sentinel}).encode()):
            with self.subTest(payload=payload), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                sha = config_checkout(root, payload)
                output = root / "output"
                result = subprocess.run([sys.executable, "-I", HELPER, "snapshot"], cwd=root,
                                        env={**os.environ, **CONFIG, "GITHUB_SHA": sha,
                                             "GITHUB_OUTPUT": str(output)}, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(output.exists())
                self.assertNotIn(sentinel, result.stdout + result.stderr)
                self.assertNotIn("Traceback", result.stderr)

    def test_snapshot_refuses_wrong_sha_or_dirty_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            sha = config_checkout(root, json.dumps(CONFIG).encode())
            output = root / "output"
            for revision, dirty in (("0" * 40, False), (sha, True)):
                with self.subTest(dirty=dirty):
                    if dirty:
                        (root / CONFIG_PATH).write_text(json.dumps({**CONFIG, CLIENT: "client_Changed"}))
                    result = subprocess.run([sys.executable, "-I", HELPER, "snapshot"], cwd=root,
                                            env={**os.environ, **CONFIG, "GITHUB_SHA": revision,
                                                 "GITHUB_OUTPUT": str(output)}, capture_output=True, text=True)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(output.exists())

    def test_invalid_input_never_creates_an_env_file_or_discloses_values(self):
        sentinel = "concealed-fixture-value"
        bad_inputs = ("{" + sentinel, json.dumps({**CONFIG, PRODUCT: sentinel}),
                      json.dumps({sentinel: sentinel}), json.dumps([sentinel]),
                      json.dumps(CONFIG)[:-1] + f', "{CLIENT}": "{sentinel}"' + "}")
        with tempfile.TemporaryDirectory() as tmp:
            env_file = Path(tmp) / "env"
            for payload in bad_inputs:
                with self.subTest(payload=payload):
                    result = subprocess.run([sys.executable, "-I", HELPER, "inject"],
                                            env={**os.environ, SNAPSHOT: payload, "GITHUB_ENV": str(env_file)},
                                            capture_output=True, text=True)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(env_file.exists())
                    self.assertNotIn(sentinel, result.stdout + result.stderr)
                    for value in CONFIG.values():
                        self.assertNotIn(value, result.stdout + result.stderr)


@unittest.skipIf(yaml is None, "workflow wiring needs PyYAML (required by packaging lint)")
class StagingWorkflow(unittest.TestCase):
    def test_only_the_environment_bound_dispatch_wrapper_passes_the_snapshot(self):
        filename = "staging-packages.yml"
        self.assertTrue((WORKFLOWS / filename).is_file(), "missing staging entry")
        wrapper = workflow(filename)
        self.assertEqual(set(wrapper["on"]), {"workflow_dispatch"})
        self.assertEqual(wrapper["permissions"], {"contents": "read"})
        snapshot = wrapper["jobs"]["snapshot"]
        packages = wrapper["jobs"]["packages"]
        self.assertEqual(snapshot["environment"], "staging")
        self.assertTrue(any(step.get("id") == "config" for step in snapshot["steps"]))
        self.assertEqual(snapshot["outputs"]["config"], "${{ steps.config.outputs.config }}")
        self.assertEqual(packages, {
            "needs": "snapshot",
            "uses": "./.github/workflows/release.yml",
            "with": {"beta_three_platforms": "true",
                     "staging_account_config": "${{ needs.snapshot.outputs.config }}"},
        })
        config_input = workflow("release.yml")["on"]["workflow_call"]["inputs"]["staging_account_config"]
        self.assertEqual((config_input["type"], config_input["default"]), ("string", ""))
        self.assertNotIn("staging_account_config", (WORKFLOWS / "release-gate.yml").read_text())
        for name in (filename, "release.yml"):
            text = (WORKFLOWS / name).read_text()
            for forbidden in ("refs/heads/", "${{ secrets.", "secrets: inherit", "contents: write",
                              "gh release ", "git push", "softprops/action-gh-release", "--notarize"):
                self.assertNotIn(forbidden, text)

    def test_wrapper_reads_only_from_the_fixed_checkout(self):
        wrapper = workflow("staging-packages.yml")
        self.assertEqual(set(wrapper["jobs"]), {"snapshot", "packages"})
        self.assertNotIn("vars", json.dumps(wrapper))
        checkout, snapshot = wrapper["jobs"]["snapshot"]["steps"]
        self.assertTrue(checkout["uses"].startswith("actions/checkout@"))
        self.assertEqual(checkout["with"], {"ref": "${{ github.sha }}", "persist-credentials": "false"})
        self.assertNotIn("env", snapshot)
        self.assertEqual(snapshot["run"], "python -I scripts/ci/staging_account_config.py snapshot")

    def test_release_verifies_staging_input_against_checkout_before_builds(self):
        steps = workflow("release.yml")["jobs"]["version"]["steps"]
        verify = next((step for step in steps if step.get("id") == "verify-staging-config"), None)
        self.assertIsNotNone(verify, "missing early staging snapshot verification")
        checkout = next(i for i, step in enumerate(steps) if step.get("uses", "").startswith("actions/checkout@"))
        self.assertGreater(steps.index(verify), checkout)
        self.assertEqual(verify["if"], "inputs.staging_account_config != ''")
        self.assertEqual(verify["env"], {SNAPSHOT: "${{ inputs.staging_account_config }}"})
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            sha = config_checkout(root, json.dumps(CONFIG).encode())
            scripts = root / "scripts/ci"
            scripts.mkdir(parents=True)
            shutil.copyfile(HELPER, scripts / HELPER.name)
            self.assertEqual(verify["run"], "python -I scripts/ci/staging_account_config.py verify")
            for values, expected in ((CONFIG, 0), ({**CONFIG, CLIENT: "client_Drift"}, 1)):
                result = subprocess.run([sys.executable, *verify["run"].split()[1:]],
                                        cwd=root, env={**os.environ, "GITHUB_SHA": sha, SNAPSHOT: json.dumps(values)},
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, expected, result.stderr)

    def test_account_config_guard_rejects_other_callers_and_anonymous_candidates(self):
        version = workflow("release.yml")["jobs"]["version"]
        step = next((step for step in version["steps"]
                     if step.get("id") == "account-config-guard"), None)
        self.assertIsNotNone(step, "missing account configuration caller guard")
        # The guard must run for every build: a skipped step let the staging wrapper build
        # anonymously when its snapshot was empty.
        self.assertNotIn("if", step)
        self.assertEqual(step["env"], {
            "CALLER_WORKFLOW_REF": "${{ github.workflow_ref }}",
            "STAGING_WORKFLOW_REF": "${{ format('{0}/.github/workflows/staging-packages.yml@{1}', github.repository, github.ref) }}",
            "RELEASE_GATE_WORKFLOW_REF": "${{ format('{0}/.github/workflows/release-gate.yml@{1}', github.repository, github.ref) }}",
            "HAS_STAGING_CONFIG": "${{ inputs.staging_account_config != '' && '1' || '0' }}",
            "HAS_PRODUCTION_CONFIG": "${{ inputs.production_account_config != '' && '1' || '0' }}",
        })
        staging = "fixture/repo/.github/workflows/staging-packages.yml@refs/heads/fixture"
        gate = "fixture/repo/.github/workflows/release-gate.yml@refs/heads/fixture"
        for caller, event, staging_config, production_config, ok in (
            (staging, "workflow_dispatch", "1", "0", True),
            (staging, "workflow_dispatch", "0", "0", False),
            (staging, "pull_request", "1", "0", False),
            (staging, "workflow_dispatch", "1", "1", False),
            (gate, "pull_request", "0", "1", True),
            (gate, "pull_request", "0", "0", False),
            (gate, "pull_request", "1", "0", False),
            (gate.replace("release-gate", "release"), "pull_request", "0", "1", False),
            (staging.replace("fixture/repo", "other/repo"), "workflow_dispatch", "1", "0", False),
            (staging + "-other", "workflow_dispatch", "1", "0", False),
            ("fixture/repo/.github/workflows/ci.yml@refs/heads/fixture", "push", "0", "0", True),
        ):
            with self.subTest(caller=caller, event=event, staging=staging_config, production=production_config):
                result = subprocess.run(
                    ["bash", "-ec", step["run"]],
                    env={**os.environ, "CALLER_WORKFLOW_REF": caller, "STAGING_WORKFLOW_REF": staging,
                         "RELEASE_GATE_WORKFLOW_REF": gate, "GITHUB_EVENT_NAME": event,
                         "HAS_STAGING_CONFIG": staging_config, "HAS_PRODUCTION_CONFIG": production_config},
                    capture_output=True,
                )
                self.assertEqual(result.returncode == 0, ok)

    def test_only_staging_reduces_the_linux_and_windows_matrices(self):
        jobs = workflow("release.yml")["jobs"]
        matrices = {
            "linux": [{"arch": "x86_64", "runner": "ubuntu-22.04", "deb": "amd64"},
                      {"arch": "aarch64", "runner": "ubuntu-22.04-arm", "deb": "arm64"}],
            "windows": [{"arch": "x64", "target": "x86_64-pc-windows-msvc"},
                        {"arch": "x86", "target": "i686-pc-windows-msvc"},
                        {"arch": "arm64", "target": "aarch64-pc-windows-msvc"}],
        }
        for name, ordinary in matrices.items():
            with self.subTest(job=name):
                include = jobs[name]["strategy"]["matrix"]["include"]
                self.assertIsInstance(include, str, "missing staging matrix selection")
                choices = re.fullmatch(
                    r"\$\{\{ fromJSON\(inputs\.staging_account_config != '' && '([^']+)' \|\| '([^']+)'\) \}\}",
                    " ".join(include.split()),
                )
                self.assertIsNotNone(choices)
                self.assertEqual(json.loads(choices[1]), ordinary[:1])
                self.assertEqual(json.loads(choices[2]), ordinary)
        self.assertEqual(jobs["macos-build"]["strategy"]["matrix"]["include"], [
            {"arch": "aarch64", "target": "aarch64-apple-darwin"},
            {"arch": "x86_64", "target": "x86_64-apple-darwin"},
        ])
        for name in ("flatpak", "arch", "snap", "freebsd", "web"):
            self.assertEqual(jobs[name]["if"], "inputs.beta_three_platforms != true")

    def test_staging_summary_requires_all_staging_jobs_but_not_skipped_other_platforms(self):
        summary = workflow("release.yml")["jobs"]["package-builds"]
        self.assertEqual(summary["if"],
                         "always() && (inputs.staging_account_config == '' || needs.version.result == 'success')")
        step = summary["steps"][0]
        self.assertEqual(step["env"]["STAGING_PACKAGES"],
                         "${{ inputs.staging_account_config != '' && '1' || '0' }}")
        jobs = {name: {"result": "success"} for name in ("version", "icons", "linux", "macos-build", "macos", "windows")}
        jobs.update({name: {"result": "skipped"} for name in ("flatpak", "arch", "snap", "freebsd", "web")})
        for failed, expected in ((None, 0), ("linux", 1), ("macos-build", 1), ("windows", 1)):
            with self.subTest(failed=failed):
                needs = jobs if not failed else {**jobs, failed: {"result": "failure"}}
                result = subprocess.run(["bash", "-ec", step["run"]],
                                        env={**os.environ, "STAGING_PACKAGES": "1", "NEEDS_JSON": json.dumps(needs)},
                                        capture_output=True)
                self.assertEqual(result.returncode, expected, result.stderr)

    def test_snapshot_is_injected_before_compile_but_not_in_the_macos_join(self):
        jobs = workflow("release.yml")["jobs"]
        snapshot_env = {SNAPSHOT: "${{ inputs.staging_account_config }}"}
        for job, command in {"linux": "packaging/linux/package.sh",
                             "windows": "./packaging/windows/package.ps1",
                             "macos-build": "packaging/macos/package.sh --build-only"}.items():
            with self.subTest(job=job):
                steps = jobs[job]["steps"]
                compile_step = next(i for i, step in enumerate(steps) if step.get("run", "").startswith(command))
                inject = steps[compile_step - 1]
                self.assertEqual(inject["env"], {**snapshot_env,
                                                 "PRODUCTION_ACCOUNT_CONFIG": "${{ inputs.production_account_config }}"})
                self.assertEqual(inject["run"],
                                 "python -I scripts/ci/staging_account_config.py inject-auto")
                self.assertEqual(inject["if"],
                                 "inputs.staging_account_config != '' || inputs.production_account_config != ''")
        join = jobs["macos"]
        self.assertFalse(set(join["env"]) & (set(CONFIG) | {SNAPSHOT}))
        uses_snapshot = [step for step in join["steps"] if SNAPSHOT in step.get("env", {})]
        self.assertEqual(len(uses_snapshot), 1)
        self.assertEqual(uses_snapshot[0]["env"], snapshot_env)
        self.assertIn("build.json", uses_snapshot[0]["run"])
        self.assertTrue(any("--arch universal --skip-build" in step.get("run", "") for step in join["steps"]))

    def test_staging_artifacts_are_prefixed_and_keep_ordinary_labels(self):
        release = workflow("release.yml")
        self.assertEqual(release["env"].get("ARTIFACT_PREFIX"),
                         "${{ inputs.staging_account_config != '' && 'staging-' || '' }}")
        jobs = release["jobs"]
        labels = {
            "icons": ["icons-${{ github.sha }}"],
            "linux": ["installed-check-linux-${{ matrix.arch }}"],
            "macos-build": ["macos-bin-${{ matrix.arch }}-${{ github.sha }}"],
            "macos": ["macos-bin-aarch64-${{ github.sha }}", "macos-bin-x86_64-${{ github.sha }}",
                      "installed-check-macos"],
            "windows": ["icons-${{ github.sha }}", "installed-check-windows-${{ matrix.arch }}"],
        }
        # A gate-bound run appends its attempt so parallel attempts cannot overwrite each other.
        attempt = "${{ inputs.gate_bound && format('-a{0}', github.run_attempt) || '' }}"
        for job, ordinary in labels.items():
            with self.subTest(job=job):
                if job in ("linux", "macos", "windows"):
                    ordinary += ["${{ env.PACKAGE_ID }}", "package-evidence-${{ env.PACKAGE_ID }}"]
                    suffix = "" if job == "macos" else "-${{ matrix.arch }}"
                    self.assertEqual(jobs[job]["env"]["PACKAGE_ID"], f"release-{job}{suffix}")
                actual = [step["with"]["name"] for step in jobs[job]["steps"]
                          if step.get("uses", "").startswith(("actions/upload-artifact@", "actions/download-artifact@"))]
                self.assertEqual(actual, ["${{ env.ARTIFACT_PREFIX }}" + label + attempt for label in ordinary])

    def test_staging_evidence_hashes_the_package_and_revalidates_the_same_snapshot(self):
        job = workflow("release.yml")["jobs"]["macos"]
        step = next((step for step in job["steps"] if SNAPSHOT in step.get("env", {})), None)
        self.assertIsNotNone(step, "missing staging evidence input")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            sha = config_checkout(root, json.dumps(CONFIG).encode())
            directory = root / "dist/release"
            directory.mkdir(parents=True)
            # A real, inspectable archive: the package-evidence step judges the built bytes
            # through release_content, so a placeholder blob would be "not inspected".
            (directory / "fixture.tar.gz").write_bytes(
                base64.b64decode("H4sIAAAAAAAC/+3RMQrCQBAF0Kk9hSeQWQy5TwoDaeMGcnw32oT0CsH3mj/85hczTmtd5setrjW+JZu+697ZHDOz7O6tL1n6e1wzfmB51mFu8/Gfxs//LwEAAAAAAAAAAMCpvACQNW+CACgAAA=="))

            scripts = root / "scripts/ci"
            scripts.mkdir(parents=True)
            for source in (HELPER, HERE / "release_content.py", HERE / "package_contents.py"):
                shutil.copyfile(source, scripts / source.name)
            env = {**os.environ, "GITHUB_SHA": sha, "PACKAGE_ID": "release-macos",
                   "PACKAGE_TARGET": "universal-apple-darwin"}
            expected = {
                "version": 1, "packageId": "release-macos", "sourceRevision": sha,
                "target": "universal-apple-darwin", "contractDigest": None,
                "artifacts": [{"name": "fixture.tar.gz",
                               "sha256": "f9e928f22757d899d5b89a120796cf4a1c82a3df7a809da4c05fd7fbfb00f8e7"}],
            }
            descriptor = directory / "build.json"
            command = [sys.executable, "-c", step["run"]]
            tuple_sha = __import__("staging_account_config").tuple_sha256(CONFIG)
            for source, snapshot, fields in (
                ("", "", expected),
                ("checked-in:staging", json.dumps(CONFIG), {
                    **expected, "accountConfig": {"source": "checked-in:staging", "sourceRevision": sha,
                                                  "path": CONFIG_PATH, "values": CONFIG},
                }),
                ("environment:production", "", {
                    **expected, "accountConfig": {"source": "environment:production",
                                                  "sourceRevision": sha, "tupleSha256": tuple_sha},
                }),
            ):
                result = subprocess.run(command, cwd=root, env={
                    **env, SNAPSHOT: snapshot, "PRODUCTION_ACCOUNT_CONFIG": "",
                    "ACCOUNT_CONFIG_SOURCE": source, "ACCOUNT_CONFIG_SHA256": tuple_sha if source else "",
                }, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(descriptor.read_text()), fields)
                descriptor.unlink()
            result = subprocess.run(command, cwd=root, env={**env, SNAPSHOT: "{}",
                                    "ACCOUNT_CONFIG_SOURCE": "checked-in:staging"},
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(descriptor.exists())
            result = subprocess.run([sys.executable, "-c", step["run"]], cwd=root,
                                    env={**env, SNAPSHOT: json.dumps({**CONFIG, CLIENT: "client_Drift"}),
                                         "ACCOUNT_CONFIG_SOURCE": "checked-in:staging"},
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(descriptor.exists())
            # Ordinary runs must not read the committed staging config at all.
            new_sha = config_checkout(root, b"not json")
            result = subprocess.run([sys.executable, "-c", step["run"]], cwd=root,
                                    env={**env, "GITHUB_SHA": new_sha, SNAPSHOT: "",
                                         "ACCOUNT_CONFIG_SOURCE": ""},
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(descriptor.read_text()), {**expected, "sourceRevision": new_sha})


def build(identity, artifacts):
    return {
        "packageId": identity,
        "sourceRevision": SHA,
        "artifacts": [{"name": name, "sha256": digest} for name, digest in artifacts],
    }


def covered(name, sha, source):
    return {"name": name, "sha256": sha, "format": "covered", "entries": 0, "coveredBy": source}


def report(identity, checked, *, distribution="native", markers=(), unchecked=(), ignored=(), derived=None,
           source_name=None, source_sha=None, agent=None, agent_markers=(), agents=None):
    value = {
        "packageId": identity,
        "sourceRevision": SHA,
        "distribution": distribution,
        "checked": [{"name": name, "sha256": digest, "format": "tar", "entries": 10} for name, digest in checked],
        "ignored": list(ignored),
        "unchecked": [{"name": name, "reason": "no inspector"} for name in unchecked],
        "markers": [{"name": "pkg", "path": path} for path in markers],
        "agentMarkers": [{"name": "pkg", "path": path} for path in agent_markers],
        "agent": agent,
        # Every artifact's own answer, as `package_contents.py` records it.
        "agents": agents if agents is not None else (
            {name: agent for name, _ in checked} if agent is not None else {}),
        "derivedFrom": derived,
        "sourceArtifact": name_of(source_name),
        "sourceSha256": source_sha,
    }
    return value


def name_of(value):
    return value


WHOLE_AGENT = {"present": True, "node": True, "nodeTargets": [], "missing": [],
               "skills": ["modelling-skills/solidcraft-modeling"]}
MAC_AGENT = {"present": True, "node": True, "missing": [],
             "nodeTargets": ["aarch64-apple-darwin", "x86_64-apple-darwin"],
             "skills": ["modelling-skills/solidcraft-modeling"]}


class NativeContentDecision(unittest.TestCase):
    def test_native_installers_with_inspected_bytes_do_not_require_mastra(self):
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG), ("solidcraft.deb", DG2)]),
            "release-windows-x64": build("release-windows-x64", [("solidcraft.msi", DG)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG), ("solidcraft.deb", DG2)]),
            "release-windows-x64": report("release-windows-x64", [("solidcraft.msi", DG)]),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents), [])

    def test_runtime_markers_fail_the_native_declaration(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)],
                                                   markers=("usr/lib/intcad/mastra/runtime/node",))}
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("mastra", problems[0])
        self.assertIn("#15", problems[0])

    def test_uninspected_artifact_fails(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [], unchecked=("solidcraft.tar.gz",))}
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("content not inspected", problems[0])

    def test_missing_content_evidence_fails(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        problems = content.problems(SHA, descriptors, {})
        self.assertTrue(problems)
        self.assertIn("missing package content evidence", problems[0])

    def test_digest_mismatch_fails(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG2)])}
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("digest mismatch", problems[0])

    def test_derived_package_must_name_an_inspected_native_source(self):
        # A derived package follows the source's own distribution: a native source here.
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": build("release-flatpak-x86_64", [("solidcraft.flatpak", DG2)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": report("release-flatpak-x86_64", [("solidcraft.flatpak", DG2)],
                                             derived="release-linux-x86_64",
                                             source_name="solidcraft.tar.gz", source_sha=DG),
        }
        contents["release-flatpak-x86_64"]["checked"] = [covered("solidcraft.flatpak", DG2, "release-linux-x86_64")]
        self.assertEqual(content.problems(SHA, descriptors, contents, "native"), [])
        # A wrapper with no coverage is not evidence.
        contents["release-flatpak-x86_64"]["checked"] = []
        self.assertIn("does not cover", content.problems(SHA, descriptors, contents, "native")[0])
        # Coverage claimed by anything but the inspected source is refused.
        contents["release-flatpak-x86_64"]["checked"] = [covered("solidcraft.flatpak", DG2, "release-macos")]
        self.assertIn("claims coverage by", content.problems(SHA, descriptors, contents, "native")[0])
        contents["release-flatpak-x86_64"]["checked"] = [covered("solidcraft.flatpak", DG2, "release-linux-x86_64")]
        contents["release-flatpak-x86_64"]["derivedFrom"] = "release-macos"
        self.assertTrue(content.problems(SHA, descriptors, contents, "native"))

    def test_a_derived_package_needs_a_cleaned_source_scan(self):
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": build("release-flatpak-x86_64", [("solidcraft.flatpak", DG2)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": report("release-flatpak-x86_64", [("solidcraft.flatpak", DG2)],
                                             derived="release-linux-x86_64",
                                             source_name="solidcraft.tar.gz", source_sha=DG),
        }
        contents["release-flatpak-x86_64"]["checked"] = [covered("solidcraft.flatpak", DG2, "release-linux-x86_64")]
        self.assertEqual(content.problems(SHA, descriptors, contents), [])
        for field, value in (("markers", ["usr/lib/intcad/mastra"]), ("unchecked", ["solidcraft.tar.gz"])):
            contents["release-flatpak-x86_64"] = report(
                "release-flatpak-x86_64", [("solidcraft.flatpak", DG2)], derived="release-linux-x86_64",
                source_name="solidcraft.tar.gz", source_sha=DG, **{field: value})
            contents["release-flatpak-x86_64"]["checked"] = [covered("solidcraft.flatpak", DG2, "release-linux-x86_64")]
            problems = content.problems(SHA, descriptors, contents)
            self.assertTrue(problems, field)
            self.assertIn("derived package", problems[0])

    def test_a_covered_standalone_exe_needs_its_covering_package(self):
        descriptors = {"release-windows-x64": build("release-windows-x64", [("solidcraft.zip", DG), ("SolidCraft.exe", DG2)])}
        contents = {
            "release-windows-x64": report("release-windows-x64", [("solidcraft.zip", DG), ("SolidCraft.exe", DG2)]),
        }
        contents["release-windows-x64"]["checked"][1]["coveredBy"] = "solidcraft.zip"
        self.assertEqual(content.problems(SHA, descriptors, contents), [])
        contents["release-windows-x64"]["checked"][1]["coveredBy"] = "missing.zip"
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("claims coverage", problems[0])

    def test_a_derived_package_may_alias_the_source_by_digest(self):
        # The release this stands for carries no agent: both packages are native.
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("intcad-0.1.0-beta.1-linux-x86_64.tar.gz", DG)]),
            "release-arch-x86_64": build("release-arch-x86_64", [("intcad-0.1.0-beta.1-linux-x86_64.pkg.tar.zst", DG2)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("intcad-0.1.0-beta.1-linux-x86_64.tar.gz", DG)]),
            "release-arch-x86_64": report("release-arch-x86_64",
                                          [("intcad-0.1.0-beta.1-linux-x86_64.pkg.tar.zst", DG2)],
                                          derived="release-linux-x86_64",
                                          source_name="intcad-0.1.0beta.1-linux-x86_64.tar.gz", source_sha=DG),
        }
        contents["release-arch-x86_64"]["checked"] = [
            covered("intcad-0.1.0-beta.1-linux-x86_64.pkg.tar.zst", DG2, "release-linux-x86_64")]
        self.assertEqual(content.problems(SHA, descriptors, contents, "native"), [])
        contents["release-arch-x86_64"]["sourceSha256"] = DG2
        self.assertTrue(content.problems(SHA, descriptors, contents, "native"))

    def test_explicit_mastra_distribution_stays_blocked(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)],
                                                   distribution="mastra")}
        problems = content.problems(SHA, descriptors, contents, "mastra")
        self.assertTrue(problems)
        self.assertIn("#15/#14", problems[0])


class AgentContentDecision(unittest.TestCase):
    """A release that declares the built-in agent must prove it from every supported package."""

    IDENTITIES = ("release-linux-x86_64", "release-macos", "release-windows-x64")

    def evidence(self, agent=WHOLE_AGENT, **extra):
        descriptors = {identity: build(identity, [("pkg", DG)]) for identity in self.IDENTITIES}
        contents = {}
        for identity in self.IDENTITIES:
            carried = MAC_AGENT if identity == "release-macos" else WHOLE_AGENT
            if agent is not None and agent is not WHOLE_AGENT:
                carried = dict(carried, **{k: v for k, v in agent.items() if v is not None})
            contents[identity] = report(identity, [("pkg", DG)], distribution="mastra-agent",
                                        agent=dict(carried) if carried else None, **extra)
        return descriptors, contents

    def test_a_release_with_the_agent_in_every_supported_package_is_supported(self):
        descriptors, contents = self.evidence()
        self.assertEqual(content.problems(SHA, descriptors, contents, "mastra-agent"), [])

    def test_a_supported_package_without_the_agent_fails_the_declaration(self):
        descriptors, contents = self.evidence()
        contents["release-macos"]["agent"] = None
        contents["release-macos"]["agents"] = {"pkg": None}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("release-macos" in problem and "no mastra-agent/" in problem for problem in problems), problems)

    def test_an_agent_without_its_node_or_with_missing_files_fails(self):
        descriptors, contents = self.evidence(agent={**WHOLE_AGENT, "node": False})
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("no Node program" in problem for problem in problems), problems)
        descriptors, contents = self.evidence(agent={**WHOLE_AGENT, "missing": ["mastra-agent/agent.json"]})
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("incomplete" in problem for problem in problems), problems)

    def test_an_agent_without_skills_fails_the_declaration(self):
        descriptors, contents = self.evidence(agent={**WHOLE_AGENT, "skills": []})
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("carries no modelling skill" in problem for problem in problems), problems)

    def test_one_broken_artifact_fails_the_package_beside_a_good_one(self):
        # A correct zip must not hide a broken MSI built from the same stage.
        descriptors, contents = self.evidence()
        identity = "release-windows-x64"
        contents[identity]["agents"] = {
            "pkg": WHOLE_AGENT,
            "broken.msi": {"present": True, "node": False, "missing": [], "skills": ["modelling-skills/solidcraft-modeling"]},
        }
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("broken.msi" in problem and "no Node program" in problem for problem in problems), problems)

    def test_windows_portable_zip_with_cli_in_version_still_needs_node(self):
        identity = "release-windows-x64"
        msi = "intcad-0.1.3-preview-cli-build-windows-x64.msi"
        portable = "intcad-0.1.3-preview-cli-build-windows-x64-portable.zip"
        artifacts = [(msi, DG), (portable, DG2)]
        descriptors = {identity: build(identity, artifacts)}
        contents = {identity: report(
            identity, artifacts, distribution="mastra-agent", agent=WHOLE_AGENT,
            agents={msi: WHOLE_AGENT, portable: dict(WHOLE_AGENT, node=False, missing=["mastra-agent/node"])},
        )}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(portable in problem and "no Node program" in problem for problem in problems), problems)

    def test_a_universal_app_needs_a_node_for_each_slice(self):
        descriptors, contents = self.evidence()
        contents["release-macos"]["agent"] = dict(MAC_AGENT, nodeTargets=["aarch64-apple-darwin"])
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("no Node for x86_64-apple-darwin" in problem for problem in problems), problems)

    def test_a_skill_that_is_not_the_modelling_set_does_not_count(self):
        descriptors, contents = self.evidence()
        for identity in self.IDENTITIES:
            carried = dict(MAC_AGENT if identity == "release-macos" else WHOLE_AGENT)
            carried["skills"] = ["context-skills/intcad-context"]
            contents[identity]["agent"] = carried
            contents[identity]["agents"] = {"pkg": carried}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("carries no modelling skill" in problem for problem in problems), problems)

    def test_a_legacy_runtime_or_stray_node_still_fails_with_the_agent(self):
        descriptors, contents = self.evidence(markers=("usr/lib/intcad/mastra/runtime.json",))
        self.assertTrue(content.problems(SHA, descriptors, contents, "mastra-agent"))
        descriptors, contents = self.evidence(agent_markers=("usr/lib/intcad/node",))
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("outside mastra-agent/" in problem for problem in problems), problems)

    def test_an_unsupported_package_that_carries_the_agent_fails(self):
        descriptors, contents = self.evidence()
        identity = "release-linux-aarch64"
        descriptors[identity] = build(identity, [("pkg", DG)])
        contents[identity] = report(identity, [("pkg", DG)], distribution="native", agent=WHOLE_AGENT)
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "only" in problem for problem in problems), problems)

    def test_a_derived_package_must_match_its_source_distribution(self):
        descriptors, contents = self.evidence()
        identity = "release-flatpak-x86_64"
        descriptors[identity] = build(identity, [("pkg", DG2)])
        contents[identity] = report(identity, [("pkg", DG2)], distribution="native",
                                    derived="release-linux-x86_64", source_name="pkg", source_sha=DG)
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "does not match its source" in problem for problem in problems), problems)
        contents[identity]["distribution"] = "mastra-agent"
        # The repack's own bytes carry the agent too, as the recorder records them.
        contents[identity]["checked"] = [{"name": "pkg", "sha256": DG2, "format": "tar", "entries": 10}]
        contents[identity]["agents"] = {"pkg": MAC_AGENT, "pkg-src": MAC_AGENT}
        contents[identity]["sourceArtifact"] = "pkg-src"
        self.assertEqual(content.problems(SHA, descriptors, contents, "mastra-agent"), [])

    def test_a_headless_cli_archive_does_not_need_an_agent_beside_the_app(self):
        identity = "release-macos"
        app = "intcad-0.1.3-preview-cli-build.1-macos-universal.dmg"
        cli = "intcad-cli-0.1.3-preview-cli-build.1-macos-universal.zip"
        artifacts = [(app, DG), (cli, DG2)]
        descriptors = {identity: build(identity, artifacts)}
        contents = {identity: report(
            identity, artifacts, distribution="mastra-agent", agent=MAC_AGENT,
            agents={app: MAC_AGENT, cli: {"present": False, "node": False, "nodeTargets": [], "missing": [], "skills": []}},
        )}
        self.assertEqual(content.problems(SHA, descriptors, contents, "mastra-agent"), [])

    def test_archives_that_only_resemble_a_headless_cli_archive_still_need_node(self):
        identity = "release-macos"
        app = "intcad-0.1.3-macos-universal.dmg"
        for archive in (
            "other-cli-0.1.3-macos-universal.zip",
            "intcad-cli-0.1.3-windows-x64-portable.zip",
            "intcad-cli-0.1.3-macos-universal.tar.gz",
            "intcad-cli-0.1.3-macos-universal-extra.zip",
            "intcad-cli-0.1.3-preview.01-macos-universal.zip",
        ):
            with self.subTest(archive=archive):
                artifacts = [(app, DG), (archive, DG2)]
                descriptors = {identity: build(identity, artifacts)}
                contents = {identity: report(
                    identity, artifacts, distribution="mastra-agent", agent=MAC_AGENT,
                    agents={app: MAC_AGENT, archive: dict(MAC_AGENT, node=False, missing=["mastra-agent/node"])},
                )}
                problems = content.problems(SHA, descriptors, contents, "mastra-agent")
                self.assertTrue(any(archive in problem and "no Node program" in problem for problem in problems), problems)

    def test_a_package_of_only_a_cli_archive_carries_no_agent(self):
        # The headless CLI has no Assistant: a package set that is only that archive proves
        # nothing about the app's agent.
        descriptors = {"release-macos": build("release-macos", [("intcad-cli-0.1.2-macos-aarch64.zip", DG)])}
        contents = {"release-macos": report(
            "release-macos", [("intcad-cli-0.1.2-macos-aarch64.zip", DG)], distribution="mastra-agent",
            agent={"present": False, "node": False, "nodeTargets": [], "missing": [], "skills": []})}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("no artifact of the package carries the agent" in problem for problem in problems), problems)

    def test_a_repack_artifact_without_a_verdict_fails_and_a_runtime_marker_does_too(self):
        descriptors, contents = self.evidence()
        identity = "release-snap-x86_64"
        descriptors[identity] = build(identity, [("one", DG), ("two", DG2)])
        contents[identity] = report(identity, [("one", DG), ("two", DG2)], distribution="mastra-agent",
                                    derived="release-linux-x86_64", source_name="src", source_sha=DG)
        contents[identity]["agents"] = {"one": WHOLE_AGENT, "src": WHOLE_AGENT}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "two: inspected without an agent verdict" in problem
                            for problem in problems), problems)
        contents[identity]["agents"]["two"] = WHOLE_AGENT
        contents[identity]["markers"] = [{"name": "two", "path": "usr/lib/intcad/mastra/runtime.json"}]
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "runtime marker" in problem for problem in problems), problems)

    def test_the_headless_cli_exemption_belongs_to_the_macos_package_only(self):
        # A Linux package that ships only a macOS-named CLI archive carries no agent.
        descriptors, contents = self.evidence()
        identity = "release-linux-x86_64"
        name = "intcad-cli-0.1.2-macos-arm64.zip"
        descriptors[identity] = build(identity, [(name, DG)])
        contents[identity] = report(identity, [(name, DG)], distribution="mastra-agent")
        contents[identity]["agents"] = {name: {"present": False, "node": False, "nodeTargets": [],
                                               "missing": [], "skills": []}}
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and name + ": carries no mastra-agent/" in problem
                            for problem in problems), problems)

    def test_a_native_repack_is_not_asked_for_the_agent(self):
        # The agent is built for x86_64 Linux only; the aarch64 Flatpak is native on both sides.
        descriptors = {
            "release-linux-aarch64": build("release-linux-aarch64", [("tarball", DG)]),
            "release-flatpak-aarch64": build("release-flatpak-aarch64", [("pkg", DG2)]),
        }
        contents = {
            "release-linux-aarch64": report("release-linux-aarch64", [("tarball", DG)]),
            "release-flatpak-aarch64": report("release-flatpak-aarch64", [("pkg", DG2)],
                                              derived="release-linux-aarch64",
                                              source_name="tarball", source_sha=DG),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents, "mastra-agent"), [])

    def test_a_repack_without_the_agent_in_its_own_bytes_fails(self):
        # The source tarball carries the agent; the repack's own payload does not.
        descriptors, contents = self.evidence()
        identity = "release-flatpak-x86_64"
        descriptors[identity] = build(identity, [("pkg", DG2)])
        contents[identity] = report(identity, [("pkg", DG2)], distribution="mastra-agent",
                                    derived="release-linux-x86_64", source_name="src", source_sha=DG)
        contents[identity]["agents"] = {
            "pkg": {"present": False, "node": False, "nodeTargets": [], "missing": [], "skills": []},
            "src": WHOLE_AGENT,
        }
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any(identity in problem and "pkg: carries no mastra-agent/" in problem
                            for problem in problems), problems)

    def test_a_declaration_the_packages_do_not_match_fails(self):
        descriptors, contents = self.evidence()
        contents["release-linux-x86_64"]["distribution"] = "native"
        problems = content.problems(SHA, descriptors, contents, "mastra-agent")
        self.assertTrue(any("recorded as native" in problem for problem in problems), problems)
        self.assertTrue(content.problems(SHA, descriptors, contents, "unknown-kind"))


class ReleaseWorkflowWiring(unittest.TestCase):
    """The aggregate must decide from package bytes, not append a permanent failure."""

    def setUp(self):
        self.gate = (ROOT / ".github/workflows/release-gate.yml").read_text()
        self.release = (ROOT / ".github/workflows/release.yml").read_text()

    def test_aggregate_no_longer_appends_unconditional_mastra_failures(self):
        self.assertNotIn("problems.append('#15: required runtime-artifact", self.gate)
        self.assertNotIn("problems.append('#14: required installed-runtime", self.gate)
        # The merged gate reads the distribution the packages job observed; a static
        # declaration cannot stand in for the installer bytes.
        self.assertIn("distribution = os.environ['DISTRIBUTION']", self.gate)
        self.assertIn("elif distribution == 'mastra-agent':", self.gate)
        self.assertIn("problems.append('missing or unapproved package distribution')", self.gate)
        self.assertNotIn("SOLIDCRAFT_DISTRIBUTION", self.gate, "the gate must not keep a declaration the script ignores")

    def test_release_jobs_record_actual_package_contents(self):
        self.assertIn('SOLIDCRAFT_DISTRIBUTION: "mastra-agent"', self.release)
        self.assertIn("scripts/ci/package_contents.py", self.release)
        self.assertIn("dist/release/contents.json", self.release)
        self.assertIn("package-evidence-${{ env.PACKAGE_ID }}", self.release)
        self.assertIn("dist/release/build.json", self.release)

    def test_derived_packages_must_name_the_inspected_linux_source(self):
        self.assertIn("release-flatpak-', 'release-arch-', 'release-snap-'", self.release)
        self.assertIn("--derived-from", self.release)
        self.assertIn("release-linux-", self.release)

if __name__ == "__main__":
    unittest.main()
