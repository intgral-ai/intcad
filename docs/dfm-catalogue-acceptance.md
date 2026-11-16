# DFM catalogue acceptance harness

Status: working harness on `dfm/c-critical-accept`, based on `origin/pil-291/c` `dae420bc6`
(PR #137's run/report layer). It drives **production** code, not hand-built `SheetBody`s:

1. the sample part is built through the normal commands (`sketch.*`, `sheet.*`, `solid.hole`,
   `sheet.convert` where the sheet cannot carry a feature);
2. the design is saved into a new project and the project selects a TEST-ONLY factory profile;
3. `dfm.check` captures the snapshot and freezes the project, then the validation worker runs;
4. the driver reads the run through `inspect.dfm`, which is the same object written to
   `.intcad/checks/runs/<run>/report.json` and `findings.json` (L4).

The harness is `crates/engine/tests/dfm_acceptance.rs`. Its fixture is
`crates/engine/src/project/test_data/critical_shop.json` (original test data, not copied from
another product). The file is deliberately small: it contains rows only for the rules the samples
target, plus the full tolerance table the declared dimensions need.

## Run it

```sh
export PATH=... # your Rust/Cargo toolchain and a linker
CARGO_TARGET_DIR=/home/hyjing/Code/intgral/intcad/.targets/dfm-c \
  cargo test -p solidcraft-engine --test dfm_acceptance
```

Both tests run the worker and are serialized by a process-wide lock, because the DFM service keeps
one worker and one validation store. One sample:

```sh
SOLIDCRAFT_DFM_SAMPLE=l_bracket \
  cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
```

Change the shared geometry or the declared plus-or-minus without editing code:

```sh
SOLIDCRAFT_DFM_SAMPLE=u_channel SOLIDCRAFT_DFM_FLANGE_LEG=4 \
  cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
SOLIDCRAFT_DFM_SAMPLE=l_bracket SOLIDCRAFT_DFM_TOLERANCE_MM=0.3 \
  cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
```

## Samples and the rule each one drives

| Sample | Geometry, built by commands | Target rule | State today |
|---|---|---|---|
| `l_bracket` | 60 × 40 base, one 90° edge flange, one hole on each flat | `sm.tolerance_across_bends` | `failed` at 0.1 mm, `passed` at 0.3 mm; measured 0.1/0.3, code `TOLERANCE_TIGHTER_THAN_SHOP` at 0.1 |
| `u_channel` | 60 × 40 base, left and right edges turn up the same way | `sm.min_web_parallel_bends` | `unsupported` (Webs not measured) |
| `no_relief` | one full-edge flange with relief width and depth 0 | `sm.bend_relief_present` | `unsupported` (ReliefEnds not measured) |
| `plate` | flat 60 × 40 plate, Ø2 hole and a Ø2 hole whose wall is 1 mm from the outline | `sm.hole_min_diameter`, `sm.hole_to_edge` | `failed`, measured 2.0 and 1.0 |
| `countersink` | 2 mm solid plate, Ø4 countersink Ø6 × 90°, converted to sheet metal | `sm.countersink_depth` | `unsupported` (Countersinks not measured) |

`unsupported` is a real result, not a skipped test: the rule row is present, the finding keeps
its body and id, and `notChecked` names the rule as not verified. When a slice lands that fact
family, change the expectation in `expectations()` and, if the rule now measures a number, add
the location assertion that the harness already applies to the decided rows.

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
- the finding this sample is about has the expected state and measured value, and a decided
  measured finding has an `anchor`, `flat` point or outline for the viewport to select;
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

- The five missing fact families (BendRuns, ReliefEnds, ReliefSizes, Webs, Countersinks) are still
  missing on this base, so their samples assert `unsupported`. This is L4 evidence for the
  report/adapter path, not for those rules' decisions.
- The countersink sample cannot be cut into a folded sheet body on this base: sheet machining
  refuses tapered tools. It is modelled on a solid plate and converted, so the DFM facts see a
  sheet body whose countersink is a pocket. A later countersink-fact slice should replace the
  builder with the feature the facts read, not with a hand-built `SheetBody`.
- The harness runs the engine + validation worker in a test process. It does not open the desktop
  window or capture L5 evidence (the finding shown in the UI and the object selected).
- A shared full-catalogue shop profile lives outside this branch at
  `worktrees/dfm-catalog/examples/dfm/test-only-full-catalogue-shop.json`; it is not copied here,
  because an acceptance test must be reproducible from this branch alone.
