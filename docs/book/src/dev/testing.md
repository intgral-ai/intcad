# Test harnesses

`cargo xtask ci` is the full local gate: fmt, `xtask assets`, `xtask licences --check`,
`xtask layers`, clippy with `-D warnings`, the tests with the `ci` profile and the wasm
build check. It does not run the action matrix, the kernel rig, the packaging matrix or
the full native suite. The authoritative rule for what to run at each stage, and when a
previous result may be reused, is [the validation policy](../../../validation-policy.md).

| Harness | Where | What it catches |
|---|---|---|
| Unit and engine tests | next to the code, `*_tests.rs` | measured results: volume, area, topology counts against derived values |
| Hostile input | `hostile_params_never_panic`, `crates/engine/src/tests.rs` | every command × every documented parameter × junk values: errors, never panics |
| Sketch fuzz | `crates/engine/src/cmd/sketch_fuzz_tests.rs` | random sketches through the solver and profile finder |
| UI scenarios | `crates/ui-egui/tests/scenarios/*.json` | the real app, headless: clicks, keys, drags, assertions |
| Layout tests | `crates/ui-egui/tests/ui_layout.rs`, `window_widths.rs` | panels and dialogs at every window width |
| Design-file corpus | `apps/solidcraft-cli/tests/corpus` | every saved file version still opens and evaluates the same |
| Kill mid-save | `apps/solidcraft-cli/tests/kill_mid_save.rs` | a save killed at any moment leaves a readable file |
| Reference tests | `cargo xtask oracle` → [docs/oracle.md](oracle.md) | parts rebuilt from `tests/oracle/*/recipe.json`, compared with analytic or FreeCAD expected values |

## Fork CI selection and evidence

Follow [fork governance](../../../fork-governance.md) for task PRs to `develop`
(squash) and checked promotion/reconciliation with merge ancestry. The initial
merge queue is disabled, and the observed repository does not offer auto-merge.
Zero mandatory independent approvals is not administrator bypass. Exact GitHub
settings and required contexts still need separate approval, activation and
read-back.

[`scripts/ci/plan.py`](../../../../scripts/ci/plan.py) emits a source-bound plan
using the existing Rust reverse-dependency selector and actual consumer inventory.
Both rename names and deleted inputs count. Unknown or incomplete diffs and shared
build/CI inputs are conservative. All PRs, including drafts and external PRs, run
their selected checks; `approved-for-ci` is not a scheduling prerequisite. Ordinary
documentation does not require native tests, but policy or shared inputs may select
additional consumers. Scheduled/full validation selects existing full consumers;
main-target promotion additionally sets `releaseRequired`.

Pull requests use a bounded deterministic lane so a standard two-vCPU runner
finishes inside 30 minutes. The counted native job tests the packages that
directly own a changed Rust file plus the `xtask` tooling smoke owner, and runs
one deterministic eighth of that selection (`--partition count:1/8`, reported in
the plan as `rustTestShard`). Only packages whose test closure fits the
30-minute cap are in that lane (`PR_TEST_PACKAGES`); the heavy UI/app/engine
closure cannot cold-codegen in time even at 1/8. The plan lists every deferred
package in `rustDeferredPackages` and names
`.github/workflows/native-baseline.yml` in `rustDeferredTo`; the aggregate prints
the deferral and never counts it as a passing test. Reverse dependents are still
built by Clippy and the wasm check and are fully tested by the nightly/full
workflow. DQ-5 in `docs/fork-governance.md` records the owner decision needed to
run the heavy suites on every PR instead. Clippy and
the wasm compile still cover the complete selected consumer scope, including
shared lockfile, toolchain, `.github/workflows/**`, `.github/actions/**` and `scripts/**`
inputs. Those inputs
select `xtask` as the bounded smoke owner; `xtask/**` and `packaging/mastra/**`
changes always select the xtask tests. The excluded tests are **not** reported as
passing: the nightly/manual `native-baseline.yml` workflow calls this workflow in
full mode and runs the complete workspace suite and the complete repository
checks (owning xtask binary tests and packaging suites). The
`Format and repository checks` job keeps the fast format, asset, licence, layer,
list, locale, mesh, skills, selector and bot checks on every PR. The browser
offline/update harness runs in its own `Web app offline use and updates` job with
a dedicated budget; it is selected for full/release validation and for the web
app, `packaging/web`, the Pages workflow and the xtask harness modules
(`xtask/src/web_offline.rs`, `xtask/src/web_offline_test.rs`, `xtask/src/main.rs`).
An unrelated `xtask/` change does not start it. Ordinary Rust/wasm changes run the
wasm build in the existing lane; they do not start the browser harness. When #5's
contract bundle/consumers are present, the `Mastra contract (Rust)` job runs the
owned `mastra_` value/codec/bundle/state/retirement conformance tests through
nextest and emits a counted receipt; it is selected for contract or shared-input
changes and full/release validation, and fails closed if the tests are absent or
fail. Process-lifecycle `mastra_peer_` coverage stays in the full native run.
When #7's runtime package is present, `Mastra runtime (Node)` installs exactly
Node 22.23.4 and npm 11.19.0, runs `npm ci` from the committed root lockfile and
the package's typecheck/test/build/verify:schema/check:deps scripts, while
`Mastra contract (TypeScript)` runs the TypeScript side of the shared fixture
bundle. Both are command-evidence jobs: their real npm argv/exit codes form the
receipt, and a present runtime can no longer use the foundation deferral.

The always-evaluated `ci` aggregate in `.github/workflows/ci.yml` checks actual
selected `needs` via `scripts/ci/main_merge.py` and `scripts/ci/gate.py`. Inspect:

1. The exact checkout SHA, immutable plan bytes/digest, selected jobs and scope.
2. Real selected job results and matching source/plan-bound receipts. Missing,
   failed, cancelled or skipped required jobs fail; nonselected jobs are not passes.
3. Complete, nonempty selected test identities and executed counts, command exits
   and real reports. An empty filter, no-tests result, skipped case, stale report
   or constant success flag cannot satisfy a selected consumer.
4. Explicit `not_applicable` reasons and `openObligations` for absent future
   consumers on a payload-free host foundation. That is **not runtime tested**.
   A requested, deleted or partial required input blocks, not a foundation exemption.

The planner keeps `artifact-policy` as an explicit host-foundation non-applicability
until #15's verifier handoff exists; it is never a silent pass. When that handoff
lands, the job must run #15's counted `distribution_` tests and its production
policy and repository host-only CLI modes, then emit a matched receipt. Known policy
metadata and packaging utility tests select the owning checks, not installed-runtime
acceptance; unknown packaging/build inputs stay conservative. The #5 contract, #7
runtime, runtime-required artifact and #14 installed consumers remain owner
handoffs. A selected job without its real adapter or receipt fails closed; do not
document future commands as runnable or invent successful stubs.

The stable `release-gate` checks full native/wasm/platform/heavy/package evidence,
all final package descriptors, the full rig/ladder handoff and required owner
consumers. Those absent consumers remain blocking; native or packaging success
cannot waive them. See [releasing](../../../releasing.md) for final-SHA rebuilds
and the publication quarantine. Host-only or policy validity is not artifact
admission.

### Raw baseline and diagnostic limits

`.github/workflows/native-baseline.yml` preserves the independent raw native and
ignored-heavy baseline. Preserve raw exits, named failures and counts separately
from the reviewed known-failing regression grader. An accepted known failure is
still raw red; unknown failures block. Do not broaden exclusions or a failure list
to label the whole suite green. Record the exact command and exit you ran before a
commit; focused checks are valid for their inputs, and a result may be reused only
while the worktree state and inputs are unchanged. Do not report an unrun check as
passed. See [the validation policy](../../../validation-policy.md).

## Dependency advisories

Scheduled and manually dispatched CI runs have a separate **Dependency advisories** job that
checks advisories, bans and sources. A pull request or merge group that changes `Cargo.lock` or
any `Cargo.toml` runs the same job for advisories only, and `ci` requires it then; other pull
requests skip it, so a new advisory does not fail an unrelated change. To run the nightly scan
locally:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --locked check --warn unmaintained advisories bans sources
```

The checked-in `deny.toml` rejects vulnerabilities and unknown registries/git sources.
Unmaintained crates, yanked versions and duplicate versions warn; licences remain covered
by `cargo xtask licences --check`. Review advisory failures and update dependencies rather
than adding broad ignores. The first scan prompted an update to quick-xml 0.41.0 for
[RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194) and
[RUSTSEC-2026-0195](https://rustsec.org/advisories/RUSTSEC-2026-0195).

Coverage excludes the version matching of local path patches: `truck-shapeops`,
`truck-stepio`, `truck-meshalgo`, `matext4cgmath` and `egui-wgpu` in `vendor/`. Their checked-in
code and patches still need comparison with their permissively licensed upstreams; a green
scan does not establish their security or incorporate an upstream fix automatically.

## UI scenarios

A scenario is a JSON list of steps run against the real app with a headless egui context:

```json
[
  {"start": "sketch.create"},
  {"click": {"plane": "XY"}},
  {"start": "sketch.rectangle.two_point"},
  {"click": {"sketch": [0, 0]}},
  {"click": {"sketch": [40, 30]}},
  {"expect": {"errors": 0}}
]
```

A new file in `crates/ui-egui/tests/scenarios/` is picked up automatically. To record a bug
before it's fixed, add a `{"pending": "why"}` step; its test is then ignored with that reason, and
the fix removes the step.

## CI profile

CI runs the tests with `--profile ci`: light optimisation, line tables, no LTO and debug
assertions off. Some truck debug assertions fail on valid geometry; the `ci` profile disables
them while building faster than `--release`. If a test fails only in a plain debug build, say
so in the PR.

Run `cargo xtask test` to build with this profile and run the test binaries concurrently,
with doc tests alongside. Arguments after `--` go to each test binary, for example
`cargo xtask test -- shell`. To run the test binaries sequentially instead, use
`cargo test --workspace --profile ci`. On a loaded machine set `SOLIDCRAFT_TEST_JOBS=N` to
bound how many test binaries and side commands run at once; each binary then gets
`RUST_TEST_THREADS` and `ACTION_MATRIX_JOBS` of `max(1, cpus / N)` so the binaries in flight
share the machine instead of each defaulting to one worker per CPU. Setting
`RUST_TEST_THREADS` or `ACTION_MATRIX_JOBS` yourself overrides that split, and leaving
`SOLIDCRAFT_TEST_JOBS` unset keeps the original all-at-once behaviour.

## Test-level scheduling for full local CI

Install [cargo-nextest](https://nexte.st/docs/installation/pre-built-binaries/), then set
`SOLIDCRAFT_TEST_RUNNER=nextest` when running `cargo xtask ci`. For example, on PowerShell:

```powershell
$env:SOLIDCRAFT_TEST_RUNNER = 'nextest'
cargo xtask ci
```

This optional backend builds the workspace tests once, then schedules individual tests using
the `local-ci` nextest profile, which inherits `ci`. The action matrix runs first, then the longest geometry
cases start before the other costly tests, so the ordinary queue does not stall behind an
all-slot reservation. It **ignores the default filter**,
so the heavy tests below still run, does not stop after a test failure, and does not retry failures.
Tests marked `#[ignore]` remain ignored, as with the ordinary full local CI.
The default concurrency is at most four tests; `NEXTEST_TEST_THREADS` overrides it. The action
matrix's child workers default to the smaller of that budget and the host CPU count;
`ACTION_MATRIX_JOBS` explicitly overrides this worker count. The local profile reserves CPU
slots for the matrix and keeps its scheduling changes separate from GitHub CI. On Windows,
the `solidcraft-gpu` and app packages, plus UI offscreen/GPU tests, use a serial group. The Wasm check and doc tests still run alongside,
and the web-offline gate still follows a successful native/Wasm/doc run.

The backend keeps Cargo and binary metadata under `target/xtask-nextest/` and reuses it for the
native run, freeing Cargo to run the side gates. Nextest prints each test's result and duration
as it finishes. `SOLIDCRAFT_TEST_RUNNER=libtest` (or leaving it unset) retains the original runner;
`cargo xtask test` and its Cargo/libtest argument forwarding are unchanged. Unknown runner names
fail explicitly.

## CI sharding and heavy tests

On GitHub, one job builds the tests into a [cargo-nextest](https://nexte.st/) archive and four
jobs run a quarter of the tests each (two halves on a pull request; `--partition slice:N/4`,
settings in `.config/nextest.toml`), the slowest tests first; doc tests run in their own job.
Pull requests skip a few heavy tests (each takes one to twelve minutes); they run on every push to
`main`, nightly, and in `cargo xtask test`:

- `solidcraft-engine`: `tests::hostile_params_never_panic`, everything in `tests::robustness`,
  `pattern_suppress_tests::enclosure_arc_extinction_keeps_a_real_sharp_inner_lip_after_resize_and_reopen`,
  `pattern_suppress_tests::enclosure_datums_follow_wall_height_and_board_depth_with_material_proofs`,
  `cmd::plastic::tests::enclosure_snap_fit_follows_resized_front_wall`, the `atomic_commands` contract tests
- `solidcraft-kernel`: `tests::boolean_identity_on_tangent_primitives`,
  `tspline::tests::hostile_tsplines_never_panic`, and the `boolean_props` property tests
  (these run only in the nightly "Property tests" job: their inputs are random, so main skips them too)
- `solidcraft-ui-egui` scenarios: `acceptance_sensor_enclosure`

The **action matrix** (`apps/solidcraft-cli/tests/action_matrix.rs`) tries every ordinary action
(sketch, offset plane, press pull, hole, draft, shell, fillet, chamfer, construction geometry, move,
mirror, pattern) on every face, edge, vertex and body of a corpus of designs and compares with a
checked-in baseline: an action that passed there and fails now fails the test. Every run checks a
fixed sample; the nightly "Action matrix" job runs it all. After a fix, record what passes now with
`cargo xtask action-matrix --bless`. The results and the causes of the failures are in
[the action matrix page](https://github.com/bherbruck/solidcraft/blob/main/docs/action-matrix.md).

To run exactly what a pull request runs: `cargo nextest run --cargo-profile ci --profile ci`;
add `--ignore-default-filter` for the full set. A new test that takes over a minute on CI can
join the list (the `default-filter` in `.config/nextest.toml`) if it guards robustness rather
than everyday behaviour.
