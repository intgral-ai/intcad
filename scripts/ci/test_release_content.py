#!/usr/bin/env python3
"""Tests for the native-package content decision (#14/#15 applicability)."""

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
            self.assertEqual(dict(line.split("=", 1) for line in env_file.read_text().splitlines()), CONFIG)

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

    def test_version_guard_rejects_other_workflows_and_events(self):
        version = workflow("release.yml")["jobs"]["version"]
        step = next((step for step in version["steps"]
                     if step.get("id") == "staging-guard"), None)
        self.assertIsNotNone(step, "missing staging caller guard")
        self.assertEqual(step["if"], "inputs.staging_account_config != '' || "
                         "github.workflow_ref == format('{0}/.github/workflows/staging-packages.yml@{1}', "
                         "github.repository, github.ref)")
        self.assertEqual(step["env"], {
            "CALLER_WORKFLOW_REF": "${{ github.workflow_ref }}",
            "STAGING_WORKFLOW_REF": "${{ format('{0}/.github/workflows/staging-packages.yml@{1}', github.repository, github.ref) }}",
            "HAS_STAGING_CONFIG": "${{ inputs.staging_account_config != '' && '1' || '0' }}",
        })
        canonical = "fixture/repo/.github/workflows/staging-packages.yml@refs/heads/fixture"
        for caller, event, snapshot, ok in (
            (canonical, "workflow_dispatch", "1", True),
            (canonical, "workflow_dispatch", "0", False),
            (canonical, "pull_request", "1", False),
            (canonical.replace("staging-packages", "release"), "workflow_dispatch", "1", False),
            (canonical.replace("fixture/repo", "other/repo"), "workflow_dispatch", "1", False),
            (canonical + "-other", "workflow_dispatch", "1", False),
        ):
            with self.subTest(caller=caller, event=event):
                result = subprocess.run(
                    ["bash", "-ec", step["run"]],
                    env={**os.environ, "CALLER_WORKFLOW_REF": caller, "STAGING_WORKFLOW_REF": canonical,
                         "GITHUB_EVENT_NAME": event, "HAS_STAGING_CONFIG": snapshot},
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
                self.assertEqual(inject.get("run"), "python -I scripts/ci/staging_account_config.py inject")
                self.assertEqual(inject["env"], snapshot_env)
                self.assertEqual(inject["if"], "inputs.staging_account_config != ''")
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
        for job, ordinary in labels.items():
            with self.subTest(job=job):
                if job in ("linux", "macos", "windows"):
                    ordinary += ["${{ env.PACKAGE_ID }}", "package-evidence-${{ env.PACKAGE_ID }}"]
                    suffix = "" if job == "macos" else "-${{ matrix.arch }}"
                    self.assertEqual(jobs[job]["env"]["PACKAGE_ID"], f"release-{job}{suffix}")
                actual = [step["with"]["name"] for step in jobs[job]["steps"]
                          if step.get("uses", "").startswith(("actions/upload-artifact@", "actions/download-artifact@"))]
                self.assertEqual(actual, ["${{ env.ARTIFACT_PREFIX }}" + label for label in ordinary])

    def test_staging_evidence_hashes_the_package_and_revalidates_the_same_snapshot(self):
        job = workflow("release.yml")["jobs"]["macos"]
        step = next((step for step in job["steps"] if SNAPSHOT in step.get("env", {})), None)
        self.assertIsNotNone(step, "missing staging evidence input")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            sha = config_checkout(root, json.dumps(CONFIG).encode())
            directory = root / "dist/release"
            directory.mkdir(parents=True)
            (directory / "fixture.tar.gz").write_bytes(b"abc")
            scripts = root / "scripts/ci"
            scripts.mkdir(parents=True)
            for source in (HELPER, HERE / "package_contents.py"):
                shutil.copyfile(source, scripts / source.name)
            env = {**os.environ, "GITHUB_SHA": sha, "PACKAGE_ID": "release-macos",
                   "PACKAGE_TARGET": "universal-apple-darwin"}
            expected = {
                "version": 1, "packageId": "release-macos", "sourceRevision": sha,
                "target": "universal-apple-darwin", "contractDigest": None,
                "artifacts": [{"name": "fixture.tar.gz",
                               "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}],
            }
            descriptor = directory / "build.json"
            command = [sys.executable, "-c", step["run"]]
            for snapshot, fields in (("", expected), (json.dumps(CONFIG), {
                **expected, "accountConfig": {"source": "checked-in:staging", "sourceRevision": sha,
                                              "path": CONFIG_PATH, "values": CONFIG},
            })):
                result = subprocess.run(command, cwd=root, env={**env, SNAPSHOT: snapshot},
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(descriptor.read_text()), fields)
                descriptor.unlink()
            result = subprocess.run(command, cwd=root, env={**env, SNAPSHOT: "{}"},
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(descriptor.exists())
            result = subprocess.run([sys.executable, "-c", step["run"]], cwd=root,
                                    env={**env, SNAPSHOT: json.dumps({**CONFIG, CLIENT: "client_Drift"})},
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(descriptor.exists())
            # Ordinary runs must not read the committed staging config at all.
            new_sha = config_checkout(root, b"not json")
            result = subprocess.run([sys.executable, "-c", step["run"]], cwd=root,
                                    env={**env, "GITHUB_SHA": new_sha, SNAPSHOT: ""},
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(descriptor.read_text()), {**expected, "sourceRevision": new_sha})


def build(identity, artifacts):
    return {
        "packageId": identity,
        "sourceRevision": SHA,
        "artifacts": [{"name": name, "sha256": digest} for name, digest in artifacts],
    }


def report(identity, checked, *, distribution="native", markers=(), unchecked=(), ignored=(), derived=None,
           source_name=None, source_sha=None):
    value = {
        "packageId": identity,
        "sourceRevision": SHA,
        "distribution": distribution,
        "checked": [{"name": name, "sha256": digest, "format": "tar", "entries": 10} for name, digest in checked],
        "ignored": list(ignored),
        "unchecked": [{"name": name, "reason": "no inspector"} for name in unchecked],
        "markers": [{"name": "pkg", "path": path} for path in markers],
        "derivedFrom": derived,
        "sourceArtifact": source_name,
        "sourceSha256": source_sha,
    }
    return value


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
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": build("release-flatpak-x86_64", [("solidcraft.flatpak", DG2)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": report("release-flatpak-x86_64", [], derived="release-linux-x86_64",
                                             source_name="solidcraft.tar.gz", source_sha=DG),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents), [])
        contents["release-flatpak-x86_64"]["derivedFrom"] = "release-macos"
        self.assertTrue(content.problems(SHA, descriptors, contents))

    def test_a_derived_package_needs_a_cleaned_source_scan(self):
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": build("release-flatpak-x86_64", [("solidcraft.flatpak", DG2)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)]),
            "release-flatpak-x86_64": report("release-flatpak-x86_64", [], derived="release-linux-x86_64",
                                             source_name="solidcraft.tar.gz", source_sha=DG),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents), [])
        for field, value in (("markers", ["usr/lib/intcad/mastra"]), ("unchecked", ["solidcraft.tar.gz"])):
            contents["release-flatpak-x86_64"] = report(
                "release-flatpak-x86_64", [], derived="release-linux-x86_64",
                source_name="solidcraft.tar.gz", source_sha=DG, **{field: value})
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
        descriptors = {
            "release-linux-x86_64": build("release-linux-x86_64", [("intcad-0.1.0-beta.1-linux-x86_64.tar.gz", DG)]),
            "release-arch-x86_64": build("release-arch-x86_64", [("intcad-0.1.0-beta.1-linux-x86_64.pkg.tar.zst", DG2)]),
        }
        contents = {
            "release-linux-x86_64": report("release-linux-x86_64", [("intcad-0.1.0-beta.1-linux-x86_64.tar.gz", DG)]),
            "release-arch-x86_64": report("release-arch-x86_64", [], derived="release-linux-x86_64",
                                          source_name="intcad-0.1.0beta.1-linux-x86_64.tar.gz", source_sha=DG),
        }
        self.assertEqual(content.problems(SHA, descriptors, contents), [])
        contents["release-arch-x86_64"]["sourceSha256"] = DG2
        self.assertTrue(content.problems(SHA, descriptors, contents))

    def test_explicit_mastra_distribution_stays_blocked(self):
        descriptors = {"release-linux-x86_64": build("release-linux-x86_64", [("solidcraft.tar.gz", DG)])}
        contents = {"release-linux-x86_64": report("release-linux-x86_64", [("solidcraft.tar.gz", DG)],
                                                   distribution="mastra")}
        problems = content.problems(SHA, descriptors, contents)
        self.assertTrue(problems)
        self.assertIn("#15/#14", problems[0])


class ReleaseWorkflowWiring(unittest.TestCase):
    """The aggregate must decide from package bytes, not append a permanent failure."""

    def setUp(self):
        self.gate = (ROOT / ".github/workflows/release-gate.yml").read_text()
        self.release = (ROOT / ".github/workflows/release.yml").read_text()

    def test_aggregate_no_longer_appends_unconditional_mastra_failures(self):
        self.assertNotIn("problems.append('#15: required runtime-artifact", self.gate)
        self.assertNotIn("problems.append('#14: required installed-runtime", self.gate)
        self.assertIn("content.problems(sha, descriptors, contents)", self.gate)

    def test_release_jobs_record_actual_package_contents(self):
        self.assertIn('SOLIDCRAFT_DISTRIBUTION: "native"', self.release)
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
