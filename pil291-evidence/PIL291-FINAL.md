# PIL-291 final delivery table (working draft)

Updated 2026-11-14. Do not mark tickets Done.

| Ticket | PR / head / base | Behaviour verified | Evidence | Remaining gap | Merge order |
|---|---|---|---|---|---|
| PIL-298 | #126 `f9f8fa43b` / develop | New and reopened `.intcad/` project | PR #126 comment | none known | 1 |
| PIL-299 | #124 `1de0329ec` / develop | Native hole measurement and comparison | PR #124 comment | none known | 1 (independent) |
| PIL-301 | #135 `09a19296a` / #126 | Factory data travels with the project, config lock, attribution | PR #135 comment; 26/26 checks pass on this head | none | 2 (after #126) |
| PIL-302 | #137 `dae420bc6` / #135 `09a19296a` | Manual DFM, background run, report persistence, stale report | PR #137 comment 6891414271; PR checks 43692533711 success and Kernel rig 43692533716 success on this head | none: the missing PR checks were the stacked base conflict (no merge commit, so no workflow); merging the updated base fixed it and the PR is MERGEABLE | 3 (after #135) |
| PIL-303 | #138 `38a3fc37a` / #135 `09a19296a`（+ #141），**ready** | Chinese phrase triggers exactly one run; card equals the disk report; page renders; model tool call adopted | PR #138 comment 6889852490; #146 build: run-0bf53bc6… (deepseek-flash, 47/47), cold start in panel; earlier run-4f206a3e… | none | 4 (after #135, parallel with #137) |
| PIL-304 | #131 `84c366271` / develop (+ #141) | Row -> Failed sheet-1/F5 label + Selected detail; label F12 -> same F12 row Selected; stale model hides old marks | PR #131 comment 6889852676 | none on this build (#146 fixed placement) | 5 (with #141) |
| PIL-305 | #142 `b0b312ae0` / base `pil-291/preview-base`（`pil-291/c` `dae420bc6` + develop） | 桌面（全新打开、无项目命令）：直接多步编辑 17→18 恰好一个 run `run-761e0be1…`（9 次工具调用含 `solid.box`，模型版本 5d101f71→db84f15c）；预览 16→16、页面接受 16→17（`run-fd3b8d14…`）；重切 head 上复跑关键链：异步打开→读开关→预览 18→18→接受 18→19（`run-1beae2b6…`）、拒绝 0、重复接受被拒 0；关闭自动检查后直接编辑 18→18、预览接受 18→18（模型确实变了）；stale 接受被拒且 0 次 | PR #142 正文与评论；`pil-291/acceptance-evidence` 分支（PIL291-ACCEPTANCE.md、evidence/、Hole Tier fixture）；旧 head 与已验证 head 分别有 tag `keep/pil305-pr142-2c1adf8`、`keep/pil305-verified-53fcc17` | 旧 head 2c1adf889 会回退 #110、且打开项目后自动检查不运行（已由本 head 修复）；PIL-300/#137 合并后需把 base 改回 develop | 6（#137 之后） |
| PIL-306 | #125 `c26b5db8f` / develop (+ #137) | 20 mm² known overlap → adapter → failed finding with area/regions/anchor | PR #125 comment | worker/desktop failure sample coverage note | 5 (with #137) |

## Candidate that verifies what

- Chat/viewport desktop acceptance: candidate 2 `coord/pil-291-candidate2` `920f6880b`
  (includes develop `fb2b4ff41` / #146 placement fix, #110 web entry, DFM card and the
  adopted model-run row), official wrapper rebuilt from this tree.
- Preview acceptance: PR #142 head `b0b312ae0`, based on `pil-291/preview-base`
  (`pil-291/c` `dae420bc6` merged with develop, so it has #110's web entry and #146's
  placement fix) and carrying only the #142 feature: review.rs, Ask Here, the page's review
  card and `review` action, the project check-switch fix and their tests. Its tree equals the
  desktop-verified `53fcc17b4` except `tests/kernel-cases/EXEMPT` lines from the newer base.
  Candidate 2 still has the Chat card and the placement fix but not #142.
- No single candidate contains both #142 and the merged PR chain yet; the merge order above
  is the intended composition.

## PIL-305 integration evidence (subagent)

- `worktrees/pil305`, branch `coord/pil-305-preview-integration`, head `211f4c4be`.
- panel.rs keeps `if web::active(app, ui.ctx()) { web::place(...) ; return; }`, the
  `unavailable(app, ui)` diagnostic, and the egui conversation only for wasm32/test;
  `apps/solidcraft/src/assistant_web.rs` is byte-identical to develop.
- Run counts: direct multi-step round → 1; preview only → 0; accept → 1; reject → 0;
  duplicate accept notice → 0 extra; auto-check off → 0. Expired preview ≡ stale accept
  (revision moved) → 0; no separate expiry test exists in the PR.

## Closing evidence (2026-11-15)

- Chinese phrase on the final #146 candidate: real page, pasted and sent
  `检查一下当前钣金件`; runs 14 -> 15, exactly one new `run-902bbfec9d234715-e9c4e55522e33dce`,
  card equals the disk report (see PIL291-ACCEPTANCE.md).
- Preview accept on the PIL-305 integration branch (`53fcc17b4`, wrapper built from
  `a9cf2146a`): fresh open, no project command; preview 16 -> 16, accept 16 -> 17 with exactly
  one new `run-fd3b8d14f8dd4d06-ce9b8be342eba693`, reject 16 -> 16, second accept refused.
  Page shows `Proposed changes` … `Accepted: the changes are in the design.`
  Evidence: `evidence/pil305-preview-card.png`, `evidence/pil305-after-accept.png`.
- #137's missing PR checks were a stacked-base conflict (no merge commit, so GitHub did not
  start `pull_request` workflows). Merging `09a19296a` into `pil-291/c` (`dae420bc6`) cleared
  it: the PR is MERGEABLE, PR checks 43692533711 success, Kernel rig 43692533716 on that head.
- All desktop acceptance instances are closed (ports 46690-46694 free).

## Merge progress (2026-11-15, develop)

Merged into develop (squash):

| PR | merge commit | note |
|---|---|---|
| #124 PIL-299 | `4bd72d0f3` | rebased onto develop first |
| #131 PIL-304 | `18366bbf0` | |
| #125 PIL-306 | `08dc35d9d` | rebase met a duplicate `turned_frame` test helper; removed |
| #126 PIL-298 | `2d53b380f` | deleting its branch closed #135 (see below) |
| #150 PIL-301 | `3166053ad` | replacement for #135, which GitHub closed when its base branch was deleted and refused to reopen (head force-pushed) |
| #137 PIL-302 | `7a3b8e02b` | wasm/icon/manifest gaps found by the develop CI were fixed on #138's branch |

Open at the time of writing: #138 (`2e9422e31`, base develop; three develop-CI regressions from #137 fixed on it: command placement, English/complete-catalog labels, icon manifest), #141 (`877638ee5`, stacked on #138), #142 (`b0b312ae0`, to be rebuilt on develop after #141).

Second batch (another agent's): #147 `59b90f0e5` (draft, behind develop; rebase in progress in `worktrees/reb147`), #149 `3b84ac913` (draft, behind; rebase in progress), #148 `8d1306023` (base `pil-291/c`, now merged, so it needs a new baseline), #151 `89de3a6b1` (base `dfm/integration-base`; retarget after its parents land).

Product decision (2026-11-15): the manual DFM entry becomes a standalone panel in the new UI, owned by the colleague's UI work; no "Run DFM" button is added to Chat in this round, and no phrase-list expansion. The panel wiring checklist is `DFM-WIRING.md` next to this file. Current Chat card and the exact phrases stay for regression.

## Merge progress update (later, 2026-11-15)

- #138 merged as `701d2bfe1` after fixing four develop-CI regressions (dfm.check's toolbar
  destination, four command labels in English and the five complete catalogs, the icon
  manifest, and the toolbar help entry).
- #141 could not be reopened (closed and its head force-pushed afterwards) and is replaced by
  **#158**, merged as `4eb71a7a6` (one commit: a report row and a model label choose the same
  finding).
- #142 was re-cut as a single commit on develop `4eb71a7a6`: head `9c3603028`, base develop,
  reviewer diff 27 files / 2757 insertions (preview module, Ask Here, the page review card and
  its `review` action, the adopted-design check-switch read). The earlier re-cut head
  `b0b312ae0` and the verification tag remain for comparison.
- Another agent merged #155 (right rail UI) into develop mid-flight, which made every open
  branch behind; all were rebased again.
- Second batch: #147 `ef4363e56` and #149 `b7acc419a` were rebased onto the new develop; #148
  `c1fa8b7dd` had a missing icon for `project.critical_set` (fixed). They merge after #142.
- #151 waits for #147/#149/#148.

## Final merge state (2026-11-16)

All thirteen authorised PRs are in develop; final SHA `dca081b61`.

| Ticket | PR | merge commit |
|---|---|---|
| PIL-299 | #124 | `4bd72d0f3` |
| PIL-304 | #131 | `18366bbf0` |
| PIL-306 | #125 | `08dc35d9d` |
| PIL-298 | #126 | `2d53b380f` |
| PIL-301 | #135 → #150 | `3166053ad` |
| PIL-302 | #137 | `7a3b8e02b` |
| PIL-303 | #138 | `701d2bfe1` |
| PIL-304 two-way | #141 → #158 | `4eb71a7a6` |
| PIL-305 | #142 | `782c734f6` |
| PIL-314 | #147 | `7de5cfba8` |
| PIL-312 | #149 → #165 | `da1545fdc` |
| PIL-313 | #148 | `d9ff9b20d` |
| catalogue evidence | #151 | `dca081b61` |

Develop source checks: `panel.rs` has `web::active` and no `agent_chat_popped`; `assistant_web.rs`
keeps `delayed_place`; `file_load.rs` reads the project switch; `web.rs` has the page `review`
action; `dfm_review.rs` (right-rail panel) is present.

## Final acceptance on develop `dca081b61` (wrapper `/nix/store/abhwg0gbjr8i81g46rrqy3mvd4qhs789-solidcraft-0.1.1`)

- Tray, control 46699: `run-3f84d013b3a048c8-872d0fc808774e79`; viewport shows
  `Failed · sheet-1/F10`…`F13` and `F20`.
- Second-batch rules now carry their real values on the card/report:
  `sm.bend_relief_present` failed `RELIEF_NO_MARGIN` (F10–F13), `sm.min_web_parallel_bends`
  passed 92.0, `sm.u_channel_ratio` passed 11.0, `sm.min_flange` `FLANGE_CROSSES_CUT` (not
  verified), countersink rows `not_applicable` on this part.
- The model-tool path could not be re-verified in this run: the agent's tool channel reported a
  project-less session while a second app instance (other task, control 46814) was running. The
  Chat card and preview-accept were verified on the #142 tree (`run-3180678d…`, accept added
  `run-746f6611…`). Recorded as an environment caveat, not a proven defect.
- Catalogue evidence (from #151, worker level): 20/20 rules have a worker report row; 18/20 also
  flip with changed input. Exceptions: `sm.countersink_to_bend` (worker always `checked`; adapter
  L3 decides) and `app.flat_no_overlap` (worker rows pass; the 20 mm² failure stays PIL-306 L3).
  `sm.bend_relief_width/depth` keep L3 measured-notch evidence and stay `checked` at L4 for a
  corner relief (outline cut). No desktop L5 run was made by #151 itself.
