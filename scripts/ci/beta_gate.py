#!/usr/bin/env python3
"""Evaluate the narrow Beta promotion gate.

A Beta may carry the reviewed, itemised action-matrix exceptions in
tests/action_matrix/beta-exceptions.json.  It may not carry a missing shard, a
build failure, a timeout, a parse failure, a newly failing action, a changed
error class, or an exception that has started passing.  The stable release gate
does not call this module.
"""
import argparse
import json
from pathlib import Path
import re
import sys

SHARDS = 16
EXCEPTIONS = Path("tests/action_matrix/beta-exceptions.json")


class BetaGateError(ValueError):
    """A Beta gate input or result is not acceptable."""


def require(condition, message):
    if not condition:
        raise BetaGateError(message)


def read_json(path):
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise BetaGateError(f"{path}: {error}") from error


def category(action, message):
    """A stable class for the failure text, independent of numeric offsets.

    Identity is the design/entity/action; this class catches a failure that
    changed from one kernel error family to another.  Unknown text is rejected
    until it is classified deliberately.
    """
    text = " ".join(str(message).lower().split())
    if "picked plane does not match its exact source face" in text:
        return "plane_source_mismatch"
    if "no longer exists as it was picked" in text:
        return "stale_face_reference"
    if "equally near to two edges or faces" in text:
        return "ambiguous_face_selection"
    if "not a fillet's or a chamfer's" in text:
        return "wrong_face_kind"
    if "no face at [" in text:
        return "face_missing_at_point"
    if "export" in text and "does not validate" in text:
        return "export_invalid"
    if "bad inner shells" in text:
        return "boolean_multilump_void"
    if "bad vertex gap" in text:
        return "boolean_vertex_gap"
    if "uvedge/uvedge gap" in text:
        return "boolean_uv_gap"
    if "nonmonotonic cpts" in text:
        return "boolean_nurbs_nonmonotonic"
    if "boolean" in text and ("check refuses" in text or "operation" in text or "failed" in text):
        return "boolean_refused"
    if action.startswith("face.shell"):
        if "status 1001" in text:
            return "shell_backend_1001"
        if "walls would meet along a line" in text:
            return "shell_walls_meet"
        if "not supported yet" in text:
            return "shell_not_supported"
        if "not a closed solid" in text:
            return "shell_not_closed"
        if "check refuses" in text:
            return "shell_check_refused"
    if action.startswith(("edge.fillet", "edge.chamfer")):
        if "status 1001" in text:
            return "blend_backend_1001"
        if "larger than the faces" in text:
            return "blend_too_large"
        if "not supported yet" in text:
            return "blend_not_supported"
        if "not a closed solid" in text:
            return "blend_not_closed"
        if "check refuses" in text:
            return "blend_check_refused"
    if action.startswith("face.draft"):
        if "status 1001" in text:
            return "draft_backend_1001"
        if "not a closed solid" in text:
            return "draft_not_closed"
        if "check refuses" in text:
            return "draft_check_refused"
    if action.startswith("face.hole"):
        if "check refuses" in text:
            return "hole_boolean_refused"
    if "status 1001" in text:
        return "backend_operation_1001"
    if "not a closed solid" in text:
        return "backend_not_closed_solid"
    if "not supported yet" in text:
        return "backend_not_supported"
    if "check refuses the result" in text:
        return "backend_check_refused"
    raise BetaGateError(f"unclassified failure for {action}: {message.splitlines()[0][:160]}")


def identity(entry, where="entry"):
    require(isinstance(entry, dict), f"{where}: not an object")
    for field in ("design", "entity", "action", "kind"):
        require(isinstance(entry.get(field), str) and entry[field], f"{where}: invalid {field}")
    return (entry["design"], entry["entity"], entry["action"])


def load_exceptions(path):
    data = read_json(path)
    require(isinstance(data, dict), "exceptions: not an object")
    require(data.get("format") == 1, "exceptions: unsupported format")
    source = data.get("source")
    require(isinstance(source, dict), "exceptions: missing source")
    require(type(source.get("totalActions")) is int and source["totalActions"] > 0,
            "exceptions: invalid source.totalActions")
    require(isinstance(source.get("run"), int) and source["run"] > 0, "exceptions: invalid source.run")
    require(isinstance(source.get("sha"), str) and re.fullmatch(r"[0-9a-f]{40}", source["sha"]),
            "exceptions: invalid source.sha")
    entries = data.get("entries")
    require(isinstance(entries, list) and entries, "exceptions: missing entries")
    result, previous = {}, None
    for index, entry in enumerate(entries):
        key = identity(entry, f"exceptions entry {index}")
        require(previous is None or previous < key, "exceptions: entries must be sorted and unique")
        previous = key
        require(isinstance(entry.get("issue"), str) and re.fullmatch(r"#\d+", entry["issue"]),
                f"exceptions {key}: invalid issue")
        require(entry.get("category") == category(entry["action"], entry.get("message", "")),
                f"exceptions {key}: category does not match its message")
        result[key] = entry
    return data, result


def load_matrix(directory):
    directory = Path(directory)
    require(directory.is_dir(), f"matrix: missing directory {directory}")
    files = sorted(directory.rglob("results-*.json"))
    found = {}
    for path in files:
        match = re.fullmatch(r"results-(\d+)\.json", path.name)
        require(match is not None, f"matrix: unexpected result file {path}")
        shard = int(match.group(1))
        require(1 <= shard <= SHARDS, f"matrix: invalid shard {shard}")
        require(shard not in found, f"matrix: duplicate shard {shard}")
        found[shard] = path
    require(set(found) == set(range(1, SHARDS + 1)),
            "matrix: missing shards " + ", ".join(str(s) for s in sorted(set(range(1, SHARDS + 1)) - set(found))))
    entries, seen, failures = [], {}, []
    for shard in range(1, SHARDS + 1):
        data = read_json(found[shard])
        require(isinstance(data, list), f"matrix shard {shard}: not a list")
        for entry in data:
            key = identity(entry, f"matrix shard {shard}")
            require(type(entry.get("ok")) is bool, f"matrix {key}: invalid ok")
            require(key not in seen, f"matrix: duplicate action {key}")
            entry["_shard"] = shard
            seen[key] = entry
            entries.append(entry)
            if not entry["ok"]:
                failures.append(entry)
    return entries, failures


def evaluate_matrix(directory, exceptions):
    data, expected = load_exceptions(exceptions)
    entries, failures = load_matrix(directory)
    require(len(entries) == data["source"]["totalActions"],
            f"matrix: {len(entries)} actions, expected {data['source']['totalActions']} from the reviewed run")
    unexcused, changed = [], []
    actual = {}
    for failure in failures:
        key = identity(failure)
        actual[key] = failure
        exception = expected.get(key)
        if exception is None:
            unexcused.append(key)
            continue
        if failure["kind"] != exception["kind"]:
            changed.append(f"{key}: kind {failure['kind']} != reviewed {exception['kind']}")
            continue
        got = category(failure["action"], failure.get("message", ""))
        if got != exception["category"]:
            changed.append(f"{key}: category {got} != reviewed {exception['category']}")
    fixed = [key for key in expected if key not in actual]
    problems = []
    if unexcused:
        problems.append("unexcused action failures: " + ", ".join(" :: ".join(key) for key in unexcused[:12]) +
                        (" ..." if len(unexcused) > 12 else ""))
    if changed:
        problems.append("changed failure class: " + "; ".join(changed[:12]) +
                        (" ..." if len(changed) > 12 else ""))
    if fixed:
        problems.append("reviewed exceptions now pass or are missing; remove them: " +
                        ", ".join(" :: ".join(key) for key in fixed[:12]) +
                        (" ..." if len(fixed) > 12 else ""))
    return dict(actions=len(entries), failures=len(failures), excused=len(failures) - len(unexcused) - len(changed),
                unexcused=len(unexcused), changed=len(changed), fixed=len(fixed),
                source=dict(data["source"]), problems=problems)


def evaluate_properties(directory):
    directory = Path(directory)
    raw = directory / "target/nextest/ci/raw-exit.txt"
    report = directory / "target/nextest/ci/known-failure-exception.txt"
    require(raw.is_file(), "properties: missing raw-exit.txt")
    require(report.is_file(), "properties: missing known-failure-exception.txt")
    raw_exit = raw.read_text(encoding="utf-8").strip()
    text = report.read_text(encoding="utf-8")
    require(raw_exit in ("0", "100"), f"properties: raw nextest exit {raw_exit!r} is not a test result")
    require("known-failure-exception: FAILED" not in text, "properties: grader reported a failure")
    require("0 unexcused" in text, "properties: report does not prove zero unexcused failures")
    return dict(rawExit=int(raw_exit), unexcused=0)


REQUIRED_JOBS = (
    "ci / Select checks",
    "ci / Format and repository checks",
    "ci / Native Linux Clippy",
    "ci / Native Linux selected regression gate",
    "ci / Compile WebAssembly",
    "ci / Mastra contract (Rust)",
    "ci / Mastra runtime (Node)",
    "ci / Mastra contract (TypeScript)",
    "ci / Host/runtime integration",
    "ci / Mastra artifact policy",
    "ci / Packaging regressions",
    "ci / Doc tests",
    "ci / Reference tests",
    "ci / Build full-baseline archive",
    "ci / Action matrix determinism",
    "ci / Windows D3D12 FXC shaders",
    "ci / Windows app startup",
    "ci / Dependency advisories",
    "kernel-rig / Build the CLI",
    "kernel-rig / kernel-rig",
    "packages / All nonpublishing package builds",
)


def load_jobs(path):
    data = read_json(path)
    jobs = data.get("jobs") if isinstance(data, dict) else data
    require(isinstance(jobs, list) and jobs, "jobs: no job records")
    result = {}
    for job in jobs:
        require(isinstance(job, dict) and isinstance(job.get("name"), str), "jobs: invalid record")
        require(job["name"] not in result, f"jobs: duplicate {job['name']}")
        result[job["name"]] = job
    return result


def evaluate_jobs(path, matrix_result, properties):
    jobs = load_jobs(path)
    problems = []
    for name in REQUIRED_JOBS:
        job = jobs.get(name)
        if job is None:
            problems.append(f"missing job: {name}")
        elif job.get("conclusion") != "success":
            problems.append(f"job not successful: {name}: {job.get('conclusion') or job.get('status')}")
    shards = [name for name in jobs if re.fullmatch(r"ci / Action matrix \(\d+/16\)", name)]
    if len(shards) != SHARDS:
        problems.append(f"action matrix shards: found {len(shards)}, expected {SHARDS}")
    for name in shards:
        job = jobs[name]
        if job.get("conclusion") not in ("success", "failure"):
            problems.append(f"action matrix shard not completed: {name}: {job.get('conclusion') or job.get('status')}")
    if matrix_result["problems"]:
        problems.extend("action matrix: " + problem for problem in matrix_result["problems"])
    prop = jobs.get("ci / Full baseline randomized properties (raw, many cases)")
    if prop is None:
        problems.append("missing job: ci / Full baseline randomized properties (raw, many cases)")
    elif prop.get("conclusion") not in ("success", "failure"):
        problems.append(f"property job not completed: {prop.get('conclusion') or prop.get('status')}")
    elif properties is None:
        problems.append("property job did not supply a successful grader report")
    return dict(problems=problems, propertyConclusion=prop.get("conclusion") if prop else None)


def evaluate(plan_path, evidence, matrix, properties, jobs, exceptions):
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import gate  # noqa: E402

    plan = gate.BoundPlan(Path(plan_path).read_bytes())
    receipts = {}
    evidence = Path(evidence)
    if evidence.is_dir():
        for path in sorted(evidence.rglob("receipt.json")):
            receipt = gate.parse_json(path.read_bytes())
            job = receipt.get("jobId")
            require(job not in receipts, f"duplicate receipt: {job}")
            receipts[job] = receipt
    selected = {job for job, entry in plan["jobs"].items() if entry["decision"] == "run"}
    missing = sorted(selected - set(receipts))
    if missing:
        raise BetaGateError("missing selected receipts: " + ", ".join(missing))
    # A receipt validates its real commands and, for the native lane, the
    # reviewed known-failing grader.  Actual per-job conclusions are checked
    # separately from the workflow run, so a green wrapper cannot stand in.
    gate.evaluate(plan, {job: "success" for job in selected}, receipts)
    matrix_result = evaluate_matrix(matrix, exceptions)
    properties_result = evaluate_properties(properties)
    jobs_result = evaluate_jobs(jobs, matrix_result, properties_result)
    problems = list(jobs_result["problems"])
    return dict(version=1, verdict="accepted-beta" if not problems else "rejected",
                sourceRevision=plan["sourceRevision"], matrix=matrix_result,
                properties=properties_result, jobs=jobs_result, problems=problems)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    evaluate_parser = sub.add_parser("evaluate")
    evaluate_parser.add_argument("--plan", required=True)
    evaluate_parser.add_argument("--evidence", required=True)
    evaluate_parser.add_argument("--matrix", required=True)
    evaluate_parser.add_argument("--properties", required=True)
    evaluate_parser.add_argument("--jobs", required=True)
    evaluate_parser.add_argument("--exceptions", default=str(EXCEPTIONS))
    evaluate_parser.add_argument("--out", required=True)
    args = parser.parse_args()
    try:
        result = evaluate(args.plan, args.evidence, args.matrix, args.properties, args.jobs, args.exceptions)
        Path(args.out).write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(json.dumps(result, indent=2, sort_keys=True))
        if result["problems"]:
            return 1
    except (OSError, ValueError, TypeError, KeyError) as error:
        print("beta-gate: " + str(error), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
