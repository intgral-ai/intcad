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
                elif is_dir and not any(other != name and other.startswith(name + "/") for _, other in lowered):
                    continue
                else:
                    hit = True
                break
        if not hit and parts and parts[-1] in RUNTIME_BASENAMES:
            hit = True
        if hit:
            found.append({"name": str(original), "path": str(original)})
    return found


def inside_agent(entry):
    """Whether a marker is the built-in agent's own Node or a package inside its folder."""
    parts = [part for part in str(entry.get("path", "")).replace("\\", "/").split("/") if part]
    return AGENT_DIR in parts


def inside_agent_markers_for(distribution, names):
    """The runtime markers of a source artifact, given the distribution it is recorded for."""
    markers = marker_entries(names)
    if distribution == "native":
        return markers
    return [entry for entry in markers if not inside_agent(entry)]


def agent_markers(names):
    """The markers outside the built-in agent's folder: a legacy ``mastra/`` runtime or enterprise
    content. The agent's own Node and ``node_modules`` are what the ``mastra-agent`` distribution
    is for, and are not markers of it.
    """
    return [entry for entry in marker_entries(names)
            if AGENT_DIR not in [part for part in str(entry.get("path", "")).replace("\\", "/").split("/") if part]]


def marker(path):
    return bool(marker_entries([path]))


def _parts(name):
    return [part for part in str(name).replace("\\", "/").split("/") if part and part != "."]


def agent_node(name, target=None):
    """Whether a package entry is the built-in agent's own Node program.

    A universal macOS app carries one folder per slice (``mastra-agent/<triple>/node``), so the
    program is directly in the agent folder or in the slice folder under it.
    """
    parts = _parts(name)
    if not parts or parts[-1] not in RUNTIME_BASENAMES:
        return False
    for index in range(len(parts) - 1):
        if parts[index] != AGENT_DIR:
            continue
        between = parts[index + 1:-1]
        return between == [] or (len(between) == 1 and TARGET_SLICE.match(between[0]) is not None)
    return False


def agent_report(names, target=None):
    """What a package carries of the built-in agent, from its entry names.

    ``present``: the folder is there at all, whatever is inside it (a damaged folder still makes
    the app offer the agent and fail to start it). ``node``: its own Node program is there.
    ``missing``: the files the agent needs that the package does not hold.
    """
    parts = [_parts(name) for name in names]
    present = any(AGENT_DIR in entry for entry in parts)
    node = any(agent_node("/".join(entry), target) for entry in parts if entry)
    node_targets = sorted({
        entry[entry.index(AGENT_DIR) + 1]
        for entry in parts
        if agent_node("/".join(entry), target) and len(entry) > entry.index(AGENT_DIR) + 2
    })
    found = set()
    skills = set()
    for entry in parts:
        for index, part in enumerate(entry):
            if part != AGENT_DIR:
                continue
            rest = entry[index + 1:]
            if rest and TARGET_SLICE.match(rest[0]):
                rest = rest[1:]
            found.add("/".join([AGENT_DIR] + rest))
            # The agent's own `mastra-agent/skills/<set>/<skill>/SKILL.md`: exactly that shape, so
            # a package's own skill file (a dependency's `node_modules/.../skills/x/y/SKILL.md`)
            # is not counted as one of the agent's.
            if len(rest) == 4 and rest[0] == "skills" and entry[-1] == "SKILL.md":
                skills.add("/".join(rest[1:3]))
    missing = [required for required in AGENT_REQUIRED if required not in found]
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
        return [line for line in _lines(_run(["tar", "--zstd", "-tf", str(path)])) if line != "."]
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
        try:
            _, names = inspect(source)
        except Exception as error:  # noqa: BLE001 - any failure is unverified content
            report["unchecked"].append({"name": source.name, "reason": str(error)})
        else:
            # The source tarball carries the same agent the derived package will. Its folder is a
            # marker only for a native distribution; for `mastra-agent` it is the point, and a Node
            # outside it stays a marker either way.
            report["markers"].extend(inside_agent_markers_for(args.distribution, names))
            report["agentMarkers"].extend(agent_markers(names))
            found = agent_report(names)
            if found["present"]:
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
            report["markers"].extend(marker_entries(names) if args.distribution == "native" else [
                entry for entry in marker_entries(names) if not inside_agent(entry)])
            report["agentMarkers"].extend(agent_markers(names))
            # A package holds one agent folder, so the report names what the first artifact with
            # one carries; another artifact with a different answer is a mismatch for the gate.
            if report["agent"] is None:
                found = agent_report(names)
                if found["present"]:
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
