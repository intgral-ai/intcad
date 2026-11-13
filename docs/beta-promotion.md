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

The direct `ci` check on the same pull request still runs the affected lane. A
Beta pull request is never a stable release candidate.

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
recorded. Neither group is counted as a pass. The 956 reviewed unbaselined identities are itemised by design/entity/action,
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

The property lane keeps its exact #56 exception and one narrow Cylinder/Sphere case.
The cylinder `at [22.5, 12.5, 2.5], axis X, r 11.25, h 5` and the sphere
`at [10, 2.5, 0], r 12.5` touch at exactly one point: the sphere's rightmost point
`(22.5, 2.5, 0)` is inside the cylinder's start cap. A union of solids that meet at a
single point is not a manifold solid, so SMLib's refusal is correct. It refuses the
union at 1x, 0.05x and offsets 0 and 1e3 with `SmVertex::Test #1 Bad Vertex Gap`
(offset 12.5 at 1x, tolerance 2e-05) and `SmFace::Test #12 Face->Vertex`; at 40x and
offset 1e5 it builds the union, whose volume is the sum of the operands, and intersect
is empty and cut is the cylinder at every transform. The test accepts only that exact
relative geometry, operand order, uniform scale and translation; a different sphere,
cylinder, error, transform or built volume still fails. The deterministic
`cyl_sphere_refusal_is_the_exact_reviewed_upstream_case` asserts the refusal, the
transform outcomes, `A+B` volume, and that the inputs are unchanged; if the kernel
starts behaving differently, that test fails and the exception must be removed.

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
descriptors are kept with the run. Before the reviewed property grader lands,
the Beta gate reads the raw JUnit itself and accepts only the exact #56 test
identity and signature; an extra failure, a changed signature or a skipped test
fails.

The historical `tests/action_matrix/baseline.json` remains report-only. The Beta
list is not a baseline refresh and never replaces the full matrix result; the
Beta evidence always states "Beta accepted exceptions", not "full matrix green".

## Candidate identity

A promotion must be built from the exact candidate SHA. A package descriptor whose
`sourceRevision` differs, an older BYOK or GUI result, or an artifact built before
the final merge is not evidence for a new candidate. The stable v0.1.0 release
continues to require the full `release-gate` without the Beta exceptions.

## Beta.3 correction evidence

The Beta.3 gate adds the itemised 956 unbaselined identities and the configurable
1800-second action timeout. Run `43381698526` at tested merge `13417bb37` produced
54,033 actions, 2,254 failures, 213 strict exceptions, 956 unbaselined failures,
and 0 unexcused/changed/fixed actions after the fourteen reviewed timeout
reclassifications and the 16 removed loft-join fillets. The property lane keeps the
exact #56 case and the exact Cylinder/Sphere case above. The determinism job has
150-minute headroom; the load-sensitive process tests use deterministic timing and
an ACP fixture with an open stdout. The candidate SHA, its gate run and the
package/install verification for this correction are recorded in the promotion PR
and the release notes. The earlier beta.2/`0.40.0-rc.2` reads were runner
contamination, not the MSI payload.
