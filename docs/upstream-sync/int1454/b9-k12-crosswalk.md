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
| Q:10 | `crates/ui-egui/src/render_workspace_tests.rs:13253–13282`: failed control reply, retained renderer credit, refusal of another Check before reset, busy state and retained credit after reset, and zero credit at teardown. | Candidate `render_workspace_tests.rs:1088–1115` covers setup failure or disconnection, not retirement. Engine `cmd/render/job_tests.rs:3041–3104` covers retention until successful worker return, not disconnected app ownership. |
| Q:11 | `crates/ui-egui/tests/scenarios/universal_panel_host_shared_actions.json:3–17`, generated by `crates/ui-egui/build.rs:96–123`: drives the actual shared move menu through Floating, Right, Bottom and Left; requests fixture disable and checks for errors. It does not independently assert each resulting placement or disabled state. | Candidate `panel_runtime/tests.rs:780–848` covers direct runtime movement and mounting, not the workspace menu sequence. Explicit placement and disabled-state checks would be requirements for a proposed replacement, not claims about the upstream assertions. |
| Q:12 | `crates/ui-egui/src/shell_tests.rs:1055–1091`: runs four shell scenarios; checks one Apply dispatch without changing the observed session, unchanged document identity, zero undo entries, and a visible Browser card with the production runtime and host still enabled. It does not assert production host shutdown. | Candidate `panel_runtime/tests.rs:611–647,867–898` and `control.rs:2175–2213` cover overlapping action and state invariants, not the shell scenarios or the transition from the specimen back to the production host. |

The owner accepted the recommended dispositions on 2026-11-23:

- **Q:10 — not applicable on `develop`.** The upstream case exercises native render-preparation retirement, which IntCAD does not have: it can only be tested by porting that feature (see feasibility below). The owner recorded it as not applicable rather than porting the feature in this sync. Recheck on the final integration SHA; if that SHA gains `render_preparation`, this disposition no longer holds and the case returns to required evidence.
- **Q:11 — deferred this cycle.** Exclude the upstream workspace menu fixture under U11/CD-6. This is not a pass or a waiver for unrelated workspace behavior.
- **Q:12 — deferred this cycle.** Separately exclude the upstream shell scenario fixture under U11/CD-6. This is not a pass or a waiver for unrelated shell behavior.

### Q:10 feasibility on `develop`

Checked before writing the required test: the behaviour does not exist on `develop` at `9c24061c`.

- `crates/engine/src/render_preparation.rs` and `crates/engine/src/render_preparation/batch.rs` exist at `db0b8dd35` and are absent at `9c24061c`. `render_preparation` is declared only in the historical candidate's `crates/engine/src/lib.rs:38`; its `Kind`, `Prepared`, `prepare`, `finish`, and `finish_retained` API is unavailable on `develop`.
- `PathHost` on `develop` (`crates/ui-egui/src/render_workspace.rs:186–193`) is only `NotStarted`, `Preparing`, `Ready` or `Failed`. It has no admission credit, retirement channel, or `prepare::inspect`/`busy`/`begin_ui` accessors.

Q:10 therefore cannot be closed by adding a test. The native render-preparation retirement path would have to be ported first, which is new user-visible scope across the engine and UI, not test coverage. The owner chose to record Q:10 as not applicable on `develop` and not to port that feature in this sync. This is a scope decision, not evidence that the upstream case passes.

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

Totals for the mapped cohort on `9c24061c`: **3 pass, 12 fail, 2 missing, 0 skipped**. All 17 still lack an exact db0 result and final integration result. Changed kernel-case expectations require separate before/after evidence; no allowance or baseline update is authorized here.

## Failure comparison: c5 and c14 versus post-M16 `develop`

Compared the exact JUnit IDs K:11–22 using the receipts above (artifact IDs `14529416299`, `14664597624`, and `14802697491`, respectively). All ZIP and JUnit digests matched. c5 has no embedded native receipt; its run metadata binds the PR head, and the recorded test-merge and head trees match. c14 and `9c24061c` have embedded source receipts. The first panic payload, excluding the panic header and backtrace hint, matched between c5 and c14 for all 12 IDs.

“Same” means the same observed rejection or failing assertion, not a proven underlying kernel cause. “Different” means a different failed case set, input, or execution stage, not proof of a new regression. “Insufficient” means the operation lines up but one receipt lacks the detail needed to compare causes.

| ID | Classification | JUnit evidence |
| --- | --- | --- |
| K:11 | Different | c5/c14: 3 of 168 designs failed to load; `develop`: 2 of 159. Only `83-boundary-loop-thicken` overlaps. The other historical failures are `fuzz_seed_185_pattern` and `fuzz_seed_73_boolean_join`; `develop` reports `77-fillet-variable-linear`. The changed corpus does not establish fixes or regressions. |
| K:12 | Same | The identical variable `solid.fillet` setup with two intermediate radii is rejected as unsupported before the control-insertion assertion. |
| K:13 | Same | The same initial body-count assertion sees 2 instead of 1, before changing the hole diameter to 100 mm. |
| K:14 | Different | c5/c14 reject a variable fillet with midpoint `waist` as unsupported. `develop` fails on the variable fillet with end radii 2 and 5 and no midpoint, with status 1001. |
| K:15 | Insufficient | Same taper operation (`radius:16`) and status 1001. c5/c14 specify `Fillet Corner Error: Unable to Make Topology`; `develop` has only the generic status. |
| K:16 | Insufficient | Same ordered edge pair fails. c5/c14 report that the fillet exceeds the space allowed by adjacent faces; `develop` reports only status 1001. |
| K:17 | Same | Same edge `(-5.000, 87.200, 12.500)` is rejected as smooth/tangential. |
| K:18 | Same | Same variable-fillet input is rejected with the same unsupported-exact-kernel message. |
| K:19 | Same | Same box-containment assertion fails at setbacks 11–15, but the excursions differ (e.g., setback 11: `5.8754e-4` vs `7.0776e-4`). Not geometric parity. |
| K:20 | Different | c5/c14 fail building the G2 fill because curvature error exceeds `1e-3`. `develop` reaches a later assertion on the native SMLib corner (`2.074735095770037` over 538 stations). |
| K:21 | Same | Same AABB tolerance assertion (`1e-6`) fails. Bounds and measured topology differ: c5/c14 log 27 edges/19 vertices; `develop` logs 30/22. Not geometric parity. |
| K:22 | Same | Same variable-round operation returns `Unsupported` before the balls-union comparison. |

**7 same observed failures, 3 different, 2 insufficient.** No test was rerun. The separately approved Q:10–12 dispositions above and final K12 acceptance do not follow from this comparison.

## Kernel area parity baseline

The K12-relevant kernel areas were run case by case on both builds with the same CLI invocation the
rig uses (`solidcraft-cli kernel-cases <case> --json`, one process per case), so a changed kernel
expectation can later be compared against a recorded baseline.

| Area | Cases | `9c24061c` | `db0b8dd35` |
| --- | --- | --- | --- |
| frames | 6 | 6 pass | 6 pass |
| solids | 11 | 11 pass | 11 pass |
| continuity | 13 | 13 pass | 13 pass |
| crashes | 12 | 12 pass | 12 pass |
| tessellation | 14 | 14 pass | 14 pass |
| inspect | 25 | 25 pass | 25 pass |
| plastic | 18 | 18 pass | 18 pass |
| history | 45 | 45 pass | 45 pass |
| shell | 37 | 36 pass, 1 fail | 37 pass |
| threads | 31 | 31 pass | 31 pass |
| **Total** | **212** | **211 pass, 1 fail** | **212 pass** |

**Ladder (12) is excluded:** those cases are graded against the ladder's recorded golden by
`cargo xtask kernel-cases --ladder`, and all 12 report `fail` when replayed standalone on both
builds. That is the harness, not a kernel difference.

**One case differs:** `shell/cov-shell-again-open-at-an-inner-wall-of-a-shelled-box` passes on
`db0b8dd35` and fails on `9c24061c` with `step 9 (solid.shell): SMLib shell: operation: SMLib
failed (status 1001)`. The case directory is byte-identical at both SHAs (tree `c6cf2004…`), so the
difference comes from the kernel below them, not from the case or from IntCAD product code:
`db0b8dd35` builds the vendored `solidcore/` tree `8b5c0144…`, while `9c24061c` pins the git revision
`8ddce901…`. Treat it as a kernel-source difference to recheck on the landed SHA, not as a
user-visible IntCAD regression or fix.

Comparison over the 163-case second batch: **162 unchanged, 1 changed**. CLIs `d098a154…` and
`b3a856e2…`; both worktrees clean at the stated SHAs.

Receipts (output of the evaluator's `kernel-collect.py`, retained locally): `areas-9c24061c.json`
SHA-256 `485961e9…`, `areas-db0.json` `531d9050…`, `areas-compare.json` `91bfefd3…`; second batch
`rest-9c24061c.json`, `rest-db0.json`, `rest-compare.json` alongside them.

**Not covered:** `fillets` (271), `booleans` (127), `sweeps` (97), `assemblies` (96),
`sheet-metal` (78), `surfaces` (63), `construction` (53) and `import` (35) — 820 cases — were not
run. This baseline therefore covers roughly a fifth of the rig and does not stand in for those
areas.

This is a parity baseline for unchanged areas, not the required evidence for changed expectations.
Those still need, on the landed SHA, the same cases with their inputs and the base-versus-landed
comparison, alongside the 17 mapped ID outcomes and the separately approved Q:10–12 dispositions.
