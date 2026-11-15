# DFM full-catalogue acceptance report

Status: integration evidence for the twelve missing sheet-metal DFM rules and the full
20-rule catalogue. Integration evidence head: `dfm/catalog-accept` `4c5e920f9`; later commits touch only this
report. Draft PR #151 is stacked on #147/#148/#149, base `dfm/integration-base` `f5a7f9f26`.
Every result below was
produced by the production path from a saved design through `dfm.check`, the validation
worker and the report written under `.intcad/checks/runs/<run>/`, then read back through
`inspect.dfm` (L4), unless the row says otherwise. Rule thresholds were never relaxed for
a test.

## 1. Catalogue count

`crates/dfm-rules/src/lib.rs::RULE_ORDER` has 23 entries: 4 gates and 19 migrated
non-gate rules. The App adds `app.flat_no_overlap`, so the catalogue has **20 non-gate
rules**. This matches the expected baseline; there is no extra or missing registered
rule.

## 2. The twelve former gaps

All twelve now receive real product inputs. The production adapter no longer lists any
of the five fact families (BendRuns, ReliefEnds, ReliefSizes, Webs, Countersinks) as
missing.

| # | Rule | Input now measured from | Evidence |
|---|---|---|---|
| 1 | `sm.min_flange` | each bend's two 20-place flange runs, longest run, crossed-cut flag | L4: 7 mm leg vs 8 mm fails; 11 mm passes |
| 2 | `sm.bend_relief_present` | bend-end material across the line / no margin | L4: corner with no margin fails `RELIEF_NO_MARGIN`; a relieved corner passes |
| 3 | `sm.bend_relief_width` | notch width at the bend end | L3: real `SheetBody` with a 3 mm notch measures 3; L4 row `checked` for a corner relief (see limit) |
| 4 | `sm.bend_relief_depth` | notch depth at the bend end | L3: real `SheetBody` with a 5 mm notch measures 5; L4 row `checked` (see limit) |
| 5 | `sm.min_web_parallel_bends` | tangent-to-tangent web between parallel bends | L4: 54 mm vs 5 mm passes; 2 mm web fails |
| 6 | `sm.u_channel_ratio` | taller flange / shared web | L4: 7 mm vs 10.8 mm passes; base 20 mm makes 7/14 fail |
| 7 | `sm.countersink_depth` | native countersink record: rim, depth, angle, land | L4: 1.0 mm vs 0.75 fails; Ø4.5 rim gives 0.25 and passes |
| 8 | `sm.countersink_to_edge` | rim to nearest cut edge | L4: 17 mm passes |
| 9 | `sm.countersink_to_bend` | rim to each bend tangent | L4: no bend in the sample -> `checked` / `not_applicable` |
| 10 | `sm.countersink_land` | flat land between rim and through hole | L4: 1.0 mm vs 0.5 passes |
| 11 | `sm.countersink_angle` | included angle against the shop's held angles | L4: 90° passes; a shop holding only 90° would fail 120° |
| 12 | `sm.tolerance_across_bends` | project `critical.toml` declarations, frozen into the run | L4: 0.1 mm vs 0.2 fails; 0.3 passes; changing it makes the old report `staleConfig` |

## 3. Full 20-rule matrix

| # | Rule | User opens / changes | Expected and observed at L4 |
|---|---|---|---|
| 1 | `sm.inside_radius` | L-bracket, R1 bend | passed, measured 1.0 vs shop 0.5 |
| 2 | `sm.min_flange` | flange height 10 -> 14 mm | failed 7 vs 8, then passed (leg 11) |
| 3 | `sm.hole_min_diameter` | Ø3 hole -> Ø2 | failed, measured 2.0 vs 3.0 |
| 4 | `sm.max_bend_length` | 60 mm bend line vs 500 mm | passed, measured 60 |
| 5 | `sm.hole_to_bend` | hole on the flange | passed |
| 6 | `sm.bend_relief_present` | two flanges at a corner, zero relief | failed `RELIEF_NO_MARGIN`; `relief_small` passes |
| 7 | `sm.hole_to_edge` | move a hole to 1 mm from the outline | failed, measured 1.0 vs 3.0 |
| 8 | `sm.min_web_parallel_bends` | U-channel base 60 -> 8 mm | passed 54 vs 5, then failed at 2 |
| 9 | `sm.u_channel_ratio` | U-channel base 60 -> 20 mm | passed 7 vs 10.8, then failed 7 vs 2.8 |
| 10 | `sm.tolerance_across_bends` | declared ±0.1 -> ±0.3 | failed `TOLERANCE_TIGHTER_THAN_SHOP`, then passed |
| 11 | `sm.countersink_depth` | Ø6 -> Ø4.5 rim | failed 1.0 vs 0.75, then passed 0.25 |
| 12 | `sm.countersink_to_edge` | countersink on a 60 x 40 sheet | passed, measured 17.0 |
| 13 | `sm.countersink_to_bend` | countersink on a flat sheet | checked / not applicable |
| 14 | `sm.hole_spacing` | two Ø3 holes 4 -> 10 mm apart | failed wall 1 vs 4, then passed wall 7 |
| 15 | `sm.bend_radius_consistent` | second flange R3 -> R1 | failed `BEND_RADII_MIXED`, then passed |
| 16 | `sm.bend_relief_width` | rectangular notch width 1 vs 2 | L3 measured 1.0; L4 row `checked` on a corner relief (limit) |
| 17 | `sm.bend_relief_depth` | rectangular notch depth 0.5 vs 1 | L3 measured 0.5; L4 row `checked` (limit) |
| 18 | `sm.countersink_land` | Ø6 over Ø4 leaves 1 mm | passed, measured 1.0 |
| 19 | `sm.countersink_angle` | 90° countersink vs shop 90° | passed, measured 90° |
| 20 | `app.flat_no_overlap` | flat pattern regions | passed on every sample; the failing 20 mm² case remains PIL-306's L3 real-`SheetBody` evidence |

Counts: 18 of 20 rules have an L4 decided result; `sm.bend_relief_width` and
`sm.bend_relief_depth` have L3 decided measurements plus L4 rows that stay `checked`
rather than inventing a size. `app.flat_no_overlap` has L4 rows (all passing on these
samples) and PIL-306's L3 failing evidence.

## 4. Gates and non-pass states

- **Input gate**: native sheet bodies run; no countersink or bend rule is run for a body
  with no applicable object without saying so.
- **Thickness gate**: a project declaring 3.0 mm on a 2.0 mm body gives
  `sm.gate_sheet_thickness` `failed`; the dependent numeric rule becomes `unsupported`
  and `notChecked` names it.
- **Bend gate**: a flat part gets one flat result, not a fake per-bend pass.
- **Coverage gate**: with no selected data, a numeric rule is `unsupported` with
  `NO_SHOP_NUMBER`; a team-reported override is `advisory`, never `failed`; a
  factory-confirmed row can fail.
- **Unmeasured opening**: a plate with a round hole and a 12 x 12 non-circular opening
  keeps the hole rule `unsupported` and names the opening in `notChecked`.

## 5. Reproduce

```sh
cd <worktree>
CARGO_TARGET_DIR=<target> cargo test -p solidcraft-engine --test dfm_acceptance
# one sample and one changed dimension:
CARGO_TARGET_DIR=<target> SOLIDCRAFT_DFM_SAMPLE=l_bracket \
  SOLIDCRAFT_DFM_FLANGE_LEG=14 cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
CARGO_TARGET_DIR=<target> SOLIDCRAFT_DFM_SAMPLE=u_channel \
  SOLIDCRAFT_DFM_BASE_WIDTH=20 cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
CARGO_TARGET_DIR=<target> SOLIDCRAFT_DFM_SAMPLE=spacing \
  SOLIDCRAFT_DFM_HOLE_GAP=10 cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
CARGO_TARGET_DIR=<target> SOLIDCRAFT_DFM_SAMPLE=countersink \
  SOLIDCRAFT_DFM_CS_DIAMETER=4.5 cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
CARGO_TARGET_DIR=<target> SOLIDCRAFT_DFM_SAMPLE=mixed_radii \
  SOLIDCRAFT_DFM_SECOND_RADIUS=1 cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
```

The harness and its sample table are in `docs/dfm-catalogue-acceptance.md`. The
TEST-ONLY rows are in `crates/engine/src/project/test_data/critical_shop.json`.

## 6. Boundaries

- **No desktop (L5) evidence in this run.** The Nix desktop build and a WebKit/Xvfb
  session are outside the local disk budget (the task's worktrees and targets exceeded
  the 200 GiB local budget before this integration; only our own regenerable targets
  were reclaimed). The report, card and viewport wiring are covered by the earlier
  PIL-303/PIL-304 host tests; they were not re-demonstrated here for each rule family.
- **Corner relief sizing**: a normal `sheet.flange` corner relief is an outline cut, not
  a notch hole, so the fact layer leaves width/depth `checked` for that shape. The
  measured-notch path is covered with a real `SheetBody` at L3. Reproduce with the
  `relief_small` sample (1 mm rectangular corner relief, 0.5 mm deep). Recommendation:
  teach `notch_at`/the relief scan to read the outline interval at the bend end, or keep
  a derived corner-relief record, then change only the two expectations to `failed`
  (1 vs 2 mm, 0.5 vs 1 mm). The rule needs no change.
- **Converted countersinks**: `sheet_convert` does not recognise a cone, so a countersink
  on a converted body still reads as a pocket. The countersink sample starts as a native
  sheet. Recommendation: recognise the cone in the converter and write the same
  `SheetCountersink` record; the facts and rules then work unchanged.
- **`sm.bend_radius_consistent`** is a whole-part result with no single anchor; the
  matrix records that explicitly rather than forcing a location.
- **PIL-306 flat-overlap failure** remains the accepted 20 mm² real-`SheetBody` L3
  evidence; this run did not fabricate a failing overlap through the worker.
