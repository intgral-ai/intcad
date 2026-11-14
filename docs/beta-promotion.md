# Beta promotion to `main`

A Beta can reach `main` while the full kernel release gate is still red for a
reviewed list of SMLib action-matrix failures. It does **not** make the stable
release gate green, does not publish, sign or tag anything, and does not waive a
build, packaging, timeout, missing-shard or evidence failure.

## Two gates, one required context

The `release-gate` workflow is the required `main` check. It chooses its
evaluation from the promotion branch:

| Branch | Gate | What it accepts |
|---|---|---|
| `release/vX.Y.Z` | stable release gate | full `ci`, kernel rig and packages must all really succeed |
| `release/vX.Y.Z-beta.N` | Beta promotion gate | every build and non-kernel job succeeds; only the itemised action-matrix exceptions are accepted |

The `main` ruleset requires the `release-gate` context (not the direct affected
`ci` context): a promotion diff contains the whole develop tree, so the direct
`ci` lane would select the full matrix again. The `release-gate` workflow still
runs the full `ci` for stable promotions and runs the Beta evaluation for a
Beta promotion; no repository setting is bypassed for an individual merge.

The direct `ci` check on the same pull request runs the affected lane during
preparation. It is not duplicated on `ready_for_review`: that transition starts
the full `release-gate`, which validates the fixed candidate. A Beta pull request
is never a stable release candidate. A commit pushed after the gate starts has no
release-gate result; mark the PR draft and ready again to verify the new SHA. See
[validation policy](validation-policy.md).

The historical baseline was recorded on the C++ SMLib backend at
`ea772a043`; it was never re-blessed after the Rust `solidcore` switch. It is
therefore **not comparable** with the current pin. The 213 entries are a ratchet
mismatch first, not proof of a fork-only kernel regression. The attribution and
the representative upstream comparisons are in `docs/kernel-attribution.md`.


## Beta package scope

A Beta promotion packages the three platforms its scope needs: Linux x86_64,
macOS universal and Windows x64. The stable `release/vX.Y.Z` path keeps every
package format and architecture (Linux aarch64, Flatpak, Arch, Snap, FreeBSD,
Windows x86/arm64 and web). A store or format outside the Beta scope, such as the
Snap store being unreachable, does not excuse a missing Beta descriptor or a
failed Linux/macOS/Windows package.

## Beta exception rules

`tests/action_matrix/beta-exceptions.json` binds each accepted failure to:

- its **design / entity / action** identity;
- its `kind` (`error`, `bad_params`, `timeline_error`, `load`, …);
- a stable failure class, derived from the error text (for example
  `shell_backend_1001`, `plane_source_mismatch`, `boolean_multilump_void`);
- an issue (`#55`, `#83`, `#88` at this revision);
- the evidence run and source SHA that produced the whole 16-shard matrix.

The reviewed run also has 1,085 failures already recorded as failing in the
historical baseline and 956 failures on actions the historical baseline never
recorded. Neither group is counted as a pass. This revision itemises
**972** reviewed unbaselined identities; the reviewed Beta.3 run recorded 956
after the documented reclassifications. They are itemised by design/entity/action,
kind and failure class in the same exception file. Fourteen reviewed timeout entries
were reclassified to their real errors after the 1800-second action limit let
them finish (nine in the earlier review, three in run 43241626115, two in run
43381698526: the two `battery_case_pleats` `F11:side:l12` variants at 902,971 ms
and 1,067,213 ms, both `error/blend_too_large`), and two new
`edge.fillet+edit` identities were reviewed from run 43241626115. In run
43241626115, 16 listed loft-join fillets passed and were removed from the list. A **new**
unbaselined failure fails the gate; a listed one that starts passing is reported
as fixed and must be removed in the change. The action count can grow by up to four
`edge.fillet+edit` actions when more base fillets complete; the gate requires at
least the stable 54,029 actions and judges the itemised identities. A baseline pass that starts failing is the
213-item gate condition above.

The property lane keeps its exact #56 exception and two narrow Cylinder/Sphere cases.
The first is a point tangency: the cylinder `at [22.5, 12.5, 2.5], axis X, r 11.25, h 5`
and the sphere `at [10, 2.5, 0], r 12.5` touch at exactly one point, so the union is not
a manifold solid and SMLib's refusal is correct. It refuses the union at 1x, 0.05x and
offsets 0 and 1e3 with `SmVertex::Test #1 Bad Vertex Gap` (offset 12.5 at 1x, tolerance
2e-05) and `SmFace::Test #12 Face->Vertex`; at 40x and offset 1e5 it builds the union,
whose volume is the sum of the operands, and intersect is empty and cut is the cylinder
at every transform.
The second is a seam class: an axis-X cylinder and a sphere build a valid union, but
SMLib refuses their intersect and cut at 1x and 0.05x with `SmFace::Test #29 Face
crosses Seam`; at offset 1e5 the same class can hit the reviewed #56 NURBS refusal.
The tests accept only that primitive pair, those two operations and that exact checker
error, in either operand order; a different primitive pair, operation or error still
fails. This is one checker limitation, not every Cylinder/Sphere failure.
The deterministic `cyl_sphere_refusal_is_the_exact_reviewed_upstream_case` and
`cyl_sphere_seam_refusal_is_the_exact_reviewed_upstream_case` assert the refusals, the
transform outcomes, the built union volume and the input invariants; if the kernel
starts behaving differently, those tests fail and the exceptions must be removed.
The randomized property lane is recorded with the Beta evidence but is not a Beta
blocker: the reviewed action matrix and the selected deterministic regressions are the
Beta kernel gate. The stable release gate keeps the property requirement. The lane is
not claimed green: other upstream SMLib refusals still appear in its JUnit and stay
visible, not accepted as passes.

The Beta gate fails when any of these changes:

- a shard is missing, duplicated, empty of a parseable result, or the total action
  count falls below the reviewed stable count;
- an action that passed in the baseline fails and is not in the list;
- a listed action fails with a different `kind` or failure class;
- a listed action starts passing (the exception must be removed in that change);
- the unbaselined failure count or identity digest changes;
- any required build, test, packaging or platform job is missing or unsuccessful.

Build failures, timeouts, cancellations, result-parse failures and unknown
platform jobs are never excused. The raw 16-shard results, the property JUnit
and raw exit (or the later grader report), the job conclusions and the package
descriptors are kept with the run. The Beta gate records the property result but
does not reject on a property failure: the reviewed action matrix and deterministic
regressions are the Beta kernel gate. The stable release gate keeps the exact #56
property requirement. A property failure remains in the evidence and the release
notes; it is not rewritten as a pass.

The historical `tests/action_matrix/baseline.json` remains report-only. The Beta
list is not a baseline refresh and never replaces the full matrix result; the
Beta evidence always states "Beta accepted exceptions", not "full matrix green".

## Re-judging an existing Beta run

A change to the exception list or baseline may be checked against the raw results
of an existing Beta run without rebuilding or re-running geometry. The command is
`python3 -I scripts/ci/rejudge.py evaluate` with the downloaded plan, evidence,
matrix, properties and job list; see [validation policy](validation-policy.md).
It refuses a changed source SHA, tree, plan, raw result, job list, receipt or
`beta_gate.py`, and it prints `formalGate: false`. A changed exception or baseline
changes the rules digest and is the supported re-judgment case; a changed
`beta_gate.py` is rejected, so this round cannot change decision code over old
results. The digests prove file/manifest consistency, not a signature or who
produced the execution. It is an auditable diagnostic: the required `release-gate`
still needs the exact candidate's full run. The Beta job records `execution.json`
(run id/attempt, source and tree, runner identity, input and rules digests) next to
`beta-gate.json`; if recording fails, there is no re-judgment evidence for that run
and the formal Beta decision is unchanged. Timeouts, cancellations, missing shards
and incomplete results remain failures.

## Candidate identity

A promotion must be built from the exact candidate SHA. A package descriptor whose
`sourceRevision` differs, an older BYOK or GUI result, or an artifact built before
the final merge is not evidence for a new candidate. The stable v0.1.0 release
continues to require the full `release-gate` without the Beta exceptions.

## Beta.3 correction evidence

The Beta.3 gate adds the itemised unbaselined identities (972 in this revision,
956 in the reviewed run) and the configurable 1800-second action timeout. Run
`43381698526` at tested merge `13417bb37` produced 54,033 actions, 2,254
failures, 213 strict exceptions, 956 unbaselined failures, and 0
unexcused/changed/fixed actions after the fourteen reviewed timeout
reclassifications and the 16 removed loft-join fillets. The property lane keeps
the exact #56 case and the exact Cylinder/Sphere cases above. The determinism job
has 150-minute headroom; the load-sensitive process tests use deterministic
timing and an ACP fixture with an open stdout. The candidate SHA, its gate run and
the package/install verification for this correction are recorded in the promotion
PR and the release notes. The earlier beta.2/`0.40.0-rc.2` reads were runner
contamination, not the MSI payload.
