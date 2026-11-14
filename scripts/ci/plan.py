#!/usr/bin/env python3
"""Explicit CI applicability; inventory is presence evidence, never test-success evidence.

Use git diff --no-renames --name-only -z BASE...HEAD for --paths so both rename
names and deletions survive. An unavailable comparison requires --diff-incomplete.
This utility does not install, execute or manufacture future consumers.
CLI exits 0 for an unblocked plan (not a test pass), 1 after writing a blocked
plan, or 2 for invalid/unavailable inputs without an explicit conservative mode.
"""
import argparse
import json
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import affected  # noqa: E402

HOST = "crates/ui-egui/src/agent_chat/"
CONTRACT_INPUTS = ("schema/mastra-runtime/v1/manifest.json", HOST + "mastra_protocol.rs",
                   HOST + "runtime_client.rs", HOST + "tests.rs")
RUNTIME_INPUTS = ("package.json", "packages/mastra-runtime/package.json",
                  "packages/mastra-runtime/src/main.ts", "packages/mastra-runtime/tsconfig.json")
NODE_PINS = (".node-version", ".nvmrc")
NODE_LOCKS = ("package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lock", "bun.lockb")
NODE_FILES = (*NODE_PINS, *NODE_LOCKS, "package.json", "pnpm-workspace.yaml", ".npmrc")
VERIFIER_ENTRY = "xtask/src/main.rs"
# Repository-tooling inputs run the xtask tests. Their Rust changes map to the
# xtask package already; packaging/mastra metadata is ignored by the Rust
# selector, so it needs an explicit owner.
TOOLING_PREFIXES = ("xtask/", "packaging/mastra/")
# The bounded pull-request test lane never compiles the whole workspace. This
# smoke owner is always present and proves the toolchain even for shared-only
# inputs whose tests belong to the nightly full run.
PR_TEST_SMOKE = "xtask"
# Deterministic slice of the bounded PR test selection.
PR_TEST_SHARD = "count:1/8"
# Packages whose test closure fits a cold 2/4-vCPU PR runner. Everything else
# (UI/app/engine closure) needs full test codegen that does not fit 30 minutes;
# its tests are explicitly deferred to the nightly full workflow, which blocks
# release. Clippy and wasm still build every selected consumer on the PR.
PR_TEST_PACKAGES = frozenset(("xtask", "solidcraft-derive", "solidcraft-doc", "solidcraft-geom",
                              "solidcraft-io", "solidcraft-profile", "solidcraft-sketch",
                              "solidcraft-step", "solidcraft-testkit"))
NIGHTLY_WORKFLOW = ".github/workflows/native-baseline.yml"
# The browser offline/update harness: the web app and its packaging, the Pages
# workflow that runs it against a real build, and the xtask implementation that
# drives Chrome. It is a separate job so its bounded Chrome launches do not share
# the repository-checks budget. An unrelated xtask change must not start it.
WEB_OFFLINE_PREFIXES = ("apps/solidcraft-web/", "packaging/web/")
WEB_OFFLINE_FILES = (".github/workflows/pages.yml", "xtask/src/main.rs",
                     "xtask/src/web_offline.rs", "xtask/src/web_offline_test.rs")
# Only these reviewed metadata/owning utility inputs are payload-free. Unknown
# packaging paths still require the real package consumers, never a prefix waiver.
PACKAGING_FOUNDATION = frozenset(("packaging/mastra/policy.json", "packaging/mastra/source-records.json",
                                  "packaging/tests/apprun_test.py", "packaging/tests/flatpak_test.py",
                                  "packaging/tests/nightly_test.py", "packaging/tests/windows_icon_test.py"))


def runtime_path(path):
    return path.startswith("packages/mastra-runtime/") or path in NODE_FILES


def contract_path(path):
    return (path.startswith("schema/mastra-runtime/") or path == "docs/mastra-runtime.md"
            or path.startswith(tuple(HOST + name for name in
                                     ("host_ai", "mastra_protocol", "runtime_client", "process"))))


def shared_path(path):
    return (path in affected.FULL_FILES or path.startswith("rust-toolchain")
            or path.startswith((".cargo/", ".config/", ".github/workflows/", ".github/actions/"))
            or (path.startswith("scripts/") and not path.startswith(("scripts/claim/", "scripts/triage/", "scripts/ci/"))))


def policy_path(path):
    return (path in ("AGENTS.md", "CONTRIBUTING.md", "deny.toml", "ATTRIBUTION.md", "NOTICE")
            or path.startswith(("packaging/", "attribution/", "xtask/", "skills/solidcraft-ground-rules/"))
            or (path.startswith("docs/") and any(word in path for word in ("policy", "provenance", "licens", "rights"))))


def verifier_present(root):
    """Recognize #15's reachable xtask mode entry points, not a competing verifier CLI."""
    pending, seen = [root / VERIFIER_ENTRY], set()
    while pending:
        path = pending.pop()
        if path in seen or not path.is_file():
            continue
        seen.add(path)
        text = path.read_text(encoding="utf-8")
        text = re.sub(r"//[^\n]*|/\*.*?\*/", "", text, flags=re.S)
        if all(re.search('"' + mode + r'"\s*\)?\s*=>', text) for mode in ("policy", "host-only", "artifact")):
            return True
        for module in re.findall(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;", text):
            directory = path.parent if path.name in ("main.rs", "mod.rs") else path.with_suffix("")
            pending.extend((directory / (module + ".rs"), directory / module / "mod.rs"))
    return False


def detect_inventory(root, paths):
    """Inspect checkout entry points/pins. Missing future code remains an obligation."""
    root = Path(root)

    def exists(path):
        return (root / path).is_file() and (root / path).stat().st_size > 0

    def manifest(path):
        try:
            data = json.loads((root / path).read_text(encoding="utf-8"))
            return data if isinstance(data, dict) else {}
        except (OSError, ValueError):
            return {}

    missing_contract = {path for path in CONTRACT_INPUTS if not exists(path)}
    if exists(HOST + "tests.rs"):
        tests = (root / (HOST + "tests.rs")).read_text(encoding="utf-8")
        tests = re.sub(r"//[^\n]*|/\*.*?\*/", "", tests, flags=re.S)
        if not all(re.search(r"#\[test\]\s*(?:#\[[^\n]*\]\s*)*fn\s+mastra_" + group + r"_\w+\s*\(", tests)
                   for group in ("values", "codec", "bundle", "state", "retirement", "peer")):
            missing_contract.add(HOST + "tests.rs")
    missing_runtime = {path for path in RUNTIME_INPUTS if not exists(path)}
    pins = [path for path in NODE_PINS if exists(path)]
    if not pins:
        missing_runtime.update([path for path in NODE_PINS if path in paths] or [NODE_PINS[0]])
    for path in pins:
        if not re.fullmatch(r"v?\d+\.\d+\.\d+\s*", (root / path).read_text(encoding="utf-8")):
            missing_runtime.add(path)
    workspace = manifest("package.json")
    manager = str(workspace.get("packageManager", "")).split("@", 1)[0]
    locks = {"npm": ("package-lock.json",), "pnpm": ("pnpm-lock.yaml",),
             "yarn": ("yarn.lock",), "bun": ("bun.lock", "bun.lockb")}.get(manager, (NODE_LOCKS[0],))
    if not any(exists(path) for path in locks):
        missing_runtime.add(locks[0])
    if manager == "pnpm" and not exists("pnpm-workspace.yaml"):
        missing_runtime.add("pnpm-workspace.yaml")
    workspaces = workspace.get("workspaces", [])
    if isinstance(workspaces, dict):
        workspaces = workspaces.get("packages", [])
    if (workspace.get("private") is not True or not isinstance(workspaces, list)
            or not any(path in workspaces for path in ("packages/mastra-runtime", "packages/*"))
            or not re.fullmatch(r"(?:npm|pnpm|yarn|bun)@\d+\.\d+\.\d+(?:\+[^\s]+)?", str(workspace.get("packageManager", "")))):
        missing_runtime.add("package.json")
    runtime = manifest("packages/mastra-runtime/package.json")
    scripts = runtime.get("scripts", {})
    if (runtime.get("name") != "@intgral-ai/solidcraft-mastra-runtime" or runtime.get("private") is not True
            or not isinstance(scripts, dict) or not all(isinstance(scripts.get(key), str) and scripts[key].strip()
                                                       for key in ("typecheck", "test", "build"))):
        missing_runtime.add("packages/mastra-runtime/package.json")
    partial_runtime = missing_runtime and ((root / "packages/mastra-runtime").exists()
                                          or any((root / path).exists() for path in NODE_FILES))
    payload = any(runtime_path(path) for path in paths) or bool(partial_runtime)
    return dict(contractPresent=not missing_contract, runtimePresent=not missing_runtime,
                runtimePayloadRequested=payload, artifactVerifierPresent=verifier_present(root),
                missingContractInputs=sorted(missing_contract), missingRuntimeInputs=sorted(missing_runtime))


def make_plan(meta: dict, paths: list[str], inventory: dict, *, source_revision: str,
              event_name: str, base_ref: str, head_ref: str, full: bool,
              release_required: bool, diff_complete: bool) -> dict:
    """Produce version 1 decisions. Blocked/missing beats release, full and path selection."""
    if not re.fullmatch(r"[0-9a-f]{40}", source_revision):
        raise ValueError("source_revision must be 40 lowercase hex characters")
    for key in ("contractPresent", "runtimePresent", "runtimePayloadRequested", "artifactVerifierPresent"):
        if type(inventory[key]) is not bool:
            raise ValueError(key + " must be a boolean")
    for key in ("missingContractInputs", "missingRuntimeInputs"):
        if not isinstance(inventory[key], list) or not all(isinstance(p, str) and p for p in inventory[key]):
            raise ValueError(key + " must be a list of paths")

    contract_missing = sorted(set(inventory["missingContractInputs"]))
    if not inventory["contractPresent"] and not contract_missing:
        contract_missing = list(sorted(CONTRACT_INPUTS))
    # A remaining contract entry point (including the checked Mastra test groups)
    # proves partial implementation. A legacy shared tests.rs alone does not.
    contract_required = (inventory["contractPresent"] or inventory["runtimePresent"]
                         or any(path not in contract_missing for path in CONTRACT_INPUTS))
    contract_change = (any(contract_path(path) for path in paths)
                       or (contract_required and HOST + "tests.rs" in paths))
    runtime_request = inventory["runtimePayloadRequested"] or any(runtime_path(path) for path in paths)
    shared = any(shared_path(path) for path in paths)
    policy = any(policy_path(path) for path in paths)
    packaging = any(path.startswith("packaging/") and (not diff_complete or path not in PACKAGING_FOUNDATION)
                    for path in paths)
    # Keep the existing Rust selector and its embedded/test-data exceptions as the
    # sole owner of Rust reverse dependencies. Map interface inputs to their host.
    rust_paths = [path for path in paths if not runtime_path(path)
                  and not path.startswith("schema/mastra-runtime/") and path != "docs/mastra-runtime.md"]
    # A release branch reconciled back into develop carries only the release
    # version label. It is not a shared code change; the always-running
    # repository checks validate the files, and the candidate build already
    # exercised the same tree. Any other path, base or head keeps the normal
    # conservative selector.
    release_version_only = (
        base_ref == "develop" and head_ref.startswith("release/v")
        and bool(rust_paths) and set(rust_paths) <= {"Cargo.toml", "Cargo.lock"}
    )
    if release_version_only:
        rust_paths = []
        shared = False
    rust_full, rust_packages = affected.select(meta, rust_paths)
    available = {p["name"] for p in meta["packages"]}
    # xtask owns the repository/runtime artifact checks. A packaging/mastra
    # metadata change must select its tests even though packaging/ is not a
    # Rust package.
    tooling = any(path.startswith(TOOLING_PREFIXES) for path in paths) and PR_TEST_SMOKE in available
    if tooling and not rust_full:
        rust_packages = sorted(set(rust_packages) | {PR_TEST_SMOKE})
    unknown = any(affected.select(meta, [path])[0] for path in rust_paths if not shared_path(path))
    full = full or event_name == "schedule" or not diff_complete or unknown
    rust_full = rust_full or shared or full or release_required
    if (contract_change or runtime_request) and not rust_full:
        _, rust_packages = affected.select(meta, rust_paths + [HOST + "mastra_protocol.rs"])
    if rust_full:
        rust_packages = sorted(p["name"] for p in meta["packages"])
    # Pull requests run a bounded, deterministic lane: the packages that directly
    # own a changed Rust file plus the tooling smoke owner. Reverse dependents are
    # still built by Clippy/wasm and fully tested by the nightly/full workflow;
    # including their test binaries overflowed the 30-minute PR cap. Only this
    # counted test lane is bounded. Excluded tests are never reported as passing.
    direct_paths = [path for path in rust_paths if not shared_path(path)]
    root = meta["workspace_root"].replace("\\", "/").rstrip("/") + "/"
    dirs = {p["name"]: p["manifest_path"].replace("\\", "/")[len(root):].rsplit("/", 1)[0] + "/"
            for p in meta["packages"]}
    direct_tests = set()
    for path in direct_paths:
        owners = [name for name, directory in dirs.items() if path.startswith(directory)]
        if owners:
            direct_tests.add(max(owners, key=lambda name: len(dirs[name])))
    if tooling:
        direct_tests = set(direct_tests) | {PR_TEST_SMOKE}
    if full or release_required:
        # Full/nightly/release executes the complete workspace; no test deferral.
        rust_test_packages = sorted(set(direct_tests))
        rust_deferred_packages = []
    else:
        rust_test_packages = sorted(p for p in direct_tests if p in PR_TEST_PACKAGES)
        # Every selected consumer whose tests are not in this bounded receipt is
        # explicitly deferred to the nightly full workflow, not silently treated
        # as passing.
        rust_deferred_packages = sorted((set(direct_tests) | set(rust_packages)) - set(rust_test_packages))
    if not rust_test_packages and (rust_packages or rust_full):
        fallback = [PR_TEST_SMOKE] if PR_TEST_SMOKE in available else sorted(rust_packages)
        rust_test_packages = fallback[:1] or sorted(available)[:1]
        rust_deferred_packages = sorted(set(rust_deferred_packages) - set(rust_test_packages))
    selected = full or shared or contract_change or runtime_request
    # Current consumers do not disappear when a required input goes missing.
    required_contract_missing = bool(contract_missing) and contract_required
    runtime_missing = sorted(set(inventory["missingRuntimeInputs"]))
    if not inventory["runtimePresent"] and not runtime_missing:
        runtime_missing = sorted((*RUNTIME_INPUTS, NODE_PINS[0], NODE_LOCKS[0]))
    verifier_missing = [] if inventory["artifactVerifierPresent"] else [VERIFIER_ENTRY]
    jobs, obligations = {}, []

    def decide(job, wanted, missing=(), absent_reason="no_relevant_change"):
        missing = sorted(set(missing))
        if wanted:
            decision = "blocked" if missing else "run"
            reason = ("missing_required_consumer" if missing else "release_required" if release_required
                      else "full_validation" if full else "selected_change")
        else:
            decision, reason = "not_applicable", absent_reason if missing else "no_relevant_change"
            if missing and absent_reason != "no_relevant_change":
                obligations.append(job)
            missing = []
        jobs[job] = dict(decision=decision, reason=reason, missingInputs=missing)

    jobs["repo"] = dict(decision="run", reason="always", missingInputs=[])
    pr = event_name in ("pull_request", "pull_request_target", "merge_group")
    jobs["pr-policy"] = dict(decision="run" if pr else "not_applicable",
                             reason="always" if pr else "no_relevant_change", missingInputs=[])
    decide("rust", bool(rust_packages) or rust_full)
    decide("wasm", bool(rust_packages) or rust_full)
    # The offline/update check runs on full/release validation, and on the web app,
    # its packaging, the harness implementation and its Pages caller. Shared CI
    # inputs (rust_full) run it too, because its job definition can change there.
    # An ordinary Rust/wasm change runs the wasm build, not the browser harness.
    web_offline = (full or release_required or rust_full
                   or any(path in WEB_OFFLINE_FILES or path.startswith(WEB_OFFLINE_PREFIXES) for path in paths))
    decide("web-offline", web_offline)
    decide("contract-rust", release_required or contract_change or runtime_request
           or (inventory["contractPresent"] and selected) or required_contract_missing,
           contract_missing, "contract_not_implemented_host_foundation")
    runtime_wanted = release_required or runtime_request or (inventory["runtimePresent"] and (selected or bool(runtime_missing)))
    decide("runtime", runtime_wanted, runtime_missing, "runtime_not_implemented_host_foundation")
    for job in ("contract-ts", "integration"):
        decide(job, runtime_wanted or (inventory["runtimePresent"] and required_contract_missing), contract_missing + runtime_missing,
               "runtime_not_implemented_host_foundation" if runtime_missing else "contract_not_implemented_host_foundation")
    decide("artifact-policy", release_required or runtime_request
           or (inventory["artifactVerifierPresent"] and (selected or policy)), verifier_missing,
           "verifier_not_implemented_host_foundation")
    decide("packages", release_required or packaging, contract_missing + runtime_missing + verifier_missing)
    # A checkout helper is run by the platform workflow that owns it, not by the
    # workspace test lane. Only explicitly named tools get here; an unknown tool
    # still selects the full workspace.
    windows_paths = sorted(path for path in paths if affected.PLATFORM_TOOLS.get(path) == "windows")
    platform = {"windows": {
        "decision": "run" if full or windows_paths else "not_applicable",
        "reason": "full_validation" if full else "selected_change" if windows_paths else "no_relevant_change",
        "paths": windows_paths,
    }}
    return dict(version=1, sourceRevision=source_revision, eventName=event_name,
                baseRef=base_ref, headRef=head_ref, scope="full" if full else "affected",
                releaseRequired=release_required, rustFull=rust_full,
                rustPackages=sorted(set(rust_packages)),
                rustTestPackages=sorted(set(rust_test_packages)),
                rustTestShard=PR_TEST_SHARD,
                rustDeferredPackages=sorted(set(rust_deferred_packages)),
                rustDeferredTo=NIGHTLY_WORKFLOW if rust_deferred_packages else "",
                jobs=jobs, platform=platform, openObligations=sorted(obligations))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("paths", "metadata", "source-revision", "event", "base-ref", "head-ref", "out"):
        parser.add_argument("--" + flag, required=True)
    for flag in ("full", "release-required", "diff-incomplete"):
        parser.add_argument("--" + flag, action="store_true")
    args = parser.parse_args()
    try:
        try:
            paths = affected.read_paths(args.paths)
        except OSError:
            if not args.diff_incomplete:
                raise
            paths = []
        meta = affected.load_metadata(args.metadata)
        result = make_plan(meta, paths, detect_inventory(Path(meta["workspace_root"]), paths),
                           source_revision=args.source_revision, event_name=args.event, base_ref=args.base_ref,
                           head_ref=args.head_ref, full=args.full, release_required=args.release_required,
                           diff_complete=not args.diff_incomplete)
        Path(args.out).write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        if any(job["decision"] == "blocked" for job in result["jobs"].values()):
            parser.exit(1, "plan: blocked applicability; see output missingInputs\n")
    except (OSError, ValueError, KeyError, AssertionError) as error:
        parser.exit(2, f"plan: {error}\n")


if __name__ == "__main__":
    main()
