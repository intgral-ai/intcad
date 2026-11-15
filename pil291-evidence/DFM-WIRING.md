# DFM 接线清单（现有代码 → 独立面板）

产品决定（2026-11-15）：新版 UI 由同事先交付；DFM 的主要手动入口是独立面板，用户在面板里运行检查、
看结果，再按需把报告卡拖进 Chat。**不再要求新版 Chat 增加“运行 DFM”按钮**，本轮也不实现面板、拖拽或
扩充触发短语。下面列出面板接线可直接复用的现有接口，以及仍缺的部分。

## 1. 面板点一次 → 启动 dfm.check、读状态、拿 run_id

现成接口：

- `crates/ui-egui/src/agent_chat/mod.rs:1442` `agent_chat::run_dfm(app) -> bool`
  —— 面板按钮直接调用。内部 `HostDfm::manual(&document_id, revision)` 生成一次手动触发，经
  `submit_dfm` 调用 `solidcraft_engine::dfm::service::start(&mut app.session)`；不会打开对话框，
  不需要 AI 连接。返回 `true` 表示已提交。
- `crates/engine/src/cmd/dfm_check.rs:24` 命令 `dfm.check`（同一入口的 command 形式，`params` 可选；
  返回 projectId、模型/配置 digest、sheet 数）。
- 运行中状态：`crates/ui-egui/src/agent_chat/mod.rs:1467` `agent_chat::running_dfm(app) -> Option<String>`
  （= `dfm::service::status().active_run`）；`crates/engine/src/cmd/dfm_check.rs:41` `dfm.status`
  返回 `{running, activeRun, lastRun, lastOutcome, lastReason, error}`。
- 取消：`agent_chat::cancel_dfm(app)`（mod.rs:1453）/ 命令 `dfm.cancel`。
- 轮询/刷新卡片：`crates/ui-egui/src/agent_chat/web.rs:870` `poll_check(app)`，以及
  `web::set_check_item(app, id, card)`（web.rs:950）把 run 状态与卡片写回行。
- run_id 来源：`dfm.status` 的 `lastRun`（本次运行完成时）或 `inspect.dfm` 返回的 `runId`；
  `solidcraft_engine::dfm::service::status()` 在窗口线程可直接读。

测试：`crates/ui-egui/src/agent_chat/tests.rs` 中
`an_accepted_preview_runs_one_check_and_a_rejection_or_stale_accept_runs_none`、
`the_web_page_accepts_a_waiting_preview`；`crates/engine/src/dfm/service_tests.rs` 的手动运行与取消用例。

## 2. 按 run_id 读报告、判过期、复用 finding ↔ 视口双向选择

现成接口：

- 报告读取：命令 `inspect.dfm`（`crates/engine/src/cmd/dfm_check.rs:47`）→ 结构化 JSON；
  `crates/ui-egui/src/dfm_chat.rs:76` `Card::from_inspect(&Value)` → `Card`
  （含 `run_id`、`model_version`、`candidate_digest`、`config_digest`、`freshness`、`findings`、`counts`）。
- 过期判断：`Card::from_inspect` 的 `freshness` 字段（`current`/`staleModel`/`staleConfig`/`staleBoth`/
  `unknown`），由 engine 对照当前 model/config digest 得出；面板只按该字段禁用旧标记即可，不要自行比较字符串。
- 卡片刷新：`dfm_chat::refresh(app) -> Card`（dfm_chat.rs:273）。
- 模型标记：`crates/ui-egui/src/dfm_overlay.rs` `RunFindings`；`crates/ui-egui/src/viewport.rs:336`
  `ViewportState::load_dfm_run(&RunFindings, Freshness)`、`:355` `select_dfm_finding(Option<&str>) -> bool`、
  `dfm_overlay.rs:300` `DfmState::locate_target()`（视口点击后给出 finding id）。
- 双向选择：`crates/ui-egui/src/agent_chat/web.rs:929` `selected_finding(app)`
  （先看视口 `viewport.dfm.selected_key()`，再看 `dfm_chat.selected`）；
  `web.rs:933` `sync_dfm_marks(app, card)` 把同一份报告送给视口；
  `web.rs:570` 的页面动作 `{"type":"finding","runId":...,"id":...}` 校验 finding 属于该 run 后同时改两边。
- 报告摘要给 AI：`dfm_chat::model_context(&Card)`（dfm_chat.rs:245，逐条 finding 的实测/阈值/规则）。

测试：`crates/ui-egui/src/agent_chat/web.rs` 内
`a_report_row_and_a_viewport_label_choose_the_same_finding`（行→视口、标记→行、非法 finding 被拒）；
`crates/ui-egui/src/dfm_overlay.rs` 与 `viewport.rs` 的 marks/freshness 用例。

## 3. Chat 现在如何接收结构化报告；缺什么

现成能力（可直接复用）：

- 报告卡在 Chat 中是 `Item::Tool{kind:"dfm.check"}`，页面数据由
  `crates/ui-egui/src/dfm_chat.rs:237` `page(&Card)` 生成并在 `web::set_check_item` 里写入行输出；
  页面组件 `packages/assistant-web/src/App.tsx` 的 `CheckCardView` 渲染计数、每条 finding、Refresh、
  `finding` 选择动作与 `dfm_display` 过滤。
- 模型发起的 `dfm.check` 工具行由 `web::adopt_model_check(app)`（web.rs）转成同一个卡片。
- 摘要进模型上下文：`dfm_chat::model_context(&Card)`；模型可通过工具再读 `inspect.dfm`。
- 附件通道现状：`crates/ui-egui/src/agent_chat/web.rs:196` `attach_dropped(app, paths)` 只接受
  **文件路径**（图片/文件→`chat.attachments`），消息体是文本 + 文件 + 选择上下文；
  目前没有“把一个结构化卡片作为附件随消息发送”的类型。

还缺（建议新增，属面板/拖拽接线范围）：

- 报告卡的附件表示：例如 `attach::Content::DfmReport { run_id }`（或消息 `context` 里一个
  `ContextItem { kind: "dfm_report", run_id }`），由 `web::act` 的 attach 入口接收；
- 发送时把该 run 的报告摘要放进消息（复用 `dfm_chat::model_context`）并在页面显示 chip；
- 面板 → Chat 的拖拽命中与投放（页面已有 file drop 通道，可扩展为接受语义词而不是路径）。

边界：`Chat` 当前用“检查一下…”短语与模型工具调用触发 DFM 的路径已验收（见 PIL-291 记录），
本轮只做回归；面板入口与拖拽由新版 UI 落地后接线。
