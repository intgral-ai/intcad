# DFM rule matrix: catalogue, inputs and current gaps

Status: working matrix for the full-catalogue acceptance. Base: `origin/develop`
`f10952b77`. The worker/report columns depend on the DFM run/report PRs (#126, #135,
#137, #141), which are open at this writing; `origin/develop` alone has the pure rule
crate, the `dfm.sheet` check kind and the hole facts.

## 1. Catalogue

The rule catalogue is `crates/dfm-rules/src/lib.rs::RULE_ORDER`: four gates and 19
migrated non-gate rules, plus the App-owned `app.flat_no_overlap`. That is 20 rules
after the gates, as expected.

| # | Rule | Kind | App input needed | Where it comes from today | Gap at `origin/develop` |
|---|---|---|---|---|---|
| G1 | `sm.gate_input_solid` | gate | one closed solid, one sheet body | App entry (`pass_input_gate`) | none for native bodies; converted-body closed-solid gate is not implemented |
| G2 | `sm.gate_sheet_thickness` | gate | declared and modelled thickness | `Declared` + `SheetBody.t` | none |
| G3 | `sm.gate_bends_found` | gate | bend count and recognition completeness | `SheetFacts.bends` | none |
| G4 | `sm.gate_profile_coverage` | gate | which selected rows apply | `Profile` + `Declared` | none; project selection lives on #135 |
| 1 | `sm.inside_radius` | min | each bend's inside radius | `SheetFacts.bends[].inside_radius_mm` | none |
| 2 | `sm.min_flange` | min | for each bend, both flats' effective length at 20 places along the bend, and the longest run per side | not measured | **BendRuns** |
| 3 | `sm.hole_min_diameter` | min | round through-hole diameter | `SheetFacts.holes[]` | none |
| 4 | `sm.max_bend_length` | max | bend-line length | `SheetFacts.bends[].length_mm` | none |
| 5 | `sm.hole_to_bend` | min | hole wall to each adjacent bend tangent | `SheetFacts.holes[].bend_gaps` | none |
| 6 | `sm.bend_relief_present` | presence | each bend end: no notch, or material beside it | not measured | **ReliefEnds** |
| 7 | `sm.hole_to_edge` | min | hole wall to nearest cut edge | `SheetFacts.holes[].edge_mm` | none |
| 8 | `sm.min_web_parallel_bends` | min | parallel-bend pairs, tangent-to-tangent web | not measured | **Webs** |
| 9 | `sm.u_channel_ratio` | max | U pair: taller flange vs web width | not measured | **Webs** |
| 10 | `sm.tolerance_across_bends` | tolerance | declared critical hole-to-hole dimensions | `CheckInput.critical` is hard-coded empty; project storage absent | **Critical declarations** |
| 11 | `sm.countersink_depth` | min | countersink depth from its rim | not measured; `HoleFact.countersink` always `None` | **Countersinks** |
| 12 | `sm.countersink_to_edge` | min | countersink rim to nearest cut edge | not measured | **Countersinks** |
| 13 | `sm.countersink_to_bend` | min | countersink rim to each bend tangent | not measured | **Countersinks** |
| 14 | `sm.hole_spacing` | min | hole wall to nearest hole wall on the same flat | `SheetFacts.holes[].nearest_hole_wall_mm` | none (rule needs `HolePanels`, already supplied) |
| 15 | `sm.bend_radius_consistent` | consistency | all bend radii | `SheetFacts.bends[].inside_radius_mm` | none |
| 16 | `sm.bend_relief_width` | min | each notch width at its bend end | not measured | **ReliefSizes** |
| 17 | `sm.bend_relief_depth` | min | each notch depth at its bend end | not measured | **ReliefSizes** |
| 18 | `sm.countersink_land` | min | flat land between countersink rim and hole wall | not measured | **Countersinks** |
| 19 | `sm.countersink_angle` | shop angle | countersink included angle | not measured | **Countersinks** |
| 20 | `app.flat_no_overlap` | App geometry | exact flat material regions, pair-wise intersection area | `sheet_dfm::flat_overlap` on #125/#137, not on `develop` | **App geometry slice (dependency)** |

The 12 rules in the gap scope are rows 2, 6, 8, 9, 10, 11–13, 16–19 across five
fact families: `BendRuns`, `ReliefEnds`, `Webs`, `ReliefSizes`, `Countersinks`, plus the
critical-dimension declaration path that is not a `FactFamily`.

## 2. What the rule layer already does

The pure rule crate is already migrated and has recorded INTdfm parity for the whole
catalogue:

- `crates/dfm-rules/tests/intdfm_baseline.rs` and `catalog_parity.rs` compare status,
  code, measured value and limit against INTdfm recordings.
- `crates/dfm-rules/tests/flange_rules.rs`, `bend_rules.rs`, `sink_rules.rs` cover the
  flange/relief/web, bend and countersink/tolerance rules over recorded facts.
- `crates/dfm-rules/tests/fixtures/catalog/v1/` holds filled `runs_mm`, `longest_mm`,
  `unrelieved`, `no_margin`, `webs`, `countersink` and `critical` examples.

This is **low-layer evidence**: it proves the rule logic, not that the App can measure
or deliver those inputs. The App adapter currently marks all five fact families as
`missing`, so every one of the 12 rules reports `RULE_NOT_IMPLEMENTED`.

## 3. Evidence levels used below

| Level | Meaning |
|---|---|
| L0 | rule-layer fixture only (hand-built facts) |
| L1 | recorded INTdfm facts replayed through the Rust rules |
| L2 | App measurement (`sheet_dfm::facts`) asserted against known geometry |
| L3 | production adapter (`engine::dfm::check_sheet`) produces the target finding |
| L4 | validation worker runs the check from a snapshot and the written report contains the finding |
| L5 | desktop UI shows the finding/label and selects the object |

Current highest level per family:

- Holes, spacing, inside radius, max bend length, bend radius consistency: L4 for the
  hole rules (PIL-302/PIL-303 runs); the other rules have not been re-exercised through
  L4 in this matrix yet.
- Flat overlap: L3/L4 adapter and report wiring (PIL-306); its own note records that no
  real worker/desktop failing sample was fabricated in that slice.
- The 12 gap rules: L0/L1 only.

## 4. Dependencies and overlap

- Open PRs that carry the worker/report, project settings and chat/viewport layers:
  #126 (project folder), #135 (data/settings), #137 (run/report), #138 (chat card),
  #141 (list ↔ viewport), plus #125 (flat overlap).
- No open PR or branch implements the five missing fact families or the critical
  declaration path. `PIL-292` ("扩展DFM checks") is Todo and unassigned in practice.
- The 12 rules can be measured and tested on `origin/develop`; their worker/desktop
  acceptance must be repeated on an integration branch after #126/#135/#137 merge, and
  the exact integration SHA recorded.

## 5. First samples

The first production chain to close is `sm.min_flange` on a saved native L-bracket:
one 90° bend on a rectangular base with a known flange leg. It exercises the new
BendRuns measurement, the adapter and the report without needing countersinks or
project declarations. The same model family (varying one leg length) then drives the
relief and web rules.
