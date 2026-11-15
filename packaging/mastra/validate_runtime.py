#!/usr/bin/env python3
"""Validate a staged Mastra runtime before a package build may copy it.

The artifact gate (`cargo xtask runtime-artifact --mode artifact`) owns approval, license and
exact-byte provenance checks. This packaging-side check binds the receipt to the repository
revision and contract/policy/source digests used for the same package build, rejects an
incomplete tree, and asserts that no Codex executable or credential was bundled.

The current artifact is unapproved (#15), so this script is expected to run only after the
approval gate passes. It never approves anything itself.
"""
import argparse
import hashlib
import json
import os
import sys

SUPPORTED = {
    "x86_64-pc-windows-msvc": "node.exe",
    "aarch64-apple-darwin": "node",
    "x86_64-apple-darwin": "node",
    "x86_64-unknown-linux-gnu": "node",
}
REQUIRED = [
    "runtime.json",
    "dist/src/main.js",
    "package.json",
    "inventory.json",
    "schema/mastra-runtime/v1/manifest.json",
    "licenses/LICENSES.txt",
    "notices/NOTICE.txt",
]
CODEX_NAMES = {"codex-acp", "codex-acp.exe", "codex-acp.cmd", "codex-acp.bat", "codex", "codex.exe", "auth.json"}


def fail(message):
    print(f"error: {message}", file=sys.stderr)
    return 1


def digest(path):
    hasher = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(block)
    return "sha256:" + hasher.hexdigest()


def relative(path, root):
    return os.path.relpath(path, root).replace(os.sep, "/")


def validate(runtime, target, approval, policy_path, revision, contract, policy_digest, sources, runtime_version, node_version):
    if target not in SUPPORTED:
        return fail(f"{target} is not a supported Mastra target; supported: {', '.join(sorted(SUPPORTED))}")
    receipt_path = os.path.join(runtime, "runtime.json")
    if not os.path.isfile(receipt_path):
        return fail(f"{runtime} has no runtime.json; the artifact gate did not produce a receipt")
    try:
        with open(policy_path, encoding="utf-8") as handle:
            policy = json.load(handle)
    except (OSError, ValueError) as error:
        return fail(f"cannot read the reviewed policy {policy_path}: {error}")
    try:
        with open(receipt_path, encoding="utf-8") as handle:
            receipt = json.load(handle)
    except (OSError, ValueError) as error:
        return fail(f"runtime.json is not valid JSON: {error}")
    if not isinstance(receipt, dict):
        return fail("runtime.json must be a JSON object")

    checks = [
        ("schemaVersion", "1.0"),
        ("protocolVersions", ["1.0"]),
        ("targetTriple", target),
        ("repositoryRevision", revision),
        ("contractDigest", contract),
        ("policyDigest", policy_digest),
        ("sourceRecordsDigest", sources),
        ("runtimeVersion", runtime_version),
        ("nodeVersion", node_version),
    ]
    for key, expected in checks:
        if receipt.get(key) != expected:
            return fail(f"runtime.json {key} is {receipt.get(key)!r}, expected {expected!r}")

    inventory = receipt.get("provenanceInventory")
    if not isinstance(inventory, dict) or inventory.get("path") != "inventory.json":
        return fail("runtime.json provenanceInventory.path must be inventory.json")
    inventory_path = os.path.join(runtime, "inventory.json")
    if not os.path.isfile(inventory_path):
        return fail("inventory.json is missing")
    actual = digest(inventory_path)
    if actual != inventory.get("digest"):
        return fail(f"inventory.json digest {actual} does not match runtime.json {inventory.get('digest')}")

    # Admission is approval-aware: a complete tree whose inventory the reviewed policy has not
    # approved must never be packaged, even if a test seam replaced the xtask gate.
    approvals = policy.get("approvals")
    if not isinstance(approvals, list):
        return fail("policy.approvals must be an array")
    approved = any(
        isinstance(item, dict)
        and item.get("id") == approval
        and item.get("target") == target
        and item.get("inventoryDigest") == actual
        for item in approvals
    )
    if not approved:
        return fail(
            f"inventory {actual} has no matching approval {approval!r} for {target} in {policy_path}; "
            "the artifact is not releasable and must not be packaged (see #15)"
        )

    for required in REQUIRED:
        if not os.path.isfile(os.path.join(runtime, required)):
            return fail(f"runtime tree is incomplete: {required} is missing")
    node = SUPPORTED[target]
    if not os.path.isfile(os.path.join(runtime, node)):
        return fail(f"runtime tree is incomplete: {node} is missing")
    if not os.path.isdir(os.path.join(runtime, "node_modules")):
        return fail("runtime tree is incomplete: node_modules is missing")

    artifact = receipt.get("artifactDigests")
    if not isinstance(artifact, list):
        return fail("runtime.json artifactDigests must be an array")
    recorded = {entry.get("path") for entry in artifact if isinstance(entry, dict)}
    for required in REQUIRED + [node]:
        if required in {"runtime.json", "inventory.json"}:
            continue
        if required not in recorded:
            return fail(f"runtime.json artifactDigests omits required file {required}")

    # No Codex executable or credential may be in the package. Codex is discovered from the
    # user's own installation, never bundled.
    for root, directories, files in os.walk(runtime):
        directories[:] = sorted(directories)
        for name in sorted(files):
            path = os.path.join(root, name)
            if os.path.islink(path):
                return fail(f"runtime tree contains a symlink: {relative(path, runtime)}")
            clamped = name.lower()
            if clamped in CODEX_NAMES or (clamped.startswith("codex") and clamped.endswith((".exe", ".cmd", ".bat", ".ps1"))):
                return fail(f"runtime tree bundles Codex ({relative(path, runtime)}); Codex is discovered, not bundled")
            parts = relative(path, runtime).split("/")
            lower = [part.lower() for part in parts]
            if any(part == "ee" for part in lower):
                return fail(f"runtime tree contains an excluded ee/ path: {relative(path, runtime)}")
            if any(part in {"codex-acp", ".codex"} or part.startswith("codex-acp.") for part in lower):
                return fail(f"runtime tree bundles Codex ({relative(path, runtime)}); Codex is discovered, not bundled")
            if parts[-1].lower() in {"auth.json", ".credentials.json"} and ".codex" in lower:
                return fail(f"runtime tree bundles Codex credentials: {relative(path, runtime)}")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--policy", required=True)
    parser.add_argument("--approval", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--contract-digest", required=True)
    parser.add_argument("--policy-digest", required=True)
    parser.add_argument("--sources-digest", required=True)
    parser.add_argument("--runtime-version", required=True)
    parser.add_argument("--node-version", required=True)
    args = parser.parse_args()
    if not os.path.isdir(args.runtime):
        return fail(f"{args.runtime} is not a directory")
    return validate(
        args.runtime,
        args.target,
        args.approval,
        args.policy,
        args.revision,
        args.contract_digest,
        args.policy_digest,
        args.sources_digest,
        args.runtime_version,
        args.node_version,
    )


if __name__ == "__main__":
    sys.exit(main())
