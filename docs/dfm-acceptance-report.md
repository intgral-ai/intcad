# DFM full-catalogue acceptance report

Status: integration evidence for the twelve missing sheet-metal DFM rules and the full
20-rule catalogue. **Integration code/evidence head: `dfm/catalog-accept` `822638e7c`**
(this includes the exact-corner-relief semantics and the input-change variants below).
Desktop evidence builds: `b106ecfa4` for the tray/sink card shots and `e65b9bd94` for the
corner-relief card; later report edits do not change those builds. Draft PR #151 is
stacked on #147/#148/#149, base `dfm/integration-base` `f5a7f9f26`.
Integration inputs: `coord/pil-291-candidate2` `fbd3b1aba` + A `59b90f0e5` + B `ab8083ae8`
(the PR is rebased to `3b84ac913`; no rule or fact logic changed) + C `8d1306023`.
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
| 3 | `sm.bend_relief_width` | notch width at the bend end | L4: 1 mm corner relief vs 2 mm fails; measured 1.0 |
| 4 | `sm.bend_relief_depth` | notch depth at the bend end | L4: 0.5 mm corner relief vs 1 mm fails; measured 0.5 |
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
| 16 | `sm.bend_relief_width` | rectangular corner relief 1 vs 2 mm | L4 failed, measured 1.0, code `RELIEF_WIDTH_BELOW_LIMIT` |
| 17 | `sm.bend_relief_depth` | rectangular corner relief 0.5 vs 1 mm | L4 failed, measured 0.5, code `RELIEF_DEPTH_BELOW_LIMIT` |
| 18 | `sm.countersink_land` | Ø6 over Ø4 leaves 1 mm | passed, measured 1.0 |
| 19 | `sm.countersink_angle` | 90° countersink vs shop 90° | passed, measured 90° |
| 20 | `app.flat_no_overlap` | flat pattern regions | passed on every sample; the failing 20 mm² case remains PIL-306's L3 real-`SheetBody` evidence |

Counts: all 20 non-gate rules have an L4 decided result. `app.flat_no_overlap` has
L4 rows (all passing on these samples) and PIL-306's L3 failing evidence; it is not
counted as a worker failure here.

## 3.1 Two coverage counts

- **20/20 rules have an actual worker result** in a written report (a row with its own
  state, not just a catalogue name).
- **18/20 rules have a worker-level proof that changing the input flips the conclusion**
  (a pass and a fail/non-pass variant through the worker). The two exceptions are
  `sm.countersink_to_bend` (worker always `checked` on the harness designs; pass/fail is
  adapter-level L3) and `app.flat_no_overlap` (worker samples all pass; the 20 mm² failure
  remains PIL-306's L3 real-`SheetBody` evidence).

| Rule | Worker result | Worker input-change proof | Evidence |
|---|---|---|---|
| `sm.inside_radius` | yes | yes | `l_bracket` passed 1.0; `SOLIDCRAFT_DFM_BEND_RADIUS=0.25` failed |
| `sm.min_flange` | yes | yes | height 10 (leg 7) failed; `SOLIDCRAFT_DFM_FLANGE_LEG=14` passed |
| `sm.bend_relief_width` | yes | yes | 1 mm corner relief failed; `SOLIDCRAFT_DFM_RELIEF_SIZE=3` passed |
| `sm.bend_relief_depth` | yes | yes | 0.5 mm corner relief failed; size 3 gives 1.5 and passes |
| `sm.min_web_parallel_bends` | yes | yes | base 60 passed; `SOLIDCRAFT_DFM_BASE_WIDTH=8` failed |
| `sm.u_channel_ratio` | yes | yes | base 60 passed; `SOLIDCRAFT_DFM_BASE_WIDTH=20` failed |
| `sm.hole_min_diameter` | yes | yes | `plate` Ø2 failed; `l_bracket` Ø3 passed |
| `sm.max_bend_length` | yes | yes | `l_bracket` 60 passed; `long_bend` 600 failed against 500 |
| `sm.hole_to_bend` | yes | yes | `l_bracket` passed; `hole_bend` 1.5 vs 2.5 failed |
| `sm.hole_to_edge` | yes | yes | `plate` 1 mm failed; `sink-report.json` hole passed |
| `sm.tolerance_across_bends` | yes | yes | declared ±0.1 failed; ±0.3 passed; old report stale between |
| `sm.countersink_depth` | yes | yes | Ø6 (1.0 mm) failed; `SOLIDCRAFT_DFM_CS_DIAMETER=4.5` (0.25) passed |
| `sm.countersink_to_edge` | yes | yes | 7 mm passed; `SOLIDCRAFT_DFM_SINK_X=5` (2 mm) failed |
| `sm.countersink_to_bend` | yes | **no worker flip** | `docs/dfm-evidence/sink-report.json` passed (20.75); the near case is adapter-level L3 |
| `sm.hole_spacing` | yes | yes | `spacing` wall 1 failed; `SOLIDCRAFT_DFM_HOLE_GAP=10` wall 7 passed |
| `sm.bend_radius_consistent` | yes | yes | R1/R3 failed; `SOLIDCRAFT_DFM_SECOND_RADIUS=1` passed |
| `sm.countersink_land` | yes | yes | Ø6 land 1.0 passed; `SOLIDCRAFT_DFM_CS_DIAMETER=7.5` land 0.25 failed |
| `sm.countersink_angle` | yes | yes | 90° passed; `SOLIDCRAFT_DFM_SINK_ANGLE=120` unsupported (shop holds 90°) |
| `sm.bend_relief_present` | yes | yes | `no_relief` failed; `relief_small` passed |
| `app.flat_no_overlap` | yes | **no worker fail** | `docs/dfm-evidence/tray-report.json` and `sink-report.json` passed; PIL-306 L3 20 mm² failure |

The `sink-report.json` and `tray-report.json` rows above are from the desktop run; the
worker harness assertions live in `crates/engine/tests/dfm_acceptance.rs` and
`docs/dfm-catalogue-acceptance.md`.

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

## 6. Desktop evidence (L5)

The official Nix package was built from this integration branch and run on a separate Xvfb
display (`:98`, X11 window system, llvmpipe), so no other agent's desktop was used and no
Cargo target was added to the project's disk budget.

- Builds: `nix build .#solidcraft` at `b106ecfa4` for the tray/sink shots
  (`/nix/store/7iz0dmmqkxhfkaraf2lv4z6rci0ada4b-solidcraft-0.1.0`) and at `e65b9bd94` for the
  corner-relief shot (`/nix/store/1pg6cinzi61q6pvihclcc0mrvas6grfl-solidcraft-0.1.0`).
- Run: `DISPLAY=:98` with `WINIT_UNIX_BACKEND=x11`, `GDK_BACKEND=x11`,
  `WEBKIT_DISABLE_COMPOSITING_MODE=1`, `WEBKIT_DISABLE_DMABUF_RENDERER=1`, a fresh
  `HOME`/XDG tree; control channel on a separate port.
- The Assistant page is a native child window, so `ui.screenshot` records it black; the
  evidence captures the X root window with ImageMagick `import`, as a person sees it.

Results:

| Family | What the desktop run showed |
|---|---|
| Holes | Tray: `sm.hole_spacing` failed with two red `! Failed · sheet-1/F20` labels; a passing spacing row beside it. |
| Bends / flanges / relief | Tray: `sm.bend_radius_consistent` failed, four `sm.bend_relief_present` ends failed (`F10`, `F11` labels), `sm.inside_radius` passed; the card listed every row. |
| Countersinks | Sink: `sm.countersink_depth` and `sm.countersink_land` failed; `sm.countersink_to_edge`, `_to_bend` and `_angle` passed; two red model labels, card rows visible. |
| Critical declaration | Tray: declared ±0.01 makes `sm.tolerance_across_bends` failed; changing the same declaration to ±0.1 and re-running makes it passed; the old report is stale in between. |
| Flat geometry | `app.flat_no_overlap` passed in the card on both designs. |
| Relief size | The 1 mm rectangular corner relief reports 1 mm across and 0.5 mm deep; the card shows two `failed` `sm.bend_relief_width` rows and two `failed` `sm.bend_relief_depth` rows, with measured values 1.0 and 0.5. |

The first desktop run found a real integration gap: an agent reaching `dfm.check` through the
generic Code Mode `execute` tool produced a plain tool row, so no card and no model marks
appeared. `adopt_model_check` now recognises the command's reply from either a direct tool or
`execute`, and a regression test covers both. The screenshots above are from the fixed build.

Card screenshots: `tray-card-4.png` (hole/bend/relief rows and model labels),
`sink-card-3.png` (countersink rows and labels), `relief-card-rows.png` (both relief-size
rules failed), `tray-check.png` (model labels only). Report JSON:
`docs/dfm-evidence/tray-report.json`, `tray-loose-report.json`, `sink-report.json`,
`relief-report.json`. The screenshots are attached to PIL-292.

## 7. Boundaries

- **Desktop scope.** L5 covers holes, bends/flanges/relief presence, countersinks, the
  critical declaration and the flat-overlap row. The build used the official Nix package
  (Nix store does not count toward the 200 GiB workspace budget) on a separate Xvfb
  display. The 1 mm rectangular corner relief reports 1 mm across and 0.5 mm deep; both
  relief-size rows fail in the card and on the model.
- **Other notch shapes.** The corner relief a `sheet.flange` command cuts is now read from
  the model bend length less the flat zone trim. Free-form or converted notches that do not
  use that model value still fall back to the L3 measured-notch tests, not a guess.
- **Converted countersinks**: `sheet_convert` does not recognise a cone, so a countersink
  on a converted body still reads as a pocket. The countersink sample starts as a native
  sheet. Recommendation: recognise the cone in the converter and write the same
  `SheetCountersink` record; the facts and rules then work unchanged.
- **`sm.bend_radius_consistent`** is a whole-part result with no single anchor; the
  matrix records that explicitly rather than forcing a location.
- **PIL-306 flat-overlap failure** remains the accepted 20 mm² real-`SheetBody` L3
  evidence; this run did not fabricate a failing overlap through the worker.
