#!/usr/bin/env python3
"""Decide whether a release-gate run is a fixed candidate.

The release gate is expensive.  Preparation commits, including commits pushed to
an already-ready pull request, must not start the full matrix automatically.
Only an explicit candidate transition starts it:

* opened (already ready), reopened (already ready), or ready_for_review;
* a main merge-group run;
* a manual/reusable call.

A draft run still creates the required ``release-gate`` context, but it fails
with a clear reason.  It is never reported as success, so a draft cannot be
merged by accident.  A ready PR with a new commit has no release-gate run for
that new SHA; mark it draft and ready again to verify the new candidate.
A manual dispatch creates ``release-gate-diagnostic`` instead: it checks out the
selected ref, not the PR merge SHA, and cannot satisfy the required context.
"""
import argparse
import json
import os
import sys


def _is_true(value):
    return str(value).strip().lower() in ("1", "true", "yes")


def state(event, action, draft, head_ref, base_ref, merge_base, merge_head):
    """Return (verify, reason) for one workflow event.

    ``verify`` is a string because it is written to ``$GITHUB_OUTPUT``.
    """
    event = (event or "").strip()
    action = (action or "").strip()
    if event == "merge_group":
        if (merge_base or "").startswith("refs/heads/main"):
            return True, "main merge group"
        # The existing release-gate job reports not_applicable for develop
        # merge groups.  Keep that behaviour; there is no merge queue here.
        return True, "merge group (applicability is decided by the gate)"
    if event in ("workflow_dispatch", "workflow_call"):
        return True, "explicit run"
    if event != "pull_request":
        return False, f"{event or 'unknown'}: not a release candidate event"
    if (base_ref or "") != "main":
        return False, "not a pull request targeting main"
    if _is_true(draft):
        return False, "draft candidate; the release gate has not been started"
    if action in ("opened", "ready_for_review", "reopened"):
        return True, action
    return False, (
        f"{action or 'unknown'}: candidate changed; mark the PR draft and ready "
        "again to verify this exact SHA"
    )


def context(event):
    """The check-run context this run can create.

    A manual run is a diagnostic: it checks out the selected ref, not the PR
    merge SHA, so it must not be able to create the required ``release-gate``
    context. Pull-request and merge-group runs keep the required name.
    """
    return "release-gate-diagnostic" if (event or "").strip() == "workflow_dispatch" else "release-gate"


def state_from_env(env=None):
    env = os.environ if env is None else env
    return state(
        env.get("EVENT_NAME", ""),
        env.get("ACTION", ""),
        env.get("DRAFT", ""),
        env.get("HEAD_REF", ""),
        env.get("BASE_REF", ""),
        env.get("MERGE_BASE", ""),
        env.get("MERGE_HEAD", ""),
    )


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", default=os.environ.get("EVENT_NAME", ""))
    parser.add_argument("--action", default=os.environ.get("ACTION", ""))
    parser.add_argument("--draft", default=os.environ.get("DRAFT", ""))
    parser.add_argument("--head-ref", default=os.environ.get("HEAD_REF", ""))
    parser.add_argument("--base-ref", default=os.environ.get("BASE_REF", ""))
    parser.add_argument("--merge-base", default=os.environ.get("MERGE_BASE", ""))
    parser.add_argument("--merge-head", default=os.environ.get("MERGE_HEAD", ""))
    parser.add_argument("--out", help="write GitHub Actions step outputs here")
    parser.add_argument("--json", action="store_true", help="print both fields as JSON")
    args = parser.parse_args(argv)
    verify, reason = state(args.event, args.action, args.draft,
                           args.head_ref, args.base_ref, args.merge_base, args.merge_head)
    reason = " ".join(str(reason).split()) or "no reason"
    check_context = context(args.event)
    if args.json:
        print(json.dumps({"verify": verify, "reason": reason, "context": check_context}, sort_keys=True))
    else:
        print(f"verify={verify}")
        print(f"reason={reason}")
        print(f"context={check_context}")
    if args.out:
        with open(args.out, "a", encoding="utf-8") as out:
            out.write(f"verify={'true' if verify else 'false'}\n")
            out.write(f"reason={reason}\n")
            out.write(f"context={check_context}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
