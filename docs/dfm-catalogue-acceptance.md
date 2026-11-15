# DFM catalogue acceptance harness

Status: the full-catalogue acceptance harness for the DFM slice. It runs on an integration of
PR #137's run/report layer (#148), the flange/relief/web facts (#147) and the countersink facts
(#149). It drives **production** code, not hand-built `SheetBody`s:

1. the sample part is built through the normal commands (`sketch.*`, `sheet.*`, `solid.hole`,
   `solid.extrude` for a non-circular opening);
2. the design is saved into a new project and the project selects a TEST-ONLY factory profile;
3. `dfm.check` captures the snapshot and freezes the project, then the validation worker runs;
4. the driver reads the run through `inspect.dfm`, which is the same object written to
   `.intcad/checks/runs/<run>/report.json` and `findings.json` (L4).

The harness is `crates/engine/tests/dfm_acceptance.rs`. Its fixture is
`crates/engine/src/project/test_data/critical_shop.json` (original TEST-ONLY test data, not copied
from another product). The file contains a confirmed factory row for every rule the samples
target, plus the full tolerance table the declared dimensions need.

## Run it

```sh
export PATH=... # your Rust/Cargo toolchain and a linker
CARGO_TARGET_DIR=/home/hyjing/Code/intgral/intcad/.targets/dfm-accept \
  cargo test -p solidcraft-engine --test dfm_acceptance
```

The tests run the worker and are serialized by a process-wide lock, because the DFM service keeps
one worker and one validation store. One sample:

```sh
SOLIDCRAFT_DFM_SAMPLE=l_bracket \
  cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
```

Change one dimension without editing code:

```sh
SOLIDCRAFT_DFM_SAMPLE=l_bracket SOLIDCRAFT_DFM_FLANGE_LEG=14 \
  cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
SOLIDCRAFT_DFM_SAMPLE=u_channel SOLIDCRAFT_DFM_BASE_WIDTH=20 \
  cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
SOLIDCRAFT_DFM_SAMPLE=spacing SOLIDCRAFT_DFM_HOLE_GAP=10 \
  cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
SOLIDCRAFT_DFM_SAMPLE=countersink SOLIDCRAFT_DFM_CS_DIAMETER=4.5 \
  cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
SOLIDCRAFT_DFM_SAMPLE=mixed_radii SOLIDCRAFT_DFM_SECOND_RADIUS=1 \
  cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
```

## Samples and the rule each one drives

| Sample | Geometry, built by commands | Target rules | State with the default values |
|---|---|---|---|
| `l_bracket` | 60 × 40 base, one 90° edge flange, one hole on each flat | `sm.tolerance_across_bends`, `sm.inside_radius`, `sm.max_bend_length`, `sm.hole_to_bend`, `sm.min_flange` | tolerance `failed` at 0.1 mm (0.3 passes); inside radius `passed` 1.0 vs 0.5; max bend length `passed` 60 vs 500; hole-to-bend `passed`; min flange `failed` 7 vs 8 |
| `u_channel` | 60 × 40 base, left and right edges turn up the same way | `sm.min_web_parallel_bends`, `sm.u_channel_ratio` | web `passed` 54 vs 5; U ratio `passed` 7 vs 10.8 (0.2 × 54) |
| `no_relief` | two flanges meeting at a base corner, relief width and depth 0 | `sm.bend_relief_present` | `failed` / `RELIEF_NO_MARGIN` |
| `relief_small` | the same corner with a 1 mm rectangular corner relief (0.5 mm into each bend end) | `sm.bend_relief_present`, `sm.bend_relief_width`, `sm.bend_relief_depth` | presence `passed`; width `failed` 1 vs 2; depth `failed` 0.5 vs 1 |
| `plate` | flat 60 × 40 plate, Ø2 hole and a Ø2 hole whose wall is 1 mm from the outline | `sm.hole_min_diameter`, `sm.hole_to_edge`, `sm.hole_spacing` | diameter `failed` 2 vs 3; edge `failed` 1 vs 3; spacing `passed` |
| `plate_mixed` | flat plate with a Ø5 hole and a 12 × 12 non-circular opening | `sm.hole_min_diameter` (coverage) | `unsupported`; `notChecked` names the non-circular opening |
| `spacing` | flat plate with two Ø3 holes 4 mm apart | `sm.hole_spacing` | `failed` 1 vs 4; at 10 mm apart, `passed` |
| `countersink` | native 2 mm sheet, Ø4 through hole with a Ø6 × 90° countersink | all five `sm.countersink_*` rules | depth `failed` 1.0 vs 0.75 (Ø4.5 countersink passes at 0.25); edge `passed` 17; land `passed` 1.0; angle `passed` 90°; to-bend `checked` (no bend) |
| `mixed_radii` | two edge flanges with R1 and R3 | `sm.bend_radius_consistent` | `failed` / `BEND_RADII_MIXED`; with both R1, `passed` |

`unsupported` is a real result, not a skipped test: the rule row is present, the finding keeps its
body and id, and `notChecked` names the rule as not verified. The `gates_and_generic_values`
test separately checks three "not a pass" paths: no selected data (`NO_SHOP_NUMBER`), a
team-reported override (advisory, never failed), and a declared thickness that conflicts with the
body (thickness gate failed, dependent rule unsupported).

## What the driver asserts

For every target expectation:

- the run's `outcome` is `current` and `available` is true;
- the report speaks for the whole catalogue: every non-gate `RULE_ORDER` row and
  `app.flat_no_overlap` is present, even when no object applied (the four gates are findings,
  not rows);
- `findings.runId`, `candidateDigest` and `configDigest` are present;
- the `rules` row for the target rule exists with the expected state and a finding count;
- at least one finding carries the target rule, and every such finding has a non-empty `id` and
  `body`;
- the finding this sample is about has the expected state and measured value, and a failing or
  advisory measured finding has an `anchor`, `flat` point or outline for the viewport to select
  (except the whole-part `sm.bend_radius_consistent`);
- the expected `code` is present when the rule gives one;
- `notChecked` is non-empty and names the target rule when the state is `unsupported`.

## Changing a declaration changes the report

`a_changed_declaration_makes_the_old_report_stale_and_the_next_check_decides` is the end-to-end
critical-dimension example:

1. the L-bracket declares `base to flange` with `toleranceMm = 0.1`;
2. `dfm.check` freezes the project and the run reports `sm.tolerance_across_bends` as
   `failed`/`TOLERANCE_TIGHTER_THAN_SHOP` with measured `0.1`;
3. `project.critical_set` changes the same declaration to `0.3`;
4. `inspect.dfm` for the **old** run now says `freshness: staleConfig` (the declaration is in the
   frozen configuration digest) and keeps the old run id and report;
5. checking again produces a new run whose tolerance row is `passed`, measured `0.3`.

Commands for the same change by hand (inside the app or a script):

```json
{"command":"project.critical_set","params":{"critical":[{"name":"base to flange","fromMm":[30,20,2],"toMm":[30,2,7],"toleranceMm":0.1}]}}
{"command":"dfm.check","params":{}}
{"command":"inspect.dfm","params":{}}
```

The points are model mm points near round through holes' centres. The harness reads the measured
folded centres from `sheet_dfm::facts` and uses them verbatim, which is what a person reads from
the report's hole list; it never builds a `SheetBody` by hand.

## Adding a fact family or a sample

- Sample geometry: add a match arm in `build()` and the id to `SAMPLE_IDS`.
- Expectation: add or change its `Expectation` in `expectations()`. Never relax the factory
  threshold to make a sample pass; the fixture rows are TEST-ONLY values chosen so the expected
  result is hand-checkable.
- A rule that starts measuring a number: set `finding_state`, `measured_mm` and `code` from the
  geometry, and the location assertion starts applying automatically.
- New project data: extend `src/project/test_data/critical_shop.json` or add another fixture; keep
  the profile `layer: "factory"` with `confirmation: "factory_confirmed"` on the rows the samples
  are expected to decide.

## Limits of this harness

- Every one of the five fact families is measured on this integration branch. A converted sheet
  still has no countersink facts: `sheet_convert` does not recognise a cone, so a countersink on a
  converted body reads as a pocket. The countersink sample therefore starts as a native sheet.
- `sm.bend_radius_consistent` is a whole-part result: it names the mixture of radii but has no
  single anchor to select. The harness records that as an explicit exception, not a pass.
- `sm.bend_relief_width` and `sm.bend_relief_depth` read the model corner-relief trim as the
  bend-end notch: a 1 mm corner relief is 1 mm across the bend and 0.5 mm deep at each end, so
  the `relief_small` worker report fails both rows. Free-form or converted notches that do not
  carry that model value remain adapter-level evidence, not a guessed size.
- The harness runs the engine + validation worker in a test process. It does not open the desktop
  window; the separate desktop run (L5) is recorded in `docs/dfm-acceptance-report.md`, with the
  card rows, model labels and screenshots.
- `app.flat_no_overlap` keeps PIL-306's evidence boundary: the 20 mm² failing sample is a real
  `SheetBody` through the production adapter, not a worker/desktop run. This harness asserts the
  rule row is present and passing on its samples; it does not fabricate a failing overlap through
  the worker.
- A shared full-catalogue shop profile also lives at
  `examples/dfm/test-only-full-catalogue-shop.json`; this harness uses its own copy so a checkout
  of this branch alone reproduces every result.
