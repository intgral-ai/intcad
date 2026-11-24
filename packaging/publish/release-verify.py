#!/usr/bin/env python3
"""Check an assembled release directory before (and after) it is published.

The directory is flat and holds exactly what `packaging/aliases.sh` and
`packaging/checksums.sh` produced from the package builds of one source revision:

    intcad-<version>-linux-<arch>.tar.gz / .AppImage / .deb / .rpm / .flatpak / .snap
    intcad-<version>-macos-universal.dmg, intcad-cli-<version>-macos-universal.zip
    intcad-<version>-windows-<arch>.msi, -portable.zip, Intcad-<version>-windows-x64.exe
    intcad-<version>-freebsd-x86_64.tar.gz, intcad-web-<version>.zip
    their unversioned aliases (what `crates/ui-egui/src/update.rs` asks for by name)
    SHA256SUMS.txt, SHA256SUMS (same bytes), SHA256SUMS.txt.minisig

This script checks the *shape*: the names, the hashes, the alias coverage, and that every
`build.json` names this checkout. It does not verify the signature (that is `minisign -V`,
which the publish workflow runs on the same directory); with `--require-signature` it refuses a
directory without one so a signing step cannot be skipped silently.

Usage:
    packaging/publish/release-verify.py --dir out --version 0.1.3 --revision <sha>
        [--min-platforms linux,macos,windows] [--require-signature] [--manifest out.json]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

SUMS = "SHA256SUMS.txt"
SUMS_LEGACY = "SHA256SUMS"
SUMS_SIG = "SHA256SUMS.txt.minisig"
EVIDENCE = ("build.json", "contents.json")

# The unversioned names the in-app update check resolves (crates/ui-egui/src/update.rs,
# `asset_name`). Each must expose one file per platform/architecture it supports.
ALIAS_SUFFIXES = (".AppImage", ".deb", ".rpm", ".flatpak", ".snap", ".msi", "-portable.zip", ".dmg")
# A versioned package that the updater looks up by its versioned name, so it needs no alias.
VERSIONED_ONLY = (".tar.gz", "-portable.zip")

PLATFORM_PREFIXES = {
    "linux": ("intcad-{v}-linux-",),
    "macos": ("intcad-{v}-macos-universal.dmg", "intcad-cli-{v}-macos-universal.zip"),
    "windows": ("intcad-{v}-windows-",),
    "freebsd": ("intcad-{v}-freebsd-",),
    "web": ("intcad-web-{v}.zip",),
}

SEMVER = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?$")
# Any version inside a file name (`intcad-0.1.3-linux-...`): a name that carries a version must
# carry this one. The numeric part is enough; a prerelease suffix (`0.1.3-beta.1`) keeps it.
SEMVER_IN_NAME = re.compile(r"-(\d+\.\d+\.\d+)")
HEX64 = re.compile(r"^[0-9a-f]{64}$")


class Problems:
    def __init__(self) -> None:
        self.items: list[str] = []

    def add(self, message: str) -> None:
        self.items.append(message)

    def require(self, condition: bool, message: str) -> None:
        if not condition:
            self.add(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_sums(path: Path, problems: Problems) -> dict[str, str]:
    """`<hex>  <name>` (or `<hex> *<name>`), lowercase hex, unique names."""
    listed: dict[str, str] = {}
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        problems.add(f"{path.name}: {error}")
        return listed
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        parts = line.split(None, 1)
        if len(parts) != 2:
            problems.add(f"{path.name}:{number}: not a checksum line")
            continue
        digest, name = parts
        name = name.lstrip("*")
        if not HEX64.match(digest):
            problems.add(f"{path.name}:{number}: {digest[:16]} is not a lowercase SHA-256")
            continue
        if Path(name).name != name or name in (".", ".."):
            problems.add(f"{path.name}:{number}: {name!r} is not a plain file name")
            continue
        if name in listed:
            problems.add(f"{path.name}:{number}: {name} is listed twice")
            continue
        listed[name] = digest
    return listed


def release_files(directory: Path) -> list[Path]:
    files = []
    for entry in sorted(directory.iterdir()):
        if entry.is_symlink():
            continue
        if entry.is_file():
            files.append(entry)
    return files


def versioned_for(version: str, platform: str) -> tuple[str, ...]:
    return tuple(pattern.format(v=version) for pattern in PLATFORM_PREFIXES[platform])


def alias_for(name: str, version: str) -> str | None:
    """The unversioned name the updater asks for, when the file has one."""
    if not name.startswith(("intcad-", "Intcad-")):
        return None
    if not any(name.endswith(suffix) for suffix in ALIAS_SUFFIXES):
        return None
    # `intcad-0.1.3-linux-x86_64.AppImage` -> `intcad-linux-x86_64.AppImage`;
    # `Intcad-0.1.3-windows-x64.exe` has no alias (the updater names it with its version).
    stem = f"-{version}-"
    if stem not in name:
        return None
    prefix, rest = name.split(stem, 1)
    return f"{prefix}-{rest}"


def check(args: argparse.Namespace, problems: Problems) -> dict[str, object]:
    directory = Path(args.dir)
    problems.require(directory.is_dir(), f"no such directory: {directory}")
    if not directory.is_dir():
        return {}
    problems.require(SEMVER.match(args.version) is not None, f"version {args.version!r} is not SemVer")

    files = release_files(directory)
    names = {entry.name for entry in files}
    problems.require(SUMS in names, f"{SUMS} is missing: run packaging/checksums.sh")
    problems.require(SUMS_LEGACY in names, f"{SUMS_LEGACY} is missing: clients before 0.1.3 fetch it")
    if args.require_signature:
        problems.require(SUMS_SIG in names, f"{SUMS_SIG} is missing: the release is not signed")

    if SUMS in names and SUMS_LEGACY in names:
        problems.require(
            (directory / SUMS).read_bytes() == (directory / SUMS_LEGACY).read_bytes(),
            f"{SUMS} and {SUMS_LEGACY} differ; they must be the same list",
        )

    listed = read_sums(directory / SUMS, problems) if SUMS in names else {}
    actual = {entry.name for entry in files if entry.name not in {SUMS, SUMS_LEGACY, SUMS_SIG, *EVIDENCE}}
    problems.require(actual == set(listed), f"{SUMS} lists {sorted(listed)}, directory holds {sorted(actual)}")
    for name in sorted(actual & set(listed)):
        problems.require(sha256(directory / name) == listed[name], f"{name}: does not match {SUMS}")

    # A package file must carry this release's version: a stale artifact from another build is
    # never relabelled, and a mismatch is what a mixed-up download looks like.
    for name in sorted(listed):
        found = SEMVER_IN_NAME.search(name)
        if found is not None:
            problems.require(found.group(1) == args.version, f"{name}: built for {found.group(1)}, not {args.version}")

    # The unversioned aliases the updater resolves by name must exist and match the file they copy.
    for name in sorted(listed):
        alias = alias_for(name, args.version)
        if alias is None:
            continue
        problems.require(alias in listed, f"{alias} is missing: {name} has no alias, so that install cannot update")
        if alias in listed:
            problems.require(listed[alias] == listed[name], f"{alias} differs from {name}")

    # The files a release promises for a platform, so a partial build is refused here and not
    # discovered by users without a package.
    for platform in [p for p in args.min_platforms.split(",") if p]:
        problems.require(platform in PLATFORM_PREFIXES, f"unknown platform {platform!r}")
        for pattern in versioned_for(args.version, platform):
            found = any(name == pattern or (pattern.endswith("-") and name.startswith(pattern)) for name in listed)
            problems.require(found, f"{platform}: no {pattern}")

    # Every package descriptor must name this source revision.
    revisions: dict[str, str] = {}
    for path in sorted(directory.rglob("build.json")):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            problems.add(f"{path.relative_to(directory)}: {error}")
            continue
        revision = data.get("sourceRevision")
        revisions[str(path.relative_to(directory))] = str(revision)
        problems.require(revision == args.revision, f"{path.relative_to(directory)}: sourceRevision {revision!r} is not {args.revision!r}")

    if args.require_signature and SUMS_SIG in names:
        problems.require((directory / SUMS_SIG).stat().st_size > 0, f"{SUMS_SIG} is empty")

    return {
        "version": args.version,
        "revision": args.revision,
        "files": {name: listed[name] for name in sorted(listed)},
        "signature": SUMS_SIG if SUMS_SIG in names else None,
        "build_revisions": revisions,
    }


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dir", required=True, help="the flat release directory")
    parser.add_argument("--version", required=True, help="the version, without a leading v")
    parser.add_argument("--revision", required=True, help="the source revision every package was built from")
    parser.add_argument("--min-platforms", default="linux,macos,windows", help="comma-separated platforms that must be present")
    parser.add_argument("--require-signature", action="store_true", help="refuse a release without SHA256SUMS.txt.minisig")
    parser.add_argument("--manifest", help="write the verified manifest here (JSON)")
    args = parser.parse_args(argv)

    problems = Problems()
    manifest = check(args, problems)
    for problem in problems.items:
        print(f"release-verify: {problem}", file=sys.stderr)
    if problems.items:
        print(f"release-verify: {len(problems.items)} problem(s); nothing was published", file=sys.stderr)
        return 1
    if args.manifest:
        Path(args.manifest).write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"release-verify: {len(manifest.get('files', {}))} files, version {args.version}, revision {args.revision}: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
