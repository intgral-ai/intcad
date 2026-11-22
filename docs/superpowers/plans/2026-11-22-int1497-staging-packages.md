# INT-1497 Staging Packages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add one nonpublishing CI entry for installable Intcad staging packages on three desktop platforms.

**Architecture:** A manual wrapper snapshots the public `staging` environment configuration once. The reusable package workflow consumes that snapshot, restricts the staging matrix, and binds configuration provenance to package evidence.

**Tech Stack:** GitHub Actions, Python standard library, PyYAML for workflow assertions, actionlint.

**Spec:** `docs/superpowers/specs/2026-11-22-int1497-staging-packages-design.md`

## Global Constraints

- Linux x86_64, macOS universal (both compile targets), Windows x64 only in staging.
- No pushes, dispatch, signing credentials, notarization, publication or production access.
- Fixed checkout SHA; keep existing non-staging flavor, targets, artifact names and metadata.
- Do not perform full builds locally. A successful build is not human install or account acceptance.

## Review Focus

- All three variables absent or any subset present must fail before packaging.
- A malformed public ID or non-origin URL must fail without echoing the rejected value.
- The universal macOS join must not be the first consumer of the staging configuration.
- The existing reusable gate and ordinary manual caller must not receive staging configuration.
- Artifact evidence must refer to the same source revision, public snapshot and hashed package bytes.

### Task 1: Manual staging entry, compile-time configuration and evidence

**Files:** Create `.github/workflows/staging-packages.yml` and
`scripts/ci/staging_account_config.py`; modify `.github/workflows/release.yml`,
`.github/workflows/packaging-lint.yml`, `scripts/ci/test_release_content.py`,
and `docs/account/desktop-configuration.md`.

**Interfaces:** Workflow input `staging_account_config` is a JSON string, default empty.
`validate_config(values: Mapping[str, str]) -> dict[str, str]` accepts exactly the three
environment keys in the spec or raises `ValueError` naming only invalid/missing keys.
The helper CLI has `snapshot` (writes `config=<compact JSON>` to `GITHUB_OUTPUT`)
and `inject` (validates `STAGING_ACCOUNT_CONFIG`, writes those three compile keys to
`GITHUB_ENV`); values enter steps through environment variables, never command expansion.
Staging build evidence adds `accountConfig: {"source": "github-environment:staging",
"values": {...}}`; ordinary `build.json` has no new field.

- [ ] **Write failing contract tests** in `scripts/ci/test_release_content.py`:
  assert all eight presence combinations (only all three passes), whitespace/control and
  injection characters in IDs, HTTP/credentials/query/fragment/non-origin path/invalid port
  in origins, and all three exact values round-tripping with synthetic HTTPS hosts. Verify
  `inject` with bad JSON writes no env file and does not disclose values. Via parsed YAML
  assert the wrapper is dispatch-only/read-only, its snapshot job is bound to `staging`, and
  its reusable call depends on snapshot. Assert expected staging matrices, compile step order,
  no staging env injection in the macOS join, distinct staging artifact labels, no production
  refs/secrets/publication, and ordinary names and metadata unchanged. Test metadata generation
  with temporary fixture bytes: SHA matches the file and configuration matches the snapshot.
- [ ] **Observe RED:** `python3 -I scripts/ci/test_release_content.py` fails new tests for missing
  staging entry/helper; distinguish any existing baseline failures.
- [ ] **Implement the helper and wrapper:** validate before output; exclusive HTTPS origins
  (bare origin, optional valid port) and public WorkOS `client_` followed by ASCII alphanumerics.
  Pass one validated compact JSON snapshot to `release.yml`. Do not inherit secrets.
- [ ] **Wire the reusable workflow:** reject nonempty staging input from any caller other than
  `staging-packages.yml` on `workflow_dispatch` in the `version` job before dependent runners.
  Use the existing Beta reduced-platform path to skip other jobs, and select Linux x86_64 and
  Windows x64 only in staging; preserve both macOS build architectures. Conditionally call
  `inject` before Linux, Windows, and macOS-arch compile steps. Keep the snapshot out of the
  macOS join's compile env; pass it there only to generate final evidence. Prefix the staging
  upload/download artifact labels, including intermediate binaries and evidence; keep package
  filenames and ordinary package IDs stable. Validate the same snapshot in the evidence step;
  do not re-read GitHub variables.
- [ ] **Document the operator gate** in `desktop-configuration.md`: exact public tuple from
  `staging`, no repository/organization shadowing, run ref must point to the approved SHA,
  manual wrapper must be discoverable for dispatch, and signed distribution/installed UX
  evidence remain out of scope. Add the regression command to packaging lint if not already run.
- [ ] **Observe GREEN:** run `python3 -I scripts/ci/test_release_content.py`,
  `python3 -I scripts/ci/test_gate.py`, `actionlint .github/workflows/staging-packages.yml .github/workflows/release.yml .github/workflows/packaging-lint.yml`,
  and `git diff --check`. Record exact outcomes; resolve actionable diagnostics or identify
  unchanged baseline diagnostics before claiming a pass. No hosted run.
- [ ] **Commit the scoped implementation locally**, then obtain independent Spec and Quality
  reviews and final integration review. Report fixed SHA and diff to the owner before any push
  or dispatch. Do not remove the worktree before the checkpoint is safely retained.
