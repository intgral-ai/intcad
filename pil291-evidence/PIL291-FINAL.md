# PIL-291 final delivery table (working draft)

Updated 2026-11-14. Do not mark tickets Done.

| Ticket | PR / head / base | Behaviour verified | Evidence | Remaining gap | Merge order |
|---|---|---|---|---|---|
| PIL-298 | #126 `f9f8fa43b` / develop | New and reopened `.intcad/` project | PR #126 comment | none known | 1 |
| PIL-299 | #124 `1de0329ec` / develop | Native hole measurement and comparison | PR #124 comment | none known | 1 (independent) |
| PIL-301 | #135 `09a19296a` / #126 | Factory data travels with the project, config lock, attribution | PR #135 comment; 26/26 checks pass on this head | none | 2 (after #126) |
| PIL-302 | #137 `dae420bc6` / #135 `09a19296a` | Manual DFM, background run, report persistence, stale report | PR #137 comment 6891414271; PR checks 43692533711 success and Kernel rig 43692533716 on this head | none: the missing PR checks were the stacked base conflict (no merge commit, so no workflow); merging the updated base fixed it and the PR is MERGEABLE | 3 (after #135) |
| PIL-303 | #138 `38a3fc37a` / #135 `09a19296a`（+ #141），**ready** | Chinese phrase triggers exactly one run; card equals the disk report; page renders; model tool call adopted | PR #138 comment 6889852490; #146 build: run-0bf53bc6… (deepseek-flash, 47/47), cold start in panel; earlier run-4f206a3e… | none | 4 (after #135, parallel with #137) |
| PIL-304 | #131 `84c366271` / develop (+ #141) | Row -> Failed sheet-1/F5 label + Selected detail; label F12 -> same F12 row Selected; stale model hides old marks | PR #131 comment 6889852676 | none on this build (#146 fixed placement) | 5 (with #141) |
| PIL-305 | #142 `2c1adf8` 堆叠；集成 `53fcc17b4`（develop `fb2b4ff41` + #142 + 页面 ReviewCard + 自动检查开关修复） | 桌面（全新打开、无项目命令）：直接多步编辑 17→18 恰好一个 run `run-761e0be1…`（9 次工具调用含 `solid.box`，模型版本 5d101f71→db84f15c）；预览 16→16、接受 16→17（`run-fd3b8d14…`）、拒绝 0、重复接受被拒 0；关闭自动检查后直接编辑 18→18、预览接受 18→18（模型确实变了）；stale 接受被拒且 0 次 | `worktrees/acceptance/PIL291-ACCEPTANCE.md`、`evidence/` 截图；PR #142 评论 6891385269 | #142 原样合并仍会回退 #110 | 6（#137 之后） |
| PIL-306 | #125 `c26b5db8f` / develop (+ #137) | 20 mm² known overlap → adapter → failed finding with area/regions/anchor | PR #125 comment | worker/desktop failure sample coverage note | 5 (with #137) |

## Candidate that verifies what

- Chat/viewport desktop acceptance: candidate 2 `coord/pil-291-candidate2` `920f6880b`
  (includes develop `fb2b4ff41` / #146 placement fix, #110 web entry, DFM card and the
  adopted model-run row), official wrapper rebuilt from this tree.
- Preview acceptance: `coord/pil-305-preview-integration` `53fcc17b4` (develop
  `fb2b4ff41` + #142 with #110's panel/tests preserved, #146's assistant_web.rs, the page's
  review card and the project check-switch fix). This branch contains both the web entry,
  the #146 placement fix and the preview changes; candidate 2 has the first two and the Chat
  card but not #142.
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
