# Validation policy

This is the authoritative rule for how much validation a change needs. Other
documents link here instead of repeating the rule.

The policy follows the changed inputs, not the number of commits. A check may be
reused only when the worktree state and its inputs are unchanged. A check that
did not run is never reported as passed.

## 1. Development iteration

Run the checks for the area being changed. Typical commands are
`cargo fmt --all -- --check`, `cargo clippy -p <crate> --all-targets -- -D warnings`,
`cargo test --profile ci -p <crate> --lib -- <test>`, or
`cargo xtask test -- <test>`. A UI, packaging or wasm change also runs its owning
check. Record the exact command and its exit status, not just "tests pass".

Changing a second area adds that area's checks. Changing the same inputs again
requires the same checks again.

## 2. Commit and pull request

Run the repository checks that the change selects. On a capable environment,
`cargo xtask ci` is the local full gate: fmt, assets, icons, licences,
runtime-artifact policy, lists, skills, layers, UI containers, mesh decisions,
workspace Clippy, wasm check, doc tests, web-offline test, and the selected
nextest run. It does **not** run the action matrix, the kernel rig, the packaging
matrix or the full native suite; CI does those at their own levels.

A preparation commit that touches a shared CI script or workflow still runs the
repository checks and the bounded pull-request test lane; it does not start the
full release matrix. The shared-input rule still builds the affected consumers
for Clippy/wasm, so it is not a free pass.

If the environment cannot run the whole gate, run the targeted checks and say
exactly what could not run. The pull-request CI lane is authoritative for the
bounded affected selection. `cargo xtask ci` is not required again for every
commit when no input changed and the previous result is still valid.

## 3. Integration

Pull requests to `develop` run the affected selection and the required `ci`
context. Pushes, merge groups and scheduled runs use the full policy. Heavy
tests stay off ordinary PRs and run in `native-baseline.yml` or the release
gate. Follow the existing risk classification and `scripts/ci/plan.py`; do not
replace it with a hand-written list.

## 4. Release

A release is validated at one fixed SHA. The `release-gate` required context is
run by `release-gate.yml` on an explicit candidate transition, not by every
preparation push. All packages are rebuilt at that SHA; old artifacts and old
job results never authorize a new candidate. The only exception is the offline
re-judgment described below, which is diagnostic unless a future change wires it
into the gate with the same identity checks.

See [Releasing](releasing.md) and [Beta promotion](beta-promotion.md) for the
candidate flow.

## Test value review

A new or changed test must state the behaviour or failure it protects and why
the existing tests are insufficient. Reuse an existing test when it already
covers the change. Use the cheapest layer that proves the behaviour; a full
app, child process, network fixture or whole-corpus sweep needs a reason in the
PR. Do not change an assertion, snapshot, golden or baseline only to make CI
green: explain the contract or evidence change in the PR.

This is a review question, not a new pipeline, coverage gate or AI review step.

## Candidate preparation and explicit verification

For a promotion branch (`release/vX.Y.Z[-beta.N]`) targeting `main`:

1. Keep the PR draft while preparing. The direct affected `ci` lane still runs
   on preparation commits; the full release gate does not.
2. Mark the PR ready when the candidate is fixed. `ready_for_review` (or opening
   an already-ready PR) starts the full gate on that candidate.
3. If a commit is added after the gate starts, the new SHA has no release-gate
   result. The previous result is not reused. To verify the new candidate,
   convert the PR to draft and ready again. A manual dispatch is diagnostic:
   it creates `release-gate-diagnostic`, not the required `release-gate`.
4. A base-branch update that makes the candidate behind is blocked by strict
   up-to-date even when an old head-bound check is green; the temporary probe
   #116 reported `mergeStateStatus: BEHIND`. Update the candidate branch first
   (creating a new head and invalidating the old check), then mark draft→ready
   or reopen to start the new run. Draft→ready alone does not merge the base.
   If the new base is already reachable from the head, there is no new base
   content; the head-bound check covers the same head tree.
5. A draft run still reports `release-gate`, but the job fails with
   "release gate not started". A draft or an intermediate commit therefore
   cannot accidentally satisfy the required context.
6. After a failure, read the recorded job and evidence first, make the smallest
   repair, and run only the affected checks. Start the next full gate only when
   the candidate is fixed.
7. Do not cancel another task's running CI. A superseded candidate's old check
   stays on its old SHA and cannot authorize the new one.

The direct affected `ci` lane and the full `release-gate` are not duplicates:
the first is the bounded preparation signal, the second is the full candidate
proof. `ci.yml` no longer reacts to `ready_for_review`, so the ready transition
does not start both.

| Event | Check-run SHA | Evidence SHA | Check context | Required or diagnostic |
|---|---|---|---|---|
| PR draft or ready | PR head SHA | PR merge SHA | `release-gate` | required; draft fails, ready run succeeds only with full evidence |
| PR with a new commit after ready | new head SHA | new merge SHA | no new run | required check missing; blocked |
| base branch updated | head SHA may stay the same | merge SHA can change | `BEHIND` on strict up-to-date; no automatic new check | blocked; update the candidate branch (new head) then verify; draft→ready alone does not merge the base |
| manual dispatch | selected ref SHA | selected ref SHA | `release-gate-diagnostic` | diagnostic only |
| main merge group | merge-group SHA | merge-group SHA | `release-gate` | required |
| non-main merge group | group SHA | group SHA | `release-gate`, then not_applicable | no stable promotion |

A check run is bound to its own SHA. GitHub attaches the PR check run to the PR
head SHA, while the plan, receipts, package descriptors and Beta evidence bind to
the merge SHA that the run checked out. Passing a SHA as a parameter does not move
a check onto that SHA. Observed on Beta.2 run `43012493350`: the required
`release-gate` check run has `head_sha` `62bd129a7` (PR head), while the plan and
package descriptors bind `decb9a0f9` (PR merge SHA).

The base-update path was probed without running a release matrix: a temporary
docs-only PR (#116) based one commit behind `develop` reported `mergeStateStatus:
BEHIND` under the strict policy, despite an otherwise mergeable PR. A base update
therefore blocks the stale candidate; update the candidate branch to create a new
head, then start a new candidate run. `draft→ready` alone does not pull the new
base into the candidate. If the new base is already reachable from the head, there
is no new base content; the head-bound check still covers the same head tree.

## Offline re-judgment

`scripts/ci/rejudge.py` re-runs only the Beta decision rules over an existing
run's raw results. It does not execute geometry, properties or package builds.

The downloaded evidence must be laid out as:

```
audit/
  selection/plan.json
  evidence/**/receipt.json
  matrix/results-1.json ... results-16.json
  properties/target/nextest/ci/junit.xml
  jobs.json
```

**Execution identity** is the source revision and tree, the plan, the executed
tests and receipts, the runner identity, the raw matrix/property/jobs files, and
the `beta_gate.py` version. **Decision rules** are the exception and baseline
files; changing those is the supported rules-only case. A change to
`beta_gate.py` is rejected rather than guessed at; this round does not support
changing decision code over old results. `execution.json` is written by the
Beta job. If it is missing or failed to generate, there is no re-judgment evidence
for that run; the formal Beta decision is unaffected and this is not a substitute
for a rerun.

The digests prove that the files match `execution.json`. They are integrity
checks, not signatures or proof of who produced the evidence.

Record the execution identity once, then check or re-judge. Run these commands from
a checkout of the execution's source SHA, so the recorded `beta_gate.py` hash and
the decision code are the ones that produced the run. The release workflow writes
`execution.json` itself; the manual form below is for an already-downloaded run.

```sh
python3 -I scripts/ci/rejudge.py record --root audit \
  --exceptions tests/action_matrix/beta-exceptions.json \
  --baseline tests/action_matrix/baseline.json \
  --source-sha "$SHA" --run-id "$RUN_ID" --run-attempt "$RUN_ATTEMPT" \
  --repository "$REPOSITORY" --workflow "Release gate" --event pull_request \
  --tree-sha "$TREE_SHA" --out audit/execution.json

python3 -I scripts/ci/rejudge.py check --root audit \
  --execution audit/execution.json --candidate-sha "$SHA"

python3 -I scripts/ci/rejudge.py evaluate --root audit \
  --execution audit/execution.json --candidate-sha "$SHA" \
  --exceptions tests/action_matrix/beta-exceptions.json \
  --baseline tests/action_matrix/baseline.json --out audit/rejudge.json
```

`check` requires every recorded input digest to match, the same `beta_gate.py`
hash, the same plan source revision and (when supplied) the same candidate SHA
and tree. A changed exception or baseline changes the rules digest and is
allowed; a changed plan, matrix, property report, job list, receipt, gate script
or source revision is refused. Runner identity is recorded for audit; a re-judgment
does not claim the rules ran on that runner. `evaluate` prints `formalGate: false`; it is a
diagnostic report, not a substitute for the required `release-gate`. Missing
shards, timeouts, cancellations and incomplete results still fail through
`beta_gate.py`.

## What still needs a full run

A changed test, production code, toolchain, runner image or execution semantics
invalidates the raw execution evidence. Rebuild and rerun the affected release
jobs at the new SHA. Reuse is limited to a rules-only change over unchanged
execution inputs; there is no general cross-SHA test cache.
