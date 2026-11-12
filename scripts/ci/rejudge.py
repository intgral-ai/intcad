#!/usr/bin/env python3
"""Record and re-judge Beta gate evidence without re-running geometry.

This is a diagnostic entry point, not a replacement for the required
``release-gate``.  It reads the raw results already uploaded by a Beta run,
verifies their identity and hashes, and re-runs only the decision rules in
``scripts/ci/beta_gate.py``.

A re-judgment is allowed only when the execution inputs are byte-identical.
``execution.json`` records the source revision, run identity, tree, the Beta
gate script hash, the rules hash and every input digest.  A changed exception
or baseline changes the rules hash and is allowed; a changed plan, matrix,
property report, job list, receipt, gate script or source revision is refused.
Digests are integrity checks, not signatures or proof of the evidence's origin.
Timeouts, cancellations, missing shards and incomplete results still fail in
``beta_gate.evaluate``.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import beta_gate  # noqa: E402

FORMAT = 1
ALGORITHM = "beta-gate-rejudge/v1"
EXECUTION = "execution.json"
PLAN = "selection/plan.json"
EVIDENCE = "evidence"
MATRIX = "matrix"
PROPERTIES = "properties"
JOBS = "jobs.json"
DEFAULT_EXCEPTIONS = "tests/action_matrix/beta-exceptions.json"
DEFAULT_BASELINE = "tests/action_matrix/baseline.json"


class RejudgeError(ValueError):
    """The execution evidence is missing, incomplete or for another source."""


def require(condition, message):
    if not condition:
        raise RejudgeError(message)


def sha256(path):
    path = Path(path)
    require(path.is_file(), f"missing input: {path}")
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tree_digest(entries, *, label):
    """Hash relative names and bytes in stable order; an empty input is not evidence."""
    entries = sorted((str(name), Path(path)) for name, path in entries)
    require(entries, f"{label}: no files")
    digest = hashlib.sha256()
    for name, path in entries:
        require(path.is_file(), f"{label}: missing {path}")
        digest.update(name.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def matrix_digest(matrix):
    matrix = Path(matrix)
    require(matrix.is_dir(), f"matrix: missing directory {matrix}")
    files = sorted(matrix.rglob("results-*.json"))
    names = [path.name for path in files]
    require(len(names) == len(set(names)), "matrix: duplicate result file")
    return tree_digest(((path.name, path) for path in files), label="matrix")


def evidence_digest(evidence):
    evidence = Path(evidence)
    require(evidence.is_dir(), f"evidence: missing directory {evidence}")
    files = sorted(evidence.rglob("receipt.json"))
    return tree_digest(((path.relative_to(evidence), path) for path in files), label="evidence")


def properties_digest(properties):
    path = Path(properties) / "target/nextest/ci/junit.xml"
    require(path.is_file(), f"properties: missing {path}")
    return sha256(path)


def rules_digest(exceptions, baseline):
    return tree_digest((("exceptions", exceptions), ("baseline", baseline)), label="rules")


def gate_script_digest():
    return sha256(Path(beta_gate.__file__))


def load_manifest(path):
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise RejudgeError(f"execution: {error}") from error
    require(isinstance(data, dict), "execution: not an object")
    require(data.get("format") == FORMAT, "execution: unsupported format")
    require(data.get("algorithm") == ALGORITHM, "execution: unsupported algorithm")
    require(isinstance(data.get("sourceRevision"), str)
            and re.fullmatch(r"[0-9a-f]{40}", data["sourceRevision"]),
            "execution: invalid sourceRevision")
    require(type(data.get("runId")) is int and data["runId"] > 0, "execution: invalid runId")
    require(type(data.get("runAttempt")) is int and data["runAttempt"] > 0,
            "execution: invalid runAttempt")
    require(isinstance(data.get("gateScriptSha256"), str)
            and re.fullmatch(r"[0-9a-f]{64}", data["gateScriptSha256"]),
            "execution: invalid gateScriptSha256")
    for field in ("planDigest", "matrixDigest", "propertiesDigest", "jobsDigest",
                  "evidenceDigest", "rulesDigest"):
        require(isinstance(data.get(field), str)
                and re.fullmatch(r"[0-9a-f]{64}", data[field]),
                f"execution: invalid {field}")
    tree = data.get("treeSha")
    require(tree is None or (isinstance(tree, str) and re.fullmatch(r"[0-9a-f]{40}", tree)),
            "execution: invalid treeSha")
    return data


def current(root, args):
    root = Path(root)
    return {
        "planDigest": sha256(root / args.plan),
        "matrixDigest": matrix_digest(root / args.matrix),
        "propertiesDigest": properties_digest(root / args.properties),
        "jobsDigest": sha256(root / args.jobs),
        "evidenceDigest": evidence_digest(root / args.evidence),
        "rulesDigest": rules_digest(args.exceptions, args.baseline),
    }


def check(root, execution, args):
    manifest = load_manifest(execution)
    actual = current(root, args)
    for field, value in actual.items():
        if field == "rulesDigest":
            continue
        require(value == manifest[field], f"{field} changed: execution is not reusable")
    require(manifest["gateScriptSha256"] == gate_script_digest(),
            "beta_gate.py changed; re-run the execution instead of re-judging it")
    plan_path = Path(root) / args.plan
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    require(plan.get("sourceRevision") == manifest["sourceRevision"],
            "plan sourceRevision does not match execution")
    if args.candidate_sha:
        require(args.candidate_sha == manifest["sourceRevision"],
                "candidate SHA differs from the execution source; execute the candidate")
    if args.candidate_tree:
        require(manifest.get("treeSha") is not None,
                "execution has no tree identity; cannot prove candidate tree")
        require(args.candidate_tree == manifest["treeSha"],
                "candidate tree differs from the execution tree; execute the candidate")
    return manifest, actual["rulesDigest"] != manifest["rulesDigest"]


def record(args):
    root = Path(args.root)
    source = args.source_sha or os.environ.get("GITHUB_SHA", "")
    run_id = args.run_id or os.environ.get("GITHUB_RUN_ID", "")
    run_attempt = args.run_attempt or os.environ.get("GITHUB_RUN_ATTEMPT", "")
    repository = args.repository or os.environ.get("GITHUB_REPOSITORY", "")
    workflow = args.workflow or os.environ.get("GITHUB_WORKFLOW", "")
    event = args.event or os.environ.get("GITHUB_EVENT_NAME", "")
    runner = {
        "os": args.runner_os or os.environ.get("RUNNER_OS", ""),
        "arch": args.runner_arch or os.environ.get("RUNNER_ARCH", ""),
        "image": args.runner_image or os.environ.get("ImageOS", ""),
    }
    require(isinstance(source, str) and re.fullmatch(r"[0-9a-f]{40}", source),
            "source SHA is required (40 lowercase hex)")
    try:
        run_id, run_attempt = int(run_id), int(run_attempt)
    except (TypeError, ValueError) as error:
        raise RejudgeError("run id and attempt must be integers") from error
    require(run_id > 0 and run_attempt > 0, "run id and attempt must be positive")
    require(repository and workflow and event, "repository, workflow and event are required")
    require(args.tree_sha is None or re.fullmatch(r"[0-9a-f]{40}", args.tree_sha),
            "tree SHA must be 40 lowercase hex")
    actual = current(root, args)
    manifest = {
        "format": FORMAT,
        "algorithm": ALGORITHM,
        "sourceRevision": source,
        "treeSha": args.tree_sha or None,
        "runId": run_id,
        "runAttempt": run_attempt,
        "repository": repository,
        "workflow": workflow,
        "event": event,
        "runner": runner,
        "gateScriptSha256": gate_script_digest(),
        **actual,
    }
    out = Path(args.out)
    out.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(manifest, indent=2, sort_keys=True))
    return 0


def evaluate(args):
    root = Path(args.root)
    manifest, rules_changed = check(root, args.execution, args)
    result = beta_gate.evaluate(
        root / args.plan,
        root / args.evidence,
        root / args.matrix,
        root / args.properties,
        root / args.jobs,
        args.exceptions,
    )
    current_rules = current(root, args)["rulesDigest"]
    report = {
        "version": 1,
        "formalGate": False,
        "algorithm": ALGORITHM,
        "execution": manifest,
        "rules": {
            "digest": manifest["rulesDigest"],
            "currentDigest": current_rules,
            "changedSinceExecution": rules_changed,
            "gateScriptSha256": manifest["gateScriptSha256"],
        },
        "reuse": {
            "allowed": True,
            "sameSourceRevision": True,
            "rulesOnly": rules_changed,
            "note": "Diagnostic re-judgment only; the required release-gate still needs this exact candidate.",
        },
        "notReexecuted": [
            "action matrix geometry",
            "randomized property execution",
            "package builds and installed checks",
        ],
        "stillRequiredForCandidate": [
            "all release-gate jobs at the candidate SHA",
            "a rebuild and install check for every package at that SHA",
        ],
        **result,
    }
    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.out:
        Path(args.out).write_text(text, encoding="utf-8")
    print(text)
    return 0 if not report.get("problems") else 1


def parser():
    top = argparse.ArgumentParser(description=__doc__)
    sub = top.add_subparsers(dest="command", required=True)
    for name in ("record", "check", "evaluate"):
        p = sub.add_parser(name)
        p.add_argument("--root", default=".")
        p.add_argument("--plan", default=PLAN)
        p.add_argument("--evidence", default=EVIDENCE)
        p.add_argument("--matrix", default=MATRIX)
        p.add_argument("--properties", default=PROPERTIES)
        p.add_argument("--jobs", default=JOBS)
        p.add_argument("--exceptions", default=DEFAULT_EXCEPTIONS)
        p.add_argument("--baseline", default=DEFAULT_BASELINE)
        if name in ("check", "evaluate"):
            p.add_argument("--execution", default=EXECUTION)
            p.add_argument("--candidate-sha")
            p.add_argument("--candidate-tree")
        if name == "record":
            p.add_argument("--source-sha")
            p.add_argument("--tree-sha")
            p.add_argument("--run-id")
            p.add_argument("--run-attempt")
            p.add_argument("--repository")
            p.add_argument("--workflow")
            p.add_argument("--event")
            p.add_argument("--runner-os")
            p.add_argument("--runner-arch")
            p.add_argument("--runner-image")
            p.add_argument("--out", required=True)
        if name == "evaluate":
            p.add_argument("--out")
    return top


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        if args.command == "record":
            return record(args)
        if args.command == "check":
            manifest, rules_changed = check(args.root, args.execution, args)
            print(json.dumps({
                "execution": manifest["sourceRevision"],
                "runId": manifest["runId"],
                "runAttempt": manifest["runAttempt"],
                "rulesChanged": rules_changed,
                "reusable": True,
            }, indent=2, sort_keys=True))
            return 0
        return evaluate(args)
    except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError) as error:
        print("rejudge: " + str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
