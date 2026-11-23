#!/usr/bin/env python3
"""Record the actual contents of the packages a release job built.

The report is derived from the built artifact bytes, never from an environment
flag, a package name or a hand-written success boolean. The release gate uses it
to decide which distribution the packages really are:

- every canonical installer must have been inspected;
- a ``native`` distribution may contain no ``mastra``, ``node_modules``, ``ee``
  or Node binary at all;
- a ``mastra-agent`` distribution carries the built-in agent in
  ``mastra-agent/`` (its Node, entry, dependencies and licences) and may carry no
  legacy ``mastra/`` runtime, no enterprise content and no other Node program;
- a ``mastra`` distribution (the legacy workflow runtime) stays refused;
- a package derived from the Linux binaries (Flatpak, Arch, Snap) must name the
  inspected Linux package and the same source digest.

An artifact this runner cannot inspect is reported as ``unchecked`` and the gate
refuses to treat the distribution as proven.
"""

import argparse
import hashlib
import json
import os
import shutil
import re
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

RUNTIME_COMPONENTS = frozenset(("mastra", "node_modules", "ee"))
RUNTIME_BASENAMES = frozenset(("node", "node.exe"))
IGNORED_SUFFIXES = (".zsync", ".sha256", ".blockmap", ".json")
# The built-in agent lives in this folder; a release package that declares ``mastra-agent`` must
# carry all of these inside it, whatever else it holds.
AGENT_DIR = "mastra-agent"
AGENT_REQUIRED = (
    "mastra-agent/agent/main.mjs",
    "mastra-agent/agent.json",
    "mastra-agent/licenses/THIRD-PARTY-LICENSES.txt",
    "mastra-agent/licenses/NODE-LICENSE.txt",
    "mastra-agent/node_modules/@mastra/code-sdk/package.json",
    "mastra-agent/node_modules/@agentclientprotocol/sdk/package.json",
)
# The only place a Node program is expected: the agent's own folder.
# The modelling skills the agent installs at start: every skill is `<set>/<skill>/SKILL.md` under
# the agent's `skills/`. A release package whose agent has none cannot do the job INT-1473 asks.
AGENT_SKILL_SUFFIX = "/SKILL.md"
# A universal macOS app carries one agent folder per CPU, named for the target.
TARGET_SLICE = re.compile(r"^(aarch64|x86_64|i686|arm64|armv7)-(apple-darwin|unknown-linux-gnu|unknown-linux-musl|pc-windows-msvc)$")


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def marker_entries(names):
    """Marker files, and marker directories that actually contain something.

    The Windows package deliberately stages an empty ``mastra`` folder so the WiX
    harvest has a stable directory. An empty folder is not a bundled runtime; a
    directory with any child, or any file named like a runtime, is.
    """
    lowered = [(name, str(name).replace("\\", "/").rstrip("/").lower()) for name in names]
    found = []
    for original, name in lowered:
        parts = [part for part in name.split("/") if part]
        is_dir = str(original).endswith(("/", "\\"))
        hit = False
        for index, part in enumerate(parts):
            if part in RUNTIME_COMPONENTS:
                if parts[index + 1:]:
                    hit = True
                elif is_dir or not any(other != name and other.startswith(name + "/") for _, other in lowered):
                    # A directory with no child is the empty `mastra/` the Windows packages stage
                    # for WiX; an archive may list it with or without a trailing slash.
                    continue
                else:
                    hit = True
                break
        if not hit and parts and parts[-1] in RUNTIME_BASENAMES:
            hit = True
        if hit:
            found.append({"name": str(original), "path": str(original)})
    return found


# The stand-in the agent builder writes in place of Mastra's enterprise chunk, next to the other
# `ee*` chunks in a `@mastra/core/dist/` folder. It is the only `ee` file a package may hold in
# one such folder, and only one of them: a second is enterprise content the builder did not remove.
STAND_IN = re.compile(r"(?:^|/)@mastra/core/dist/ee[.-][A-Za-z0-9_.-]+$", re.IGNORECASE)


def stand_in_paths(names):
    """The `ee-*` stand-ins a package may hold: one per agent folder.

    The builder writes one stand-in per `@mastra/core` copy, and a universal app carries one copy
    per CPU, so `mastra-agent/aarch64-apple-darwin/node_modules/...` and
    `mastra-agent/x86_64-apple-darwin/node_modules/...` each hold their own. What is refused is a
    *second* `ee-*` file under the same `@mastra/core/dist/`: that is enterprise content the
    builder left behind, not the stand-in it wrote.
    """
    by_core = {}
    for raw in names:
        name = str(raw).replace("\\", "/").lstrip("./")
        if not STAND_IN.search(name):
            continue
        core = name.rsplit("/@mastra/core/dist/", 1)[0]
        by_core.setdefault(core, []).append(name)
    allowed = set()
    for core, found in by_core.items():
        if len(found) == 1:
            allowed.add(found[0])
    return frozenset(allowed)


# The two things a package may not carry, by where they are rather than by a word in a name:
# Mastra's enterprise chunks (`ee-*.js` in a `@mastra/core/dist/` folder; the builder removes them
# and writes one stand-in) and the legacy workflow runtime (a `mastra/` folder beside the app, not
# a package's own `dist/mastra/` or the `mastra-agent/` folder this change adds).
def forbidden_parts(entry, stand_ins=frozenset()):
    """The forbidden components of one package entry, by path."""
    text = str(entry.get("path", "")).replace("\\", "/").lstrip("./")
    parts = [part for part in text.split("/") if part]
    bad = []
    for index in range(len(parts) - 3):
        if (parts[index] == "@mastra" and parts[index + 1].lower() == "core"
                and parts[index + 2].lower() == "dist"):
            name = parts[-1].lower()
            if name.startswith(("ee-", "ee.")) and text not in stand_ins:
                bad.append(parts[-1])
            break
    if not any(part.lower() == "node_modules" for part in parts):
        for part in parts:
            if part == "mastra":
                bad.append(part)
                break
    return bad


def empty_legacy_folder(name, names):
    """The Windows packages stage an empty ``mastra/`` so WiX has a directory to harvest; an empty
    one is not a runtime."""
    text = str(name).replace("\\", "/").rstrip("/")
    parts = [part for part in text.split("/") if part]
    if not parts or parts[-1] != "mastra":
        return False
    return not any(str(other).replace("\\", "/") != str(name).replace("\\", "/")
                   and str(other).replace("\\", "/").startswith(text + "/") for other in names)


def inside_agent(entry, stand_ins=(), roots=None):
    """Whether a marker is the built-in agent's own Node or a package inside its folder, at the
    place the app looks for it (`roots`)."""
    parts = [part for part in str(entry.get("path", "")).replace("\\", "/").split("/") if part]
    return _agent_index(parts, roots) is not None and not forbidden_parts(entry, stand_ins)


def inside_agent_markers_for(distribution, names, roots=None):
    """The runtime markers of a source artifact, given the distribution it is recorded for."""
    markers = marker_entries(names)
    if distribution == "native":
        return markers
    stand_ins = stand_in_paths(names)
    return [entry for entry in markers if not inside_agent(entry, stand_ins, roots)]


def agent_markers(names, roots=None):
    """What a ``mastra-agent`` package may not carry besides its own agent folder: a legacy
    ``mastra/`` runtime, enterprise content, or a Node program outside the folder. Enterprise
    content and the legacy runtime are refused inside the folder too.

    Every entry is looked at, not only the runtime markers: an ``ee-`` file of Mastra's compiles
    to a name `marker_entries` does not recognise on its own, and that is exactly the content the
    licence forbids. The one stand-in `agent.json` names is allowed.
    """
    stand_ins = stand_in_paths(names)
    seen = set()
    found = []
    for name in names:
        entry = {"name": str(name), "path": str(name)}
        if entry["path"] in seen:
            continue
        seen.add(entry["path"])
        if empty_legacy_folder(name, names):
            continue
        if forbidden_parts(entry, stand_ins) or (entry in marker_entries([name])
                                                 and not inside_agent(entry, stand_ins, roots)):
            found.append(entry)
    return found


def marker(path):
    return bool(marker_entries([path]))


def _parts(name):
    return [part for part in str(name).replace("\\", "/").split("/") if part and part != "."]


# Where each platform's packager puts the agent: the app looks beside its own resources, so a
# `mastra-agent/` folder anywhere else (an `extras/` tree, a stray copy) does not make a package
# that can run one. The path before `mastra-agent/` is matched as whole components.
AGENT_ROOTS = {
    "linux": (("lib", "intcad", "mastra-agent"),),
    "macos": (("Contents", "Resources", "mastra-agent"),),
    "windows": (("mastra-agent",),),
}


def roots_for(package_id):
    """The agent folder's install places for a package id, or ``None`` to accept any."""
    if package_id.startswith("release-macos"):
        return AGENT_ROOTS["macos"]
    if package_id.startswith("release-windows"):
        return AGENT_ROOTS["windows"]
    if package_id.startswith(("release-linux", "release-flatpak", "release-arch", "release-snap",
                              "release-appimage", "release-deb", "release-rpm")):
        return AGENT_ROOTS["linux"]
    return None


def _agent_index(parts, roots):
    """The index of `mastra-agent` in `parts` when it sits at one of `roots`, else ``None``."""
    if roots is None:
        return parts.index(AGENT_DIR) if AGENT_DIR in parts else None
    for root in roots:
        size = len(root)
        for index in range(len(parts) - size + 1):
            if list(parts[index:index + size]) == list(root):
                return index + size - 1
    return None


def agent_node(name, target=None, roots=None):
    """Whether a package entry is the built-in agent's own Node program.

    A universal macOS app carries one folder per slice (``mastra-agent/<triple>/node``), so the
    program is directly in the agent folder or in the slice folder under it.
    """
    parts = _parts(name)
    if not parts or parts[-1] not in RUNTIME_BASENAMES:
        return False
    index = _agent_index(parts, roots)
    if index is None:
        return False
    between = parts[index + 1:-1]
    return (not between) or (len(between) == 1 and TARGET_SLICE.match(between[0]) is not None)


def agent_report(names, target=None, roots=None):
    """What a package carries of the built-in agent, from its entry names.

    ``present``: the folder is there at all, whatever is inside it (a damaged folder still makes
    the app offer the agent and fail to start it). ``node``: its own Node program is there.
    ``missing``: the files the agent needs that the package does not hold.
    """
    parts = [_parts(name) for name in names]
    anchored = [entry for entry in parts if _agent_index(entry, roots) is not None]
    present = bool(anchored)
    node = any(agent_node("/".join(entry), target, roots) for entry in anchored if entry)
    node_targets = sorted({
        entry[_agent_index(entry, roots) + 1]
        for entry in anchored
        if agent_node("/".join(entry), target, roots)
        and len(entry) > _agent_index(entry, roots) + 2
    })
    found = {}
    skills = set()
    for entry in anchored:
        for index, part in enumerate(entry):
            if part != AGENT_DIR or _agent_index(entry, roots) != index:
                continue
            rest = entry[index + 1:]
            slice_ = ""
            if rest and TARGET_SLICE.match(rest[0]):
                slice_ = rest[0]
                rest = rest[1:]
            found.setdefault(slice_, set()).add("/".join([AGENT_DIR] + rest))
            # The agent's own `mastra-agent/skills/<set>/<skill>/SKILL.md`: exactly that shape, so
            # a package's own skill file (a dependency's `node_modules/.../skills/x/y/SKILL.md`)
            # is not counted as one of the agent's.
            if len(rest) == 4 and rest[0] == "skills" and entry[-1] == "SKILL.md":
                skills.add("/".join(rest[1:3]))
    # A universal app carries one whole agent per CPU: each folder is checked on its own, so the
    # Intel folder cannot consist of a Node and borrow the ARM folder's entry, deps and skills.
    missing = []
    for slice_ in sorted(found):
        have = found[slice_]
        prefix = "" if not slice_ else slice_ + "/"
        missing.extend(prefix + required for required in AGENT_REQUIRED if required not in have)
        if not any(name.startswith(AGENT_DIR + "/skills/") for name in have):
            missing.append(prefix + AGENT_DIR + "/skills/<set>/<skill>/SKILL.md")
    return {"present": present, "node": node, "nodeTargets": node_targets,
            "missing": missing, "skills": sorted(skills)}


def _walk(root):
    root = Path(root)
    names = []
    for directory, dirnames, filenames in os.walk(root):
        for name in dirnames:
            names.append(str((Path(directory) / name).relative_to(root)))
        for name in filenames:
            names.append(str((Path(directory) / name).relative_to(root)))
    return names


def _run(args, **kwargs):
    result = subprocess.run(args, capture_output=True, text=True, **kwargs)
    if result.returncode != 0:
        raise OSError((result.stderr or result.stdout or "command failed").strip()[:200])
    return result.stdout


def _lines(text):
    return [line.strip() for line in text.splitlines() if line.strip()]


def names_tar(path):
    name = path.name.lower()
    if name.endswith(".tar.zst") or name.endswith(".tzst"):
        try:
            listing = _run(["tar", "--zstd", "-tf", str(path)])
        except OSError:
            # GNU tar shells out to `zstd`; bsdtar reads zstd itself, and the Arch job installs
            # it. A machine may have only one of them.
            bsdtar = shutil.which("bsdtar")
            if not bsdtar:
                raise
            listing = _run([bsdtar, "-tf", str(path)])
        return [line for line in _lines(listing) if line != "."]
    mode = "r:gz" if name.endswith((".tar.gz", ".tgz")) else "r:xz" if name.endswith((".tar.xz", ".txz")) else "r:"
    with tarfile.open(path, mode) as archive:
        return [member.name for member in archive.getmembers() if member.name != "."]


def names_zip(path):
    with zipfile.ZipFile(path) as archive:
        return [item.filename for item in archive.infolist() if item.filename]


def names_deb(path):
    tool = shutil.which("dpkg-deb")
    if not tool:
        raise OSError("dpkg-deb is unavailable")
    names = []
    for line in _lines(_run([tool, "-c", str(path)])):
        fields = line.split()
        if len(fields) >= 6:
            names.append(fields[-1].lstrip("./"))
    return names


def names_rpm(path):
    tool = shutil.which("rpm")
    if not tool:
        raise OSError("rpm is unavailable")
    return [line.lstrip("./") for line in _lines(_run([tool, "-qlp", str(path)])) if line]


def names_snap(path):
    tool = shutil.which("unsquashfs")
    if not tool:
        raise OSError("unsquashfs is unavailable")
    names = []
    for line in _lines(_run([tool, "-l", str(path)])):
        token = line.split()[-1] if line.split() else ""
        if token and token != "squashfs-root" and not token.startswith("Parallel"):
            names.append(token.split("squashfs-root/", 1)[-1])
    return names


def names_appimage(path):
    # Extraction runs in a temporary working directory, so the AppImage path
    # must be absolute; the package job hands us a relative dist/release path.
    path = Path(path).resolve()
    with tempfile.TemporaryDirectory(prefix="contents-appimage-") as directory:
        env = dict(os.environ)
        env["APPIMAGE_EXTRACT_AND_RUN"] = "1"
        _run([str(path), "--appimage-extract"], cwd=directory, env=env)
        extracted = Path(directory) / "squashfs-root"
        if not extracted.is_dir():
            raise OSError("the AppImage did not extract")
        return _walk(extracted)


def names_dmg(path):
    tool = shutil.which("hdiutil")
    if not tool:
        raise OSError("hdiutil is unavailable")
    with tempfile.TemporaryDirectory(prefix="contents-dmg-") as directory:
        mount = Path(directory) / "mnt"
        mount.mkdir()
        _run([tool, "attach", "-nobrowse", "-readonly", "-mountpoint", str(mount), str(path)])
        try:
            return _walk(mount)
        finally:
            subprocess.run([tool, "detach", str(mount)], capture_output=True, text=True)


# The Flatpak bundle is an OSTree commit, which Python cannot read; the flatpak job installs the
# bundle it has just built (`flatpak install --user --reinstall`, inside flatpak-bundle.sh) and its
# smoke runs against it, so the installed tree is these bytes. The artifact's digest is recorded
# beside the listing, and its absence is a failure, not a pass.
FLATPAK_APP_ID = "ai.intgral.intcad"


def names_flatpak(path):
    tool = shutil.which("flatpak")
    if not tool:
        raise OSError("flatpak is unavailable")
    # Install these bytes before listing them: `--reinstall` replaces the deployment from this
    # file, so the tree cannot be a stale or different bundle. The runtime the bundle names is
    # already installed by the job that built and smoked it; `--noninteractive` refuses rather
    # than waits if it is not.
    _run([tool, "install", "--user", "-y", "--noninteractive", "--reinstall",
          str(Path(path).resolve())], timeout=900)
    try:
        location = _run([tool, "info", "--user", "--show-location", FLATPAK_APP_ID]).strip()
    except OSError as error:
        raise OSError("the app is not installed from a bundle: " + str(error)) from error
    root = Path(location) if location else None
    if root is None or not root.is_dir():
        raise OSError("the app is not installed from a bundle")
    names = _walk(root)
    if not names:
        raise OSError("the installed app carries no files")
    return names


def names_msi(path):
    seven = shutil.which("7z") or shutil.which("7za")
    if seven:
        names = []
        for line in _lines(_run([seven, "l", "-slt", str(path)])):
            if line.startswith("Path = "):
                value = line[len("Path = "):].strip()
                if value:
                    names.append(value)
        if names:
            return names
    tool = shutil.which("msiexec") or shutil.which("msiexec.exe")
    if not tool:
        raise OSError("neither 7z nor msiexec is available")
    with tempfile.TemporaryDirectory(prefix="contents-msi-") as directory:
        _run([tool, "/a", str(path), "/qn", "TARGETDIR=" + directory])
        return _walk(directory)


def inspect(path):
    name = path.name.lower()
    if name.endswith((".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.zst", ".tzst", ".tar")):
        return "tar", names_tar(path)
    if name.endswith(".zip"):
        return "zip", names_zip(path)
    if name.endswith(".deb"):
        return "deb", names_deb(path)
    if name.endswith(".rpm"):
        return "rpm", names_rpm(path)
    if name.endswith(".snap"):
        return "snap", names_snap(path)
    if name.endswith((".appimage",)):
        return "appimage", names_appimage(path)
    if name.endswith(".dmg"):
        return "dmg", names_dmg(path)
    if name.endswith(".msi"):
        return "msi", names_msi(path)
    if name.endswith(".flatpak"):
        return "flatpak", names_flatpak(path)
    raise OSError("no content inspector for this artifact")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-id", required=True)
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--distribution", default="native", choices=("native", "mastra", "mastra-agent"))
    parser.add_argument("--out", required=True)
    parser.add_argument("--derived-from")
    parser.add_argument("--source-artifact")
    parser.add_argument("artifacts", nargs="*")
    args = parser.parse_args()

    roots = roots_for(args.package_id)
    report = {
        "schemaVersion": 1,
        "packageId": args.package_id,
        "sourceRevision": args.source_revision,
        "distribution": args.distribution,
        "checked": [],
        "ignored": [],
        "unchecked": [],
        "markers": [],
        "agentMarkers": [],
        "agent": None,
        "derivedFrom": args.derived_from,
        "sourceArtifact": None,
        "sourceSha256": None,
    }

    if args.derived_from:
        if not args.source_artifact:
            raise SystemExit("--derived-from needs --source-artifact")
        source = Path(args.source_artifact)
        if not source.is_file():
            raise SystemExit("source artifact is missing: " + str(source))
        report["sourceArtifact"] = source.name
        report["sourceSha256"] = digest(source)
        # The repack's own bytes are what a person installs: they are inspected and recorded like
        # any other artifact, and the gate binds them to this report by digest.
        for value in args.artifacts:
            path = Path(value)
            if not path.is_file() or path.name.lower().endswith(IGNORED_SUFFIXES):
                continue
            try:
                kind, names = inspect(path)
            except Exception as error:  # noqa: BLE001 - any failure is unverified content
                report["unchecked"].append({"name": path.name, "reason": str(error)})
                continue
            report["checked"].append({"name": path.name, "sha256": digest(path), "format": kind, "entries": len(names)})
            report["markers"].extend(inside_agent_markers_for(args.distribution, names, roots))
            report["agentMarkers"].extend(agent_markers(names, roots))
            report.setdefault("agents", {})[path.name] = agent_report(names, roots=roots)
        # And the source tarball it was built from: the same agent, from the bytes that were
        # already inspected when the Linux package was made.
        try:
            _, names = inspect(source)
        except Exception as error:  # noqa: BLE001 - any failure is unverified content
            report["unchecked"].append({"name": source.name, "reason": str(error)})
        else:
            # The source tarball carries the same agent the derived package will. Its folder is a
            # marker only for a native distribution; for `mastra-agent` it is the point, and a Node
            # outside it stays a marker either way.
            report["markers"].extend(inside_agent_markers_for(args.distribution, names, roots))
            report["agentMarkers"].extend(agent_markers(names, roots))
            found = agent_report(names, roots=roots)
            report.setdefault("agents", {})[source.name] = found
            if found["present"] and report["agent"] is None:
                report["agent"] = found
    else:
        for value in args.artifacts:
            path = Path(value)
            if not path.is_file():
                continue
            if path.name.lower().endswith(IGNORED_SUFFIXES):
                report["ignored"].append(path.name)
                continue
            try:
                kind, names = inspect(path)
            except Exception as error:  # noqa: BLE001 - any failure is unverified content
                report["unchecked"].append({"name": path.name, "reason": str(error)})
                continue
            report["checked"].append({"name": path.name, "sha256": digest(path), "format": kind, "entries": len(names)})
            # A `mastra-agent` package carries the agent's own Node and node_modules by design;
            # everything else that looks like a runtime, and every Node program outside the
            # agent's folder, is still a marker.
            stand_ins = stand_in_paths(names)
            report["markers"].extend(marker_entries(names) if args.distribution == "native" else [
                entry for entry in marker_entries(names) if not inside_agent(entry, stand_ins, roots)])
            report["agentMarkers"].extend(agent_markers(names, roots))
            # Every artifact is recorded, not only the first: the gate refuses a set whose
            # artifacts disagree, so a correct zip cannot hide a broken MSI beside it.
            found = agent_report(names, roots=roots)
            report.setdefault("agents", {})[path.name] = found
            if report["agent"] is None and found["present"]:
                report["agent"] = found

        # A standalone app executable is one binary, not an install tree. Its bytes are
        # a subset of the MSI/zip payload built from the same stage; the covering
        # artifact is inspected above, so it cannot hide a sidecar on its own.
        covered = next((item["name"] for item in report["checked"]
                        if item["name"].lower().endswith((".msi", ".zip"))), None)
        if covered:
            still_unchecked = []
            for item in report["unchecked"]:
                if item["name"].lower().endswith(".exe") and any(p.name == item["name"] for p in map(Path, args.artifacts)):
                    path = next(p for p in map(Path, args.artifacts) if p.name == item["name"])
                    report["checked"].append({"name": path.name, "sha256": digest(path), "format": "covered",
                                              "entries": 0, "coveredBy": covered})
                    # One binary, a subset of the covering package's bytes: it has no folder of its
                    # own to inspect, so its verdict is the covering package's.
                    if covered in report.get("agents", {}):
                        report["agents"][path.name] = dict(report["agents"][covered], coveredBy=covered)
                else:
                    still_unchecked.append(item)
            report["unchecked"] = still_unchecked

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print("contents: " + args.package_id + ": "
          + str(len(report["checked"])) + " checked, "
          + str(len(report["unchecked"])) + " unchecked, "
          + str(len(report["markers"])) + " runtime markers")


if __name__ == "__main__":
    main()
