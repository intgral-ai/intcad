# INT-1454 — exact commands for the experiments

All paths are the study worktrees under `/home/hyjing/Code/intgral/intcad/worktrees/`.
`intcad-e2e` is `/home/hyjing/Code/intgral/intcad/.dev/e2e/bin/intcad-e2e`.
Global E2E options go before the subcommand; the task name is stable.

## Baseline

```sh
git fetch origin develop --tags
git rev-parse origin/develop v0.1.0
git rev-parse v0.41.0
git log -1 --format='%H %P %s' d1315f0e8
git log --first-parent --format='%h %ad %s' --date=iso f45d9774 -10
```

Recorded:

- origin/develop `02a786d47180ffa44a99f0508749c1c11b383c0d`
- v0.1.0 `3b9a975299ed6fc6813593e10036f8a00f6a4d8c`
- #145 base `ac560eec548746872a6502f6df3a296cd08fea6d`
- #145 merge `d1315f0e8606b4d5b91b9b75a67b302ff717dcc2` (parents `ac560eec` + `3c8efdb5`)
- #145 head `f45d977481614fad850c7f9ca67e48e6e7956e5f`
- v0.41.0 `3c8efdb51952ed9a5f3ebc722c2bad1454fe6d80`
- brand licence first `312454f899a371f6dac52348749bc4dcb94d94f0`
- pre-brand `a34756830185f82da3a0e3ddd1ea25ef21111558`

## Experiment 1 — duplicate-circle profile

Worktree `worktrees/int1454-exp1`, branch `probe/int1454-exp1`, commit `eda5fbd3e`.

```sh
cd worktrees/int1454-exp1
git show 3c93d21ca -- crates/sketch/src/profiles.rs | git apply --3way
cargo test -p solidcraft-sketch --lib --offline -j 8 a_circle_drawn_twice_is_one_hole
```

The same test was first added alone to the baseline; it failed with
`ring.hole_curves = [["c1"], ["c2"]]` (2 holes, expected 1). After the 34-line
`profiles.rs` change it passes.

E2E (host/VM):

```sh
intcad-e2e --source .../worktrees/final      --task int1454a up
intcad-e2e --source .../worktrees/final      --task int1454a run
intcad-e2e --source .../worktrees/final      --task int1454a shot exp1-ring-before

# Create the four concentric circles and extrude the ring explicitly.
intcad-e2e ... control engine.execute '{"command":"file.new","params":{"name":"dup-circle"}}'
intcad-e2e ... control engine.batch '{"commands":[
 {"command":"sketch.create","params":{"plane":"XY","name":"Dup"}},
 {"command":"sketch.circle.center","params":{"center":[0,0],"diameter":3.2}},
 {"command":"sketch.circle.center","params":{"center":[0,0],"diameter":3.2}},
 {"command":"sketch.circle.center","params":{"center":[0,0],"diameter":6.5}},
 {"command":"sketch.circle.center","params":{"center":[0,0],"diameter":8.0}},
 {"command":"sketch.finish","params":{}}]}'
intcad-e2e ... control engine.execute '{"command":"solid.extrude","params":{
 "sketch":"Dup","profiles":[{"point":[2.5,0]}],"exact_regions":true,"distance":10}}'
intcad-e2e ... control engine.execute '{"command":"inspect.measure","params":{}}'
```

- Baseline: `331.83072403475137 mm³`, 3 faces, 1 cylinder, no hole.
- Fixed: `251.40595210516116 mm³`, 4 faces, 2 cylinders, one hole.
- Expected ring: `π(3.25² − 1.6²) × 10 = 251.40595 mm³`.

The fixed build is `worktrees/int1454-exp1`; E2E build 459 s, host build 255 s.

## Experiment 2 — orbit pivot

Worktree `worktrees/int1454-exp2`, branch `probe/int1454-exp2`.

```sh
cd worktrees/int1454-exp2
git cherry-pick -n b1ebc2c2d     # one conflict in viewport.rs
git cherry-pick -n 2534c2853
cargo test -p solidcraft-ui-egui --lib --offline -j 8 \
  orbit_after_home_turns_the_model_in_place_beside_the_browser
```

The conflict was `hover_joint` + `orbit_pivot` in `ViewportState`; keeping
`hover_joint` and the new orbit comment resolves it. The test passed after the
patch (347 s build). A baseline test cannot compile because the test uses
`viewport_layout::framing_centre`, which the patch adds; the E2E before/after is
the baseline evidence.

```sh
intcad-e2e --source .../worktrees/final --task int1454a up
intcad-e2e --source .../worktrees/final --task int1454a run
# close the About dialog, Home, screenshot, drag-orbit, screenshot
intcad-e2e ... control ui.prefs '{"browser_show":"shown"}'
intcad-e2e ... control ui.click '{"x":843,"y":819}'   # nav:slot:home
intcad-e2e ... shot exp2-orbit-before-home
intcad-e2e ... drag 700 450 820 500
intcad-e2e ... shot exp2-orbit-before-rotated
```

The same sequence on the fixed build (`.../worktrees/int1454-exp2 --task int1454c`)
keeps the model centred.

## DFM model with two findings on one face

Guest paths: project `/home/intcad/e2e/proj2/dfm-e2e`, factory
`/home/intcad/e2e/critical_shop.json` (copied from
`worktrees/final/crates/engine/src/project/test_data/critical_shop.json`).

```sh
intcad-e2e ... control engine.execute '{"command":"file.new","params":{"name":"dfm-e2e"}}'
intcad-e2e ... control engine.execute '{"command":"project.create","params":{
 "name":"dfm-e2e","folder":"/home/intcad/e2e/proj2"}}'
intcad-e2e ... control engine.execute '{"command":"project.settings_set","params":{
 "process":"press brake","material":"5052-H32","thicknessMm":2}}'
intcad-e2e ... control engine.execute '{"command":"project.data_set","params":{
 "kind":"factory","path":"/home/intcad/e2e/critical_shop.json"}}'
intcad-e2e ... control engine.batch '{"commands":[
 {"command":"sheet.manage_rules","params":{"name":"Acceptance","thickness":"2 mm",
  "k_factor":"0.5","bend_radius":"1 mm","relief_width":"4 mm","relief_depth":"4 mm","active":true}},
 {"command":"sketch.create","params":{"plane":"XY","name":"Base"}},
 {"command":"sketch.rectangle.two_point","params":{"p0":[0,0],"p1":[60,40]}},
 {"command":"sketch.finish","params":{}},
 {"command":"sheet.flange","params":{"base":"Base"}},
 {"command":"solid.hole","params":{"position":[30,20,2],"diameter":2}},
 {"command":"solid.hole","params":{"position":[2,20,2],"diameter":2}}]}'
intcad-e2e ... control engine.execute '{"command":"file.save","params":{}}'
intcad-e2e ... control ui.prefs '{"right_rail":true,"right_rail_open":"review"}'
intcad-e2e ... control engine.execute '{"command":"dfm.check","params":{}}'
# click "Run the check again" so the page loads the stored report
intcad-e2e ... control ui.click '{"x":1096,"y":100}'
```

Findings: `F5` (`sm.hole_min_diameter`, hole 1), `F8` (`sm.hole_to_edge`,
hole 1; same anchor `[2,20,1]`), `F6` (`sm.hole_min_diameter`, hole 2,
anchor `[30,20,1]`). `F5`/`F8` are the same-face pair.

## Assistant check card and two-way selection

The probe branch `probe/int1454-exp2` commit `c2631046a` makes the control
channel's `send` follow the page's phrase rule: a message equal to
"check the sheet metal part" calls `agent_chat::web::run_check`, the same host
function the page's "Run DFM check" button uses. Without this probe fix, the
control `send` path calls `send` directly and no check card is created.

```sh
intcad-e2e --source .../worktrees/int1454-exp2 --task int1454c build
intcad-e2e --source .../worktrees/int1454-exp2 --task int1454c up
intcad-e2e --source .../worktrees/int1454-exp2 --task int1454c run

# open the saved project design and its stored report
intcad-e2e ... control engine.execute '{"command":"doc.open","params":{
 "path":"/home/intcad/e2e/proj2/dfm-e2e/dfm-e2e.sc"}}'
intcad-e2e ... control engine.execute '{"command":"dfm.check","params":{}}'
intcad-e2e ... control ui.click '{"x":1096,"y":100}'   # review:run

# open the Assistant and create the real check card
intcad-e2e ... control ui.agent_chat '{"open":true,"side":"right"}'
intcad-e2e ... control ui.agent_chat '{"send":"check the sheet metal part"}'
```

The Assistant WebView now renders a check card with rows. Row positions are
screen coordinates and shift when a row expands; use the current screenshot.

```sh
# Assistant row F5 -> viewport/Review F5
intcad-e2e ... click 1100 463
intcad-e2e ... shot assistant-row-f5
# close Assistant and load Review to read the same selection
intcad-e2e ... control ui.agent_chat '{"open":false}'
intcad-e2e ... control ui.prefs '{"right_rail":true,"right_rail_open":"review"}'
intcad-e2e ... shot review-after-assistant-f5

# Assistant row F8 -> viewport label F8 (row at the current y after scroll)
intcad-e2e ... click 1100 495
intcad-e2e ... shot assistant-row-f8

# viewport label F6 -> Assistant row F6
intcad-e2e ... click 180 528
intcad-e2e ... shot assistant-label-f6-row
```

Evidence:

- `docs/screenshots/int1454-v2/assistant-check-card.png` — real DFM check card.
- `assistant-row-f5.png` — F5 row selected/expanded.
- `review-after-assistant-f5.png` — Review row F5 selected after the Assistant row.
- `assistant-row-f8.png` — F8 row selected and viewport label F8 selected.
- `assistant-label-f6-row.png` — viewport label F6 clicked; Assistant row F6 selected.

Observation: the F5/F8 labels share the same anchor and stack; earlier clicks
near the F8 label hit an expanded row or the wrong label because the card
scrolled. With the current row positions, both directions were observed
correctly. The remaining risk is the 33 px stacked-label spacing; publish a
`dfm:label:<id>` handle to make this automatable.
