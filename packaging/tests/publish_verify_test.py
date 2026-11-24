"""Release signing may consume only the exact evidence set accepted by the stable gate."""
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
import release_handoff as release

SHA = "a" * 40
DIGEST = "sha256:" + "b" * 64
V = "0.1.3"


class Publish(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.artifacts = self.root / "artifacts"
        self.artifacts.mkdir()
        self.index = {}
        for identity, target in release.TARGETS.items():
            package = self.artifacts / identity
            pair = self.artifacts / ("package-evidence-" + identity)
            package.mkdir()
            pair.mkdir()
            names = release.asset_names(identity, V)
            for name in names:
                (package / name).write_bytes(name.encode())
            entries = [{"name": name, "sha256": release.digest(package / name)} for name in sorted(names)]
            build = dict(version=1, packageId=identity, target=target, sourceRevision=SHA,
                         contractDigest=DIGEST, artifacts=entries)
            report = dict(schemaVersion=1, packageId=identity, sourceRevision=SHA, distribution="native",
                          checked=[a for a in entries if not a["name"].endswith(".zsync")],
                          ignored=[a["name"] for a in entries if a["name"].endswith(".zsync")],
                          unchecked=[], markers=[], derivedFrom=None, sourceArtifact=None, sourceSha256=None)
            if identity.startswith(("release-flatpak-", "release-arch-", "release-snap-")):
                arch = identity.rsplit("-", 1)[1]
                linux = "release-linux-" + arch
                tar = f"intcad-{V}-linux-{arch}.tar.gz"
                report.update(derivedFrom=linux, sourceArtifact=tar, sourceSha256=hashlib.sha256(tar.encode()).hexdigest(),
                              checked=[], ignored=[])
            for name, data in (("build.json", build), ("contents.json", report)):
                raw = json.dumps(data).encode()
                for directory in (package, pair):
                    (directory / name).write_bytes(raw)
            self.index[identity] = {name: release.digest(pair / name) for name in release.DESCRIPTORS}

    def stage(self):
        release.stage(self.artifacts, self.root / "out", V, SHA, DIGEST, self.index)

    def test_complete_matrix(self):
        self.stage()
        self.assertTrue((self.root / "out" / f"intcad-{V}-windows-x64.msi").is_file())
        self.assertFalse(any(p.suffix == ".json" for p in (self.root / "out").iterdir()))

    def test_no_build_descriptor_is_refused(self):
        (self.artifacts / "package-evidence-release-macos" / "build.json").unlink()
        with self.assertRaisesRegex(ValueError, "descriptor"):
            self.stage()
        self.assertFalse((self.root / "out").exists())

    def test_missing_pair_is_refused(self):
        (self.artifacts / "release-macos").rename(self.root / "missing")
        with self.assertRaisesRegex(ValueError, "pairs"):
            self.stage()

    def test_unexpected_duplicate_pair_is_refused(self):
        (self.artifacts / "package-evidence-release-macos-copy").mkdir()
        with self.assertRaisesRegex(ValueError, "pairs"):
            self.stage()

    def test_recomputed_package_digest_cannot_replace_accepted_evidence(self):
        p = self.artifacts / "package-evidence-release-macos" / "build.json"
        p.write_text(p.read_text().replace("universal-apple-darwin", "bogus"))
        with self.assertRaisesRegex(ValueError, "accepted descriptor"):
            self.stage()

    def test_modified_package_bytes(self):
        (self.artifacts / "release-macos" / f"intcad-{V}-macos-universal.dmg").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "package bytes"):
            self.stage()

    def change_report(self, identity, filename, **fields):
        a = self.artifacts / identity / filename
        b = self.artifacts / ("package-evidence-" + identity) / filename
        body = json.loads(a.read_text()) | fields
        for p in (a, b):
            p.write_text(json.dumps(body))
        self.index[identity][filename] = release.digest(b)

    def test_wrong_target_or_contract_in_accepted_descriptor(self):
        for fields in ({"target": "bogus"}, {"contractDigest": "bogus"}):
            with self.subTest(fields=fields):
                self.change_report("release-macos", "build.json", target=release.TARGETS["release-macos"], contractDigest=DIGEST)
                self.change_report("release-macos", "build.json", **fields)
                with self.assertRaisesRegex(ValueError, "target|contract"):
                    self.stage()

    def test_contents_cannot_disagree_with_build(self):
        self.change_report("release-macos", "contents.json", checked=[])
        with self.assertRaisesRegex(ValueError, "content evidence"):
            self.stage()

    def test_derived_contents_must_match_correct_linux_target(self):
        self.change_report("release-flatpak-aarch64", "contents.json", derivedFrom="release-linux-x86_64")
        with self.assertRaisesRegex(ValueError, "derived"):
            self.stage()

    def test_package_and_evidence_copy_must_agree(self):
        p = self.artifacts / "release-macos" / "contents.json"
        p.write_text(p.read_text() + "\n")
        with self.assertRaisesRegex(ValueError, "descriptor copy"):
            self.stage()

    def test_archives_must_have_unique_names_ids_and_digests(self):
        names = ["x-a2", "y-a2"]
        artifact = lambda name, i: dict(name=name, id=i, expired=False, digest=DIGEST)
        self.assertEqual(set(release.artifact_index([artifact("x-a2", 1), artifact("y-a2", 2)], names)), set(names))
        for rows in (
            [artifact("x-a2", 1), artifact("x-a2", 2), artifact("y-a2", 3)],
            [artifact("x-a2", 1), artifact("y-a2", 1)],
            [artifact("x-a2", 1)],
            [artifact("x-a2", 1), artifact("y-a2", 2) | {"digest": None}],
        ):
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                release.artifact_index(rows, names)

    def test_gate_attempt_uses_real_jobs_not_pr_head(self):
        run = dict(id=42, run_attempt=2, path=".github/workflows/release-gate.yml", event="pull_request",
                   conclusion="success", status="completed", head_sha="b" * 40)
        jobs = [dict(name=name, run_id=42, run_attempt=2, conclusion="success", status="completed")
                for name in release.GATE_JOBS]
        release.verify_execution(run, jobs, 42, 2)
        for bad in ([jobs[0]], jobs + jobs, [job | {"run_attempt": 1} for job in jobs]):
            with self.assertRaises(ValueError):
                release.verify_execution(run, bad, 42, 2)
        with self.assertRaises(ValueError):
            release.verify_execution(run | {"event": "workflow_dispatch"}, jobs, 42, 2)

    def test_dispatch_is_data_and_stable_only(self):
        self.assertEqual(release.dispatch("v0.1.3", "42", "2"), (V, 42, 2))
        for values in [("v0.1.3-rc.1", "42", "2"), ("v0.1.3\n", "42", "2"), ("v0.1.3", "../42", "2"),
                       ("v0.1.3", "42", "0"), ("v00.1.3", "42", "2")]:
            with self.assertRaises(ValueError):
                release.dispatch(*values)

    def test_draft_readback_requires_all_assets_and_digests(self):
        root = self.root / "draft"
        root.mkdir()
        for name in ("Intcad.msi", "SHA256SUMS", "SHA256SUMS.txt", "SHA256SUMS.txt.minisig"):
            (root / name).write_text(name)
        assets = [dict(name=p.name, state="uploaded", size=p.stat().st_size, digest="sha256:" + release.digest(p))
                  for p in root.iterdir()]
        draft = dict(id=14, tag_name="v0.1.3", draft=True, prerelease=False, body="Notes\n", assets=assets)
        release.verify_draft(draft, root, "v0.1.3", "Notes\n")
        for changes in [dict(draft=False), dict(prerelease=True), dict(assets=assets[:-1]),
                        dict(assets=[a | {"digest": None} for a in assets]), dict(assets=assets + assets),
                        dict(assets=[a | {"digest": DIGEST} for a in assets])]:
            with self.assertRaises(ValueError):
                release.verify_draft(draft | changes, root, "v0.1.3", "Notes\n")

    def test_archive_is_verified_before_extracting_exact_flat_members(self):
        archive = self.root / "blob.zip"
        target = self.root / "download"
        with ZipFile(archive, "w") as zip_file:
            zip_file.writestr("build.json", "{}")
            zip_file.writestr("nested/contents.json", "{}")
        with self.assertRaisesRegex(ValueError, "archive digest"):
            release.extract_archive(archive, target, DIGEST, set(release.DESCRIPTORS))
        with self.assertRaisesRegex(ValueError, "archive files"):
            release.extract_archive(archive, target, "sha256:" + release.digest(archive), set(release.DESCRIPTORS))
        self.assertFalse(target.exists())

    def test_manifest_pins_repo_attempt_sha_version_and_live_archive_identity(self):
        rows = [dict(id=i, name=name, digest=DIGEST, expired=False)
                for i, name in enumerate(release.pair_names(2), 1)]
        accepted = release.artifact_index(rows, release.pair_names(2))
        manifest = release.make_manifest(12, 42, 2, SHA, V, DIGEST, "native", accepted, self.index)
        self.assertEqual(release.verify_manifest(manifest, 12, 42, 2, SHA, V, DIGEST, accepted), self.index)
        for repo, run, attempt, sha, version in (
            (13, 42, 2, SHA, V), (12, 41, 2, SHA, V), (12, 42, 1, SHA, V),
            (12, 42, 2, "b" * 40, V), (12, 42, 2, SHA, "0.1.4"),
        ):
            with self.subTest(attempt=attempt, sha=sha), self.assertRaises(ValueError):
                release.verify_manifest(manifest, repo, run, attempt, sha, version, DIGEST, accepted)
        name = rows[0]["name"]
        swapped = accepted | {name: accepted[name] | {"id": 99}}
        with self.assertRaises(ValueError):
            release.verify_manifest(manifest, 12, 42, 2, SHA, V, DIGEST, swapped)

    def test_authorization_requires_configured_private_nonfork_repository(self):
        repo = dict(id=12, full_name="intgral-ai/intcad-client", visibility="private", private=True, fork=False)
        release.verify_repository(repo, "12", "private")
        public = dict(id=1446384930, full_name="intgral-ai/intcad", visibility="public", private=False, fork=False)
        release.verify_repository(public, "", "public")
        for metadata, configured, mode in [
            (repo, "", "private"), (repo, "99", "private"), (repo, "12", "public"),
            (repo | {"private": False}, "12", "private"), (repo | {"fork": True}, "12", "private"),
            (repo | {"full_name": "intgral-ai/intcad"}, "12", "private"),
            (public | {"id": 12}, "12", "public"), (public | {"fork": True}, "", "public"),
            (public | {"visibility": "internal"}, "", "public"), (public, "99", "public"),
        ]:
            with self.assertRaises(ValueError):
                release.verify_repository(metadata, configured, mode)

    def test_agent_candidate_requires_explicit_legal_and_installed_review_for_exact_execution(self):
        release.verify_agent_review("native", SHA, 42, 2, "", "")
        receipt = f"{SHA}:42:2"
        release.verify_agent_review("mastra-agent", SHA, 42, 2, receipt, receipt)
        for legal, installed in [("", receipt), (receipt, ""), (receipt, f"{SHA}:42:1")]:
            with self.assertRaises(ValueError):
                release.verify_agent_review("mastra-agent", SHA, 42, 2, legal, installed)

    def test_gate_handoff_excludes_drafts_foreign_prs_and_diagnostics(self):
        ref = dict(ref="release/v0.1.3", repo={"id": 12})
        pr = dict(draft=False, head=ref, base=dict(ref="main", repo={"id": 12}))
        event = dict(action="ready_for_review", pull_request=pr)
        self.assertTrue(release.gate_candidate("pull_request", event, 12, V))
        self.assertFalse(release.gate_candidate("workflow_dispatch", event, 12, V))
        for edited in [pr | {"draft": True}, pr | {"head": ref | {"repo": {"id": 13}}},
                       pr | {"head": ref | {"ref": "release/v0.1.3-beta.1"}}, pr | {"base": {"ref": "develop"}}]:
            self.assertFalse(release.gate_candidate("pull_request", event | {"pull_request": edited}, 12, V))
        group = dict(action="checks_requested", merge_group={"base_ref": "refs/heads/main"})
        self.assertTrue(release.gate_candidate("merge_group", group, 12, V))
        self.assertFalse(release.gate_candidate("merge_group", group, 12, V + "-beta.1"))

    def test_environment_requires_review_and_only_exact_main_branch_policy(self):
        reviewers = dict(type="required_reviewers", prevent_self_review=True, reviewers=[{"type": "Team"}])
        env = dict(name="release-signing", protection_rules=[reviewers],
                   deployment_branch_policy=dict(protected_branches=False, custom_branch_policies=True))
        policies = [dict(name="main", type="branch")]
        release.verify_environment(env, policies)
        for metadata, rules in [
            (env | {"protection_rules": []}, policies),
            (env | {"protection_rules": [reviewers | {"prevent_self_review": False}]}, policies),
            (env | {"can_admins_bypass": True}, policies),
            (env, [dict(name="main", type="tag")]),
            (env, policies + [dict(name="release/*", type="branch")]),
        ]:
            with self.assertRaises(ValueError):
                release.verify_environment(metadata, rules)

    def test_present_but_unlisted_package_and_missing_alias_are_refused(self):
        extra = self.artifacts / "release-macos" / "notes.txt"
        extra.write_text("not a package")
        with self.assertRaisesRegex(ValueError, "package files"):
            self.stage()
        extra.unlink()
        self.stage()
        with self.assertRaisesRegex(ValueError, "release files"):
            release.verify_release_files(self.root / "out", V, signed=False)


if __name__ == "__main__":
    unittest.main()
