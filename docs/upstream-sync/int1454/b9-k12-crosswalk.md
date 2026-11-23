# B9 → K12: upstream test evidence

This crosswalk inventories retained outcomes, not test allowances or acceptance. The upstream cohort is frozen at `e151d5ea6866fb055e011379153fb1cae0566f22`: 13 IDs from `tests/KNOWN_FAILING_TESTS` (blob `b9648f2fabfc18f8b51c5d3034bcaca3c9c28670`) and 7 from `tests/QUARANTINED_TESTS` (blob `2937bac0c41d500940ff5f014cbc33d8708ac4d1`). K and Q below refer to those two files and their source line numbers.

Match `testcase@classname + "::" + testcase@name` exactly. **Missing** means no exact result in the retained evidence, not passed, failed, or skipped. **Candidate KF** is membership in the existing list at `db0b8dd350bbdffed8927845c86c5028d280cb0e`, not a result or permission to waive a failure.

| Source | Exact test ID | Candidate KF | c5 | c14 | db0 |
| --- | --- | --- | --- | --- | --- |
| K:10 | `solidcraft-cli::action_matrix::action_matrix_sample` | No | Pass | Pass | Missing |
| K:11 | `solidcraft-cli::naming_corpus::corpus_designs_name_alike_every_build_and_fail_where_a_reference_is_gone` | Yes | Fail | Fail | Missing |
| K:12 | `solidcraft-engine::cmd::edit::tests::inserting_variable_fillet_control_preserves_parameters_at_existing_positions` | Yes | Fail | Fail | Missing |
| K:13 | `solidcraft-engine::hole_face_tests::a_hole_wider_than_the_part_says_it_removes_all_of_it` | Yes | Fail | Fail | Missing |
| K:14 | `solidcraft-engine::tests::chord_and_variable_fillets` | Yes | Fail | Fail | Missing |
| K:15 | `solidcraft-engine::tests::fillet_fit_taper_closes_to_a_point` | Yes | Fail | Fail | Missing |
| K:16 | `solidcraft-kernel::blend::corner_tests::a_round_ending_in_an_earlier_round_matches_both_together` | Yes | Fail | Fail | Missing |
| K:17 | `solidcraft-kernel::blend_corner::tests::issue2422_vertical_after_separate_bottom_rounds` | Yes | Fail | Fail | Missing |
| K:18 | `solidcraft-kernel::blend_variable::tests::variable_radius_with_a_corner_through_a_middle_point` | Yes | Fail | Fail | Missing |
| K:19 | `solidcraft-kernel::corner_g2::tests::a_long_setback_keeps_the_g2_patch_inside_the_box` | Yes | Fail | Fail | Missing |
| K:20 | `solidcraft-kernel::corner_g2::tests::curvature_rounds_meet_at_a_g2_corner` | Yes | Fail | Fail | Missing |
| K:21 | `solidcraft-kernel::corner_g2::tests::the_box_corner_g2_patch_is_a_closed_solid_curvature_continuous_to_its_neighbours` | Yes | Fail | Fail | Missing |
| K:22 | `solidcraft-smlib::ops::tests::a_variable_round_with_a_corner_in_its_law_is_its_balls_union` | Yes | Fail | Fail | Missing |
| Q:7 | `solidcraft-engine::cmd::features::patterns_tests::pattern_of_512_hole_cuts_finishes_in_reasonable_time` | No | Pass | Pass | Missing |
| Q:8 | `solidcraft-engine::cmd::joints::tests::an_older_design_with_a_to_object_extrude_opens_pinned_to_its_saved_placements` | No | Pass | Pass | Missing |
| Q:9 | `solidcraft-engine::cmd::joints::tests::an_older_design_with_unplaced_components_opens_with_its_to_object_extrude_pinned` | No | Pass | Pass | Missing |
| Q:10 | `solidcraft-ui-egui::render_workspace::tests::native_preparation_disconnected_retirement_holds_unavailable_owner_across_document_reset` | No | Missing | Missing | Missing |
| Q:11 | `solidcraft-ui-egui::scenarios::universal_panel_host_shared_actions` | No | Missing | Missing | Missing |
| Q:12 | `solidcraft-ui-egui::shell::tests::universal_host_scenarios_drive_real_shared_actions_and_leave_the_fixture_disabled` | No | Missing | Missing | Missing |
| Q:13 | `solidcraft-ui-egui::viewport::appearance_tests::theme_switches_refresh_cached_viewport_colours_without_remeshing` | No | Pass | Pass | Missing |

## Receipts

- **c5:** [run 44505747549, native job 135497237989](https://github.com/intgral-ai/intcad/actions/runs/44505747549/job/135497237989), attempt 1. Test merge `2a20b4b3974d11b68388db8f72facbd44aadedc6` and PR head `c5f2f42fe6f06ed5c2c4ec1df04f7e9c6aae50ad` share tree `6f289a06dc8635a3e9306e900b5a9682312069fd`. `rust/junit.xml` SHA-256 `ebd54d6fca6657697b96c3507ea82dca6aa4e81c48f9c8d9acbf65e1b5bfe925`. Native job red.
- **c14:** [run 44789018375, native job 136471765949](https://github.com/intgral-ai/intcad/actions/runs/44789018375/job/136471765949), attempt 1. The evidence ZIP's `rust/receipt.json` pins `c14f2a1b0b23e16dcb8808fc8bfb832b5a877e4c`; `rust/junit.xml` SHA-256 `ba28f24fc02bea08569c3f0265b1cf92ac148b8583c9578a896c753328aadd45`. Native gate passed after grading existing failures; the whole run was red.
- **db0:** [run 44811790548, attempt 2](https://github.com/intgral-ai/intcad/actions/runs/44811790548/attempts/2). Native job `136590836789` lost runner communication and did not upload `evidence-rust`. The retained matrix index SHA-256 is `0e68ba74c380366cc3e449e09cf8c45289891b13744bdc33358d411faae263fe`. Its 16 shard JUnits, the determinism JUnit and the retained properties JUnit have no cohort IDs. `action_matrix_shard` and `action_matrix_determinism` are distinct from `action_matrix_sample`.

Each historical JUnit records **5 passes, 12 failures, 3 missing, and no skips** for this cohort. The three missing UI IDs have no verified exact aliases or replacement coverage. The existing db0 known-failing list (blob `1359412a109e5c4f4cbab019a45a77d494515522`) overlaps 12 upstream known-failing IDs and none of the quarantine IDs; db0 has no quarantine file.

Keep same-head execution and before/after kernel-case evidence open under [K12](ledger.md) and [acceptance](acceptance.md). The historical results do not close K12, and no assertions, lists, or gates change.

## Candidate source gaps and evidence routing

At `db0b8dd35`, 17 exact IDs are present in source. The three UI IDs below are absent, not passed, failed, or skipped. Nearby candidate tests are not verified replacements.

| ID | Frozen upstream source (`e151d5ea`) | Candidate gap |
| --- | --- | --- |
| Q:10 | `crates/ui-egui/src/render_workspace_tests.rs:13253` | Candidate setup-error tests do not check that disconnected preflight retains ownership across document reset, rejects a new request, and releases ownership at app teardown. |
| Q:11 | `crates/ui-egui/tests/scenarios/universal_panel_host_shared_actions.json`, generated into a scenario test by `crates/ui-egui/build.rs` | Candidate panel-runtime tests do not drive the shared move menu through the workspace host or check fixture disablement. |
| Q:12 | `crates/ui-egui/src/shell_tests.rs:1055` | Candidate runtime tests do not run the shell scenarios or prove shared action dispatch, session/undo preservation, and fixture lifecycle. |

Keep Q:11 and Q:12 open under U11/CD-6; do not import kit-coupled fixtures to close this table. Keep Q:10 open as a separate render-workspace evidence gap.

When authorized CI runs on the reviewed candidate, use the full-mode **Native Linux selected regression gate** (`rust` in `.github/workflows/ci.yml`) to obtain the 17 exact outcomes. The ordinary PR lane in `scripts/ci/main_merge.py` applies an affected-package partition and excludes `action_matrix_sample` (K:10). Use SHA-bound `evidence-rust/rust/{selection.json,junit.xml,commands.json,receipt.json}`, not the job conclusion; preserve raw failures even if the existing grader accepts them. This route is not a new dispatch or a waiver. Separate case-level before/after Kernel rig evidence remains required for changed K12 expectations.

## Independent evidence check: 17 mapped IDs

The source anchors are on historical candidate `db0b8dd35`. The observed results below are from the existing post-M16 `develop` [run 45128352679](https://github.com/intgral-ai/intcad/actions/runs/45128352679), artifact `14802697491`, **not** the migration candidate or final integration SHA. The verified ZIP SHA-256 is `3f39dc38ae4800c4be9583f04e1b9c22a89ebee2c36854a52959bcf17f83d2e0`; its receipt pins `9c24061cef48327d6d04663c24ecfd3b6a8c6e81`, and its JUnit SHA-256 is `1badd4decd6fc8df3ede3fa0c67319db8f3b65b9bbff20713f96d840b9d920b2`. Raw nextest exited `100`; the existing grader exited `0`. No tests were rerun.

| ID | `db0b8dd35` source anchor | `9c24061c` exact JUnit | Evidence still required |
| --- | --- | --- | --- |
| K:10 | `apps/solidcraft-cli/tests/action_matrix.rs:1114` | Pass | Exact sample outcome on final SHA, not shard or determinism evidence. |
| K:11 | `apps/solidcraft-cli/tests/naming_corpus.rs:365` | Fail | Final exact result and failure comparison; not reference parity. |
| K:12 | `crates/engine/src/cmd/edit_tests.rs:597` | Fail | Final exact result and failure comparison for fillet-control parameter preservation. |
| K:13 | `crates/engine/src/hole_face_tests.rs:597` | Fail | Final exact result and failure comparison for full-removal diagnostic. |
| K:14 | `crates/engine/src/tests.rs:4140` | Fail | Final exact result and failure comparison for chord/variable fillets. |
| K:15 | `crates/engine/src/tests.rs:6779` | Fail | Final exact result and failure comparison for taper closure. |
| K:16 | `crates/kernel/src/curvecorner_tests.rs:216` | Fail | Final exact result and failure comparison for sequential vs joined rounds. |
| K:17 | `crates/kernel/src/blend_corner.rs:426` | Fail | Final exact result and failure comparison for vertical round after bottom rounds. |
| K:18 | `crates/kernel/src/blend_variable.rs:584` | Fail | Final exact result and failure comparison for variable radius through a middle point. |
| K:19 | `crates/kernel/src/corner_g2_tests.rs:252` | Fail | Final exact result and failure comparison for long-setback containment. |
| K:20 | `crates/kernel/src/corner_g2_tests.rs:360` | Fail | Final exact result and failure comparison for rounds meeting at a G2 corner. |
| K:21 | `crates/kernel/src/corner_g2_tests.rs:81` | Fail | Final exact result and failure comparison for closed G2 patch and neighbour continuity. |
| K:22 | `crates/smlib/src/ops_tests.rs:324` | Fail | Final exact result and failure comparison against the balls-union assertion. |
| Q:7 | `crates/engine/src/cmd/patterns_tests.rs:791` | Pass | Final exact pattern performance result. |
| Q:8 | `crates/engine/src/cmd/joints_tests.rs:1054` | Missing | Exact test absent from landed source and receipt; final saved-placement migration result. |
| Q:9 | `crates/engine/src/cmd/joints_tests.rs:1080` | Missing | Exact test absent from landed source and receipt; final unplaced-component migration result. |
| Q:13 | `crates/ui-egui/src/viewport_appearance_tests.rs:342` | Pass | Final exact viewport cache/result, not full desktop acceptance. |

Totals for the mapped cohort on `9c24061c`: **3 pass, 12 fail, 2 missing, 0 skipped**. The twelve failures agree with the c5/c14 **ID and outcome** comparison, not necessarily in failure mechanism. All 17 still lack an exact db0 result and final integration result. Changed kernel-case expectations require separate before/after evidence; no allowance or baseline update is authorized here.
