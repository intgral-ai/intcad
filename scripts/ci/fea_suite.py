#!/usr/bin/env python3
"""The stress estimate's real-solve suite in CI (PIL-294 A14; docs/design/fea-engineering-ready.md).

The suite is the engine's ignored tests that mesh with gmsh and solve with CalculiX:

    cargo test --release -p solidcraft-engine --lib -- --ignored --test-threads=1 --nocapture cmd::stress fea::

`.github/workflows/fea-suite.yml` installs the versions pinned here, runs the suite and keeps its
raw output; `grade` judges that output. The release gate (`release-gate.yml`) downloads the same
run's output for its candidate commit and grades it again with `handoff_problems`, so a failing case,
an empty run or another program version blocks the release. Nothing here trusts a pass flag
written by the job: every verdict is read again from the raw files.

A run passes only when:
- the listing names at least one test, and every listed test printed `test <name> ... ok`;
- libtest's summary says ok, with as many passed as listed and none failed;
- the programs on the runner name the pinned versions (`gmsh --version`, `ccx -v`);
- every reply the tests printed names the pinned versions in its `programs` field (the engine's
  own reading of the programs' banners), and at least one reply did.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

# The versions the suite was proven with (spec section 2: gmsh 4.12.1, CalculiX 2.21). Changing a
# pin is a reviewed change here: the workflow installs exactly these, and the grader refuses
# anything else, in the job and again in the release gate.
PINS = {
    "linux": {
        "mesher": "gmsh 4.12.1",
        "solver": "CalculiX 2.21",
        # Ubuntu 24.04's own packages, from noble/universe: the release pocket, which never changes.
        "apt": {
            "gmsh": "4.12.1+ds1-1.1build2",
            "libgmsh4.12t64": "4.12.1+ds1-1.1build2",
            "calculix-ccx": "2.21-1",
        },
    },
    # "windows": joins before "ready" (spec decisions 13 and 14), with the same two versions. Add
    # its pins here, a `windows` job to fea-suite.yml that uploads `fea-suite-windows-<sha>`, and
    # "windows" to PLATFORMS; the release gate then requires both.
}

# The platforms whose run the release gate requires. Linux only for showing the Stress button
# (spec decision 14).
PLATFORMS = ("linux",)

# The files one platform's run leaves in its folder (the artifact `fea-suite-<platform>-<sha>`).
FILES = {
    "revision": "revision.txt",   # `git rev-parse HEAD` of the checkout that was tested
    "programs": "programs.json",  # what `gmsh --version` and `ccx -v` printed
    "listing": "listing.txt",     # `cargo test ... -- --ignored --list ...` (stdout)
    "stdout": "suite.stdout",     # libtest's own lines: one `test <name> ... ok` per test
    "stderr": "suite.stderr",     # what the tests printed, the replies among it
}

LISTED = re.compile(r"^(\S+): test$")
RESULT = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)$")
SUMMARY = re.compile(r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; "
                     r"\d+ filtered out; finished in ([0-9.]+)s$")
REPLY_PROGRAM = re.compile(r'"(mesher|solver)":\s*"([^"]*)"')
GMSH_BANNER = re.compile(r"^\s*(\d+\.\d+\.\d+)\s*$", re.M)
CCX_BANNER = re.compile(r"This is Version (\S+)")


def apt_packages(platform):
    """`name=version` for each pinned package, for `apt-get install`."""
    return [f"{name}={version}" for name, version in sorted(PINS[platform]["apt"].items())]


def program_versions(gmsh_text, ccx_text):
    """("gmsh 4.12.1", "CalculiX 2.21") from what `gmsh --version` and `ccx -v` print, or None."""
    gmsh = GMSH_BANNER.search(gmsh_text or "")
    ccx = CCX_BANNER.search(ccx_text or "")
    return (f"gmsh {gmsh[1]}" if gmsh else None, f"CalculiX {ccx[1].rstrip('.,')}" if ccx else None)


def ask_programs(gmsh, ccx):
    """What the two programs print about their versions. `ccx -v` exits non-zero (201 on 2.21)
    after printing; only the text counts."""
    def run(argv):
        try:
            done = subprocess.run(argv, capture_output=True, text=True, timeout=120, stdin=subprocess.DEVNULL)
            return done.stdout + done.stderr
        except (OSError, subprocess.SubprocessError) as error:
            return f"could not run {argv[0]}: {error}"
    return {"gmsh": {"argv": [gmsh, "--version"], "output": run([gmsh, "--version"])},
            "ccx": {"argv": [ccx, "-v"], "output": run([ccx, "-v"])}}


def read(path):
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None


def grade(directory, platform):
    """Judge one platform's run from its raw files. Returns the verdict with its problems; an
    empty `problems` list is the only pass."""
    directory = Path(directory)
    problems = []
    pins = PINS.get(platform)
    if pins is None:
        return dict(platform=platform, problems=[f"no pinned versions for platform {platform!r}"])
    text = {key: read(directory / name) for key, name in FILES.items()}
    for key, name in FILES.items():
        if text[key] is None:
            problems.append(f"{platform}: missing {name}")

    revision = (text["revision"] or "").strip()
    if text["revision"] is not None and not re.fullmatch(r"[0-9a-f]{40}", revision):
        problems.append(f"{platform}: {FILES['revision']} is not a commit id: {revision[:60]!r}")

    # The programs on the runner, as they name themselves.
    programs = {}
    if text["programs"] is not None:
        try:
            programs = json.loads(text["programs"])
        except ValueError as error:
            problems.append(f"{platform}: {FILES['programs']} is not JSON: {error}")
    def output(name):
        entry = programs.get(name) if isinstance(programs, dict) else None
        value = entry.get("output") if isinstance(entry, dict) else None
        return value if isinstance(value, str) else None
    on_runner = program_versions(output("gmsh"), output("ccx"))
    if text["programs"] is not None:
        for found, want, name in ((on_runner[0], pins["mesher"], "gmsh"), (on_runner[1], pins["solver"], "ccx")):
            if found != want:
                problems.append(f"{platform}: {name} on the runner is {found or 'unknown'}, not the pinned {want}")

    # The listing: the tests the run must have passed.
    listed = []
    for line in (text["listing"] or "").splitlines():
        match = LISTED.match(line.strip())
        if match:
            listed.append(match[1])
    if text["listing"] is not None and not listed:
        problems.append(f"{platform}: the listing names no test: an empty run is not a pass")
    if len(set(listed)) != len(listed):
        problems.append(f"{platform}: the listing names a test twice")

    # libtest's own lines.
    results, summary = {}, None
    for line in (text["stdout"] or "").splitlines():
        line = line.rstrip()
        match = RESULT.match(line)
        if match:
            if match[1] in results:
                problems.append(f"{platform}: {match[1]} reported twice")
            results[match[1]] = match[2]
        match = SUMMARY.match(line)
        if match:
            if summary is not None:
                problems.append(f"{platform}: more than one test summary")
            summary = dict(verdict=match[1], passed=int(match[2]), failed=int(match[3]), ignored=int(match[4]),
                           seconds=float(match[5]))
    for name in listed:
        status = results.get(name)
        if status != "ok":
            problems.append(f"{platform}: {name}: {status or 'no result'}")
    for name in sorted(set(results) - set(listed)):
        problems.append(f"{platform}: {name} ran but was not listed")
    if text["stdout"] is not None:
        if summary is None:
            problems.append(f"{platform}: no test summary (the run did not finish)")
        elif summary["verdict"] != "ok" or summary["failed"] or summary["passed"] != len(listed):
            problems.append(f"{platform}: summary {summary['verdict']}: {summary['passed']} passed, "
                            f"{summary['failed']} failed, against {len(listed)} listed")

    # The replies' `programs` (the engine's reading of the banners while it ran).
    named = {"mesher": set(), "solver": set()}
    for key, value in REPLY_PROGRAM.findall(text["stderr"] or ""):
        named[key].add(value)
    if text["stderr"] is not None:
        for key in ("mesher", "solver"):
            if not named[key]:
                problems.append(f"{platform}: no reply named its {key}: the versions it ran with are unknown")
            for other in sorted(named[key] - {pins[key]}):
                problems.append(f"{platform}: a reply names the {key} {other!r}, not the pinned {pins[key]!r}")

    return dict(platform=platform, sourceRevision=revision or None, pins=dict(mesher=pins["mesher"], solver=pins["solver"]),
                onRunner=dict(mesher=on_runner[0], solver=on_runner[1]),
                inReplies={key: sorted(values) for key, values in named.items()},
                listed=listed, results=results, summary=summary, problems=problems)


def handoff_problems(root, revision, platforms=PLATFORMS):
    """The release gate's check: each required platform's run of this very commit, graded again
    from its raw files. `root` holds the downloaded artifacts, one folder each."""
    problems = []
    for platform in platforms:
        directory = Path(root) / f"fea-suite-{platform}-{revision}"
        if not directory.is_dir():
            problems.append(f"fea-suite {platform}: no run of this commit ({directory.name})")
            continue
        verdict = grade(directory, platform)
        if verdict.get("sourceRevision") != revision:
            problems.append(f"fea-suite {platform}: tested {verdict.get('sourceRevision')}, not this commit {revision}")
        problems.extend("fea-suite " + p for p in verdict["problems"])
    return problems


def summary_markdown(verdict):
    lines = [f"## Real-solve FEA suite ({verdict['platform']}): " + ("PASS" if not verdict["problems"] else "FAIL")]
    lines.append(f"Commit `{verdict.get('sourceRevision')}`. Pinned: {verdict.get('pins')}; on the runner: "
                 f"{verdict.get('onRunner')}; in the replies: {verdict.get('inReplies')}.")
    summary = verdict.get("summary")
    if summary:
        lines.append(f"libtest: {summary['passed']} passed, {summary['failed']} failed in {summary['seconds']:.0f} s.")
    lines += ["", "| Test | Result |", "|---|---|"]
    for name in verdict.get("listed", []):
        lines.append(f"| `{name}` | {verdict['results'].get(name, 'no result')} |")
    if verdict["problems"]:
        lines += ["", "Problems:"] + [f"- {p}" for p in verdict["problems"]]
    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    sub = parser.add_subparsers(dest="mode", required=True)
    apt = sub.add_parser("apt-packages", help="print the pinned apt packages as name=version")
    apt.add_argument("--platform", required=True)
    programs = sub.add_parser("programs", help="record the programs' versions and refuse unpinned ones")
    programs.add_argument("--platform", required=True)
    programs.add_argument("--gmsh", required=True)
    programs.add_argument("--ccx", required=True)
    programs.add_argument("--out", required=True)
    graded = sub.add_parser("grade", help="judge a run from its raw files")
    graded.add_argument("--platform", required=True)
    graded.add_argument("--dir", required=True)
    graded.add_argument("--summary", help="append a Markdown summary here ($GITHUB_STEP_SUMMARY)")
    args = parser.parse_args(argv)

    if args.platform not in PINS:
        print(f"fea-suite: no pinned versions for platform {args.platform!r}", file=sys.stderr)
        return 2
    if args.mode == "apt-packages":
        print(" ".join(apt_packages(args.platform)))
        return 0
    if args.mode == "programs":
        found = ask_programs(args.gmsh, args.ccx)
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(found, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        mesher, solver = program_versions(found["gmsh"]["output"], found["ccx"]["output"])
        pins = PINS[args.platform]
        print(f"fea-suite: {args.gmsh} is {mesher}; {args.ccx} is {solver}")
        wrong = [(name, got, want) for name, got, want in (("gmsh", mesher, pins["mesher"]), ("ccx", solver, pins["solver"]))
                 if got != want]
        for name, got, want in wrong:
            print(f"::error::{name} is {got or 'unknown'}, not the pinned {want}")
        return 1 if wrong else 0
    verdict = grade(args.dir, args.platform)
    Path(args.dir, "result.json").write_text(json.dumps(verdict, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    text = summary_markdown(verdict)
    print(text)
    if args.summary:
        with open(args.summary, "a", encoding="utf-8") as summary:
            summary.write(text)
    for problem in verdict["problems"]:
        print(f"::error::{problem}")
    return 1 if verdict["problems"] else 0


if __name__ == "__main__":
    sys.exit(main())
