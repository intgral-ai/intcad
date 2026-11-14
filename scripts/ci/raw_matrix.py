#!/usr/bin/env python3
"""Capture and verify the action-matrix evidence the nightly raw decision needs.

The geometry matrix runs once. Each shard and the separate determinism job record
their real nextest listing, run argv/exit code and JUnit report. The nightly raw
verifier consumes all 17 bundles and reports the raw result from those process
exit codes. It never reads per-action ``ok`` fields and never uses the reviewed
known-failing list to reconstruct a nextest exit code.

A missing bundle, wrong revision, wrong filter, skipped case, timeout or any
non-zero/non-100 process exit is a raw failure, not a pass.
"""
import argparse
import hashlib
import json
import os
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate  # noqa: E402
import main_merge  # noqa: E402

VERSION = 1
SHARDS = tuple(str(number) for number in range(1, 17)) + ("determinism",)
ENVIRONMENT = ("ACTION_MATRIX_JOBS", "ACTION_MATRIX_ACTION_TIMEOUT", "SOLIDCRAFT_GPU",
               "SOLIDCRAFT_REQUIRE_GPU", "WGPU_BACKEND")
RUN_EXPRESSION = {
    **{str(number): "binary(action_matrix) & test(=action_matrix_shard)" for number in range(1, 17)},
    "determinism": "binary(action_matrix) & test(=action_matrix_determinism)",
}


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def expression(argv):
    if "-E" not in argv:
        return None
    index = argv.index("-E")
    return argv[index + 1] if index + 1 < len(argv) else None


def commands_for(bundle):
    commands = gate.parse_json((Path(bundle) / "commands.json").read_bytes())
    listings = [command for command in commands if command["argv"][:3] == ["cargo", "nextest", "list"]]
    runs = [command for command in commands if command["argv"][:3] == ["cargo", "nextest", "run"]]
    gate.require(len(listings) == 1 and len(runs) == 1,
                 "expected exactly one nextest list and one nextest run")
    gate.require(commands.index(listings[0]) < commands.index(runs[0]), "nextest listing must precede the run")
    gate.require(listings[0]["exitCode"] == 0, "nextest listing failed")
    gate.require(runs[0]["exitCode"] in (0, 100), "nextest infrastructure/unapproved exit")
    gate.require(gate.nextest_configuration(listings[0]["argv"], "list") ==
                 gate.nextest_configuration(runs[0]["argv"], "run"),
                 "listing/run configuration mismatch")
    for label, command in (("run", runs[0]), ("listing", listings[0])):
        gate.require("--ignore-default-filter" in command["argv"], label + " did not disable the default filter")
        gate.require("--run-ignored" in command["argv"] and "only" in command["argv"],
                     label + " did not select ignored geometry cases")
    gate.require("--test-threads" in runs[0]["argv"] and runs[0]["argv"][runs[0]["argv"].index("--test-threads") + 1:][:1] == ["2"],
                 "geometry run must use two test threads")
    return commands, listings[0], runs[0]


def validate_bundle(bundle, plan, shard):
    bundle = Path(bundle)
    gate.require(shard in SHARDS, "unexpected shard: " + shard)
    evidence = gate.parse_json((bundle / "matrix-evidence.json").read_bytes())
    if os.environ.get("GITHUB_RUN_ID"):
        gate.require(evidence.get("runId") == os.environ["GITHUB_RUN_ID"], "evidence belongs to another run")
        gate.require(evidence.get("runAttempt") == os.environ.get("GITHUB_RUN_ATTEMPT", ""),
                     "evidence belongs to another run attempt")
    gate.require(evidence.get("version") == VERSION, "unsupported evidence version")
    gate.require(evidence.get("shard") == shard, "evidence shard mismatch")
    gate.require(evidence.get("matrixShard") == ("" if shard == "determinism" else shard + "/16"),
                 "executed matrix shard differs from its evidence")
    environment = evidence.get("environment")
    gate.require(isinstance(environment, dict), "evidence has no environment snapshot")
    gate.require(environment.get("ACTION_MATRIX_JOBS") == "2", "matrix did not use two workers")
    gate.require(environment.get("ACTION_MATRIX_ACTION_TIMEOUT") == "1800", "matrix action timeout changed")
    gate.require(evidence.get("sourceRevision") == plan["sourceRevision"], "evidence sourceRevision mismatch")
    gate.require(evidence.get("planDigest") == gate.digest_for(plan), "evidence planDigest mismatch")
    commands, listing, run = commands_for(bundle)
    gate.require(expression(listing["argv"]) == RUN_EXPRESSION[shard], "wrong listing filter for shard " + shard)
    gate.require(expression(run["argv"]) == RUN_EXPRESSION[shard], "wrong run filter for shard " + shard)
    expected = gate.read_nextest_selection(bundle / "selection.json")
    outcomes = gate.read_junit(bundle / "junit.xml")
    skipped = sorted(key for key, value in outcomes.items() if value == "skipped")
    gate.require(not skipped, "skipped cases are not execution evidence")
    gate.require(set(outcomes) == expected, "expected and executed cases differ")
    failed = sorted(key for key, value in outcomes.items() if value == "failed")
    gate.require(bool(failed) == (run["exitCode"] == 100), "nextest exit disagrees with JUnit")
    gate.require(evidence.get("rawExitCode") == run["exitCode"], "recorded raw exit differs from nextest run")
    gate.require(evidence.get("junitSha256") == sha256(bundle / "junit.xml"), "JUnit digest mismatch")
    gate.require(evidence.get("expectedIds") == sorted(expected), "recorded expected IDs differ from listing")
    gate.require(evidence.get("passedIds") == sorted(key for key, value in outcomes.items() if value == "passed"),
                 "recorded passed IDs differ from JUnit")
    gate.require(evidence.get("failedIds") == failed, "recorded failed IDs differ from JUnit")
    gate.require(evidence.get("skippedIds") == skipped, "recorded skipped IDs differ from JUnit")
    return dict(shard=shard, exitCode=run["exitCode"], expected=len(expected),
                failed=failed, junitSha256=evidence["junitSha256"])


def discover(root):
    root = Path(root)
    found = {}
    for path in sorted(root.rglob("matrix-evidence.json")):
        data = gate.parse_json(path.read_bytes())
        shard = data.get("shard")
        gate.require(isinstance(shard, str) and shard, "evidence has no shard")
        gate.require(shard not in found, "duplicate evidence for shard: " + shard)
        found[shard] = path.parent
    return found


def verify(plan_path, evidence_root, out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    try:
        plan = gate.BoundPlan(Path(plan_path).read_bytes())
        gate.require(plan.get("scope") == "full", "raw geometry matrix needs a full plan")
    except (OSError, ValueError, KeyError) as error:
        result = dict(version=VERSION, sourceRevision="", planDigest="", expectedShards=list(SHARDS),
                      shards={}, rawPassed=False, problems=[str(error)])
        (out / "raw-result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print("RAW FAIL: " + str(error), file=sys.stderr)
        print("raw geometry matrix: RED")
        return 1
    problems, shards = [], {}
    try:
        found = discover(evidence_root)
        missing = sorted(set(SHARDS) - set(found))
        unexpected = sorted(set(found) - set(SHARDS))
        if missing:
            problems.append("missing evidence: " + ", ".join(missing))
        if unexpected:
            problems.append("unexpected evidence: " + ", ".join(unexpected))
        for shard in SHARDS:
            if shard not in found:
                continue
            try:
                shards[shard] = validate_bundle(found[shard], plan, shard)
            except (OSError, ValueError, KeyError) as error:
                problems.append(f"{shard}: {error}")
    except (OSError, ValueError, KeyError) as error:
        problems.append(str(error))
    failed = sorted(shard for shard, value in shards.items() if value["exitCode"] == 100)
    if not problems and failed:
        problems.append("raw nextest failure in: " + ", ".join(failed))
    raw = not problems
    result = dict(version=VERSION, sourceRevision=plan["sourceRevision"], planDigest=gate.digest_for(plan),
                  expectedShards=list(SHARDS), shards=shards, rawPassed=raw, problems=problems)
    (out / "raw-result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for shard in sorted(shards):
        value = shards[shard]
        print(f"{shard}: exit {value['exitCode']}, {value['expected']} expected, "
              f"{len(value['failed'])} failed")
    for problem in problems:
        print("RAW FAIL: " + problem, file=sys.stderr)
    print("raw geometry matrix: " + ("PASS" if raw else "RED"))
    return 0 if raw else 1


def record(plan_path, shard, out):
    plan = gate.BoundPlan(Path(plan_path).read_bytes())
    main_merge.check_revision(plan)
    gate.require(shard in SHARDS, "unexpected shard: " + shard)
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    expression_value = RUN_EXPRESSION[shard]
    common = ["--archive-file", "tests.tar.zst", "--workspace-remap", ".", "--extract-to", ".",
              "--extract-overwrite", "--profile", "ci", "--ignore-default-filter", "--run-ignored", "only",
              "-E", expression_value]
    junit = Path("target/nextest/ci/junit.xml")
    junit.unlink(missing_ok=True)
    commands = main_merge.capture_commands([
        ["cargo", "nextest", "list", *common, "--message-format", "json"],
        ["cargo", "nextest", "run", *common, "--success-output", "final", "--test-threads", "2"],
    ], out)
    if junit.exists():
        shutil.copyfile(junit, out / "junit.xml")
    run = next((command for command in commands if command["argv"][:3] == ["cargo", "nextest", "run"]), None)
    problems, expected, outcomes = [], set(), {}
    try:
        gate.require(run is not None, "nextest run did not execute")
        expected = gate.read_nextest_selection(out / "selection.json")
        outcomes = gate.read_junit(out / "junit.xml")
        gate.require(set(outcomes) == expected, "expected and executed cases differ")
        gate.require("skipped" not in outcomes.values(), "skipped cases are not execution evidence")
        gate.require(("failed" in outcomes.values()) == (run["exitCode"] == 100),
                     "nextest exit disagrees with JUnit")
    except (OSError, ValueError, KeyError) as error:
        problems.append(str(error))
    failed = sorted(key for key, value in outcomes.items() if value == "failed")
    evidence = dict(version=VERSION, shard=shard, matrixShard=os.environ.get("ACTION_MATRIX_SHARD", ""),
                    sourceRevision=plan["sourceRevision"],
                    planDigest=gate.digest_for(plan), runId=os.environ.get("GITHUB_RUN_ID", ""),
                    environment={key: os.environ.get(key, "") for key in ENVIRONMENT},
                    runAttempt=os.environ.get("GITHUB_RUN_ATTEMPT", ""),
                    rawExitCode=run["exitCode"] if run else 1,
                    expectedIds=sorted(expected), passedIds=sorted(key for key, value in outcomes.items() if value == "passed"),
                    failedIds=failed, skippedIds=sorted(key for key, value in outcomes.items() if value == "skipped"),
                    junitSha256=sha256(out / "junit.xml") if (out / "junit.xml").is_file() else "")
    (out / "matrix-evidence.json").write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for problem in problems:
        print("EVIDENCE FAIL: " + problem, file=sys.stderr)
    if problems:
        return 1
    if run["exitCode"] == 100:
        print(f"raw nextest failure: {len(failed)} case(s)")
        return 100
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    recorder = modes.add_parser("run")
    recorder.add_argument("--plan", type=Path, required=True)
    recorder.add_argument("--shard", required=True)
    recorder.add_argument("--out", type=Path, required=True)
    verifier = modes.add_parser("verify")
    verifier.add_argument("--plan", type=Path, required=True)
    verifier.add_argument("--evidence", type=Path, required=True)
    verifier.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.mode == "run":
            return record(args.plan, args.shard, args.out)
        return verify(args.plan, args.evidence, args.out)
    except (OSError, ValueError, KeyError) as error:
        print("raw geometry matrix: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
