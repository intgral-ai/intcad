#!/usr/bin/env python3
"""Coverage check for the INT-1454 upstream-sync ledger.

Every non-merge commit in FORK..TARGET must belong to at least one ledger area.
The areas partition by path; a commit that touches several areas is counted in each,
so area counts overlap and do not sum to the range.

Usage:
    python3 coverage.py [target]            # default target: v0.44.1
    python3 coverage.py --emit OUT.tsv      # also write the per-commit inventory
Exit status is 1 when a commit belongs to no area.
"""

import subprocess
import sys
import collections

args = [a for a in sys.argv[1:]]
EMIT = None
if "--emit" in args:
    i = args.index("--emit")
    EMIT = args[i + 1]
    del args[i:i + 2]
FORK = "06749ac4"
TARGET = args[0] if args else "v0.44.1"

# Pathspecs per area. A pathspec may be a directory or a file.
AREAS = {
    "kernel": [
        "solidcore",
        "crates/kernel", "crates/smlib", "crates/geom", "crates/dynamics",
        "tests/kernel-cases", "tests/oracle", "tests/action_matrix",
        "tests/hole_matrix", "tests/taper", "tests/ssi-corpus", "tests/projection",
        "examples/regressions", "xtask/mesh-decisions.txt",
    ],
    "doc-engine": [
        "crates/doc", "crates/format", "crates/engine", "crates/sketch",
        "crates/io", "crates/step", "tests/step-conformance",
        "tests/step-regressions", "tests/external-corpus.toml",
    ],
    "assistant": [
        "crates/mcp", "crates/mcp-server", "crates/control", "crates/control-egui",
        "crates/craft-plugin", "crates/craft-plugin-wasm",
        "crates/ui-egui/src/agent_chat",
    ],
    "ui": [
        "crates/ui-egui", "crates/ui-kit", "crates/look", "crates/theme",
        "crates/color", "crates/ui-text", "crates/ui-registry",
        "crates/display-units", "crates/viewport", "crates/viewport-gpu",
        "crates/viewport-egui", "crates/render", "crates/gpu",
        "crates/space-mouse", "assets",
    ],
    "build": [
        ".github", "xtask", "scripts", "packaging", "vendor", ".cargo", ".config",
        "tools", "flake.nix", "flake.lock", "package.json", "package-lock.json",
        "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "deny.toml",
        "clippy.toml", "rustfmt.toml", ".gitignore", ".gitattributes",
        ".coderabbit.yaml", "THIRD-PARTY-LICENSES.txt",
    ],
    "apps": ["apps", "schema", "packages"],
    "docs": [
        "docs", "skills", "attribution", "README.md", "CONTRIBUTING.md",
        "AGENTS.md", "CLAUDE.md", "NOTICE", "ATTRIBUTION.md", "ROADMAP.md",
        "SECURITY.md", "DISCLAIMER.md", "LICENSE-APACHE", "LICENSE-MIT",
    ],
    "test-policy": ["tests/KNOWN_FAILING_TESTS", "tests/QUARANTINED_TESTS"],
    "retired": [
        "plan", "reports", "harness", "site", "fixtures", "docs/generic-blend",
        "crates/difftest", "crates/goldens", "crates/solidcore",
        "crates/solidcore-ffi", "crates/solidcore-ops", "crates/solidcore-tess",
        "crates/solidcore-record", "crates/solidcore-convert",
        "crates/solidcore-step", "crates/solidcore-ssi", "crates/step-import",
        "crates/step-brep", "crates/step-export", "crates/ssi",
        "crates/nurbs-grid-mesh", "crates/xtask", "crates/diagnostics",
    ],
}


def run(args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def commits():
    out = run(["log", "--no-merges", "--full-history", "--reverse",
               "--format=@@%H", "--name-only", f"{FORK}..{TARGET}"])
    result, cur = {}, None
    for line in out.splitlines():
        if line.startswith("@@"):
            cur = line[2:]
            result[cur] = []
        elif line.strip():
            result[cur].append(line)
    return result


def main():
    subjects = {}
    for line in run(["log", "--no-merges", "--format=%H%x09%s", f"{FORK}..{TARGET}"]).splitlines():
        sha, _, subject = line.partition("\t")
        subjects[sha] = subject

    files = commits()
    counts = collections.Counter()
    unassigned = []
    empty = []
    inventory = []

    for sha, paths in files.items():
        if not paths:
            empty.append(sha)
            inventory.append((sha, "empty", 0))
            continue
        hit = {area for area, specs in AREAS.items()
               if any(_under(p, s) for p in paths for s in specs)}
        if not hit:
            unassigned.append((sha, paths))
        for area in hit:
            counts[area] += 1
        inventory.append((sha, ",".join(sorted(hit)), len(paths)))

    if EMIT:
        with open(EMIT, "w", encoding="utf-8") as fh:
            fh.write("sha\tareas\tfiles\tsubject\n")
            for sha, areas, n in inventory:
                fh.write(f"{sha}\t{areas}\t{n}\t{subjects.get(sha, '')}\n")

    print(f"range {FORK}..{TARGET}: {len(files)} non-merge commits")
    for area in AREAS:
        print(f"  {area:12s} {counts[area]:5d}")
    print(f"  {'(empty)':12s} {len(empty):5d}")

    if unassigned:
        print(f"\nUNASSIGNED: {len(unassigned)}")
        for sha, paths in unassigned:
            print(f"  {sha[:9]} {subjects.get(sha, '')[:80]}")
            for p in paths[:3]:
                print(f"      {p}")
        return 1
    print("\nall commits assigned")
    return 0


def _under(path, spec):
    return path == spec or path.startswith(spec.rstrip("/") + "/")


if __name__ == "__main__":
    sys.exit(main())
