# PIL-291 acceptance runbook and record (2026-11-14)

## Builds

- Candidate 2 integration: `coord/pil-291-candidate2`, was `c0a7566d4`, now merged with the
  PIL-303 fix `94f1d27c4` (branch `pil-303-chat-dfm`, PR #138) as `03ec672ba`. Wrapper rebuilt
  from this tree.
- The wrapper under test is the official Nix wrapper
  `/nix/store/<hash>-solidcraft-0.1.0-beta.3/bin/solidcraft`, launched as
  `DISPLAY=:0 WAYLAND_DISPLAY=wayland-1 SOLIDCRAFT_WAYLAND=0 GTK_IM_MODULE=fcitx <wrapper> --control <port>`.
- Placement workaround: Linux WebView placement is INT-1472 (another agent). When the page
  sits at the window's (0,0) instead of the Assistant rect, we click it through its X11 child
  coordinates and note the temporary condition; no GTK/X11 code is changed on this branch.

## Fixture

`worktrees/acceptance/fixtures/hole-tier/` (copied from `/tmp/opencode/tierclean/Hole Tier`).
It keeps the project's persisted DFM runs, so the pre-run state is the stored report:

- project latest run: `run-64361371f3754794-f4da1a4c7b9db411`
- `candidateDigest`/`modelVersion`: `sha256:74986e001ce9e777c25c7f71702b1a0b2f540e1c4c8f563cfa46466e7caf7bc3`
- `configDigest`: `2eda2417b83903b9dd84328e5bd3a1ec8415d7c333899d3d59a60a112d4de1e9`
- stored state: `passed`; 20 `sm.hole_min_diameter` findings at 2.5 mm requirement
- app `dfm.status.lastRun` is `null` on a fresh app until it runs or loads a report.

## Chinese shortcut entry: gap found before the fix

On the wrapper before the fix, in the real page:

1. Connected DeepSeek (`deepseek-flash`), opened the fixture, panel open.
2. Pasted `检查一下当前钣金件` into the composer, clicked send.
3. Result: `state=busy`, one model turn, **zero new run directories**, `lastRun=null`.
   The phrase went to the model, because `is_check_request` only knew `检查一下当前设计`.

Fix (PIL-303, PR #138, commit `94f1d27c4`): accept the sheet-metal wordings and cover the
negative, history-question and repeated wordings in
`agent_chat::web::tests::the_chat_phrase_runs_the_same_check_entry_as_the_button`
(`cargo test -p solidcraft-ui-egui --lib ...` → 1 passed).

## Card render defect found on the rebuilt candidate (before the counts fix)

The first rebuilt wrapper ran the phrase correctly (one run `run-e1be3949c4b044a8-3ae6d5de72f1e861`,
card/run ids and 47 findings matching the disk report 5/5 on the first rows), but the page
turned black and the host log said:

    solidcraft: assistant web view: error: TypeError: undefined is not an object (evaluating 'n.counts.failed')

`set_check_item` stored the raw `Card`, while the page reads `card.counts.failed/advisory/
checked/unsupported`. `dfm_chat::page(&card)` already adds `counts`; the item output was not
using it. Fixed in PR #138 `0d8e55547` (store the page card) with the test
`agent_chat::web::tests::the_pages_card_carries_the_counts`; merged into candidate 2 as
`c0356b6a1` and the wrapper rebuilt.

Before counts | after counts → second page error:

    TypeError: undefined is not an object (evaluating 'is[n][r]')

The finding row passed `variant="default"` to the page's Badge, whose variant table keys are
neutral/success/destructive/info/warning/colours. `is['default']` is undefined, so
`is[n][r]` throws for any finding that is neither failed nor advisory (the first `passed`
row). Fixed in PR #138 `8e2e29061`: use `neutral`, rebuild the page from the pinned Mastra
commit (`aa406eaa`) and refresh `inventory.json`. `node packages/assistant-web/tools/build.mjs
--mastra <checkout> --check` passes; merged into candidate 2 as `c79488bed`.

## After develop #146 (fb2b4ff41): placement smoke and DFM re-run

Candidate 2 `920f6880b` (merge of develop into coord/pil-291-candidate2), wrapper
`/nix/store/s35mqwy6n331zvhk1cww5xang9ri1fnc-solidcraft-0.1.0`, PID 277908, control 46690.

- Cold start with Hole Tier and the Assistant open: the page child is 515x1203 at the
  reserved panel (747,145), not (0,0). Clicking the composer at its page position puts the
  caret at the start of the placeholder. (Synthetic typing into the page stays blocked by
  the fcitx/XWayland focus in this automation session; the user validated real wrapper
  input, and the phrase matcher is unchanged.)
- Model path with deepseek-flash: 13 runs before, 14 after; new
  `run-0bf53bc664294059-6bee76b9e9c71820`. The adopted card equals the disk report
  (runId, modelVersion `sha256:69020142...`, candidate/config digests, 47/47 findings).
- Row -> geometry: clicking a failed row highlights viewport label `Failed · sheet-1/F5`;
  the row shows Selected and `Measured 3 mm / Requirement 3.5 mm · factory · TEST-ONLY`.
- Marker -> row: clicking `Failed · sheet-1/F12` highlights it in the viewport and the same
  F12 row becomes Selected in the card.
- Model change: adding Body3 makes `inspect.dfm.freshness` non-current (`unknown`); the UI
  treats the report as stale and hides the old marks, so an old row cannot locate the new
  geometry.

## Chinese phrase on the final #146 candidate (real page)

Wrapper `s35mq...` relaunched with `GTK_IM_MODULE=gtk-im-context-simple` so the automation can
type (same binary; the fcitx instance cannot take synthetic keys because fcitx reports
focus:0 for XWayland clients). Hole Tier opened, assistant panel in the reserved rect.

- Pasted `检查一下当前钣金件` into the real composer and sent with Enter.
- Runs 14 -> 15: exactly one new run `run-902bbfec9d234715-e9c4e55522e33dce`, outcome current.
- Card equals the disk report (runId, modelVersion sha256:69020142..., candidate/config
  digests; one dfm.check row).

## Preview accept in the new Assistant: page gap found

The preview review card (steps, made/changed/removed, Accept/Reject) existed only in the
egui panel; with the web view active the page covered it and the page had no review item, so
the new Assistant could not accept a preview. Added in the pil305 integration branch: the
`review` item in host.ts/App.tsx (card with Accept/Reject posting `{type:'review'}`), the
matching strings and the host action calling `review::accept/reject` in web.rs. Page
rebuilt from the pinned Mastra commit; `build.mjs --check` passes.


`tests/kernel-cases/EXEMPT` needed entries for the new commands:
`dfm.override`, `project.ci_set/data_set/settings_set` on #135 (and its children), and
`dfm.check/cancel/status`, `inspect.dfm` on #137/#142. `cargo xtask kernel-cases
--coverage` and `cargo xtask lists` pass on the branches.

## To record after the rebuilt wrapper starts (counts fix)



1. Pre-run: `dfm.status`, `ui.agent_chat` (model/deepseek-flash), project latest run above.
2. Send `检查一下当前钣金件` from the page composer; wait for `running=false`.
3. Exactly one new run directory; `ui.agent_chat` has one `kind=dfm.check` Tool item whose
   `output` card has the new run id.
4. `python3 worktrees/acceptance/compare_card.py <chat.json> worktrees/acceptance/fixtures/hole-tier`
   compares card vs the new `report.json`/`findings.json` (run id, digests, model version,
   finding count, measured/requirement/status/state/rules of the first findings).
5. Capture the page card (child window) and the host `inspect.dfm` reply.

## Model tool entry (separate)

A request that the shortcut does not catch, e.g. `帮我看看这个模型的孔位有没有问题`.
Record the backend (`deepseek-flash` via the configured API agent) and whether the model
returns a structured `dfm.check` tool call or leaks the DeepSeek DSML syntax as text. If it
leaks, record that as a separate backend finding, not a DFM defect.

## Linkage

Pick a stable hole finding from the new run. Click its card row, then the viewport marker.
Save run id, finding id, and screenshots of both directions. If the misplaced page makes a
click land elsewhere, mark only that click item as blocked by INT-1472.

Recorded on the model run `run-eb5d970332b945f7-748e9f43eb815401`:

- Row -> host selection works. Clicking the unique `checked sm.gate_sheet_thickness · Body1`
  row shows `Selected` (and its Not covered text). The report has exactly one such finding:
  `sheet-1/F1` (Body1, checked, rule `sm.gate_sheet_thickness`). Clicking a failed hole row
  also selects it and expands `Measured 3 mm / Requirement 3.5 mm · factory · TEST-ONLY`.
- Reverse viewport mark -> row and the visible geometry highlight: **blocked** on this
  desktop by the page placement (the page is at window 0,0 and the right panel is blank;
  INT-1472). Host-side, `a_report_row_and_a_viewport_label_choose_the_same_finding` passes.


## Preview accept on the PIL-305 integration candidate (desktop)

Wrapper `/nix/store/5drg9nxglyy65p7wcf54c7gk147jll47-solidcraft-0.1.0` (branch
`coord/pil-305-preview-integration`, `208d9968a`), control 46692:

- The new Assistant's page renders the preview card: `Proposed changes`, the step
  (`Box 10 × 10 × 10 mm → Cube`), `Made Cube`, and real Accept / Reject buttons.
  Screenshot: `/tmp/opencode/preview-page.png` (window 0xa0000c at 747,145, 515x1203).
- While a preview waited: run directories 15 -> 15 (no check).
- Reject (`run` count 16 -> 16): no check.
- A second Accept notification: `no previewed changes are waiting`, no check.
- Accept: 15 -> 16 and exactly one new run `run-fd0b9581bebb49f8-b930d4e32ab8195a`
  (modelVersion `sha256:c7616447b7f3c21579c5920e6c846ffed4cbbaa4b10279f6462c623de1b05f1a`,
  the model after the accepted change; summary lists the real findings).

Defect found on this candidate: after a fresh `doc.open` the automatic check did not run at
all — Accept and direct rounds both committed with zero runs. The switch is read in
`finish_command`, which the asynchronous open path never calls; `project.ci_set` was what
made the next Accept run its check. Fixed on the integration branch (`a9cf2146a`): read the
switch when the design is adopted and again where the automatic check decides, plus a
regression test (`an_accepted_preview_reads_the_projects_check_switch`). Re-run below.

Synthetic pointer input into the WebKit child window is unreliable here (an earlier Accept
click landed and a later one at the same layout did not; clicks report the parent window),
so the Accept that produced the run above went through the app's own control channel
(`ui.agent_chat {"review":"accept"}`), the same `review::accept` the page button calls.

### Re-run on the fixed build (`a9cf2146a`, wrapper `/nix/store/2ri9vrzms4r18pds3qbsz894fqavzpxc-solidcraft-0.1.0`)

Fresh app, control 46694, opened Hole Tier only (no project command):

- preview waiting: runs 16 -> 16 (no check)
- accept: 16 -> 17, exactly one new run `run-fd3b8d14f8dd4d06-ce9b8be342eba693`, outcome `current`
- the accepted edit's model version is in that run's `report.json` (differs from
  `run-fd0b9581…` because accepted geometry changes the model)
- reject on the same build: 16 -> 16; a second accept: `no previewed changes are waiting`, no run

Before the fix the same fresh-open flow gave 0 runs on accept; after the fix it gives 1.

## Direct round, switch off, and stale accept on the same fixed build

Wrapper `/nix/store/2ri9vrzms4r18pds3qbsz894fqavzpxc-solidcraft-0.1.0` (`a9cf2146a`), fresh
app, control 46695, only Hole Tier opened.

**Direct multi-step round, checks on (preview off).** Prompt `再加一个 12 mm × 12 mm × 12 mm 的方块`:
9 tool calls (`inspect_design`, `measure`, `describe_command`, `skill_reference`,
`list_commands`, `timeline.inspect` ×2, `execute: solid.box`, `measure`), the box committed,
runs 17 -> **18** with exactly one new run `run-761e0be1641e46c2-9971a4d2febcac2a`
(outcome `current`); its modelVersion `sha256:db84f15c…` differs from the one before the round
(`sha256:5d101f71…`), so the check is of the changed model.

**Switch off.** `project.ci_set {"enabled": false}` (ci.toml `enabled = false`), then:
- direct round `再加一个 14 mm × 14 mm × 14 mm 的方块`: `execute: solid.box` Completed,
  Body1 bbox z 16, runs 18 -> **18** (no check).
- preview round `再加一个 16 mm × 16 mm × 16 mm 的方块`: preview waiting 18 -> 18, accept
  18 -> **18** (no check) while the model did change (Body1 bbox z 18).
Restored `enabled = true` afterwards.

**Stale accept.** Preview 2 waiting with runs 18; then `solid.cylinder` created Body3
(revision moved) and Accept was refused: `The design changed since the preview, so these
changes can't be applied. Reject them and ask the agent again.` — runs 18 -> **18**.

## Final #142 head b0b312ae0 (re-cut), desktop key chain

Wrapper `/nix/store/q30f6g0pjid3p633ssacf46lv1ngvc56-solidcraft-0.1.0`, fresh app, control
46696, only Hole Tier opened (asynchronous open, no project command).

- Project `ci.toml` `enabled = true`; page placed in the panel (515x1203 at 747,145), so the
  INT-1472 fix is in this head.
- The page shows the review card: `Proposed changes`, `Box 20 × 20 × 20 mm → Cube20`,
  `Made Cube20`, `Accept` / `Reject` (`evidence/pil305-v2-preview-card.png`).
- Preview waiting: runs 18 -> 18.
- Accept: 18 -> **19**, exactly one new run `run-1beae2b62b3c4809-f25a45fe2df23e14`, outcome
  `current`, modelVersion `sha256:c0353c20…` (the model after the accepted edit). This is the
  chain that used to run zero checks after an asynchronous open, on a head that is the PR's own.
- Switch off (`project.ci_set {"enabled": false}`), preview round with
  `再加一个 22 mm × 22 mm × 22 mm 的方块`, accept: runs 19 -> **19** while the model did change
  (Body1 bbox max x 190 -> 217, z 20 -> 22). Switch restored to `true`.
- Instance closed.

Earlier counts on the same code (tree identical except EXEMPT): direct multi-step round 1,
preview 0, accept 1, reject 0, duplicate accept 0, stale accept 0.

## Combined candidate desktop run (first batch: develop + #138 + #141 + #142)

Wrapper `/nix/store/iaq3gzb3hh2z2x3hhhbsanq6xp1apq27-solidcraft-0.1.0`, built from the composed
tree (`worktrees/p142comp`, develop + #138 + #141 + the #142 feature), control 46697, Tray
project copied from the second batch's desktop fixture.

- A Chat request (`运行 DFM 检查并把报告告诉我`, model path) started one run:
  `run-337b54f69cf74679-a69636835dcdfe61`; the card equals the disk report (runId, model
  version, candidate and config digests, status `done / current`), 25 findings.
- Clicking the failed `sm.bend_radius_consistent` row put `Failed · sheet-1/F19` on the model and
  highlighted the hole (`evidence/combined-row-to-marker.png`); the reverse direction is covered
  by `a_report_row_and_a_viewport_label_choose_the_same_finding` and was shown on the PIL-304
  desktop run.
- With preview on, an agent request to change R1 to R2 waited as a preview (runs 5 -> 5); Accept
  committed it and added exactly one run `run-9bfd9ba204a5421d-8a1a0453127b0566`
  (`evidence/combined-after-accept.png`).
- Second-batch rule visibility is not yet possible here: `sm.bend_relief_*`, `sm.min_flange`,
  `sm.u_channel_ratio` and `sm.countersink_*` report `RULE_NOT_IMPLEMENTED` until #147/#149 merge.
