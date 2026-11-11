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

## Beta exception rules

`tests/action_matrix/beta-exceptions.json` binds each accepted failure to:

- its **design / entity / action** identity;
- its `kind` (`error`, `bad_params`, `timeline_error`, `load`, …);
- a stable failure class, derived from the error text (for example
  `shell_backend_1001`, `plane_source_mismatch`, `boolean_multilump_void`);
- an issue (`#55`, `#83`, `#88` at this revision);
- the evidence run and source SHA that produced the whole 16-shard matrix.

The reviewed run also has 1,085 failures already recorded as failing in the
historical baseline and 972 failures on actions the historical baseline never
recorded. Neither group is counted as a pass. The unbaselined identities are
reported in `beta-gate.json`; they were never baseline passes, so a change in
that group is not a new regression. A baseline pass that starts failing is the
gate condition and must be in the reviewed list.

The Beta gate fails when any of these changes:

- a shard is missing, duplicated, empty of a parseable result, or the total action
  count differs from the reviewed run;
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
