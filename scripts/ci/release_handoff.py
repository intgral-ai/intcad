#!/usr/bin/env python3
"""Bind an approved stable release to one checkout and one complete Actions attempt."""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tomllib
from pathlib import Path, PurePosixPath
from tempfile import TemporaryDirectory
from urllib.parse import urlencode
from zipfile import BadZipFile, ZipFile

# Also usable as a Python -I entry point, with imports from this checkout only.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from gate import parse_json
import release_content

PUBLIC_SOURCE = "intgral-ai/intcad"
PUBLIC_SOURCE_ID = 1446384930
PRIVATE_SOURCE = "intgral-ai/intcad-client"
DIST_REPO = "intgral-ai/intcad-releases"
WORKFLOW = ".github/workflows/release-gate.yml"
CONTRACT = "schema/mastra-runtime/v1/manifest.json"
PUBLIC_KEY = "crates/ui-egui/src/update/trusted-release-key.pub"
DESCRIPTORS = ("build.json", "contents.json")
SUMS = "SHA256SUMS.txt"
LEGACY_SUMS = "SHA256SUMS"
SIGNATURE = SUMS + ".minisig"
SIGNING_ASSETS = (SUMS, LEGACY_SUMS, SIGNATURE)
STABLE = r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
HEX = r"[0-9a-f]{64}"
ARCHES = ("x86_64", "aarch64")
WINDOWS = {"x64": "x86_64", "x86": "i686", "arm64": "aarch64"}
DERIVED = {"flatpak": ".flatpak", "arch": ".pkg.tar.zst", "snap": ".snap"}
TARGETS = {
    **{f"release-{kind}-{arch}": f"{arch}-unknown-linux-gnu"
       for kind in ("linux", *DERIVED) for arch in ARCHES},
    "release-freebsd-x86_64": "x86_64-unknown-freebsd",
    "release-macos": "universal-apple-darwin",
    **{f"release-windows-{arch}": f"{target}-pc-windows-msvc" for arch, target in WINDOWS.items()},
    "release-web": "wasm32-unknown-unknown",
}
PACKAGE_JOBS = {
    "packages / " + name for name in (
        "version", "icons", "All nonpublishing package builds", "FreeBSD x86_64",
        "macOS universal (ad-hoc only)", "Web (wasm)",
        *(f"macOS build {arch}" for arch in ARCHES),
        *(f"{platform} {arch}" for platform in ("Linux", "Flatpak", "Arch Linux", "Snap") for arch in ARCHES),
        *(f"Windows {arch} (unsigned)" for arch in WINDOWS),
    )
}
GATE_JOBS = {"release-gate", *PACKAGE_JOBS}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def matches(pattern, value):
    return isinstance(value, str) and re.fullmatch(pattern, value) is not None


def positive(value):
    return type(value) is int and value > 0


def number(value):
    require(matches(r"[1-9][0-9]{0,19}", value), "expected a positive decimal ID or attempt")
    return int(value)


def dispatch(tag, run_id, attempt):
    require(matches("v" + STABLE, tag), "expected an exact stable vX.Y.Z tag")
    return tag[1:], number(run_id), number(attempt)


def verify_repository(metadata, configured, mode):
    require(mode in ("public", "private"), "configure the approved source visibility")
    expected, ident = (PUBLIC_SOURCE, PUBLIC_SOURCE_ID) if mode == "public" else (PRIVATE_SOURCE, number(configured))
    # `intgral-ai/intcad` is deliberately still a fork of SolidCraft while it is the
    # public release source. The private client repository must stand alone.
    require(metadata.get("full_name") == expected and metadata.get("visibility") == mode
            and metadata.get("private") is (mode == "private")
            and (mode != "private" or metadata.get("fork") is False)
            and metadata.get("id") == ident and (not configured or number(configured) == ident),
            "release source must be the approved repository, ID and visibility"
            + ("" if mode != "private" else " (the private client must not be a fork)"))


def verify_execution(run, jobs, run_id, attempt, repo_id=None):
    # Do not substitute run.head_sha for the actual PR merge checkout.
    require(run.get("id") == run_id and run.get("run_attempt") == attempt
            and run.get("path") == WORKFLOW and run.get("event") in {"pull_request", "merge_group"}
            and run.get("status") == "completed" and run.get("conclusion") == "success",
            "the current run attempt is not a successful formal release-gate run")
    if repo_id is not None:
        for field in ("repository", "head_repository"):
            require((run.get(field) or {}).get("id") == repo_id, f"run {field} is not the approved source")
    seen = set()
    for job in jobs:
        name = job.get("name", "")
        if name not in GATE_JOBS and not name.startswith("packages / "):
            continue
        require(name in GATE_JOBS and name not in seen, f"unexpected/duplicate release job: {name}")
        require(job.get("run_id") == run_id and job.get("run_attempt") == attempt
                and job.get("status") == "completed" and job.get("conclusion") == "success",
                f"release job {name} did not succeed in the exact attempt")
        seen.add(name)
    require(seen == GATE_JOBS, f"missing exact release jobs: {sorted(GATE_JOBS - seen)}")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def sha256(raw):
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def pair(identity, attempt):
    package = f"{identity}-a{attempt}"
    return package, f"package-evidence-{package}"


def pair_names(attempt):
    return tuple(name for identity in sorted(TARGETS) for name in pair(identity, attempt))


def manifest_name(attempt):
    return f"stable-release-handoff-a{attempt}"


def artifact_index(rows, names):
    expected = set(names)
    selected = {}
    ids = set()
    for row in rows:
        name = row.get("name")
        if name not in expected:
            continue
        ident, checksum = row.get("id"), row.get("digest")
        require(name not in selected and positive(ident) and ident not in ids, "duplicate/invalid artifact name or id")
        require(row.get("expired") is False and matches("sha256:" + HEX, checksum),
                f"{name}: expired artifact or missing archive digest")
        ids.add(ident)
        selected[name] = {"id": ident, "digest": checksum}
    require(set(selected) == expected, f"missing exact attempt artifacts: {sorted(expected - set(selected))}")
    return selected


def asset_names(identity, version):
    parts = identity.removeprefix("release-").split("-")
    kind, *machine = parts
    arch = machine[0] if machine else None
    if kind == "linux":
        base = f"intcad-{version}-linux-{arch}"
        return {base + ext for ext in (".AppImage", ".AppImage.zsync", ".deb", ".rpm", ".tar.gz")} | {
            f"{earlier}-{version}-linux-{arch}.AppImage.zsync" for earlier in ("solidcraft", "solvecraft")
        }
    if kind in DERIVED:
        return {f"intcad-{version}-linux-{arch}{DERIVED[kind]}"}
    if kind == "windows":
        base = f"intcad-{version}-windows-{arch}"
        names = {base + ".msi", base + "-portable.zip"}
        return names | ({f"Intcad-{version}-windows-x64.exe"} if arch == "x64" else set())
    return {
        "freebsd": {f"intcad-{version}-freebsd-x86_64.tar.gz"},
        "macos": {f"intcad-{version}-macos-universal.dmg", f"intcad-cli-{version}-macos-universal.zip"},
        "web": {f"intcad-web-{version}.zip"},
    }[kind]


def entries(root, expected, *, directories=False, message):
    require(not root.is_symlink() and root.is_dir(), message)
    found = {path.name: path for path in root.iterdir()}
    require(set(found) == set(expected) and all(
        not path.is_symlink() and (path.is_dir() if directories else path.is_file()) for path in found.values()
    ), message)
    return found


def descriptor_index(root, version, sha, contract, *, attempt=None, accepted=None):
    """Validate both descriptors before accepting an index or any package bytes."""
    evidence_names = {identity: pair(identity, attempt)[1] if attempt is not None else f"package-evidence-{identity}"
                      for identity in TARGETS}
    builds, reports, hashes = {}, {}, {}
    for identity, name in evidence_names.items():
        files = entries(root / name, DESCRIPTORS, message=f"{name}: missing or extra descriptor")
        raw = {name: path.read_bytes() for name, path in files.items()}
        hashes[identity] = {name: hashlib.sha256(data).hexdigest() for name, data in raw.items()}
        if accepted is not None:
            require(hashes[identity] == accepted[identity], f"{identity}: does not match accepted descriptor digest")
        build, report = (parse_json(raw[name]) for name in DESCRIPTORS)
        require(build.get("version") == 1 and build.get("packageId") == identity
                and build.get("sourceRevision") == sha, f"{identity}: build provenance mismatch")
        require(build.get("target") == TARGETS[identity], f"{identity}: target mismatch")
        require(matches("sha256:" + HEX, contract) and build.get("contractDigest") == contract,
                f"{identity}: contract digest mismatch")
        built = {}
        artifacts = build.get("artifacts")
        require(isinstance(artifacts, list), f"{identity}: missing package digests")
        for item in artifacts:
            asset, checksum = item.get("name"), item.get("sha256")
            require(isinstance(asset, str) and asset not in built and matches(HEX, checksum),
                    f"{identity}: invalid/duplicate package digest")
            built[asset] = checksum
        require(set(built) == asset_names(identity, version), f"{identity}: wrong version or package files")
        require(report.get("schemaVersion") == 1 and report.get("packageId") == identity
                and report.get("sourceRevision") == sha, f"{identity}: content evidence provenance mismatch")
        builds[identity], reports[identity] = build, report

    for identity, report in reports.items():
        arch = identity.rsplit("-", 1)[-1]
        if any(identity.startswith(f"release-{kind}-") for kind in DERIVED):
            source = f"release-linux-{arch}"
            tarball = f"intcad-{version}-linux-{arch}.tar.gz"
            artifacts = {item["name"]: item["sha256"] for item in builds[source]["artifacts"]}
            require(report.get("derivedFrom") == source and report.get("sourceArtifact") == tarball
                    and report.get("sourceSha256") == artifacts[tarball],
                    f"{identity}: derived package source is not the same-target Linux tarball")
        else:
            require(all(report.get(field) is None for field in ("derivedFrom", "sourceArtifact", "sourceSha256")),
                    f"{identity}: unexpected derived package source")
    return builds, reports, hashes


def stage(root, out, version, sha, contract, accepted, *, attempt=None, distribution="native"):
    package_names = {identity: pair(identity, attempt)[0] if attempt is not None else identity for identity in TARGETS}
    evidence_names = {identity: pair(identity, attempt)[1] if attempt is not None else f"package-evidence-{identity}"
                      for identity in TARGETS}
    entries(root, {*package_names.values(), *evidence_names.values()}, directories=True,
            message="missing or unexpected staging pairs")
    builds, reports, _ = descriptor_index(root, version, sha, contract, attempt=attempt, accepted=accepted)
    if distribution == "native":
        problems = release_content.problems(sha, builds, reports)
        require(not problems, "content evidence: " + "; ".join(problems))
    else:
        # The stable gate at the candidate SHA already evaluated #214's real
        # per-asset agent-content checks; the exact report digests are pinned.
        require(distribution == "mastra-agent", "unknown or unapproved distribution")
    assets = {}
    for identity, name in package_names.items():
        expected = {item["name"]: item["sha256"] for item in builds[identity]["artifacts"]}
        files = entries(root / name, {*expected, *DESCRIPTORS}, message=f"{identity}: unexpected package files")
        for descriptor in DESCRIPTORS:
            require(digest(files[descriptor]) == accepted[identity][descriptor], f"{identity}: descriptor copy mismatch")
        for asset, checksum in expected.items():
            require(asset not in assets, f"duplicate package file: {asset}")
            require(digest(files[asset]) == checksum, f"{asset}: package bytes differ from accepted descriptor")
            assets[asset] = files[asset]
    # Do not create the flat output until the entire staging tree has passed.
    out.mkdir()
    for asset, path in assets.items():
        shutil.move(path, out / asset)


def make_manifest(repo, run, attempt, sha, version, contract, distribution, archives, descriptors):
    return dict(schemaVersion=1, repositoryId=repo, workflow=WORKFLOW, runId=run, runAttempt=attempt,
                sourceRevision=sha, version=version, contractDigest=contract, distribution=distribution,
                packages={
                    identity: {
                        "archives": {name: archives[name] for name in pair(identity, attempt)},
                        "descriptors": descriptors[identity],
                    } for identity in sorted(TARGETS)
                })


def verify_manifest(data, repo, run, attempt, sha, version, contract, archives):
    distribution = data.get("distribution")
    require(distribution in ("native", "mastra-agent"), "unknown or unapproved distribution")
    packages = data.get("packages")
    require(isinstance(packages, dict) and set(packages) == set(TARGETS), "missing exact manifest pairs")
    require(set(archives) == set(pair_names(attempt)), "live archive names do not match the exact attempt")
    descriptors = {}
    for identity, package in packages.items():
        item = package.get("descriptors")
        require(isinstance(item, dict) and set(item) == set(DESCRIPTORS)
                and all(matches(HEX, value) for value in item.values()), f"{identity}: invalid accepted descriptor digests")
        descriptors[identity] = item
    require(data == make_manifest(repo, run, attempt, sha, version, contract, distribution, archives, descriptors),
            "manifest does not match the approved source, exact attempt or live archive identities")
    return descriptors


def extract_archive(archive, target, checksum, expected):
    require("sha256:" + digest(archive) == checksum, f"{target.name}: archive digest mismatch")
    with ZipFile(archive) as zipped:
        members = zipped.infolist()
        require(len(members) == len(expected) and {item.filename for item in members} == set(expected)
                and all(not item.is_dir() and stat.S_IFMT(item.external_attr >> 16) in (0, stat.S_IFREG)
                        for item in members),
                f"{target.name}: unexpected, duplicate or non-regular archive files")
        target.mkdir()
        for item in members:
            with zipped.open(item) as src, (target / item.filename).open("xb") as dst:
                shutil.copyfileobj(src, dst)


def alias_for(name, version):
    prefix = f"intcad-{version}-"
    if name.startswith(prefix) and name.endswith((
        ".AppImage", ".deb", ".rpm", ".flatpak", ".snap", ".msi", "-portable.zip", ".dmg"
    )):
        return "intcad-" + name.removeprefix(prefix)
    return None


def verify_release_files(root, version, *, signed):
    original = set().union(*(asset_names(identity, version) for identity in TARGETS))
    aliases = {alias_for(name, version): name for name in original if alias_for(name, version)}
    metadata = set(SIGNING_ASSETS if signed else SIGNING_ASSETS[:-1])
    files = entries(root, original | set(aliases) | metadata, message="incorrect exact release files or aliases")
    sums = {name: digest(files[name]) for name in original | set(aliases)}
    for alias, name in aliases.items():
        require(sums[alias] == sums[name], f"{alias}: alias bytes do not match {name}")
    expected = "".join(f"{sums[name]}  {name}\n" for name in sorted(sums)).encode()
    require(files[SUMS].read_bytes() == expected and files[LEGACY_SUMS].read_bytes() == expected,
            "checksum lists must be identical and exactly cover all package and alias bytes")


def trusted_key(raw):
    key = raw.decode("ascii").removesuffix("\n")
    require(matches(r"[A-Za-z0-9+/]{56}", key), "production acceptance key is absent or not configured")
    decoded = base64.b64decode(key, validate=True)
    require(len(decoded) == 42 and decoded.startswith(b"Ed"), "invalid minisign production public key")
    return key


def verify_signature(root, tag, key):
    lines = (root / SIGNATURE).read_bytes().split(b"\n")
    require(len(lines) >= 4 and lines[2] == f"trusted comment: Intcad {tag}".encode(),
            "signature does not carry the exact trusted comment")
    subprocess.run(["minisign", "-Vm", str(root / SUMS), "-x", str(root / SIGNATURE), "-P", key], check=True)


def verify_downloaded(out, downloaded):
    """Every local release file must come back from the draft with the same bytes."""
    require(downloaded.is_dir() and not downloaded.is_symlink(), "invalid download folder")
    for folder in (out, downloaded):
        require(all(path.is_file() and not path.is_symlink() for path in folder.iterdir()),
                "release folder must hold regular files only")
    names = {path.name for path in out.iterdir()}
    require({path.name for path in downloaded.iterdir()} == names, "download set differs from the upload")
    for name in sorted(names):
        require(digest(downloaded / name) == digest(out / name),
                f"{name}: downloaded bytes differ from the signed upload")


def verify_draft(draft, root, tag, notes):
    require(positive(draft.get("id")) and draft.get("tag_name") == tag and draft.get("draft") is True
            and draft.get("prerelease") is False and draft.get("body") == notes,
            "release read-back is not the expected stable draft with frozen notes")
    files = {path.name: path for path in root.iterdir()}
    require(all(path.is_file() and not path.is_symlink() for path in files.values())
            and set(SIGNING_ASSETS) <= set(files), "invalid local release asset set")
    assets = draft.get("assets")
    require(isinstance(assets, list), "missing draft assets")
    seen = set()
    for asset in assets:
        name = asset.get("name")
        require(name in files and name not in seen and asset.get("state") == "uploaded", "unexpected draft asset")
        path = files[name]
        require(asset.get("size") == path.stat().st_size and asset.get("digest") == "sha256:" + digest(path),
                f"{name}: draft asset digest missing or does not match")
        seen.add(name)
    require(seen == set(files), "missing draft asset")


def verify_agent_review(distribution, sha, run, attempt, legal, installed):
    if distribution == "mastra-agent":
        execution = f"{sha}:{run}:{attempt}"
        require(legal == execution, "built-in agent distribution needs owner-confirmed npm/Node and file-rights review")
        require(installed == execution, "built-in agent distribution needs owner-confirmed installed-agent acceptance")
    else:
        require(distribution == "native", "unknown or unapproved distribution")


def git(*args):
    return subprocess.check_output(["git", *args])


def checkout_info(sha):
    require(matches(r"[0-9a-f]{40}", sha), "expected immutable 40-hex source SHA")
    cargo = tomllib.loads(git("show", f"{sha}:Cargo.toml").decode("utf-8"))
    return cargo["workspace"]["package"]["version"], sha256(git("show", f"{sha}:{CONTRACT}"))


def frozen_notes(sha, name, version, repo="."):
    parts = PurePosixPath(name).parts
    require(bool(parts) and "/".join(parts) == name
            and all(matches(r"[A-Za-z0-9_.-]+", part) and part not in (".", "..") for part in parts)
            and name.endswith(".md"), "notes must be a repository-relative Markdown path")
    # Only the reviewed release-notes location for this version. Any tracked Markdown
    # would otherwise let internal documentation be published as the update text.
    exact = {"docs/release-notes-" + version + ".md", "docs/release-notes/" + version + ".md"}
    require(name in exact,
            f"notes must be exactly {sorted(exact)}")
    mode = git("-C", repo, "ls-tree", sha, "--", name).split(b" ", 1)[0]
    require(mode in (b"100644", b"100755"), "notes must be a tracked regular file at the frozen SHA")
    notes = git("-C", repo, "show", f"{sha}:{name}").decode("utf-8")
    require(bool(notes.strip()), "release notes are empty")
    require(PRIVATE_SOURCE.casefold() not in notes.casefold(), "release notes refer to the private source repository")
    return notes


def api(*args):
    return parse_json(subprocess.check_output(["gh", "api", *args]))


def pages(endpoint):
    return api("--paginate", "--slurp", endpoint + "?per_page=100")


def run_artifacts(repo, run):
    return [row for page in pages(f"repos/{repo}/actions/runs/{run}/artifacts") for row in page["artifacts"]]


def download(endpoint, path, *, asset=False):
    headers = ["-H", "Accept: application/octet-stream"] if asset else []
    with path.open("xb") as out:
        subprocess.run(["gh", "api", *headers, endpoint], stdout=out, check=True)


def download_artifact(repo, record, target, expected):
    # Stream the API response, then verify the exact archive before extracting.
    with TemporaryDirectory(dir=target.parent) as temp:
        archive = Path(temp) / "artifact.zip"
        download(f"repos/{repo}/actions/artifacts/{record['id']}/zip", archive)
        extract_archive(archive, target, record["digest"], expected)


def gate_candidate(event, payload, repo_id, version):
    if not matches(STABLE, version):
        return False
    if event == "pull_request":
        pr = payload.get("pull_request") or {}
        head, base = pr.get("head") or {}, pr.get("base") or {}
        return (payload.get("action") in ("opened", "reopened", "ready_for_review")
                and pr.get("draft") is False and head.get("ref") == f"release/v{version}"
                and base.get("ref") == "main"
                and all((ref.get("repo") or {}).get("id") == repo_id for ref in (head, base)))
    return (event == "merge_group" and payload.get("action") == "checks_requested"
            and (payload.get("merge_group") or {}).get("base_ref") == "refs/heads/main")


def gate_record(env):
    sha = env["GITHUB_SHA"]
    require(git("rev-parse", "HEAD").decode().strip() == sha, "gate checkout is not GITHUB_SHA")
    version, contract = checkout_info(sha)
    repo_id = number(env["GITHUB_REPOSITORY_ID"])
    payload = parse_json(Path(env["GITHUB_EVENT_PATH"]).read_bytes())
    if not gate_candidate(env["GITHUB_EVENT_NAME"], payload, repo_id, version):
        print("Not a formal stable candidate; no publication handoff")
        return
    require((payload.get("repository") or {}).get("id") == repo_id, "wrong gate event repository")
    repo, run, attempt = env["GITHUB_REPOSITORY"], number(env["GITHUB_RUN_ID"]), number(env["GITHUB_RUN_ATTEMPT"])
    archives = artifact_index(run_artifacts(repo, run), pair_names(attempt))
    evidence = Path("package-evidence")
    need = {pair(identity, attempt)[1] for identity in TARGETS}
    entries(evidence, need, directories=True, message="unexpected exact-attempt gate evidence")
    builds, reports, descriptors = descriptor_index(evidence, version, sha, contract, attempt=attempt)
    # Prove the bytes assessed by the gate came from these exact evidence IDs.
    with TemporaryDirectory() as temp:
        for identity in TARGETS:
            name = pair(identity, attempt)[1]
            folder = Path(temp) / name
            download_artifact(repo, archives[name], folder, set(DESCRIPTORS))
            require(all(digest(folder / desc) == descriptors[identity][desc] for desc in DESCRIPTORS),
                    f"{name}: downloaded archive does not match the assessed descriptors")
    distribution = env["DISTRIBUTION"]
    require(distribution in ("native", "mastra-agent"), "unexpected distribution after gate evaluation")
    manifest = make_manifest(repo_id, run, attempt, sha, version, contract, distribution, archives, descriptors)
    Path("stable-release-handoff.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    with open(env["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        print("eligible=true", file=output)


def authorize(env):
    repo, mode, configured = env["GITHUB_REPOSITORY"], env.get("SOURCE_MODE") or "public", env.get("SOURCE_ID", "")
    repo_id = number(env["GITHUB_REPOSITORY_ID"])
    metadata = api(f"repos/{repo}")
    verify_repository(metadata, configured, mode)
    require(metadata["id"] == repo_id and metadata.get("default_branch") == "main",
            "publisher repository context/default branch mismatch")
    require(env["GITHUB_EVENT_NAME"] == "workflow_dispatch" and env["GITHUB_REF"] == "refs/heads/main"
            and env["PROTECTED_REF"] == "true", "publisher requires dispatch from protected main")
    return repo, repo_id


def verify_environment(environment, policies):
    rules = [rule for rule in environment.get("protection_rules") or []
             if rule.get("type") == "required_reviewers"]
    require(environment.get("name") == "release-signing" and len(rules) == 1
            and rules[0].get("prevent_self_review") is True and bool(rules[0].get("reviewers"))
            and environment.get("can_admins_bypass") is False,
            "release-signing must require independent reviewers and forbid admin bypass")
    policy = environment.get("deployment_branch_policy") or {}
    require(policy.get("protected_branches") is False and policy.get("custom_branch_policies") is True
            and [(row.get("name"), row.get("type")) for row in policies] == [("main", "branch")],
            "release-signing must allow only the exact main branch, not tags or other branches")


def check_environment(env):
    repo = env["GITHUB_REPOSITORY"]
    endpoint = f"repos/{repo}/environments/release-signing"
    verify_environment(api(endpoint), [row for page in pages(f"{endpoint}/deployment-branch-policies")
                                       for row in page["branch_policies"]])


def live_execution(repo, repo_id, run, attempt):
    prefix = f"repos/{repo}/actions/runs/{run}"
    jobs = [job for page in pages(f"{prefix}/attempts/{attempt}/jobs") for job in page["jobs"]]
    # Use the latest run endpoint after reading attempt-scoped jobs. An earlier
    # attempt no longer authorizes signing once a rerun has started.
    verify_execution(api(prefix), jobs, run, attempt, repo_id)


# Every executable input the gate and package jobs use. Comparing the whole trees,
# not a hand-picked file list, closes the gap where one unpinned helper (a planner,
# a candidate classifier, a packaging script) could fake the evidence.
PRODUCER_TREES = ("scripts", ".github/workflows", "packaging", "tools")


def require_producers(sha, repo="."):
    """The gate evidence must have been produced by the protected main revision.

    The gate runs the candidate checkout, so a candidate could otherwise rewrite any
    script or workflow to emit self-consistent evidence. The publisher re-checks the
    tagged trees against its own protected-main checkout before it trusts the handoff.
    A change that is already merged into main is outside this boundary: main is the
    reviewed trust root.
    """
    for tree in PRODUCER_TREES:
        frozen = git("-C", repo, "ls-tree", "-r", "--full-tree", sha, "--", tree)
        protected = git("-C", repo, "ls-tree", "-r", "--full-tree", "HEAD", "--", tree)
        require(frozen == protected, f"{tree}: the release candidate changed the evidence producer")


def selection(env, *, resolve_tag=False):
    repo, repo_id = authorize(env)
    version, run, attempt = dispatch(env["TAG"], env["RUN_ID"], env["RUN_ATTEMPT"])
    sha = env["SOURCE_SHA"]
    source_version, contract = checkout_info(sha)
    require(source_version == version, "tag does not match the frozen workspace version")
    if resolve_tag:
        tag = git("rev-parse", "--verify", f"refs/tags/{env['TAG']}^{{commit}}").decode().strip()
        require(tag == sha, "source tag does not point to the frozen source SHA")
        # The released commit must already be on the protected branch this dispatch runs from:
        # a tag on a private or unmerged commit is not a promoted release.
        require(subprocess.run(["git", "merge-base", "--is-ancestor", sha, "HEAD"],
                               capture_output=True).returncode == 0,
                "source SHA is not on the protected main branch")
    live_execution(repo, repo_id, run, attempt)
    return repo, repo_id, run, attempt, sha, version, contract


def prepare(env):
    repo, ident, run, attempt, sha, version, contract = selection(env, resolve_tag=True)
    require_producers(sha)
    notes = frozen_notes(sha, env["NOTES_PATH"], version)
    key = trusted_key(git("show", f"{sha}:{PUBLIC_KEY}"))
    work = Path("release-work")
    work.mkdir()
    live = artifact_index(run_artifacts(repo, run), (*pair_names(attempt), manifest_name(attempt)))
    handoff = work / "handoff"
    download_artifact(repo, live[manifest_name(attempt)], handoff, {"stable-release-handoff.json"})
    manifest = parse_json((handoff / "stable-release-handoff.json").read_bytes())
    descriptors = verify_manifest(manifest, ident, run, attempt, sha, version, contract,
                                  {name: live[name] for name in pair_names(attempt)})
    distribution = manifest["distribution"]
    verify_agent_review(distribution, sha, run, attempt, env.get("AGENT_LEGAL_APPROVED"),
                        env.get("AGENT_INSTALLED_APPROVED"))
    staging = work / "staging"
    staging.mkdir()
    for identity in sorted(TARGETS):
        package, evidence = pair(identity, attempt)
        download_artifact(repo, live[evidence], staging / evidence, set(DESCRIPTORS))
        download_artifact(repo, live[package], staging / package, asset_names(identity, version) | set(DESCRIPTORS))
    stage(staging, work / "out", version, sha, contract, descriptors, attempt=attempt, distribution=distribution)
    (work / "notes.md").write_bytes(notes.encode())
    (work / "acceptance-key.pub").write_text(key, encoding="ascii")
    (work / "candidate.json").write_text(json.dumps(dict(
        source=repo, repositoryId=ident, runId=run, runAttempt=attempt, sourceRevision=sha, version=version,
        notesSha256=sha256(notes.encode()), archives=live,
    )) + "\n", encoding="utf-8")


def confirm(env):
    candidate = parse_json(Path("release-work/candidate.json").read_bytes())
    repo, ident = authorize(env)
    require(candidate["source"] == repo and candidate["repositoryId"] == ident, "frozen source changed")
    run, attempt = candidate["runId"], candidate["runAttempt"]
    live_execution(repo, ident, run, attempt)
    live = artifact_index(run_artifacts(repo, run), (*pair_names(attempt), manifest_name(attempt)))
    require(live == candidate["archives"], "accepted archive identities changed before signing/staging")


def assert_tag_absent(tag):
    result = subprocess.run(["gh", "api", "--include", f"repos/{DIST_REPO}/git/ref/tags/{tag}"],
                            capture_output=True)
    status = result.stdout.splitlines()[0] if result.stdout else b""
    require(result.returncode != 0 and re.fullmatch(rb"HTTP/\S+ 404(?: .*)?", status),
            "distribution tag already exists or its absence cannot be confirmed")


def upload_asset(draft_id, path, token):
    # Pin the upload to the new draft ID, never re-resolve a mutable tag.
    url = f"https://uploads.github.com/repos/{DIST_REPO}/releases/{draft_id}/assets?" + urlencode({"name": path.name})
    subprocess.run(["curl", "--fail", "--silent", "--show-error", "--output", os.devnull,
                    "--config", "-", "--request", "POST", "--header", "Content-Type: application/octet-stream",
                    "--data-binary", "@" + str(path), url],
                   input=f'header = "Authorization: Bearer {token}"\n', text=True, check=True)


def draft_release(env, work, candidate):
    tag = "v" + candidate["version"]
    out = work / "out"
    notes = (work / "notes.md").read_bytes()
    require(sha256(notes) == candidate["notesSha256"], "frozen notes changed")
    key = trusted_key((work / "acceptance-key.pub").read_bytes())
    verify_release_files(out, candidate["version"], signed=True)
    verify_signature(out, tag, key)
    destination = api(f"repos/{DIST_REPO}")
    require(destination.get("full_name") == DIST_REPO and destination.get("visibility") == "public"
            and destination.get("default_branch") == "main", "unexpected distribution repository")
    release_endpoint = f"repos/{DIST_REPO}/releases"
    require(not any(release.get("tag_name") == tag for page in pages(release_endpoint) for release in page),
            "distribution release or draft already exists")
    assert_tag_absent(tag)
    create = dict(tag_name=tag, target_commitish="main", name=f"Intcad {candidate['version']}",
                  body=notes.decode(), draft=True, prerelease=False)
    body = work / "draft-request.json"
    body.write_text(json.dumps(create), encoding="utf-8")
    created = api("-X", "POST", "--input", str(body), release_endpoint)
    draft_id = created.get("id")
    require(positive(draft_id) and created.get("draft") is True and created.get("tag_name") == tag
            and created.get("target_commitish") == "main",
            "GitHub did not return the new draft")
    print(f"Created draft ID {draft_id}; leave it unpublished until all read-back checks pass", flush=True)
    for path in sorted(out.iterdir()):
        upload_asset(draft_id, path, env["GH_TOKEN"])
    draft = api(f"{release_endpoint}/{draft_id}")
    require(draft.get("id") == draft_id and draft.get("target_commitish") == "main", "unexpected draft identity")
    draft["assets"] = [row for page in pages(f"{release_endpoint}/{draft_id}/assets") for row in page]
    verify_draft(draft, out, tag, notes.decode())
    downloaded = work / "downloaded"
    downloaded.mkdir()
    for asset in draft["assets"]:
        # Every asset is read back, not only the signing files: metadata digests can
        # agree while the stored bytes do not.
        name = asset["name"]
        require(positive(asset.get("id")), "invalid downloaded release asset ID")
        download(f"{release_endpoint}/assets/{asset['id']}", downloaded / name, asset=True)
    verify_downloaded(out, downloaded)
    verify_signature(downloaded, tag, key)
    with open(env["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
        print(f"### Verified draft {tag}\n"
              f"\n- Distribution: `{DIST_REPO}` (draft ID `{draft_id}`)"
              f"\n- Source revision: `{candidate['sourceRevision']}`"
              f"\n- Release gate: `{candidate['runId']}` attempt `{candidate['runAttempt']}`"
              f"\n- Asset digests, both checksum lists and signature verified; not published", file=summary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("record", "environment", "prepare", "checksums", "signature", "confirm", "draft"))
    args = parser.parse_args()
    env = os.environ
    try:
        if args.command == "record":
            gate_record(env)
        elif args.command == "environment":
            check_environment(env)
        elif args.command == "prepare":
            prepare(env)
        elif args.command == "checksums":
            verify_release_files(Path("release-work/out"), env["TAG"][1:], signed=False)
        elif args.command == "signature":
            work = Path("release-work")
            verify_signature(work / "out", env["TAG"], trusted_key((work / "acceptance-key.pub").read_bytes()))
        elif args.command == "confirm":
            confirm(env)
        elif args.command == "draft":
            work = Path("release-work")
            draft_release(env, work, parse_json((work / "candidate.json").read_bytes()))
    except (ValueError, OSError, KeyError, TypeError, AttributeError, BadZipFile, subprocess.CalledProcessError) as error:
        sys.exit(f"Release handoff refused: {error}")


if __name__ == "__main__":
    main()
