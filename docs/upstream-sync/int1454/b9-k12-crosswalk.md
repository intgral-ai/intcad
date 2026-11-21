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
